//! Speech to text through xAI (`transcribe`, VOICE-011…VOICE-018): the
//! recording and the hint words as a multipart POST, and the answer turned
//! into a transcript or a message the user can act on.

use std::time::Duration;

use nodus_domain::TranscribeResult;
use nodus_domain::js::{is_space, trim};
use reqwest::multipart::{Form, Part};
use serde_json::Value;

/// Where recordings go.
pub const STT_URL: &str = "https://api.x.ai/v1/stt";
/// The model that transcribes them.
pub const MODEL: &str = "grok-voice-transcribe-2.0";
/// How long xAI gets, the answer's body included (VOICE-015).
pub const TIMEOUT: Duration = Duration::from_secs(90);

pub const NO_KEY: &str = "Add an xAI API key in Settings to speak prompts.";
pub const TIMED_OUT: &str = "xAI didn’t answer in time.";
pub const UNREACHABLE: &str = "Couldn’t reach xAI. Check your connection.";
pub const KEY_REFUSED: &str = "xAI didn’t accept the API key.";

/// VOICE-011: what a transcription without a key answers.
pub fn no_key() -> TranscribeResult {
    TranscribeResult::Failed {
        error: NO_KEY.into(),
        key_problem: Some(true),
    }
}

/// VOICE-013: the upload's type, from the recording's MIME type, and the
/// file name: `audio/webm;codecs=opus` is `audio/webm` and `prompt.webm`.
pub fn upload_type(mime_type: &str) -> (String, String) {
    let ty = mime_type.split(';').next().unwrap_or_default();
    let ty = if ty.is_empty() { "audio/webm" } else { ty };
    let name = format!("prompt.{}", ty.split('/').nth(1).unwrap_or("webm"));
    (ty.to_owned(), name)
}

/// The type a `Blob` keeps: lowercased, or empty when it has characters
/// outside printable ASCII. FormData sends an empty one as
/// `application/octet-stream`.
fn blob_type(ty: &str) -> String {
    if ty.chars().all(|c| (' '..='~').contains(&c)) {
        ty.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// FormData writes a value's line breaks as CRLF.
fn crlf(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                out.push_str("\r\n");
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            '\n' => out.push_str("\r\n"),
            c => out.push(c),
        }
    }
    out
}

/// VOICE-014: the multipart body: the model, one `keyterm` per hint word,
/// then the audio.
pub fn form(audio: Vec<u8>, mime_type: &str, keyterms: &[String]) -> Form {
    let (ty, name) = upload_type(mime_type);
    let mut form = Form::new().text("model", MODEL);
    for term in keyterms {
        form = form.text("keyterm", crlf(term));
    }
    let content_type = Some(blob_type(&ty))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "application/octet-stream".into());
    // A type reqwest can't parse (`audio`, with no subtype) goes without one.
    let part = match Part::bytes(audio.clone())
        .file_name(name.clone())
        .mime_str(&content_type)
    {
        Ok(part) => part,
        Err(_) => Part::bytes(audio).file_name(name),
    };
    form.part("file", part)
}

/// xAI's own message without its trailing diagnostic codes
/// (`/\s*\[WKE=[^\]]*\]/g`), trimmed; or a message naming the status.
pub fn error_text(body: Option<&Value>, status: u16) -> String {
    let message = match body.and_then(|b| b.get("error")) {
        Some(Value::String(error)) => trim(&strip_wke(error)).to_owned(),
        _ => String::new(),
    };
    if message.is_empty() {
        format!("xAI answered with an error ({status}).")
    } else {
        message
    }
}

