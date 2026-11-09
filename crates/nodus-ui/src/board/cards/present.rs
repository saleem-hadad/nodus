//! What the cards, the deck, the archive and the notes show, as pure
//! functions: previews, times, labels, the fan's pick and side, the
//! archive's sizes, notes' signatures, and the small state machines (the
//! deck's hover and pin, waking an expanded card, the wheel's owner). No
//! GPUI here, so all of it is tested headless.
//!
//! Ports the logic of SessionCardNode.tsx, ExpandedCard.tsx, AgentDeck.tsx,
//! ArchiveZoneNode.tsx, NoteNode.tsx, notes.ts and layout.ts.

use std::time::{Duration, Instant};

use nodus_domain::{AgentInfo, BoardNote, FolderColor, Project, SessionCard, SessionKind, SessionStatus, js};

use crate::format::{agent_status_label, by_urgency, card_title, format_duration, tilde_path};

// ---------------------------------------------------------------- cards

/// Whether a card runs a shell rather than Claude.
pub fn is_terminal(card: &SessionCard) -> bool {
    card.node.kind == Some(SessionKind::Terminal)
}

/// CARD-001, XCARD-001: a card is drawn expanded only while active.
pub fn is_expanded(card: &SessionCard) -> bool {
    card.node.expanded == Some(true) && !card.node.archived
}

/// The card's title (CARD-017).
pub fn title(card: &SessionCard) -> String {
    card_title(Some(card))
}

/// What the status dot (or shell glyph) says (CARD-011, CARD-016).
pub fn status_label(card: &SessionCard) -> &'static str {
    if is_terminal(card) {
        if card.live { "Running" } else { "Exited" }
    } else {
        match card.status {
            SessionStatus::Working => "Working",
            SessionStatus::Waiting => "Needs you",
            SessionStatus::Done => "Done",
        }
    }
}

/// CARD-043.
pub fn aria_label(card: &SessionCard) -> String {
    format!("{}, {}", title(card), status_label(card))
}

