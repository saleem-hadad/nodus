//! The panel's page (`CaptureApp.tsx`): what's typed, what comes along,
//! where it goes and Claude's guess at it, sending, the receipt, recents and
//! the keys. Drawing is in `render.rs`, `menu.rs` and `receipt.rs`; the rules
//! (route, kinds, guesses, history, the offer key, words and numbers) are
//! nodus-capture's `page`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::component::input::{InputEvent, TextareaState};
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EntityInputHandler as _, Focusable as _, KeystrokeEvent, Pixels,
    RenderImage, Subscription, Task, Window,
};
use nodus_core::capture_service::Screenshot;
use nodus_core::capture_service::page::{
    self, GuessState, POLL_MS, RECEIPT_MS, Route, cycle_kind, history_down, history_up, offer_key, pick_kind,
    pick_project, pick_session, remember_sent, route, with_tab,
};
use nodus_core::{Nodus, VoiceController};
use nodus_domain::js;
use nodus_domain::{
    CaptureAttachment, CaptureContext, CaptureContextPatch, CaptureGuess, CaptureKind, CaptureReceipt, CaptureSession,
    CaptureShortcut, CaptureSubmission, CaptureTargets, ClipboardOffer, Event, OpenCardRequest,
};
use nodus_voice::controller::{EscTarget, Phase, VoiceSlot, VoiceState};
use tokio::sync::{mpsc, watch};

use super::bridge::PanelSlot;
use super::stubs::Stubs;

/// A guess in flight: `None` until it answers.
type Pending = watch::Receiver<Option<Option<CaptureGuess>>>;

/// The receipt, with the words that went (`Shown`).
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub receipt: CaptureReceipt,
    pub text: String,
}

