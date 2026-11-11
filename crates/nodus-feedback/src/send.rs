//! Posting a reviewed report to the feedback server (`POST /v1/feedback`),
//! which uploads the screenshots and files the issue on GitHub. Only the
//! server holds GitHub's credentials; the app never does (RPT-030).

use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde_json::Value;

use crate::images::kind_of;

/// Long enough to connect to a server that was asleep.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Five 10 MB screenshots over a slow uplink take minutes.
const TIMEOUT: Duration = Duration::from_secs(180);

/// Which copy of Nodus sent it, for the issue's footer (RPT-023).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// `1.0.2`
    pub version: String,
    /// `macOS 27.2`
    pub os: String,
    /// `arm64`
    pub arch: String,
}

/// A report as the user reviewed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub title: String,
    pub body: String,
    /// The GitHub username to notify, already checked.
    pub reporter: Option<String>,
    pub client: ClientInfo,
    /// Each screenshot's bytes, as [`crate::prepare`] left them.
    pub shots: Vec<Vec<u8>>,
}

/// The issue the report became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub number: u64,
    pub url: String,
}

/// Why a report wasn't posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// RPT-034: no server is set, it couldn't be reached, it took too long,
    /// or what answered wasn't the server (a proxy's page). The browser
    /// takes over.
    Unreachable,
    /// RPT-034: the server reached GitHub and GitHub didn't take the
    /// report (502). The browser can still.
    GitHub(String),
    /// RPT-035: the server turned the report down (400, 401, 413, 429): its
    /// sentence, to show as it is.
    Refused(String),
}

/// RPT-031, RPT-032: sends `report` to `server`'s `/v1/feedback` with the
/// app's key: the text fields, then each screenshot as `screenshots`.
pub async fn send(server: &str, key: &str, report: &Report) -> Result<Posted, SendError> {
    let server = server.trim().trim_end_matches('/');
    if server.is_empty() {
        return Err(SendError::Unreachable);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .build()
        .map_err(|_| SendError::Unreachable)?;
    let response = client
        .post(format!("{server}/v1/feedback"))
        .header("X-Nodus-Key", key)
        .multipart(form(report))
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "report: the feedback server couldn't be reached");
            SendError::Unreachable
        })?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let bytes = response.bytes().await.map_err(|_| SendError::Unreachable)?;
    answer(status, retry_after, serde_json::from_slice(&bytes).ok())
}

/// The multipart body (RPT-032).
fn form(report: &Report) -> Form {
    let mut form = Form::new()
        .text("title", report.title.clone())
        .text("body", report.body.clone())
        .text("version", report.client.version.clone())
        .text("os", report.client.os.clone())
        .text("arch", report.client.arch.clone());
    if let Some(reporter) = &report.reporter {
        form = form.text("reporter", reporter.clone());
    }
    for (i, shot) in report.shots.iter().enumerate() {
        let mime = kind_of(shot).unwrap_or("image/png");
        let ext = mime.trim_start_matches("image/").replace("jpeg", "jpg");
        let part = Part::bytes(shot.clone()).file_name(format!("screenshot-{}.{ext}", i + 1));
        let part = match part.mime_str(mime) {
            Ok(part) => part,
            Err(_) => Part::bytes(shot.clone()).file_name(format!("screenshot-{}.png", i + 1)),
        };
        form = form.part("screenshots", part);
    }
    form
}