/// The value the rename field starts with (CARD-044): never a placeholder.
pub fn rename_initial(card: &SessionCard) -> String {
    card.node
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(|| card.meta.as_ref().map(|m| m.title.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_default()
}

/// CARD-046: the title to save, if the trimmed value changed.
pub fn rename_commit(value: &str, initial: &str) -> Option<String> {
    let v = js::trim(value);
    (v != js::trim(initial)).then(|| v.to_owned())
}

/// What kind of text the preview shows (CARD-026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Reply,
    Prompt,
    Empty,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub text: String,
    pub kind: PreviewKind,
}

/// CARD-026: a terminal's folder, Claude's last reply, the first prompt, or
/// `No messages yet`.
pub fn preview(card: &SessionCard, repo_path: Option<&str>) -> Preview {
    if is_terminal(card) {
        return Preview {
            text: repo_path.filter(|p| !p.is_empty()).map(tilde_path).unwrap_or_default(),
            kind: PreviewKind::Path,
        };
    }
    let meta = card.meta.as_ref();
    if let Some(reply) = meta.and_then(|m| m.last_assistant_text.as_deref()).filter(|t| !t.is_empty()) {
        return Preview {
            text: strip_markdown(reply),
            kind: PreviewKind::Reply,
        };
    }
    if let Some(prompt) = meta.and_then(|m| m.first_prompt.as_deref()).filter(|t| !t.is_empty()) {
        return Preview {
            text: prompt.to_owned(),
            kind: PreviewKind::Prompt,
        };
    }
    Preview {
        text: "No messages yet".into(),
        kind: PreviewKind::Empty,
    }
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// CARD-027: `stripMarkdown`, as its four regular expressions run in V8.
pub fn strip_markdown(text: &str) -> String {
    // /```[\s\S]*?```/g → ' '
    let mut s = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        let Some(close) = after.find("```") else {
            break;
        };
        s.push_str(&rest[..open]);
        s.push(' ');
        rest = &after[close + 3..];
    }
    s.push_str(rest);

    // /\[([^\]]+)\]\([^)]*\)/g → '$1'
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some(close) = chars[i + 1..].iter().position(|&c| c == ']').map(|p| p + i + 1)
            && close > i + 1
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = chars[close + 2..].iter().position(|&c| c == ')').map(|p| p + close + 2)
        {
            out.extend(&chars[i + 1..close]);
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    // /(\*\*|__|`)/g → ''
    let chars: Vec<char> = out.chars().collect();
    let mut s = String::with_capacity(out.len());
    let mut i = 0;
    while i < chars.len() {
        let pair = (chars[i], chars.get(i + 1).copied());
        if matches!(pair, ('*', Some('*')) | ('_', Some('_'))) {
            i += 2;
        } else if chars[i] == '`' {
            i += 1;
        } else {
            s.push(chars[i]);
            i += 1;
        }
    }

    // /^\s{0,3}(#{1,6}|>)\s+/gm → ''
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let at_line_start = i == 0 || is_line_terminator(chars[i - 1]);
        if at_line_start && let Some(end) = heading_marker(&chars, i) {
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The end of `\s{0,3}(#{1,6}|>)\s+` matched at `at`, backtracking as a
/// regular expression does.
fn heading_marker(chars: &[char], at: usize) -> Option<usize> {
    let lead_max = chars[at..].iter().take(3).take_while(|&&c| js::is_space(c)).count();
    for lead in (0..=lead_max).rev() {
        let q = at + lead;
        let markers: Vec<usize> = match chars.get(q) {
            Some('>') => vec![q + 1],
            Some('#') => {
                let hashes = chars[q..].iter().take(6).take_while(|&&c| c == '#').count();
                (1..=hashes).rev().map(|h| q + h).collect()
            }
            _ => vec![],
        };
        for m in markers {
            let ws = chars[m..].iter().take_while(|&&c| js::is_space(c)).count();
            if ws > 0 {
                return Some(m + ws);
            }
        }
    }
    None
}

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

/// A date's fields in the local time zone: (year, month 1–12, day).
pub fn local_date(ms: f64) -> (i64, u32, u32) {
    let secs = (ms / 1000.0).floor() as i64;
    #[cfg(unix)]
    {
        // SAFETY: localtime_r writes only into `tm`, which outlives the call.
        unsafe {
            let t: libc::time_t = secs as libc::time_t;
            let mut tm: libc::tm = std::mem::zeroed();
            if !libc::localtime_r(&t, &mut tm).is_null() {
                return (i64::from(tm.tm_year) + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32);
            }
        }
    }
    civil_from_days(secs.div_euclid(86_400))
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// CARD-041: `timeAgo`. Dates read as en-US writes them (`Oct 3`,
/// `Oct 3, 2025`).
pub fn time_ago(ts: f64, now: f64) -> String {
    time_ago_with(ts, now, local_date)
}

pub fn time_ago_with(ts: f64, now: f64, date: impl Fn(f64) -> (i64, u32, u32)) -> String {
    if ts == 0.0 || ts.is_nan() {
        return String::new();
    }
    let diff = (now - ts).max(0.0);
    if diff < MINUTE {
        return "just now".into();
    }
    if diff < HOUR {
        return format!("{}m ago", (diff / MINUTE).floor());
    }
    if diff < DAY {
        return format!("{}h ago", (diff / HOUR).floor());
    }
    if diff < 7.0 * DAY {
        return format!("{}d ago", (diff / DAY).floor());
    }
    let (y, m, d) = date(ts);
    let month = MONTHS[(m.clamp(1, 12) - 1) as usize];
    if now - ts > 300.0 * DAY {
        format!("{month} {d}, {y}")
    } else {
        format!("{month} {d}")
    }
}

/// CARD-042: the time a card shows is its transcript's, else its own.
pub fn updated_at(card: &SessionCard) -> f64 {
    card.meta.as_ref().map_or(card.node.created_at, |m| m.updated_at)
}

// ---------------------------------------------------------------- agents

/// DECK-001: the current run's agents, while the card is active.
pub fn deck_agents(card: &SessionCard) -> &[AgentInfo] {
    if card.node.archived {
        &[]
    } else {
        card.agents.as_deref().unwrap_or(&[])
    }
}

pub fn running(agents: &[AgentInfo]) -> usize {
    agents.iter().filter(|a| a.status != SessionStatus::Done).count()
}

/// CARD-036: the running badge's dot.
pub fn running_status(agents: &[AgentInfo]) -> SessionStatus {
    if agents.iter().any(|a| a.status == SessionStatus::Waiting) {
        SessionStatus::Waiting
    } else {
        SessionStatus::Working
    }
}

/// CARD-037, XCARD-009: the deck triggers' tooltip (no singular, as 0.9.4).
pub fn deck_tooltip(agents: &[AgentInfo]) -> String {
    let r = running(agents);
    if r > 0 {
        format!("{r} of {} agents running", agents.len())
    } else {
        format!("{} agents, all done", agents.len())
    }
}

/// CARD-038.
pub fn agents_viewer_tooltip(count: u32) -> String {
    format!("{count} {}: show what they did", if count == 1 { "agent" } else { "agents" })
}

/// DECK-002: the agents peeking out under the card, deepest first, each with
/// its depth (1 = nearest).
pub fn deck_layers(agents: &[AgentInfo]) -> Vec<(usize, &AgentInfo)> {
    let mut urgent: Vec<&AgentInfo> = agents.iter().collect();
    urgent.sort_by(|a, b| by_urgency(a, b));
    urgent
        .into_iter()
        .take(3)
        .enumerate()
        .map(|(i, a)| (i + 1, a))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// DECK-018: the agents the fan lists, in the run's order.
pub fn fan_pick(agents: &[AgentInfo]) -> Vec<&AgentInfo> {
    const FAN_MAX: usize = 6;
    if agents.len() <= FAN_MAX {
        return agents.iter().collect();
    }
    let mut urgent: Vec<&AgentInfo> = agents.iter().collect();
    urgent.sort_by(|a, b| by_urgency(a, b));
    let picked: std::collections::HashSet<&str> =
        urgent.iter().take(FAN_MAX - 1).map(|a| a.agent_id.as_str()).collect();
    agents.iter().filter(|a| picked.contains(a.agent_id.as_str())).collect()
}

/// DECK-017.
pub fn fan_count(agents: &[AgentInfo]) -> String {
    let r = running(agents);
    if r > 0 {
        format!("{r} of {} running", agents.len())
    } else {
        "All done".into()
    }
}

/// DECK-025: the last item of the fan.
pub fn fan_all_text(listed: usize, run: usize, total: u32) -> String {
    if run > listed {
        format!("{} more, and every agent so far", run - listed)
    } else {
        format!("All {total} {} of this session", if total == 1 { "agent" } else { "agents" })
    }
}

/// DECK-024: how long an agent has run, or ran.
pub fn agent_elapsed(agent: &AgentInfo, now: f64) -> String {
    let end = if agent.status == SessionStatus::Done {
        agent.updated_at
    } else {
        now
    };
    format_duration(end - agent.started_at)
}

/// DECK-021: line 3 of a fan row.
pub fn agent_meta(agent: &AgentInfo, now: f64) -> Vec<String> {
    let mut out = vec![
        agent_status_label(agent).to_owned(),
        agent.agent_type.clone(),
        agent_elapsed(agent, now),
    ];
    if agent.tool_count > 0 {
        out.push(format!(
            "{} {}",
            agent.tool_count,
            if agent.tool_count == 1 { "tool" } else { "tools" }
        ));
    }
    out
}

/// Where the fan deals out (DECK-015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanSide {
    Right,
    Left,
    Inside,
}

impl FanSide {
    /// DECK-026: where the cards are dealt from, sideways.
    pub fn deal_x(self) -> f32 {
        match self {
            Self::Right => -46.0,
            Self::Left | Self::Inside => 46.0,
        }
    }
}

/// DECK-015: right if the fan fits on screen there, else left, else over
/// the card. `card` and `board` are (left, right) on screen; `zoom` scales
/// the 18px gap and the 260px fan.
pub fn fan_side(card: (f32, f32), board: (f32, f32), zoom: f32) -> FanSide {
    let gap = 18.0 * zoom;
    let width = 260.0 * zoom;
    if card.1 + gap + width <= board.1 {
        FanSide::Right
    } else if card.0 - gap - width >= board.0 {
        FanSide::Left
    } else {
        FanSide::Inside
    }
}

/// The deck's open/pinned state (DECK-008…DECK-013), with the 280ms close
/// as a deadline the view waits for.
#[derive(Debug, Clone, Default)]
pub struct DeckState {
    pub hover: bool,
    pub pinned: bool,
    pub close_at: Option<Instant>,
    /// The side chosen when the fan last opened (DECK-015).
    pub side: Option<FanSide>,
}

pub const DECK_CLOSE_DELAY: Duration = Duration::from_millis(280);

impl DeckState {
    pub fn open(&self) -> bool {
        self.hover || self.pinned
    }
    /// DECK-008
    pub fn point(&mut self) {
        self.close_at = None;
        self.hover = true;
    }
    /// The pointer is back on the node or the fan.
    pub fn stay(&mut self) {
        self.close_at = None;
    }
    /// DECK-010: left the node; closes after a beat unless it comes back.
    pub fn leave(&mut self, now: Instant) {
        self.close_at = Some(now + DECK_CLOSE_DELAY);
    }
    /// DECK-011, DECK-013
    pub fn close(&mut self) {
        self.close_at = None;
        self.hover = false;
        self.pinned = false;
    }
    /// DECK-009, DECK-012
    pub fn toggle(&mut self) {
        self.close_at = None;
        self.pinned = !self.pinned;
        self.hover = true;
    }
    /// Applies a close that came due; true if it changed anything.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.close_at {
            Some(at) if now >= at => {
                self.close_at = None;
                let was = self.hover;
                self.hover = false;
                was
            }
            _ => false,
        }
    }
    /// DECK-011: a press outside the node, or Escape, closes a pinned fan.
    pub fn outside_press(&mut self) -> bool {
        if self.pinned {
            self.close();
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------- expanded cards

/// What an expanded card's terminal is doing (`useAttachedTerminal`'s phase).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Loading,
    Ready,
    Error(Option<String>),
}

/// An expanded card's start state (XCARD-019…XCARD-026).
#[derive(Debug, Clone)]
pub struct ExpandedStart {
    pub started: bool,
    pub woke_at: Option<Instant>,
    /// Whether the session was live when last woken, for the loading label.
    pub started_live: bool,
    pub wakes: u64,
}

/// XCARD-026
pub const FOCUS_WINDOW: Duration = Duration::from_millis(1500);

/// What a wake asks of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeEffect {
    /// Became started: attaches, starting or resuming the process.
    Start,
    /// Already started, but the terminal needs opening again.
    Reattach,
    Nothing,
}

impl ExpandedStart {
    /// XCARD-019: started on first render iff live, already open, or a wake
    /// was asked for before the card rendered.
    pub fn new(live: bool, needs_open: bool, pending_wake: bool, now: Instant) -> Self {
        Self {
            started: live || !needs_open || pending_wake,
            woke_at: pending_wake.then_some(now),
            started_live: live,
            wakes: 0,
        }
    }

    /// XCARD-020: once live, started for good.
    pub fn saw_live(&mut self, live: bool) {
        if live {
            self.started = true;
        }
    }

    /// XCARD-025
    pub fn wake(&mut self, live: bool, needs_open: bool, phase: &Phase, now: Instant) -> WakeEffect {
        self.woke_at = Some(now);
        self.started_live = live;
        self.wakes += 1;
        if !self.started {
            self.started = true;
            WakeEffect::Start
        } else if needs_open && *phase != Phase::Loading {
            WakeEffect::Reattach
        } else {
            WakeEffect::Nothing
        }
    }

    /// XCARD-021
    pub fn enabled(&self, in_modal: bool, project_loaded: bool) -> bool {
        self.started && !in_modal && project_loaded
    }

    /// XCARD-026: focus only right after a wake.
    pub fn should_focus(&self, now: Instant) -> bool {
        self.woke_at.is_some_and(|at| now.duration_since(at) < FOCUS_WINDOW)
    }

    /// XCARD-011: Resume / Restart shows.
    pub fn resumable(&self, live: bool, phase: &Phase) -> bool {
        self.started && !live && *phase != Phase::Loading
    }

    /// XCARD-023: the loading pill's words, fixed at wake time.
    pub fn loading_label(&self, terminal: bool) -> &'static str {
        if terminal {
            "Starting shell…"
        } else if self.started_live {
            "Starting Claude…"
        } else {
            "Resuming session…"
        }
    }
}

/// Who a scroll gesture belongs to (XCARD-027, XCARD-028).
#[derive(Debug, Clone, Default)]
pub struct WheelArbiter {
    last: Option<Instant>,
    /// The terminal the gesture began over, if it did.
    owner: Option<String>,
}

pub const GESTURE_GAP: Duration = Duration::from_millis(250);

impl WheelArbiter {
    /// Records a wheel event at `now`, over the enabled card terminal
    /// `over` (if any), and says whether that terminal scrolls with it (the
    /// board doesn't) — false: the board pans.
    pub fn wheel(&mut self, now: Instant, over: Option<&str>, ctrl: bool) -> bool {
        let new_gesture = self.last.is_none_or(|t| now.duration_since(t) > GESTURE_GAP);
        self.last = Some(now);
        if new_gesture {
            self.owner = if ctrl { None } else { over.map(str::to_owned) };
        }
        if ctrl {
            return false;
        }
        matches!((&self.owner, over), (Some(o), Some(t)) if o == t)
    }
}

// ---------------------------------------------------------------- archive

pub const ARCHIVE_HEADER_H: f32 = 52.0;
pub const ARCHIVE_PAD: f32 = 28.0;
pub const CARD_W: f32 = 288.0;
pub const CARD_H: f32 = 216.0;
pub const GAP: f32 = 32.0;
pub const STACK_W: f32 = 344.0;
pub const STACK_H: f32 = 362.0;
pub const STACK_SEARCH_Y: f32 = 306.0;
pub const STACK_SEARCH_H: f32 = 34.0;
pub const STACK_RESULTS_X: f32 = ARCHIVE_PAD + CARD_W + 40.0;
pub const STACK_BAND_H: f32 = 2.0 * CARD_H + GAP;

/// ARCH-008, NOTEUI-041: the label stays legible zoomed out.
pub fn label_scale(zoom: f32) -> f32 {
    (1.0 / zoom).clamp(1.0, 2.2)
}

/// ARCH-001: the frame's size, folded (`None`) or open with `results`.
pub fn archive_size(results: Option<usize>) -> (f32, f32) {
    let Some(r) = results else {
        return (STACK_W, STACK_H);
    };
    let cols = if r > 1 { 2.0 } else { 1.0 };
    let rows = (r.div_ceil(2)).clamp(1, 2) as f32;
    (
        STACK_RESULTS_X + cols * CARD_W + (cols - 1.0) * GAP + ARCHIVE_PAD,
        STACK_H.max(ARCHIVE_HEADER_H + rows * CARD_H + (rows - 1.0) * GAP + ARCHIVE_PAD),
    )
}

/// How far the open panel's results scroll (`stackScrollMax`).
pub fn stack_scroll_max(results: usize) -> f32 {
    (results.div_ceil(2) as f32 - 2.0).max(0.0) * (CARD_H + GAP)
}

/// ARCH-036: the scrollbar's thumb height and offset.
pub fn scrollbar(scroll: f32, max: f32) -> (f32, f32) {
    let thumb = 32f32.max(STACK_BAND_H * STACK_BAND_H / (STACK_BAND_H + max));
    let offset = if max > 0.0 {
        scroll / max * (STACK_BAND_H - thumb)
    } else {
        0.0
    };
    (thumb, offset)
}

/// ARCH-019: the search field shows.
pub fn search_visible(count: usize, hover: bool, open: bool, query: &str) -> bool {
    count > 0 && (hover || open || !query.is_empty())
}

/// ARCH-033: every archived card is out among the results.
pub fn dealt_placeholder(open: bool, count: usize, results: usize) -> bool {
    open && count > 0 && results == count
}

pub fn dealt_title(count: usize) -> String {
    if count == 1 {
        "1 archived session".into()
    } else {
        format!("{count} archived sessions")
    }
}

pub fn dealt_hint(scroll_max: f32) -> &'static str {
    if scroll_max > 0.0 {
        "Scroll the list, or search to filter"
    } else {
        "Search to filter"
    }
}

/// ARCH-035
pub fn no_match_text(query: &str) -> String {
    format!("No archived sessions match “{}”", js::trim(query))
}

/// What a key in the search field does (ARCH-023…ARCH-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKey {
    Close,
    Submit,
    Open,
    Nothing,
}

pub fn search_key(key: &str, open: bool) -> SearchKey {
    match key {
        "escape" => SearchKey::Close,
        "enter" if open => SearchKey::Submit,
        "enter" => SearchKey::Open,
        "down" if !open => SearchKey::Open,
        _ => SearchKey::Nothing,
    }
}

/// How an archived card looks in the stack or among the results
/// (ARCH-013…ARCH-017), as the canvas computes it (BOARD-015).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StackLook {
    /// 0 = the top card; 1, 2 peek out; 3 is sinking away.
    pub depth: u8,
    /// A result dealt out of the stack.
    pub result: bool,
    /// The stack lifts (ARCH-016): pointer over it, or a card over it.
    pub lifted: bool,
    /// A card flying up out of the stack: (dx, dy) from its slot to the
    /// stack top, and its stagger slot (ARCH-032).
    pub rise: Option<(f32, f32, usize)>,
}

