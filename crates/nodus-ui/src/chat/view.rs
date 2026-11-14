//! The chat view's state: following the transcript, the rows the list shows,
//! what the composer sends where, and when the terminal is docked.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gpui::component::input::{InputEvent, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, FollowMode, Global, InteractiveElement,
    IntoElement, ListAlignment, ListState, ParentElement, Render, StatefulInteractiveElement as _, Styled,
    Subscription, Task, WeakEntity, Window, div, list, px, relative,
};
use nodus_domain::{Answers, ChatEntry, QueuedPrompt, SessionCard, SessionStatus};
use nodus_transcripts::ChatLog;
use serde_json::Value;

use crate::state::AppState;
use crate::terminal::actions::card;
use crate::terminal::host::{HostEvent, TerminalHost};
use crate::terminal::store::{Terminals, TerminalsEvent};
use crate::theme::{self, fonts};

/// How often the transcript is looked at while the chat is up (CHAT-018).
const POLL: Duration = Duration::from_millis(250);

/// The chat views on screen, by session, so the card's mic and wake reach
/// the composer rather than the terminal (CHAT-030).
#[derive(Default)]
pub struct ChatViews(HashMap<String, WeakEntity<ChatView>>);

impl Global for ChatViews {}

impl ChatViews {
    /// The chat showing a session, if one is.
    pub fn get(session_id: &str, cx: &App) -> Option<Entity<ChatView>> {
        cx.try_global::<Self>()?.0.get(session_id)?.upgrade()
    }

    fn put(session_id: &str, view: WeakEntity<ChatView>, cx: &mut App) {
        cx.default_global::<Self>().0.insert(session_id.to_owned(), view);
    }
}

/// One row of the list: a message, one of Claude's blocks, or a run of tool
/// calls drawn together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    User(usize),
    Entry(usize),
    /// Entries `start..end`, all tool calls.
    Tools(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    pub kind: RowKind,
    /// The first of Claude's rows after the user spoke: it carries the name.
    pub header: bool,
}

impl Row {
    fn end(&self) -> usize {
        match self.kind {
            RowKind::User(i) | RowKind::Entry(i) => i + 1,
            RowKind::Tools(_, end) => end,
        }
    }
}

/// CHAT-009: consecutive tool calls make one row; Claude's first row after
/// the user's carries its name.
pub(crate) fn rows_of(entries: &[ChatEntry]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut claude_spoke = false;
    let mut i = 0;
    while i < entries.len() {
        let entry = &entries[i];
        if entry.is_user() {
            rows.push(Row {
                kind: RowKind::User(i),
                header: false,
            });
            claude_spoke = false;
            i += 1;
            continue;
        }
        if let ChatEntry::Notice { .. } = entry {
            rows.push(Row {
                kind: RowKind::Entry(i),
                header: false,
            });
            i += 1;
            continue;
        }
        let header = !claude_spoke;
        claude_spoke = true;
        if let ChatEntry::Tool { .. } = entry {
            let start = i;
            while i < entries.len() && matches!(entries[i], ChatEntry::Tool { .. }) {
                i += 1;
            }
            rows.push(Row {
                kind: RowKind::Tools(start, i),
                header,
            });
            continue;
        }
        rows.push(Row {
            kind: RowKind::Entry(i),
            header,
        });
        i += 1;
    }
    rows
}

/// Where the session's terminal is: docked under the chat or out of sight.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Dock {
    pub open: bool,
    /// Opened by the chat itself (Claude asked, a slash command ran), so it
    /// closes again when that's over.
    pub auto: bool,
}

