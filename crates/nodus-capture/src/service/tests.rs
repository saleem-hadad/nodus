//! The capture service with every OS seam faked: a panel window that keeps
//! its state in memory, a made-up screen and clipboard, a fake key hook and
//! permissions, and a paused clock.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use nodus_domain::{CaptureKind, ImageAttachment, RecordingSink};
use nodus_platform::env::EnvMap;
use serde_json::json;

use super::*;
use crate::hook::HookEvent;
use crate::submit::tests::{Call, Handlers, seeded, store_with};
use crate::targets::tests::{FakeSessions, project};

const NOW: f64 = 1_800_000_000_000.0;
const WORK: Rect = Rect {
    x: 0.0,
    y: 25.0,
    w: 1512.0,
    h: 920.0,
};

#[derive(Debug, Clone, PartialEq, Default)]
struct Window {
    exists: bool,
    visible: bool,
    focused: bool,
    loading: bool,
    bounds: Option<Rect>,
    /// Another Nodus window has the keyboard.
    board_focused: bool,
    log: Vec<&'static str>,
}

#[derive(Default)]
struct FakePanel {
    window: Mutex<Window>,
    picked: Mutex<Option<Vec<String>>>,
}

impl FakePanel {
    fn get(&self) -> Window {
        self.window.lock().unwrap().clone()
    }

    fn with<R>(&self, change: impl FnOnce(&mut Window) -> R) -> R {
        change(&mut self.window.lock().unwrap())
    }

    fn log(&self) -> Vec<&'static str> {
        std::mem::take(&mut self.window.lock().unwrap().log)
    }
}

impl PanelWindow for FakePanel {
    fn exists(&self) -> bool {
        self.get().exists
    }

    fn create(&self) {
        self.with(|w| {
            w.exists = true;
            w.bounds = Some(Rect {
                x: 0.0,
                y: 0.0,
                w: PANEL_W,
                h: PANEL_H,
            });
            w.log.push("create");
        });
    }

    fn is_visible(&self) -> bool {
        self.get().visible
    }

    fn is_focused(&self) -> bool {
        self.get().focused
    }

    fn is_loading(&self) -> bool {
        self.get().loading
    }

    fn bounds(&self) -> Rect {
        self.get().bounds.unwrap()
    }

    fn set_bounds(&self, bounds: Rect) {
        self.with(|w| w.bounds = Some(bounds));
    }

    fn show_and_focus(&self) {
        self.with(|w| {
            w.visible = true;
            w.focused = true;
            w.log.push("show+focus");
        });
    }

    fn show_inactive(&self) {
        self.with(|w| {
            w.visible = true;
            w.focused = false;
            w.log.push("show inactive");
        });
    }

    fn hide(&self) {
        self.with(|w| {
            w.visible = false;
            w.focused = false;
            w.log.push("hide");
        });
    }

    fn work_area_under_pointer(&self) -> Rect {
        WORK
    }

    fn any_window_focused(&self) -> bool {
        let w = self.get();
        w.board_focused || w.focused
    }

    fn pick_files(&self) -> BoxFuture<'_, Option<Vec<String>>> {
        let picked = self.picked.lock().unwrap().clone();
        Box::pin(async move { picked })
    }
}

#[derive(Default)]
struct FakeClipboard(Mutex<Option<String>>);

impl Clipboard for FakeClipboard {
    fn text(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }

    fn image(&self) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Default)]
struct FakeScreen {
    front: Mutex<Option<FrontApp>>,
    tab: Mutex<Option<TabAttachment>>,
    clipboard: FakeClipboard,
    shot: Mutex<Option<Screenshot>>,
    asked: Mutex<Vec<&'static str>>,
}

