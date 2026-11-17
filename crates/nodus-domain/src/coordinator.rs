//! A board's coordinator (spec 22, beta): one Claude session the user talks
//! to, which the board's other sessions report to. Its record sits on the
//! board (`coordinators`, at most one). How a card stands with it is kept
//! with the fields this version doesn't know (`inside`, `startedBy`,
//! `noCoordinator` in `extra`), as a card's pull requests are.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::board::BoardNode;
use crate::hosted::{read, write};

string_enum! {
    /// What a member's event says (COORD-020).
    pub enum CoordinatorEventKind {
        Turn = "turn",
        Waiting = "waiting",
        Failed = "failed",
        Pr = "pr",
    }
}

/// Something a member did that the coordinator hears about (COORD-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorEvent {
    pub at: f64,
    pub session_id: String,
    pub kind: CoordinatorEventKind,
    pub text: String,
}

/// How many reports went out on a local day (COORD-021).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReportDay {
    /// Days since 1970-01-01 in local time.
    pub day: i64,
    pub count: u32,
}

/// A board's coordinator (COORD-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardCoordinator {
    /// Its session, a Claude card on the board.
    pub session_id: String,
    pub created_at: f64,
    /// Events not yet reported, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<CoordinatorEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub today: Option<ReportDay>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BoardCoordinator {
    /// At most this many events wait; the oldest go first.
    pub const MAX_PENDING: usize = 50;
    /// At most this many reports go out on a day.
    pub const MAX_A_DAY: u32 = 100;

    pub fn new(session_id: &str, now: f64) -> Self {
        Self {
            session_id: session_id.to_owned(),
            created_at: now,
            pending: Vec::new(),
            today: None,
            extra: Map::new(),
        }
    }
}

/// COORD-022: the first line of a report the board sends its coordinator.
pub const REPORT_LEAD: &str = "[Nodus] Board report";

/// COORD-034: the first words of a coordinator's message to a member.
pub const FROM_COORDINATOR_LEAD: &str = "[Nodus] From the coordinator";

/// One event in a report, as its line says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLine {
    /// `H:MM`, local time.
    pub time: String,
    pub session_id: String,
    pub title: String,
    pub text: String,
}

/// COORD-022: a report's line for an event: `- 16:02 · <id> · “<title>” <text>`,
/// all on one line.
pub fn report_line(time: &str, session_id: &str, title: &str, text: &str) -> String {
    let one = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {time} · {session_id} · “{}” {}", one(title), one(text))
}

