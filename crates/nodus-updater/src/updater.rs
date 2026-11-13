//! The updater itself: `src/main/updater.ts`, behavior for behavior.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use nodus_domain::{Event, EventSink, UpdateState, UpdateStatus};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior, timeout};

use crate::env::{App, Platform, System};
use crate::release::{
    LATEST_URL, Release, asset_name, is_newer, js_trim, newest_installer, parse_release, team_identifier,
};

/// A little after launch, so the first check never competes with opening the board.
pub const FIRST_CHECK: Duration = Duration::from_secs(30);
pub const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);
/// The lookup, its body included.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);
/// A download that sends nothing for this long is given up.
pub const STALL: Duration = Duration::from_secs(60);

/// UPD-061/062: what the swap script runs once the app has exited, verbatim
/// (the JavaScript template literal starts and ends with a newline).
pub const SWAP_SCRIPT: &str = r#"
trap '' HUP INT TERM
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
app=$2 new=$3 old=$4
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    rm -rf "$app"
    mv "$old" "$app"
    echo "$6" > "$5"
  fi
else
  echo "$6" > "$5"
fi
[ -n "$7" ] || exit 0
if [ -n "$8" ]; then open -n --env "NODUS_USER_DATA=$8" "$app"; else open -n "$app"; fi
"#;

/// The installer's arguments: `/S` installs without asking, `--updated` keeps
/// the shortcuts as they are and `--force-run` opens Nodus after.
pub const INSTALLER_ARGS: [&str; 3] = ["--updated", "/S", "--force-run"];

// What Settings shows, verbatim.
pub const OFF_PLATFORM: &str = "Updates are only built for macOS and Windows.";
pub const OFF_DEVELOPMENT: &str = "Development builds don’t update themselves.";
pub const OFF_APPLICATIONS: &str = "Your account can’t change apps in Applications, so Nodus can’t update itself.";
pub const OFF_MOVE: &str = "Move Nodus to your Applications folder so it can update itself.";
pub const UNREACHABLE: &str = "Couldn’t reach GitHub to check for updates.";
pub const RATE_LIMITED: &str = "GitHub is limiting update checks right now. Try again later.";
pub const DAMAGED: &str = "The update came through damaged. Try again later.";
pub const INTERRUPTED: &str = "The update didn’t finish downloading. Check your connection.";
pub const NOT_PREPARED: &str = "The update couldn’t be prepared. Try again later.";
pub const NO_APP: &str = "The update has no app in it.";
pub const BAD_SIGNATURE: &str = "The new version’s signature didn’t check out.";
pub const NOT_EXPECTED: &str = "The update isn’t the expected version of Nodus.";
pub const OTHER_DEVELOPER: &str = "The update isn’t signed by the same developer.";
pub const NO_CHECKSUM: &str = "GitHub had no checksum for the update, so it wasn’t downloaded.";
pub const INSTALLER_FAILED: &str = "The installer didn’t start, so Nodus wasn’t updated. Try Check for Updates again.";
pub const STOP_DETAIL: &str =
    "Windows can’t update Nodus while its sessions run. You can resume them from their cards afterwards.";

/// The new version waiting to go in: its app, copied out of its DMG, or on
/// Windows its installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub version: String,
    pub app: PathBuf,
}

/// UPD-066: what to ask before Windows stops the sessions, as Electron's
/// `dialog.showMessageBox` question (no parent window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopSessionsQuestion {
    pub message: String,
    pub detail: String,
    /// `Stop and Update`, `Not Now`.
    pub buttons: [String; 2],
    /// The button Enter picks: `Stop and Update`.
    pub default_id: usize,
    /// The button Escape or closing picks: `Not Now`.
    pub cancel_id: usize,
}

impl StopSessionsQuestion {
    pub fn new(running: usize) -> Self {
        let sessions = if running == 1 {
            "the running session".to_owned()
        } else {
            format!("{running} running sessions")
        };
        Self {
            message: format!("Stop {sessions} to update?"),
            detail: STOP_DETAIL.to_owned(),
            buttons: ["Stop and Update".to_owned(), "Not Now".to_owned()],
            default_id: 0,
            cancel_id: 1,
        }
    }
}

