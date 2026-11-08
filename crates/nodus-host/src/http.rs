//! The hook endpoint on `127.0.0.1`: Claude Code's hooks and status line
//! post here, and the board tools' MCP requests come here to be relayed to
//! the app, so both keep working while the app restarts.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use nodus_domain::js::utf16_len;
use nodus_host_proto::{MCP_PATH, MCP_SESSION_HEADER, Utf8Decoder};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::host::Host;

/// How long a relayed request waits for the app's answer.
pub(crate) const RELAY_TIMEOUT: Duration = Duration::from_millis(20_000);
/// The largest MCP body, compared with its decoded length in UTF-16 units.
const MCP_MAX: usize = 1024 * 1024;
/// What a request gets when the app isn't there to answer it.
const APP_CLOSED: &str = "Nodus isn’t open, so the board can’t be reached right now.";

type Reply = Response<Full<Bytes>>;

/// Listens on `127.0.0.1`, on a port the system picks. (HOST-026)
pub(crate) async fn bind() -> std::io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

/// Serves hooks and the board tools until the host exits.
pub(crate) async fn serve(host: Arc<Host>, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let host = Arc::clone(&host);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |req| {
                let host = Arc::clone(&host);
                async move { Ok::<_, Infallible>(route(&host, req).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}

fn empty(status: StatusCode) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = status;
    reply
}

fn json_reply(value: &Value) -> Reply {
    let body = nodus_domain::json::to_string(value).expect("a reply is always JSON");
    let mut reply = Response::new(Full::new(Bytes::from(body)));
    reply
        .headers_mut()
        .insert("Content-Type", HeaderValue::from_static("application/json"));
    reply
}

async fn route(host: &Host, req: Request<Incoming>) -> Reply {
    let path = normalize_path(req.uri().path());
    let query = req.uri().query().map(parse_query).unwrap_or_default();
    // HOST-068: the board tools' path goes first, whatever the method or query.
    if path == MCP_PATH {
        return match serve_mcp(host, req).await {
            Ok(reply) => reply,
            Err(err) => {
                // HOST-090
                host.log.log(&["mcp request failed", &err]);
                empty(StatusCode::INTERNAL_SERVER_ERROR)
            }
        };
    }
    // HOST-069: processes started by an older app send the token in the URL.
    let token = bearer(req.headers()).or_else(|| first(&query, "token").map(str::to_owned));
    if req.method() != Method::POST || path != "/hook" || !host.is_hook_token(token.as_deref()) {
        // HOST-071
        return empty(StatusCode::NOT_FOUND);
    }
    // HOST-072: read to its end; the 204 goes out with nothing to wait for.
    let Ok(body) = req.into_body().collect().await else {
        return empty(StatusCode::NO_CONTENT);
    };
    let body = body.to_bytes();
    // HOST-073: no event, or no such session, and nothing more happens.
    let Some(event) = first(&query, "evt").filter(|event| !event.is_empty()) else {
        return empty(StatusCode::NO_CONTENT);
    };
    let session_id = first(&query, "sid").unwrap_or("");
    if !host.has_session(session_id) {
        return empty(StatusCode::NO_CONTENT);
    }
    // HOST-074: an empty body, or one that isn't JSON, is {}: status still
    // moves on the event's name alone.
    let text = String::from_utf8_lossy(&body);
    let payload = if text.is_empty() {
        json!({})
    } else {
        nodus_domain::json::from_str(&text).unwrap_or_else(|_| json!({}))
    };
    host.hook(event, session_id, payload);
    empty(StatusCode::NO_CONTENT)
}

/// The MCP streamable HTTP transport, answering every request on its POST:
/// there are no streams. (HOST-079–089)
async fn serve_mcp(host: &Host, req: Request<Incoming>) -> Result<Reply, String> {
    let authorized = host.is_hook_token(bearer(req.headers()).as_deref());
    let session_id = header_text(req.headers(), MCP_SESSION_HEADER);
    let live = host.has_session(&session_id);
    if !authorized || !live {
        return Ok(empty(StatusCode::UNAUTHORIZED));
    }
    if req.method() != Method::POST {
        // There's no stream to open (GET) and no session to end (DELETE).
        if req.method() == Method::DELETE {
            return Ok(empty(StatusCode::NO_CONTENT));
        }
        let mut reply = empty(StatusCode::METHOD_NOT_ALLOWED);
        reply.headers_mut().insert("Allow", HeaderValue::from_static("POST"));
        return Ok(reply);
    }

    // Decoded across chunks, so a character split between two arrives whole.
    let mut decoder = Utf8Decoder::new();
    let mut text = String::new();
    let mut length = 0;
    let mut body = req.into_body();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| format!("Error: {err}"))?;
        if let Ok(chunk) = frame.into_data() {
            let decoded = decoder.decode(&chunk);
            length += utf16_len(&decoded);
            text.push_str(&decoded);
            if length > MCP_MAX {
                let mut reply = empty(StatusCode::PAYLOAD_TOO_LARGE);
                reply
                    .headers_mut()
                    .insert("Connection", HeaderValue::from_static("close"));
                return Ok(reply);
            }
        }
    }
    text.push_str(&decoder.finish());

    let Ok(parsed) = nodus_domain::json::from_str::<Value>(&text) else {
        return Ok(json_reply(
            &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
        ));
    };
    let batch = parsed.is_array();
    let items = match parsed {
        Value::Array(items) => items,
        single => vec![single],
    };
    let requests: Vec<Value> = items.into_iter().filter(is_request).collect();
    // Notifications (no id) get no answer.
    if requests.is_empty() {
        return Ok(empty(StatusCode::ACCEPTED));
    }
    // Relayed together, in order, and answered in order.
    let relays: Vec<_> = requests
        .iter()
        .map(|request| host.start_relay(&session_id, request.clone()))
        .collect();
    let mut answers = Vec::with_capacity(relays.len());
    for (request, relay) in requests.iter().zip(relays) {
        let reply = match relay {
            Some(pending) => host.finish_relay(pending).await,
            None => None,
        };
        answers.push(match reply {
            Some(reply) if !reply.is_null() => reply,
            _ => unanswered(request),
        });
    }
    Ok(json_reply(&if batch {
        Value::Array(answers)
    } else {
        answers.swap_remove(0)
    }))
}