/// RPT-033…RPT-035: what the server's answer means. Only the server's own
/// JSON counts as an answer; anything else is a server that wasn't reached.
fn answer(status: u16, retry_after: Option<u64>, json: Option<Value>) -> Result<Posted, SendError> {
    let Some(json) = json else {
        return Err(SendError::Unreachable);
    };
    let said = json
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    match status {
        200 | 201 => {
            let number = json.get("number").and_then(Value::as_u64);
            let url = json
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://github.com/"));
            match (number, url) {
                (Some(number), Some(url)) => Ok(Posted {
                    number,
                    url: url.to_owned(),
                }),
                _ => Err(SendError::Unreachable),
            }
        }
        502 => Err(SendError::GitHub(said.unwrap_or_else(|| {
            "GitHub didn't accept the report. Try again in a minute.".to_owned()
        }))),
        429 => Err(SendError::Refused(said.unwrap_or_else(|| {
            let minutes = retry_after.map_or(60, |s| s.div_ceil(60).max(1));
            format!("Too many reports for now. Try again in {minutes} min.")
        }))),
        400 | 401 | 413 => {
            Err(SendError::Refused(said.unwrap_or_else(|| {
                "The feedback server turned the report down.".to_owned()
            })))
        }
        _ => Err(SendError::Unreachable),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn reads_a_posted_issue() {
        let ok = answer(
            201,
            None,
            Some(json!({"number": 42, "url": "https://github.com/saleem-hadad/nodus/issues/42"})),
        );
        assert_eq!(
            ok,
            Ok(Posted {
                number: 42,
                url: "https://github.com/saleem-hadad/nodus/issues/42".into()
            })
        );
        // Not a GitHub link: not trusted.
        let odd = answer(201, None, Some(json!({"number": 42, "url": "https://example.com/x"})));
        assert_eq!(odd, Err(SendError::Unreachable));
    }

    #[test]
    fn tells_refusals_from_an_unreachable_server() {
        assert_eq!(answer(502, None, None), Err(SendError::Unreachable));
        assert_eq!(
            answer(503, None, Some(json!({"error": "down"}))),
            Err(SendError::Unreachable)
        );
        assert_eq!(
            answer(502, None, Some(json!({"error": "GitHub said no."}))),
            Err(SendError::GitHub("GitHub said no.".into()))
        );
        assert_eq!(
            answer(400, None, Some(json!({"error": "That isn't a GitHub username."}))),
            Err(SendError::Refused("That isn't a GitHub username.".into()))
        );
        assert_eq!(
            answer(429, Some(90), Some(json!({}))),
            Err(SendError::Refused(
                "Too many reports for now. Try again in 2 min.".into()
            ))
        );
    }

    #[tokio::test]
    async fn no_server_is_unreachable() {
        assert_eq!(send(" ", "k", &Report::default()).await, Err(SendError::Unreachable));
        // Nothing listens on port 9 here.
        assert_eq!(
            send("http://127.0.0.1:9", "k", &Report::default()).await,
            Err(SendError::Unreachable)
        );
    }

    /// One request's raw bytes, answered with `reply`.
    async fn serve_once(reply: &'static str) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                let Some(head_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let head = text[..head_end].to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok());
                let done = match length {
                    Some(length) => got.len() >= head_end + 4 + length,
                    None => text.ends_with("\r\n0\r\n\r\n"),
                };
                if done {
                    break;
                }
            }
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            got
        });
        (url, task)
    }

    #[tokio::test]
    async fn posts_the_report_as_multipart() {
        let body = r#"{"number":7,"url":"https://github.com/saleem-hadad/nodus/issues/7"}"#;
        let reply: &'static str = Box::leak(
            format!(
                "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (url, task) = serve_once(reply).await;
        let report = Report {
            title: "It broke".into(),
            body: "Reported by @ada\n\nDetails".into(),
            reporter: Some("ada".into()),
            client: ClientInfo {
                version: "1.0.2".into(),
                os: "macOS 27.2".into(),
                arch: "arm64".into(),
            },
            shots: vec![b"\x89PNG\r\n\x1a\nfake".to_vec(), vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2]],
        };
        let posted = send(&format!("{url}/"), "the-key", &report).await;
        assert_eq!(
            posted,
            Ok(Posted {
                number: 7,
                url: "https://github.com/saleem-hadad/nodus/issues/7".into()
            })
        );
        let raw = String::from_utf8_lossy(&task.await.unwrap()).into_owned();
        assert!(raw.starts_with("POST /v1/feedback HTTP/1.1\r\n"), "{raw}");
        assert!(raw.to_ascii_lowercase().contains("x-nodus-key: the-key\r\n"));
        assert!(raw.contains("multipart/form-data; boundary="));
        for (name, value) in [
            ("title", "It broke"),
            ("body", "Reported by @ada\n\nDetails"),
            ("reporter", "ada"),
            ("version", "1.0.2"),
            ("os", "macOS 27.2"),
            ("arch", "arm64"),
        ] {
            assert!(
                raw.contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n")),
                "{name} missing in {raw}"
            );
        }
        assert!(raw.contains("name=\"screenshots\"; filename=\"screenshot-1.png\"\r\nContent-Type: image/png"));
        assert!(raw.contains("name=\"screenshots\"; filename=\"screenshot-2.jpg\"\r\nContent-Type: image/jpeg"));
    }

    #[tokio::test]
    async fn a_page_that_isnt_json_is_unreachable() {
        let (url, _task) = serve_once(
            "HTTP/1.1 502 Bad Gateway\r\ncontent-type: text/html\r\ncontent-length: 5\r\nconnection: close\r\n\r\n<html",
        )
        .await;
        assert_eq!(send(&url, "k", &Report::default()).await, Err(SendError::Unreachable));
    }
}
