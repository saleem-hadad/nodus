//! Goals (spec 21): one session pursued over days or weeks. The board wakes
//! it for a check-in every so often, resuming the same conversation so it
//! remembers what it tried, and Claude reports back with `goal_update` and
//! `goal_finish`: where the number stands, what it learned, and when it
//! wants to look again.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Weekday};
use nodus_domain::js::{round, slice_utf16, trim};
use nodus_domain::{
    BoardGoal, CreateSessionOptions, Event, GoalEntry, GoalMetric, GoalPoint, GoalStatus, GoalStep, Rect, SessionStatus,
};
use nodus_store::BoardState;
use serde_json::{Map, Value, json};

/// A string cut to its first `max` UTF-16 units.
fn clip(s: &str, max: usize) -> String {
    slice_utf16(s, 0, max)
}

use crate::BoardError;
use crate::board::Board;
use crate::create::Launcher;
use crate::geometry::{CARD_W, GAP, NEW_CARD_H, Point, Size, card_size, dock_rect, find_free_spot, find_spot_beside};
use crate::notes::{obstacles, rightmost_card};

/// GOAL-003: the fewest minutes between check-ins the user can pick, and the default.
pub const MIN_EVERY: f64 = 30.0;
pub const DEFAULT_EVERY: f64 = 60.0;
/// GOAL-003: a goal pauses after this many check-ins.
pub const MAX_CHECK_INS: u32 = 60;
/// GOAL-020: when a goal checks in next if Claude doesn't say.
pub const DEFAULT_NEXT_MS: f64 = 24.0 * 3_600_000.0;
/// GOAL-004: what a goal keeps.
pub const JOURNAL_MAX: usize = 200;
pub const HISTORY_MAX: usize = 200;
const STEPS_MAX: usize = 20;
const TITLE_MAX: usize = 60;
const PHASE_MAX: usize = 80;
const ENTRY_MAX: usize = 500;
const MISSION_MAX: usize = 4000;

const TILE: Size = Size {
    w: BoardGoal::W,
    h: BoardGoal::H,
};
const NEW_CARD: Size = Size {
    w: CARD_W,
    h: NEW_CARD_H,
};

/// A goal's title before Claude names it: the mission's first line.
pub fn first_title(mission: &str) -> String {
    let line = trim(mission).lines().next().unwrap_or_default();
    let title = clip(trim(line), TITLE_MAX);
    if title.is_empty() { "Goal".to_owned() } else { title }
}

/// GOAL-022: what a check-in says to the goal's session.
pub fn check_in_prompt(goal: &BoardGoal, now: f64) -> String {
    let number = goal.check_ins + 1;
    let first = goal.check_ins == 0;
    let mut out = format!(
        "Check-in #{number} for the goal you are pursuing on the user's Nodus board.\n\nThe goal: {}\n",
        goal.mission
    );
    if first {
        out.push_str(
            "\nThis is the first check-in. Work out how to tell how far along it is: when there's a number to move, find where it stands now (its start) and the target; when there isn't, write down the few steps to it. Then call goal_update with a short title, the metric (or the steps), a journal line and when to check in next.\n",
        );
    } else {
        out.push_str(&format!("\nWhere it stands: {}.\n", standing(goal)));
        if !goal.journal.is_empty() {
            out.push_str("\nYour journal, oldest first:\n");
            let skip = goal.journal.len().saturating_sub(30);
            for entry in &goal.journal[skip..] {
                out.push_str(&format!("- {}: {}\n", day_label(entry.at, now), entry.text));
            }
        }
    }
    out.push_str(
        "\nDo one meaningful step this check-in, then call goal_update before you finish: where the number stands now (or the steps done), a journal line on what you learned or did, with numbers and links, and when to check in next, in words (\"in 7 days\", \"tomorrow at 9\"). After changing something, give it long enough to show in the numbers. Ask the user (AskUserQuestion) only when you aren't sure what they'd want; otherwise decide and carry on. When the goal is reached, or can't be, call goal_finish.",
    );
    out
}

