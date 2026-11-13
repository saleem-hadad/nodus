//! The pure parts of `TranscriptPreview.tsx` and `AgentViewer.tsx`: times
//! and ranges as `Intl.DateTimeFormat` shows them (TRANS-019, TRANS-020),
//! the message a search led to (TRANS-009), the collapse rule (TRANS-015),
//! the free slot beside the archive (TRANS-022), the earlier-calls toggle
//! (TRANS-018), the default agent (AGV-009), the live line (AGV-018) and the
//! stick-to-bottom rule (AGV-014).

use nodus_domain::{AgentInfo, Rect, SessionCard, SessionStatus, TranscriptMessage};

use super::actions::PreviewFind;
use crate::format::{agent_step, by_urgency};

/// A message is collapsed past this many lines…
pub const COLLAPSE_LINES: usize = 12;
/// …or characters (UTF-16 units).
pub const COLLAPSE_CHARS: usize = 1200;
/// Calls shown from a long run until the rest are asked for.
pub const CALLS_SHOWN: usize = 8;
/// Scrolled this close to the bottom, an agent's transcript follows new steps.
pub const STICK_PX: f32 = 48.0;
/// How often the agent list, and a running agent's transcript, are read again.
pub const POLL_MS: u64 = 1500;

/// A moment in local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 = January.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// `ms` since the epoch in the machine's time zone.
pub fn local(ms: f64) -> Local {
    let secs = (ms / 1000.0).floor() as libc::time_t;
    // SAFETY: `tm` is plain data, filled in by the C library from a valid time.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    // SAFETY: both pointers are valid for the call.
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }
    #[cfg(windows)]
    // SAFETY: both pointers are valid for the call.
    unsafe {
        libc::localtime_s(&mut tm, &secs);
    }
    Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `{ hour: 'numeric', minute: '2-digit' }` in en-US: `2:58 PM`.
pub fn time_of(t: Local) -> String {
    let (h, suffix) = match t.hour {
        0 => (12, "AM"),
        1..=11 => (t.hour, "AM"),
        12 => (12, "PM"),
        _ => (t.hour - 12, "PM"),
    };
    format!("{h}:{:02} {suffix}", t.minute)
}

/// `formatDay`: `Oct 18`, or `Oct 18, 2025` in another year.
pub fn day_of(t: Local, now: Local) -> String {
    if t.year == now.year {
        format!("{} {}", MONTHS[t.month as usize], t.day)
    } else {
        format!("{} {}, {}", MONTHS[t.month as usize], t.day, t.year)
    }
}

fn same_day(a: Local, b: Local) -> bool {
    (a.year, a.month, a.day) == (b.year, b.month, b.day)
}

/// `formatTime` (TRANS-019): the time today, else the day and time.
pub fn format_time(ms: f64, now_ms: f64) -> String {
    format_time_at(local(ms), local(now_ms))
}

fn format_time_at(t: Local, now: Local) -> String {
    if same_day(t, now) {
        time_of(t)
    } else {
        format!("{}, {}", day_of(t, now), time_of(t))
    }
}

/// `formatRange` (TRANS-020).
pub fn format_range(start: f64, end: f64, now_ms: f64) -> String {
    format_range_at(local(start), local(end), local(now_ms))
}

fn format_range_at(a: Local, b: Local, now: Local) -> String {
    if same_day(a, b) {
        format!("{}, {} – {}", day_of(a, now), time_of(a), time_of(b))
    } else {
        format!("{} – {}", day_of(a, now), day_of(b, now))
    }
}

/// `foundIndex` (TRANS-009): the turn the best match happened in, or the
/// first message that says the words.
pub fn found_index(messages: &[TranscriptMessage], find: &PreviewFind, terms: &[String]) -> Option<usize> {
    if let Some(at) = find.snippet.as_ref().map(|s| s.timestamp).filter(|t| *t != 0.0) {
        let turn = messages.iter().rposition(|m| m.timestamp > 0.0 && m.timestamp <= at);
        if turn.is_some() {
            return turn;
        }
    }
    messages
        .iter()
        .position(|m| !nodus_transcripts::search::match_ranges(&m.text, terms).is_empty())
}

/// TRANS-015: more than 12 lines or 1200 characters (of the trimmed text).
pub fn is_long(text: &str) -> bool {
    text.split('\n').count() > COLLAPSE_LINES || nodus_domain::js::utf16_len(text) > COLLAPSE_CHARS
}

/// TRANS-018: `Show 1 earlier call` / `Show N earlier calls`.
pub fn earlier_calls_label(hidden: usize) -> String {
    format!("Show {hidden} earlier {}", if hidden == 1 { "call" } else { "calls" })
}

/// TRANS-007: `1 message` / `N messages`.
pub fn messages_label(count: u32) -> String {
    format!("{count} {}", if count == 1 { "message" } else { "messages" })
}

const CARD_W: f64 = 288.0;
const CARD_H: f64 = 216.0;
const GAP: f64 = 40.0;

