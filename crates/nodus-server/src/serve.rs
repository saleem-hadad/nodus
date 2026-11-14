//! `nodus-server serve`: the core with no window (SRV-002), on the
//! network (SRV-020), until the service stops it.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use nodus_core::{EnvSource, HostMode, Nodus, NodusConfig, RunnerSource, SafeStorage};
use nodus_domain::Event;
use nodus_host::Logger;
use nodus_updater::{BoxFuture, HttpResponse, Platform, RunOutput, System};
use tokio::net::TcpListener;

use crate::auth::Auth;
use crate::data::{self, ServerEnv};
use crate::hosting::ServerHosting;
use crate::http;
use crate::net::{self, Hosts, Tailscale};
use crate::presence::Presences;
use crate::terminals::Terminals;

/// What `serve` was asked.
pub(crate) struct ServeOptions {
    /// `--bind`: another address to listen on (SRV-020).
    pub bind: Option<String>,
    /// `--dev-token`: a token that gets in as no member, for development and
    /// tests only. Refused unless the server listens on loopback: anywhere
    /// else, people sign in with GitHub (SRV-041).
    pub dev_token: Option<String>,
    /// `--run-home`: the sessions' home, where claude keeps transcripts.
    pub run_home: Option<PathBuf>,
    /// `--host-socket`: the session host `nodus-host.service` runs as
    /// `nodus-run`, to connect to instead of starting one (SRV-004).
    pub host_socket: Option<PathBuf>,
    /// `--uploads`: where uploads are kept (SRV-163), a folder sessions can
    /// read, which the data folder isn't: `install-server.sh` passes
    /// `/srv/nodus/uploads` (2750, `nodus:nodus-run`). Without it they go in
    /// `<data>/uploads`, for development and tests run as one user.
    pub uploads: Option<PathBuf>,
}