/// Where a goal stands, in words, for its check-in.
fn standing(goal: &BoardGoal) -> String {
    let pct = goal.percent().map(|p| format!(", {}% of the way", round(p)));
    match &goal.metric {
        Some(m) => {
            let n = |v: Option<f64>| v.map_or_else(|| "?".to_owned(), |v| format!("{v}{}", m.unit));
            format!(
                "{} at {} (started at {}, target {}){}",
                m.name,
                n(m.value),
                n(m.start),
                n(m.target),
                pct.unwrap_or_default()
            )
        }
        None if !goal.steps.is_empty() => {
            let done = goal.steps.iter().filter(|s| s.done).count();
            format!("{done} of {} steps done{}", goal.steps.len(), pct.unwrap_or_default())
        }
        None => "not measured yet".to_owned(),
    }
}

fn local(ms: f64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms as i64).single()
}

/// `Mon 9:00`, for the journal in a check-in.
fn day_label(at: f64, _now: f64) -> String {
    local(at).map_or_else(String::new, |t| t.format("%a %-d %b %-H:%M").to_string())
}

/// When a check-in is next, said back to Claude: `Mon 23 Nov at 9:00`.
pub fn when_label(at: f64) -> String {
    local(at).map_or_else(String::new, |t| t.format("%a %-d %b at %-H:%M").to_string())
}

/// GOAL-024: when "in 7 days", "tomorrow at 9", "monday", "in 4 hours",
/// "2026-11-24 09:00" is, after `now`. None for words it can't read.
pub fn parse_after<Tz: TimeZone>(words: &str, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let text = words.trim().to_lowercase();
    let tz = now.timezone();
    // An ISO date, with or without a time.
    if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&text.replace('t', " "), "%Y-%m-%d %H:%M") {
        return tz.from_local_datetime(&t).earliest();
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
        return tz.from_local_datetime(&d.and_hms_opt(9, 0, 0)?).earliest();
    }
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .collect();
    let number = |w: &str| match w {
        "a" | "an" | "one" => Some(1.0),
        "two" => Some(2.0),
        "three" => Some(3.0),
        "half" => Some(0.5),
        _ => w.parse::<f64>().ok(),
    };
    let unit = |w: &str| -> Option<f64> {
        let w = w.trim_end_matches('s');
        Some(match w {
            "minute" | "min" | "m" => 60_000.0,
            "hour" | "hr" | "h" => 3_600_000.0,
            "day" | "d" => 86_400_000.0,
            "week" | "wk" | "w" => 7.0 * 86_400_000.0,
            _ => return None,
        })
    };
    // "in 7 days", "in an hour", "7 days", "in half an hour".
    let mut i = usize::from(words.first() == Some(&"in"));
    if let Some(n) = words.get(i).and_then(|w| number(w)) {
        i += 1;
        if words.get(i) == Some(&"an") || words.get(i) == Some(&"a") {
            i += 1;
        }
        if let Some(u) = words.get(i).and_then(|w| unit(w)) {
            return Some(now.clone() + Duration::milliseconds((n * u) as i64));
        }
    }
    // A time of day: "at 9", "9am", "17:30".
    let time = words.iter().find_map(|w| {
        let w = w.trim_start_matches("at");
        let (digits, pm) = match w.strip_suffix("pm") {
            Some(d) => (d, Some(true)),
            None => (w.strip_suffix("am").unwrap_or(w), w.ends_with("am").then_some(false)),
        };
        let (h, m) = match digits.split_once(':') {
            Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
            None => (digits.parse::<u32>().ok()?, 0),
        };
        let h = match pm {
            Some(true) if h < 12 => h + 12,
            Some(false) if h == 12 => 0,
            _ => h,
        };
        NaiveTime::from_hms_opt(h, m, 0)
    });
    let at = |date: chrono::NaiveDate| {
        tz.from_local_datetime(&date.and_time(time.unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).expect("9:00"))))
            .earliest()
    };
    let today = now.date_naive();
    if words.contains(&"tomorrow") {
        return at(today + Duration::days(1));
    }
    let weekday = words.iter().find_map(|w| match w.trim_end_matches('s') {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    });
    if let Some(day) = weekday {
        let ahead = (7 + day.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64) % 7;
        return at(today + Duration::days(if ahead == 0 { 7 } else { ahead }));
    }
    if words.contains(&"today") || time.is_some() {
        let t = at(today)?;
        return if t > *now {
            Some(t)
        } else {
            at(today + Duration::days(1))
        };
    }
    None
}

/// GOAL-024: [`parse_after`] in local time, as ms.
pub fn parse_after_ms(words: &str, now_ms: f64) -> Option<f64> {
    parse_after(words, &local(now_ms)?).map(|t| t.timestamp_millis() as f64)
}

