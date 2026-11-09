//! HOST-098 on Windows, the only platform it can happen on: anyone can make
//! a pipe of the host's name first, so the host proves itself with the key it
//! wrote, and the client trusts nothing else with the sessions.
#![cfg(windows)]

use std::sync::Arc;
use std::time::Duration;

use nodus_domain::{Event, EventSink, SessionStatus};
use nodus_host_proto::{HostPaths, MessageReader, PROTOCOL, Platform, host_paths};
use nodus_sessions::client::{
    BoardTools, BoxFuture, ClientError, HostClient, HostLauncher, HostPrompt, ReplaceChoice, Wiring,
};
use nodus_sessions::hooks::{HookEndpoint, SessionInput};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

const UNTRUSTED: &str = "Another program answered as the session host, so Nodus didn’t trust it with your sessions.";

struct Quiet;

impl SessionInput for Quiet {
    fn process_started(&self, _: &str, _: SessionStatus) {}
    fn process_exited(&self, _: &str) {}
    fn hook(&self, _: &str, _: &str, _: &Value, _: Option<f64>) {}
    fn user_input(&self, _: &str, _: &str, _: Option<f64>) {}
}

impl EventSink for Quiet {
    fn emit(&self, _: Event) {}
}

impl BoardTools for Quiet {
    fn answer(&self, _: &str, _: Value) -> BoxFuture<Result<Option<Value>, String>> {
        Box::pin(async { Ok(None) })
    }
}

impl HostPrompt for Quiet {
    fn other_version(&self) -> BoxFuture<ReplaceChoice> {
        Box::pin(async { ReplaceChoice::Quit })
    }
}

impl HostLauncher for Quiet {
    fn start(&self, _: &HostPaths) -> std::io::Result<()> {
        Ok(())
    }
}

fn client(user_data: &std::path::Path) -> HostClient {
    HostClient::new(
        user_data,
        Wiring {
            events: Arc::new(Quiet),
            status: Arc::new(Quiet),
            endpoint: Arc::new(HookEndpoint::new()),
            board_tools: Arc::new(Quiet),
            prompt: Arc::new(Quiet),
            launcher: Arc::new(Quiet),
        },
    )
}

/// Answers the client's hello with `welcome`, as a pipe someone else made would.
async fn answer(mut pipe: NamedPipeServer, welcome: Value) -> NamedPipeServer {
    pipe.connect().await.unwrap();
    let mut reader = MessageReader::new();
    let mut buffer = vec![0u8; 4096];
    loop {
        let n = pipe.read(&mut buffer).await.unwrap();
        if reader.feed(&buffer[..n]).iter().any(|m| m["type"] == "hello") {
            break;
        }
    }
    let line = nodus_domain::json::to_string(&welcome).unwrap() + "\n";
    pipe.write_all(line.as_bytes()).await.unwrap();
    pipe
}

fn welcome(key: Option<&str>) -> Value {
    let mut welcome = json!({"type": "welcome", "protocol": PROTOCOL, "hookPort": 1, "hookToken": "t", "sessions": []});
    if let Some(key) = key {
        welcome["key"] = json!(key);
    }
    welcome
}

#[tokio::test]
async fn host_098_a_pipe_that_cant_show_the_hosts_key_isnt_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let user_data = dir.path().join("ud");
    let paths = host_paths(&user_data, Platform::Windows);
    nodus_host_proto::ensure_private_dir(&paths.dir).unwrap();
    nodus_host_proto::write_private_file(&paths.key, "the-real-key").unwrap();

    for key in [None, Some(""), Some("a-guess")] {
        let pipe = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&paths.socket)
            .unwrap();
        let server = tokio::spawn(answer(pipe, welcome(key)));
        let result = tokio::time::timeout(Duration::from_secs(15), client(&user_data).connect_host(false))
            .await
            .unwrap();
        assert_eq!(result, Err(ClientError::Failed(UNTRUSTED.into())), "{key:?}");
        drop(server.await.unwrap());
    }

    // The key the host wrote is trusted.
    let pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&paths.socket)
        .unwrap();
    let server = tokio::spawn(answer(pipe, welcome(Some("the-real-key"))));
    let client = client(&user_data);
    tokio::time::timeout(Duration::from_secs(15), client.connect_host(false))
        .await
        .unwrap()
        .unwrap();
    assert!(client.live_session_ids().is_empty());
    client.disconnect();
    drop(server.await.unwrap());
}
