//! The host on Windows: a named pipe, and the key that proves the pipe is
//! the user's own host (spec 02, HOST-005, HOST-012, HOST-023, HOST-029).
//! Runs on the Windows CI job.
#![cfg(windows)]

use std::time::Duration;

use nodus_host::Logger;
use nodus_host::transport::connect;
use nodus_host_proto::{MessageReader, PROTOCOL, Platform, host_paths, trusts_host};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn host_023_needs_a_key_path_on_windows() {
    let (log, lines) = Logger::memory();
    assert_eq!(
        nodus_host::run(&[r"\\.\pipe\nodus-test-nokey".to_owned()], log).await,
        1
    );
    assert!(lines.lock().unwrap()[0].ends_with("Z failed to start Error: usage: host <socket path> <key path>"));
}

#[tokio::test]
async fn host_029_writes_a_key_and_welcomes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = host_paths(dir.path(), Platform::Windows);
    let (log, _lines) = Logger::memory();
    let args = [
        paths.socket.to_string_lossy().into_owned(),
        paths.key.to_string_lossy().into_owned(),
    ];
    let host = tokio::spawn(async move { nodus_host::run(&args, log).await });

    let mut stream = None;
    for _ in 0..100 {
        if let Ok(connected) = connect(&paths.socket).await {
            stream = Some(connected);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut stream = stream.expect("the host listens on its pipe");
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
    let key = welcome["key"].as_str().expect("a key on Windows");
    assert_eq!(key.len(), 64);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(std::fs::read_to_string(&paths.key).unwrap(), key);
    assert!(trusts_host(welcome.get("key"), &paths, Platform::Windows));

    // HOST-027: a second host can't take the pipe, and leaves.
    let (log, lines) = Logger::memory();
    let args = [
        paths.socket.to_string_lossy().into_owned(),
        paths.key.to_string_lossy().into_owned(),
    ];
    assert_eq!(nodus_host::run(&args, log).await, 0);
    assert!(lines.lock().unwrap()[0].ends_with("Z another host is running"));

    stream.write_all(b"{\"type\":\"shutdown\"}\n").await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), host)
            .await
            .unwrap()
            .unwrap(),
        0
    );
}