/// What Restart to Update did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartOutcome {
    /// Nothing is staged, or Restart to Update is already under way (UPD-054, UPD-064).
    Nothing,
    /// The app is quitting into the update.
    Quitting,
    /// Windows with sessions running: the UI asks this, then answers with
    /// [`Updater::answer_stop_sessions`]. Until then a second click asks nothing.
    Ask(StopSessionsQuestion),
    /// `Not Now`: the sessions keep running and the update stays ready (UPD-067).
    NotNow,
    /// The installer didn't start; the status says so (UPD-069).
    Failed,
}

#[derive(Debug)]
struct Inner {
    status: UpdateStatus,
    /// A lookup or download is under way.
    busy: bool,
    staged: Option<Staged>,
    /// A version that failed to install on an earlier quit. It isn't
    /// downloaded again unless the user asks.
    failed_version: Option<String>,
    /// Open the new version once it's swapped in (Restart to Update).
    relaunch: bool,
    /// The swap runs on quit (macOS, updates on).
    swap_on_quit: bool,
    swapping: bool,
    /// Restart to Update is under way on Windows.
    installing: bool,
    /// The stop-the-sessions question is out, waiting for its answer.
    asking: bool,
}

/// Why preparing an update failed.
enum Failure {
    /// A message fit to show in Settings (`UpdateError`).
    Shown(String),
    /// Anything else, logged; Settings says [`NOT_PREPARED`].
    Other(String),
}

impl Failure {
    fn shown(message: &str) -> Self {
        Self::Shown(message.to_owned())
    }

    fn other(err: impl std::fmt::Display) -> Self {
        Self::Other(err.to_string())
    }

    fn message(&self) -> &str {
        match self {
            Self::Shown(message) => message,
            Self::Other(_) => NOT_PREPARED,
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Shown(message) => message,
            Self::Other(detail) => detail,
        }
    }
}

/// Keeps the app up to date from GitHub releases.
pub struct Updater {
    app: Arc<dyn App>,
    system: Arc<dyn System>,
    sink: Arc<dyn EventSink>,
    inner: Mutex<Inner>,
    /// Leftovers from earlier runs are cleared before the first check.
    cleaned: watch::Sender<bool>,
    me: Weak<Updater>,
}

