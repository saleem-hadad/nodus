//! The actions rail's logic, for nodus-ui to draw (`ActionRail.tsx`): what it
//! lists and in what order, which action is running, what the hint, the fan
//! and the toasts say, and when it slides in and out.

use std::time::Duration;

use nodus_domain::js::{slice_utf16, trim_end, utf16_len};
use nodus_domain::{ProjectAction, SessionCard, SessionStatus};

/// The rail slides in this long after the pointer reaches the edge (ACT-049).
pub const REVEAL_DELAY: Duration = Duration::from_millis(100);
/// …and out this long after it leaves (ACT-050).
pub const HIDE_DELAY: Duration = Duration::from_millis(300);
/// Space kept between the fan and the window's top and bottom (ACT-064).
pub const FAN_MARGIN: f64 = 8.0;

/// `truncate(text, max)` from the board's layout: cut to `max − 1` UTF-16
/// units, trailing spaces dropped, and `…` added.
pub fn truncate(text: &str, max: usize) -> String {
    if utf16_len(text) <= max {
        return text.to_owned();
    }
    format!("{}…", trim_end(&slice_utf16(text, 0, max.saturating_sub(1))))
}

/// ACT-047: the project's own actions, then the global ones; each non-empty
/// block is followed by a separator, and `+` comes last.
pub fn blocks<'a>(own: Option<&'a [ProjectAction]>, global: &'a [ProjectAction]) -> Vec<&'a [ProjectAction]> {
    [own.unwrap_or_default(), global]
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect()
}

/// ACT-048: an action's run: the last card that runs it and isn't archived.
pub fn run_for<'a>(cards: &'a [SessionCard], action_id: &str) -> Option<&'a SessionCard> {
    cards.iter().rev().find(|c| {
        !c.node.archived
            && c.node
                .run_of
                .as_deref()
                .is_some_and(|r| !r.is_empty() && r == action_id)
    })
}

/// ACT-063: the running actions, in rail order, with their runs.
pub fn running<'a>(
    actions: impl IntoIterator<Item = &'a ProjectAction>,
    cards: &'a [SessionCard],
) -> Vec<(&'a ProjectAction, &'a SessionCard)> {
    actions
        .into_iter()
        .filter_map(|a| run_for(cards, &a.id).map(|run| (a, run)))
        .collect()
}

/// What a press does (ACT-056).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// Close the rail at once and open the run's terminal.
    ShowRun { session_id: String },
    /// Start a run (ACT-031), leaving the rail as it is.
    Run,
}

pub fn press(cards: &[SessionCard], action: &ProjectAction) -> Press {
    match run_for(cards, &action.id) {
        Some(run) => Press::ShowRun {
            session_id: run.node.session_id.clone(),
        },
        None => Press::Run,
    }
}

/// ACT-055: the hint beside a hovered button: its name and a detail line.
/// `None` is the `+` button.
pub fn hint(action: Option<&ProjectAction>) -> (String, String) {
    let Some(action) = action else {
        return ("New Action".into(), "Describe a button, and Claude makes it".into());
    };
    let description = if action.description.is_empty() {
        "Right-click to edit"
    } else {
        &action.description
    };
    let desc = truncate(description, 80);
    let detail = if action.global == Some(true) {
        format!("Every project · {desc}")
    } else {
        desc
    };
    (action.name.clone(), detail)
}

/// ACT-057: a running tile's dot: red while the run waits on the user,
/// orange otherwise (including `done`).
pub fn dot_status(run: &SessionCard) -> SessionStatus {
    if run.status == SessionStatus::Waiting {
        SessionStatus::Waiting
    } else {
        SessionStatus::Working
    }
}

/// ACT-059: the mark on the edge while the rail is hidden: red if any run
/// waits, orange if any runs, none otherwise.
pub fn edge_mark<'a>(runs: impl IntoIterator<Item = &'a SessionCard>) -> Option<SessionStatus> {
    let mut any = false;
    for run in runs {
        if run.status == SessionStatus::Waiting {
            return Some(SessionStatus::Waiting);
        }
        any = true;
    }
    any.then_some(SessionStatus::Working)
}

