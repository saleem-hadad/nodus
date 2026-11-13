//! Making a capture: a session, the next prompt of a session, a note or a
//! view on a project's board, in a free spot near where the board was last
//! left. The "Making it" part of `src/main/capture.ts`.

use nodus_board::BoxFuture;
use nodus_board::geometry::{
    EXPANDED_W, NEW_CARD_H, Point, Size, VIEW_H, VIEW_W, card_size, dock_rect, find_free_spot,
};
use nodus_board::notes::{fit_note, obstacles};
use nodus_domain::js;
use nodus_domain::json::number_to_string;
use nodus_domain::{
    BoardNote, CaptureAttachment, CaptureKind, CaptureReceipt, CreateSessionOptions, Event, EventSink, FileAttachment,
    FolderColor, ImageAttachment, OpenCardRequest, TabAttachment, TextAttachment,
};
use nodus_store::{BoardState, Store};
use serde_json::Value;

use crate::screen::is_web_url;
use crate::targets::{CaptureSessions, remember, session_info};
use crate::text::{TITLE_MAX, title_from, with_screen};

/// How much of what's typed a capture keeps, in UTF-16 units.
pub const MAX_TEXT_CHARS: usize = 20_000;
/// Where captures go when the board window isn't open: its default size.
pub const DEFAULT_WINDOW: (f64, f64) = (1440.0, 920.0);
/// At most this many attachments come along.
pub const MAX_ATTACHMENTS: usize = 20;

/// What capture asks of the other domains and the window. nodus-core
/// implements it; the TypeScript's `CaptureHandlers`.
pub trait CaptureHandlers: Send + Sync {
    /// The board window's content size, while it's open (CAP-134).
    fn window_size(&self) -> Option<(f64, f64)>;
    /// The board's own create path (nodus-board's `Board::create_session`
    /// with the launcher): a new card, its process started. Answers its
    /// session id, or the error to show as it is.
    fn create_session(&self, project_id: &str, opts: CreateSessionOptions) -> BoxFuture<'_, Result<String, String>>;
    /// The board's send path (nodus-board's `Board::send_to_session`):
    /// typed into a live session, refused for one that's waiting, or resumed
    /// with it. The error is shown as it is.
    fn send_to_session(&self, project_id: &str, session_id: &str, text: &str) -> BoxFuture<'_, Result<(), String>>;
    /// A session's title as its card shows it (nodus-board's `Board::session_title`).
    fn session_title(&self, project_id: &str, session_id: &str) -> String;
    /// nodus-views' `createView(projectId, { description, x, y })`: titled
    /// `New view`, its brain started on it. Answers the view's id.
    fn create_view(&self, project_id: &str, description: &str, at: Point) -> BoxFuture<'_, Result<String, String>>;
    /// Brings the board window up on a card or a board (`notify.openCard`).
    fn open_card(&self, request: OpenCardRequest);
    /// Brings the board window up and opens its Settings (`menu: settings`).
    fn open_settings(&self);
}

/// Why a capture wasn't made. The text is what the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    #[error("Type something first")]
    NoText,
    #[error("Pick what to make")]
    NoKind,
    /// `Unknown project <id>`, the id as JavaScript writes it.
    #[error("Unknown project {0}")]
    UnknownProject(String),
    #[error("That session is no longer on the board")]
    NotOnBoard,
    /// From the board or the views: shown as it is.
    #[error("{0}")]
    Failed(String),
}

/// `str(v, max)`: a string cut to `max` UTF-16 units, else empty.
fn str_of(value: Option<&Value>, max: usize) -> String {
    match value {
        Some(Value::String(s)) => js::slice_utf16(s, 0, max),
        _ => String::new(),
    }
}

/// `String(v)`, for the id in `Unknown project <id>`.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `Number(v) || 0`, as a file size.
fn size_of_value(value: Option<&Value>) -> u64 {
    let n = match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::String(s)) => {
            let s = js::trim(s);
            if s.is_empty() {
                0.0
            } else {
                s.parse::<f64>().unwrap_or(0.0)
            }
        }
        _ => 0.0,
    };
    // A file's size is a whole number of bytes; anything else reads as 0.
    if n.is_finite() && n > 0.0 { n as u64 } else { 0 }
}

