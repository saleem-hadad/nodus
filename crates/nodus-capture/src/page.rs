//! What the panel's page decides, apart from how it looks: where a capture
//! goes, choosing and cycling kinds, when to ask Claude, the sent-text
//! history, the clipboard offer's key, and the words and numbers it shows.
//! The logic of `src/renderer/src/capture/CaptureApp.tsx`, for the UI track
//! to draw from; nothing here draws.

use nodus_domain::js;
use nodus_domain::json::number_to_string;
use nodus_domain::{
    CaptureAttachment, CaptureGuess, CaptureKind, CaptureRecent, CaptureSession, CaptureTargets, ClipboardOffer,
    SessionStatus, TabAttachment,
};

/// How long typing has to pause before Claude is asked again (CAP-064).
pub const GUESS_AFTER_MS: u64 = 700;
/// Shorter than this, there's nothing to guess from yet (CAP-064).
pub const MIN_GUESS_CHARS: usize = 3;
/// How long the receipt shows before the panel goes (CAP-147).
pub const RECEIPT_MS: u64 = 2600;
/// How often the targets are read while the panel is up (CAP-118).
pub const POLL_MS: u64 = 2000;
/// Sent texts kept for ↑ (CAP-123).
pub const MAX_HISTORY: usize = 50;
/// The field grows to this many lines (CAP-090).
pub const MAX_ROWS: usize = 8;
/// Room under the open menu for its shadow (CAP-010).
pub const MENU_ROOM: f64 = 48.0;
/// Sessions listed per project in the menu (CAP-111).
pub const MENU_SESSIONS: usize = 8;

/// A kind's words: its label in the menu, its send button's verb, its hint (CAP-106, CAP-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCopy {
    pub label: &'static str,
    pub verb: &'static str,
    pub hint: &'static str,
}

/// The words for each kind, verbatim.
pub fn kind_copy(kind: CaptureKind) -> KindCopy {
    let (label, verb, hint) = match kind {
        CaptureKind::Session => ("New session", "Start session", "Claude starts on it, in auto mode"),
        CaptureKind::Into => ("Send into a session", "Send", "Adds to its conversation"),
        CaptureKind::Note => ("Note", "Put up note", "A sticky on the board, nothing runs"),
        CaptureKind::View => ("View", "Build view", "A live page Claude keeps up to date"),
    };
    KindCopy { label, verb, hint }
}

/// Where a capture goes: a kind, a project and, for `into`, a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub kind: CaptureKind,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
}

/// The route (CAP-084): the user's choice, else Claude's guess, else a new
/// session in the default project (or the first one).
pub fn route(choice: Option<&Route>, guess: Option<&CaptureGuess>, targets: Option<&CaptureTargets>) -> Route {
    if let Some(choice) = choice {
        return choice.clone();
    }
    if let Some(guess) = guess {
        return Route {
            kind: guess.kind,
            project_id: Some(guess.project_id.clone()),
            session_id: guess.session_id.clone(),
        };
    }
    let project_id = targets.and_then(|t| {
        t.default_project_id
            .clone()
            .or_else(|| t.projects.first().map(|p| p.id.clone()))
    });
    Route {
        kind: CaptureKind::Session,
        project_id,
        session_id: None,
    }
}

/// A session that can be sent into: one not waiting on the user.
pub fn sendable(session: &CaptureSession) -> bool {
    session.status != SessionStatus::Waiting
}

/// A session to send into in a project: the one already picked, else the
/// most recent that isn't waiting.
pub fn into_session(sessions: &[CaptureSession], project_id: Option<&str>, current: Option<&str>) -> Option<String> {
    let in_project: Vec<&CaptureSession> = sessions
        .iter()
        .filter(|s| Some(s.project_id.as_str()) == project_id && sendable(s))
        .collect();
    in_project
        .iter()
        .find(|s| current == Some(s.session_id.as_str()))
        .or_else(|| in_project.first())
        .map(|s| s.session_id.clone())
}

/// Picking a kind (CAP-113): into takes the route's session if it's still
/// sendable, else its project's first sendable one, else the first sendable
/// one anywhere; None when there's none at all (nothing happens).
pub fn pick_kind(kind: CaptureKind, route: &Route, sessions: &[CaptureSession]) -> Option<Route> {
    if kind != CaptureKind::Into {
        return Some(Route {
            kind,
            project_id: route.project_id.clone(),
            session_id: None,
        });
    }
    if let Some(session_id) = into_session(sessions, route.project_id.as_deref(), route.session_id.as_deref()) {
        return Some(Route {
            kind,
            project_id: route.project_id.clone(),
            session_id: Some(session_id),
        });
    }
    // None here: the most recent session anywhere.
    let any = sessions.iter().find(|s| sendable(s))?;
    Some(Route {
        kind,
        project_id: Some(any.project_id.clone()),
        session_id: Some(any.session_id.clone()),
    })
}