/// The card's transform in the stack: (translate y, scale, brightness,
/// opacity, contents shown).
pub fn stack_visual(look: &StackLook) -> (f32, f32, f32, f32, bool) {
    if look.result {
        return (0.0, 1.0, 1.0, 1.0, true);
    }
    match (look.depth, look.lifted) {
        (0, false) => (0.0, 1.0, 1.0, 1.0, true),
        (0, true) => (-3.0, 1.0, 1.0, 1.0, true),
        (1, false) => (10.0, 0.94, 0.955, 1.0, false),
        (1, true) => (14.0, 0.94, 0.955, 1.0, false),
        (2, false) => (20.0, 0.88, 0.91, 1.0, false),
        (2, true) => (28.0, 0.88, 0.91, 1.0, false),
        _ => (20.0, 0.84, 1.0, 0.0, false),
    }
}

// ---------------------------------------------------------------- notes

pub const NOTE_W: f32 = 216.0;
pub const NOTE_H: f32 = 216.0;
pub const NOTE_MIN: f32 = 120.0;
pub const EXPANDED_MIN_W: f32 = 360.0;
pub const EXPANDED_MIN_H: f32 = 240.0;
/// NOTEUI-038
pub const LANDING_MS: f64 = 4000.0;

/// NOTEUI-001
pub fn note_size(note: &BoardNote) -> (f32, f32) {
    (
        note.w.map_or(NOTE_W, |w| w as f32),
        note.h.map_or(NOTE_H, |h| h as f32),
    )
}

