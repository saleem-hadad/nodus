//! `window.api.capture`: the floating capture panel's commands (`capture:*`
//! in `src/main/ipc.ts`), and capture's hooks into the rest of the app
//! (`registerCapture`'s handlers, `boardOpened`, `syncShortcut` on every
//! settings change, `stopCapture` on quit).
//!
//! The service itself is nodus-capture's [`Capture`]. What only the windowed
//! app has (the panel window, the screen, the permissions, the key hook and
//! the board window's size) it hands over once with
//! [`CaptureApi::register`]; until then, and in a headless Nodus, capture
//! has no panel, reads nothing from the screen and never hears a key.
//!
//! Every command runs on the core's runtime, never on the caller's thread:
//! the panel's window answers from the UI thread, which may be the caller.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock, Weak};
use std::time::Duration;

use nodus_board::BoxFuture;
use nodus_board::geometry::Point;
use nodus_capture::hook::HookSink;
use nodus_capture::service::{PANEL_H, PANEL_W};
use nodus_capture::{
    Capture, CaptureDeps, CaptureHandlers, CaptureSessions, ClaudeEnv, Clipboard, FrontApp, KeyHook, PanelWindow,
    Permissions, RunningHook, Screen, Screenshot,
};
use nodus_domain::{
    BubbleGuess, CaptureAttachment, CaptureContext, CaptureGuess, CaptureReceipt, CaptureShortcut, CaptureSubmission,
    CaptureTargets, CreateSessionOptions, CreateViewOptions, Event, EventSink, MenuCommand, OpenCardRequest, Rect,
    SessionMeta, SessionStatus, TabAttachment,
};
use nodus_platform::{ClaudeNotFound, Env};
use nodus_store::Store;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::error::OnChannel;
use crate::nodus::Inner;
use crate::wiring::CoreLauncher;
use crate::{CommandResult, Nodus};

/// The board window's content size while it's open (CAP-134).
pub type WindowSize = Arc<dyn Fn() -> Option<(f64, f64)> + Send + Sync>;

/// What only the windowed app gives capture.
#[derive(Clone)]
pub struct CaptureSystem {
    /// The panel's window (an NSPanel on macOS).
    pub panel: Arc<dyn PanelWindow>,
    /// The app in front, its browser tab, the clipboard and screenshots:
    /// nodus-capture's `SystemScreen`, or a stand-in under the test driver.
    pub screen: Arc<dyn Screen>,
    /// Accessibility and Screen Recording: `SystemPermissions`.
    pub permissions: Arc<dyn Permissions>,
    /// The global key hook for the double tap: `SystemHook`.
    pub hook: Arc<dyn KeyHook>,
    /// The board window's content size while it's open (CAP-134).
    pub window_size: WindowSize,
}

impl CaptureSystem {
    /// No panel, nothing read from the screen, no permission, no key hook:
    /// a headless Nodus, and capture before the app registers its own.
    pub fn headless() -> Self {
        Self {
            panel: Arc::new(NoPanel),
            screen: Arc::new(NoScreen),
            permissions: Arc::new(NoPermissions),
            hook: Arc::new(NoHook),
            window_size: Arc::new(|| None),
        }
    }
}

static HEADLESS: LazyLock<CaptureSystem> = LazyLock::new(CaptureSystem::headless);

/// No window: it never exists, so nothing is shown.
struct NoPanel;

impl PanelWindow for NoPanel {
    fn exists(&self) -> bool {
        false
    }
    fn create(&self) {}
    fn is_visible(&self) -> bool {
        false
    }
    fn is_focused(&self) -> bool {
        false
    }
    fn is_loading(&self) -> bool {
        false
    }
    fn bounds(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: PANEL_W,
            h: PANEL_H,
        }
    }
    fn set_bounds(&self, _: Rect) {}
    fn show_and_focus(&self) {}
    fn show_inactive(&self) {}
    fn hide(&self) {}
    fn work_area_under_pointer(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1440.0,
            h: 900.0,
        }
    }
    fn any_window_focused(&self) -> bool {
        false
    }
    fn pick_files(&self) -> BoxFuture<'_, Option<Vec<String>>> {
        Box::pin(async { None })
    }
}

