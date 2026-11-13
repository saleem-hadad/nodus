//! Tells the user, while Nodus is in the background, when a session in any
//! project needs them or finishes its turn, and counts the sessions waiting on
//! them on the Dock icon. Clicking a notification opens its card.
//!
//! What's said and when is here; showing it is the operating system's, behind
//! [`Notifier`], and the window behind [`AppWindow`]. Ports `src/main/notify.ts`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nodus_domain::{Event, EventSink, OpenCardRequest, SessionKind, SessionMeta, SessionStatus};
use nodus_store::{State, Store};

use crate::hooks::StatusMachine;
use crate::jsvalue::{collapse_spaces, trim};

/// What a change of status is worth telling the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The session started needing the user.
    Waiting,
    /// Its turn ended.
    Finished,
}

/// HOST-202: what a session's change of status is worth telling the user, if anything.
pub fn notice_for(previous: Option<SessionStatus>, status: SessionStatus, live: bool) -> Option<Notice> {
    match status {
        SessionStatus::Waiting => (previous != Some(SessionStatus::Waiting)).then_some(Notice::Waiting),
        // A process that went (closed, or its host lost) finished nothing.
        SessionStatus::Done if previous == Some(SessionStatus::Working) && live => Some(Notice::Finished),
        _ => None,
    }
}

/// HOST-214: sessions waiting on the user across every project: the red
/// cards, and action runs with a question. The same ones the project folders count.
pub fn waiting_count(state: &State, is_live: impl Fn(&str) -> bool, status: impl Fn(&str) -> SessionStatus) -> u32 {
    let mut count = 0;
    for board in state.boards.values() {
        for node in board.nodes.values() {
            // A view's brain isn't on the board.
            if node.view_of.as_deref().is_some_and(|v| !v.is_empty()) || node.archived || !is_live(&node.session_id) {
                continue;
            }
            if status(&node.session_id) == SessionStatus::Waiting {
                count += 1;
            }
        }
    }
    count
}

/// Identifies a notification shown, so a click on it can be told back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationId(pub u64);

/// A notification about a session (HOST-208).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The card's title, else the transcript's, else `New session`.
    pub title: String,
    /// The project's name; macOS shows it.
    pub subtitle: String,
    pub body: String,
    /// Gathers a project's notifications in Notification Center: the project's id.
    pub group_id: String,
}

/// The operating system's notifications and Dock badge.
pub trait Notifier: Send + Sync {
    /// Whether the system shows notifications at all (HOST-210).
    fn is_supported(&self) -> bool;
    /// Shows a notification. A click on it is passed to [`Notify::clicked`] with `id`.
    fn show(&self, id: NotificationId, notification: &Notification);
    /// Takes a notification back, from the screen and Notification Center.
    fn close(&self, id: NotificationId);
    /// Whether there's a Dock icon to badge (macOS).
    fn has_dock(&self) -> bool;
    /// Sets the Dock icon's badge; empty clears it.
    fn set_badge(&self, text: &str);
}

/// The app's window, as notifications need it.
pub trait AppWindow: Send + Sync {
    /// Whether there's a window that hasn't been closed.
    fn is_open(&self) -> bool;
    /// Whether that window has focus.
    fn is_focused(&self) -> bool;
    /// Whether it's still loading, so it can't take a request yet and asks
    /// for it with `app:take-open-request` once it can.
    fn is_loading(&self) -> bool;
    /// Brings the window to the front: opened again if it was closed,
    /// restored if it was minimised.
    fn show(&self);
}

/// What notifications say about a session, from its transcript (03's `readSession`).
pub trait SessionMetas: Send + Sync {
    fn session_meta(&self, repo_path: &str, session_id: &str) -> Option<SessionMeta>;
}

/// Notifications and the Dock badge.
pub struct Notify {
    store: Arc<Store>,
    status: StatusMachine,
    notifier: Arc<dyn Notifier>,
    window: Arc<dyn AppWindow>,
    metas: Arc<dyn SessionMetas>,
    /// Where `ev:open-card` goes.
    sink: Arc<dyn EventSink>,
    state: Mutex<NotifyState>,
}