/// The floating capture panel's page.
pub struct CapturePanel {
    pub(super) nodus: Nodus,
    pub(super) targets: Option<CaptureTargets>,
    pub(super) context: CaptureContext,
    pub(super) field: Entity<TextareaState>,
    /// The field's text, as typed (untrimmed).
    pub(super) text: String,
    pub(super) attachments: Vec<CaptureAttachment>,
    pub(super) guess: GuessState,
    pending: Option<Pending>,
    guess_timer: Option<Task<()>>,
    /// What the user picked, which stands over Claude's guess until the text is cleared.
    pub(super) choice: Option<Route>,
    pub(super) menu_open: bool,
    pub(super) sending: bool,
    pub(super) error: Option<String>,
    /// The last screenshot was kept from Nodus: no Screen Recording permission.
    pub(super) shot_blocked: bool,
    pub(super) receipt: Option<Shown>,
    receipt_timer: Option<Task<()>>,
    /// The key of the last clipboard offer seen.
    pub(super) offer_done: Option<String>,
    history: Vec<String>,
    /// Where ↑ is in the history; -1 while typing.
    history_at: i32,
    /// The field's placeholder now (CAP-089).
    pub(super) placeholder: &'static str,
    /// Setting the field's text from here, not from typing.
    setting_text: bool,
    /// The height the window was last asked to be (CAP-010).
    pub(super) sent_height: Rc<Cell<f64>>,
    /// The open menu's bottom, in the window, once drawn.
    pub(super) menu_bottom: Rc<Cell<Option<f32>>>,
    /// The header button's bounds, for "a click outside the menu and the header".
    pub(super) route_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Bumped each time the card is made again (after a receipt), to replay its entrance.
    pub(super) card_epoch: u64,
    /// Bumped each time the recents appear.
    pub(super) recents_epoch: u64,
    recents_shown: bool,
    pub(super) menu_epoch: u64,
    pub(super) receipt_epoch: u64,
    /// Thumbnails, decoded once.
    thumbs: HashMap<String, Option<Arc<RenderImage>>>,
    /// CAP-103: the mic (05's voice hook), the panel window's only one.
    pub(super) voice: Arc<VoiceController>,
    pub(super) voice_state: VoiceState,
    /// The project a transcript is hinted with: the route's, or none (`''`).
    voice_project: Arc<Mutex<Option<String>>>,
    _voice: Task<()>,
    _said: Task<()>,
    _poll: Task<()>,
    _events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl CapturePanel {
    /// CAP-081, CAP-082: subscribes to the panel coming up and to what
    /// arrives after, then asks whether it's already up.
    pub fn new(nodus: Nodus, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        // CAP-082, CAP-157: the draft, its attachments, what was sent and
        // the clipboard already offered, as the page last left them.
        let text: String = stored(&nodus, DRAFT).unwrap_or_default();
        let attachments: Vec<CaptureAttachment> = stored(&nodus, ATTACHMENTS).unwrap_or_default();
        let history: Vec<String> = stored(&nodus, HISTORY).unwrap_or_default();
        let offer_done: Option<String> = stored(&nodus, CLIPBOARD_DONE);
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                // CAP-089, CAP-090: 3 rows, growing to 8, then scrolling.
                .auto_grow(3, page::MAX_ROWS)
                .placeholder("Ask Claude to do something, or jot a note")
                .context_menu(false)
        });
        if !text.is_empty() {
            let value = text.clone();
            field.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        let (voice, voice_project, voice_task, said_task) = mic(&nodus, window, cx);
        let mut rx = nodus.subscribe();
        let events = cx.spawn_in(window, async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(Event::CaptureShown(context)) => {
                        if this
                            .update_in(cx, |p, window, cx| p.on_shown(context, window, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Event::CaptureContext(patch)) => {
                        if this.update(cx, |p, cx| p.on_context(patch, cx)).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        // CAP-118, CAP-177: the recents' status, every 2 s while it's up.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
                let visible = cx.update(|cx| cx.try_global::<PanelSlot>().is_some_and(PanelSlot::is_up));
                if visible && this.update(cx, Self::refresh).is_err() {
                    break;
                }
            }
        });
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |p, field, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    p.on_field_changed(value, cx);
                }
            }),
            // CAP-012: the panel lost the keyboard. The test driver never gives
            // it the keyboard, and ignores blurs as the harness does.
            cx.observe_window_activation(window, |p, window, _| {
                if !window.is_window_active() && !crate::shell::window::quiet() {
                    p.nodus.capture().blurred();
                }
            }),
            // The theme follows the app's, as the board window's does.
            cx.observe_window_appearance(window, |_, window, cx| {
                let preference = cx
                    .try_global::<crate::theme::Theme>()
                    .map_or(nodus_domain::ThemePreference::System, |t| t.preference);
                crate::theme::apply_preference(preference, cx);
                window.refresh();
            }),
            // CAP-119…CAP-122: the field's keys, before the field has them.
            {
                let me = cx.entity().downgrade();
                let panel_window = window.window_handle().window_id();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if window.window_handle().window_id() != panel_window {
                        return;
                    }
                    if let Some(panel) = me.upgrade() {
                        panel.update(cx, |p, cx| p.on_key(event, window, cx));
                    }
                })
            },
        ];
        let mut panel = Self {
            nodus: nodus.clone(),
            targets: None,
            context: CaptureContext {
                app: None,
                tab: None,
                clipboard: None,
                shortcut: CaptureShortcut::Ready,
            },
            field,
            text,
            attachments,
            guess: GuessState::default(),
            pending: None,
            guess_timer: None,
            choice: None,
            menu_open: false,
            sending: false,
            error: None,
            shot_blocked: false,
            receipt: None,
            receipt_timer: None,
            offer_done,
            history,
            history_at: -1,
            placeholder: "Ask Claude to do something, or jot a note",
            setting_text: false,
            sent_height: Rc::new(Cell::new(0.0)),
            menu_bottom: Rc::new(Cell::new(None)),
            route_bounds: Rc::new(Cell::new(None)),
            card_epoch: 0,
            recents_epoch: 0,
            recents_shown: false,
            menu_epoch: 0,
            receipt_epoch: 0,
            thumbs: HashMap::new(),
            voice_state: voice.state(),
            voice,
            voice_project,
            _voice: voice_task,
            _said: said_task,
            _poll: poll,
            _events: events,
            _subscriptions: subscriptions,
        };
        // A draft restored asks Claude once the page is up, as typing it would.
        panel.reschedule_guess(cx);
        // The panel came up while this page was being made.
        cx.spawn_in(window, async move |this, cx| {
            if let Some(context) = nodus.capture().ready().await {
                let _ = this.update_in(cx, |p, window, cx| p.on_shown(context, window, cx));
            }
        })
        .detach();
        panel
    }

    // ---- what it reads -------------------------------------------------------

    /// Where it goes now (CAP-084).
    pub(super) fn route(&self) -> Route {
        route(self.choice.as_ref(), self.guess.guess.as_ref(), self.targets.as_ref())
    }

    pub(super) fn sessions(&self) -> &[CaptureSession] {
        self.targets.as_ref().map_or(&[], |t| t.sessions.as_slice())
    }

    /// CAP-085.
    pub(super) fn has_text(&self) -> bool {
        !js::trim(&self.text).is_empty()
    }

    pub(super) fn no_projects(&self) -> bool {
        self.targets.as_ref().is_some_and(|t| t.projects.is_empty())
    }

    /// The clipboard on offer, unless it was seen already (CAP-098, CAP-167).
    pub(super) fn offer(&self) -> Option<&ClipboardOffer> {
        self.context
            .clipboard
            .as_ref()
            .filter(|offer| Some(offer_key(offer)) != self.offer_done)
    }

    /// While sending: still waiting for Claude to say where (CAP-106).
    pub(super) fn working_it_out(&self) -> bool {
        self.sending && self.guess.must_wait(&self.text, self.choice.is_some())
    }

    /// A data URL's picture, decoded once.
    pub(super) fn thumb(&mut self, url: &str) -> Option<Arc<RenderImage>> {
        if let Some(image) = self.thumbs.get(url) {
            return image.clone();
        }
        let image = decode_thumb(url);
        self.thumbs.insert(url.to_owned(), image.clone());
        image
    }

    // ---- the panel coming up -------------------------------------------------

    /// CAP-083: a fresh context, the receipt and menu gone, the draft kept,
    /// and the keyboard in the field at the end of what's there.
    fn on_shown(&mut self, context: CaptureContext, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.receipt_timer = None;
        self.clear_receipt();
        self.menu_open = false;
        self.error = None;
        self.shot_blocked = false;
        self.context = context;
        // Capture placed the window just before telling the page, at the
        // height it had: the page says again how tall it is.
        self.sent_height.set(0.0);
        self.refresh(cx);
        self.focus_field_at_end(window, cx);
        let field = self.field.clone();
        window.on_next_frame(move |window, cx| {
            field.update(cx, |s, cx| {
                if !s.focus_handle(cx).is_focused(window) {
                    s.focus(window, cx);
                }
            });
        });
        cx.notify();
    }

    /// CAP-083: later parts of the context merged in; a tab arriving comes
    /// along at the front (CAP-051, CAP-166).
    fn on_context(&mut self, patch: CaptureContextPatch, cx: &mut Context<'_, Self>) {
        if let Some(app) = patch.app {
            self.context.app = Some(app);
        }
        if let Some(clipboard) = patch.clipboard {
            self.context.clipboard = Some(clipboard);
        }
        if let Some(shortcut) = patch.shortcut {
            self.context.shortcut = shortcut;
        }
        if let Some(tab) = patch.tab {
            let changed = self.context.tab.as_ref() != Some(&tab);
            self.context.tab = Some(tab.clone());
            if changed {
                let attachments = with_tab(&self.attachments, &tab);
                if attachments != self.attachments {
                    self.attachments = attachments;
                    self.save_attachments();
                    self.reschedule_guess(cx);
                }
            }
        }
        cx.notify();
    }

    /// Reads the targets again (CAP-149).
    pub(super) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        let statuses = cx.try_global::<Stubs>().and_then(|s| s.statuses.clone());
        cx.spawn(async move |this, cx| {
            let mut targets = nodus.capture().targets().await;
            if let Some(statuses) = &statuses {
                super::stubs::overlay(&mut targets, statuses);
            }
            let _ = this.update(cx, |p, cx| {
                p.targets = Some(targets);
                p.note_recents();
                cx.notify();
            });
        })
        .detach();
    }

    /// The recents rise each time they appear (`cap-rise`, CAP-152).
    fn note_recents(&mut self) {
        let shown =
            self.receipt.is_none() && !self.has_text() && self.targets.as_ref().is_some_and(|t| !t.recents.is_empty());
        if shown && !self.recents_shown {
            self.recents_epoch += 1;
        }
        self.recents_shown = shown;
    }

    pub(super) fn focus_field(&self, window: &mut Window, cx: &mut App) {
        self.field.update(cx, |s, cx| s.focus(window, cx));
    }

    fn focus_field_at_end(&self, window: &mut Window, cx: &mut App) {
        let text = self.text.clone();
        self.field.update(cx, |s, cx| {
            let position = end_position(&text);
            s.set_cursor_position(position, window, cx);
        });
    }

    // ---- typing -------------------------------------------------------------

    /// The user typed (CAP-089): history browsing and the error end.
    fn on_field_changed(&mut self, value: String, cx: &mut Context<'_, Self>) {
        if self.setting_text || value == self.text {
            return;
        }
        self.text = value;
        self.save_draft();
        self.history_at = -1;
        self.error = None;
        self.reschedule_guess(cx);
        self.note_recents();
        cx.notify();
    }

    /// Puts `text` in the field from here (history, a transcript, sending).
    fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.setting_text = true;
        let value = text.clone();
        self.field.update(cx, |s, cx| s.set_value(value, window, cx));
        self.setting_text = false;
        self.text = text;
        self.save_draft();
        self.reschedule_guess(cx);
        self.note_recents();
        cx.notify();
    }

    /// CAP-064, CAP-065: each pause in the typing asks again, until the user
    /// picks for themselves; clearing the text forgets the guess and the choice.
    fn reschedule_guess(&mut self, cx: &mut Context<'_, Self>) {
        self.guess_timer = None;
        if js::trim(&self.text).is_empty() {
            self.guess.clear();
            self.pending = None;
            self.choice = None;
            return;
        }
        if !self.guess.wants_guess(&self.text, self.choice.is_some()) {
            return;
        }
        self.guess_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(page::GUESS_AFTER_MS))
                .await;
            let _ = this.update(cx, |p, cx| {
                p.ask_guess(cx);
            });
        }));
    }

    /// CAP-066: asks Claude; only the latest answer counts, and none keeps
    /// the guess on screen.
    fn ask_guess(&mut self, cx: &mut Context<'_, Self>) -> Pending {
        let run = self.guess.begin();
        let (tx, rx) = watch::channel(None);
        self.pending = Some(rx.clone());
        let nodus = self.nodus.clone();
        let text = self.text.clone();
        let attachments = self.attachments.clone();
        cx.spawn(async move |this, cx| {
            let answer = nodus.capture().guess(&text, &attachments).await.ok().flatten();
            let _ = tx.send(Some(answer.clone()));
            let _ = this.update(cx, |p, cx| {
                if p.guess.settle(run, answer, &text) {
                    p.pending = None;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        rx
    }

    // ---- the keys -------------------------------------------------------------

    /// CAP-119…CAP-122, CAP-104: Return sends, Tab cycles kinds, Esc closes
    /// the menu or the panel, ↑/↓ go through what was sent; nothing while an
    /// input method is composing.
    fn on_key(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        // CAP-104: Esc while the mic is busy cancels it, and goes no further.
        // The field is the panel's own, so its Esc counts too.
        if event.keystroke.key == "escape" && self.voice.on_escape(EscTarget::OwnField) {
            cx.stop_propagation();
            return;
        }
        if self.receipt.is_some() || !self.field.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let composing = self.field.update(cx, |s, cx| s.marked_text_range(window, cx)).is_some();
        if composing {
            return;
        }
        let keystroke = &event.keystroke;
        let shift = keystroke.modifiers.shift;
        match keystroke.key.as_str() {
            "enter" if !shift => {
                cx.stop_propagation();
                self.send(window, cx);
            }
            "tab" => {
                cx.stop_propagation();
                if self.has_text() {
                    self.cycle(if shift { -1 } else { 1 }, cx);
                }
            }
            "escape" => {
                cx.stop_propagation();
                if self.menu_open {
                    self.menu_open = false;
                    cx.notify();
                } else {
                    self.nodus.capture().hide();
                }
            }
            "up" => {
                if let Some(at) = history_up(&self.text, self.history_at, self.history.len()) {
                    cx.stop_propagation();
                    self.history_at = at;
                    let text = self.history[at as usize].clone();
                    self.set_text(text, window, cx);
                    self.focus_field_at_end(window, cx);
                }
            }
            "down" => {
                if let Some(at) = history_down(self.history_at) {
                    cx.stop_propagation();
                    self.history_at = at;
                    let text = usize::try_from(at)
                        .ok()
                        .and_then(|i| self.history.get(i).cloned())
                        .unwrap_or_default();
                    self.set_text(text, window, cx);
                    self.focus_field_at_end(window, cx);
                }
            }
            _ => {}
        }
    }

    // ---- choosing ---------------------------------------------------------------

    fn choose(&mut self, choice: Route, cx: &mut Context<'_, Self>) {
        self.choice = Some(choice);
        self.reschedule_guess(cx);
        cx.notify();
    }

    /// CAP-113: a kind from the menu (or Tab); nothing when there's no
    /// session at all to send into.
    pub(super) fn pick_kind(&mut self, kind: CaptureKind, cx: &mut Context<'_, Self>) {
        if let Some(choice) = pick_kind(kind, &self.route(), self.sessions()) {
            self.error = None;
            self.choose(choice, cx);
        }
    }

    /// CAP-114.
    fn cycle(&mut self, step: i32, cx: &mut Context<'_, Self>) {
        if let Some(choice) = cycle_kind(step, &self.route(), self.sessions()) {
            self.error = None;
            self.choose(choice, cx);
        }
    }

    /// CAP-115.
    pub(super) fn pick_project(&mut self, project_id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let choice = pick_project(project_id, &self.route(), self.sessions());
        self.menu_open = false;
        self.choose(choice, cx);
        self.focus_field(window, cx);
    }

    /// CAP-116.
    pub(super) fn pick_session(&mut self, session: &CaptureSession, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.menu_open = false;
        self.choose(pick_session(session), cx);
        self.focus_field(window, cx);
    }

    /// CAP-087: the header toggles the menu, and the keyboard stays in the field.
    pub(super) fn toggle_menu(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.menu_open = !self.menu_open;
        if self.menu_open {
            self.menu_epoch += 1;
        }
        self.focus_field(window, cx);
        cx.notify();
    }

    /// CAP-107: a press outside the menu and the header closes it.
    pub(super) fn close_menu(&mut self, cx: &mut Context<'_, Self>) {
        if self.menu_open {
            self.menu_open = false;
            cx.notify();
        }
    }

    // ---- what comes along ---------------------------------------------------------

    fn attach(&mut self, more: Vec<CaptureAttachment>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !more.is_empty() {
            self.attachments.extend(more);
            self.save_attachments();
            self.reschedule_guess(cx);
        }
        self.focus_field(window, cx);
        cx.notify();
    }

    /// CAP-096.
    pub(super) fn detach(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            self.save_attachments();
            self.reschedule_guess(cx);
        }
        self.focus_field(window, cx);
        cx.notify();
    }

    /// CAP-098: takes the clipboard afresh, and marks the offer seen.
    pub(super) fn take_offer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(offer) = self.offer().cloned() else { return };
        let nodus = self.nodus.clone();
        let stub = cx.try_global::<Stubs>().and_then(|s| s.clipboard.clone());
        cx.spawn_in(window, async move |this, cx| {
            let taken = match stub {
                Some(stub) => stub,
                None => nodus.capture().clipboard().await,
            };
            let _ = this.update_in(cx, |p, window, cx| {
                p.mark_offer_done(offer_key(&offer));
                p.attach(taken.into_iter().collect(), window, cx);
            });
        })
        .detach();
    }

    /// CAP-100: a screenshot, the notice when macOS kept it from Nodus.
    pub(super) fn screenshot(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        let stub = cx.try_global::<Stubs>().and_then(|s| s.screenshot.clone());
        cx.spawn_in(window, async move |this, cx| {
            let shot = match stub {
                Some(stub) => stub,
                None => nodus.capture().screenshot().await,
            };
            let _ = this.update_in(cx, |p, window, cx| {
                p.shot_blocked = matches!(shot, Screenshot::NeedsPermission);
                match shot {
                    Screenshot::Image(image) => p.attach(vec![CaptureAttachment::Image(image)], window, cx),
                    Screenshot::NeedsPermission | Screenshot::None => {
                        p.focus_field(window, cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// CAP-059.
    pub(super) fn allow_screen(&self, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |_, _| nodus.capture().allow_screen().await)
            .detach();
    }

    /// CAP-099: the paperclip's dialog.
    pub(super) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        let stub = cx.try_global::<Stubs>().and_then(|s| s.files.clone());
        cx.spawn_in(window, async move |this, cx| {
            let files = match stub {
                Some(stub) => stub,
                None => nodus.capture().files(None).await,
            };
            let _ = this.update_in(cx, |p, window, cx| p.attach(files, window, cx));
        })
        .detach();
    }

    /// CAP-062: files dropped on the card.
    pub(super) fn drop_files(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if paths.is_empty() {
            return;
        }
        let nodus = self.nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let files = nodus.capture().files(Some(&paths)).await;
            let _ = this.update_in(cx, |p, window, cx| p.attach(files, window, cx));
        })
        .detach();
    }

    /// CAP-117: Allow… in the banner; the state it returns replaces the context's.
    pub(super) fn allow(&self, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus.clone();
        cx.spawn(async move |this, cx| {
            let shortcut = nodus.capture().allow().await;
            let _ = this.update(cx, |p, cx| {
                p.context.shortcut = shortcut;
                cx.notify();
            });
        })
        .detach();
    }

    // ---- sending ------------------------------------------------------------------

    /// CAP-123, CAP-124: waits for Claude's guess when it hasn't looked at
    /// this text, then makes it; the receipt follows, or the error shows and
    /// the draft stays.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let trimmed = js::trim(&self.text).to_owned();
        if trimmed.is_empty() || self.sending {
            return;
        }
        self.sending = true;
        self.error = None;
        self.menu_open = false;
        let mut to = self.route();
        let pending = if self.guess.must_wait(&self.text, self.choice.is_some()) {
            Some(match self.pending.clone() {
                Some(pending) => pending,
                None => self.ask_guess(cx),
            })
        } else {
            None
        };
        let attachments = self.attachments.clone();
        let nodus = self.nodus.clone();
        let stub = cx.try_global::<Stubs>().and_then(|s| s.submit.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            if let Some(mut pending) = pending {
                let answer = pending
                    .wait_for(Option::is_some)
                    .await
                    .ok()
                    .and_then(|answer| answer.clone())
                    .flatten();
                if let Some(guess) = answer {
                    to = Route {
                        kind: guess.kind,
                        project_id: Some(guess.project_id),
                        session_id: guess.session_id,
                    };
                }
            }
            let made = match (&to.project_id, to.kind, &to.session_id) {
                (None, _, _) => Err("Add a project in Nodus first".to_owned()),
                (Some(_), CaptureKind::Into, None) => {
                    Err("There’s no session to send into. Pick another kind.".to_owned())
                }
                (Some(project_id), kind, session_id) => {
                    let submission = CaptureSubmission {
                        text: trimmed.clone(),
                        kind,
                        project_id: project_id.clone(),
                        session_id: if kind == CaptureKind::Into {
                            session_id.clone()
                        } else {
                            None
                        },
                        attachments,
                    };
                    match stub {
                        Some(receipt) => Ok(receipt),
                        // CAP-124: the plain message, with no IPC prefix.
                        None => nodus
                            .capture()
                            .submit(&submission)
                            .await
                            .map_err(|err| err.message().to_owned()),
                    }
                }
            };
            let _ = this.update_in(cx, |p, window, cx| {
                match made {
                    Ok(receipt) => p.sent(receipt, trimmed, window, cx),
                    Err(message) => p.error = Some(message),
                }
                p.sending = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// CAP-123 step 5: remembered, the offer seen, the draft cleared, and the
    /// receipt up for 2.6 s (CAP-147).
    fn sent(&mut self, receipt: CaptureReceipt, text: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.history = remember_sent(&self.history, &text);
        store(&self.nodus, HISTORY, Some(&self.history));
        // What was on the clipboard was seen, taken or not.
        if let Some(key) = self.context.clipboard.as_ref().map(offer_key) {
            self.mark_offer_done(key);
        }
        self.attachments.clear();
        self.save_attachments();
        self.choice = None;
        self.history_at = -1;
        self.receipt = Some(Shown { receipt, text });
        self.receipt_epoch += 1;
        self.set_text(String::new(), window, cx);
        self.refresh(cx);
        let nodus = self.nodus.clone();
        self.receipt_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(RECEIPT_MS)).await;
            let _ = this.update(cx, |p, cx| {
                p.clear_receipt();
                cx.notify();
            });
            nodus.capture().hide();
        }));
    }

    fn clear_receipt(&mut self) {
        if self.receipt.take().is_some() {
            // The card is made again, so it rises again.
            self.card_epoch += 1;
            self.recents_shown = false;
            self.note_recents();
        }
    }

    /// CAP-144, CAP-152: open a receipt's or a recent's card, or its board.
    pub(super) fn open(&mut self, project_id: String, session_id: Option<String>, cx: &mut Context<'_, Self>) {
        self.receipt_timer = None;
        self.clear_receipt();
        cx.notify();
        let nodus = self.nodus.clone();
        let request = OpenCardRequest { project_id, session_id };
        cx.spawn(async move |_, _| nodus.capture().open(&request).await)
            .detach();
    }

    // ---- the mic (CAP-103) ----------------------------------------------------------

    /// What the mic hints a transcript with: the route's project.
    pub(super) fn sync_voice_project(&self) {
        let project = self.route().project_id;
        *self.voice_project.lock().unwrap_or_else(PoisonError::into_inner) = project;
    }

    /// The mic's button: stops and transcribes while recording, else listens.
    pub(super) fn press_mic(&mut self, cx: &mut Context<'_, Self>) {
        let voice = Arc::clone(&self.voice);
        let recording = self.voice_state.phase == Phase::Recording;
        let work = self.nodus.voice().run(async move {
            if recording {
                voice.stop().await;
            } else {
                voice.record().await;
            }
        });
        cx.spawn(async move |_, _| work.await).detach();
    }

    /// The voice notice's Retry: the same recording again.
    pub(super) fn retry_voice(&mut self, cx: &mut Context<'_, Self>) {
        let voice = Arc::clone(&self.voice);
        let work = self.nodus.voice().run(async move { voice.retry().await });
        cx.spawn(async move |_, _| work.await).detach();
    }

    /// What was said, after what's typed (`page::append_transcript`), and
    /// the keyboard back in the field.
    fn on_said(&mut self, said: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = page::append_transcript(&self.text, said);
        self.history_at = -1;
        self.error = None;
        self.set_text(text, window, cx);
        self.focus_field_at_end(window, cx);
        self.focus_field(window, cx);
    }

    // ---- what outlives the panel (CAP-157) -------------------------------------------

    /// The draft, removed when empty.
    fn save_draft(&self) {
        store(&self.nodus, DRAFT, Some(&self.text).filter(|t| !t.is_empty()));
    }

    /// The draft's attachments, thumbnails included (CAP-172), removed when none.
    fn save_attachments(&self) {
        store(
            &self.nodus,
            ATTACHMENTS,
            Some(&self.attachments).filter(|a| !a.is_empty()),
        );
    }

    /// CAP-098, CAP-167: this clipboard isn't offered again, after a restart either.
    fn mark_offer_done(&mut self, key: String) {
        store(&self.nodus, CLIPBOARD_DONE, Some(&key));
        self.offer_done = Some(key);
    }

    /// Whether the banner under the card shows (CAP-117).
    pub(super) fn banner(&self) -> Option<CaptureShortcut> {
        let shortcut = self.context.shortcut;
        (!self.has_text()
            && matches!(
                shortcut,
                CaptureShortcut::NeedsPermission | CaptureShortcut::Unavailable
            ))
        .then_some(shortcut)
    }
}

/// The mic, the project it hints with, and the tasks bringing back what it
/// shows and what was said.
type Mic = (Arc<VoiceController>, Arc<Mutex<Option<String>>>, Task<()>, Task<()>);

/// The panel's mic (CAP-103): 05's voice hook on the core's voice commands,
/// its own slot (the panel is a window of its own), a transcript hinted with
/// the route's project, and a missing key opening Settings in the board
/// window (`capture:settings`, CAP-159). What it shows and what was said
/// come back to the page.
fn mic(nodus: &Nodus, window: &mut Window, cx: &mut Context<'_, CapturePanel>) -> Mic {
    let project = Arc::new(Mutex::new(None::<String>));
    let hint = Arc::clone(&project);
    let settings = nodus.clone();
    let backend = nodus.voice().backend(
        Arc::new(move || hint.lock().unwrap_or_else(PoisonError::into_inner).clone()),
        Arc::new(move || {
            let nodus = settings.clone();
            drop(tokio::spawn(async move { nodus.capture().open_settings().await }));
        }),
    );
    let (said_tx, mut said_rx) = mpsc::unbounded_channel::<String>();
    let voice = Arc::new(VoiceController::new(backend, VoiceSlot::default(), move |said| {
        let _ = said_tx.send(said);
    }));
    let mut states = voice.subscribe();
    let shows = cx.spawn(async move |this, cx| {
        while states.changed().await.is_ok() {
            let state = states.borrow_and_update().clone();
            if this
                .update(cx, |p, cx| {
                    p.voice_state = state;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
        }
    });
    let said = cx.spawn_in(window, async move |this, cx| {
        while let Some(said) = said_rx.recv().await {
            if this
                .update_in(cx, |p, window, cx| p.on_said(&said, window, cx))
                .is_err()
            {
                return;
            }
        }
    });
    (voice, project, shows, said)
}

/// The page's `localStorage` keys (CAP-157).
const DRAFT: &str = "capture.draft";
const ATTACHMENTS: &str = "capture.attachments";
const HISTORY: &str = "capture.history";
const CLIPBOARD_DONE: &str = "capture.clipboardDone";

/// `store.get(key, fallback)`: the value saved under `key`, none when it's
/// missing or unreadable.
fn stored<T: serde::de::DeserializeOwned>(nodus: &Nodus, key: &str) -> Option<T> {
    nodus
        .local_storage()
        .get(key)
        .and_then(|text| nodus_domain::json::from_str(&text).ok())
}

/// `store.set(key, value)`: none removes it.
fn store<T: serde::Serialize + ?Sized>(nodus: &Nodus, key: &str, value: Option<&T>) {
    let text = value.and_then(|v| nodus_domain::json::to_string(v).ok());
    nodus.local_storage().set(key, text.as_deref());
}

/// Where the caret goes to be at the end of `text`: its last line and column
/// (columns in UTF-16 units, as the field counts them).
fn end_position(text: &str) -> gpui::component::input::Position {
    let line = text.matches('\n').count() as u32;
    let last = text.rsplit('\n').next().unwrap_or_default();
    gpui::component::input::Position::new(line, js::utf16_len(last) as u32)
}

/// A thumbnail's `data:image/png;base64,…`, ready to draw (BGRA).
pub(super) fn decode_thumb(url: &str) -> Option<Arc<RenderImage>> {
    let data = url.split_once("base64,")?.1;
    let bytes = base64_decode(data)?;
    let mut image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

/// Standard base64, padding optional.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_as_data_urls_hold_it() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("a$"), None);
    }

    #[test]
    fn puts_the_caret_at_the_end() {
        let p = end_position("one\ntwo three");
        assert_eq!((p.line, p.character), (1, 9));
        let p = end_position("😀");
        assert_eq!((p.line, p.character), (0, 2));
    }
}
