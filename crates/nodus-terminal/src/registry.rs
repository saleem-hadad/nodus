//! registry.ts as plain Rust: one terminal per session for the life of the
//! window (TERM-001), shown in one container at a time (TERM-002) and parked
//! otherwise with its screen, scrollback and modes intact (TERM-003), still
//! parsing its session's output while parked (TERM-007).
//!
//! Ordering: `Sessions::open` returns the output the session host buffered up
//! to the moment it replied. Output that arrives while the open is in flight
//! is part of that replay and is dropped (TERM-009); once the replay is
//! written the terminal is ready and live output goes straight in. The caller
//! must deliver `pty_data` events and the open's reply in the order the host
//! sent them, as Electron's in-order IPC did.
//!
//! What a terminal sends (keys, pastes, focus reports, query answers) goes to
//! the session unless its process has exited (TERM-025).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use nodus_domain::SessionKind;
use tokio::sync::watch;

use crate::keys::{KeyAction, KeyEvent, Platform};
use crate::terminal::{DEFAULT_COLS, DEFAULT_ROWS, Terminal};

/// A starting session that never says it's listening gets its words after this long anyway.
pub const LISTEN_LIMIT: Duration = Duration::from_millis(30_000);

/// The line written when the process exits (TERM-015).
pub fn exit_line(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Claude => "\r\n\x1b[2m[process exited · press Resume or reopen the card to continue]\x1b[0m\r\n",
        SessionKind::Terminal => {
            "\r\n\x1b[2m[shell exited · press Restart or reopen the card for a new shell]\x1b[0m\r\n"
        }
    }
}

/// What `sessions.open` resolves to: the buffered output to replay, or the
/// error's message.
pub type OpenFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// The session side the registry drives (`window.api.sessions`). Its methods
/// must not call back into the registry.
pub trait Sessions: Send + Sync + 'static {
    /// Starts or resumes the session's process at this size.
    fn open(&self, project_id: &str, session_id: &str, cols: usize, rows: usize) -> OpenFuture;
    fn write(&self, session_id: &str, data: &str);
    fn resize(&self, session_id: &str, cols: usize, rows: usize);
}

/// Where a terminal is shown: the session window, or a card. The UI picks the ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContainerId(pub u64);

/// The grid a container fits, or `None` while it can't be measured (the fit is skipped).
pub type Fit = Option<(usize, usize)>;

/// Removes an output listener (registry.ts's `onOutput` return value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// `sessions.open` is in flight; live data is covered by its replay.
    Opening,
    /// Replay written; live data flows straight into the terminal.
    Ready,
    /// The process ended (or opening failed); the next attach resumes it.
    Exited,
}

type Listener = Arc<dyn Fn() + Send + Sync>;
#[cfg(test)]
type TraceLog = Arc<Mutex<Vec<Vec<String>>>>;
type OpenResult = Option<Result<(), String>>;

struct Entry {
    /// Tells this entry from a later one for the same session.
    id: u64,
    kind: SessionKind,
    terminal: Terminal,
    state: State,
    opening: Option<watch::Receiver<OpenResult>>,
    /// The process exited while the open was in flight.
    exit_pending: bool,
    has_output: bool,
    listeners: Vec<(ListenerId, Listener)>,
    /// Where it is shown; `None` while parked.
    container: Option<ContainerId>,
    /// Starting sessions waiting to turn on bracketed paste (`onWriteParsed`
    /// listeners), each with its 30 s timer.
    waiters: Vec<(u64, tokio::task::JoinHandle<()>)>,
}

struct Pending {
    text: String,
    starting: bool,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    pending: HashMap<String, Pending>,
    next_id: u64,
}

