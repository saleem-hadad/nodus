//! The server's HTTP side on port 7420 (SRV-021, SRV-022): `/api/hello`,
//! the setup code's claim, signing in, the GitHub App's manifest flow,
//! `/api/ws` upgraded to the WebSocket for a request with a token, and the
//! files' routes, which `files` answers (SRV-163).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONNECTION, HOST, HeaderMap, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nodus_wire::{
    GITHUB_CALLBACK_PATH, GITHUB_NONCE_PATH, GITHUB_START_PATH, HELLO_PATH, HttpError, MAX_FRAME, NonceRequest,
    SETUP_CLAIM_PATH, SIGN_IN_PATH, SIGN_IN_POLL_PATH, SOCKET_PATH, SetupClaim, SetupClaimed, SignInPoll,
    SignInPollRequest, SignInStart,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use crate::auth::SetupUse;
use crate::manifest;
use crate::net::bearer;
use crate::serve::Server;
use crate::setup::{self, Claim};
use crate::store::now_ms;
use crate::{files, socket};

type Reply = Response<Full<Bytes>>;

/// The largest request body a route reads: a PEM key is a few KB.
const MAX_BODY: usize = 64 * 1024;

/// Serves every connection `listener` accepts, until the server stops.
pub(crate) async fn accept(server: Arc<Server>, listener: TcpListener) {
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            // Out of file descriptors, most likely: give it a moment.
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |req| {
                let server = Arc::clone(&server);
                async move { Ok::<_, Infallible>(route(&server, req, peer).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

async fn route(server: &Arc<Server>, req: Request<Incoming>, peer: SocketAddr) -> Reply {
    // SRV-021: only a request that calls the server by one of its own
    // names, so a page whose name was pointed here (DNS rebinding) can't.
    let host = req.headers().get(HOST).and_then(|host| host.to_str().ok());
    if !server.hosts.allows(host) {
        return empty(StatusCode::FORBIDDEN);
    }
    // SRV-163: the files' routes are their module's.
    if files::handles(req.uri().path()) {
        return files::route(server, req).await;
    }
    let host = host.unwrap_or_default().to_owned();
    let auth = &server.auth;
    match (req.method().clone(), req.uri().path()) {
        (Method::GET, HELLO_PATH) => json(&auth.hello()),
        (Method::GET, SOCKET_PATH) => upgrade(server, req),
        (Method::POST, SETUP_CLAIM_PATH) => claim(server, req).await,
        (Method::POST, GITHUB_NONCE_PATH) => nonce(server, req).await,
        (Method::GET, GITHUB_START_PATH) => {
            let (status, page) = manifest::start(auth, &manifest::query(req.uri()), &host);
            html(status, page)
        }
        (Method::GET, GITHUB_CALLBACK_PATH) => {
            let (status, page) = manifest::callback(auth, &manifest::query(req.uri())).await;
            html(status, page)
        }
        (Method::POST, SIGN_IN_PATH) => {
            let setup_token = bearer(req.headers()).map(str::to_owned);
            let start: SignInStart = match read_json(req).await {
                Ok(start) => start,
                Err(reply) => return *reply,
            };
            match auth.start_sign_in(setup_token.as_deref(), &start.device_name).await {
                Ok(started) => json(&started),
                Err((status, message)) => failed(status, &message),
            }
        }
        (Method::POST, SIGN_IN_POLL_PATH) => {
            let poll: SignInPollRequest = match read_json(req).await {
                Ok(poll) => poll,
                Err(reply) => return *reply,
            };
            match auth.poll_sign_in(&poll.handle, peer.ip()).await {
                Ok(polled) => {
                    let status = match polled {
                        SignInPoll::Pending { .. } | SignInPoll::SignedIn { .. } => 200,
                        SignInPoll::Refused { .. } | SignInPoll::Denied { .. } => 403,
                        SignInPoll::Expired { .. } => 410,
                    };
                    json_status(status, &polled)
                }
                Err((status, message)) => failed(status, &message),
            }
        }
        _ => empty(StatusCode::NOT_FOUND),
    }
}

/// `POST /api/setup/claim` (SRV-060): the setup code, single-use, 10 tries
/// an hour, for a setup token.
async fn claim(server: &Arc<Server>, req: Request<Incoming>) -> Reply {
    let auth = &server.auth;
    let claim: SetupClaim = match read_json(req).await {
        Ok(claim) => claim,
        Err(reply) => return *reply,
    };
    if !auth.needs_setup() {
        return failed(409, "This server is set up already: sign in with GitHub");
    }
    match setup::claim(&server.data, &claim.code, now_ms()) {
        Ok(Claim::Token(token)) => {
            auth.audit.record("setup.claimed", None, None, json!({}));
            json(&SetupClaimed { token })
        }
        Ok(Claim::Wrong) => failed(
            401,
            "That code isn't right. Run nodus setup-code on the server for a new one.",
        ),
        Ok(Claim::TooMany) => failed(
            429,
            "Too many tries. Wait an hour, or run nodus setup-code on the server for a new code.",
        ),
        Err(err) => failed(500, &err),
    }
}

/// `POST /api/github/setup/nonce` (SRV-022, SRV-062): with a setup or
/// device token.
async fn nonce(server: &Arc<Server>, req: Request<Incoming>) -> Reply {
    let Some(who) = bearer(req.headers()).and_then(|token| server.auth.authenticate(token, SetupUse::Take)) else {
        return failed(401, "Sign in, or claim the setup code, first");
    };
    let asked: NonceRequest = match read_json_or_default(req).await {
        Ok(asked) => asked,
        Err(reply) => return *reply,
    };
    json(&server.auth.new_nonce(&who, asked.workflows))
}

/// `GET /api/ws`: the token first, then the WebSocket handshake (RFC 6455).
/// A device's token, or a setup token for the wizard's first steps.
fn upgrade(server: &Arc<Server>, mut req: Request<Incoming>) -> Reply {
    // SRV-022: in the Authorization header, and nowhere else.
    let Some(who) = bearer(req.headers()).and_then(|token| server.auth.authenticate(token, SetupUse::Take)) else {
        return empty(StatusCode::UNAUTHORIZED);
    };
    let Some(key) = websocket_key(req.headers()) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let accept = derive_accept_key(key.as_bytes());
    let upgrading = hyper::upgrade::on(&mut req);
    let server = Arc::clone(server);
    tokio::spawn(async move {
        let Ok(upgraded) = upgrading.await else {
            return;
        };
        // SRV-023: a bigger frame is an error, which closes the connection.
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let socket = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config)).await;
        socket::serve(server, socket, who).await;
    });
    let mut reply = empty(StatusCode::SWITCHING_PROTOCOLS);
    let headers = reply.headers_mut();
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    if let Ok(accept) = HeaderValue::from_str(&accept) {
        headers.insert(SEC_WEBSOCKET_ACCEPT, accept);
    }
    reply
}

/// The client's key, when the request is a version 13 WebSocket upgrade.
fn websocket_key(headers: &HeaderMap) -> Option<String> {
    let has = |name, token: &str| {
        headers.get_all(name).iter().any(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.split(',').any(|part| part.trim().eq_ignore_ascii_case(token)))
        })
    };
    let version = headers.get("Sec-WebSocket-Version").and_then(|v| v.to_str().ok());
    if !has(UPGRADE, "websocket") || !has(CONNECTION, "upgrade") || version != Some("13") {
        return None;
    }
    let key = headers.get(SEC_WEBSOCKET_KEY)?.to_str().ok()?;
    Some(key.to_owned())
}