/// Tab (`step` 1) and ⇧Tab (-1) (CAP-114): the next kind in order session,
/// into, note, view, without into when nothing can be sent into, wrapping.
pub fn cycle_kind(step: i32, route: &Route, sessions: &[CaptureSession]) -> Option<Route> {
    let can_into = sessions.iter().any(sendable);
    let order: Vec<CaptureKind> = CaptureKind::ALL
        .iter()
        .copied()
        .filter(|k| *k != CaptureKind::Into || can_into)
        .collect();
    let len = order.len() as i32;
    // `indexOf` is -1 for a kind that was left out.
    let at = order.iter().position(|k| *k == route.kind).map_or(-1, |i| i as i32);
    let next = order[((at + step + len) % len) as usize];
    pick_kind(next, route, sessions)
}

/// Picking a project (CAP-115): into its first sendable session, or a new
/// session there when it has none; other kinds keep their kind.
pub fn pick_project(project_id: &str, route: &Route, sessions: &[CaptureSession]) -> Route {
    if route.kind == CaptureKind::Into {
        return match into_session(sessions, Some(project_id), None) {
            Some(session_id) => Route {
                kind: CaptureKind::Into,
                project_id: Some(project_id.to_owned()),
                session_id: Some(session_id),
            },
            None => Route {
                kind: CaptureKind::Session,
                project_id: Some(project_id.to_owned()),
                session_id: None,
            },
        };
    }
    Route {
        kind: route.kind,
        project_id: Some(project_id.to_owned()),
        session_id: None,
    }
}

/// Picking a session (CAP-116): into it, in its own project.
pub fn pick_session(session: &CaptureSession) -> Route {
    Route {
        kind: CaptureKind::Into,
        project_id: Some(session.project_id.clone()),
        session_id: Some(session.session_id.clone()),
    }
}

/// Claude's guess as the page keeps it (CAP-064…CAP-066): a run counter so
/// a slow answer to old text is dropped, the text the guess on screen was
/// for, and whether one is in flight.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuessState {
    run: u64,
    /// The text, trimmed, the guess on screen was made for.
    pub guessed_for: String,
    pub guessing: bool,
    pub guess: Option<CaptureGuess>,
}

impl GuessState {
    /// Whether to ask 700 ms after this text last changed (CAP-064).
    pub fn wants_guess(&self, text: &str, has_choice: bool) -> bool {
        let trimmed = js::trim(text);
        !has_choice && js::utf16_len(trimmed) >= MIN_GUESS_CHARS && trimmed != self.guessed_for
    }

    /// The text was cleared (CAP-065): answers in flight are dropped and the
    /// guess forgotten. The caller clears the user's choice too.
    pub fn clear(&mut self) {
        self.run += 1;
        self.guessed_for.clear();
        self.guessing = false;
        self.guess = None;
    }

    /// A guess is asked for (CAP-066); answer it with [`settle`](Self::settle).
    pub fn begin(&mut self) -> u64 {
        self.run += 1;
        self.guessing = true;
        self.run
    }

    /// An answer to run `run`, about `text` (untrimmed). Only the latest run
    /// counts; `None` keeps the guess on screen. Returns whether it counted.
    pub fn settle(&mut self, run: u64, answer: Option<CaptureGuess>, text: &str) -> bool {
        if run != self.run {
            return false;
        }
        self.guessing = false;
        if let Some(answer) = answer {
            self.guessed_for = js::trim(text).to_owned();
            self.guess = Some(answer);
        }
        true
    }

    /// Sending waits for Claude's guess when the user hasn't chosen and the
    /// guess on screen isn't for this text (CAP-123).
    pub fn must_wait(&self, text: &str, has_choice: bool) -> bool {
        !has_choice && self.guessed_for != js::trim(text)
    }
}

/// ↑ in the field (CAP-122): the next earlier sent text, when the field is
/// empty or already browsing; None leaves the key to the caret.
pub fn history_up(text: &str, at: i32, history_len: usize) -> Option<i32> {
    let browsing = js::trim(text).is_empty() || at >= 0;
    (browsing && history_len > 0).then(|| (at + 1).min(history_len as i32 - 1))
}

/// ↓ in the field (CAP-122): the next later one, -1 being the empty field.
pub fn history_down(at: i32) -> Option<i32> {
    (at >= 0).then_some(at - 1)
}

/// The history once `sent` went (CAP-123): it first, an earlier same one
/// gone, 50 at most.
pub fn remember_sent(history: &[String], sent: &str) -> Vec<String> {
    std::iter::once(sent.to_owned())
        .chain(history.iter().filter(|h| *h != sent).cloned())
        .take(MAX_HISTORY)
        .collect()
}