/// Every session's terminal, for the whole window.
pub struct Registry {
    inner: Mutex<Inner>,
    sessions: Arc<dyn Sessions>,
    platform: Platform,
    runtime: tokio::runtime::Handle,
    this: Weak<Registry>,
    /// Tests record what the registry does to its terminals here, beside
    /// what their fake session side records.
    #[cfg(test)]
    trace: Mutex<Option<TraceLog>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

/// What `attach` does once it has looked at the entry.
enum Next {
    Wait(watch::Receiver<OpenResult>),
    Open {
        id: u64,
        project: String,
        cols: usize,
        rows: usize,
        done: watch::Sender<OpenResult>,
    },
    Done,
}

impl Registry {
    /// A registry whose terminals behave as on `platform`, timing on `runtime`.
    pub fn new(sessions: Arc<dyn Sessions>, platform: Platform, runtime: tokio::runtime::Handle) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            inner: Mutex::new(Inner::default()),
            sessions,
            platform,
            runtime,
            this: this.clone(),
            #[cfg(test)]
            trace: Mutex::new(None),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn trace(&self, event: &[&str]) {
        if let Some(log) = self.trace.lock().unwrap().as_ref() {
            log.lock()
                .unwrap()
                .push(event.iter().map(|s| (*s).to_owned()).collect());
        }
    }

    #[cfg(not(test))]
    fn trace(&self, _event: &[&str]) {}

    /// registry.ts's `onData`: the terminal sends `data` to the session unless
    /// its process has exited (TERM-025).
    fn send(&self, session_id: &str, entry: &Entry, data: &str) {
        if entry.state != State::Exited && !data.is_empty() {
            self.sessions.write(session_id, data);
        }
    }

    /// `writeOutput`: into the terminal; non-empty output marks it as having
    /// output and tells the listeners (TERM-010). Returns the listeners to call.
    fn write_output(&self, session_id: &str, entry: &mut Entry, data: &str) -> Vec<Listener> {
        if data.is_empty() {
            return Vec::new();
        }
        self.trace(&["term-write", session_id, data]);
        let replies = entry.terminal.feed(data);
        for reply in replies {
            self.send(session_id, entry, &reply);
        }
        entry.has_output = true;
        entry.listeners.iter().map(|(_, l)| l.clone()).collect()
    }

    /// `markExited`: the exit line, then `exited` (TERM-013).
    fn mark_exited(&self, session_id: &str, entry: &mut Entry) {
        let line = exit_line(entry.kind);
        self.trace(&["term-write", session_id, line]);
        let replies = entry.terminal.feed(line);
        for reply in replies {
            self.send(session_id, entry, &reply);
        }
        entry.state = State::Exited;
    }

    /// `fitEntry`: fits a shown terminal; a resize reaches the session only
    /// when it is ready (TERM-024). Parked terminals are never fitted (TERM-006).
    fn fit(&self, session_id: &str, entry: &mut Entry, fit: Fit) {
        if entry.container.is_none() {
            return;
        }
        let Some((cols, rows)) = fit else {
            return;
        };
        if entry.terminal.resize(cols, rows) && entry.state == State::Ready {
            let (cols, rows) = (entry.terminal.cols(), entry.terminal.rows());
            self.sessions.resize(session_id, cols, rows);
        }
    }