/// Nothing in front, an empty clipboard, and every screenshot cancelled.
struct NoScreen;

struct EmptyClipboard;

impl Clipboard for EmptyClipboard {
    fn text(&self) -> Option<String> {
        None
    }
    fn image(&self) -> Option<Vec<u8>> {
        None
    }
}

impl Screen for NoScreen {
    fn front_app(&self) -> BoxFuture<'_, Option<FrontApp>> {
        Box::pin(async { None })
    }
    fn browser_tab<'a>(&'a self, _: &'a FrontApp) -> BoxFuture<'a, Option<TabAttachment>> {
        Box::pin(async { None })
    }
    fn clipboard(&self) -> &dyn Clipboard {
        &EmptyClipboard
    }
    fn screenshot<'a>(&'a self, _: &'a std::path::Path) -> BoxFuture<'a, Screenshot> {
        Box::pin(async { Screenshot::None })
    }
}

/// Never trusted, and asking does nothing.
struct NoPermissions;

impl Permissions for NoPermissions {
    fn accessibility_trusted(&self) -> bool {
        false
    }
    fn request_accessibility(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
    fn open_screen_capture_settings(&self) {}
}

/// No key hook here.
struct NoHook;

impl KeyHook for NoHook {
    fn start(&self, _: HookSink) -> Result<Box<dyn RunningHook>, String> {
        Err("no key hook in a headless Nodus".into())
    }
}

/// The parts the app registers, or the headless ones until it has.
#[derive(Clone)]
struct Late(Arc<OnceLock<CaptureSystem>>);

impl Late {
    fn get(&self) -> &CaptureSystem {
        self.0.get().unwrap_or(&HEADLESS)
    }
}

impl PanelWindow for Late {
    fn exists(&self) -> bool {
        self.get().panel.exists()
    }
    fn create(&self) {
        self.get().panel.create();
    }
    fn is_visible(&self) -> bool {
        self.get().panel.is_visible()
    }
    fn is_focused(&self) -> bool {
        self.get().panel.is_focused()
    }
    fn is_loading(&self) -> bool {
        self.get().panel.is_loading()
    }
    fn bounds(&self) -> Rect {
        self.get().panel.bounds()
    }
    fn set_bounds(&self, bounds: Rect) {
        self.get().panel.set_bounds(bounds);
    }
    fn show_and_focus(&self) {
        self.get().panel.show_and_focus();
    }
    fn show_inactive(&self) {
        self.get().panel.show_inactive();
    }
    fn hide(&self) {
        self.get().panel.hide();
    }
    fn work_area_under_pointer(&self) -> Rect {
        self.get().panel.work_area_under_pointer()
    }
    fn any_window_focused(&self) -> bool {
        self.get().panel.any_window_focused()
    }
    fn pick_files(&self) -> BoxFuture<'_, Option<Vec<String>>> {
        self.get().panel.pick_files()
    }
}

impl Screen for Late {
    fn front_app(&self) -> BoxFuture<'_, Option<FrontApp>> {
        self.get().screen.front_app()
    }
    fn browser_tab<'a>(&'a self, front: &'a FrontApp) -> BoxFuture<'a, Option<TabAttachment>> {
        self.get().screen.browser_tab(front)
    }
    fn clipboard(&self) -> &dyn Clipboard {
        self.get().screen.clipboard()
    }
    fn screenshot<'a>(&'a self, user_data: &'a std::path::Path) -> BoxFuture<'a, Screenshot> {
        self.get().screen.screenshot(user_data)
    }
}

impl Permissions for Late {
    fn accessibility_trusted(&self) -> bool {
        self.get().permissions.accessibility_trusted()
    }
    fn request_accessibility(&self) -> BoxFuture<'_, bool> {
        self.get().permissions.request_accessibility()
    }
    fn open_screen_capture_settings(&self) {
        self.get().permissions.open_screen_capture_settings();
    }
}

impl KeyHook for Late {
    fn start(&self, sink: HookSink) -> Result<Box<dyn RunningHook>, String> {
        self.get().hook.start(sink)
    }
}