/// ACT-060: the context menu's first item.
pub fn menu_run_label(running: bool) -> &'static str {
    if running { "Show Run" } else { "Run" }
}

/// ACT-062: the delete confirmation's title and message; the button says `Delete`.
pub fn delete_confirm(action: &ProjectAction) -> (String, &'static str) {
    let message = if action.global == Some(true) {
        "It’s a global button, so it goes from every project. A run already going keeps going."
    } else {
        "The button and its prompt are deleted. A run already going keeps going."
    };
    (format!("Delete “{}”?", action.name), message)
}

/// ACT-064: the fan's top, level with the first running button (`anchor_top`
/// − 4) and kept inside the window.
pub fn fan_top(anchor_top: f64, fan_height: f64, window_height: f64) -> f64 {
    let wanted = anchor_top - 4.0;
    let max = window_height - fan_height - FAN_MARGIN;
    FAN_MARGIN.max(wanted.min(max))
}

/// ACT-065: a fan card's step line.
pub fn fan_step(run: &SessionCard) -> String {
    if run.status == SessionStatus::Waiting {
        return "It needs you. Open it to answer".into();
    }
    match run.meta.as_ref().and_then(|m| m.last_assistant_text.as_deref()) {
        Some(text) if !text.is_empty() => truncate(text, 140),
        _ => "Starting…".into(),
    }
}

/// ACT-065: a fan card's status label.
pub fn fan_status(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Working => "Working",
        SessionStatus::Waiting => "Needs you",
        SessionStatus::Done => "Finishing",
    }
}

/// ACT-066: a fan card's tooltip.
pub fn fan_title(action: &ProjectAction) -> String {
    format!("Open {}", action.name)
}

/// ACT-039: the board toast when a run finishes.
pub fn finished_toast(action_name: &str, message: Option<&str>) -> String {
    match message.filter(|m| !m.is_empty()) {
        Some(message) => format!("{action_name}: {}", truncate(message, 180)),
        None => format!("{action_name} finished"),
    }
}

/// ACT-031: the board toast when a run can't start; `error` is the full IPC
/// error text (ACT-083).
pub fn run_failed_toast(action_name: &str, error: &str) -> String {
    format!("Couldn't run {action_name}: {error}")
}

/// A timer the rail asks for: run [`Rail::fire`] with its token once `delay`
/// has passed. A newer request replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    pub token: u64,
    pub delay: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Open,
    Close,
}

/// The rail's open state and its one pending timer (ACT-049…ACT-052).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rail {
    open: bool,
    menu: bool,
    timer: Option<(u64, Pending)>,
    next: u64,
    /// Cleared on close (ACT-050).
    pub hint: Option<HintAt>,
}

/// A hint on show: the hovered action (`None` for `+`) and the button's
/// vertical centre in window coordinates (ACT-054).
#[derive(Debug, Clone, PartialEq)]
pub struct HintAt {
    pub action_id: Option<String>,
    pub top: f64,
}

impl Eq for HintAt {}

impl Rail {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// ACT-052: shown while open or while its context menu is up; otherwise
    /// it's inert.
    pub fn shown(&self) -> bool {
        self.open || self.menu
    }

    pub fn set_menu(&mut self, up: bool) {
        self.menu = up;
    }

    fn schedule(&mut self, pending: Pending, delay: Duration) -> Timer {
        self.next += 1;
        self.timer = Some((self.next, pending));
        Timer {
            token: self.next,
            delay,
        }
    }

    /// ACT-051: the pointer is on the rail or the fan: no pending change.
    pub fn keep_open(&mut self) {
        self.timer = None;
    }