    /// Shows the session's terminal in `container` (fitted to `fit`),
    /// creating it and starting or resuming its process if needed
    /// (TERM-016…TERM-022). Resolves once output is flowing; fails with the
    /// error's message when the process couldn't be started.
    pub async fn attach(
        &self,
        session_id: &str,
        project_id: &str,
        container: ContainerId,
        kind: SessionKind,
        fit: Fit,
    ) -> Result<(), String> {
        let next = {
            let mut inner = self.lock();
            let is_new = !inner.entries.contains_key(session_id);
            if is_new {
                let id = inner.next_id;
                inner.next_id += 1;
                let terminal = Terminal::new(kind, self.platform, DEFAULT_COLS, DEFAULT_ROWS);
                inner.entries.insert(
                    session_id.to_owned(),
                    Entry {
                        id,
                        kind,
                        terminal,
                        state: State::Opening,
                        opening: None,
                        exit_pending: false,
                        has_output: false,
                        listeners: Vec::new(),
                        container: None,
                        waiters: Vec::new(),
                    },
                );
            }
            let entry = inner.entries.get_mut(session_id).expect("just inserted");
            entry.container = Some(container);
            self.fit(session_id, entry, fit);
            if let Some(rx) = &entry.opening {
                // Already opening: the same open, no second process (TERM-020).
                Next::Wait(rx.clone())
            } else if is_new || entry.state == State::Exited {
                if !is_new {
                    // A resumed Claude redraws the whole conversation, and a
                    // restarted shell is a new one: start from a clean screen (TERM-021).
                    self.trace(&["term-reset", session_id]);
                    entry.terminal.reset();
                    entry.has_output = false;
                }
                let (done, rx) = watch::channel(None);
                entry.opening = Some(rx);
                entry.state = State::Opening;
                entry.exit_pending = false;
                Next::Open {
                    id: entry.id,
                    project: project_id.to_owned(),
                    cols: entry.terminal.cols(),
                    rows: entry.terminal.rows(),
                    done,
                }
            } else {
                let (cols, rows) = (entry.terminal.cols(), entry.terminal.rows());
                self.sessions.resize(session_id, cols, rows);
                Next::Done
            }
        };
        match next {
            Next::Done => Ok(()),
            Next::Wait(mut rx) => match rx.wait_for(Option::is_some).await {
                Ok(result) => result.clone().unwrap_or(Ok(())),
                Err(_) => Ok(()),
            },
            Next::Open {
                id,
                project,
                cols,
                rows,
                done,
            } => {
                let result = self.open_process(session_id, &project, id, cols, rows).await;
                done.send_replace(Some(result.clone()));
                result
            }
        }
    }

    /// `openProcess` (TERM-017…TERM-019).
    async fn open_process(
        &self,
        session_id: &str,
        project_id: &str,
        id: u64,
        cols: usize,
        rows: usize,
    ) -> Result<(), String> {
        let reply = self.sessions.open(project_id, session_id, cols, rows).await;
        let mut listeners = Vec::new();
        let result = {
            let mut inner = self.lock();
            let Inner { entries, pending, .. } = &mut *inner;
            match entries.get_mut(session_id).filter(|e| e.id == id) {
                // Disposed meanwhile: nothing more (TERM-017).
                None => Ok(()),
                Some(entry) => {
                    entry.opening = None;
                    match reply {
                        Ok(replay) => {
                            listeners = self.write_output(session_id, entry, &replay);
                            entry.state = State::Ready;
                            // The size may have changed while opening (TERM-018).
                            let (cols, rows) = (entry.terminal.cols(), entry.terminal.rows());
                            self.sessions.resize(session_id, cols, rows);
                            if entry.exit_pending {
                                entry.exit_pending = false;
                                self.mark_exited(session_id, entry);
                            } else if pending.contains_key(session_id) {
                                self.type_pending(session_id, entry, pending);
                            }
                            Ok(())
                        }
                        // Left resumable: the next attach retries (TERM-019).
                        Err(message) => {
                            entry.state = State::Exited;
                            Err(message)
                        }
                    }
                }
            }
        };
        for listener in listeners {
            listener();
        }
        result
    }

    /// Starts a terminal whose process exited over again from what the open
    /// replays, once the session is live again without this terminal having
    /// asked: a server moved it to another Claude account (SRV-104), or
    /// someone else resumed it. Only an exited terminal; resolves once the
    /// replay is written.
    pub async fn revive(&self, session_id: &str, project_id: &str) -> Result<(), String> {
        let (id, cols, rows, done) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(session_id).filter(|e| e.state == State::Exited) else {
                return Ok(());
            };
            self.trace(&["term-reset", session_id]);
            entry.terminal.reset();
            entry.has_output = false;
            let (done, rx) = watch::channel(None);
            entry.opening = Some(rx);
            entry.state = State::Opening;
            entry.exit_pending = false;
            (entry.id, entry.terminal.cols(), entry.terminal.rows(), done)
        };
        let result = self.open_process(session_id, project_id, id, cols, rows).await;
        done.send_replace(Some(result.clone()));
        result
    }