/// Attachments from the page, kept to their known shapes (CAP-135): the
/// first 20; a tab only with a web URL; an image or a file only with a
/// path; copied text only when it isn't blank. Thumbnails are dropped.
/// Paths aren't checked against the disk.
pub fn clean_attachments(raw: &Value) -> Vec<CaptureAttachment> {
    let Value::Array(items) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items.iter().take(MAX_ATTACHMENTS) {
        let Value::Object(a) = item else {
            continue;
        };
        let field = |name: &str| a.get(name);
        let path = str_of(field("path"), 4000);
        match field("kind").and_then(Value::as_str) {
            Some("tab") if is_web_url(&str_of(field("url"), 4000)) => out.push(CaptureAttachment::Tab(TabAttachment {
                url: str_of(field("url"), 4000),
                title: str_of(field("title"), 500),
                app: str_of(field("app"), 100),
            })),
            Some("image") if !path.is_empty() => out.push(CaptureAttachment::Image(ImageAttachment {
                path,
                thumb: String::new(),
                copied: Some(field("copied") == Some(&Value::Bool(true))),
            })),
            Some("file") if !path.is_empty() => out.push(CaptureAttachment::File(FileAttachment {
                path,
                name: str_of(field("name"), 500),
                size: size_of_value(field("size")),
            })),
            Some("text") => {
                let text = str_of(field("text"), MAX_TEXT_CHARS);
                if !js::trim(&text).is_empty() {
                    out.push(CaptureAttachment::Text(TextAttachment { text }));
                }
            }
            _ => {}
        }
    }
    out
}

/// [`clean_attachments`] for attachments the window already typed.
pub fn clean(attachments: &[CaptureAttachment]) -> Vec<CaptureAttachment> {
    clean_attachments(&serde_json::to_value(attachments).unwrap_or(Value::Null))
}

/// A string JavaScript reads as true.
fn set(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Near the middle of the board as the window last showed it, in the
/// nearest free spot (CAP-133); without a saved view of it, right of the
/// right-most card, else right of the archive dock.
pub fn spot_on_board(board: &BoardState, size: Size, window: (f64, f64)) -> Point {
    let dock = dock_rect(board.archive);
    let preferred = match board.viewport {
        Some(vp) if vp.zoom > 0.0 => Point {
            x: (window.0 / 2.0 - vp.x) / vp.zoom - size.w / 2.0,
            y: (window.1 / 2.0 - vp.y) / vp.zoom - size.h / 2.0,
        },
        _ => {
            let cards = board
                .nodes
                .values()
                .filter(|n| !n.archived && !set(n.run_of.as_deref()) && !set(n.view_of.as_deref()));
            // The right-most; on a tie, the earlier one.
            let right = cards.fold(None, |best: Option<&nodus_domain::BoardNode>, n| match best {
                Some(b) if b.x >= n.x => Some(b),
                _ => Some(n),
            });
            match right {
                Some(card) => Point {
                    x: card.x + card_size(Some(card)).w + 32.0,
                    y: card.y,
                },
                None => Point {
                    x: dock.x + dock.w + 80.0,
                    y: dock.y,
                },
            }
        }
    };
    let spot = find_free_spot(preferred, &obstacles(board, None), Some(dock), size);
    Point {
        x: js::round(spot.x),
        y: js::round(spot.y),
    }
}

/// What making a capture reaches.
pub struct Maker<'a> {
    pub store: &'a Store,
    pub handlers: &'a dyn CaptureHandlers,
    pub sessions: &'a dyn CaptureSessions,
    pub events: &'a dyn EventSink,
    /// `Date.now()`.
    pub now: &'a (dyn Fn() -> f64 + Send + Sync),
}