/// NOTEUI-032: notes on the board, and those stuck on cards, by session.
pub fn split_notes<'a>(
    notes: &'a [BoardNote],
    cards: &[SessionCard],
) -> (Vec<&'a BoardNote>, indexmap::IndexMap<String, Vec<&'a BoardNote>>) {
    let on_board: std::collections::HashSet<&str> = cards.iter().map(|c| c.node.session_id.as_str()).collect();
    let mut free = Vec::new();
    let mut stuck: indexmap::IndexMap<String, Vec<&BoardNote>> = indexmap::IndexMap::new();
    for note in notes {
        match note.session_id.as_deref() {
            Some(s) if on_board.contains(s) => stuck.entry(s.to_owned()).or_default().push(note),
            _ => free.push(note),
        }
    }
    (free, stuck)
}

/// NOTEUI-038: a sticker lands on a card just started from its note.
pub fn sticker_lands(card_created_at: f64, now: f64) -> bool {
    now - card_created_at < LANDING_MS
}

/// What a note's signature says, its tooltip, and what a click opens
/// (NOTEUI-018, NOTEUI-019).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub text: String,
    pub tooltip: String,
    pub target: SignatureTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureTarget {
    /// The writer's card is on this board.
    Here { session_id: String },
    /// It wrote from another project's board.
    Away { project_id: String, session_id: Option<String> },
}