    /// Whether the session's terminal shows its process exited.
    pub fn is_exited(&self, session_id: &str) -> bool {
        self.lock()
            .entries
            .get(session_id)
            .is_some_and(|e| e.state == State::Exited)
    }

    /// Opens a live session again and starts its terminal over from what
    /// the open replays: for a server session whose output stopped coming
    /// while the connection was down (SRV-028). Only a terminal that's
    /// ready is opened again; one opening or exited waits for its next
    /// attach, as before. Resolves once the replay is written.
    pub async fn reopen(&self, session_id: &str, project_id: &str) -> Result<(), String> {
        let (id, cols, rows, done) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(session_id).filter(|e| e.state == State::Ready) else {
                return Ok(());
            };
            self.trace(&["term-reset", session_id]);
            entry.terminal.reset();
            entry.has_output = false;
            let (done, rx) = watch::channel(None);
            entry.opening = Some(rx);
            entry.state = State::Opening;
            entry.exit_pending = false;
            (entry.id, entry.terminal.cols(), entry.terminal.rows(), done)
        };
        let result = self.open_process(session_id, project_id, id, cols, rows).await;
        done.send_replace(Some(result.clone()));
        result
    }

    /// Parks the terminal, keeping everything for the next attach (TERM-003,
    /// TERM-005). With `container`, only if it is still shown there (TERM-004).
    pub fn detach(&self, session_id: &str, container: Option<ContainerId>) {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(session_id) else {
            return;
        };
        if container.is_some() && entry.container != container {
            return;
        }
        self.trace(&["term-blur", session_id]);
        if let Some(report) = entry.terminal.blur() {
            self.send(session_id, entry, &report);
        }
        entry.container = None;
    }

    /// Destroys the terminal and forgets its queued words (TERM-027).
    pub fn dispose(&self, session_id: &str) {
        let mut inner = self.lock();
        inner.pending.remove(session_id);
        if let Some(entry) = inner.entries.remove(session_id) {
            self.trace(&["term-dispose", session_id]);
            for (_, timer) in entry.waiters {
                timer.abort();
            }
        }
    }

    /// Refits to `container`'s new size, if the terminal is shown there (TERM-023).
    pub fn refit(&self, session_id: &str, container: ContainerId, fit: Fit) {
        let mut inner = self.lock();
        if let Some(entry) = inner
            .entries
            .get_mut(session_id)
            .filter(|e| e.container == Some(container))
        {
            self.fit(session_id, entry, fit);
        }
    }

    /// Gives the terminal keyboard focus; nothing while it is parked (TERM-085).
    pub fn focus(&self, session_id: &str) {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(session_id) else {
            return;
        };
        self.trace(&["term-focus", session_id]);
        if entry.container.is_none() {
            return;
        }
        if let Some(report) = entry.terminal.focus() {
            self.send(session_id, entry, &report);
        }
    }

    /// The terminal lost keyboard focus to something else.
    pub fn blur(&self, session_id: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.get_mut(session_id)
            && let Some(report) = entry.terminal.blur()
        {
            self.send(session_id, entry, &report);
        }
    }

    /// Whether the terminal has shown any output since it was created or reset (TERM-026).
    pub fn has_output(&self, session_id: &str) -> bool {
        self.lock().entries.get(session_id).is_some_and(|e| e.has_output)
    }

    /// Whether the next attach will start or resume a process (TERM-026).
    pub fn needs_open(&self, session_id: &str) -> bool {
        self.lock()
            .entries
            .get(session_id)
            .is_none_or(|e| e.state != State::Ready)
    }

    /// Whether the session has a terminal.
    pub fn contains(&self, session_id: &str) -> bool {
        self.lock().entries.contains_key(session_id)
    }

    /// Runs `f` on the session's terminal (to draw it, select, scroll…).
    pub fn with_terminal<R>(&self, session_id: &str, f: impl FnOnce(&mut Terminal) -> R) -> Option<R> {
        self.lock().entries.get_mut(session_id).map(|e| f(&mut e.terminal))
    }

    /// Calls `listener` whenever output is written to the session's terminal
    /// (`onOutput`); `None` when it has no terminal.
    pub fn on_output(&self, session_id: &str, listener: impl Fn() + Send + Sync + 'static) -> Option<ListenerId> {
        let mut inner = self.lock();
        let id = ListenerId(inner.next_id);
        inner.next_id += 1;
        let entry = inner.entries.get_mut(session_id)?;
        entry.listeners.push((id, Arc::new(listener)));
        Some(id)
    }

    pub fn off_output(&self, session_id: &str, id: ListenerId) {
        if let Some(entry) = self.lock().entries.get_mut(session_id) {
            entry.listeners.retain(|(l, _)| *l != id);
        }
    }

    /// Output from the session's process (`ev:pty-data`, TERM-008…TERM-011).
    pub fn pty_data(&self, session_id: &str, data: &str) {
        let listeners = {
            let mut inner = self.lock();
            let Inner { entries, pending, .. } = &mut *inner;
            let Some(entry) = entries.get_mut(session_id).filter(|e| e.state == State::Ready) else {
                // No terminal: the host keeps it for the next open's replay.
                // Opening: it is already in the pending replay. Exited: dropped.
                return;
            };
            let listeners = self.write_output(session_id, entry, data);
            if !data.is_empty() {
                self.check_listening(session_id, entry, pending);
            }
            listeners
        };
        for listener in listeners {
            listener();
        }
    }

    /// The session's process exited (`ev:pty-exit`, TERM-012…TERM-014).
    pub fn pty_exit(&self, session_id: &str) {
        let mut inner = self.lock();
        let Inner { entries, pending, .. } = &mut *inner;
        let Some(entry) = entries.get_mut(session_id) else {
            return;
        };
        match entry.state {
            State::Opening => entry.exit_pending = true,
            State::Ready => {
                self.mark_exited(session_id, entry);
                self.check_listening(session_id, entry, pending);
            }
            State::Exited => {}
        }
    }

    /// Something the terminal sends as typed input (a key, text, a focus
    /// report): to the session unless the process has exited (TERM-025).
    pub fn input(&self, session_id: &str, data: &str) {
        let inner = self.lock();
        if let Some(entry) = inner.entries.get(session_id) {
            self.send(session_id, entry, data);
        }
    }

    /// A key went down in the session's terminal: applies it, sends what it
    /// types, and returns what the UI does next.
    pub fn key_down(&self, session_id: &str, ev: &KeyEvent) -> Option<KeyAction> {
        let mut inner = self.lock();
        let entry = inner.entries.get_mut(session_id)?;
        let action = entry.terminal.key_down(ev);
        match &action {
            KeyAction::Send(data) => self.send(session_id, entry, data),
            // ⇧Enter goes straight to the session, exited or not (TERM-036).
            KeyAction::WriteDirect(data) => self.sessions.write(session_id, data),
            _ => {}
        }
        Some(action)
    }

    /// Text from the platform's text input or a finished IME composition.
    pub fn input_text(&self, session_id: &str, text: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.get_mut(session_id) {
            let data = entry.terminal.input_text(text);
            self.send(session_id, entry, &data);
        }
    }

    /// Pastes `text` as xterm does (TERM-035, TERM-044).
    pub fn paste(&self, session_id: &str, text: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.get_mut(session_id) {
            self.paste_into(session_id, entry, text);
        }
    }

    fn paste_into(&self, session_id: &str, entry: &mut Entry, text: &str) {
        self.trace(&["term-paste", session_id, text]);
        let data = entry.terminal.paste(text);
        self.send(session_id, entry, &data);
    }

    /// Files dropped on the terminal: their paths pasted, quoted, then focus (TERM-032).
    pub fn drop_paths<S: AsRef<str>>(&self, session_id: &str, paths: &[S]) {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(session_id) else {
            return;
        };
        if let Some(text) = crate::paste::dropped_paths(paths) {
            self.paste_into(session_id, entry, &text);
        }
        self.trace(&["term-focus", session_id]);
        if entry.container.is_some()
            && let Some(report) = entry.terminal.focus()
        {
            self.send(session_id, entry, &report);
        }
    }

    /// Types `text` at the session's prompt without sending it, as a paste
    /// ending in a space (TERM-028): at once if the terminal is ready and
    /// nothing is queued, else once it opens, and for a `starting` session
    /// once Claude is listening (TERM-029…TERM-031).
    pub fn type_in(&self, session_id: &str, text: &str, starting: bool) {
        let mut inner = self.lock();
        let Inner { entries, pending, .. } = &mut *inner;
        let words = format!("{} ", nodus_domain::js::trim(text));
        let queued = pending.get(session_id);
        if let Some(entry) = entries.get_mut(session_id).filter(|e| e.state == State::Ready)
            && queued.is_none()
        {
            self.paste_into(session_id, entry, &words);
            return;
        }
        let (text, was_starting) = queued.map_or((String::new(), false), |q| (q.text.clone(), q.starting));
        pending.insert(
            session_id.to_owned(),
            Pending {
                text: text + &words,
                starting: starting || was_starting,
            },
        );
    }

    /// `typePending`, run once the replay has been written.
    fn type_pending(&self, session_id: &str, entry: &mut Entry, pending: &mut HashMap<String, Pending>) {
        let Some(queued) = pending.get(session_id) else {
            return;
        };
        if !queued.starting {
            // The words go in once the replay has been parsed, which it now is.
            self.trace(&["term-write", session_id, ""]);
            self.type_now(session_id, entry, pending);
            return;
        }
        // Claude turns on bracketed paste once its prompt reads the keyboard.
        if entry.terminal.modes().bracketed_paste {
            self.type_now(session_id, entry, pending);
            return;
        }
        let waiter = entry.id << 20 | (entry.waiters.len() as u64 + 1);
        let this = self.this.clone();
        let session = session_id.to_owned();
        let entry_id = entry.id;
        let timer = self.runtime.spawn(async move {
            tokio::time::sleep(LISTEN_LIMIT).await;
            if let Some(registry) = this.upgrade() {
                registry.listen_limit(&session, entry_id, waiter);
            }
        });
        entry.waiters.push((waiter, timer));
    }

    /// `type`: only into the same terminal while it is ready; otherwise the
    /// words wait for the next open (TERM-031).
    fn type_now(&self, session_id: &str, entry: &mut Entry, pending: &mut HashMap<String, Pending>) {
        if entry.state != State::Ready {
            return;
        }
        if let Some(queued) = pending.remove(session_id) {
            self.paste_into(session_id, entry, &queued.text);
        }
    }

    /// After output was parsed: waiting starting sessions type once bracketed paste is on.
    fn check_listening(&self, session_id: &str, entry: &mut Entry, pending: &mut HashMap<String, Pending>) {
        if entry.waiters.is_empty() || !entry.terminal.modes().bracketed_paste {
            return;
        }
        for (_, timer) in std::mem::take(&mut entry.waiters) {
            timer.abort();
            self.type_now(session_id, entry, pending);
        }
    }

    /// The 30 s limit passed without bracketed paste (TERM-030).
    fn listen_limit(&self, session_id: &str, entry_id: u64, waiter: u64) {
        let mut inner = self.lock();
        let Inner { entries, pending, .. } = &mut *inner;
        let Some(entry) = entries.get_mut(session_id).filter(|e| e.id == entry_id) else {
            return;
        };
        let before = entry.waiters.len();
        entry.waiters.retain(|(w, _)| *w != waiter);
        if entry.waiters.len() != before {
            self.type_now(session_id, entry, pending);
        }
    }
}

#[cfg(test)]
mod tests;
