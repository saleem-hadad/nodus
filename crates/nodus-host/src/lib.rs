//! The session host: a background process that owns the sessions'
//! pseudo-terminals, so they keep running after the app quits.
//!
//! It keeps each session's recent output and everything that moved its
//! status, so a relaunched app gets them all back. It also serves the hook
//! endpoint every claude process posts to, and relays the board tools' MCP
//! requests to the app, since both have to outlive the app as the sessions
//! do. It exits once it has no sessions and no app connected.
//!
//! The app starts it detached as `Nodus host <socket> <key>` and talks to it
//! over a Unix socket, or a named pipe on Windows (`nodus-host-proto`). It
//! speaks the same protocol as the 0.9.4 Node host, so either app can adopt
//! either host.
//!
//! Ports `src/main/host.ts`.

mod host;
mod http;
mod log;
pub mod pty;
pub mod transport;

use std::path::Path;
use std::sync::Arc;

pub use log::Logger;
use tokio::task::JoinSet;

/// `Nodus host <socket> <key>`: runs the host in this process until it
/// exits, and returns the exit code. Logs go to standard output, which the
/// app points at `host.log`. Nothing here touches the UI, so the host shows
/// no Dock icon, window or menu. (HOST-035)
pub fn main(args: &[String]) -> i32 {
    let log = Logger::stdout();
    log_panics(&log);
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            log.log(&["failed to start", &format!("Error: {err}")]);
            return 1;
        }
    };
    let code = runtime.block_on(run(args, log));
    // As process.exit: whatever is still running goes with the process.
    runtime.shutdown_background();
    code
}

/// Runs a host with these arguments (the socket path and the key path)
/// until it decides to exit; returns its exit code. Once it returns, the
/// host stops listening and its sessions' terminals are closed.
pub async fn run(args: &[String], log: Logger) -> i32 {
    let arg = |i: usize| args.get(i).filter(|arg| !arg.is_empty());
    // HOST-022, HOST-023
    let (Some(socket), key) = (arg(0), arg(1)) else {
        log.log(&["failed to start", "Error: usage: host <socket path> <key path>"]);
        return 1;
    };
    if cfg!(windows) && key.is_none() {
        log.log(&["failed to start", "Error: usage: host <socket path> <key path>"]);
        return 1;
    }
    let socket = Path::new(socket);

    // HOST-030, HOST-029
    let hook_token = random_hex(16);
    let host_key = cfg!(windows).then(|| random_hex(32));
    let host = host::Host::new(log.clone(), hook_token, host_key);

    // HOST-024: two apps starting at once could each start a host; the second one leaves.
    if transport::already_served(socket).await {
        log.log(&["another host is running"]);
        return 0;
    }
    transport::remove_stale(socket);

    let failed = |err: &dyn std::fmt::Display| {
        log.log(&["failed to start", &format!("Error: {err}")]);
        1
    };
    // HOST-026: the hook server first.
    let (hooks, port) = match http::bind().await {
        Ok(bound) => bound,
        Err(err) => return failed(&err),
    };
    host.hook_port
        .store(u64::from(port), std::sync::atomic::Ordering::SeqCst);
    // HOST-027, HOST-028
    let mut listener = match transport::Listener::bind(socket) {
        Ok(listener) => listener,
        Err(err) => return failed(&err),
    };
    // HOST-029: before any welcome goes out, and only once the pipe is ours:
    // a second host that lost the race to it never gets here.
    if let (Some(key), Some(key_path)) = (&host.host_key, key)
        && let Err(err) = nodus_host_proto::write_private_file(Path::new(key_path), key)
    {
        return failed(&err);
    }
    // HOST-031
    log.log(&["listening", &socket.to_string_lossy(), "hooks on", &port.to_string()]);

    let mut tasks = JoinSet::new();
    tasks.spawn(http::serve(Arc::clone(&host), hooks));
    let serving = Arc::clone(&host);
    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    connections.spawn(Arc::clone(&serving).serve(stream));
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
            while connections.try_join_next().is_some() {}
        }
    });
    let unused = Arc::clone(&host);
    tasks.spawn(async move {
        tokio::time::sleep(host::UNUSED).await;
        unused.unused_check();
    });

    let code = host.exited().await;
    tasks.shutdown().await;
    host.close_sessions();
    code
}

/// HOST-033: a crash would take every session with it, so a panic anywhere
/// in this process is logged as `uncaught Error: <message>` and the host
/// carries on (tokio keeps a panicking task to itself). [`main`] installs it
/// before the host starts.
pub fn log_panics(log: &Logger) {
    let panics = log.clone();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        panics.log(&["uncaught", &format!("Error: {message}")]);
    }));
}

/// `crypto.randomBytes(n).toString('hex')`.
fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the system has randomness");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_tokens_of_lowercase_hex() {
        let token = random_hex(16);
        assert_eq!(token.len(), 32);
        assert!(token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_ne!(token, random_hex(16));
        assert_eq!(random_hex(32).len(), 64);
    }
}