/// A JSON-RPC request: an object with an `id` (any value) and a string `method`. (HOST-083)
fn is_request(item: &Value) -> bool {
    item.as_object()
        .is_some_and(|fields| fields.contains_key("id") && fields.get("method").is_some_and(Value::is_string))
}

/// What a request gets when the app isn't there to answer it. A failed tool
/// call tells Claude why. (HOST-088)
fn unanswered(request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("method").and_then(Value::as_str) == Some("tools/call") {
        json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": APP_CLOSED}], "isError": true}})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": APP_CLOSED}})
    }
}

/// A header as Node reads it: Latin-1, and repeated ones joined by `, `.
fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get_all(name)
        .iter()
        .map(|value| latin1(value.as_bytes()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// `/^Bearer (\S+)$/` on the first Authorization header.
fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = latin1(headers.get("authorization")?.as_bytes());
    let token = value.strip_prefix("Bearer ")?;
    (!token.is_empty() && !token.chars().any(is_js_space)).then(|| token.to_owned())
}

/// JavaScript's `\s`.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `URLSearchParams`: name and value pairs, `+` as a space, percent escapes
/// decoded as UTF-8.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_decode(name), form_decode(value))
        })
        .collect()
}

fn form_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            ) {
                (Some(high), Some(low)) => {
                    out.push(high * 16 + low);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok())
}

/// `searchParams.get(name)`: the first value.
fn first<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// A request path as the WHATWG URL parser leaves it: `\` read as `/`, and
/// `.` and `..` segments (`%2e` too) resolved.
fn normalize_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    let parts: Vec<&str> = path.split('/').skip(1).collect();
    let last = parts.len().saturating_sub(1);
    for (i, segment) in parts.iter().enumerate() {
        let lower = segment.to_ascii_lowercase();
        let single = lower == "." || lower == "%2e";
        let double = matches!(lower.as_str(), ".." | ".%2e" | "%2e." | "%2e%2e");
        if double {
            segments.pop();
            if i == last {
                segments.push("");
            }
        } else if single {
            if i == last {
                segments.push("");
            }
        } else {
            segments.push(segment);
        }
    }
    format!("/{}", segments.join("/"))
}

impl Host {
    /// Compared in constant time, so the token can't be worked out from how
    /// long a check takes. (HOST-070)
    pub(crate) fn is_hook_token(&self, value: Option<&str>) -> bool {
        let given = value.unwrap_or("").as_bytes();
        let expected = self.hook_token.as_bytes();
        if given.len() != expected.len() {
            return false;
        }
        let difference = given.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b));
        std::hint::black_box(difference) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_queries_as_url_search_params_does() {
        let query = parse_query("evt=Stop&sid=a%20b+c&evt=Other&token=&flag&x=%E2%82%AC&bad=%zz%4");
        assert_eq!(first(&query, "evt"), Some("Stop"));
        assert_eq!(first(&query, "sid"), Some("a b c"));
        assert_eq!(first(&query, "token"), Some(""));
        assert_eq!(first(&query, "flag"), Some(""));
        assert_eq!(first(&query, "x"), Some("€"));
        assert_eq!(first(&query, "bad"), Some("%zz%4"));
        assert_eq!(first(&query, "missing"), None);
    }

    #[test]
    fn takes_a_bearer_token_only_in_the_exact_form() {
        let headers = |value: &'static str| {
            let mut map = HeaderMap::new();
            map.insert("authorization", HeaderValue::from_static(value));
            map
        };
        assert_eq!(bearer(&headers("Bearer abc123")), Some("abc123".into()));
        assert_eq!(bearer(&headers("bearer abc123")), None);
        assert_eq!(bearer(&headers("Bearer  abc123")), None);
        assert_eq!(bearer(&headers("Bearer abc 123")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn resolves_paths_as_the_url_parser_does() {
        assert_eq!(normalize_path("/hook"), "/hook");
        assert_eq!(normalize_path("/x/../hook"), "/hook");
        assert_eq!(normalize_path("/./mcp"), "/mcp");
        assert_eq!(normalize_path("/%2e/mcp"), "/mcp");
        assert_eq!(normalize_path("/hook/"), "/hook/");
        assert_eq!(normalize_path("/hook/."), "/hook/");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn answers_for_an_app_that_isnt_there() {
        assert_eq!(
            nodus_domain::json::to_string(&unanswered(&json!({"jsonrpc":"2.0","id":7,"method":"tools/call"}))).unwrap(),
            r#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"Nodus isn’t open, so the board can’t be reached right now."}],"isError":true}}"#
        );
        assert_eq!(
            nodus_domain::json::to_string(&unanswered(&json!({"jsonrpc":"2.0","id":"a","method":"tools/list"})))
                .unwrap(),
            r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32603,"message":"Nodus isn’t open, so the board can’t be reached right now."}}"#
        );
    }

    #[test]
    fn tells_requests_from_notifications() {
        assert!(is_request(&json!({"id": null, "method": "x"})));
        assert!(is_request(&json!({"id": 1, "method": "x"})));
        assert!(!is_request(&json!({"method": "notifications/initialized"})));
        assert!(!is_request(&json!({"id": 1, "method": 5})));
        assert!(!is_request(&json!([1])));
    }
}