/// `registerCapture`'s handlers (ipc.ts), on the core's domains.
struct CoreCapture {
    core: Weak<Inner>,
    system: Late,
}

impl CaptureHandlers for CoreCapture {
    fn window_size(&self) -> Option<(f64, f64)> {
        (self.system.get().window_size)()
    }

    /// `createSession` (03), once the login shell's environment is in, as
    /// `session:create` waits for it.
    fn create_session(&self, project_id: &str, opts: CreateSessionOptions) -> BoxFuture<'_, Result<String, String>> {
        let project_id = project_id.to_owned();
        Box::pin(async move {
            let core = self.core.upgrade().ok_or_else(gone)?;
            core.env().await;
            let launcher = CoreLauncher(self.core.clone());
            core.board
                .create_session(&project_id, &opts, &launcher)
                .await
                .map(|card| card.node.session_id)
                .map_err(|err| err.to_string())
        })
    }

    /// `sendToSession` (03).
    fn send_to_session(&self, project_id: &str, session_id: &str, text: &str) -> BoxFuture<'_, Result<(), String>> {
        let (project_id, session_id, text) = (project_id.to_owned(), session_id.to_owned(), text.to_owned());
        Box::pin(async move {
            let core = self.core.upgrade().ok_or_else(gone)?;
            core.env().await;
            let launcher = CoreLauncher(self.core.clone());
            core.board
                .send_to_session(&project_id, &session_id, &text, &launcher)
                .await
                .map_err(|err| err.to_string())
        })
    }

    /// `sessionTitle` (03): the card's title, else `New session`.
    fn session_title(&self, project_id: &str, session_id: &str) -> String {
        self.core
            .upgrade()
            .and_then(|core| core.board.session_title(project_id, session_id).ok())
            .unwrap_or_else(|| "New session".into())
    }

    /// `views.createView` (04): titled `New view`, its brain started on it.
    fn create_view(&self, project_id: &str, description: &str, at: Point) -> BoxFuture<'_, Result<String, String>> {
        let project_id = project_id.to_owned();
        let opts = CreateViewOptions {
            description: description.to_owned(),
            x: at.x,
            y: at.y,
        };
        Box::pin(async move {
            let core = self.core.upgrade().ok_or_else(gone)?;
            core.views
                .create(&project_id, &opts)
                .await
                .map(|view| view.id)
                .map_err(|err| err.to_string())
        })
    }

    /// `notify.openCard` (HOST-212).
    fn open_card(&self, request: OpenCardRequest) {
        if let Some(core) = self.core.upgrade() {
            core.notify.open_card(request);
        }
    }

    /// CAP-159: the board window up, re-made if it was closed, then its
    /// Settings at Capture & Voice, where the voice key goes. A window still loading hasn't subscribed to menu commands
    /// yet, so the command waits until it has (`did-finish-load`).
    fn open_settings(&self) {
        let Some(core) = self.core.upgrade() else { return };
        let reopened = !core.shell.is_open() || core.shell.is_loading();
        core.shell.show();
        if !reopened {
            let _ = core.events.send(Event::Menu(MenuCommand::VoiceSettings));
            return;
        }
        let weak = self.core.clone();
        core.runtime.spawn(async move {
            // As long as a window takes to load, and no longer.
            for _ in 0..600 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let Some(core) = weak.upgrade() else { return };
                if core.shell.is_open() && !core.shell.is_loading() {
                    let _ = core.events.send(Event::Menu(MenuCommand::VoiceSettings));
                    return;
                }
            }
        });
    }
}

/// What capture needs about sessions: the session host, the status machine
/// and the transcripts. None of it touches the `Store`.
struct CoreCaptureSessions(Weak<Inner>);

impl CaptureSessions for CoreCaptureSessions {
    fn is_live(&self, session_id: &str) -> bool {
        self.0.upgrade().is_some_and(|core| core.host.is_live(session_id))
    }

    fn status(&self, session_id: &str) -> SessionStatus {
        self.0
            .upgrade()
            .map_or(SessionStatus::Done, |core| core.status.status(session_id))
    }

    fn meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta> {
        let core = self.0.upgrade()?;
        core.board
            .transcripts()
            .session(repo_path, session_id)
            .map(|found| found.meta)
    }
}

