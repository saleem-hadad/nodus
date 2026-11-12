//! The server's HTTP side on port 7420 (SRV-021, SRV-022): `/api/hello`,
//! and `/api/ws` upgraded to the WebSocket for a request with the token.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONNECTION, HOST, HeaderMap, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nodus_wire::{HELLO_PATH, MAX_FRAME, SOCKET_PATH};
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use crate::net::{bearer, same_token};
use crate::serve::Server;
use crate::socket;

type Reply = Response<Full<Bytes>>;

/// Serves every connection `listener` accepts, until the server stops.
pub(crate) async fn accept(server: Arc<Server>, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            // Out of file descriptors, most likely: give it a moment.
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |req| {
                let server = Arc::clone(&server);
                async move { Ok::<_, Infallible>(route(&server, req)) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

fn route(server: &Arc<Server>, req: Request<Incoming>) -> Reply {
    // SRV-021: only a request that calls the server by one of its own
    // names, so a page whose name was pointed here (DNS rebinding) can't.
    let host = req.headers().get(HOST).and_then(|host| host.to_str().ok());
    if !server.hosts.allows(host) {
        return empty(StatusCode::FORBIDDEN);
    }
    match (req.method(), req.uri().path()) {
        (&Method::GET, HELLO_PATH) => json(&server.hello),
        (&Method::GET, SOCKET_PATH) => upgrade(server, req),
        _ => empty(StatusCode::NOT_FOUND),
    }
}

/// `GET /api/ws`: the token first, then the WebSocket handshake (RFC 6455).
fn upgrade(server: &Arc<Server>, mut req: Request<Incoming>) -> Reply {
    // SRV-022: in the Authorization header, and nowhere else.
    if !bearer(req.headers()).is_some_and(|token| same_token(token, &server.token)) {
        return empty(StatusCode::UNAUTHORIZED);
    }
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
        socket::serve(server, socket).await;
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
    let body = nodus_domain::json::to_string(value).expect("a reply is always JSON");
    let mut reply = Response::new(Full::new(Bytes::from(body)));
    reply
        .headers_mut()
        .insert("Content-Type", HeaderValue::from_static("application/json"));
    reply
}