impl Screen for FakeScreen {
    fn front_app(&self) -> BoxFuture<'_, Option<FrontApp>> {
        self.asked.lock().unwrap().push("front app");
        let front = self.front.lock().unwrap().clone();
        Box::pin(async move { front })
    }

    fn browser_tab<'a>(&'a self, _front: &'a FrontApp) -> BoxFuture<'a, Option<TabAttachment>> {
        self.asked.lock().unwrap().push("tab");
        let tab = self.tab.lock().unwrap().clone();
        Box::pin(async move { tab })
    }

    fn clipboard(&self) -> &dyn Clipboard {
        self.asked.lock().unwrap().push("clipboard");
        &self.clipboard
    }

    fn screenshot<'a>(&'a self, _user_data: &'a Path) -> BoxFuture<'a, Screenshot> {
        self.asked.lock().unwrap().push("screenshot");
        let shot = self.shot.lock().unwrap().clone().unwrap_or(Screenshot::None);
        Box::pin(async move { shot })
    }
}

#[derive(Default)]
struct FakePermissions {
    trusted: AtomicBool,
    requests: AtomicU32,
    screen_panes: AtomicU32,
}

impl Permissions for FakePermissions {
    fn accessibility_trusted(&self) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }

    /// As nodus-platform's: nothing when already trusted.
    fn request_accessibility(&self) -> BoxFuture<'_, bool> {
        let asked = !self.accessibility_trusted();
        if asked {
            self.requests.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async move { asked })
    }

    fn open_screen_capture_settings(&self) {
        self.screen_panes.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct FakeHook {
    fail: AtomicBool,
    fail_stop: AtomicBool,
    starts: AtomicU32,
    stops: Arc<AtomicU32>,
    sink: Mutex<Option<HookSink>>,
}

struct Running {
    stops: Arc<AtomicU32>,
    fail: bool,
}

impl RunningHook for Running {
    fn stop(self: Box<Self>) -> Result<(), String> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if self.fail { Err("stuck".into()) } else { Ok(()) }
    }
}

impl KeyHook for FakeHook {
    fn start(&self, sink: HookSink) -> Result<Box<dyn RunningHook>, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err("no hook here".into());
        }
        *self.sink.lock().unwrap() = Some(sink);
        Ok(Box::new(Running {
            stops: self.stops.clone(),
            fail: self.fail_stop.load(Ordering::SeqCst),
        }))
    }
}

/// No Claude Code installed.
struct NoClaude;

impl ClaudeEnv for NoClaude {
    fn claude(&self) -> Result<PathBuf, ClaudeNotFound> {
        Err(ClaudeNotFound)
    }

    fn spawn_env(&self) -> EnvMap {
        EnvMap::new()
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    capture: Arc<Capture>,
    panel: Arc<FakePanel>,
    screen: Arc<FakeScreen>,
    permissions: Arc<FakePermissions>,
    hook: Arc<FakeHook>,
    handlers: Arc<Handlers>,
    events: Arc<RecordingSink>,
    user_data: PathBuf,
}

impl Setup {
    fn new(raw: &Value) -> Self {
        Self::with_claude(raw, Arc::new(NoClaude))
    }

    fn with_claude(raw: &Value, claude: Arc<dyn ClaudeEnv>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(store_with(dir.path(), raw));
        let user_data = dir.path().join("userdata");
        let panel = Arc::new(FakePanel::default());
        let screen = Arc::new(FakeScreen::default());
        let permissions = Arc::new(FakePermissions::default());
        permissions.trusted.store(true, Ordering::SeqCst);
        let hook = Arc::new(FakeHook::default());
        let handlers = Arc::new(Handlers::default());
        let events = Arc::new(RecordingSink::default());
        let capture = Capture::new(CaptureDeps {
            store,
            user_data: user_data.clone(),
            handlers: handlers.clone(),
            sessions: Arc::new(FakeSessions::default()),
            panel: panel.clone(),
            screen: screen.clone(),
            permissions: permissions.clone(),
            hook: hook.clone(),
            claude,
            events: events.clone(),
            runtime: tokio::runtime::Handle::current(),
            clock: Arc::new(|| NOW),
        });
        Self {
            _dir: dir,
            capture,
            panel,
            screen,
            permissions,
            hook,
            handlers,
            events,
            user_data,
        }
    }

