//! What the updater needs from the app and from the OS, as two traits so
//! tests can stand in for either: [`App`] (nodus-core: settings, sessions,
//! quitting) and [`System`] (the OS, GitHub and the tools the updater runs;
//! [`RealSystem`](crate::RealSystem) is the real one).

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// A boxed future, so the traits work behind `Arc<dyn …>`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The OS the updater is running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Other,
}

impl Platform {
    /// The OS this was built for.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// The app around the updater (nodus-core implements it).
pub trait App: Send + Sync + 'static {
    /// The running version (`app.getVersion()`).
    fn version(&self) -> String;
    /// The userData folder (01).
    fn user_data(&self) -> PathBuf;
    /// `settings.autoUpdate`, read at each automatic check.
    fn auto_update(&self) -> bool;
    /// How many sessions are live (02 `liveSessionIds().length`).
    fn live_session_count(&self) -> usize;
    /// Stops every session and the session host (02 `stopAll()`).
    fn stop_all(&self) -> BoxFuture<'_, Result<(), String>>;
    /// Quits normally, as `app.quit()` does: windows close, the quit path
    /// runs (which calls [`Updater::on_will_quit`](crate::Updater::on_will_quit)
    /// on macOS) and the process exits.
    fn quit(&self);
    /// `process.env.NODUS_USER_DATA`, passed on to the relaunched copy.
    fn user_data_override(&self) -> Option<String> {
        std::env::var("NODUS_USER_DATA").ok()
    }
}

/// An HTTP answer: its status, and its body as it arrives.
pub struct HttpResponse {
    pub status: u16,
    /// None when the answer came without a body.
    pub body: Option<Box<dyn Body>>,
}

/// A response body, chunk by chunk.
pub trait Body: Send {
    /// The next chunk; None at the end.
    fn next_chunk(&mut self) -> BoxFuture<'_, Option<io::Result<Vec<u8>>>>;
}

/// What a finished `execFile` gives back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
}

/// The OS, GitHub and the tools the updater runs.
pub trait System: Send + Sync + 'static {
    fn platform(&self) -> Platform;
    /// Electron's `app.isPackaged`.
    fn is_packaged(&self) -> bool;
    /// An arm64 build, or an x64 one running translated (UPD-021).
    fn arm64_machine(&self) -> bool;
    /// The running `.app` on macOS: `<exe>/../../..`.
    fn bundle_path(&self) -> PathBuf;
    /// Electron's `app.isInApplicationsFolder()`.
    fn is_in_applications_folder(&self) -> bool;
    /// `fs.accessSync(path, W_OK)` succeeds.
    fn writable(&self, path: &Path) -> bool {
        writable(path)
    }
    /// This process's id, for the swap script to wait on.
    fn pid(&self) -> u32 {
        std::process::id()
    }
    /// `GET url` with these headers, following redirects. Err when no answer
    /// came (no connection, TLS…). The caller applies its own timeouts.
    fn get(&self, url: &str, headers: Vec<(&'static str, String)>) -> BoxFuture<'_, io::Result<HttpResponse>>;
    /// Node's `execFile(program, args)`: Err (with the message) when it
    /// doesn't start, exits non-zero or is stopped.
    fn run(&self, program: &str, args: Vec<String>) -> BoxFuture<'_, Result<RunOutput, String>>;
    /// Node's `spawn(program, args, { detached: true, stdio: 'ignore' })`:
    /// Ok once the OS has started it (the `spawn` event), Err otherwise.
    fn spawn_detached(&self, program: &Path, args: Vec<String>) -> io::Result<()>;
}

/// Whether the current user may write to `path` (`access(path, W_OK)`).
pub fn writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok()
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(path).is_ok_and(|meta| !meta.permissions().readonly())
    }
}