/// `stackZone(archive, STACK_ROWS * 2)`: the archive panel at its largest.
fn stack_zone_largest(archive: &Rect) -> Rect {
    // STACK_RESULTS_X + two columns + the gap + ARCHIVE_PAD; ARCHIVE_HEADER_H + two rows.
    let (pad, header, board_gap) = (28.0, 52.0, 32.0);
    let results_x = pad + CARD_W + 40.0;
    let stack_h = header + CARD_H + 38.0 + 34.0 + 22.0;
    Rect {
        x: archive.x,
        y: archive.y,
        w: results_x + 2.0 * CARD_W + board_gap + pad,
        h: stack_h.max(header + 2.0 * CARD_H + board_gap + pad),
    }
}

/// `freeSlotBesideArchive` (TRANS-022): the first spot right of the archive,
/// columns filled top to bottom, clear of every other active card.
pub fn free_slot_beside_archive(archive: &Rect, cards: &[SessionCard], session_id: &str) -> (f64, f64) {
    let zone = stack_zone_largest(archive);
    let others: Vec<&SessionCard> = cards
        .iter()
        .filter(|c| !c.node.archived && c.node.session_id != session_id)
        .collect();
    let rows = ((zone.h / (CARD_H + GAP)).floor() as usize).max(3);
    let overlaps = |x: f64, y: f64| {
        others.iter().any(|c| {
            x < c.node.x + CARD_W + GAP
                && x + CARD_W + GAP > c.node.x
                && y < c.node.y + CARD_H + GAP
                && y + CARD_H + GAP > c.node.y
        })
    };
    for i in 0..rows * 40 {
        let x = zone.x + zone.w + 80.0 + (i / rows) as f64 * (CARD_W + GAP);
        let y = zone.y + (i % rows) as f64 * (CARD_H + GAP);
        if !overlaps(x, y) {
            return (x, y);
        }
    }
    (zone.x + zone.w + 80.0, zone.y + others.len() as f64 * 24.0)
}

/// `defaultAgent` (AGV-009): the most urgent still at work, else the latest.
pub fn default_agent(agents: &[AgentInfo]) -> Option<&AgentInfo> {
    let mut busy: Vec<&AgentInfo> = agents.iter().filter(|a| a.status != SessionStatus::Done).collect();
    busy.sort_by(|a, b| by_urgency(a, b));
    busy.first().copied().or_else(|| {
        agents.iter().fold(None, |best: Option<&AgentInfo>, a| match best {
            Some(b) if a.started_at <= b.started_at => Some(b),
            _ => Some(a),
        })
    })
}

/// `liveLine` (AGV-018): what an agent at work is doing.
pub fn live_line(agent: &AgentInfo) -> String {
    let step = agent_step(agent);
    if let Some(tool) = step.tool {
        return match step.text {
            Some(text) if !text.is_empty() => format!("{tool}: {text}"),
            _ => tool,
        };
    }
    if agent.background.is_some_and(|b| b > 0) || agent.status == SessionStatus::Waiting {
        step.text.unwrap_or_else(|| "Working…".into())
    } else {
        "Working…".into()
    }
}

/// `agentElapsed`: running time, counting while at work (AGV-011).
pub fn agent_elapsed(agent: &AgentInfo, now_ms: f64) -> String {
    let end = if agent.status == SessionStatus::Done {
        agent.updated_at
    } else {
        now_ms
    };
    crate::format::format_duration((end - agent.started_at).max(0.0))
}

/// AGV-014: whether a transcript scrolled this far keeps following new steps.
pub fn sticks(scroll_height: f32, scroll_top: f32, client_height: f32) -> bool {
    scroll_height - scroll_top - client_height < STICK_PX
}