/// GOAL-023: Claude's `goal_update`, applied. Returns what to tell it.
pub fn apply_update(goal: &mut BoardGoal, args: &Value, now: f64) -> Result<Value, String> {
    let text = |key: &str, max: usize| match args.get(key) {
        Some(Value::String(s)) => Some(clip(trim(s), max)),
        _ => None,
    };
    let number = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|v| v.is_finite());
    if let Some(title) = text("title", TITLE_MAX).filter(|t| !t.is_empty()) {
        goal.title = title;
    }
    if let Some(m) = args.get("metric").and_then(Value::as_object) {
        let prev = goal.metric.take();
        let name = m
            .get("name")
            .and_then(Value::as_str)
            .map(|n| clip(trim(n), TITLE_MAX))
            .filter(|n| !n.is_empty())
            .or_else(|| prev.as_ref().map(|p| p.name.clone()))
            .unwrap_or_else(|| "Progress".to_owned());
        goal.metric = Some(GoalMetric {
            name,
            unit: m
                .get("unit")
                .and_then(Value::as_str)
                .map(|u| clip(trim(u), 12))
                .or_else(|| prev.as_ref().map(|p| p.unit.clone()))
                .unwrap_or_default(),
            start: number(m.get("start")).or_else(|| prev.as_ref().and_then(|p| p.start)),
            target: number(m.get("target")).or_else(|| prev.as_ref().and_then(|p| p.target)),
            value: prev.as_ref().and_then(|p| p.value),
        });
    }
    if let Some(value) = number(args.get("value")) {
        let metric = goal.metric.get_or_insert_with(|| GoalMetric {
            name: "Progress".to_owned(),
            unit: String::new(),
            start: None,
            target: None,
            value: None,
        });
        metric.value = Some(value);
        // The first number measured is where it started, unless it was said.
        metric.start.get_or_insert(value);
        goal.history.push(GoalPoint { at: now, value });
        let over = goal.history.len().saturating_sub(HISTORY_MAX);
        goal.history.drain(..over);
    }
    if let Some(p) = number(args.get("progress")) {
        goal.progress = Some(p.clamp(0.0, 100.0));
    }
    if let Some(Value::Array(steps)) = args.get("steps") {
        goal.steps = steps
            .iter()
            .filter_map(|s| {
                let text = clip(trim(s.get("text")?.as_str()?), 200);
                (!text.is_empty()).then(|| GoalStep {
                    text,
                    done: s.get("done") == Some(&Value::Bool(true)),
                })
            })
            .take(STEPS_MAX)
            .collect();
    }
    if let Some(phase) = text("phase", PHASE_MAX) {
        goal.phase = phase;
    }
    if let Some(entry) = text("journal", ENTRY_MAX).filter(|e| !e.is_empty()) {
        goal.journal.push(GoalEntry { at: now, text: entry });
        let over = goal.journal.len().saturating_sub(JOURNAL_MAX);
        goal.journal.drain(..over);
    }
    let floor = now + goal.every * 60_000.0;
    let mut note = None;
    if let Some(words) = text("next_check_in", 200).filter(|w| !w.is_empty()) {
        match parse_after_ms(&words, now) {
            Some(at) => {
                if at < floor {
                    note = Some(format!(
                        "That's sooner than the user allows; it checks in at {} instead.",
                        when_label(floor)
                    ));
                }
                goal.next_at = Some(at.max(floor));
            }
            None => {
                return Err(format!(
                    "Couldn’t read when “{words}” is. Try “in 7 days” or “tomorrow at 9”."
                ));
            }
        }
    }
    let mut reply = Map::new();
    reply.insert("ok".into(), json!(true));
    if let Some(p) = goal.percent() {
        reply.insert("percent".into(), json!(round(p)));
    }
    if let (GoalStatus::Active, Some(at)) = (goal.status, goal.next_at) {
        reply.insert("next_check_in".into(), json!(when_label(at)));
    }
    if let Some(note) = note {
        reply.insert("note".into(), json!(note));
    }
    Ok(Value::Object(reply))
}

