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
//! over a Unix socket, or a named pipe on Windows (`nodus-host-proto`). On a
//! server it runs on its own as `nodus host --socket <path>`, as another
//! user than `nodus serve`, which connects to it and has it run every
//! process (SRV-004, SRV-005). It
//! sends the same messages as the 0.9.4 Node host under protocol number 2
//! (0.9.4 says 1, decision U1), so an app of either kind that finds the other
//! kind's host asks to stop its sessions rather than adopting them.
//!
//! Ports `src/main/host.ts`.

pub mod exec;
mod host;
mod http;
mod log;
pub mod pty;
pub mod transport;
pub mod trust;

use std::path::Path;
use std::sync::Arc;

pub use log::Logger;
use tokio::task::JoinSet;

/// `Nodus host <socket> <key>`, or `nodus host --socket <path>`: runs the
/// host in this process until it exits, and returns the exit code. Logs go
/// to standard output, which the app points at `host.log` (and systemd at
/// its journal). Nothing here touches the UI, so the host shows no Dock
/// icon, window or menu. (HOST-035)
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

/// Runs a host with these arguments (the socket path and the key path, or
/// `--socket` and the socket path) until it decides to exit; returns its
/// exit code. Once it returns, the host stops listening and its sessions'
/// terminals are closed.
pub async fn run(args: &[String], log: Logger) -> i32 {
    let arg = |i: usize| args.get(i).filter(|arg| !arg.is_empty());
    // SRV-004: on its own, at a socket its app's user can reach through its
    // group. Unix only: a Windows pipe has no group, and its app needs a key.
    if args.first().map(String::as_str) == Some("--socket") {
        let Some(socket) = arg(1).filter(|_| cfg!(unix)) else {
            log.log(&["failed to start", "Error: usage: host --socket <socket path>"]);
            return 1;
        };
        return serve(Path::new(socket), None, true, log).await;
    }
    // HOST-022, HOST-023
    let (Some(socket), key) = (arg(0), arg(1)) else {
        log.log(&["failed to start", "Error: usage: host <socket path> <key path>"]);
        return 1;
    };
    if cfg!(windows) && key.is_none() {
        log.log(&["failed to start", "Error: usage: host <socket path> <key path>"]);
        return 1;
    }
    serve(Path::new(socket), key.map(Path::new), false, log).await
}

/// The host at `socket`, the app's own (HOST-022…035) or standalone
/// (SRV-004): that one makes the socket's folder when it's missing (0750),
/// lets the socket's group connect (0660), never gives up waiting for an
/// app (HOST-032, HOST-047), and leaves when an app on another protocol
/// says hello.
async fn serve(socket: &Path, key: Option<&Path>, standalone: bool, log: Logger) -> i32 {
    // HOST-030, HOST-029
    let hook_token = random_hex(16);
    let host_key = cfg!(windows).then(|| random_hex(32));
    // SRV-143: sessions with tokens of their own keep them beside the socket.
    let hook_configs = socket.parent().map(|dir| dir.join("hooks"));
    let host = host::Host::new(log.clone(), hook_token, host_key, standalone, hook_configs);

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
    if standalone {
        // Never somewhere else, as the app's own host goes to the temp folder
        // (HOST-004): on Linux that's every user's.
        if let Some(most) = too_long(socket) {
            return failed(&format!(
                "the socket path {} is longer than {most} bytes",
                socket.display()
            ));
        }
        // SRV-004: systemd's RuntimeDirectory makes it; anywhere else, this does.
        if let Err(err) = make_socket_dir(socket) {
            return failed(&err);
        }
    }
    // HOST-026: the hook server first.
    let (hooks, port) = match http::bind().await {
        Ok(bound) => bound,
        Err(err) => return failed(&err),
    };
    host.hook_port
        .store(u64::from(port), std::sync::atomic::Ordering::SeqCst);
    // HOST-027, HOST-028
    let mut listener = match transport::Listener::bind(socket, standalone) {
        Ok(listener) => listener,
        Err(err) => return failed(&err),
    };
    // HOST-029: before any welcome goes out, and only once the pipe is ours:
    // a second host that lost the race to it never gets here.
    if let (Some(key), Some(key_path)) = (&host.host_key, key)
        && let Err(err) = nodus_host_proto::write_private_file(key_path, key)
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
    if !standalone {
        let unused = Arc::clone(&host);
        tasks.spawn(async move {
            tokio::time::sleep(host::UNUSED).await;
            unused.unused_check();
        });
    }

    let code = host.exited().await;
    tasks.shutdown().await;
    host.close_sessions();
    code
}

/// The longest a socket's path can be, when `socket` is longer: what fits
/// in `sockaddr_un.sun_path` with its NUL (108 bytes on Linux, 104 on macOS).
fn too_long(socket: &Path) -> Option<usize> {
    let most = if cfg!(target_os = "linux") { 107 } else { 103 };
    (socket.as_os_str().len() > most).then_some(most)
}

/// The socket's folder, and any missing above it, made 0750 when missing.
fn make_socket_dir(socket: &Path) -> std::io::Result<()> {
    let Some(dir) = socket.parent().filter(|dir| !dir.as_os_str().is_empty()) else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o750).create(dir)
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
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