pub fn signature(note: &BoardNote, cards: &[SessionCard], projects: &[Project]) -> Option<Signature> {
    let from = note.from.as_deref();
    if let Some(card) = from.and_then(|f| cards.iter().find(|c| c.node.session_id == f)) {
        let t = title(card);
        return Some(Signature {
            text: format!("— {t}"),
            tooltip: format!("Written by Claude in “{t}”. Click to open that session."),
            target: SignatureTarget::Here {
                session_id: card.node.session_id.clone(),
            },
        });
    }
    let away = note
        .from_project
        .as_deref()
        .and_then(|p| projects.iter().find(|x| x.id == p))?;
    let from_title = note.from_title.clone().unwrap_or_else(|| "undefined".into());
    Some(Signature {
        text: format!("— {from_title} · {}", away.name),
        tooltip: format!(
            "Written by Claude in “{from_title}”, on the {} board. Click to open that session.",
            away.name
        ),
        target: SignatureTarget::Away {
            project_id: away.id.clone(),
            session_id: note.from.clone(),
        },
    })
}

/// NOTEUI-022: a note with writing on it offers ▶.
pub fn can_start(note: &BoardNote, draft: bool, editing: bool) -> bool {
    !draft && !editing && !js::trim(&note.text).is_empty()
}

/// NOTEUI-031
pub const NOTE_COLOR: FolderColor = FolderColor::Yellow;

/// Resizing from an edge or corner (XCARD-029/030, NOTEUI-025/026), as
/// React Flow's NodeResizer moves the box: the far edges stay put and the
/// size never goes below `min`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

pub fn resize(rect: (f32, f32, f32, f32), edges: ResizeEdges, dx: f32, dy: f32, min: (f32, f32)) -> (f32, f32, f32, f32) {
    let (mut x, mut y, mut w, mut h) = rect;
    if edges.right {
        w = (rect.2 + dx).max(min.0);
    }
    if edges.left {
        w = (rect.2 - dx).max(min.0);
        x = rect.0 + rect.2 - w;
    }
    if edges.bottom {
        h = (rect.3 + dy).max(min.1);
    }
    if edges.top {
        h = (rect.3 - dy).max(min.1);
        y = rect.1 + rect.3 - h;
    }
    (x, y, w, h)
}