/// GOAL-025: Claude's `goal_finish`: reached, or given up.
pub fn apply_finish(goal: &mut BoardGoal, args: &Value, now: f64) -> Value {
    let reached = args.get("reached") != Some(&Value::Bool(false));
    let reason = match args.get("reason") {
        Some(Value::String(s)) => clip(trim(s), ENTRY_MAX),
        _ => String::new(),
    };
    goal.status = if reached { GoalStatus::Done } else { GoalStatus::Failed };
    goal.next_at = None;
    goal.phase = if reached {
        "Reached".to_owned()
    } else {
        "Stopped".to_owned()
    };
    if !reason.is_empty() {
        goal.journal.push(GoalEntry {
            at: now,
            text: reason.clone(),
        });
        goal.reason = Some(reason);
    }
    json!({ "ok": true, "status": goal.status.as_str() })
}

/// GOAL-011: where a new goal's tile goes: beside the rightmost card, else
/// right of the dock.
pub fn place_goal(board: &BoardState) -> Point {
    let obstacles = obstacles(board, None);
    let dock = dock_rect(board.archive);
    match rightmost_card(board) {
        Some(card) => {
            let size = card_size(Some(card));
            let anchor = Rect {
                x: card.x,
                y: card.y,
                w: size.w,
                h: size.h,
            };
            find_spot_beside(anchor, TILE, &obstacles, Some(dock), GAP)
        }
        None => find_free_spot(
            Point {
                x: dock.x + dock.w + 80.0,
                y: dock.y,
            },
            &obstacles,
            Some(dock),
            TILE,
        ),
    }
}

/// What starting a goal needs (GOAL-010…GOAL-013).
#[derive(Debug, Clone, Default)]
pub struct NewGoal {
    pub mission: String,
    /// Minutes; clamped to MIN_EVERY…a week.
    pub every: Option<f64>,
    /// Where its tile goes; beside the cards when None.
    pub at: Option<(f64, f64)>,
    /// A session to make the goal's, instead of starting one.
    pub session_id: Option<String>,
}

impl Board {
    /// GOAL-010…GOAL-013: a goal and its first check-in: its own new session
    /// in auto mode beside its tile, or a session the user picked, resumed.
    pub async fn create_goal(
        &self,
        project_id: &str,
        new: &NewGoal,
        launcher: &dyn Launcher,
    ) -> Result<BoardGoal, BoardError> {
        let mission = clip(trim(&new.mission), MISSION_MAX);
        if mission.is_empty() {
            return Err(BoardError::Invalid("Say what the goal is".into()));
        }
        let now = self.now();
        let id = uuid::Uuid::new_v4().to_string();
        let every = new
            .every
            .filter(|e| e.is_finite())
            .unwrap_or(DEFAULT_EVERY)
            .clamp(MIN_EVERY, 10_080.0)
            .round();
        let (tile, session_at) = self.read_board(project_id, |_, board| {
            let tile = match (new.at, new.session_id.as_deref().and_then(|s| board.nodes.get(s))) {
                // GOAL-011: where it was asked for, else the nearest free spot to it.
                (Some((x, y)), _) => find_free_spot(
                    Point {
                        x: round(x),
                        y: round(y),
                    },
                    &obstacles(board, None),
                    Some(dock_rect(board.archive)),
                    TILE,
                ),
                (None, Some(node)) => {
                    let size = card_size(Some(node));
                    let anchor = Rect {
                        x: node.x,
                        y: node.y,
                        w: size.w,
                        h: size.h,
                    };
                    find_spot_beside(
                        anchor,
                        TILE,
                        &obstacles(board, None),
                        Some(dock_rect(board.archive)),
                        GAP,
                    )
                }
                (None, None) => place_goal(board),
            };
            let rect = Rect {
                x: tile.x,
                y: tile.y,
                w: TILE.w,
                h: TILE.h,
            };
            let mut around = obstacles(board, None);
            around.push(rect);
            let session_at = find_spot_beside(rect, NEW_CARD, &around, Some(dock_rect(board.archive)), GAP);
            (tile, session_at)
        });
        let mut goal = BoardGoal {
            id,
            title: first_title(&mission),
            mission,
            session_id: String::new(),
            x: tile.x,
            y: tile.y,
            status: GoalStatus::Active,
            metric: None,
            progress: None,
            steps: Vec::new(),
            phase: "Getting started".to_owned(),
            next_at: Some(now + DEFAULT_NEXT_MS),
            every,
            check_ins: 0,
            max_check_ins: MAX_CHECK_INS,
            last_at: Some(now),
            history: Vec::new(),
            journal: Vec::new(),
            reason: None,
            created_at: now,
            extra: Map::new(),
        };
        let prompt = check_in_prompt(&goal, now);
        goal.check_ins = 1;
        match new.session_id.as_deref() {
            Some(session_id) => {
                goal.session_id = session_id.to_owned();
                self.save_goal(project_id, &goal);
                self.send_to_session(project_id, session_id, &prompt, launcher).await?;
            }
            None => {
                let opts = CreateSessionOptions {
                    x: round(session_at.x),
                    y: round(session_at.y),
                    name: Some(goal.title.clone()),
                    initial_prompt: Some(prompt),
                    auto_mode: Some(true),
                    ..CreateSessionOptions::default()
                };
                let card = self.create_session(project_id, &opts, None, launcher).await?;
                goal.session_id = card.node.session_id;
                self.save_goal(project_id, &goal);
            }
        }
        Ok(goal)
    }