impl std::fmt::Debug for Updater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updater")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl Updater {
    /// UPD-001: idle, with the running version, until [`start`](Self::start).
    pub fn new(app: Arc<dyn App>, system: Arc<dyn System>, sink: Arc<dyn EventSink>) -> Arc<Self> {
        let current = app.version();
        Arc::new_cyclic(|me| Self {
            app,
            system,
            sink,
            inner: Mutex::new(Inner {
                status: UpdateStatus {
                    state: UpdateState::Idle,
                    current,
                },
                busy: false,
                staged: None,
                failed_version: None,
                relaunch: false,
                swap_on_quit: false,
                swapping: false,
                installing: false,
                asking: false,
            }),
            cleaned: watch::channel(true).0,
            me: me.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `updates:status` (UPD-076).
    pub fn status(&self) -> UpdateStatus {
        self.lock().status.clone()
    }

    /// The update waiting to go in, if any.
    pub fn staged(&self) -> Option<Staged> {
        self.lock().staged.clone()
    }

    /// UPD-002: the whole status replaced, with the running version, and sent.
    fn set_state(&self, state: UpdateState) {
        let status = UpdateStatus {
            state,
            current: self.app.version(),
        };
        self.lock().status = status.clone();
        self.sink.emit(Event::UpdateStatus(status));
    }

    fn platform(&self) -> Platform {
        self.system.platform()
    }

    fn updates_dir(&self) -> PathBuf {
        self.app.user_data().join("updates")
    }

    fn failed_file(&self) -> PathBuf {
        self.updates_dir().join("failed")
    }

    fn user_agent(&self) -> String {
        format!("Nodus/{}", self.app.version())
    }

    /// UPD-004…UPD-009: why this copy can't replace itself, or None when it can.
    pub fn why_off(&self) -> Option<&'static str> {
        match self.platform() {
            Platform::Other => return Some(OFF_PLATFORM),
            _ if !self.system.is_packaged() => return Some(OFF_DEVELOPMENT),
            // The installer puts the new version where this one is, in the user's own folder.
            Platform::Windows => return None,
            Platform::Mac => {}
        }
        // The swap renames the app, so it needs write access to the app and to
        // its folder. A copy running from the DMG, or one macOS translocated, has neither.
        let bundle = self.system.bundle_path();
        let parent = bundle.parent().map(Path::to_path_buf).unwrap_or_default();
        if self.system.writable(&bundle) && self.system.writable(&parent) {
            return None;
        }
        Some(if self.system.is_in_applications_folder() {
            OFF_APPLICATIONS
        } else {
            OFF_MOVE
        })
    }

    /// UPD-003, UPD-010…UPD-012: off with a reason, or clean-up now, the swap
    /// on quit (macOS), and the automatic checks: 30 s after this, then every
    /// 4 h from this, while `autoUpdate` is on at that moment.
    pub fn start(&self) {
        if let Some(reason) = self.why_off() {
            self.set_state(UpdateState::Off { reason: reason.into() });
            return;
        }
        self.cleaned.send_replace(false);
        let me = self.me.clone();
        tokio::spawn(async move {
            if let Some(this) = me.upgrade() {
                this.clean_up().await;
                this.cleaned.send_replace(true);
            }
        });
        // On Windows an update would stop the sessions, so it only installs when the user asks.
        if self.platform() == Platform::Mac {
            self.lock().swap_on_quit = true;
        }
        let me = self.me.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            auto_check(&me);
        });
        let me = self.me.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval_at(Instant::now() + CHECK_EVERY, CHECK_EVERY);
            // Node's timers don't catch up in a burst after the machine sleeps.
            every.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if me.strong_count() == 0 {
                    return;
                }
                auto_check(&me);
            }
        });
    }

    /// `check(asked)` (UPD-015…UPD-029): looks for a newer release and
    /// starts preparing it. Resolves once the lookup is done; the download
    /// carries on. `asked` is a check the user started (Check Now, the menu
    /// item), which also retries a version that failed to install before.
    pub async fn check(&self, asked: bool) {
        let _ = self.cleaned.subscribe().wait_for(|done| *done).await;
        {
            let mut inner = self.lock();
            let off = matches!(inner.status.state, UpdateState::Off { .. });
            if inner.busy || inner.staged.is_some() || off {
                return;
            }
            inner.busy = true;
        }
        self.set_state(UpdateState::Checking);
        let release = match self.latest_release().await {
            Ok(release) => release,
            Err(message) => {
                self.lock().busy = false;
                return self.set_state(UpdateState::Error { message });
            }
        };
        let Some(release) = release.filter(|release| is_newer(&release.version, &self.app.version())) else {
            self.lock().busy = false;
            return self.set_state(UpdateState::Latest);
        };
        let failed = self.lock().failed_version.as_deref() == Some(release.version.as_str());
        if failed && !asked {
            self.lock().busy = false;
            let message = format!("Nodus {} downloaded but couldn’t install itself.", release.version);
            return self.set_state(UpdateState::Error { message });
        }
        let Some(this) = self.me.upgrade() else { return };
        tokio::spawn(async move {
            this.prepare(release).await;
            this.lock().busy = false;
        });
    }

    /// The latest release, or None while it has nothing finished for this
    /// machine (UPD-016…UPD-024). Err carries the message for Settings.
    ///
    /// The 20 s timeout covers the body as well as the answer, as
    /// `AbortSignal.timeout` does. 0.9.4 shows the JavaScript error's own
    /// words when it fires during the body, or the body isn't JSON; here the
    /// first is [`UNREACHABLE`] and the second serde's message.
    async fn latest_release(&self) -> Result<Option<Release>, String> {
        let headers = vec![
            ("Accept", "application/vnd.github+json".to_owned()),
            ("User-Agent", self.user_agent()),
        ];
        let lookup = async {
            let response = self
                .system
                .get(LATEST_URL, headers)
                .await
                .map_err(|_| UNREACHABLE.to_owned())?;
            // Calls without a token get 60 an hour per address, which a shared network can use up.
            if matches!(response.status, 403 | 429) {
                return Err(RATE_LIMITED.to_owned());
            }
            if !(200..300).contains(&response.status) {
                return Err(format!(
                    "GitHub answered the update check with an error ({}).",
                    response.status
                ));
            }
            let mut text = Vec::new();
            if let Some(mut body) = response.body {
                while let Some(chunk) = body.next_chunk().await {
                    text.extend(chunk.map_err(|_| UNREACHABLE.to_owned())?);
                }
            }
            let asset = asset_name(self.platform(), self.system.arm64_machine());
            parse_release(&String::from_utf8_lossy(&text), &asset).map_err(|err| err.to_string())
        };
        timeout(LOOKUP_TIMEOUT, lookup)
            .await
            .unwrap_or_else(|_| Err(UNREACHABLE.to_owned()))
    }

    /// Electron's `app.isPackaged`: a development build shows no What's new.
    pub fn packaged(&self) -> bool {
        self.system.is_packaged()
    }

    /// UPD-030…UPD-035: downloads a release to `file`, reporting whole
    /// percents as they change, then checks its size and SHA-256.
    async fn download(&self, release: &Release, file: &Path) -> Result<(), Failure> {
        let interrupted = || Failure::shown(INTERRUPTED);
        let headers = vec![("User-Agent", self.user_agent())];
        let response = timeout(STALL, self.system.get(&release.url, headers))
            .await
            .map_err(|_| interrupted())?
            .map_err(|_| interrupted())?;
        let (true, Some(mut body)) = ((200..300).contains(&response.status), response.body) else {
            let message = format!("GitHub answered the download with an error ({}).", response.status);
            return Err(Failure::Shown(message));
        };
        let mut out = tokio::fs::File::create(file).await.map_err(|_| interrupted())?;
        let mut hash = Sha256::new();
        let mut received: u64 = 0;
        let mut shown = 0u8;
        loop {
            let chunk = match timeout(STALL, body.next_chunk()).await.map_err(|_| interrupted())? {
                None => break,
                Some(chunk) => chunk.map_err(|_| interrupted())?,
            };
            received += chunk.len() as u64;
            hash.update(&chunk);
            let percent = percent_of(received, release.size);
            if percent != shown {
                shown = percent;
                self.set_state(UpdateState::Downloading {
                    version: release.version.clone(),
                    percent,
                });
            }
            out.write_all(&chunk).await.map_err(|_| interrupted())?;
        }
        out.flush().await.map_err(|_| interrupted())?;
        drop(out);
        let digest = hex(&hash.finalize());
        if received != release.size || release.sha256.as_ref().is_some_and(|sha| *sha != digest) {
            return Err(Failure::shown(DAMAGED));
        }
        Ok(())
    }

    /// Deletes a file or folder: `/bin/rm -rf` on macOS (as 0.9.4, whose fs
    /// couldn't delete an app's `app.asar`), a recursive forced removal on
    /// Windows.
    async fn remove(&self, target: &Path) -> Result<(), String> {
        if self.platform() == Platform::Windows {
            return remove_all(target).map_err(|err| err.to_string());
        }
        self.system
            .run("/bin/rm", vec!["-rf".into(), path_arg(target)])
            .await
            .map(drop)
    }

    /// UPD-037…UPD-053: downloads a release and stages it.
    async fn prepare(&self, release: Release) {
        if self.platform() == Platform::Windows {
            return self.prepare_installer(release).await;
        }
        let updates = self.updates_dir();
        let dir = updates.join(&release.version);
        let dmg = updates.join(format!("Nodus-{}.dmg", release.version));
        let target = dir.join("Nodus.app");
        self.downloading(&release, 0);
        let prepared = async {
            self.remove(&dir).await.map_err(Failure::Other)?;
            std::fs::create_dir_all(&dir).map_err(Failure::other)?;
            self.download(&release, &dmg).await?;
            self.copy_app(&dmg, &target).await?;
            self.verify(&target, &release.version).await
        }
        .await;
        match prepared {
            Ok(()) => self.stage(&release, target),
            Err(failure) => {
                tracing::error!("[updater] update failed {}", failure.detail());
                let _ = self.remove(&dir).await;
                self.set_state(UpdateState::Error {
                    message: failure.message().into(),
                });
            }
        }
        let _ = std::fs::remove_file(&dmg);
    }

    fn downloading(&self, release: &Release, percent: u8) {
        self.set_state(UpdateState::Downloading {
            version: release.version.clone(),
            percent,
        });
    }

    fn stage(&self, release: &Release, app: PathBuf) {
        self.lock().staged = Some(Staged {
            version: release.version.clone(),
            app,
        });
        self.set_state(UpdateState::Ready {
            version: release.version.clone(),
        });
    }

    /// UPD-040…UPD-043: copies the app out of the DMG, mounted where Finder
    /// doesn't show it.
    async fn copy_app(&self, dmg: &Path, target: &Path) -> Result<(), Failure> {
        let mount = make_temp_dir(&self.updates_dir(), "mount-").map_err(Failure::other)?;
        let copied = async {
            let attach = [
                "attach".into(),
                path_arg(dmg),
                "-nobrowse".into(),
                "-noautoopen".into(),
                "-readonly".into(),
                "-mountpoint".into(),
                path_arg(&mount),
            ];
            self.system
                .run("/usr/bin/hdiutil", attach.to_vec())
                .await
                .map_err(Failure::Other)?;
            let copied = async {
                let name = first_app(&mount)
                    .map_err(Failure::other)?
                    .ok_or_else(|| Failure::shown(NO_APP))?;
                // --noqtn: a quarantine flag would have Gatekeeper stop the new version on its first launch.
                let args = vec!["--noqtn".into(), path_arg(&mount.join(name)), path_arg(target)];
                self.system
                    .run("/usr/bin/ditto", args)
                    .await
                    .map(drop)
                    .map_err(Failure::Other)
            }
            .await;
            let detach = vec!["detach".into(), path_arg(&mount), "-force".into()];
            if let Err(err) = self.system.run("/usr/bin/hdiutil", detach).await {
                tracing::error!("[updater] detach failed {err}");
            }
            copied
        }
        .await;
        let _ = std::fs::remove_dir(&mount);
        copied
    }

    /// `plutil -extract <key> raw -o - <app>/Contents/Info.plist`, trimmed.
    async fn plist_value(&self, app: &Path, key: &str) -> Result<String, String> {
        let plist = app.join("Contents").join("Info.plist");
        let args = vec![
            "-extract".into(),
            key.into(),
            "raw".into(),
            "-o".into(),
            "-".into(),
            path_arg(&plist),
        ];
        let output = self.system.run("/usr/bin/plutil", args).await?;
        Ok(js_trim(&output.stdout).to_owned())
    }

    /// The team that signed an app, or None for an ad-hoc signature.
    async fn team_id(&self, app: &Path) -> Result<Option<String>, String> {
        let output = self
            .system
            .run("/usr/bin/codesign", vec!["-dv".into(), path_arg(app)])
            .await?;
        Ok(team_identifier(&output.stderr).map(str::to_owned))
    }

    /// UPD-044…UPD-046: the new app is intact, is this app, is the version
    /// GitHub named, and is signed by the same team as this copy (when this
    /// copy has one).
    async fn verify(&self, app: &Path, version: &str) -> Result<(), Failure> {
        let deep = vec!["--verify".into(), "--deep".into(), "--strict".into(), path_arg(app)];
        if self.system.run("/usr/bin/codesign", deep).await.is_err() {
            return Err(Failure::shown(BAD_SIGNATURE));
        }
        let bundle = self.system.bundle_path();
        let (id, own_id, new_version) = tokio::join!(
            self.plist_value(app, "CFBundleIdentifier"),
            self.plist_value(&bundle, "CFBundleIdentifier"),
            self.plist_value(app, "CFBundleShortVersionString"),
        );
        let (id, own_id, new_version) = (
            id.map_err(Failure::Other)?,
            own_id.map_err(Failure::Other)?,
            new_version.map_err(Failure::Other)?,
        );
        if id != own_id || new_version != version {
            return Err(Failure::shown(NOT_EXPECTED));
        }
        // Once releases are signed with a Developer ID, only one from the same team may replace this copy.
        if let Some(team) = self.team_id(&bundle).await.map_err(Failure::Other)?
            && self.team_id(app).await.map_err(Failure::Other)?.as_deref() != Some(team.as_str())
        {
            return Err(Failure::shown(OTHER_DEVELOPER));
        }
        Ok(())
    }

    /// UPD-048…UPD-053: downloads a release's installer and keeps it for
    /// Restart to Update. It isn't code signed, so GitHub's digest is what
    /// vouches for it; one an earlier run downloaded is checked against it.
    async fn prepare_installer(&self, release: Release) {
        let updates = self.updates_dir();
        let installer = updates.join(format!("Nodus-Setup-{}.exe", release.version));
        self.downloading(&release, 0);
        let prepared = async {
            let Some(sha256) = release.sha256.as_deref() else {
                return Err(Failure::shown(NO_CHECKSUM));
            };
            std::fs::create_dir_all(&updates).map_err(Failure::other)?;
            if !already_downloaded(&installer, release.size, sha256) {
                self.download(&release, &installer).await?;
            }
            Ok(())
        }
        .await;
        match prepared {
            Ok(()) => self.stage(&release, installer),
            Err(failure) => {
                tracing::error!("[updater] update failed {}", failure.detail());
                let _ = self.remove(&installer).await;
                self.set_state(UpdateState::Error {
                    message: failure.message().into(),
                });
            }
        }
    }

    /// `updates:install` (UPD-054…UPD-056, UPD-064…UPD-069): on macOS, quits
    /// so the swap puts the new version in and opens it; sessions keep
    /// running in the session host. On Windows, starts the installer, once
    /// the user agrees to stop the running sessions.
    pub async fn restart_to_update(&self) -> RestartOutcome {
        let Some(staged) = self.staged() else {
            return RestartOutcome::Nothing;
        };
        if self.platform() == Platform::Windows {
            return self.install_on_windows(&staged.app).await;
        }
        self.lock().relaunch = true;
        self.app.quit();
        RestartOutcome::Quitting
    }

    async fn install_on_windows(&self, installer: &Path) -> RestartOutcome {
        {
            let mut inner = self.lock();
            if inner.installing {
                return RestartOutcome::Nothing;
            }
            inner.installing = true;
        }
        // Before the sessions stop, so a missing installer stops nothing.
        if let Err(err) = std::fs::metadata(installer) {
            return self.installer_failed(installer, &err.to_string()).await;
        }
        let running = self.app.live_session_count();
        if running > 0 {
            self.lock().asking = true;
            return RestartOutcome::Ask(StopSessionsQuestion::new(running));
        }
        self.stop_and_install(installer).await
    }

    /// The answer to [`RestartOutcome::Ask`]: `Stop and Update` (true) or
    /// `Not Now` (false, and closing the question).
    pub async fn answer_stop_sessions(&self, stop: bool) -> RestartOutcome {
        let staged = {
            let mut inner = self.lock();
            if !inner.asking {
                return RestartOutcome::Nothing;
            }
            inner.asking = false;
            if !stop {
                inner.installing = false;
                return RestartOutcome::NotNow;
            }
            inner.staged.clone()
        };
        let Some(staged) = staged else {
            self.lock().installing = false;
            return RestartOutcome::Nothing;
        };
        self.stop_and_install(&staged.app).await
    }

    /// UPD-068: stops every session and the host, then starts the installer
    /// on its own and quits once Windows has started it.
    async fn stop_and_install(&self, installer: &Path) -> RestartOutcome {
        if let Err(err) = self.app.stop_all().await {
            return self.installer_failed(installer, &err).await;
        }
        let args = INSTALLER_ARGS.iter().map(|&arg| arg.to_owned()).collect();
        if let Err(err) = self.system.spawn_detached(installer, args) {
            return self.installer_failed(installer, &err.to_string()).await;
        }
        self.app.quit();
        self.lock().installing = false;
        RestartOutcome::Quitting
    }

    /// UPD-069: Nodus stays open and says so; Check for Updates downloads it again.
    async fn installer_failed(&self, installer: &Path, err: &str) -> RestartOutcome {
        tracing::error!("[updater] the installer didn’t start {err}");
        self.lock().staged = None;
        let _ = self.remove(installer).await;
        self.set_state(UpdateState::Error {
            message: INSTALLER_FAILED.into(),
        });
        self.lock().installing = false;
        RestartOutcome::Failed
    }

    /// UPD-057/058: on macOS, at quit with an update staged, starts the swap
    /// script detached: it waits for this process to exit, puts the new app
    /// in place of this one, and opens it after Restart to Update. Once per
    /// run, and never on Windows (UPD-070). The app calls this from its
    /// normal quit path, after its windows close and before it exits; not
    /// from paths that exit outright.
    pub fn on_will_quit(&self) {
        let (staged, relaunch) = {
            let mut inner = self.lock();
            let Some(staged) = inner.staged.clone().filter(|_| inner.swap_on_quit && !inner.swapping) else {
                return;
            };
            inner.swapping = true;
            (staged, inner.relaunch)
        };
        let user_data = if relaunch {
            // A test copy keeps its own data once it opens again.
            self.app.user_data_override().unwrap_or_default()
        } else {
            String::new()
        };
        let mut args = vec!["-c".to_owned(), SWAP_SCRIPT.to_owned(), "nodus-update".to_owned()];
        args.extend([
            self.system.pid().to_string(),
            path_arg(&self.system.bundle_path()),
            path_arg(&staged.app),
            path_arg(&self.updates_dir().join("previous.app")),
            path_arg(&self.failed_file()),
            staged.version,
            if relaunch { "1" } else { "" }.to_owned(),
            user_data,
        ]);
        if let Err(err) = self.system.spawn_detached(Path::new("/bin/sh"), args) {
            tracing::error!("[updater] swap failed to start {err}");
        }
    }

    /// UPD-071…UPD-075: clears what earlier runs left behind (downloads, a
    /// staged app that never went in, the previous version), keeping the
    /// failed marker while it matters and, on Windows, the newest installer.
    async fn clean_up(&self) {
        let dir = self.updates_dir();
        if let Ok(text) = std::fs::read_to_string(self.failed_file()) {
            let failed = js_trim(&text);
            // Once this copy has caught up with that version, the failure no longer matters.
            if is_newer(failed, &self.app.version()) {
                self.lock().failed_version = Some(failed.to_owned());
            }
        }
        let names: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let windows = self.platform() == Platform::Windows;
        let kept = if windows {
            newest_installer(names.iter().map(String::as_str), &self.app.version()).map(str::to_owned)
        } else {
            None
        };
        let has_failed = self.lock().failed_version.is_some();
        for name in &names {
            if (name == "failed" && has_failed) || kept.as_ref() == Some(name) {
                continue;
            }
            let path = dir.join(name);
            // A DMG still mounted from a quit mid-copy.
            if !windows && name.starts_with("mount-") {
                let detach = vec!["detach".into(), path_arg(&path), "-force".into()];
                let _ = self.system.run("/usr/bin/hdiutil", detach).await;
            }
            if let Err(err) = self.remove(&path).await {
                tracing::error!("[updater] cleanup failed {err}");
            }
        }
    }
}