/// The rect saved when a resize ends: rounded as `Math.round` rounds.
pub fn rounded(rect: (f32, f32, f32, f32)) -> (f64, f64, f64, f64) {
    let r = |v: f32| js::round(f64::from(v));
    (r(rect.0), r(rect.1), r(rect.2), r(rect.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(json: serde_json::Value) -> SessionCard {
        let mut base = serde_json::json!({
            "sessionId": "s", "x": 0, "y": 0, "archived": false, "createdAt": 1000,
            "status": "done", "live": false
        });
        if let (Some(b), Some(extra)) = (base.as_object_mut(), json.as_object()) {
            for (k, v) in extra {
                b.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value(base).unwrap()
    }

    fn meta(json: serde_json::Value) -> serde_json::Value {
        let mut base = serde_json::json!({
            "sessionId": "s", "title": "", "messageCount": 0, "createdAt": 0, "updatedAt": 0,
            "transcriptPath": "/t"
        });
        if let (Some(b), Some(extra)) = (base.as_object_mut(), json.as_object()) {
            for (k, v) in extra {
                b.insert(k.clone(), v.clone());
            }
        }
        base
    }

    fn agent(id: &str, status: &str, started: f64) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "agentId": id, "agentType": "Explore", "description": id, "status": status,
            "toolCount": 0, "startedAt": started, "updatedAt": started + 5000.0
        }))
        .unwrap()
    }

    #[test]
    fn strips_markdown_from_previews() {
        // CARD-027
        assert_eq!(strip_markdown("a ```code\nx``` b"), "a   b");
        assert_eq!(strip_markdown("see [the docs](http://x) now"), "see the docs now");
        assert_eq!(strip_markdown("**Done.** `x` __y__"), "Done. x y");
        assert_eq!(strip_markdown("## Plan\n\n1. Reuse"), "Plan\n\n1. Reuse");
        assert_eq!(strip_markdown("   > quoted"), "quoted");
        assert_eq!(strip_markdown("    # four spaces"), "    # four spaces");
        assert_eq!(strip_markdown("####### seven"), "####### seven");
        assert_eq!(strip_markdown("a # not a heading"), "a # not a heading");
        assert_eq!(strip_markdown("[]() [a]"), "[]() [a]");
        // An unclosed fence is only backticks, which go.
        assert_eq!(strip_markdown("```unclosed"), "unclosed");
    }

    #[test]
    fn chooses_the_preview() {
        // CARD-026
        let t = card(serde_json::json!({ "kind": "terminal" }));
        assert_eq!(
            preview(&t, Some("/Users/example/repos/shop")),
            Preview {
                text: "~/repos/shop".into(),
                kind: PreviewKind::Path
            }
        );
        assert_eq!(preview(&t, None).text, "");
        let c = card(serde_json::json!({ "meta": meta(serde_json::json!({
            "lastAssistantText": "**Hi**", "firstPrompt": "Do it" })) }));
        assert_eq!(preview(&c, None).text, "Hi");
        let c = card(serde_json::json!({ "meta": meta(serde_json::json!({ "firstPrompt": "Do it" })) }));
        assert_eq!(preview(&c, None).kind, PreviewKind::Prompt);
        let c = card(serde_json::json!({}));
        assert_eq!(
            preview(&c, None),
            Preview {
                text: "No messages yet".into(),
                kind: PreviewKind::Empty
            }
        );
    }

    #[test]
    fn says_how_long_ago() {
        // CARD-041
        let now = 1_792_508_400_000.0;
        let date = |_: f64| (2025, 10, 3);
        let ago = |ms: f64| time_ago_with(now - ms, now, date);
        assert_eq!(time_ago_with(0.0, now, date), "");
        assert_eq!(ago(59_000.0), "just now");
        assert_eq!(ago(60_000.0), "1m ago");
        assert_eq!(ago(59.0 * MINUTE), "59m ago");
        assert_eq!(ago(HOUR), "1h ago");
        assert_eq!(ago(23.0 * HOUR), "23h ago");
        assert_eq!(ago(DAY), "1d ago");
        assert_eq!(ago(6.0 * DAY), "6d ago");
        assert_eq!(ago(7.0 * DAY), "Oct 3");
        assert_eq!(ago(301.0 * DAY), "Oct 3, 2025");
        assert_eq!(time_ago_with(now + 5000.0, now, date), "just now");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_746), (2026, 10, 20));
    }

    #[test]
    fn labels_cards() {
        let c = card(serde_json::json!({ "status": "waiting" }));
        assert_eq!(aria_label(&c), "New session, Needs you");
        let t = card(serde_json::json!({ "kind": "terminal", "live": true }));
        assert_eq!(aria_label(&t), "Terminal, Running");
        // CARD-044/046
        assert_eq!(rename_initial(&t), "");
        assert_eq!(rename_commit("  New  ", "New"), None);
        assert_eq!(rename_commit(" Other ", "New"), Some("Other".into()));
        assert_eq!(rename_commit("", "New"), Some(String::new()));
        assert_eq!(rename_commit("", ""), None);
    }

    #[test]
    fn picks_the_fan() {
        // DECK-018
        let few: Vec<AgentInfo> = (0..6).map(|i| agent(&format!("a{i}"), "done", i as f64)).collect();
        assert_eq!(fan_pick(&few).len(), 6);
        let mut many: Vec<AgentInfo> = (0..7).map(|i| agent(&format!("a{i}"), "done", i as f64)).collect();
        many[6].status = SessionStatus::Waiting;
        many[3].status = SessionStatus::Working;
        let ids: Vec<&str> = fan_pick(&many).iter().map(|a| a.agent_id.as_str()).collect();
        // The waiting and working agents, then the three oldest done, in run order.
        assert_eq!(ids, ["a0", "a1", "a2", "a3", "a6"]);
        assert_eq!(fan_all_text(5, 7, 9), "2 more, and every agent so far");
        assert_eq!(fan_all_text(3, 3, 1), "All 1 agent of this session");
        assert_eq!(fan_all_text(3, 3, 4), "All 4 agents of this session");
        assert_eq!(deck_tooltip(&few[..1]), "1 agents, all done");
        assert_eq!(agents_viewer_tooltip(1), "1 agent: show what they did");
    }

    #[test]
    fn layers_the_deck_by_urgency() {
        // DECK-002: deepest first, the most urgent nearest (depth 1).
        let agents = [
            agent("done", "done", 1.0),
            agent("work", "working", 2.0),
            agent("wait", "waiting", 3.0),
            agent("work2", "working", 0.5),
        ];
        let layers: Vec<(usize, &str)> = deck_layers(&agents)
            .into_iter()
            .map(|(d, a)| (d, a.agent_id.as_str()))
            .collect();
        assert_eq!(layers, [(3, "work"), (2, "work2"), (1, "wait")]);
    }

    #[test]
    fn places_the_fan() {
        // DECK-015
        assert_eq!(fan_side((100.0, 400.0), (0.0, 1000.0), 1.0), FanSide::Right);
        assert_eq!(fan_side((700.0, 900.0), (0.0, 1000.0), 1.0), FanSide::Left);
        assert_eq!(fan_side((100.0, 900.0), (0.0, 1000.0), 1.0), FanSide::Inside);
        assert_eq!(fan_side((100.0, 800.0), (0.0, 1000.0), 0.5), FanSide::Right);
    }

    #[test]
    fn runs_the_deck_state_machine() {
        let t0 = Instant::now();
        let mut d = DeckState::default();
        d.point();
        assert!(d.open());
        d.leave(t0);
        assert!(!d.tick(t0 + Duration::from_millis(279)));
        assert!(d.tick(t0 + DECK_CLOSE_DELAY));
        assert!(!d.open());
        // Coming back cancels the close.
        d.point();
        d.leave(t0);
        d.stay();
        assert!(!d.tick(t0 + Duration::from_secs(1)));
        assert!(d.open());
        // A click pins; another unpins but stays open.
        d.toggle();
        assert!(d.pinned && d.open());
        d.leave(t0);
        d.tick(t0 + Duration::from_secs(1));
        assert!(d.open(), "pinned stays");
        d.toggle();
        assert!(!d.pinned && d.open());
        // A press outside closes only a pinned fan.
        assert!(!d.outside_press());
        d.toggle();
        assert!(d.outside_press());
        assert!(!d.open());
        // Dragging closes.
        d.toggle();
        d.close();
        assert!(!d.open());
    }

    #[test]
    fn wakes_expanded_cards() {
        let t0 = Instant::now();
        assert!(ExpandedStart::new(true, true, false, t0).started);
        assert!(ExpandedStart::new(false, false, false, t0).started);
        assert!(ExpandedStart::new(false, true, true, t0).started);
        let mut s = ExpandedStart::new(false, true, false, t0);
        assert!(!s.started);
        assert!(!s.should_focus(t0), "restored expanded: never steals focus");
        s.saw_live(true);
        assert!(s.started);
        let mut s = ExpandedStart::new(false, true, false, t0);
        assert_eq!(s.wake(false, true, &Phase::Idle, t0), WakeEffect::Start);
        assert_eq!(s.loading_label(false), "Resuming session…");
        assert_eq!(s.wake(true, true, &Phase::Loading, t0), WakeEffect::Nothing);
        assert_eq!(s.loading_label(false), "Starting Claude…");
        assert_eq!(s.wake(true, true, &Phase::Ready, t0), WakeEffect::Reattach);
        assert_eq!(s.wake(true, false, &Phase::Ready, t0), WakeEffect::Nothing);
        assert!(s.should_focus(t0 + Duration::from_millis(1499)));
        assert!(!s.should_focus(t0 + FOCUS_WINDOW));
        assert!(s.resumable(false, &Phase::Ready));
        assert!(!s.resumable(false, &Phase::Loading));
        assert!(!s.enabled(true, true));
        assert!(!s.enabled(false, false));
        assert_eq!(s.loading_label(true), "Starting shell…");
    }

    #[test]
    fn hands_wheel_gestures_to_one_owner() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut w = WheelArbiter::default();
        // Starts over the terminal: it scrolls the whole gesture.
        assert!(w.wheel(ms(0), Some("a"), false));
        assert!(w.wheel(ms(100), Some("a"), false));
        // Starts on the board: keeps panning across the terminal.
        assert!(!w.wheel(ms(400), None, false));
        assert!(!w.wheel(ms(500), Some("a"), false));
        // A 250ms gap starts a new gesture.
        assert!(w.wheel(ms(800), Some("a"), false));
        // A pinch always goes to the board.
        assert!(!w.wheel(ms(850), Some("a"), true));
    }

    #[test]
    fn sizes_the_archive() {
        // ARCH-001
        assert_eq!(archive_size(None), (344.0, 362.0));
        assert_eq!(archive_size(Some(0)), (672.0, 362.0));
        assert_eq!(archive_size(Some(1)), (672.0, 362.0));
        assert_eq!(archive_size(Some(2)), (992.0, 362.0));
        assert_eq!(archive_size(Some(3)), (992.0, 544.0));
        assert_eq!(archive_size(Some(5)), (992.0, 544.0));
        assert_eq!(label_scale(0.4), 2.2);
        assert_eq!(label_scale(0.8), 1.25);
        assert_eq!(label_scale(1.6), 1.0);
        // ARCH-036
        let (thumb, offset) = scrollbar(0.0, 248.0);
        assert!((thumb - 464.0 * 464.0 / 712.0).abs() < 1e-3);
        assert_eq!(offset, 0.0);
        let (thumb, offset) = scrollbar(248.0, 248.0);
        assert!((offset - (464.0 - thumb)).abs() < 1e-3);
        assert_eq!(scrollbar(0.0, 100_000.0).0, 32.0);
        assert!(search_visible(1, true, false, ""));
        assert!(!search_visible(0, true, true, "x"));
        assert!(search_visible(3, false, false, "x"));
        assert!(dealt_placeholder(true, 3, 3));
        assert!(!dealt_placeholder(true, 3, 2));
        assert_eq!(no_match_text("  fix "), "No archived sessions match “fix”");
        assert_eq!(search_key("escape", true), SearchKey::Close);
        assert_eq!(search_key("enter", true), SearchKey::Submit);
        assert_eq!(search_key("enter", false), SearchKey::Open);
        assert_eq!(search_key("down", false), SearchKey::Open);
        assert_eq!(search_key("down", true), SearchKey::Nothing);
    }

    #[test]
    fn looks_like_a_stack() {
        let look = |depth, lifted| StackLook {
            depth,
            result: false,
            lifted,
            rise: None,
        };
        assert_eq!(stack_visual(&look(1, false)), (10.0, 0.94, 0.955, 1.0, false));
        assert_eq!(stack_visual(&look(2, true)), (28.0, 0.88, 0.91, 1.0, false));
        assert_eq!(stack_visual(&look(0, true)).0, -3.0);
        assert_eq!(stack_visual(&look(3, false)).3, 0.0);
    }

    fn note(json: serde_json::Value) -> BoardNote {
        let mut base = serde_json::json!({ "id": "n", "text": "", "color": "yellow", "x": 0, "y": 0, "createdAt": 0 });
        if let (Some(b), Some(extra)) = (base.as_object_mut(), json.as_object()) {
            for (k, v) in extra {
                b.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn splits_and_signs_notes() {
        let cards = [card(serde_json::json!({ "sessionId": "c1", "title": "Plan it" }))];
        let notes = [
            note(serde_json::json!({ "id": "a", "sessionId": "c1" })),
            note(serde_json::json!({ "id": "b", "sessionId": "gone" })),
            note(serde_json::json!({ "id": "c" })),
        ];
        let (free, stuck) = split_notes(&notes, &cards);
        assert_eq!(free.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        assert_eq!(stuck["c1"][0].id, "a");

        let here = note(serde_json::json!({ "from": "c1" }));
        let sig = signature(&here, &cards, &[]).unwrap();
        assert_eq!(sig.text, "— Plan it");
        assert_eq!(sig.tooltip, "Written by Claude in “Plan it”. Click to open that session.");
        let project: Project = serde_json::from_value(serde_json::json!({
            "id": "p2", "name": "docs-site", "repoPath": "/r", "color": "blue", "createdAt": 0
        }))
        .unwrap();
        let away = note(serde_json::json!({ "from": "x", "fromProject": "p2", "fromTitle": "Update docs" }));
        let sig = signature(&away, &cards, std::slice::from_ref(&project)).unwrap();
        assert_eq!(sig.text, "— Update docs · docs-site");
        assert_eq!(
            sig.tooltip,
            "Written by Claude in “Update docs”, on the docs-site board. Click to open that session."
        );
        assert!(signature(&note(serde_json::json!({})), &cards, &[project]).is_none());
        assert!(sticker_lands(1000.0, 4999.0));
        assert!(!sticker_lands(1000.0, 5000.0));
        assert!(can_start(&note(serde_json::json!({ "text": "x" })), false, false));
        assert!(!can_start(&note(serde_json::json!({ "text": "  " })), false, false));
        assert!(!can_start(&note(serde_json::json!({ "text": "x" })), true, false));
    }

    #[test]
    fn resizes_from_any_edge() {
        let all = |l, r, t, b| ResizeEdges {
            left: l,
            right: r,
            top: t,
            bottom: b,
        };
        let r = (100.0, 100.0, 400.0, 300.0);
        assert_eq!(resize(r, all(false, true, false, true), 20.0, -10.0, (360.0, 240.0)), (100.0, 100.0, 420.0, 290.0));
        assert_eq!(resize(r, all(true, false, true, false), 100.0, 100.0, (360.0, 240.0)), (140.0, 160.0, 360.0, 240.0));
        assert_eq!(rounded((0.5, -0.5, 1.4, 2.6)), (1.0, -0.0, 1.0, 3.0));
    }
}