    fn set_capture_setting(&self, on: bool) {
        self.capture.deps.store.update(|s| s.settings.capture = on);
    }
}

/// Lets spawned tasks and timers run, `ms` of paused time.
async fn pass(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[test]
fn places_the_panel_a_fifth_of_the_way_down() {
    // CAP-008: centred, rounded, the card's top at 20 % of the work area.
    assert_eq!(
        place(WORK, 333.0),
        Rect {
            x: 430.0,
            y: js::round(25.0 + 920.0 * 0.2 - 40.0),
            w: 652.0,
            h: 333.0
        }
    );
    let odd = Rect {
        x: -1920.0,
        y: 0.5,
        w: 1281.0,
        h: 801.0,
    };
    assert_eq!(place(odd, 320.0).x, js::round(-1920.0 + (1281.0 - 652.0) / 2.0));
    assert_eq!(place(odd, 320.0).y, js::round(0.5 + 801.0 * 0.2 - 40.0));
}

#[test]
fn resizes_to_what_the_page_asks() {
    // CAP-009
    let bounds = Rect {
        x: 10.0,
        y: 20.0,
        w: 652.0,
        h: 320.0,
    };
    assert_eq!(resized(bounds, 400.4), Some(Rect { h: 400.0, ..bounds }));
    assert_eq!(resized(bounds, 400.5), Some(Rect { h: 401.0, ..bounds }));
    assert_eq!(resized(bounds, 5.0), Some(Rect { h: 120.0, ..bounds }));
    assert_eq!(resized(bounds, 5000.0), Some(Rect { h: 900.0, ..bounds }));
    assert_eq!(resized(bounds, 319.6), None);
    assert_eq!(resized(bounds, f64::NAN), None);
    assert_eq!(resized(bounds, f64::INFINITY), None);
}

#[test]
fn counts_blurs_only_when_they_mean_it() {
    // CAP-012
    let late = Duration::from_millis(250);
    assert!(blur_counts(false, false, late));
    assert!(!blur_counts(false, false, Duration::from_millis(249)));
    assert!(!blur_counts(true, false, late));
    assert!(!blur_counts(false, true, late));
}

/// CAP-063, CAP-038, CAP-034, CAP-036: registering sweeps captures/, starts
/// the hook when allowed, and makes the panel 3 s later.
#[tokio::test(start_paused = true)]
async fn registers_and_starts_the_hook() {
    let s = Setup::new(&seeded());
    // Before registration, nothing starts.
    s.capture.sync_shortcut();
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 0);
    s.capture.register();
    assert!(s.user_data.join("captures").is_dir());
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 1);
    assert_eq!(s.capture.shortcut_state(), CaptureShortcut::Ready);
    // Starting again while it runs does nothing.
    s.capture.sync_shortcut();
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 1);
    pass(2999).await;
    assert!(!s.panel.get().exists);
    pass(2).await;
    let window = s.panel.get();
    assert!(window.exists && !window.visible);
}

/// CAP-032: off, unavailable, needs permission, ready, in that order.
#[tokio::test(start_paused = true)]
async fn says_how_the_shortcut_is() {
    let s = Setup::new(&seeded());
    s.hook.fail.store(true, Ordering::SeqCst);
    s.permissions.trusted.store(false, Ordering::SeqCst);
    s.capture.register();
    assert_eq!(s.capture.shortcut_state(), CaptureShortcut::NeedsPermission);
    s.permissions.trusted.store(true, Ordering::SeqCst);
    s.capture.sync_shortcut();
    // CAP-034: the hook failed, for the rest of the run.
    assert_eq!(s.capture.shortcut_state(), CaptureShortcut::Unavailable);
    s.capture.sync_shortcut();
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 1);
    s.set_capture_setting(false);
    assert_eq!(s.capture.shortcut_state(), CaptureShortcut::Off);
}