/// Runs a check that isn't asked for, while Settings allows it.
fn auto_check(me: &Weak<Updater>) {
    let Some(this) = me.upgrade() else { return };
    if this.app.auto_update() {
        tokio::spawn(async move { this.check(false).await });
    }
}

/// `Math.min(100, Math.floor(received / size * 100))`, in doubles as
/// JavaScript computes it.
fn percent_of(received: u64, size: u64) -> u8 {
    let percent = ((received as f64 / size as f64) * 100.0).floor().min(100.0);
    if percent.is_nan() { 0 } else { percent as u8 }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether `file` is the release's installer already, as an earlier run
/// downloaded it: the same size and SHA-256.
fn already_downloaded(file: &Path, size: u64, sha256: &str) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    if meta.len() != size {
        return false;
    }
    let Ok(bytes) = std::fs::read(file) else { return false };
    hex(&Sha256::digest(&bytes)) == sha256
}

/// The first entry of `dir` whose name ends in `.app`, in the order the OS
/// lists them, as `readdir` and `find` do.
fn first_app(dir: &Path) -> std::io::Result<Option<String>> {
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".app") {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// `fs.mkdtemp(<dir>/<prefix>)`: a new folder named `prefix` and six random characters.
fn make_temp_dir(dir: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    loop {
        let random = uuid::Uuid::new_v4().simple().to_string();
        let path = dir.join(format!("{prefix}{}", &random[..6]));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
}

/// `fs.rm(target, { recursive: true, force: true })`.
fn remove_all(target: &Path) -> std::io::Result<()> {
    let result = match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(err) => Err(err),
    };
    match result {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_percent_as_javascript_does() {
        assert_eq!(percent_of(0, 100), 0);
        // 29 / 100 * 100 is 28.999999999999996 in doubles.
        assert_eq!(percent_of(29, 100), 28);
        assert_eq!(percent_of(50, 100), 50);
        assert_eq!(percent_of(100, 100), 100);
        assert_eq!(percent_of(150, 100), 100);
        assert_eq!(percent_of(1, 0), 100);
    }

    #[test]
    fn asks_about_one_session_or_several() {
        let one = StopSessionsQuestion::new(1);
        assert_eq!(one.message, "Stop the running session to update?");
        assert_eq!(
            StopSessionsQuestion::new(3).message,
            "Stop 3 running sessions to update?"
        );
        assert_eq!(
            one.detail,
            "Windows can’t update Nodus while its sessions run. You can resume them from their cards afterwards."
        );
        assert_eq!(one.buttons, ["Stop and Update".to_owned(), "Not Now".to_owned()]);
        assert_eq!((one.default_id, one.cancel_id), (0, 1));
    }

    #[test]
    fn makes_mount_folders_as_mkdtemp_does() {
        let dir = tempfile::TempDir::new().unwrap();
        let mount = make_temp_dir(&dir.path().join("updates"), "mount-").unwrap();
        let name = mount.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("mount-") && name.len() == 12, "{name}");
        assert!(mount.is_dir());
        assert_ne!(make_temp_dir(&dir.path().join("updates"), "mount-").unwrap(), mount);
    }

    #[test]
    fn removes_like_fs_rm_forced() {
        let dir = tempfile::TempDir::new().unwrap();
        let folder = dir.path().join("a");
        std::fs::create_dir_all(folder.join("b")).unwrap();
        std::fs::write(folder.join("b").join("c"), "x").unwrap();
        remove_all(&folder).unwrap();
        assert!(!folder.exists());
        remove_all(&folder).unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        remove_all(&file).unwrap();
        assert!(!file.exists());
    }
}