/// What tells one clipboard apart from the next, to offer it only once
/// (CAP-098, CAP-167): `t:` and the text's first 2000 UTF-16 units, or `i:`
/// and the thumbnail's last 120.
pub fn offer_key(offer: &ClipboardOffer) -> String {
    match offer {
        ClipboardOffer::Text { text } => format!("t:{}", js::slice_utf16(text, 0, 2000)),
        ClipboardOffer::Image { thumb } => {
            let len = js::utf16_len(thumb);
            format!("i:{}", js::slice_utf16(thumb, len.saturating_sub(120), len))
        }
    }
}

/// The browser tab in front comes along at the front, unless the draft
/// already has a tab (CAP-051, CAP-166).
pub fn with_tab(attachments: &[CaptureAttachment], tab: &TabAttachment) -> Vec<CaptureAttachment> {
    if attachments.iter().any(|a| matches!(a, CaptureAttachment::Tab(_))) {
        return attachments.to_vec();
    }
    std::iter::once(CaptureAttachment::Tab(tab.clone()))
        .chain(attachments.iter().cloned())
        .collect()
}

/// A transcript added to the field (CAP-103): after the text with one
/// space, or in place of a blank field.
pub fn append_transcript(text: &str, said: &str) -> String {
    if js::trim(text).is_empty() {
        said.to_owned()
    } else {
        format!("{} {said}", js::trim_end(text))
    }
}

/// A file's size on its tile (CAP-094): `B`, `KB` rounded, or `MB` to one
/// decimal as `toFixed(1)` writes it (halves up).
pub fn bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if n < KB {
        return format!("{n} B");
    }
    if n < MB {
        return format!("{} KB", number_to_string(js::round(n as f64 / KB as f64)));
    }
    // n / 2^20 is exact in binary, so toFixed's halves are exact too.
    let scaled = u128::from(n) * 10;
    let tenths = (scaled >> 20) + u128::from(scaled & (u128::from(MB) - 1) >= u128::from(MB) / 2);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

/// How long ago, on a recent (CAP-152, CAP-170): `now`, `5m`, `3h`, `2d`.
pub fn ago(now: f64, at: f64) -> String {
    let min = js::round((now - at) / 60_000.0);
    if min < 1.0 {
        return "now".into();
    }
    if min < 60.0 {
        return format!("{}m", number_to_string(min));
    }
    let h = js::round(min / 60.0);
    if h < 24.0 {
        format!("{}h", number_to_string(h))
    } else {
        format!("{}d", number_to_string(js::round(h / 24.0)))
    }
}

/// Text cut to `max` UTF-16 units with `…`, its end trimmed (CAP-141, CAP-143).
pub fn clip(text: &str, max: usize) -> String {
    if js::utf16_len(text) > max {
        format!("{}…", js::trim_end(&js::slice_utf16(text, 0, max.saturating_sub(1))))
    } else {
        text.to_owned()
    }
}

/// A recent's status line (CAP-152).
pub fn recent_status(recent: &CaptureRecent) -> &'static str {
    match (recent.kind, recent.status) {
        (CaptureKind::Note, _) => "Note",
        (CaptureKind::View, _) => "View",
        (_, Some(SessionStatus::Waiting)) => "Needs you",
        (_, Some(SessionStatus::Working)) => "Working",
        _ => "Done",
    }
}

/// The pill under three recents (CAP-153): how many more, and the titles of
/// the 4th and 5th; the faint ` · ↑ for earlier captures` follows it.
pub fn more_recents(recents: &[CaptureRecent]) -> Option<String> {
    (recents.len() > 3).then(|| {
        let titles: Vec<&str> = recents.iter().skip(3).take(2).map(|r| r.title.as_str()).collect();
        format!("{} more · {}", recents.len() - 3, titles.join(", "))
    })
}

/// Another project's line in the into menu (CAP-111): `<n> sessions`, or
/// `no sessions`, verbatim (so `1 sessions`).
pub fn sessions_count(n: usize) -> String {
    if n == 0 {
        "no sessions".into()
    } else {
        format!("{n} sessions")
    }
}

/// The host a tab tile shows (CAP-092): without a leading `www.`; the whole
/// URL when there's no host to read. Hosts are lowercased and default ports
/// dropped, as the URL parser does; international names stay as written
/// (the parser would show them in punycode).
pub fn host(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    if authority.is_empty() {
        return url.to_owned();
    }
    let scheme = scheme.to_ascii_lowercase();
    let mut host = authority.to_lowercase();
    let default_port = match scheme.as_str() {
        "http" => Some(":80"),
        "https" => Some(":443"),
        _ => None,
    };
    if let Some(port) = default_port
        && host.ends_with(port)
    {
        host.truncate(host.len() - port.len());
    }
    host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
}

#[cfg(test)]
mod tests;