/// CAP-037, CAP-165: without the permission it polls every 3 s and starts
/// the hook the first time it's there; the setting off stops it.
#[tokio::test(start_paused = true)]
async fn polls_for_the_permission() {
    let s = Setup::new(&seeded());
    s.permissions.trusted.store(false, Ordering::SeqCst);
    s.capture.register();
    pass(9100).await;
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 0);
    s.permissions.trusted.store(true, Ordering::SeqCst);
    pass(3000).await;
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 1);
    assert!(lock(&s.capture.shortcut).poll.is_none());
    s.set_capture_setting(false);
    s.capture.sync_shortcut();
    assert_eq!(s.hook.stops.load(Ordering::SeqCst), 1);
    assert_eq!(s.capture.shortcut_state(), CaptureShortcut::Off);
    // A sync with the setting off and no permission doesn't poll.
    s.permissions.trusted.store(false, Ordering::SeqCst);
    s.capture.sync_shortcut();
    assert!(lock(&s.capture.shortcut).poll.is_none());
}

/// CAP-039, CAP-035: quitting stops the poll and the hook; a hook that
/// won't stop is let go.
#[tokio::test(start_paused = true)]
async fn stops_on_quit() {
    let s = Setup::new(&seeded());
    s.hook.fail_stop.store(true, Ordering::SeqCst);
    s.capture.register();
    s.capture.stop();
    assert_eq!(s.hook.stops.load(Ordering::SeqCst), 1);
    assert!(lock(&s.capture.shortcut).hook.is_none());
    s.capture.stop();
    assert_eq!(s.hook.stops.load(Ordering::SeqCst), 1);

    let s = Setup::new(&seeded());
    s.permissions.trusted.store(false, Ordering::SeqCst);
    s.capture.register();
    s.capture.stop();
    s.permissions.trusted.store(true, Ordering::SeqCst);
    pass(10_000).await;
    assert_eq!(s.hook.starts.load(Ordering::SeqCst), 0);
}

/// CAP-040: Allow… asks macOS and polls when it isn't trusted; either way
/// it answers the state.
#[tokio::test(start_paused = true)]
async fn asks_for_the_permission() {
    let s = Setup::new(&seeded());
    s.permissions.trusted.store(false, Ordering::SeqCst);
    s.capture.register();
    assert_eq!(s.capture.allow().await, CaptureShortcut::NeedsPermission);
    assert_eq!(s.permissions.requests.load(Ordering::SeqCst), 1);
    assert!(lock(&s.capture.shortcut).poll.is_some());
    s.permissions.trusted.store(true, Ordering::SeqCst);
    assert_eq!(s.capture.allow().await, CaptureShortcut::Ready);
    assert_eq!(s.permissions.requests.load(Ordering::SeqCst), 1);
}

fn shown(events: &RecordingSink) -> Vec<Event> {
    events
        .take()
        .into_iter()
        .filter(|e| matches!(e, Event::CaptureShown(_) | Event::CaptureContext(_)))
        .collect()
}

fn safari() -> FrontApp {
    FrontApp {
        name: "Safari".into(),
        bundle_id: "com.apple.Safari".into(),
    }
}

fn tab() -> TabAttachment {
    TabAttachment {
        url: "https://example.com/issues/1".into(),
        title: "Issue".into(),
        app: "Safari".into(),
    }
}

