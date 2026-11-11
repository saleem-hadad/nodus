//! The capture service: the panel coming up and going, the double tap and
//! its permission, and the `capture:*` commands. The rest of
//! `src/main/capture.ts`.
//!
//! The panel is a window the UI makes (an NSPanel on macOS) and drives
//! through [`PanelWindow`]; this decides when it opens, where, what its page
//! is told, and when it hides. Its look and its window behaviors (level,
//! Spaces, focus without activating, first mouse) are the UI's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use nodus_board::BoxFuture;
use nodus_domain::js;
use nodus_domain::{
    BubbleGuess, CaptureAttachment, CaptureContext, CaptureContextPatch, CaptureGuess, CaptureReceipt, CaptureShortcut,
    CaptureTargets, ClipboardOffer, Event, EventSink, OpenCardRequest, Rect, TabAttachment,
};
use nodus_platform::ClaudeNotFound;
use nodus_store::Store;
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::guess::{ClaudeEnv, GuessInput, Guesser};
use crate::hook::{HookSink, KeyHook, Listener, RunningHook};
use crate::screen::{self, Clipboard, FrontApp, Screenshot};
use crate::submit::{CaptureHandlers, MAX_TEXT_CHARS, Maker, SubmitError, clean_attachments};
use crate::targets::{self, CaptureSessions, missing_boards};

/// The panel's width: the card's, with a see-through margin its shadow fits
/// in (CAP-007).
pub const PANEL_W: f64 = 652.0;
/// Its height until the page says how tall it is (CAP-007).
pub const PANEL_H: f64 = 320.0;
/// The heights the page may ask for (CAP-009).
pub const MIN_PANEL_H: f64 = 120.0;
pub const MAX_PANEL_H: f64 = 900.0;
/// From the window's top to the card's (CAP-008).
pub const CARD_TOP: f64 = 40.0;
/// How far down the screen the card's top sits (CAP-008).
pub const PANEL_TOP: f64 = 0.2;
/// A blur this soon after the panel came up is left over from before (CAP-012).
pub const BLUR_GRACE: Duration = Duration::from_millis(250);
/// A blur counts only if the panel still hasn't the keyboard this much later (CAP-012).
pub const BLUR_RECHECK: Duration = Duration::from_millis(50);
/// The panel is made this long after the hook starts, so the first double
/// tap doesn't wait for it (CAP-036).
pub const PRECREATE_AFTER: Duration = Duration::from_millis(3000);
/// Until macOS trusts Nodus, it checks this often (CAP-037).
pub const PERMISSION_POLL: Duration = Duration::from_millis(3000);
/// How long the hidden panel gets to leave the screen before a shot (CAP-058).
pub const SCREENSHOT_SETTLE: Duration = Duration::from_millis(150);

/// Where the panel goes on a display (CAP-008): centred, its card a fifth of
/// the way down the work area, as tall as it was.
pub fn place(work_area: Rect, height: f64) -> Rect {
    Rect {
        x: js::round(work_area.x + (work_area.w - PANEL_W) / 2.0),
        y: js::round(work_area.y + work_area.h * PANEL_TOP - CARD_TOP),
        w: PANEL_W,
        h: height,
    }
}

/// The panel's bounds once the page asks for `height` (CAP-009): clamped,
/// rounded, the top, left and width kept. None when nothing changes, or the
/// height isn't a number.
pub fn resized(bounds: Rect, height: f64) -> Option<Rect> {
    if !height.is_finite() {
        return None;
    }
    let h = js::round(height.clamp(MIN_PANEL_H, MAX_PANEL_H));
    (h != bounds.h).then_some(Rect { h, ..bounds })
}

/// Whether a blur is worth checking again 50 ms later (CAP-012): not while a
/// screenshot or file dialog is up, nor while the receipt rests, nor within
/// 250 ms of the panel coming up.
pub fn blur_counts(busy: bool, resting: bool, since_shown: Duration) -> bool {
    !busy && !resting && since_shown >= BLUR_GRACE
}