/// Removes each `[WKE=…]` and the spaces before it.
fn strip_wke(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("[WKE=") {
        let tail = &rest[at + "[WKE=".len()..];
        let Some(close) = tail.find(']') else {
            break;
        };
        out.push_str(rest[..at].trim_end_matches(is_space));
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// VOICE-016…VOICE-018: the answer, by status and body (`None` when it
/// wasn't JSON or didn't arrive in time).
pub fn read_answer(status: u16, body: Option<&Value>) -> TranscribeResult {
    if !(200..300).contains(&status) {
        let message = error_text(body, status);
        // A wrong key comes back as a 400 that says so; a missing or revoked one as a 401 or 403.
        let key_problem = status == 401 || status == 403 || message.to_ascii_lowercase().contains("api key");
        return TranscribeResult::Failed {
            error: if key_problem { KEY_REFUSED.into() } else { message },
            key_problem: key_problem.then_some(true),
        };
    }
    let text = match body.and_then(|b| b.get("text")) {
        Some(Value::String(text)) => trim(text).to_owned(),
        _ => String::new(),
    };
    TranscribeResult::Text { text }
}

/// `res.json()`: the body as UTF-8 (a leading BOM dropped), parsed.
fn parse_body(bytes: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    nodus_domain::json::from_str(text).ok()
}

/// Sends a recording to xAI and reads the answer. `url` is [`STT_URL`]
/// outside tests.
pub async fn send(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    audio: Vec<u8>,
    mime_type: &str,
    keyterms: &[String],
    timeout: Duration,
) -> TranscribeResult {
    let deadline = tokio::time::Instant::now() + timeout;
    let request = client
        .post(url)
        .header("Authorization", format!("Bearer {key}"))
        .multipart(form(audio, mime_type, keyterms))
        .send();
    // VOICE-015
    let res = match tokio::time::timeout_at(deadline, request).await {
        Ok(Ok(res)) => res,
        Ok(Err(_)) => return failed(UNREACHABLE),
        Err(_) => return failed(TIMED_OUT),
    };
    let status = res.status().as_u16();
    // VOICE-016: a body that can't be read in time is none.
    let body = match tokio::time::timeout_at(deadline, res.bytes()).await {
        Ok(Ok(bytes)) => parse_body(&bytes),
        _ => None,
    };
    read_answer(status, body.as_ref())
}

fn failed(error: &str) -> TranscribeResult {
    TranscribeResult::Failed {
        error: error.into(),
        key_problem: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_the_upload_by_its_type() {
        let cases = [
            ("audio/webm;codecs=opus", "audio/webm", "prompt.webm"),
            ("audio/webm", "audio/webm", "prompt.webm"),
            ("", "audio/webm", "prompt.webm"),
            (";codecs=opus", "audio/webm", "prompt.webm"),
            ("audio/ogg; codecs=opus", "audio/ogg", "prompt.ogg"),
            ("audio", "audio", "prompt.webm"),
            ("a/b/c", "a/b/c", "prompt.b"),
        ];
        for (mime, ty, name) in cases {
            assert_eq!(upload_type(mime), (ty.to_owned(), name.to_owned()), "{mime}");
        }
        assert_eq!(blob_type("Audio/WebM"), "audio/webm");
        assert_eq!(blob_type("audio/wébm"), "");
        assert_eq!(crlf("a\nb\r\nc\rd"), "a\r\nb\r\nc\r\nd");
    }

    #[test]
    fn says_what_went_wrong() {
        let body = json!({ "error": "Something broke [WKE=abc:123]  " });
        assert_eq!(error_text(Some(&body), 500), "Something broke");
        let both = json!({ "error": "One [WKE=a] two\u{FEFF} [WKE=b]" });
        assert_eq!(error_text(Some(&both), 500), "One two");
        assert_eq!(
            error_text(Some(&json!({ "error": " [WKE=x] " })), 502),
            "xAI answered with an error (502)."
        );
        assert_eq!(
            error_text(Some(&json!({ "error": 5 })), 400),
            "xAI answered with an error (400)."
        );
        assert_eq!(error_text(None, 503), "xAI answered with an error (503).");
        // An unclosed code is left as it is.
        assert_eq!(error_text(Some(&json!({ "error": "x [WKE=" })), 500), "x [WKE=");
    }

    #[test]
    fn reads_the_answer() {
        let ok = |body: Value| read_answer(200, Some(&body));
        assert_eq!(
            ok(json!({ "text": "  hello there " })),
            TranscribeResult::Text {
                text: "hello there".into()
            }
        );
        assert_eq!(ok(json!({ "text": 5 })), TranscribeResult::Text { text: String::new() });
        assert_eq!(read_answer(204, None), TranscribeResult::Text { text: String::new() });
        let refused = TranscribeResult::Failed {
            error: KEY_REFUSED.into(),
            key_problem: Some(true),
        };
        assert_eq!(read_answer(401, None), refused);
        assert_eq!(read_answer(403, Some(&json!({ "error": "Forbidden" }))), refused);
        assert_eq!(
            read_answer(400, Some(&json!({ "error": "Incorrect API Key provided" }))),
            refused
        );
        assert_eq!(
            read_answer(429, Some(&json!({ "error": "Too many requests [WKE=rate]" }))),
            TranscribeResult::Failed {
                error: "Too many requests".into(),
                key_problem: None
            }
        );
        assert_eq!(
            nodus_domain::json::to_string(&no_key()).unwrap(),
            r#"{"error":"Add an xAI API key in Settings to speak prompts.","keyProblem":true}"#
        );
        assert_eq!(
            parse_body(b"\xEF\xBB\xBF{\"text\":\"x\"}"),
            Some(json!({ "text": "x" }))
        );
        assert_eq!(parse_body(b"<html>"), None);
    }
}