    /// ACT-049: the pointer enters or moves over the edge strip.
    pub fn reveal(&mut self, buttons_held: bool) -> Option<Timer> {
        if self.open {
            self.keep_open();
            return None;
        }
        // Not while dragging a card or panning the board into the edge.
        if buttons_held || self.timer.is_some() {
            return None;
        }
        Some(self.schedule(Pending::Open, REVEAL_DELAY))
    }

    /// ACT-050: the pointer leaves the edge strip, the rail or the fan.
    pub fn hide(&mut self) -> Timer {
        let delay = if self.open { HIDE_DELAY } else { Duration::ZERO };
        self.schedule(Pending::Close, delay)
    }

    /// ACT-056, ACT-061, ACT-066: closes at once.
    pub fn close(&mut self) {
        self.keep_open();
        self.open = false;
        self.hint = None;
    }

    /// A timer went off; stale ones do nothing.
    pub fn fire(&mut self, token: u64) {
        match self.timer {
            Some((current, pending)) if current == token => {
                self.timer = None;
                match pending {
                    Pending::Open => self.open = true,
                    Pending::Close => {
                        self.open = false;
                        self.hint = None;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNode, SessionMeta};
    use serde_json::{Map, json};

    fn action(id: &str, global: bool, description: &str) -> ProjectAction {
        ProjectAction {
            id: id.into(),
            name: format!("Action {id}"),
            icon: "zap".into(),
            description: description.into(),
            prompt: "p".into(),
            global: global.then_some(true),
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    fn card(id: &str, run_of: Option<&str>, archived: bool, status: SessionStatus) -> SessionCard {
        let node: BoardNode = serde_json::from_value(json!({
            "sessionId": id, "x": 0, "y": 0, "archived": archived, "createdAt": 0, "runOf": run_of
        }))
        .unwrap();
        SessionCard {
            node,
            meta: None,
            status,
            live: true,
            agents: None,
            agent_count: None,
            usage: None,
        }
    }

    #[test]
    fn lists_own_actions_then_global_ones() {
        let own = [action("a", false, "")];
        let global = [action("g", true, "")];
        let b = blocks(Some(&own), &global);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0][0].id, "a");
        assert_eq!(blocks(None, &global).len(), 1);
        assert!(blocks(Some(&[]), &[]).is_empty());
    }

    #[test]
    fn finds_each_actions_run() {
        use SessionStatus::*;
        let cards = [
            card("s1", Some("a"), false, Working),
            card("s2", Some("a"), false, Waiting),
            card("s3", Some("b"), true, Working),
            card("s4", None, false, Working),
        ];
        assert_eq!(run_for(&cards, "a").unwrap().node.session_id, "s2");
        assert!(run_for(&cards, "b").is_none());
        let a = action("a", false, "");
        assert_eq!(
            press(&cards, &a),
            Press::ShowRun {
                session_id: "s2".into()
            }
        );
        assert_eq!(press(&cards, &action("b", false, "")), Press::Run);
        let actions = [action("b", false, ""), a.clone()];
        let runs = running(&actions, &cards);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0.id, "a");
        assert_eq!(edge_mark(runs.iter().map(|(_, c)| *c)), Some(Waiting));
        assert_eq!(edge_mark([&cards[0]]), Some(Working));
        assert_eq!(edge_mark([]), None);
        assert_eq!(dot_status(&card("x", None, false, Done)), Working);
        assert_eq!(menu_run_label(true), "Show Run");
        assert_eq!(menu_run_label(false), "Run");
    }

    #[test]
    fn says_what_a_button_does() {
        assert_eq!(
            hint(None),
            ("New Action".into(), "Describe a button, and Claude makes it".into())
        );
        assert_eq!(hint(Some(&action("a", false, ""))).1, "Right-click to edit");
        assert_eq!(hint(Some(&action("g", true, "Tidy"))).1, "Every project · Tidy");
        let long = hint(Some(&action("a", false, &"word ".repeat(30)))).1;
        assert_eq!(utf16_len(&long), 80);
        assert!(long.ends_with("word…"));
        assert_eq!(
            delete_confirm(&action("g", true, "")),
            (
                "Delete “Action g”?".into(),
                "It’s a global button, so it goes from every project. A run already going keeps going."
            )
        );
        assert_eq!(
            delete_confirm(&action("a", false, "")).1,
            "The button and its prompt are deleted. A run already going keeps going."
        );
    }

    #[test]
    fn deals_the_fan_beside_the_rail() {
        assert_eq!(fan_top(200.0, 100.0, 800.0), 196.0);
        assert_eq!(fan_top(2.0, 100.0, 800.0), 8.0);
        assert_eq!(fan_top(780.0, 100.0, 800.0), 692.0);
        // A fan taller than the window starts at the margin.
        assert_eq!(fan_top(300.0, 900.0, 800.0), 8.0);
        let mut run = card("s", Some("a"), false, SessionStatus::Working);
        assert_eq!(fan_step(&run), "Starting…");
        run.meta = Some(SessionMeta {
            session_id: "s".into(),
            title: "t".into(),
            first_prompt: None,
            last_prompt: None,
            last_assistant_text: Some("x".repeat(200)),
            git_branch: None,
            message_count: 1,
            created_at: 0.0,
            updated_at: 0.0,
            transcript_path: String::new(),
        });
        assert_eq!(fan_step(&run), format!("{}…", "x".repeat(139)));
        run.status = SessionStatus::Waiting;
        assert_eq!(fan_step(&run), "It needs you. Open it to answer");
        assert_eq!(fan_status(SessionStatus::Done), "Finishing");
        assert_eq!(fan_status(SessionStatus::Waiting), "Needs you");
        assert_eq!(fan_title(&action("a", false, "")), "Open Action a");
    }

    #[test]
    fn toasts_runs() {
        assert_eq!(
            finished_toast("Tidy", Some("Archived 3 sessions.")),
            "Tidy: Archived 3 sessions."
        );
        assert_eq!(finished_toast("Tidy", None), "Tidy finished");
        assert_eq!(finished_toast("Tidy", Some("")), "Tidy finished");
        assert_eq!(utf16_len(&finished_toast("T", Some(&"y".repeat(500)))), 3 + 180);
        assert_eq!(
            run_failed_toast(
                "Tidy",
                "Error invoking remote method 'session:create': Error: That action no longer exists"
            ),
            "Couldn't run Tidy: Error invoking remote method 'session:create': Error: That action no longer exists"
        );
    }

    #[test]
    fn slides_in_and_out_on_time() {
        let mut rail = Rail::default();
        // Holding a button (dragging) doesn't reveal it.
        assert_eq!(rail.reveal(true), None);
        let t = rail.reveal(false).unwrap();
        assert_eq!(t.delay, REVEAL_DELAY);
        // A pending reveal isn't restarted by moving.
        assert_eq!(rail.reveal(false), None);
        rail.fire(t.token);
        assert!(rail.is_open() && rail.shown());
        // Leaving hides it after 300 ms, unless the pointer comes back.
        let h = rail.hide();
        assert_eq!(h.delay, HIDE_DELAY);
        rail.keep_open();
        rail.fire(h.token);
        assert!(rail.is_open());
        rail.hint = Some(HintAt {
            action_id: None,
            top: 10.0,
        });
        let h = rail.hide();
        rail.fire(h.token);
        assert!(!rail.is_open());
        assert_eq!(rail.hint, None);
        // Leaving before it opened cancels the reveal at once.
        let t = rail.reveal(false).unwrap();
        let h = rail.hide();
        assert_eq!(h.delay, Duration::ZERO);
        rail.fire(t.token);
        assert!(!rail.is_open());
        // The context menu keeps it shown.
        rail.set_menu(true);
        assert!(rail.shown());
        rail.close();
        assert!(!rail.is_open());
    }
}