/// `requireClaude` and `spawnEnv` on the user's environment: the login
/// shell's once it's in, the app's own until then.
struct CoreClaude(watch::Receiver<Option<Arc<Env>>>);

impl CoreClaude {
    fn env(&self) -> Arc<Env> {
        self.0.borrow().clone().unwrap_or_else(|| Arc::new(Env::from_process()))
    }
}

impl ClaudeEnv for CoreClaude {
    fn claude(&self) -> Result<PathBuf, ClaudeNotFound> {
        ClaudeEnv::claude(&*self.env())
    }

    fn spawn_env(&self) -> nodus_platform::env::EnvMap {
        ClaudeEnv::spawn_env(&*self.env())
    }
}

/// The fire-and-forget commands (`ipcRenderer.send`), kept in order.
enum Fire {
    Hide,
    Resize(f64),
    Blur,
}

/// Capture, as the core holds it.
pub(crate) struct CaptureCore {
    pub(crate) capture: Arc<Capture>,
    system: Arc<OnceLock<CaptureSystem>>,
    sends: mpsc::UnboundedSender<Fire>,
}

impl CaptureCore {
    /// Capture on the core's domains, with the headless parts until the app
    /// registers its own.
    pub(crate) fn new(
        core: &Weak<Inner>,
        store: &Arc<Store>,
        user_data: PathBuf,
        env: watch::Receiver<Option<Arc<Env>>>,
        events: Arc<dyn EventSink>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let system = Arc::new(OnceLock::new());
        let late = Late(Arc::clone(&system));
        let capture = Capture::new(CaptureDeps {
            store: Arc::clone(store),
            user_data,
            handlers: Arc::new(CoreCapture {
                core: core.clone(),
                system: late.clone(),
            }),
            sessions: Arc::new(CoreCaptureSessions(core.clone())),
            panel: Arc::new(late.clone()),
            screen: Arc::new(late.clone()),
            permissions: Arc::new(late.clone()),
            hook: Arc::new(late),
            claude: Arc::new(CoreClaude(env)),
            events,
            runtime: runtime.clone(),
            clock: nodus_board::real_clock(),
        });
        let (sends, mut rx) = mpsc::unbounded_channel();
        let weak = Arc::downgrade(&capture);
        runtime.spawn(async move {
            while let Some(send) = rx.recv().await {
                let Some(capture) = weak.upgrade() else { return };
                match send {
                    Fire::Hide => capture.hide(),
                    Fire::Resize(height) => capture.resize(height),
                    // It waits 50 ms before it decides, on a task of its own.
                    Fire::Blur => drop(tokio::spawn(async move { capture.on_blur().await })),
                }
            }
        });
        Self { capture, system, sends }
    }

    /// `boardOpened` (CAP-154), before `board:load` loads it (SESS-071 step 1).
    pub(crate) fn board_opened(&self, project_id: &str) {
        self.capture.board_opened(project_id);
    }

    /// `stopCapture` (CAP-039, PLAT-032 step 1).
    pub(crate) fn stop(&self) {
        self.capture.stop();
    }
}