/// CAP-015, CAP-014: the order a panel opens in, and what its page is told.
#[tokio::test(start_paused = true)]
async fn opens_the_panel_over_the_app_in_front() {
    let s = Setup::new(&seeded());
    *s.screen.front.lock().unwrap() = Some(safari());
    *s.screen.tab.lock().unwrap() = Some(tab());
    *s.screen.clipboard.0.lock().unwrap() = Some(" copied ".into());
    s.capture.show().await;
    assert_eq!(*s.screen.asked.lock().unwrap(), ["front app", "clipboard", "tab"]);
    let window = s.panel.get();
    assert_eq!(window.log, ["create", "show+focus"]);
    assert_eq!(window.bounds, Some(place(WORK, PANEL_H)));
    assert_eq!(
        shown(&s.events),
        vec![
            Event::CaptureShown(CaptureContext {
                app: Some("Safari".into()),
                tab: None,
                clipboard: None,
                shortcut: CaptureShortcut::Ready,
            }),
            Event::CaptureContext(CaptureContextPatch {
                clipboard: Some(ClipboardOffer::Text { text: "copied".into() }),
                ..Default::default()
            }),
            Event::CaptureContext(CaptureContextPatch {
                tab: Some(tab()),
                ..Default::default()
            }),
        ]
    );
    // `ready` answers the whole of it while it's open.
    assert_eq!(
        s.capture.ready(),
        Some(CaptureContext {
            app: Some("Safari".into()),
            tab: Some(tab()),
            clipboard: Some(ClipboardOffer::Text { text: "copied".into() }),
            shortcut: CaptureShortcut::Ready,
        })
    );
    s.capture.hide();
    assert_eq!(s.capture.ready(), None);
}

/// CAP-168, CAP-014: over a Nodus window there's no app and no tab; a page
/// still loading is told nothing, and asks.
#[tokio::test(start_paused = true)]
async fn opens_over_nodus_and_while_loading() {
    let s = Setup::new(&seeded());
    *s.screen.front.lock().unwrap() = Some(safari());
    s.panel.with(|w| {
        w.board_focused = true;
        w.loading = true;
    });
    *s.screen.clipboard.0.lock().unwrap() = Some("x".into());
    s.capture.show().await;
    assert_eq!(*s.screen.asked.lock().unwrap(), ["clipboard"]);
    assert!(shown(&s.events).is_empty());
    assert_eq!(
        s.capture.ready(),
        Some(CaptureContext {
            app: None,
            tab: None,
            clipboard: Some(ClipboardOffer::Text { text: "x".into() }),
            shortcut: CaptureShortcut::Ready,
        })
    );
}

/// CAP-017, CAP-019, CAP-131: the double tap puts away a focused panel,
/// and brings back one resting with its receipt.
#[tokio::test(start_paused = true)]
async fn toggles_and_rests() {
    let s = Setup::new(&seeded());
    s.capture.toggle().await;
    assert!(s.panel.get().focused);
    s.capture.toggle().await;
    assert!(!s.panel.get().visible);
    s.capture.toggle().await;
    s.panel.log();
    // A capture made rests the panel: hidden and shown without the keyboard.
    let receipt = s
        .capture
        .submit(&json!({ "text": "a note", "kind": "note", "projectId": "p" }))
        .await
        .unwrap();
    assert_eq!(receipt.kind, CaptureKind::Note);
    assert_eq!(s.panel.log(), ["hide", "show inactive"]);
    assert!(s.capture.is_open());
    assert!(lock(&s.capture.state).resting);
    // Resting, a blur doesn't hide it.
    pass(1000).await;
    s.capture.on_blur().await;
    assert!(s.capture.is_open());
    // The double tap brings it back with the keyboard.
    s.capture.toggle().await;
    assert!(s.panel.get().focused);
    assert!(!lock(&s.capture.state).resting);
    // A failed capture doesn't rest it.
    s.panel.log();
    let err = s.capture.submit(&json!({ "text": " " })).await.unwrap_err();
    assert_eq!(err, SubmitError::NoText);
    assert!(s.panel.log().is_empty());
    // CAP-162: made while hidden, it doesn't come back.
    s.capture.hide();
    s.panel.log();
    s.capture
        .submit(&json!({ "text": "another", "kind": "note", "projectId": "p" }))
        .await
        .unwrap();
    assert!(s.panel.log().is_empty());
}