    /// Keeps a goal as it is now, and tells the window.
    pub fn save_goal(&self, project_id: &str, goal: &BoardGoal) {
        self.store.update(|state| {
            state.board_mut(project_id).goals.insert(goal.id.clone(), goal.clone());
        });
        self.events.emit(Event::BoardChanged {
            project_id: project_id.to_owned(),
        });
    }

    /// A goal, by id.
    pub fn goal(&self, project_id: &str, id: &str) -> Option<BoardGoal> {
        self.read_board(project_id, |_, board| board.goals.get(id).cloned())
    }

    /// GOAL-023: the goal a session pursues, and its project.
    pub fn goal_of_session(&self, session_id: &str) -> Option<(String, BoardGoal)> {
        self.store.read(|state| {
            let project_id = state.project_id_for_session(session_id)?;
            let goal = state
                .board(project_id)?
                .goals
                .values()
                .find(|g| g.session_id == session_id)?
                .clone();
            Some((project_id.to_owned(), goal))
        })
    }

    /// GOAL-023, GOAL-025: a goal's session reporting back with its tools.
    pub fn goal_tool(&self, session_id: &str, finish: bool, args: &Value) -> Result<Value, String> {
        let Some((project_id, mut goal)) = self.goal_of_session(session_id) else {
            return Err("This session isn’t pursuing a goal".into());
        };
        let now = self.now();
        let named = goal.title.clone();
        let reply = if finish {
            apply_finish(&mut goal, args, now)
        } else {
            apply_update(&mut goal, args, now)?
        };
        // GOAL-023: the name Claude gives the goal names its session too.
        if goal.title != named {
            self.store.update(|state| {
                if let Some(node) = state.board_mut(&project_id).nodes.get_mut(session_id) {
                    node.title = Some(goal.title.clone());
                }
            });
        }
        self.save_goal(&project_id, &goal);
        Ok(reply)
    }

    /// GOAL-020: the goals that are due a check-in: active, their time come,
    /// their session on the board and not busy or waiting for the user.
    pub fn due_goals(&self, now: f64) -> Vec<(String, BoardGoal)> {
        self.store.read(|state| {
            let mut out = Vec::new();
            for project in state.projects.iter().filter(|p| p.archived != Some(true)) {
                let Some(board) = state.board(&project.id) else {
                    continue;
                };
                for goal in board.goals.values() {
                    let due = goal.status == GoalStatus::Active && goal.next_at.is_some_and(|at| at <= now);
                    let on_board = board.nodes.get(&goal.session_id).is_some_and(|n| !n.archived);
                    let busy = self.sessions.is_live(&goal.session_id)
                        && self.sessions.status(&goal.session_id) != SessionStatus::Done;
                    if due && on_board && !busy {
                        out.push((project.id.clone(), goal.clone()));
                    }
                }
            }
            out
        })
    }

    /// GOAL-020, GOAL-026: a check-in: its session resumed on the check-in
    /// prompt. The next one is a day off until Claude says otherwise; after
    /// the most it may make, the goal pauses instead.
    pub async fn check_in(&self, project_id: &str, goal_id: &str, launcher: &dyn Launcher) -> Result<(), BoardError> {
        let Some(mut goal) = self.goal(project_id, goal_id) else {
            return Err(BoardError::Invalid("That goal is gone".into()));
        };
        let now = self.now();
        if goal.check_ins >= goal.max_check_ins {
            goal.status = GoalStatus::Paused;
            goal.next_at = None;
            goal.reason = Some(format!("Checked in {} times; resume it to go on.", goal.check_ins));
            self.save_goal(project_id, &goal);
            return Ok(());
        }
        let prompt = check_in_prompt(&goal, now);
        goal.check_ins += 1;
        goal.last_at = Some(now);
        goal.next_at = Some(now + DEFAULT_NEXT_MS);
        self.save_goal(project_id, &goal);
        self.send_to_session(project_id, &goal.session_id, &prompt, launcher)
            .await
    }