/// The panel's window, as the UI makes it. Its methods may be called from
/// any thread; the UI carries them to its own.
pub trait PanelWindow: Send + Sync {
    /// Whether the window exists (it was made and not closed since).
    fn exists(&self) -> bool;
    /// Makes the window, hidden, `PANEL_W` × `PANEL_H`, if it doesn't exist (CAP-001, CAP-002).
    fn create(&self);
    fn is_visible(&self) -> bool;
    /// Whether it has the keyboard.
    fn is_focused(&self) -> bool;
    /// Its page is still loading: it isn't told anything, and asks with `ready` (CAP-014).
    fn is_loading(&self) -> bool;
    fn bounds(&self) -> Rect;
    fn set_bounds(&self, bounds: Rect);
    /// Shows it and gives it the keyboard, without bringing Nodus forward.
    fn show_and_focus(&self);
    /// Shows it without the keyboard (Electron's `showInactive`).
    fn show_inactive(&self);
    fn hide(&self);
    /// The work area (without menu bar, Dock or taskbar) of the display
    /// nearest the pointer.
    fn work_area_under_pointer(&self) -> Rect;
    /// Whether any Nodus window has the keyboard, the panel included.
    fn any_window_focused(&self) -> bool;
    /// The open dialog for files (several, no folders), attached to the
    /// panel. None when cancelled (CAP-060).
    fn pick_files(&self) -> BoxFuture<'_, Option<Vec<String>>>;
}

/// What comes from the user's screen. [`SystemScreen`] is the real one.
pub trait Screen: Send + Sync {
    /// The app in front (CAP-045).
    fn front_app(&self) -> BoxFuture<'_, Option<FrontApp>>;
    /// Its browser tab (CAP-050).
    fn browser_tab<'a>(&'a self, front: &'a FrontApp) -> BoxFuture<'a, Option<TabAttachment>>;
    /// The clipboard.
    fn clipboard(&self) -> &dyn Clipboard;
    /// A screenshot saved in `<user_data>/captures/` (CAP-055, CAP-056).
    fn screenshot<'a>(&'a self, user_data: &'a Path) -> BoxFuture<'a, Screenshot>;
}

/// The user's screen: `lsappinfo`, `osascript`, the clipboard and `screencapture`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemScreen;

impl Screen for SystemScreen {
    fn front_app(&self) -> BoxFuture<'_, Option<FrontApp>> {
        Box::pin(screen::front_app())
    }

    fn browser_tab<'a>(&'a self, front: &'a FrontApp) -> BoxFuture<'a, Option<TabAttachment>> {
        Box::pin(screen::browser_tab(front))
    }

    fn clipboard(&self) -> &dyn Clipboard {
        &screen::SystemClipboard
    }

    fn screenshot<'a>(&'a self, user_data: &'a Path) -> BoxFuture<'a, Screenshot> {
        Box::pin(screen::take_screenshot(user_data))
    }
}

/// The permissions capture needs on macOS. [`SystemPermissions`] is the real one.
pub trait Permissions: Send + Sync {
    /// Whether macOS trusts Nodus with Accessibility, asked without a prompt (CAP-032).
    fn accessibility_trusted(&self) -> bool;
    /// Allow…'s OS steps (CAP-040, CAP-041); whether it did anything.
    fn request_accessibility(&self) -> BoxFuture<'_, bool>;
    /// Opens the Screen Recording pane (CAP-059).
    fn open_screen_capture_settings(&self);
}

/// `nodus_platform::permissions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPermissions;

impl Permissions for SystemPermissions {
    fn accessibility_trusted(&self) -> bool {
        nodus_platform::permissions::accessibility_trusted(false)
    }

    fn request_accessibility(&self) -> BoxFuture<'_, bool> {
        Box::pin(nodus_platform::permissions::request_accessibility())
    }

    fn open_screen_capture_settings(&self) {
        nodus_platform::permissions::open_screen_capture_settings();
    }
}

/// Everything capture reaches.
pub struct CaptureDeps {
    pub store: Arc<Store>,
    /// `<userData>`, where `captures/` is.
    pub user_data: PathBuf,
    pub handlers: Arc<dyn CaptureHandlers>,
    pub sessions: Arc<dyn CaptureSessions>,
    pub panel: Arc<dyn PanelWindow>,
    pub screen: Arc<dyn Screen>,
    pub permissions: Arc<dyn Permissions>,
    pub hook: Arc<dyn KeyHook>,
    pub claude: Arc<dyn ClaudeEnv>,
    pub events: Arc<dyn EventSink>,
    /// Where the double tap, the permission poll and timers run.
    pub runtime: tokio::runtime::Handle,
    /// `Date.now()`.
    pub clock: nodus_board::Clock,
}