/// The events in a report, when the message is one.
pub fn parse_report(message: &str) -> Option<Vec<ReportLine>> {
    let mut lines = message.trim().lines();
    if lines.next()?.trim() != REPORT_LEAD {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        let Some(rest) = line.trim().strip_prefix("- ") else {
            continue;
        };
        let mut parts = rest.splitn(3, " · ");
        let (Some(time), Some(id), Some(said)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (title, text) = match said.strip_prefix('“').and_then(|s| s.split_once("” ")) {
            Some((title, text)) => (title, text),
            None => ("", said),
        };
        out.push(ReportLine {
            time: time.to_owned(),
            session_id: id.to_owned(),
            title: title.to_owned(),
            text: text.to_owned(),
        });
    }
    (!out.is_empty()).then_some(out)
}

/// COORD-034: what a coordinator sends a member.
pub fn coordinator_message(title: &str, text: &str) -> String {
    format!("{FROM_COORDINATOR_LEAD} “{title}”:\n\n{}", text.trim())
}

/// The coordinator's title and words, when the message is one of its.
pub fn parse_coordinator_message(message: &str) -> Option<(String, String)> {
    let rest = message.trim().strip_prefix(FROM_COORDINATOR_LEAD)?.trim_start();
    let (head, body) = rest.split_once(":\n").unwrap_or((rest, ""));
    let title = head.trim().trim_start_matches('“').trim_end_matches('”');
    Some((title.to_owned(), body.trim().to_owned()))
}

impl BoardNode {
    /// COORD-002: inside the coordinator, kept off the board as a run is.
    pub fn inside(&self) -> bool {
        read::<bool>(&self.extra, "inside").unwrap_or(false)
    }

    pub fn set_inside(&mut self, inside: bool) {
        write(&mut self.extra, "inside", inside.then_some(&true));
    }

    /// The coordinator that started it.
    pub fn started_by(&self) -> Option<String> {
        read(&self.extra, "startedBy")
    }

    pub fn set_started_by(&mut self, by: Option<&str>) {
        write(&mut self.extra, "startedBy", by.map(str::to_owned).as_ref());
    }

    /// The user took it out of the coordinator's members.
    pub fn no_coordinator(&self) -> bool {
        read::<bool>(&self.extra, "noCoordinator").unwrap_or(false)
    }

    pub fn set_no_coordinator(&mut self, out: bool) {
        write(&mut self.extra, "noCoordinator", out.then_some(&true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_how_a_card_stands_with_its_extra_fields() {
        let mut node: BoardNode =
            serde_json::from_value(json!({ "sessionId": "s", "x": 0, "y": 0, "archived": false, "createdAt": 0 }))
                .unwrap();
        assert!(!node.inside() && node.started_by().is_none() && !node.no_coordinator());
        node.set_inside(true);
        node.set_started_by(Some("c"));
        node.set_no_coordinator(true);
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["inside"], json!(true));
        assert_eq!(v["startedBy"], json!("c"));
        assert_eq!(v["noCoordinator"], json!(true));
        node.set_inside(false);
        node.set_started_by(None);
        node.set_no_coordinator(false);
        let v = serde_json::to_value(&node).unwrap();
        assert!(v.get("inside").is_none() && v.get("startedBy").is_none() && v.get("noCoordinator").is_none());
    }

    #[test]
    fn writes_a_report_and_reads_it_back() {
        let line = report_line("16:02", "abc", "Fork “here”", "ended its turn:\n  #64 is green.");
        assert_eq!(line, "- 16:02 · abc · “Fork “here”” ended its turn: #64 is green.");
        let report = format!(
            "{REPORT_LEAD}\n{line}\n{}",
            report_line("16:03", "def", "Panes", "#65 · merged")
        );
        let lines = parse_report(&report).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].session_id, "abc");
        assert_eq!(lines[0].text, "ended its turn: #64 is green.");
        assert_eq!(lines[1].title, "Panes");
        assert_eq!(lines[1].text, "#65 · merged");
        assert!(parse_report("Board report\n- a · b · c").is_none());
        assert!(parse_report(REPORT_LEAD).is_none());
    }

    #[test]
    fn writes_the_coordinators_message_and_reads_it_back() {
        let m = coordinator_message("Release Nodus", "  Merge #64 now.\n");
        assert_eq!(m, "[Nodus] From the coordinator “Release Nodus”:\n\nMerge #64 now.");
        assert_eq!(
            parse_coordinator_message(&m),
            Some(("Release Nodus".to_owned(), "Merge #64 now.".to_owned()))
        );
        assert!(parse_coordinator_message("Merge #64 now.").is_none());
    }

    #[test]
    fn reads_a_coordinator_and_writes_it_back() {
        let raw = json!({
            "sessionId": "c", "createdAt": 1.0,
            "pending": [{ "at": 2.0, "sessionId": "m", "kind": "turn", "text": "ended its turn" }],
            "today": { "day": 20409, "count": 3 },
            "later": "kept",
        });
        let c: BoardCoordinator = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(c.pending[0].kind, CoordinatorEventKind::Turn);
        assert_eq!(serde_json::to_value(&c).unwrap(), raw);
        let bare = serde_json::to_value(BoardCoordinator::new("c", 1.0)).unwrap();
        assert_eq!(bare, json!({ "sessionId": "c", "createdAt": 1.0 }));
    }
}
