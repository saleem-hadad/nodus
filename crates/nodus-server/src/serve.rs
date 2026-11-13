//! `nodus-server serve`: the core with no window (SRV-002), on the
//! network (SRV-020), until the service stops it.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use nodus_core::{EnvSource, HostMode, Nodus, NodusConfig, RunnerSource, SafeStorage};
use nodus_host::Logger;
use nodus_updater::{BoxFuture, HttpResponse, Platform, RunOutput, System};
use nodus_wire::{Hello, PROTOCOL};
use tokio::net::TcpListener;

use crate::data::{self, ServerEnv};
use crate::http;
use crate::net::{self, Hosts, Tailscale};

/// What `serve` was asked.
pub(crate) struct ServeOptions {
    /// `--bind`: another address to listen on (SRV-020).
    pub bind: Option<String>,
    /// `--dev-token`: the token every connection carries. A random one,
    /// printed, when none is given. Until people sign in (slice 3).
    pub dev_token: Option<String>,
    /// `--run-home`: the sessions' home, where claude keeps transcripts.
    pub run_home: Option<PathBuf>,
    /// `--host-socket`: the session host `nodus-host.service` runs as
    /// `nodus-run`, to connect to instead of starting one (SRV-004).
    pub host_socket: Option<PathBuf>,
}

/// What every connection shares.
pub(crate) struct Server {
    pub nodus: Nodus,
    pub token: String,
    pub hosts: Hosts,
    pub hello: Hello,
    /// The data folder: uploads are kept in it (SRV-163).
    pub data: PathBuf,
    /// Each session's project, once looked up: a session never moves.
    sessions: Mutex<HashMap<String, String>>,
}

impl Server {
    /// The project whose board has the session (SRV-026).
    pub(crate) fn session_project(&self, session_id: &str) -> Option<String> {
        let mut known = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(project_id) = known.get(session_id) {
            return Some(project_id.clone());
        }
        let project_id = self.nodus.board().session_project(session_id)?;
        known.insert(session_id.to_owned(), project_id.clone());
        Some(project_id)
    }
}

/// Runs the server until it's stopped (SIGTERM or SIGINT), logging to
/// standard output as the host does, and gives the exit code.
pub(crate) fn serve(options: ServeOptions) -> i32 {
    let log = Logger::stdout();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            log.log(&["failed to start", &format!("Error: {err}")]);
            return 1;
        }
    };
    let code = runtime.block_on(async {
        match run(options, &log).await {
            Ok(()) => 0,
            Err(err) => {
                log.log(&["failed to start", &format!("Error: {err}")]);
                1
            }
        }
    });
    runtime.shutdown_background();
    code
}

async fn run(options: ServeOptions, log: &Logger) -> Result<(), String> {
    let data = data::data_dir().ok_or("No data folder: set NODUS_USER_DATA or HOME")?;
    nodus_host_proto::ensure_private_dir(&data).map_err(|err| format!("Can't make {}: {err}", data.display()))?;
    let _lock = data::lock(&data)?;
    let env = data::server_env(&data.join("env"), options.run_home.as_deref())?;
    let tailscale = Tailscale::detect().await;
    let address = net::bind_address(options.bind.as_deref(), tailscale.ip)?;
    let listener = TcpListener::bind(address)
        .await
        .map_err(|err| format!("Can't listen on {address}: {err}"))?;
    let address = listener.local_addr().map_err(|err| err.to_string())?;
    let nodus = start_core(&data, env, options.host_socket).await?;

    let generated = options.dev_token.is_none();
    let token = options.dev_token.unwrap_or_else(random_token);
    let server = Arc::new(Server {
        nodus: nodus.clone(),
        token,
        hosts: Hosts::new(&tailscale, address.ip()),
        hello: Hello {
            name: machine_name(),
            version: nodus_core::VERSION.to_owned(),
            protocol: PROTOCOL,
            needs_setup: false,
        },
        data: data.clone(),
        sessions: Mutex::new(HashMap::new()),
    });
    log.log(&["data in", &data.to_string_lossy()]);
    if generated {
        log.log(&["dev token", &server.token]);
    }
    log.log(&["listening", &format!("http://{address}")]);
    tokio::select! {
        () = http::accept(Arc::clone(&server), listener) => {}
        () = stopped() => {}
    }
    // Sessions keep running in the host, to be adopted when the server
    // starts again (SRV-004, PLAT-033).
    log.log(&["stopping"]);
    nodus.shutdown();
    Ok(())
}

