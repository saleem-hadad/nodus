//! `trust`: the host marks a folder trusted in its own user's `.claude.json`
//! (SRV-126). Its own test binary, as it points `CLAUDE_CONFIG_DIR` at a
//! scratch folder for the whole process.
#![cfg(unix)]

use std::time::Duration;

use nodus_host::Logger;
use nodus_host::transport::connect;
use nodus_host_proto::{MessageReader, PROTOCOL};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn trusts_a_folder_for_the_hosts_user() {
    let dir = tempfile::Builder::new()
        .prefix("nodus-trust-")
        .tempdir_in("/tmp")
        .unwrap();
    let config = dir.path().join(".claude.json");
    std::fs::write(
        &config,
        r#"{"numStartups": 3, "projects": {"/srv/a": {"history": []}}}"#,
    )
    .unwrap();
    // SAFETY: this binary's only test, on a current-thread runtime, before
    // anything that reads the environment runs.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };

    let socket = dir.path().join("host.sock");
    let (log, _lines) = Logger::memory();
    let args = [socket.to_string_lossy().into_owned(), String::new()];
    tokio::spawn(async move { nodus_host::run(&args, log).await });
    let mut stream = None;
    for _ in 0..100 {
        if let Ok(connected) = connect(&socket).await {
            stream = Some(connected);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut stream = stream.expect("the host listens");
    let mut reader = MessageReader::new();
    let mut said = Vec::new();
    let mut ask = async |msg: Value| {
        let line = nodus_domain::json::to_string(&msg).unwrap() + "\n";
        stream.write_all(line.as_bytes()).await.unwrap();
        let mut buffer = vec![0u8; 4096];
        loop {
            if !said.is_empty() {
                return said.remove(0);
            }
            let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            said.extend(reader.feed(&buffer[..n]));
        }
    };
    assert_eq!(
        ask(json!({"type": "hello", "protocol": PROTOCOL})).await["type"],
        "welcome"
    );
    assert_eq!(
        ask(json!({"type": "trust", "id": 1, "path": "/srv/a"})).await,
        json!({"type": "trusted", "id": 1})
    );
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(
        written,
        json!({"numStartups": 3, "projects": {"/srv/a": {"history": [], "hasTrustDialogAccepted": true}}})
    );
    assert_eq!(
        ask(json!({"type": "trust", "id": 2, "path": ""})).await,
        json!({"type": "trusted", "id": 2, "error": "There’s no folder to trust"})
    );
}