    /// GOAL-032: the user's changes to a goal: paused or resumed, its
    /// mission, how often it may check in, where its tile is.
    pub fn change_goal(&self, project_id: &str, id: &str, patch: &Value) -> Result<BoardGoal, BoardError> {
        let Some(mut goal) = self.goal(project_id, id) else {
            return Err(BoardError::Invalid("That goal is gone".into()));
        };
        let now = self.now();
        if let Some(status) = patch.get("status").and_then(Value::as_str).and_then(GoalStatus::parse) {
            match status {
                GoalStatus::Active => {
                    goal.status = GoalStatus::Active;
                    goal.reason = None;
                    // Resumed: a fresh run of check-ins, the next one now.
                    goal.max_check_ins = goal.check_ins + MAX_CHECK_INS;
                    goal.next_at = Some(goal.next_at.unwrap_or(now).min(now));
                }
                GoalStatus::Paused => {
                    goal.status = GoalStatus::Paused;
                    goal.next_at = None;
                    goal.reason = Some("Paused by you".into());
                }
                GoalStatus::Done | GoalStatus::Failed => {
                    goal.status = status;
                    goal.next_at = None;
                    goal.phase = if status == GoalStatus::Done {
                        "Reached"
                    } else {
                        "Stopped"
                    }
                    .to_owned();
                }
            }
        }
        if let Some(Value::String(m)) = patch.get("mission") {
            let m = clip(trim(m), MISSION_MAX);
            if !m.is_empty() {
                goal.mission = m;
            }
        }
        if let Some(every) = patch.get("every").and_then(Value::as_f64).filter(|e| e.is_finite()) {
            goal.every = every.clamp(MIN_EVERY, 10_080.0).round();
        }
        if let (Some(x), Some(y)) = (
            patch.get("x").and_then(Value::as_f64),
            patch.get("y").and_then(Value::as_f64),
        ) {
            goal.x = round(x);
            goal.y = round(y);
        }
        self.save_goal(project_id, &goal);
        Ok(goal)
    }