/// SRV-002: the core with the server's own environment instead of a login
/// shell's (SRV-141); no window or dialogs; no notifications, since a
/// session's status reaches the apps as events whatever the notifier does
/// (SRV-185 comes later); the voice key nowhere to be kept, and the app's
/// updater off. Then the sessions a running host has are adopted, without
/// starting one.
async fn start_core(data: &Path, env: ServerEnv, host_socket: Option<PathBuf>) -> Result<Nodus, String> {
    let ServerEnv { vars, home } = env;
    let env = nodus_platform::Env::with_vars(vars, Some(&home)).await;
    let (host, runner) = host_mode(host_socket);
    let nodus = Nodus::start(NodusConfig {
        env: EnvSource::Given(env),
        home,
        host,
        runner,
        updates: Some(Arc::new(NoAppUpdates)),
        secrets: Some(Arc::new(SafeStorage::unavailable())),
        ..NodusConfig::new(data, tokio::runtime::Handle::current())
    })
    .map_err(|err| format!("Can't open the data in {}: {err}", data.display()))?;
    // A headless shell never chooses to quit over another version's sessions.
    let _ = nodus.launch().await;
    Ok(nodus)
}

/// How the core reaches the session host, and where it runs every other
/// process: the one place that decides.
///
/// With `--host-socket`, as on a server (SRV-004, SRV-005): `serve` never
/// starts the host. It connects to the one `nodus-host.service` runs as
/// `nodus-run`, retrying every second until it's up, and git, the
/// background `claude -p` calls and views' commands run there too, so
/// nothing in a project folder runs as `nodus`.
///
/// Without it, for development and tests on one user: as the app does, the
/// core starts this binary as `nodus-server host <socket> <key>`, detached,
/// so sessions outlive a restart of `serve` (HOST-091), and runs the rest
/// itself.
fn host_mode(host_socket: Option<PathBuf>) -> (HostMode, RunnerSource) {
    match host_socket {
        Some(socket) => (HostMode::Connect(socket), RunnerSource::Host),
        None => (HostMode::Launch, RunnerSource::Local),
    }
}

/// The app's own updates, off: a server is updated from the app (SRV-200),
/// and swapping a bundle on quit means nothing here (SRV-002).
struct NoAppUpdates;

impl System for NoAppUpdates {
    fn platform(&self) -> Platform {
        Platform::Other
    }
    fn is_packaged(&self) -> bool {
        false
    }
    fn arm64_machine(&self) -> bool {
        false
    }
    fn bundle_path(&self) -> PathBuf {
        PathBuf::new()
    }
    fn is_in_applications_folder(&self) -> bool {
        false
    }
    fn get(&self, _: &str, _: Vec<(&'static str, String)>) -> BoxFuture<'_, io::Result<HttpResponse>> {
        Box::pin(async { Err(io::Error::other("no app updates on a server")) })
    }
    fn run(&self, _: &str, _: Vec<String>) -> BoxFuture<'_, Result<RunOutput, String>> {
        Box::pin(async { Err("no app updates on a server".to_owned()) })
    }
    fn spawn_detached(&self, _: &Path, _: Vec<String>) -> io::Result<()> {
        Err(io::Error::other("no app updates on a server"))
    }
}

/// When the service is told to stop.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            }
            _ => std::future::pending().await,
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// 32 random bytes as hex.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The machine's name without its domain, until the server is named in
/// setup (SRV-061).
fn machine_name() -> String {
    #[cfg(unix)]
    {
        let mut name = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname writes at
        // most that much, and the name is read only up to its NUL.
        let ok = unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } == 0;
        let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let name = String::from_utf8_lossy(&name[..len]);
        let short = name.split('.').next().unwrap_or("");
        if ok && !short.is_empty() {
            return short.to_owned();
        }
    }
    #[cfg(not(unix))]
    if let Ok(name) = std::env::var("COMPUTERNAME")
        && !name.is_empty()
    {
        return name;
    }
    "Nodus".to_owned()
}