fn empty(status: StatusCode) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = status;
    reply
}

fn json(value: &impl serde::Serialize) -> Reply {
    json_status(200, value)
}

fn json_status(status: u16, value: &impl serde::Serialize) -> Reply {
    let body = nodus_domain::json::to_string(value).expect("a reply is always JSON");
    let mut reply = Response::new(Full::new(Bytes::from(body)));
    *reply.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = reply.headers_mut();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    reply
}

/// A route that failed, saying why.
fn failed(status: u16, message: &str) -> Reply {
    json_status(
        status,
        &HttpError {
            message: message.to_owned(),
        },
    )
}

/// A page for the browser. It never leaves for another site but by its
/// form, and tells no other site where it came from: the nonce is in its
/// address.
fn html(status: u16, page: String) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::from(page)));
    *reply.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = reply.headers_mut();
    headers.insert("Content-Type", HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    reply
}

/// A JSON body of at most [`MAX_BODY`] bytes, or the reply saying why not.
async fn read_json<T: DeserializeOwned>(req: Request<Incoming>) -> Result<T, Box<Reply>> {
    let bytes = read_body(req).await?;
    nodus_domain::json::from_str(&String::from_utf8_lossy(&bytes))
        .map_err(|err| Box::new(failed(400, &format!("Bad request: {err}"))))
}

/// [`read_json`], with an empty body read as `T`'s default.
async fn read_json_or_default<T: DeserializeOwned + Default>(req: Request<Incoming>) -> Result<T, Box<Reply>> {
    let bytes = read_body(req).await?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    nodus_domain::json::from_str(&String::from_utf8_lossy(&bytes))
        .map_err(|err| Box::new(failed(400, &format!("Bad request: {err}"))))
}

async fn read_body(req: Request<Incoming>) -> Result<Bytes, Box<Reply>> {
    match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(body) => Ok(body.to_bytes()),
        Err(_) => Err(Box::new(failed(413, "The request is too big"))),
    }
}