#[derive(Default)]
struct NotifyState {
    /// Hooks replay what happened while the app was closed; none of that is news.
    armed: bool,
    /// HOST-203: each live session's last status, to tell what changed.
    statuses: HashMap<String, SessionStatus>,
    /// The latest notification about each session, kept so its click still
    /// opens the card and it can be taken back.
    shown: HashMap<String, Shown>,
    /// A card to open once a window that was closed has loaded again.
    pending_open: Option<OpenCardRequest>,
    badge: u32,
    next_id: u64,
}

struct Shown {
    id: NotificationId,
    notice: Notice,
    project_id: String,
}

/// The card a notification about a session would open, and what to say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    pub project_id: String,
    pub project_name: String,
    /// The card's title, else the transcript's, else `New session`.
    pub title: String,
    /// Claude's last reply, for a finished turn.
    pub last_reply: Option<String>,
}

impl Notify {
    pub fn new(
        store: Arc<Store>,
        status: StatusMachine,
        notifier: Arc<dyn Notifier>,
        window: Arc<dyn AppWindow>,
        metas: Arc<dyn SessionMetas>,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            store,
            status,
            notifier,
            window,
            metas,
            sink,
            state: Mutex::default(),
        }
    }

    /// HOST-206: called once the sessions running before the app started
    /// have caught up (or failed to), so replayed history never notifies.
    pub fn arm(&self) {
        self.lock().armed = true;
        self.update_badge();
    }

    /// Every change of a session's status passes through here (HOST-220).
    pub fn status_changed(&self, session_id: &str, status: SessionStatus, live: bool) {
        self.changed(session_id, status, live, || self.describe(session_id));
    }

    /// A session on a server board changed status (SRV-185). The board
    /// still shows it to everyone; only the card's starter is told, so the
    /// core gives `about` for the cards this person started, and none for
    /// the rest.
    pub fn server_status_changed(&self, session_id: &str, status: SessionStatus, live: bool, about: Option<About>) {
        self.changed(session_id, status, live, || about);
    }

    fn changed(&self, session_id: &str, status: SessionStatus, live: bool, about: impl FnOnce() -> Option<About>) {
        let (previous, withdrawn, armed) = {
            let mut state = self.lock();
            let previous = if live {
                state.statuses.insert(session_id.to_owned(), status)
            } else {
                state.statuses.remove(session_id)
            };
            // HOST-204: a notification saying a session needs the user no longer holds once it doesn't.
            let withdrawn = if previous == Some(SessionStatus::Waiting) && status != SessionStatus::Waiting {
                match state.shown.get(session_id) {
                    Some(shown) if shown.notice == Notice::Waiting => state.shown.remove(session_id).map(|s| s.id),
                    _ => None,
                }
            } else {
                None
            };
            (previous, withdrawn, state.armed)
        };
        if let Some(id) = withdrawn {
            self.notifier.close(id);
        }
        self.update_badge();

        // HOST-205
        let Some(notice) = notice_for(previous, status, live) else {
            return;
        };
        if armed
            && self.store.read(|state| state.settings.notify)
            && self.in_background()
            && let Some(about) = about()
        {
            self.show(session_id, notice, about);
        }
    }

    /// HOST-215: shows how many sessions are waiting on the user on the Dock
    /// icon, when that's on. Set only when the number changes.
    pub fn update_badge(&self) {
        if !self.lock().armed || !self.notifier.has_dock() {
            return;
        }
        let count = self.store.read(|state| {
            if state.settings.dock_badge {
                waiting_count(state, |id| self.status.is_live(id), |id| self.status.status(id))
            } else {
                0
            }
        });
        {
            let mut state = self.lock();
            if count == state.badge {
                return;
            }
            state.badge = count;
        }
        self.notifier
            .set_badge(&if count > 0 { count.to_string() } else { String::new() });
    }

    /// HOST-212: brings the window up on a card, or on a project's board.
    pub fn open_card(&self, request: OpenCardRequest) {
        let exists = self
            .store
            .read(|state| state.projects.iter().any(|p| p.id == request.project_id));
        if !exists {
            return self.window.show();
        }
        let reopened = !self.window.is_open() || self.window.is_loading();
        self.window.show();
        // A window that's still loading picks the request up itself (take_open_request).
        if reopened {
            self.lock().pending_open = Some(request);
        } else {
            self.sink.emit(Event::OpenCard(request));
        }
    }

    /// HOST-213: the card a notification asked to open before the window had
    /// loaded, once.
    pub fn take_open_request(&self) -> Option<OpenCardRequest> {
        self.lock().pending_open.take()
    }

    /// HOST-211: the user clicked a notification. It opens its project and,
    /// if the session is still on that project's board, its card; a finished
    /// action run has left the board, so only the board opens.
    pub fn clicked(&self, id: NotificationId) {
        let found = {
            let mut state = self.lock();
            let session_id = state
                .shown
                .iter()
                .find(|(_, shown)| shown.id == id)
                .map(|(session_id, _)| session_id.clone());
            session_id.and_then(|session_id| {
                let shown = state.shown.remove(&session_id)?;
                Some((session_id, shown.project_id))
            })
        };
        let Some((session_id, project_id)) = found else {
            return;
        };
        // A server board's cards are on its server, which the board asks.
        let on_board = self.store.read(|state| {
            state.project_id_for_session(&session_id) == Some(project_id.as_str())
                || state.project_server(&project_id).is_some()
        });
        self.open_card(OpenCardRequest {
            project_id,
            session_id: on_board.then_some(session_id),
        });
    }

    fn in_background(&self) -> bool {
        !self.window.is_open() || !self.window.is_focused()
    }

    /// HOST-207: only a session whose card is on a project's board, and isn't
    /// a view's brain or a terminal, notifies.
    fn describe(&self, session_id: &str) -> Option<About> {
        let (project_id, project_name, repo_path, node_title) = self.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            let project = state.projects.iter().find(|p| p.id == project_id)?;
            let node = state.boards.get(project_id)?.nodes.get(session_id)?;
            if node.view_of.as_deref().is_some_and(|v| !v.is_empty()) || node.kind == Some(SessionKind::Terminal) {
                return None;
            }
            Some((
                project.id.clone(),
                project.name.clone(),
                project.repo_path.clone(),
                node.title.clone(),
            ))
        })?;
        let meta = self.metas.session_meta(&repo_path, session_id);
        let title = node_title
            .filter(|t| !t.is_empty())
            .or_else(|| meta.as_ref().map(|m| m.title.clone()).filter(|t| !t.is_empty()))
            .unwrap_or_else(|| "New session".into());
        let last_reply = meta
            .and_then(|m| m.last_assistant_text)
            .map(|text| trim(&collapse_spaces(&text)).to_owned());
        Some(About {
            project_id,
            project_name,
            title,
            last_reply,
        })
    }

    /// HOST-208, HOST-209
    fn show(&self, session_id: &str, notice: Notice, about: About) {
        if !self.notifier.is_supported() {
            return;
        }
        let body = match notice {
            Notice::Waiting => "Needs you".to_owned(),
            Notice::Finished => about
                .last_reply
                .filter(|reply| !reply.is_empty())
                .unwrap_or_else(|| "Finished".into()),
        };
        let notification = Notification {
            title: about.title,
            subtitle: about.project_name,
            body,
            group_id: about.project_id.clone(),
        };
        let (id, previous) = {
            let mut state = self.lock();
            state.next_id += 1;
            let id = NotificationId(state.next_id);
            let previous = state.shown.insert(
                session_id.to_owned(),
                Shown {
                    id,
                    notice,
                    project_id: about.project_id,
                },
            );
            (id, previous)
        };
        if let Some(previous) = previous {
            self.notifier.close(previous.id);
        }
        self.notifier.show(id, &notification);
    }

    fn lock(&self) -> MutexGuard<'_, NotifyState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