/// CAP-012: clicking away hides it, unless it's too soon, it's busy, or it
/// has the keyboard again 50 ms later.
#[tokio::test(start_paused = true)]
async fn hides_on_a_blur_that_counts() {
    let s = Setup::new(&seeded());
    s.capture.show().await;
    s.panel.with(|w| w.focused = false);
    // Within 250 ms of showing: left over from before.
    pass(249).await;
    s.capture.on_blur().await;
    assert!(s.capture.is_open());
    pass(1).await;
    // Focused again within the 50 ms: stays.
    let capture = s.capture.clone();
    let blur = tokio::spawn(async move { capture.on_blur().await });
    pass(10).await;
    s.panel.with(|w| w.focused = true);
    blur.await.unwrap();
    assert!(s.capture.is_open());
    // Still unfocused 50 ms later: hidden.
    s.panel.with(|w| w.focused = false);
    s.capture.on_blur().await;
    assert!(!s.capture.is_open());
}

/// CAP-058, CAP-020: the panel hidden, 150 ms, the shot, and the panel back
/// with the keyboard; blurs meanwhile don't hide it.
#[tokio::test(start_paused = true)]
async fn takes_a_screenshot_aside() {
    let s = Setup::new(&seeded());
    let image = ImageAttachment {
        path: "/captures/x.png".into(),
        thumb: "data:".into(),
        copied: None,
    };
    *s.screen.shot.lock().unwrap() = Some(Screenshot::Image(image.clone()));
    s.capture.show().await;
    s.panel.log();
    let capture = s.capture.clone();
    let shot = tokio::spawn(async move { capture.screenshot().await });
    pass(100).await;
    assert!(lock(&s.capture.state).busy);
    assert!(!s.screen.asked.lock().unwrap().contains(&"screenshot"));
    s.capture.on_blur().await;
    pass(60).await;
    assert_eq!(shot.await.unwrap(), Screenshot::Image(image));
    assert_eq!(s.panel.log(), ["hide", "show+focus"]);
    assert!(!lock(&s.capture.state).busy);
    // CAP-059
    s.capture.allow_screen();
    assert_eq!(s.permissions.screen_panes.load(Ordering::SeqCst), 1);
}

/// CAP-060: dropped paths, or the dialog aside.
#[tokio::test(start_paused = true)]
async fn attaches_files() {
    let s = Setup::new(&seeded());
    let file = s.user_data.join("state.json").to_string_lossy().into_owned();
    let dropped = s.capture.files(&json!([file, "relative.txt", 5])).await;
    assert_eq!(dropped.len(), 1);
    assert!(s.panel.log().is_empty());
    // Cancelled.
    assert!(s.capture.files(&Value::Null).await.is_empty());
    assert_eq!(s.panel.log(), ["create", "show+focus"]);
    *s.panel.picked.lock().unwrap() = Some(vec![file.clone(), "/no/such/file".into()]);
    let picked = s.capture.files(&json!("not a list")).await;
    assert_eq!(picked.len(), 1);
}

/// CAP-054: the clipboard taken afresh.
#[tokio::test(start_paused = true)]
async fn takes_the_clipboard() {
    let s = Setup::new(&seeded());
    *s.screen.clipboard.0.lock().unwrap() = Some("  text ".into());
    assert_eq!(
        s.capture.clipboard(),
        Some(CaptureAttachment::Text(nodus_domain::TextAttachment {
            text: "text".into()
        }))
    );
}

/// CAP-009: resizing needs a panel.
#[tokio::test(start_paused = true)]
async fn resizes_the_panel() {
    let s = Setup::new(&seeded());
    s.capture.resize(500.0);
    assert!(!s.panel.get().exists);
    s.capture.show().await;
    let before = s.panel.get().bounds.unwrap();
    s.capture.resize(1000.0);
    assert_eq!(s.panel.get().bounds, Some(Rect { h: 900.0, ..before }));
    s.capture.resize(f64::NAN);
    assert_eq!(s.panel.get().bounds.unwrap().h, 900.0);
}