/// What the panel opened onto, for the guesses (CAP-067).
struct OpenedOn {
    app: Option<String>,
    targets: CaptureTargets,
}

#[derive(Default)]
struct PanelState {
    /// A screenshot or a file dialog is up: losing focus to it doesn't hide the panel.
    busy: bool,
    /// The receipt is up, without the keyboard, which went back to the app the user was in.
    resting: bool,
    /// When the panel last came up.
    shown_at: Option<Instant>,
    /// The board open last in the window, for where a capture goes before Claude has guessed.
    last_board: Option<String>,
    opened_on: Option<OpenedOn>,
    /// What the page was last told it's up over, for a page that loads after being told.
    shown_context: Option<CaptureContext>,
}

#[derive(Default)]
struct ShortcutState {
    /// Capture is registered: settings applied before then don't start the hook.
    registered: bool,
    hook: Option<Box<dyn RunningHook>>,
    /// The hook couldn't start; not tried again until relaunch (CAP-034).
    failed: bool,
    poll: Option<JoinHandle<()>>,
}

/// Floating capture.
pub struct Capture {
    deps: CaptureDeps,
    guesser: Guesser,
    /// The board bubble's guesses, apart from the panel's.
    bubble: Guesser,
    state: Mutex<PanelState>,
    shortcut: Mutex<ShortcutState>,
    me: Weak<Capture>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Capture {
    pub fn new(deps: CaptureDeps) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            guesser: Guesser::new(deps.claude.clone()),
            bubble: Guesser::new(deps.claude.clone()),
            deps,
            state: Mutex::new(PanelState::default()),
            shortcut: Mutex::new(ShortcutState::default()),
            me: me.clone(),
        })
    }

    /// `registerCapture`: clears away old captures (CAP-063), then matches
    /// the hook to the setting and the permission (CAP-038).
    pub fn register(&self) {
        screen::prune_shots(&self.deps.user_data, (self.deps.clock)());
        lock(&self.shortcut).registered = true;
        self.sync_shortcut();
    }

    // ---- the panel -------------------------------------------------------

    /// "Open" is visible, resting with the receipt included (CAP-018).
    pub fn is_open(&self) -> bool {
        self.deps.panel.exists() && self.deps.panel.is_visible()
    }

    /// `getPanel`: the window, made if it isn't there (CAP-001).
    fn panel(&self) -> &dyn PanelWindow {
        let panel = &*self.deps.panel;
        if !panel.exists() {
            panel.create();
        }
        panel
    }

    /// Tells the page what it's up over (CAP-014): a show replaces what it
    /// was told, an update adds to it. A page still loading is told nothing,
    /// and asks with [`ready`](Self::ready).
    fn tell_shown(&self, context: CaptureContext) {
        lock(&self.state).shown_context = Some(context.clone());
        if !self.deps.panel.is_loading() {
            self.deps.events.emit(Event::CaptureShown(context));
        }
    }

    fn tell_more(&self, patch: CaptureContextPatch) {
        if let Some(shown) = lock(&self.state).shown_context.as_mut() {
            if let Some(app) = &patch.app {
                shown.app = Some(app.clone());
            }
            if let Some(tab) = &patch.tab {
                shown.tab = Some(tab.clone());
            }
            if let Some(clipboard) = &patch.clipboard {
                shown.clipboard = Some(clipboard.clone());
            }
            if let Some(shortcut) = patch.shortcut {
                shown.shortcut = shortcut;
            }
        }
        if !self.deps.panel.is_loading() {
            self.deps.events.emit(Event::CaptureContext(patch));
        }
    }

    /// `openPanel` (CAP-015, CAP-168): floats the panel up over whatever app
    /// is in front, then tells it what's on the clipboard, then the tab.
    pub async fn open_panel(&self) {
        // Read before the panel shows: Nodus may be the app in front, but the panel never is.
        let own = self.deps.panel.any_window_focused();
        let front = if own { None } else { self.deps.screen.front_app().await };
        let panel = self.panel();
        let height = panel.bounds().h;
        panel.set_bounds(place(panel.work_area_under_pointer(), height));
        let app = front.as_ref().map(|f| f.name.clone());
        let targets = self.targets_now();
        lock(&self.state).opened_on = Some(OpenedOn {
            app: app.clone(),
            targets,
        });
        self.tell_shown(CaptureContext {
            app,
            tab: None,
            clipboard: None,
            shortcut: self.shortcut_state(),
        });
        {
            let mut state = lock(&self.state);
            state.shown_at = Some(Instant::now());
            state.resting = false;
        }
        panel.show_and_focus();
        // What takes longer arrives after the panel is up: the clipboard, then the tab.
        let clipboard: Option<ClipboardOffer> = screen::read_clipboard(self.deps.screen.clipboard());
        if let Some(clipboard) = clipboard
            && self.is_open()
        {
            self.tell_more(CaptureContextPatch {
                clipboard: Some(clipboard),
                ..Default::default()
            });
        }
        if let Some(front) = front.filter(|f| screen::is_browser(&f.bundle_id))
            && let Some(tab) = self.deps.screen.browser_tab(&front).await
            && self.is_open()
        {
            self.tell_more(CaptureContextPatch {
                tab: Some(tab),
                ..Default::default()
            });
        }
    }

    /// `hidePanel` (CAP-016): stops a guess in progress and hides the panel;
    /// it's never closed.
    pub fn hide_panel(&self) {
        self.guesser.cancel();
        lock(&self.state).resting = false;
        let panel = &self.deps.panel;
        if panel.exists() && panel.is_visible() {
            panel.hide();
        }
    }

    /// The double tap (CAP-017, CAP-031): puts away a panel that's open and
    /// has the keyboard; otherwise opens it, the resting receipt included.
    pub async fn toggle(&self) {
        if self.is_open() && self.deps.panel.is_focused() {
            self.hide_panel();
        } else {
            self.open_panel().await;
        }
    }

    /// Gives the keyboard back to the app the user was in, keeping the panel
    /// up for its receipt until its page hides it (CAP-019).
    fn rest(&self) {
        if !self.is_open() {
            return;
        }
        lock(&self.state).resting = true;
        self.deps.panel.hide();
        self.deps.panel.show_inactive();
    }

    /// Hides nothing itself: marks the panel busy while the user does
    /// something outside it, then brings it back with the keyboard (CAP-020).
    async fn aside<T>(&self, work: impl Future<Output = T>) -> T {
        self.panel();
        lock(&self.state).busy = true;
        let result = work.await;
        lock(&self.state).busy = false;
        if self.deps.panel.exists() {
            self.deps.panel.show_and_focus();
        }
        result
    }

    /// The panel lost the keyboard (the UI calls this on blur, CAP-012):
    /// clicking away puts it away. A blur can arrive late, from hiding it just
    /// before it was shown again, so only one that leaves it unfocused a
    /// moment later counts.
    pub async fn on_blur(&self) {
        let counts = {
            let state = lock(&self.state);
            let since = state.shown_at.map_or(Duration::MAX, |at| at.elapsed());
            blur_counts(state.busy, state.resting, since)
        };
        if !counts {
            return;
        }
        tokio::time::sleep(BLUR_RECHECK).await;
        let busy = lock(&self.state).busy;
        if !busy && self.deps.panel.exists() && !self.deps.panel.is_focused() {
            self.hide_panel();
        }
    }

    // ---- the commands ------------------------------------------------------

    /// `capture:ready` (CAP-014): what the panel is up over, for a page that
    /// loaded after it was told; None while it isn't open.
    pub fn ready(&self) -> Option<CaptureContext> {
        if self.is_open() {
            lock(&self.state).shown_context.clone()
        } else {
            None
        }
    }

    /// The targets as they are now, a project without a board given an empty one.
    fn targets_now(&self) -> CaptureTargets {
        let missing = self.deps.store.read(missing_boards);
        if !missing.is_empty() {
            self.deps.store.update(|state| {
                for id in &missing {
                    state.board_mut(id);
                }
            });
        }
        let last_board = lock(&self.state).last_board.clone();
        self.deps
            .store
            .read(|state| targets::targets(state, last_board.as_deref(), &*self.deps.sessions))
    }

    /// `capture:targets` (CAP-149): also what the next guesses go on.
    pub fn targets(&self) -> CaptureTargets {
        let targets = self.targets_now();
        if let Some(opened_on) = lock(&self.state).opened_on.as_mut() {
            opened_on.targets = targets.clone();
        }
        targets
    }

    /// `capture:guess` (CAP-067): on the targets and the app the panel
    /// opened on. Fails when Claude Code isn't installed, which the page
    /// takes as no guess.
    pub async fn guess(&self, text: &str, attachments: &Value) -> Result<Option<CaptureGuess>, ClaudeNotFound> {
        let opened = {
            let state = lock(&self.state);
            state.opened_on.as_ref().map(|o| (o.app.clone(), o.targets.clone()))
        };
        let (app, targets) = match opened {
            Some(opened) => opened,
            None => (None, self.targets_now()),
        };
        let text = js::slice_utf16(text, 0, MAX_TEXT_CHARS);
        let attachments = clean_attachments(attachments);
        let input = GuessInput {
            text: &text,
            attachments: &attachments,
            app: app.as_deref(),
            targets: &targets,
        };
        self.guesser.guess(input, (self.deps.clock)()).await
    }

    /// Claude's guess at what the board bubble's text becomes, in `project`
    /// (BUBBLE-023). Fails when Claude Code isn't installed.
    pub async fn guess_bubble(&self, text: &str, project: &str) -> Result<Option<BubbleGuess>, ClaudeNotFound> {
        self.bubble.guess_bubble(text, project).await
    }

    /// Stops the bubble's guess in progress: the bubble closed (BUBBLE-024).
    pub fn cancel_bubble(&self) {
        self.bubble.cancel();
    }

    /// `capture:submit` (CAP-125…CAP-131): makes it, then rests the panel
    /// with its receipt. A failure doesn't rest it.
    pub async fn submit(&self, raw: &Value) -> Result<CaptureReceipt, SubmitError> {
        let clock = self.deps.clock.clone();
        let now = move || clock();
        let receipt = Maker {
            store: &self.deps.store,
            handlers: &*self.deps.handlers,
            sessions: &*self.deps.sessions,
            events: &*self.deps.events,
            now: &now,
        }
        .submit(raw)
        .await?;
        self.rest();
        Ok(receipt)
    }

    /// `capture:screenshot` (CAP-058): the panel hidden, 150 ms for it to
    /// leave the screen, the shot, then the panel back with the keyboard.
    pub async fn screenshot(&self) -> Screenshot {
        self.aside(async {
            self.deps.panel.hide();
            tokio::time::sleep(SCREENSHOT_SETTLE).await;
            self.deps.screen.screenshot(&self.deps.user_data).await
        })
        .await
    }

    /// `capture:allow-screen` (CAP-059).
    pub fn allow_screen(&self) {
        self.deps.permissions.open_screen_capture_settings();
    }

    /// `capture:files(paths)` (CAP-060): dropped files' absolute paths, or,
    /// without a list, the open dialog with the panel set aside.
    pub async fn files(&self, paths: &Value) -> Vec<CaptureAttachment> {
        if let Value::Array(items) = paths {
            let paths: Vec<String> = items.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect();
            return screen::describe_dropped(&paths);
        }
        self.aside(async {
            match self.deps.panel.pick_files().await {
                Some(picked) => screen::describe_files(&picked),
                None => Vec::new(),
            }
        })
        .await
    }

    /// `capture:clipboard` (CAP-054): read afresh, attached.
    pub fn clipboard(&self) -> Option<CaptureAttachment> {
        screen::take_clipboard(self.deps.screen.clipboard(), &self.deps.user_data)
    }

    /// `capture:hide` (CAP-160).
    pub fn hide(&self) {
        self.hide_panel();
    }

    /// `capture:resize(height)` (CAP-009).
    pub fn resize(&self, height: f64) {
        let panel = &self.deps.panel;
        if !panel.exists() {
            return;
        }
        if let Some(bounds) = resized(panel.bounds(), height) {
            panel.set_bounds(bounds);
        }
    }

    /// `capture:open(request)` (CAP-145): hides the panel, and brings the
    /// board window up on the card or the board.
    pub fn open(&self, request: &Value) {
        self.hide_panel();
        if let Some(project_id) = request.get("projectId").and_then(Value::as_str) {
            self.deps.handlers.open_card(OpenCardRequest {
                project_id: project_id.to_owned(),
                session_id: request.get("sessionId").and_then(Value::as_str).map(str::to_owned),
            });
        }
    }

    /// `capture:settings` (CAP-159): hides the panel, and opens Settings in
    /// the board window.
    pub fn settings(&self) {
        self.hide_panel();
        self.deps.handlers.open_settings();
    }

    /// `capture:show` (CAP-043) and File › Capture… (CAP-042): opens, never toggles.
    pub async fn show(&self) {
        self.open_panel().await;
    }

    /// `boardOpened` (CAP-154): `board:load` tells it the board the window opened.
    pub fn board_opened(&self, project_id: &str) {
        lock(&self.state).last_board = Some(project_id.to_owned());
    }

    // ---- the double tap ----------------------------------------------------

    /// `capture:shortcut` (CAP-032, CAP-044): off, unavailable, needs
    /// permission or ready, in that order.
    pub fn shortcut_state(&self) -> CaptureShortcut {
        if !self.deps.store.read(|state| state.settings.capture) {
            return CaptureShortcut::Off;
        }
        if lock(&self.shortcut).failed {
            return CaptureShortcut::Unavailable;
        }
        if !self.deps.permissions.accessibility_trusted() {
            return CaptureShortcut::NeedsPermission;
        }
        CaptureShortcut::Ready
    }

    /// Starts the hook unless it runs or has failed (CAP-034), and makes the
    /// panel 3 s later (CAP-036).
    fn start_hook(&self) {
        let mut shortcut = lock(&self.shortcut);
        if shortcut.hook.is_some() || shortcut.failed {
            return;
        }
        let me = self.me.clone();
        let runtime = self.deps.runtime.clone();
        let listener = Arc::new(Listener::new(move || {
            let me = me.clone();
            runtime.spawn(async move {
                if let Some(capture) = me.upgrade() {
                    capture.toggle().await;
                }
            });
        }));
        let sink: HookSink = Arc::new(move |event| listener.hear(event));
        match self.deps.hook.start(sink) {
            Ok(hook) => {
                shortcut.hook = Some(hook);
                let me = self.me.clone();
                self.deps.runtime.spawn(async move {
                    tokio::time::sleep(PRECREATE_AFTER).await;
                    if let Some(capture) = me.upgrade() {
                        capture.panel();
                    }
                });
            }
            Err(err) => {
                shortcut.failed = true;
                tracing::error!("[capture] the key hook failed to start {err}");
            }
        }
    }

    /// Stops the hook; a failure is logged (CAP-035).
    fn stop_hook(&self) {
        let hook = lock(&self.shortcut).hook.take();
        if let Some(hook) = hook
            && let Err(err) = hook.stop()
        {
            tracing::error!("[capture] the key hook failed to stop {err}");
        }
    }

    /// `syncShortcut` (CAP-037, CAP-038): starts or stops listening for the
    /// double tap to match the setting and the permission. Called at
    /// registration and on every settings update.
    pub fn sync_shortcut(&self) {
        {
            let mut shortcut = lock(&self.shortcut);
            if !shortcut.registered {
                return;
            }
            if let Some(poll) = shortcut.poll.take() {
                poll.abort();
            }
        }
        if !self.deps.store.read(|state| state.settings.capture) {
            return self.stop_hook();
        }
        if self.deps.permissions.accessibility_trusted() {
            return self.start_hook();
        }
        // Until macOS trusts Nodus, check now and then: the user may be in System Settings allowing it.
        let me = self.me.clone();
        let poll = self.deps.runtime.spawn(async move {
            loop {
                tokio::time::sleep(PERMISSION_POLL).await;
                let Some(capture) = me.upgrade() else {
                    return;
                };
                if capture.deps.permissions.accessibility_trusted() {
                    lock(&capture.shortcut).poll = None;
                    capture.start_hook();
                    return;
                }
            }
        });
        lock(&self.shortcut).poll = Some(poll);
    }

    /// `capture:allow` (CAP-040, CAP-041, CAP-165): macOS asked to trust
    /// Nodus (a packaged build first clears an entry another copy left), the
    /// pane opened, and the poll started; then the state, whatever it is.
    pub async fn allow(&self) -> CaptureShortcut {
        if self.deps.permissions.request_accessibility().await {
            self.sync_shortcut();
        }
        self.shortcut_state()
    }

    /// `stopCapture` (CAP-039): on quit, the poll cleared and the hook stopped.
    pub fn stop(&self) {
        if let Some(poll) = lock(&self.shortcut).poll.take() {
            poll.abort();
        }
        self.stop_hook();
    }
}

#[cfg(test)]
mod tests;