/// AGV-015: who said an agent transcript's message.
pub fn agent_author(message: &TranscriptMessage, index: usize, agent_type: &str) -> String {
    match message.role {
        nodus_domain::MessageRole::Assistant => agent_type.to_owned(),
        _ if message.notice == Some(true) => "Background task".into(),
        _ if index == 0 => "Task".into(),
        _ => "Message".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{MessageRole, SearchSnippet, SearchSnippetKind};

    fn msg(role: MessageRole, text: &str, timestamp: f64) -> TranscriptMessage {
        TranscriptMessage {
            role,
            text: text.into(),
            timestamp,
            tools: None,
            calls: None,
            notice: None,
            by: None,
        }
    }

    fn agent(id: &str, status: SessionStatus, started_at: f64) -> AgentInfo {
        AgentInfo {
            agent_id: id.into(),
            agent_type: "Explore".into(),
            description: id.into(),
            status,
            parent_agent_id: None,
            stopped: None,
            tool: None,
            activity: None,
            background: None,
            tool_count: 0,
            started_at,
            updated_at: started_at + 60_000.0,
        }
    }

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Local {
        Local {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    #[test]
    fn trans_019_trans_020_times_and_ranges() {
        let now = at(2026, 9, 20, 15, 0);
        let earlier = at(2026, 9, 18, 14, 58);
        assert_eq!(format_time_at(at(2026, 9, 20, 14, 59), now), "2:59 PM");
        assert_eq!(format_time_at(at(2026, 9, 20, 0, 5), now), "12:05 AM");
        assert_eq!(format_time_at(earlier, now), "Oct 18, 2:58 PM");
        assert_eq!(
            format_range_at(earlier, at(2026, 9, 18, 15, 0), now),
            "Oct 18, 2:58 PM – 3:00 PM"
        );
        assert_eq!(format_range_at(earlier, now, now), "Oct 18 – Oct 20");
        assert_eq!(format_time_at(at(2025, 8, 15, 15, 0), now), "Sep 15, 2025, 3:00 PM");
        // The machine's zone turns an instant into one of these.
        let t = local(1_792_508_400_000.0);
        assert!(t.year == 2026 && t.month == 9);
    }

    #[test]
    fn trans_009_found_message() {
        let messages = vec![
            msg(MessageRole::User, "add apple pay", 10.0),
            msg(MessageRole::Assistant, "done with the button", 20.0),
            msg(MessageRole::User, "and the button colour", 30.0),
        ];
        let terms = vec!["button".to_owned()];
        let mut find = PreviewFind {
            query: "button".into(),
            snippet: None,
        };
        assert_eq!(found_index(&messages, &find, &terms), Some(1));
        find.snippet = Some(SearchSnippet {
            kind: SearchSnippetKind::Reply,
            tool: None,
            file: None,
            agent: None,
            text: String::new(),
            ranges: vec![],
            timestamp: 25.0,
        });
        assert_eq!(found_index(&messages, &find, &terms), Some(1));
        find.snippet.as_mut().unwrap().timestamp = 5.0;
        assert_eq!(found_index(&messages, &find, &terms), Some(1));
        assert_eq!(found_index(&messages, &find, &["zebra".to_owned()]), None);
    }

    #[test]
    fn trans_015_collapse_rule() {
        assert!(!is_long(&"a\n".repeat(11)));
        assert!(is_long(&"a\n".repeat(12)));
        assert!(!is_long(&"x".repeat(1200)));
        assert!(is_long(&"x".repeat(1201)));
    }

    #[test]
    fn trans_018_trans_007_labels() {
        assert_eq!(earlier_calls_label(1), "Show 1 earlier call");
        assert_eq!(earlier_calls_label(3), "Show 3 earlier calls");
        assert_eq!(messages_label(1), "1 message");
        assert_eq!(messages_label(2), "2 messages");
    }

    #[test]
    fn trans_022_free_slot_beside_archive() {
        let archive = Rect {
            x: 0.0,
            y: 0.0,
            w: 344.0,
            h: 362.0,
        };
        // The zone is 992 wide: the first slot is 80 to its right.
        assert_eq!(free_slot_beside_archive(&archive, &[], "s"), (1072.0, 0.0));
    }

    #[test]
    fn agv_009_default_agent() {
        let agents = vec![
            agent("old-done", SessionStatus::Done, 1.0),
            agent("new-done", SessionStatus::Done, 3.0),
        ];
        assert_eq!(default_agent(&agents).unwrap().agent_id, "new-done");
        let agents = vec![
            agent("working", SessionStatus::Working, 1.0),
            agent("waiting", SessionStatus::Waiting, 2.0),
            agent("done", SessionStatus::Done, 3.0),
        ];
        assert_eq!(default_agent(&agents).unwrap().agent_id, "waiting");
        assert!(default_agent(&[]).is_none());
    }

    #[test]
    fn agv_018_live_line() {
        let mut a = agent("a", SessionStatus::Working, 0.0);
        assert_eq!(live_line(&a), "Working…");
        a.tool = Some("Grep".into());
        a.activity = Some("useProducts".into());
        assert_eq!(live_line(&a), "Grep: useProducts");
        a.activity = None;
        assert_eq!(live_line(&a), "Grep");
        a.status = SessionStatus::Waiting;
        assert_eq!(live_line(&a), "Waiting for you in the session’s terminal");
        a.status = SessionStatus::Working;
        a.background = Some(2);
        assert_eq!(live_line(&a), "Waiting for 2 background commands to finish");
    }

    #[test]
    fn agv_014_agv_011_stick_and_elapsed() {
        assert!(sticks(1000.0, 520.0, 450.0));
        assert!(!sticks(1000.0, 400.0, 450.0));
        let a = agent("a", SessionStatus::Done, 0.0);
        assert_eq!(agent_elapsed(&a, 999_999.0), "1m");
        let mut w = agent("w", SessionStatus::Working, 0.0);
        w.updated_at = 1.0;
        assert_eq!(agent_elapsed(&w, 42_000.0), "42s");
    }

    #[test]
    fn agv_015_authors() {
        let task = msg(MessageRole::User, "t", 0.0);
        assert_eq!(agent_author(&task, 0, "Explore"), "Task");
        assert_eq!(agent_author(&task, 2, "Explore"), "Message");
        let mut notice = task.clone();
        notice.notice = Some(true);
        assert_eq!(agent_author(&notice, 2, "Explore"), "Background task");
        assert_eq!(
            agent_author(&msg(MessageRole::Assistant, "a", 0.0), 1, "Explore"),
            "Explore"
        );
    }
}
