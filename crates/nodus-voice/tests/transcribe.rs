//! The key and the xAI request against a local stand-in for xAI: nothing
//! here reaches the real API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{Project, TranscribeResult, VoiceKeySource};
use nodus_git::Git;
use nodus_platform::secrets::SafeStorage;
use nodus_store::Store;
use nodus_voice::{KeyError, Voice, VoiceKeys, stt};
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A request as the stand-in saw it.
#[derive(Debug, Clone, Default)]
struct Seen {
    head: String,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    }

    /// The multipart parts: (headers, content).
    fn parts(&self) -> Vec<(String, Vec<u8>)> {
        let ct = self.header("content-type").unwrap();
        let boundary = ct.split("boundary=").nth(1).unwrap().trim_matches('"');
        let delimiter = format!("--{boundary}");
        let body = &self.body;
        let mut parts = Vec::new();
        let mut at = find(body, delimiter.as_bytes(), 0).unwrap() + delimiter.len();
        loop {
            if body[at..].starts_with(b"--") {
                break;
            }
            at += 2; // CRLF after the delimiter
            let end = find(body, format!("\r\n{delimiter}").as_bytes(), at).unwrap();
            let part = &body[at..end];
            let split = find(part, b"\r\n\r\n", 0).unwrap();
            parts.push((
                String::from_utf8(part[..split].to_vec()).unwrap(),
                part[split + 4..].to_vec(),
            ));
            at = end + 2 + delimiter.len();
        }
        parts
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// How the stand-in answers.
#[derive(Clone)]
enum Reply {
    Json(u16, String),
    Never,
    /// Headers, then less body than promised, then nothing.
    StalledBody,
}

struct Stub {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

async fn stub(reply: Reply) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/stt", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let reply = reply.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let request = read_request(&mut socket).await;
                log.lock().unwrap().push(request);
                match reply {
                    Reply::Json(status, body) => {
                        let res = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(res.as_bytes()).await;
                    }
                    Reply::Never => tokio::time::sleep(Duration::from_secs(60)).await,
                    Reply::StalledBody => {
                        let _ = socket
                            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"text\":")
                            .await;
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                }
            });
        }
    });
    Stub { url, seen }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Seen {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = socket.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(at) = find(&buf, b"\r\n\r\n", 0) {
            break at;
        }
    };
    let head = String::from_utf8(buf[..head_end].to_vec()).unwrap();
    let mut seen = Seen {
        head,
        body: buf[head_end + 4..].to_vec(),
    };
    if let Some(len) = seen.header("content-length") {
        let len: usize = len.parse().unwrap();
        while seen.body.len() < len {
            let n = socket.read(&mut chunk).await.unwrap();
            seen.body.extend_from_slice(&chunk[..n]);
        }
    } else {
        // Chunked: read to the last chunk, then decode.
        while find(&seen.body, b"\r\n0\r\n\r\n", 0).is_none() && !seen.body.starts_with(b"0\r\n\r\n") {
            let n = socket.read(&mut chunk).await.unwrap();
            seen.body.extend_from_slice(&chunk[..n]);
        }
        let mut decoded = Vec::new();
        let mut at = 0;
        loop {
            let line_end = find(&seen.body, b"\r\n", at).unwrap();
            let size = usize::from_str_radix(std::str::from_utf8(&seen.body[at..line_end]).unwrap(), 16).unwrap();
            if size == 0 {
                break;
            }
            decoded.extend_from_slice(&seen.body[line_end + 2..line_end + 2 + size]);
            at = line_end + 2 + size + 2;
        }
        seen.body = decoded;
    }
    seen
}

struct Fixture {
    _dir: TempDir,
    store: Arc<Store>,
    project: Project,
}

fn fixture(repo: &std::path::Path) -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let project: Project = serde_json::from_value(json!({
        "id": "p1", "name": "Nodus Demo", "repoPath": repo.display().to_string(), "color": "blue", "createdAt": 1
    }))
    .unwrap();
    store.update(|s| s.projects.push(project.clone()));
    Fixture {
        _dir: dir,
        store,
        project,
    }
}

fn storage() -> Arc<SafeStorage> {
    Arc::new(SafeStorage::with_mac_password(b"synthetic test password"))
}