pub struct ChatView {
    pub(crate) state: Entity<AppState>,
    pub session_id: String,
    pub(crate) project_id: String,
    pub(crate) terminal: Entity<TerminalHost>,
    log: Option<Arc<Mutex<ChatLog>>>,
    pub(crate) entries: Arc<[ChatEntry]>,
    pub(crate) loaded: bool,
    pub(crate) rows: Vec<Row>,
    pub(crate) list: ListState,
    pub(crate) composer: Entity<TextareaState>,
    /// The composer's `/` menu (CHAT-042).
    pub(crate) slash: super::slash::SlashMenu,
    /// Claude's charts and forms, read once each.
    pub(crate) shows: super::shows::Shows,
    /// Tool runs shown in full, by their first call's id.
    pub(crate) opened: HashSet<String>,
    /// When entries that came in while the chat was up arrived, by id, so
    /// they move in (CHAT-040).
    pub(crate) born: HashMap<String, Instant>,
    /// When Claude's turn started, as this chat saw it: the clock its
    /// working motion runs on (CHAT-019).
    pub(crate) working_since: Option<Instant>,
    /// When Claude's last turn ended, so the working line eases away
    /// (MOTION-040).
    pub(crate) turn_ended: Option<Instant>,
    /// When tool calls that ran while the chat was up finished, by id, so
    /// their ticks spring in (MOTION-042).
    pub(crate) finished: HashMap<String, Instant>,
    pub(crate) dock: Dock,
    /// Everything draws this many times its size (a card on a zoomed board).
    pub(crate) scale: f32,
    /// In an expanded card rather than the session window.
    pub(crate) in_card: bool,
    /// The terminal may attach here (it can't while the session window has it).
    pub(crate) terminal_enabled: bool,
    /// What the last send or queue couldn't do.
    pub(crate) error: Option<String>,
    pub(crate) sending: bool,
    /// Words for the composer that came without a window (the card's mic).
    pending_text: Option<String>,
    /// Claude shows a dialog in its terminal (a trust check, a permission
    /// prompt, a picker), so the terminal docks and nothing is typed in.
    pub(crate) dialog: bool,
    /// The terminal drew something since the screen was last looked at.
    screen_dirty: bool,
    ticks: u32,
    /// The queue as last seen, to give back what an ended session didn't take.
    last_queue: Vec<QueuedPrompt>,
    last_status: Option<SessionStatus>,
    focus: FocusHandle,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatView {
    pub fn new(
        state: Entity<AppState>,
        session_id: String,
        project_id: String,
        terminal: Entity<TerminalHost>,
        in_card: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .context_menu(true)
                .submit_on_enter(true)
                .placeholder("Message Claude…")
        });
        let nodus = state.read(cx).nodus.clone();
        let log = nodus
            .sessions()
            .transcript_file(&project_id, &session_id)
            .map(|file| Arc::new(Mutex::new(ChatLog::new(file))));
        let list = ListState::new(0, ListAlignment::Bottom, px(1200.));
        list.set_follow_mode(FollowMode::Tail);
        let terminals = Terminals::global(cx);
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| {
                this.follow_status(cx);
                cx.notify();
            }),
            cx.subscribe(&terminal, |_, _, _: &HostEvent, cx| cx.notify()),
            cx.subscribe(&terminals, |this: &mut Self, _, e: &TerminalsEvent, _| match e {
                TerminalsEvent::Output(id) | TerminalsEvent::Changed(id) if *id == this.session_id => {
                    this.screen_dirty = true;
                }
                _ => {}
            }),
            cx.subscribe_in(&composer, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Change => {
                    this.error = None;
                    this.slash_changed(cx);
                    cx.notify();
                }
                // CHAT-020: Return sends (or queues), ⌘Return sends at once.
                InputEvent::PressEnter {
                    secondary,
                    shift: false,
                } => this.press_enter(*secondary, window, cx),
                _ => {}
            }),
        ];
        let queue = nodus.sessions().queue(&session_id);
        state.update(cx, |s, _| {
            s.queues.insert(session_id.clone(), queue);
        });
        ChatViews::put(&session_id, cx.entity().downgrade(), cx);
        let mut this = Self {
            state,
            session_id,
            project_id,
            terminal,
            log,
            entries: Arc::from(Vec::new()),
            loaded: false,
            rows: Vec::new(),
            list,
            composer,
            slash: super::slash::SlashMenu::default(),
            shows: super::shows::Shows::default(),
            opened: HashSet::new(),
            born: HashMap::new(),
            working_since: None,
            turn_ended: None,
            finished: HashMap::new(),
            dock: Dock::default(),
            scale: 1.0,
            in_card,
            terminal_enabled: true,
            error: None,
            sending: false,
            pending_text: None,
            dialog: false,
            screen_dirty: true,
            ticks: 0,
            last_queue: Vec::new(),
            last_status: None,
            focus: cx.focus_handle(),
            _poll: None,
            _subscriptions: subscriptions,
        };
        this.follow_status(cx);
        this.poll(cx);
        this
    }

    /// CHAT-018: reads what the transcript gained, off the main thread,
    /// every [`POLL`] while the chat is up.
    fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let Some(log) = self.log.clone() else {
            self.loaded = true;
            return;
        };
        self._poll = Some(cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                let reading = Arc::clone(&log);
                let (changed, entries, skills) = cx
                    .background_spawn(async move {
                        let mut log = reading.lock().unwrap_or_else(PoisonError::into_inner);
                        let changed = log.refresh();
                        (changed, log.snapshot(), log.skills())
                    })
                    .await;
                let gone = this
                    .update(cx, |this, cx| {
                        if changed || first {
                            this.set_entries(entries, cx);
                        }
                        // CHAT-043: the skills Claude Code listed, for the `/` menu.
                        this.slash.set_listed(skills);
                        this.look_at_screen(cx);
                    })
                    .is_err();
                if gone {
                    break;
                }
                first = false;
                cx.background_executor().timer(POLL).await;
            }
        }));
    }

    /// New entries: the rows from the first changed one are measured again.
    fn set_entries(&mut self, entries: Arc<[ChatEntry]>, cx: &mut Context<'_, Self>) {
        let first_change = self
            .entries
            .iter()
            .zip(entries.iter())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| self.entries.len().min(entries.len()));
        // CHAT-040: what's new since the chat opened moves in; what it opened
        // with is already there.
        if self.loaded {
            let now = Instant::now();
            self.born
                .retain(|_, at| now.duration_since(*at) < Duration::from_secs(5));
            let known: HashSet<&str> = self.entries.iter().map(ChatEntry::id).collect();
            for entry in entries.iter() {
                if !known.contains(entry.id()) {
                    self.born.insert(entry.id().to_owned(), now);
                }
            }
            // MOTION-042: the calls that were running and now aren't.
            self.finished
                .retain(|_, at| now.duration_since(*at) < Duration::from_secs(5));
            let running: HashSet<&str> = self
                .entries
                .iter()
                .filter_map(|e| match e {
                    ChatEntry::Tool {
                        id,
                        done: false,
                        failed: false,
                        ..
                    } => Some(id.as_str()),
                    _ => None,
                })
                .collect();
            for entry in entries.iter() {
                if let ChatEntry::Tool { id, done, failed, .. } = entry
                    && (*done || *failed)
                    && running.contains(id.as_str())
                {
                    self.finished.insert(id.clone(), now);
                }
            }
        }
        let rows = rows_of(&entries);
        let from = self
            .rows
            .iter()
            .position(|r| r.end() >= first_change)
            .unwrap_or(self.rows.len());
        let from = from.min(rows.len());
        self.list.splice(from..self.rows.len(), rows.len() - from);
        self.rows = rows;
        self.entries = entries;
        self.loaded = true;
        cx.notify();
    }

    pub(crate) fn card(&self, cx: &App) -> Option<SessionCard> {
        card(self.state.read(cx), &self.session_id)
    }

    /// The prompts waiting for this session's turn to end.
    pub(crate) fn queue(&self, cx: &App) -> Vec<QueuedPrompt> {
        self.state
            .read(cx)
            .queues
            .get(&self.session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// CHAT-033: a dialog on Claude's screen (its folder trust check, a
    /// permission prompt, a picker) docks the terminal until it's answered.
    fn look_at_screen(&mut self, cx: &mut Context<'_, Self>) {
        // Output held for a synchronized update lands without an event, so
        // the screen is looked at every second as well.
        self.ticks = self.ticks.wrapping_add(1);
        if !std::mem::take(&mut self.screen_dirty) && !self.ticks.is_multiple_of(4) {
            return;
        }
        let registry = Terminals::global(cx).read(cx).registry.clone();
        let text = registry
            .with_terminal(&self.session_id, |t| {
                let s = t.snapshot();
                s.cells
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|c| if c.text.is_empty() { " " } else { c.text.as_str() })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let dialog = shows_dialog(&text);
        if dialog == self.dialog {
            return;
        }
        self.dialog = dialog;
        if dialog && !self.dock.open {
            self.dock = Dock { open: true, auto: true };
            self.wake_terminal(cx);
        } else if !dialog && self.dock.auto && self.card(cx).map(|c| c.status) != Some(SessionStatus::Waiting) {
            self.dock = Dock::default();
        }
        cx.notify();
    }

    /// CHAT-024: the terminal docks itself while Claude waits on the user
    /// there, and goes again when it stops waiting.
    fn follow_status(&mut self, cx: &mut Context<'_, Self>) {
        let status = self.card(cx).map(|c| c.status);
        let working = self
            .card(cx)
            .is_some_and(|c| c.live && c.status == SessionStatus::Working);
        if !working {
            if self.working_since.take().is_some() {
                self.turn_ended = Some(Instant::now());
            }
        } else if self.working_since.is_none() {
            self.working_since = Some(Instant::now());
            self.turn_ended = None;
        }
        let waiting = status == Some(SessionStatus::Waiting);
        let was_waiting = self.last_status == Some(SessionStatus::Waiting);
        if waiting && !was_waiting && !self.dock.open {
            self.dock = Dock { open: true, auto: true };
            self.wake_terminal(cx);
        } else if !waiting && was_waiting && self.dock.auto {
            self.dock = Dock::default();
        }
        // CHAT-032: a session that stopped with prompts still queued (QUEUE-012)
        // gives them back to the composer.
        let queue = self.queue(cx);
        let live = self.card(cx).is_some_and(|c| c.live);
        if queue.is_empty() && !self.last_queue.is_empty() && !live {
            let left: Vec<String> = std::mem::take(&mut self.last_queue)
                .into_iter()
                .map(|p| p.text)
                .collect();
            let pending = self.pending_text.get_or_insert_default();
            if !pending.is_empty() {
                pending.push_str("\n\n");
            }
            pending.push_str(&left.join("\n\n"));
            self.error = Some("Claude stopped before your queued messages went out. They’re back in the box.".into());
        }
        self.last_queue = queue;
        // CHAT-019: the last row carries the working line, so it's measured again.
        if status != self.last_status && !self.rows.is_empty() {
            let last = self.rows.len() - 1;
            self.list.splice(last..last + 1, 1);
        }
        self.last_status = status;
    }

    pub(crate) fn set_dock(&mut self, open: bool, cx: &mut Context<'_, Self>) {
        self.dock = Dock { open, auto: false };
        if open {
            self.wake_terminal(cx);
        }
        cx.notify();
    }

    /// A card's terminal attaches only once the card is woken (XCARD-025).
    fn wake_terminal(&self, cx: &mut App) {
        if self.in_card {
            crate::board::cards::expanded_card::wake_card_terminal(&self.session_id, cx);
        }
    }

    /// Set by the card each frame: the board's zoom, and whether the
    /// terminal may attach here.
    pub fn set_frame(&mut self, scale: f32, terminal_enabled: bool) {
        self.scale = scale;
        self.terminal_enabled = terminal_enabled;
    }

    /// Gives the composer the keyboard.
    pub fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        let handle = self.composer_handle(cx);
        window.focus(&handle, cx);
    }

    /// The composer's focus, for giving it the keyboard from outside.
    pub fn composer_handle(&self, cx: &App) -> FocusHandle {
        self.composer.read(cx).focus_handle(cx)
    }

    /// CHAT-030: words said into the mic go into the composer, at the caret.
    pub fn insert_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = nodus_domain::js::trim(text);
        if text.is_empty() {
            return;
        }
        self.composer.update(cx, |s, cx| {
            let joined = if s.value().trim().is_empty() {
                text.to_owned()
            } else {
                format!(" {text}")
            };
            s.insert(joined, window, cx);
        });
        self.focus_composer(window, cx);
    }

    /// [`Self::insert_text`] on the next frame, for callers without a window.
    pub fn insert_later(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        let pending = self.pending_text.get_or_insert_default();
        if !pending.is_empty() {
            pending.push(' ');
        }
        pending.push_str(text);
        cx.notify();
    }

    /// CHAT-020…023: the composer's words go to Claude.
    ///
    /// - While Claude works (or waits on the user), Return queues them: they
    ///   go when the turn ends.
    /// - `now` types them in at once: Claude takes them at its next step.
    /// - An idle session gets them typed; one that isn't running resumes with them.
    pub(crate) fn submit(&mut self, now: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.composer.update(cx, |s, cx| s.set_value("", window, cx));
        self.deliver(text, now, cx);
    }

    /// Return (⌘Return when `now`): CHAT-045's `/` menu runs its command
    /// while it's open, else CHAT-020 sends the composer's words.
    fn press_enter(&mut self, now: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.slash_run(now, window, cx) {
            self.submit(now, window, cx);
        }
    }

    pub(crate) fn deliver(&mut self, text: String, now: bool, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        self.error = None;
        // CHAT-033: words typed into a dialog would answer it; they wait in
        // the queue while Claude works, else in the composer.
        if self.dialog && (now || !self.busy(cx)) {
            self.pending_text = Some(text);
            self.error = Some("Claude is asking something in the terminal below. Answer it first.".into());
            cx.notify();
            return;
        }
        // CHAT-025: a slash command may open a picker, which only the terminal shows.
        if text.starts_with('/') && (now || !self.busy(cx)) {
            self.dock = Dock { open: true, auto: true };
            self.wake_terminal(cx);
        }
        self.sending = true;
        cx.notify();
        let (project, session) = (self.project_id.clone(), self.session_id.clone());
        cx.spawn(async move |this, cx| {
            let result = if now {
                nodus.sessions().send(&project, &session, &text).await.map(|()| None)
            } else {
                nodus.sessions().deliver(&project, &session, &text).await.map(Some)
            };
            let _ = this.update(cx, |this, cx| {
                this.sending = false;
                if let Err(err) = result {
                    this.error = Some(err.ipc_message());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Claude is running and in a turn (or waiting on the user in one).
    pub(crate) fn busy(&self, cx: &App) -> bool {
        self.card(cx)
            .is_some_and(|c| c.live && matches!(c.status, SessionStatus::Working | SessionStatus::Waiting))
    }

    /// CHAT-022: Esc for Claude, as in its terminal.
    pub(crate) fn interrupt(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        nodus.sessions().write(&self.session_id, "\x1b");
    }

    /// CHAT-027: answers to a form, sent like anything typed.
    pub(crate) fn send_answers(&mut self, to: &str, title: &str, answers: Value, cx: &mut Context<'_, Self>) {
        let message = Answers {
            to: to.to_owned(),
            title: title.to_owned(),
            answers,
        }
        .to_message();
        self.deliver(message, false, cx);
    }

    /// CHAT-021: a queued prompt taken out of the queue.
    pub(crate) fn unqueue(&mut self, id: &str, cx: &mut Context<'_, Self>) -> Option<QueuedPrompt> {
        let mut items = self.queue(cx);
        let at = items.iter().position(|p| p.id == id)?;
        let item = items.remove(at);
        let nodus = self.state.read(cx).nodus.clone();
        if let Err(err) = nodus.sessions().set_queue(&self.session_id, items) {
            self.error = Some(err.ipc_message());
        }
        Some(item)
    }

    /// CHAT-021: back into the composer to change it.
    pub(crate) fn edit_queued(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(item) = self.unqueue(id, cx) {
            self.composer.update(cx, |s, cx| s.set_value(item.text, window, cx));
            self.focus_composer(window, cx);
        }
    }

    /// CHAT-023: with Claude idle (it was interrupted, so no turn ended),
    /// the first queued prompt goes now.
    pub(crate) fn send_queued_now(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if let Some(item) = self.unqueue(id, cx) {
            self.deliver(item.text, true, cx);
        }
    }

    pub(crate) fn toggle_tools(&mut self, first_id: &str, cx: &mut Context<'_, Self>) {
        if !self.opened.remove(first_id) {
            self.opened.insert(first_id.to_owned());
        }
        if let Some(ix) = self.rows.iter().position(|r| match r.kind {
            RowKind::Tools(start, _) => self.entries.get(start).is_some_and(|e| e.id() == first_id),
            _ => false,
        }) {
            self.list.splice(ix..ix + 1, 1);
        }
        cx.notify();
    }

    /// The board's view a page put up: the window closes and the board
    /// shows it (CHAT-028).
    pub(crate) fn show_on_board(&mut self, view_id: &str, cx: &mut Context<'_, Self>) {
        let view_id = view_id.to_owned();
        self.state.update(cx, |s, cx| s.show_node(view_id, cx));
    }

    pub(crate) fn px(&self, v: f32) -> gpui::Pixels {
        px(v * self.scale)
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(text) = self.pending_text.take() {
            self.insert_text(&text, window, cx);
        }
        let live = self.card(cx).is_some_and(|c| c.live);
        // CHAT-034: the session window's terminal attaches while the session
        // runs (to watch its screen) or the dock is up, which resumes it; a
        // card's terminal follows the card.
        if !self.in_card {
            let attach = live || self.dock.open;
            self.terminal.update(cx, |h, cx| h.set_enabled(attach, cx));
        }
        let t = theme::tokens(cx);
        let messages = self.render_messages(window, cx);
        let dock = self.dock.open.then(|| self.render_dock(window, cx));
        let composer = self.render_composer(window, cx);
        // CHAT-034: while it isn't docked, the running session's terminal is
        // laid out off screen at the dock's size, so its screen can be read.
        let hidden = (!self.dock.open && live && self.terminal_enabled).then(|| {
            div()
                .absolute()
                .top_0()
                .left(px(-20000.))
                .w_full()
                .when(self.in_card, |d| d.h(relative(0.5)))
                .when(!self.in_card, |d| d.h(px(320.)))
                .child(self.terminal.clone())
        });
        div()
            .id(("chat", cx.entity_id()))
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg_elevated)
            .font_family(fonts::UI)
            .text_color(t.text_primary)
            .text_size(self.px(13.5))
            .line_height(self.px(20.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(messages)
                    .children(self.render_edges(window, cx)),
            )
            .children(dock)
            .child(composer)
            .children(hidden)
    }
}

impl ChatView {
    fn render_messages(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let t = theme::tokens(cx);
        if self.rows.is_empty() {
            let live = self.card(cx).is_some_and(|c| c.live);
            let (title, line) = if !self.loaded {
                ("", "")
            } else if self.log.is_none() {
                ("This session runs on a server", "Its chat isn’t available here yet.")
            } else if live {
                (
                    "Ask Claude anything",
                    "Charts, questions and pages Claude makes show up here.",
                )
            } else {
                ("No messages yet", "Send a message to start this session.")
            };
            // CHAT-035: Claude's mark over the words, once there's something to say.
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(self.px(4.))
                .when(self.loaded, |d| {
                    d.child(super::look::avatar(40. * self.scale, &t).mb(self.px(10.)))
                })
                .child(
                    div()
                        .text_size(self.px(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_size(self.px(12.5)).text_color(t.text_secondary).child(line))
                .into_any_element();
        }
        list(
            self.list.clone(),
            cx.processor(|this, ix: usize, window, cx| this.render_row(ix, window, cx)),
        )
        .size_full()
        .into_any_element()
    }

    /// CHAT-036: the list's edges fade into the chat, and a button takes the
    /// list back to the newest row once it's been scrolled up from it.
    fn render_edges(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<gpui::AnyElement> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let t = theme::tokens(cx);
        let fade = |top: bool, h: f32| {
            let (from, to) = (theme::tokens::alpha(t.bg_elevated, 0.), gpui::Hsla::from(t.bg_elevated));
            div()
                .absolute()
                .left_0()
                .right_0()
                .h(self.px(h))
                .when(top, Styled::top_0)
                .when(!top, Styled::bottom_0)
                .bg(gpui::linear_gradient(
                    if top { 0. } else { 180. },
                    gpui::linear_color_stop(from, 0.),
                    gpui::linear_color_stop(to, 1.),
                ))
                .into_any_element()
        };
        let away = !self.list.is_following_tail() && self.list.is_scrolled_to_end() == Some(false);
        let id = cx.entity_id();
        let shown = crate::components::tween::transition(
            window,
            cx,
            ("chat-latest", id),
            if away { 1f32 } else { 0. },
            Duration::from_millis(180),
            theme::motion::ease_out,
        );
        let mut edges = vec![fade(true, 14.), fade(false, 22.)];
        if shown > 0.01 {
            let me = cx.entity().downgrade();
            edges.push(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(self.px(10.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("chat-latest")
                            .size(self.px(32.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(super::look::field_bg(&t))
                            .border_1()
                            .border_color(t.border)
                            .shadow(t.md())
                            .hover(|d| d.bg(t.bg_hover))
                            .opacity(shown)
                            .translate(px(0.), self.px(6. * (1. - shown)))
                            .child(crate::components::icon::icon(
                                crate::components::icon::Icon::ChevronDown,
                                15. * self.scale,
                                2.2,
                                t.text_secondary,
                            ))
                            .tooltip(crate::components::tooltip::native_tooltip("Jump to the latest".into()))
                            .on_click(move |_, _, cx| {
                                let _ = me.update(cx, |this, cx| {
                                    this.list.set_follow_mode(FollowMode::Tail);
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        edges
    }

    /// CHAT-024: the terminal, under the chat.
    fn render_dock(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let t = theme::tokens(cx);
        let waiting = self.dialog || self.card(cx).is_some_and(|c| c.status == SessionStatus::Waiting);
        let label = if waiting { "Claude needs you here" } else { "Terminal" };
        let me = cx.entity().downgrade();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(self.px(8.))
            .h(self.px(30.))
            .px(self.px(14.))
            .text_size(self.px(12.))
            .text_color(rgba_white(0.72))
            .child(
                div()
                    .size(self.px(7.))
                    .rounded_full()
                    .bg(if waiting { t.status_waiting } else { t.status_done }),
            )
            .child(div().flex_1().child(label))
            .child(
                div()
                    .id("chat-dock-close")
                    .cursor_pointer()
                    .p(self.px(4.))
                    .rounded(self.px(5.))
                    .hover(|d| d.bg(rgba_white(0.08)))
                    .child(crate::components::icon::icon(
                        crate::components::icon::Icon::ChevronDown,
                        13. * self.scale,
                        2.,
                        rgba_white(0.6),
                    ))
                    .on_click(move |_, _, cx| {
                        let _ = me.update(cx, |this, cx| this.set_dock(false, cx));
                    }),
            );
        let body = if self.terminal_enabled {
            div()
                .flex_1()
                .min_h_0()
                .pl(self.px(12.))
                .pr(self.px(4.))
                .pb(self.px(8.))
                .child(self.terminal.clone())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(self.px(12.))
                .text_color(rgba_white(0.6))
                .child("The session window has this terminal open.")
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .when(self.in_card, |d| d.h(relative(0.5)))
            .when(!self.in_card, |d| d.h(px(320.)))
            .bg(t.term_bg)
            .border_t_1()
            .border_color(t.border)
            .child(header)
            .child(body)
            .into_any_element()
    }
}

/// CHAT-033: what Claude Code's dialogs say at their foot, or their
/// highlighted numbered choice.
pub(crate) fn shows_dialog(screen: &str) -> bool {
    const FEET: [&str; 7] = [
        "Enter to confirm",
        "Enter to select",
        "Enter to continue",
        "Esc to cancel",
        "Esc to exit",
        "Esc to go back",
        "Press Enter to",
    ];
    FEET.iter().any(|f| screen.contains(f))
        || screen.lines().any(|line| {
            let line = line.trim_start();
            line.strip_prefix('❯').map(str::trim_start).is_some_and(|rest| {
                rest.split_once(". ")
                    .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
        })
}

fn rgba_white(a: f32) -> gpui::Rgba {
    theme::tokens::rgba(255, 255, 255, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(id: &str) -> ChatEntry {
        ChatEntry::User {
            id: id.into(),
            at: 0.,
            text: "hi".into(),
            queued: false,
            answers: None,
        }
    }

    fn tool(id: &str) -> ChatEntry {
        ChatEntry::Tool {
            id: id.into(),
            at: 0.,
            name: "Bash".into(),
            detail: String::new(),
            failed: false,
            done: true,
        }
    }

    fn text(id: &str) -> ChatEntry {
        ChatEntry::Text {
            id: id.into(),
            at: 0.,
            markdown: "x".into(),
        }
    }

    #[test]
    fn chat_033_dialogs_are_seen_on_the_screen() {
        assert!(shows_dialog(
            " ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel"
        ));
        assert!(shows_dialog(" Do you want to proceed?\n ❯ 1. Yes\n   2. No"));
        assert!(!shows_dialog("╭──────╮\n│ >    │\n╰──────╯\n  ? for shortcuts"));
        assert!(!shows_dialog("❯ fix the tests"));
    }

    #[test]
    fn tool_runs_group_and_claude_is_named_once_per_reply() {
        let entries = vec![
            user("u"),
            text("a"),
            tool("t1"),
            tool("t2"),
            ChatEntry::Visual {
                id: "v".into(),
                at: 0.,
                spec: json!({}),
            },
            user("u2"),
            tool("t3"),
        ];
        let rows = rows_of(&entries);
        let kinds: Vec<_> = rows.iter().map(|r| (r.kind, r.header)).collect();
        assert_eq!(
            kinds,
            [
                (RowKind::User(0), false),
                (RowKind::Entry(1), true),
                (RowKind::Tools(2, 4), false),
                (RowKind::Entry(4), false),
                (RowKind::User(5), false),
                (RowKind::Tools(6, 7), true),
            ]
        );
    }
}
