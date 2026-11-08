//! The socket the app and the host talk over: a Unix socket on macOS, a
//! named pipe on Windows. Both ends are here, so the app's client
//! (`nodus-sessions`) connects the way the host listens.

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

/// A connection to the host, as the app makes it.
#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Anything a connection can be read from and written to.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Duplex for T {}

/// Connects to the host at `socket` (`net.createConnection(socket)`).
#[cfg(unix)]
pub async fn connect(socket: &Path) -> io::Result<ClientStream> {
    tokio::net::UnixStream::connect(socket).await
}

/// Connects to the host's pipe, waiting while every instance is busy as
/// libuv does (up to 30 s).
#[cfg(windows)]
pub async fn connect(socket: &Path) -> io::Result<ClientStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match ClientOptions::new().open(socket.as_os_str()) {
            Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY) && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

/// The host's end: accepts the app's connections.
#[cfg(unix)]
pub(crate) struct Listener(tokio::net::UnixListener);

#[cfg(unix)]
impl Listener {
    /// Listens at `socket`, readable and writable only by the user. (HOST-027, HOST-028)
    pub(crate) fn bind(socket: &Path) -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let listener = tokio::net::UnixListener::bind(socket)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self(listener))
    }

    pub(crate) async fn accept(&mut self) -> io::Result<tokio::net::UnixStream> {
        self.0.accept().await.map(|(stream, _)| stream)
    }
}

/// The host's end: one pipe instance waiting for the next app at a time.
/// The first is made with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a second host
/// can't share the name; default security keeps other users out. (HOST-027)
#[cfg(windows)]
pub(crate) struct Listener {
    name: std::ffi::OsString,
    next: tokio::net::windows::named_pipe::NamedPipeServer,
}

#[cfg(windows)]
impl Listener {
    pub(crate) fn bind(socket: &Path) -> io::Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = socket.as_os_str().to_owned();
        let next = ServerOptions::new().first_pipe_instance(true).create(&name)?;
        Ok(Self { name, next })
    }

    pub(crate) async fn accept(&mut self) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        use tokio::net::windows::named_pipe::ServerOptions;
        self.next.connect().await?;
        let waiting = ServerOptions::new().create(&self.name)?;
        Ok(std::mem::replace(&mut self.next, waiting))
    }
}

/// Whether another host already serves `socket`: a connection succeeds. The
/// probe sends nothing, so that host doesn't count it. (HOST-024)
pub(crate) async fn already_served(socket: &Path) -> bool {
    connect(socket).await.is_ok()
}

/// Removes a stale socket file left by a host that died, ignoring errors. (HOST-025)
pub(crate) fn remove_stale(socket: &Path) {
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket);
    #[cfg(windows)]
    let _ = socket;
}