    /// GOAL-033: a goal off the board. Its session stays, as an ordinary card.
    pub fn remove_goal(&self, project_id: &str, id: &str) {
        self.store
            .update(|state| state.board_mut(project_id).goals.shift_remove(id));
        self.events.emit(Event::BoardChanged {
            project_id: project_id.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn goal() -> BoardGoal {
        BoardGoal {
            id: "g".into(),
            title: "Goal".into(),
            mission: "Get signup conversion from 3.1% to 4%".into(),
            session_id: "s".into(),
            x: 0.0,
            y: 0.0,
            status: GoalStatus::Active,
            metric: None,
            progress: None,
            steps: Vec::new(),
            phase: String::new(),
            next_at: None,
            every: 60.0,
            check_ins: 1,
            max_check_ins: MAX_CHECK_INS,
            last_at: None,
            history: Vec::new(),
            journal: Vec::new(),
            reason: None,
            created_at: 0.0,
            extra: Map::new(),
        }
    }

    /// 2026-11-16 is a Monday.
    fn t(day: u32, h: u32, m: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(4 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, day, h, m, 0)
            .unwrap()
    }

    #[test]
    fn reads_when_to_check_in_next() {
        let now = t(16, 10, 30);
        let p = |w: &str| parse_after(w, &now);
        assert_eq!(p("in 7 days"), Some(t(23, 10, 30)));
        assert_eq!(p("in 4 hours"), Some(t(16, 14, 30)));
        assert_eq!(p("in an hour"), Some(t(16, 11, 30)));
        assert_eq!(p("2 weeks"), Some(t(30, 10, 30)));
        assert_eq!(p("in half an hour"), Some(t(16, 11, 0)));
        assert_eq!(p("tomorrow"), Some(t(17, 9, 0)));
        assert_eq!(p("tomorrow at 2pm"), Some(t(17, 14, 0)));
        assert_eq!(p("monday"), Some(t(23, 9, 0)));
        assert_eq!(p("Friday at 17:30"), Some(t(20, 17, 30)));
        assert_eq!(p("at 9"), Some(t(17, 9, 0)));
        assert_eq!(p("today at 15:00"), Some(t(16, 15, 0)));
        assert_eq!(p("2026-11-24 09:00"), Some(t(24, 9, 0)));
        assert_eq!(p("2026-11-24"), Some(t(24, 9, 0)));
        assert_eq!(p("when it feels right"), None);
    }

    #[test]
    fn works_out_how_far_along_it_is() {
        let mut g = goal();
        assert_eq!(g.percent(), None);
        g.metric = Some(GoalMetric {
            name: "Signup conversion".into(),
            unit: "%".into(),
            start: Some(3.1),
            target: Some(4.0),
            value: Some(3.7),
        });
        assert_eq!(g.percent().map(round), Some(67.0));
        g.metric.as_mut().unwrap().value = Some(2.9);
        assert_eq!(g.percent(), Some(0.0));
        g.metric.as_mut().unwrap().value = Some(4.4);
        assert_eq!(g.percent(), Some(100.0));
        let mut s = goal();
        s.steps = vec![
            GoalStep {
                text: "a".into(),
                done: true,
            },
            GoalStep {
                text: "b".into(),
                done: true,
            },
            GoalStep {
                text: "c".into(),
                done: true,
            },
            GoalStep {
                text: "d".into(),
                done: false,
            },
            GoalStep {
                text: "e".into(),
                done: false,
            },
        ];
        assert_eq!(s.percent(), Some(60.0));
        s.progress = Some(70.0);
        assert_eq!(s.percent(), Some(70.0), "what Claude says wins over the steps");
    }

    #[test]
    fn takes_claudes_updates() {
        let mut g = goal();
        let now = 1_794_000_000_000.0;
        let reply = apply_update(
            &mut g,
            &json!({
                "title": " Signup conversion ",
                "metric": { "name": "Signup conversion", "unit": "%", "target": 4.0 },
                "value": 3.1,
                "phase": "Measuring the funnel",
                "journal": "Baseline 3.1% over 7 days.",
                "next_check_in": "in 7 days"
            }),
            now,
        )
        .unwrap();
        assert_eq!(g.title, "Signup conversion");
        let m = g.metric.clone().unwrap();
        assert_eq!(
            (m.start, m.target, m.value),
            (Some(3.1), Some(4.0), Some(3.1)),
            "the first value is the start"
        );
        assert_eq!(g.history.len(), 1);
        assert_eq!(g.journal[0].text, "Baseline 3.1% over 7 days.");
        assert_eq!(g.next_at, Some(now + 7.0 * 86_400_000.0));
        assert_eq!(reply["percent"], json!(0.0));
        // Sooner than allowed: the floor, and it says so.
        let reply = apply_update(&mut g, &json!({ "value": 3.4, "next_check_in": "in 5 minutes" }), now).unwrap();
        assert_eq!(g.next_at, Some(now + 60.0 * 60_000.0));
        assert!(reply["note"].as_str().unwrap().starts_with("That's sooner"));
        assert_eq!(g.metric.as_ref().unwrap().start, Some(3.1), "the start stays");
        assert!(apply_update(&mut g, &json!({ "next_check_in": "someday" }), now).is_err());
        // Finished.
        let reply = apply_finish(
            &mut g,
            &json!({ "reached": true, "reason": "4.0% two weeks running." }),
            now,
        );
        assert_eq!(
            (g.status, g.next_at, reply["status"].clone()),
            (GoalStatus::Done, None, json!("done"))
        );
    }

    #[test]
    fn says_what_to_do_at_a_check_in() {
        let mut g = goal();
        g.check_ins = 0;
        let first = check_in_prompt(&g, 0.0);
        assert!(first.starts_with("Check-in #1 for the goal"));
        assert!(first.contains("This is the first check-in."));
        g.check_ins = 3;
        g.journal.push(GoalEntry {
            at: 0.0,
            text: "Tried one password field.".into(),
        });
        let later = check_in_prompt(&g, 0.0);
        assert!(later.starts_with("Check-in #4"));
        assert!(later.contains("Where it stands: not measured yet."));
        assert!(later.contains(": Tried one password field."));
        assert!(later.contains("Ask the user (AskUserQuestion) only when you aren't sure"));
    }
}
