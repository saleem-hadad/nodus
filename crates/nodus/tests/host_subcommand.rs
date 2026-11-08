//! `Nodus host <socket> <key>` runs the session host (spec 02, HOST-022,
//! HOST-023, HOST-034, HOST-035).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use nodus_host::transport::connect;
use nodus_host_proto::{MessageReader, PROTOCOL};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const NODUS: &str = env!("CARGO_BIN_EXE_nodus");

#[test]
fn host_023_without_a_socket_logs_the_usage_and_exits_1() {
    let out = Command::new(NODUS).arg("host").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let log = String::from_utf8(out.stdout).unwrap();
    // HOST-034: an ISO time, a space, the parts.
    let (time, rest) = log.split_at(24);
    assert!(time.ends_with('Z') && time.as_bytes()[10] == b'T', "{log:?}");
    assert_eq!(rest, " failed to start Error: usage: host <socket path> <key path>\n");
}

/// The app starts the host with its stdout and stderr on host.log; the host
/// listens, greets an app, and stops when asked.
#[tokio::test]
async fn host_022_runs_the_host_from_the_app_binary() {
    let dir = tempfile::Builder::new()
        .prefix("nodus-bin-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    #[cfg(unix)]
    let socket = dir.path().join("host.sock");
    #[cfg(windows)]
    let socket = std::path::PathBuf::from(format!(r"\\.\pipe\nodus-test-{}", std::process::id()));
    let log_path = dir.path().join("host.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut host = Command::new(NODUS)
        .args([
            "host".as_ref(),
            socket.as_os_str(),
            dir.path().join("host.key").as_os_str(),
        ])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();

    let mut stream = wait_for(&socket).await;
    stream
        .write_all(format!("{{\"type\":\"hello\",\"protocol\":{PROTOCOL}}}\n").as_bytes())
        .await
        .unwrap();
    let mut reader = MessageReader::new();
    let mut buffer = vec![0u8; 4096];
    let welcome = loop {
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        if let Some(msg) = reader.feed(&buffer[..n]).pop() {
            break msg;
        }
    };
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["sessions"], json!([]));
    stream.write_all(b"{\"type\":\"shutdown\"}\n").await.unwrap();

    let status = tokio::task::spawn_blocking(move || host.wait()).await.unwrap().unwrap();
    assert_eq!(status.code(), Some(0));
    let logged = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = logged.lines().map(|line| &line[25..]).collect();
    assert_eq!(
        lines[0],
        format!("listening {} hooks on {}", socket.display(), welcome["hookPort"])
    );
    assert_eq!(
        &lines[1..],
        ["app connected ", "stopping every session", "idle, exiting"]
    );
}

async fn wait_for(socket: &Path) -> nodus_host::transport::ClientStream {
    for _ in 0..100 {
        if let Ok(stream) = connect(socket).await {
            return stream;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the host didn't start");
}