fn git() -> Git {
    let mut env: nodus_platform::env::EnvMap = std::env::vars().collect();
    env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    Git::new(env, Arc::new(nodus_platform::runner::LocalRunner))
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap()
        .success();
    assert!(ok);
}

#[tokio::test]
async fn sends_the_recording_with_hint_words() {
    let repo = TempDir::new().unwrap();
    run_git(repo.path(), &["init", "-q", "-b", "feature/voice"]);
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    std::fs::write(repo.path().join("src/BoardView.tsx"), "x").unwrap();
    std::fs::write(repo.path().join("a.ts"), "x").unwrap();
    let f = fixture(repo.path());
    let server = stub(Reply::Json(200, r#"{"text":"  Open the BoardView.  "}"#.into())).await;
    let voice = Voice::new(f.store.clone(), storage()).with_endpoint(&server.url, Duration::from_secs(5));

    let audio = vec![0x1a, 0x45, 0xdf, 0xa3, 0, 1, 2, 255];
    let result = voice
        .transcribe(
            &git(),
            &f.project,
            audio.clone(),
            "audio/webm;codecs=opus",
            Some(" xai-test-key "),
        )
        .await;
    assert_eq!(
        result,
        TranscribeResult::Text {
            text: "Open the BoardView.".into()
        }
    );

    let seen = server.seen.lock().unwrap()[0].clone();
    assert!(seen.head.starts_with("POST /v1/stt HTTP/1.1\r\n"), "{}", seen.head);
    assert_eq!(seen.header("authorization").as_deref(), Some("Bearer xai-test-key"));
    assert!(
        seen.header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    let parts = seen.parts();
    let names: Vec<(&str, String)> = parts
        .iter()
        .map(|(h, b)| (h.as_str(), String::from_utf8_lossy(b).into_owned()))
        .collect();
    assert_eq!(names.len(), 5);
    assert_eq!(
        names[0],
        (
            "Content-Disposition: form-data; name=\"model\"",
            "grok-voice-transcribe-2.0".into()
        )
    );
    assert_eq!(
        names[1],
        ("Content-Disposition: form-data; name=\"keyterm\"", "Nodus Demo".into())
    );
    assert_eq!(
        names[2],
        (
            "Content-Disposition: form-data; name=\"keyterm\"",
            "feature/voice".into()
        )
    );
    assert_eq!(
        names[3],
        ("Content-Disposition: form-data; name=\"keyterm\"", "BoardView".into())
    );
    assert_eq!(
        parts[4].0,
        "Content-Disposition: form-data; name=\"file\"; filename=\"prompt.webm\"\r\nContent-Type: audio/webm"
    );
    assert_eq!(parts[4].1, audio);
}

#[tokio::test]
async fn asks_for_a_key_without_sending_anything() {
    let f = fixture(std::path::Path::new("/nonexistent"));
    let server = stub(Reply::Json(200, "{}".into())).await;
    let voice = Voice::new(f.store.clone(), storage()).with_endpoint(&server.url, Duration::from_secs(5));
    let result = voice
        .transcribe(&git(), &f.project, vec![1], "audio/webm", Some("  "))
        .await;
    assert_eq!(result, stt::no_key());
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reports_what_xai_said() {
    let f = fixture(std::path::Path::new("/nonexistent"));
    let cases = [
        (401, r#"{"error":"Unauthorized"}"#, stt::KEY_REFUSED, true),
        (
            400,
            r#"{"error":"Incorrect API key provided: xa***ey. [WKE=unauthenticated:bad-key]"}"#,
            stt::KEY_REFUSED,
            true,
        ),
        (
            500,
            r#"{"error":"The model is overloaded [WKE=internal:overload]"}"#,
            "The model is overloaded",
            false,
        ),
        (
            502,
            "<html>Bad gateway</html>",
            "xAI answered with an error (502).",
            false,
        ),
    ];
    for (status, body, error, key_problem) in cases {
        let server = stub(Reply::Json(status, body.into())).await;
        let voice = Voice::new(f.store.clone(), storage()).with_endpoint(&server.url, Duration::from_secs(5));
        let result = voice
            .transcribe(&git(), &f.project, vec![1], "audio/webm", Some("k"))
            .await;
        assert_eq!(
            result,
            TranscribeResult::Failed {
                error: error.into(),
                key_problem: key_problem.then_some(true)
            },
            "{status}"
        );
    }
}

#[tokio::test]
async fn gives_up_on_xai_in_time() {
    let f = fixture(std::path::Path::new("/nonexistent"));
    let quick = Duration::from_millis(300);
    let server = stub(Reply::Never).await;
    let voice = Voice::new(f.store.clone(), storage()).with_endpoint(&server.url, quick);
    let result = voice
        .transcribe(&git(), &f.project, vec![1], "audio/webm", Some("k"))
        .await;
    assert_eq!(
        result,
        TranscribeResult::Failed {
            error: stt::TIMED_OUT.into(),
            key_problem: None
        }
    );
    // A body cut off by the time limit is no body: an empty transcript.
    let server = stub(Reply::StalledBody).await;
    let voice = Voice::new(f.store.clone(), storage()).with_endpoint(&server.url, quick);
    let result = voice
        .transcribe(&git(), &f.project, vec![1], "audio/webm", Some("k"))
        .await;
    assert_eq!(result, TranscribeResult::Text { text: String::new() });
}

#[tokio::test]
async fn says_when_xai_cant_be_reached() {
    let f = fixture(std::path::Path::new("/nonexistent"));
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/stt", closed.local_addr().unwrap());
    drop(closed);
    let voice = Voice::new(f.store.clone(), storage()).with_endpoint(url, Duration::from_secs(5));
    let result = voice
        .transcribe(&git(), &f.project, vec![1], "audio/webm", Some("k"))
        .await;
    assert_eq!(
        result,
        TranscribeResult::Failed {
            error: stt::UNREACHABLE.into(),
            key_problem: None
        }
    );
}

#[test]
fn saves_and_forgets_the_key() {
    let f = fixture(std::path::Path::new("/nonexistent"));
    let keys = VoiceKeys::new(f.store.clone(), storage());
    // VOICE-001
    assert_eq!(keys.key_source(None), None);
    assert_eq!(keys.key_source(Some("  ")), None);
    assert_eq!(keys.key_source(Some("shell")), Some(VoiceKeySource::Env));
    // VOICE-003, VOICE-004
    assert_eq!(
        keys.set_key(Some("  saved-key  "), None),
        Ok(Some(VoiceKeySource::Settings))
    );
    let sealed = f.store.read(|s| s.voice_key.clone()).unwrap();
    assert!(sealed.starts_with("djEw"), "base64 of v10…: {sealed}");
    assert!(!sealed.contains("saved-key"));
    // VOICE-006: the saved key wins.
    assert_eq!(keys.key(Some("shell")).as_deref(), Some("saved-key"));
    // A key that won't open still reads as saved, and the shell's is used.
    let other = VoiceKeys::new(f.store.clone(), Arc::new(SafeStorage::with_mac_password(b"another")));
    assert_eq!(other.key_source(None), Some(VoiceKeySource::Settings));
    assert_eq!(other.key(Some("shell")).as_deref(), Some("shell"));
    assert_eq!(other.key(None), None);
    // Forgetting: empty, blank or missing.
    assert_eq!(keys.set_key(Some("   "), Some("shell")), Ok(Some(VoiceKeySource::Env)));
    assert_eq!(f.store.read(|s| s.voice_key.clone()), None);
    keys.set_key(Some("k"), None).unwrap();
    assert_eq!(keys.set_key(None, None), Ok(None));
    // No Keychain: nothing changes.
    keys.set_key(Some("kept"), None).unwrap();
    let unavailable = VoiceKeys::new(f.store.clone(), Arc::new(SafeStorage::unavailable()));
    let err = unavailable.set_key(Some("new"), None).unwrap_err();
    assert_eq!(err, KeyError::Unavailable);
    assert!(
        err.to_string()
            .ends_with("so the key can’t be saved. Set XAI_API_KEY in your shell instead.")
    );
    assert_eq!(keys.key(None).as_deref(), Some("kept"));
    // VOICE-005
    assert_eq!(nodus_voice::key_arg(&json!(5)), None);
    assert_eq!(nodus_voice::key_arg(&json!("k")), Some("k"));
}