/// `window.api.capture`.
pub struct CaptureApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl CaptureApi<'_> {
    fn core(&self) -> &CaptureCore {
        &self.nodus.inner.capture
    }

    fn service(&self) -> Arc<Capture> {
        Arc::clone(&self.core().capture)
    }

    /// Runs `work` on the core's runtime and waits for it there.
    async fn run<T: Send + 'static>(&self, fallback: T, work: impl Future<Output = T> + Send + 'static) -> T {
        self.nodus.inner.runtime.spawn(work).await.unwrap_or(fallback)
    }

    /// `registerCapture` (CAP-038, CAP-063): the windowed app's panel,
    /// screen, permissions, key hook and window size, given once. Clears
    /// away captures older than 30 days, matches the double tap to the
    /// setting and the permission, and again on every settings change.
    pub fn register(&self, system: CaptureSystem) {
        if self.core().system.set(system).is_err() {
            return;
        }
        let capture = self.service();
        let weak = Arc::downgrade(&capture);
        // `applySettings` → `syncShortcut`, whatever changed.
        self.nodus.on_settings_applied(Arc::new(move |_| {
            if let Some(capture) = weak.upgrade() {
                capture.sync_shortcut();
            }
        }));
        let _runtime = self.nodus.inner.runtime.enter();
        capture.register();
    }

    /// `capture:ready` (CAP-014): what the panel is up over, while it's up.
    pub async fn ready(&self) -> Option<CaptureContext> {
        let capture = self.service();
        self.run(None, async move { capture.ready() }).await
    }

    /// `capture:targets` (CAP-149).
    pub async fn targets(&self) -> CaptureTargets {
        let capture = self.service();
        let empty = CaptureTargets {
            projects: Vec::new(),
            sessions: Vec::new(),
            recents: Vec::new(),
            default_project_id: None,
        };
        self.run(empty, async move { capture.targets() }).await
    }

    /// `capture:guess` (CAP-067): None when Claude can't say, or a newer
    /// guess replaced it; fails when Claude Code isn't installed.
    pub async fn guess(&self, text: &str, attachments: &[CaptureAttachment]) -> CommandResult<Option<CaptureGuess>> {
        let capture = self.service();
        let text = text.to_owned();
        let attachments = serde_json::to_value(attachments).unwrap_or(Value::Null);
        self.run(Ok(None), async move {
            capture.guess(&text, &attachments).await.map_err(|err| err.to_string())
        })
        .await
        .on("capture:guess")
    }

    /// Where the board bubble's text goes (BUBBLE-023): None when Claude
    /// can't say; fails when Claude Code isn't installed. Asks run side by
    /// side.
    pub async fn route_bubble(&self, text: &str, project: &str) -> CommandResult<Option<BubbleGuess>> {
        let capture = self.service();
        let (text, project) = (text.to_owned(), project.to_owned());
        self.run(Ok(None), async move {
            capture
                .route_bubble(&text, &project)
                .await
                .map_err(|err| err.to_string())
        })
        .await
        .on("bubble:route")
    }

    /// `capture:submit` (CAP-125…CAP-131): makes it on the board; the panel
    /// rests with its receipt. Fails with the message the panel shows.
    pub async fn submit(&self, submission: &CaptureSubmission) -> CommandResult<CaptureReceipt> {
        let capture = self.service();
        let raw = serde_json::to_value(submission).unwrap_or(Value::Null);
        self.run(Err("Nodus is quitting".to_owned()), async move {
            capture.submit(&raw).await.map_err(|err| err.to_string())
        })
        .await
        .on("capture:submit")
    }

    /// `capture:screenshot` (CAP-058): the panel set aside for a screenshot.
    pub async fn screenshot(&self) -> Screenshot {
        let capture = self.service();
        self.run(Screenshot::None, async move { capture.screenshot().await })
            .await
    }

    /// `capture:allow-screen` (CAP-059).
    pub async fn allow_screen(&self) {
        let capture = self.service();
        self.run((), async move { capture.allow_screen() }).await;
    }

    /// `capture:files(paths)` (CAP-060): dropped files, or with none, the
    /// open dialog.
    pub async fn files(&self, paths: Option<&[String]>) -> Vec<CaptureAttachment> {
        let capture = self.service();
        let paths = paths.map_or(Value::Null, |p| serde_json::to_value(p).unwrap_or(Value::Null));
        self.run(Vec::new(), async move { capture.files(&paths).await }).await
    }

    /// `capture:clipboard` (CAP-054).
    pub async fn clipboard(&self) -> Option<CaptureAttachment> {
        let capture = self.service();
        self.run(None, async move { capture.clipboard() }).await
    }

    /// `capture:hide` (CAP-160), fire and forget.
    pub fn hide(&self) {
        let _ = self.core().sends.send(Fire::Hide);
    }

    /// `capture:resize(height)` (CAP-009), fire and forget.
    pub fn resize(&self, height: f64) {
        let _ = self.core().sends.send(Fire::Resize(height));
    }

    /// The panel lost the keyboard (CAP-012): clicking away puts it away.
    pub fn blurred(&self) {
        let _ = self.core().sends.send(Fire::Blur);
    }

    /// `capture:open(request)` (CAP-145).
    pub async fn open(&self, request: &OpenCardRequest) {
        let capture = self.service();
        let request = serde_json::to_value(request).unwrap_or(Value::Null);
        self.run((), async move { capture.open(&request) }).await;
    }

    /// `capture:settings` (CAP-159).
    pub async fn open_settings(&self) {
        let capture = self.service();
        self.run((), async move { capture.settings() }).await;
    }

    /// `capture:shortcut` (CAP-044).
    pub async fn shortcut(&self) -> CaptureShortcut {
        let capture = self.service();
        self.run(CaptureShortcut::Unavailable, async move { capture.shortcut_state() })
            .await
    }

    /// `capture:allow` (CAP-040).
    pub async fn allow(&self) -> CaptureShortcut {
        let capture = self.service();
        self.run(CaptureShortcut::Unavailable, async move { capture.allow().await })
            .await
    }

    /// `capture:show` (CAP-043) and File › Capture… (CAP-042, PLAT-048):
    /// opens the panel, never toggles.
    pub async fn show(&self) {
        let capture = self.service();
        self.run((), async move { capture.show().await }).await;
    }
}