/// What every connection shares.
pub(crate) struct Server {
    pub nodus: Nodus,
    /// The data folder (SRV-003).
    pub data: PathBuf,
    /// Where uploads are kept (SRV-163): `--uploads`, else `<data>/uploads`.
    pub uploads: PathBuf,
    /// Members, devices, tokens, the GitHub App and the audit log.
    pub auth: Arc<Auth>,
    /// Sessions' tokens, commit identities and installation tokens, and
    /// the repositories projects come from (SRV-080, SRV-120, SRV-143).
    pub hosting: Arc<ServerHosting>,
    /// The sessions' home: projects are cloned into its `projects/`
    /// (SRV-003).
    pub run_home: PathBuf,
    pub hosts: Hosts,
    /// Who is on which board (SRV-180).
    pub presences: Arc<Presences>,
    /// Whose size each session's terminal follows (SRV-144).
    pub terminals: Terminals,
    /// Updating the server from the app (SRV-200).
    pub updates: crate::update::Updates,
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

/// A session's process ended: its next one starts at its opener's
/// terminal size (SRV-144).
async fn follow_exits(server: std::sync::Weak<Server>, mut events: tokio::sync::broadcast::Receiver<Event>) {
    loop {
        match events.recv().await {
            Ok(Event::PtyExit { session_id, .. }) => match server.upgrade() {
                Some(server) => server.terminals.exited(&session_id),
                None => return,
            },
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
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
    let mut env = data::server_env(&data.join("env"), options.run_home.as_deref())?;
    if data::strip_pool_token(&mut env) {
        log.log(&["warning", data::POOL_TOKEN_IGNORED]);
    }
    data::add_session_env(&mut env);
    let tailscale = Tailscale::detect().await;
    let address = net::bind_address(options.bind.as_deref(), tailscale.ip)?;
    if options.dev_token.is_some() && !address.ip().is_loopback() {
        return Err(format!(
            "--dev-token is for development on loopback only, and {} isn't loopback: sign in with GitHub instead",
            address.ip()
        ));
    }
    let auth = Arc::new(Auth::open(&data, options.dev_token, machine_name())?);
    let hosting = Arc::new(ServerHosting::open(&data, Arc::clone(&auth))?);
    let listener = TcpListener::bind(address)
        .await
        .map_err(|err| format!("Can't listen on {address}: {err}"))?;
    let address = listener.local_addr().map_err(|err| err.to_string())?;
    let run_home = env.home.clone();
    let uploads = options.uploads.unwrap_or_else(|| crate::files::uploads_dir(&data));
    let nodus = start_core(&data, env, options.host_socket, Arc::clone(&hosting) as _, &uploads).await?;
    // SRV-104: the pool moves sessions and frees accounts from now on.
    tokio::spawn(crate::pool::run(Arc::clone(&hosting.pool), nodus.clone()));

    let updates = crate::update::Updates::new(auth.github.api());
    let server = Arc::new(Server {
        nodus: nodus.clone(),
        uploads,
        data: data.clone(),
        auth,
        hosting,
        run_home,
        hosts: Hosts::new(&tailscale, address.ip()),
        presences: Presences::start(),
        terminals: Terminals::default(),
        updates,
        sessions: Mutex::new(HashMap::new()),
    });
    tokio::spawn(follow_exits(Arc::downgrade(&server), nodus.subscribe()));
    // Tailscale may come up after the server at boot: it's looked for
    // until it's there, and listened on then too.
    let late_tailscale = (options.bind.is_none() && tailscale.ip.is_none())
        .then(|| tokio::spawn(listen_on_tailscale_once_up(Arc::clone(&server), log.clone())));
    log.log(&["data in", &data.to_string_lossy()]);
    if server.auth.needs_setup() {
        log.log(&["not set up yet", "run nodus-server setup-code for a setup code"]);
    }
    log.log(&["listening", &format!("http://{address}")]);
    tokio::select! {
        () = http::accept(Arc::clone(&server), listener) => {}
        () = stopped() => {}
        // SRV-200: the new version is in place; systemd starts it.
        () = server.updates.restart.notified() => log.log(&["restarting", "for the new version"]),
    }
    if let Some(late) = late_tailscale {
        late.abort();
    }
    // Sessions keep running in the host, to be adopted when the server
    // starts again (SRV-004, PLAT-033).
    log.log(&["stopping"]);
    nodus.shutdown();
    Ok(())
}

/// SRV-002: the core with the server's own environment instead of a login
/// shell's (SRV-141); no window or dialogs; no notifications, since a
/// session's status reaches the apps as events, and each app tells only the
/// card's starter (SRV-185); the voice key nowhere to be kept, and the app's
/// updater off; and the server's own part (`hosting`): sessions' tokens,
/// git through the App and a worktree per card. Then the sessions a running
/// host has are adopted, without starting one.
async fn start_core(
    data: &Path,
    env: ServerEnv,
    host_socket: Option<PathBuf>,
    hosting: Arc<dyn nodus_core::Hosting>,
    uploads: &Path,
) -> Result<Nodus, String> {
    let ServerEnv { vars, home } = env;
    // Never `with_vars`, which asks `claude` its version: the `claude` it
    // finds is `nodus-run`'s, which a session can replace, and this is
    // `nodus` (SRV-005).
    let env = nodus_platform::Env::given(vars, Some(&home));
    let (host, runner) = host_mode(host_socket);
    let nodus = Nodus::start(NodusConfig {
        env: EnvSource::Given(env),
        home,
        host,
        runner,
        updates: Some(Arc::new(NoAppUpdates)),
        secrets: Some(Arc::new(SafeStorage::unavailable())),
        hosting: Some(hosting),
        // Pictures put up on a server board are its uploads (SRV-163).
        picture_dirs: vec![uploads.to_owned()],
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

/// How often a server that started before Tailscale looks for it again.
const TAILSCALE_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// A server started on loopback because tailscaled wasn't up yet, as at
/// boot, looks every few seconds until Tailscale has an address, then
/// listens there too (port 7420) and answers to its names, without a
/// restart. Loopback stays, for an SSH tunnel.
async fn listen_on_tailscale_once_up(server: Arc<Server>, log: Logger) {
    loop {
        tokio::time::sleep(TAILSCALE_RETRY).await;
        let tailscale = Tailscale::detect().await;
        let Some(ip) = tailscale.ip else { continue };
        let address = std::net::SocketAddr::new(std::net::IpAddr::V4(ip), nodus_wire::PORT);
        match TcpListener::bind(address).await {
            Ok(listener) => {
                server.hosts.add(&tailscale, address.ip());
                log.log(&["listening", &format!("http://{address}")]);
                http::accept(server, listener).await;
                return;
            }
            Err(err) => log.log(&["can't listen on Tailscale yet", &format!("{address}: {err}")]),
        }
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