/// CAP-145, CAP-159, CAP-160: open, settings and hide all hide the panel first.
#[tokio::test(start_paused = true)]
async fn opens_cards_and_settings() {
    let s = Setup::new(&seeded());
    s.capture.show().await;
    s.capture.open(&json!({ "projectId": "p", "sessionId": "s1" }));
    assert!(!s.capture.is_open());
    s.capture.open(&json!({ "projectId": 5 }));
    s.capture.open(&json!(null));
    s.capture.show().await;
    s.capture.settings();
    assert!(!s.capture.is_open());
    assert_eq!(
        s.handlers.calls(),
        vec![
            Call::OpenCard(OpenCardRequest {
                project_id: "p".into(),
                session_id: Some("s1".into())
            }),
            Call::OpenSettings,
        ]
    );
}

/// CAP-154, CAP-149: the board opened last is the default before any
/// capture or opening says otherwise; targets refresh what guesses go on.
#[tokio::test(start_paused = true)]
async fn remembers_the_board_opened_last() {
    let raw = json!({
        "projects": [
            project("a", "a", json!({})),
            project("b", "b", json!({})),
        ],
    });
    let s = Setup::new(&raw);
    assert_eq!(s.capture.targets().default_project_id.as_deref(), Some("a"));
    s.capture.board_opened("b");
    assert_eq!(s.capture.targets().default_project_id.as_deref(), Some("b"));
    // Projects without boards got empty ones.
    assert!(s.capture.deps.store.read(|st| st.board("a").is_some()));
}

/// CAP-068, CAP-016, CAP-161: a guess with nothing to go on is none; with
/// no Claude Code it fails; hiding stops a guess.
#[tokio::test(start_paused = true)]
async fn guesses_through_the_panel() {
    let s = Setup::new(&seeded());
    assert_eq!(s.capture.guess("  ", &json!([])).await, Ok(None));
    assert_eq!(s.capture.guess("fix it", &json!([])).await, Err(ClaudeNotFound));
}

/// CAP-031: the hook's double tap toggles the panel.
#[tokio::test(start_paused = true)]
async fn the_double_tap_toggles_the_panel() {
    let s = Setup::new(&seeded());
    s.capture.register();
    let sink = s.hook.sink.lock().unwrap().clone().unwrap();
    for event in [
        HookEvent::KeyDown(42),
        HookEvent::KeyUp(42),
        HookEvent::KeyDown(54),
        HookEvent::KeyUp(54),
    ] {
        sink(event);
    }
    pass(1).await;
    assert!(s.capture.is_open());
}

// The guess goes on what the panel opened over: a stand-in claude logs it.
#[cfg(unix)]
#[tokio::test]
async fn guesses_on_the_app_the_panel_opened_over() {
    use std::os::unix::fs::PermissionsExt;

    struct StandIn(PathBuf);

    impl ClaudeEnv for StandIn {
        fn claude(&self) -> Result<PathBuf, ClaudeNotFound> {
            Ok(self.0.clone())
        }

        fn spawn_env(&self) -> EnvMap {
            EnvMap::from([("PATH".into(), "/usr/bin:/bin".into())])
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do last=\"$a\"; done\nprintf '%s' \"$last\" > '{}/prompt.log'\necho '{{\"structured_output\":{{\"kind\":\"note\",\"project\":\"p\",\"session\":\"\",\"reason\":\"An idea.\"}}}}'\n",
            dir.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = Setup::with_claude(&seeded(), Arc::new(StandIn(script)));
    *s.screen.front.lock().unwrap() = Some(safari());
    s.capture.show().await;
    let attachments =
        json!([{ "kind": "tab", "url": "https://example.com/x", "title": "X", "app": "Safari", "thumb": "nope" }]);
    let guess = s
        .capture
        .guess("remember the thing", &attachments)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((guess.kind, guess.project_id.as_str()), (CaptureKind::Note, "p"));
    let prompt = std::fs::read_to_string(dir.path().join("prompt.log")).unwrap();
    assert!(
        prompt.contains("The app in front: Safari\nIts browser tab: X (https://example.com/x)"),
        "{prompt}"
    );
}