fn gone() -> String {
    "Nodus is quitting".into()
}

#[cfg(test)]
mod tests {
    use nodus_domain::CaptureKind;

    use super::*;
    use crate::tests::nodus;

    /// A project on a real folder.
    async fn project(nodus: &Nodus, dir: &std::path::Path) -> String {
        let repo = dir.join("repos").join("storefront");
        std::fs::create_dir_all(&repo).unwrap();
        let project = nodus
            .projects()
            .add(Some(repo.to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap();
        project.id
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lists_targets_and_puts_a_note_up() {
        let (dir, nodus) = nodus();
        let id = project(&nodus, dir.path()).await;
        let targets = nodus.capture().targets().await;
        assert_eq!(targets.projects.len(), 1);
        assert_eq!(targets.default_project_id.as_deref(), Some(id.as_str()));
        let mut events = nodus.subscribe();
        let receipt = nodus
            .capture()
            .submit(&CaptureSubmission {
                text: "  Remember to ask Sam about the coupon limits  ".into(),
                kind: CaptureKind::Note,
                project_id: id.clone(),
                session_id: None,
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(receipt.kind, CaptureKind::Note);
        assert_eq!(receipt.title, "Remember to ask Sam about the coupon limits");
        assert!(receipt.note_id.is_some());
        // CAP-130: an open board reloads.
        assert_eq!(
            events.recv().await.unwrap(),
            Event::BoardChanged { project_id: id.clone() }
        );
        // CAP-155: remembered for the recents.
        let targets = nodus.capture().targets().await;
        assert_eq!(targets.recents.len(), 1);
        assert_eq!(targets.recents[0].kind, CaptureKind::Note);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_with_the_words_the_panel_shows() {
        let (_dir, nodus) = nodus();
        let err = nodus
            .capture()
            .submit(&CaptureSubmission {
                text: "Fix it".into(),
                kind: CaptureKind::Session,
                project_id: "gone".into(),
                session_id: None,
                attachments: Vec::new(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.message(), "Unknown project gone");
        assert_eq!(err.channel, "capture:submit");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn has_no_panel_and_no_guess_without_projects_when_headless() {
        let (_dir, nodus) = nodus();
        // CAP-068: no projects, no claude run.
        assert_eq!(nodus.capture().guess("Fix the coupon field", &[]).await, Ok(None));
        nodus.capture().show().await;
        // CAP-014: nothing is up.
        assert_eq!(nodus.capture().ready().await, None);
        // CAP-032: a headless Nodus isn't trusted with Accessibility.
        assert_eq!(nodus.capture().shortcut().await, CaptureShortcut::NeedsPermission);
        nodus
            .settings()
            .update_json(serde_json::json!({ "capture": false }))
            .await;
        assert_eq!(nodus.capture().shortcut().await, CaptureShortcut::Off);
        // Nothing to take, nothing picked.
        assert_eq!(nodus.capture().clipboard().await, None);
        assert_eq!(nodus.capture().files(None).await, Vec::new());
        assert!(matches!(nodus.capture().screenshot().await, Screenshot::None));
        nodus.capture().hide();
        nodus.capture().resize(400.0);
    }

    /// A key hook that counts how often it starts and stops.
    #[derive(Clone, Default)]
    struct CountingHook(Arc<std::sync::Mutex<(u32, u32)>>);

    impl CountingHook {
        fn counts(&self) -> (u32, u32) {
            *self.0.lock().unwrap()
        }
    }

    struct Running(Arc<std::sync::Mutex<(u32, u32)>>);

    impl KeyHook for CountingHook {
        fn start(&self, _sink: HookSink) -> Result<Box<dyn RunningHook>, String> {
            self.0.lock().unwrap().0 += 1;
            Ok(Box::new(Running(Arc::clone(&self.0))))
        }
    }

    impl RunningHook for Running {
        fn stop(self: Box<Self>) -> Result<(), String> {
            self.0.lock().unwrap().1 += 1;
            Ok(())
        }
    }

    /// Accessibility granted.
    struct Trusted;

    impl Permissions for Trusted {
        fn accessibility_trusted(&self) -> bool {
            true
        }

        fn request_accessibility(&self) -> BoxFuture<'_, bool> {
            Box::pin(async { false })
        }

        fn open_screen_capture_settings(&self) {}
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn follows_the_setting_with_the_key_hook_and_stops_it_on_quit() {
        let (_dir, nodus) = nodus();
        let hook = CountingHook::default();
        // CAP-038: registering matches the hook to the setting (on by default).
        nodus.capture().register(CaptureSystem {
            hook: Arc::new(hook.clone()),
            permissions: Arc::new(Trusted),
            ..CaptureSystem::headless()
        });
        assert_eq!(hook.counts(), (1, 0));
        assert_eq!(nodus.capture().shortcut().await, CaptureShortcut::Ready);
        // CAP-037, CAP-038: every settings change syncs it.
        nodus
            .settings()
            .update_json(serde_json::json!({ "capture": false }))
            .await;
        assert_eq!(hook.counts(), (1, 1));
        assert_eq!(nodus.capture().shortcut().await, CaptureShortcut::Off);
        nodus
            .settings()
            .update_json(serde_json::json!({ "capture": true }))
            .await;
        assert_eq!(hook.counts(), (2, 1));
        // A change elsewhere leaves a running hook alone.
        nodus
            .settings()
            .update_json(serde_json::json!({ "notify": false }))
            .await;
        assert_eq!(hook.counts(), (2, 1));
        // CAP-039, PLAT-032 step 1: quitting stops it.
        nodus.shutdown();
        assert_eq!(hook.counts(), (2, 2));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hears_which_board_opened_before_it_loads() {
        let (dir, nodus) = nodus();
        let first = project(&nodus, dir.path()).await;
        let other = dir.path().join("repos").join("docs-site");
        std::fs::create_dir_all(&other).unwrap();
        let second = nodus
            .projects()
            .add(Some(other.to_string_lossy().into_owned()))
            .await
            .unwrap()
            .unwrap()
            .id;
        // CAP-151, CAP-154: with nothing captured yet, the board opened last
        // is where a capture goes.
        nodus.board().load(&first).await.unwrap();
        assert_eq!(
            nodus.capture().targets().await.default_project_id.as_deref(),
            Some(first.as_str())
        );
        nodus.board().load(&second).await.unwrap();
        assert_eq!(
            nodus.capture().targets().await.default_project_id.as_deref(),
            Some(second.as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn describes_dropped_files() {
        let (dir, nodus) = nodus();
        let file = dir.path().join("coupon-spec.pdf");
        std::fs::write(&file, vec![0u8; 2048]).unwrap();
        let path = file.to_string_lossy().into_owned();
        let files = nodus
            .capture()
            .files(Some(&[path.clone(), "relative.txt".into()]))
            .await;
        assert_eq!(
            files,
            vec![CaptureAttachment::File(nodus_domain::FileAttachment {
                path,
                name: "coupon-spec.pdf".into(),
                size: 2048,
            })]
        );
    }
}