impl Maker<'_> {
    /// `capture:submit` (CAP-125…CAP-130, CAP-132): checks what came, makes
    /// it, remembers it for the recents, tells an open board, and answers the
    /// receipt. What fails part-way leaves nothing behind.
    pub async fn submit(&self, raw: &Value) -> Result<CaptureReceipt, SubmitError> {
        let field = |name: &str| raw.get(name);
        let text = str_of(field("text"), MAX_TEXT_CHARS);
        let text = js::trim(&text).to_owned();
        if text.is_empty() {
            return Err(SubmitError::NoText);
        }
        let kind = field("kind")
            .and_then(Value::as_str)
            .and_then(CaptureKind::parse)
            .ok_or(SubmitError::NoKind)?;
        let project_id = field("projectId").and_then(Value::as_str).unwrap_or_default();
        let project = self
            .store
            .read(|state| state.project(project_id).ok().cloned())
            .ok_or_else(|| SubmitError::UnknownProject(js_string(field("projectId"))))?;
        let board = self.board(&project.id);
        let attachments = clean_attachments(field("attachments").unwrap_or(&Value::Null));
        let prompt = with_screen(&text, &attachments);
        let window = self.handlers.window_size().unwrap_or(DEFAULT_WINDOW);
        let mut receipt = CaptureReceipt {
            kind,
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            title: title_from(&text, TITLE_MAX),
            session_id: None,
            note_id: None,
            view_id: None,
        };

        match kind {
            // CAP-126: like the prompt bar's, expanded in a free spot, in auto
            // mode, with the words stuck on it.
            CaptureKind::Session => {
                let size = Size {
                    w: EXPANDED_W,
                    h: NEW_CARD_H,
                };
                let at = spot_on_board(&board, size, window);
                let opts = CreateSessionOptions {
                    x: at.x,
                    y: at.y,
                    kind: None,
                    name: None,
                    initial_prompt: Some(prompt),
                    parents: None,
                    action_id: None,
                    expanded: Some(true),
                    auto_mode: Some(true),
                    from_view: None,
                    model: None,
                };
                let session_id = self
                    .handlers
                    .create_session(&project.id, opts)
                    .await
                    .map_err(SubmitError::Failed)?;
                let note = BoardNote {
                    id: uuid::Uuid::new_v4().to_string(),
                    text: text.clone(),
                    color: FolderColor::Yellow,
                    x: at.x,
                    y: at.y,
                    w: None,
                    h: None,
                    session_id: Some(session_id.clone()),
                    from: None,
                    from_project: None,
                    from_title: None,
                    created_at: (self.now)(),
                    extra: Default::default(),
                };
                self.store.update(|state| {
                    state.board_mut(&project.id).notes.insert(note.id.clone(), note);
                });
                receipt.session_id = Some(session_id);
            }
            // CAP-127
            CaptureKind::Into => {
                let session_id = str_of(field("sessionId"), 100);
                let on_board = !session_id.is_empty()
                    && self
                        .store
                        .read(|state| session_info(state, &project.id, &session_id, self.sessions).is_some());
                if !on_board {
                    return Err(SubmitError::NotOnBoard);
                }
                self.handlers
                    .send_to_session(&project.id, &session_id, &prompt)
                    .await
                    .map_err(SubmitError::Failed)?;
                receipt.title = self.handlers.session_title(&project.id, &session_id);
                receipt.session_id = Some(session_id);
            }
            // CAP-128
            CaptureKind::Note => {
                let size = fit_note(&prompt);
                let at = spot_on_board(&board, size, window);
                let note = BoardNote {
                    id: uuid::Uuid::new_v4().to_string(),
                    text: prompt,
                    color: FolderColor::Yellow,
                    x: at.x,
                    y: at.y,
                    w: Some(size.w),
                    h: Some(size.h),
                    session_id: None,
                    from: None,
                    from_project: None,
                    from_title: None,
                    created_at: (self.now)(),
                    extra: Default::default(),
                };
                receipt.note_id = Some(note.id.clone());
                self.store.update(|state| {
                    state.board_mut(&project.id).notes.insert(note.id.clone(), note);
                });
            }
            // CAP-129
            CaptureKind::View => {
                let at = spot_on_board(&board, Size { w: VIEW_W, h: VIEW_H }, window);
                let view_id = self
                    .handlers
                    .create_view(&project.id, &prompt, at)
                    .await
                    .map_err(SubmitError::Failed)?;
                receipt.view_id = Some(view_id);
            }
        }
        // CAP-130
        let now = (self.now)();
        self.store
            .update(|state| remember(state, &receipt, uuid::Uuid::new_v4().to_string(), now));
        self.events.emit(Event::BoardChanged {
            project_id: project.id.clone(),
        });
        Ok(receipt)
    }

    /// `getBoard(id)`: the project's board, made empty first if it has none.
    fn board(&self, project_id: &str) -> BoardState {
        if let Some(board) = self.store.read(|state| state.board(project_id).cloned()) {
            return board;
        }
        self.store.update(|state| state.board_mut(project_id).clone())
    }
}

#[cfg(test)]
pub(crate) mod tests;
