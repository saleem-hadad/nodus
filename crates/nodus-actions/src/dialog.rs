//! The action dialog's logic, for nodus-ui to draw (`ActionDialog.tsx`,
//! ACT-067…ACT-080): describe, let Claude draft, review, save. The window's
//! calls (`actions:draft`, `actions:save`) are made by the UI with what this
//! hands it, and their answers handed back.

use nodus_domain::js::trim;
use nodus_domain::{ActionDraft, ActionIcon, ActionSchedule, ProjectAction};

use crate::schedule;
use serde_json::{Map, Value, json};

/// ACT-070: descriptions to start from, label → text.
pub const EXAMPLES: [(&str, &str); 5] = [
    (
        "Pull and push",
        "Pull the latest changes from the remote, then push my local commits. If the pull conflicts, resolve the conflicts so both sides’ changes are kept, finish the merge, and push. Never force-push, and stop if there are uncommitted changes that would get in the way.",
    ),
    (
        "Tidy the board",
        "Clean up the board: archive the sessions that are done and haven’t been active for a day. Leave anything that is working or waiting for me alone.",
    ),
    (
        "Run the tests",
        "Run the project’s tests. If any fail, fix them, run the tests again, and tell me what you changed.",
    ),
    (
        "Commit my changes",
        "Look at the uncommitted changes, split them into sensible commits with clear messages, and commit them. Don’t push.",
    ),
    (
        "Review the diff",
        "Review the uncommitted changes for bugs and risky edits, and tell me what you found. Don’t change any files.",
    ),
];

/// SCHED-046: an automation's examples: label, what it does, when it runs.
pub const AUTOMATION_EXAMPLES: [(&str, &str, &str); 4] = [
    (
        "Morning PR sweep",
        "Look at the open pull requests and sum up what waits on me: reviews I owe, my PRs with new comments, and anything failing CI. Don’t change anything.",
        "every weekday at 9",
    ),
    (
        "Dependency check",
        "Check the project’s dependencies for updates and security advisories, and write a short note on the board with what’s worth upgrading.",
        "mondays at 10",
    ),
    (
        "Nightly tests",
        "Run the full test suite on the main branch and tell me what failed and why. Don’t fix anything.",
        "every day at 2am",
    ),
    (
        "Tidy the board",
        "Archive the sessions that are done and haven’t been active since the morning. Leave anything working or waiting for me alone.",
        "every day at 6pm",
    ),
];

/// SCHED-046.
pub const AUTOMATION_PLACEHOLDER: &str = "For example: look at the open pull requests and sum up what waits on me.";

/// ACT-069.
pub const DESCRIPTION_PLACEHOLDER: &str =
    "For example: archive the sessions that are done, so the board only shows what’s still going on.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Describe,
    Review,
}

/// A draft to ask Claude for (`actions.draft(projectId, description, global)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRequest {
    /// Hand it back with the answer.
    pub id: i64,
    pub description: String,
    pub global: bool,
}

/// A footer button: its label and whether it can be pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub enabled: bool,
}

/// What ⌘/Ctrl+Enter does (ACT-078).
#[derive(Debug, Clone, PartialEq)]
pub enum Submit {
    Draft(DraftRequest),
    Save(Value),
    Nothing,
}

/// The dialog's state.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionDialog {
    /// The action being edited; a new one when `None`.
    pub action: Option<ProjectAction>,
    pub step: Step,
    pub description: String,
    pub name: String,
    pub icon: String,
    pub prompt: String,
    pub global: bool,
    pub drafting: bool,
    pub saving: bool,
    /// ACT-080: shown between the body and the footer until the next draft or save.
    pub error: Option<String>,
    /// SCHED-040: `On a schedule` rather than `When I press it`.
    pub scheduled: bool,
    /// SCHED-041: the When field: what the user typed, or the schedule said back.
    pub when: String,
    /// SCHED-041: what Nodus understood; the day chips and the time show it.
    pub schedule: Option<ActionSchedule>,
    /// SCHED-041: the When field says something that isn't a time.
    pub when_unclear: bool,
    /// SCHED-045: opened as an automation (`New automation…`, or a tile's
    /// `Edit automation…`): always on a schedule, never global.
    pub automation: bool,
    request: i64,
}

/// SCHED-040: the schedule a switch to `On a schedule` starts with.
pub const DEFAULT_WHEN: &str = "every weekday at 9";

/// SCHED-041: shown under the When field when it isn't a time.
pub const WHEN_UNCLEAR: &str = "Not sure when that is. Try “every weekday at 9” or “every 2 hours”.";

impl ActionDialog {
    /// ACT-067: a new action starts on describe, an existing one on review.
    pub fn new(action: Option<ProjectAction>) -> Self {
        let a = action.as_ref();
        Self {
            step: if a.is_some() { Step::Review } else { Step::Describe },
            description: a.map(|a| a.description.clone()).unwrap_or_default(),
            name: a.map(|a| a.name.clone()).unwrap_or_default(),
            icon: a.map_or_else(|| ActionIcon::DEFAULT.as_str().to_owned(), |a| a.icon.clone()),
            prompt: a.map(|a| a.prompt.clone()).unwrap_or_default(),
            global: a.and_then(|a| a.global).unwrap_or(false),
            drafting: false,
            saving: false,
            error: None,
            scheduled: a.is_some_and(|a| a.schedule.is_some()),
            when: a
                .and_then(|a| a.schedule.as_ref())
                .map(|s| {
                    if s.words.is_empty() {
                        schedule::describe(s)
                    } else {
                        s.words.clone()
                    }
                })
                .unwrap_or_default(),
            schedule: a.and_then(|a| a.schedule.clone()),
            when_unclear: false,
            automation: false,
            request: 0,
            action,
        }
    }

    /// SCHED-045: the dialog as an automation, new or one being edited. A
    /// new one starts every weekday at 9, with the When field empty.
    pub fn automation(action: Option<ProjectAction>) -> Self {
        let mut d = Self::new(action);
        d.automation = true;
        d.scheduled = true;
        d.global = false;
        if d.schedule.is_none() {
            d.schedule = schedule::parse(DEFAULT_WHEN);
        }
        d
    }

    /// ACT-068.
    pub fn title(&self) -> &'static str {
        match (self.automation, self.action.is_some()) {
            (true, true) => "Edit automation",
            (true, false) => "New automation",
            (false, true) => "Edit action",
            (false, false) => "New action",
        }
    }

    /// ACT-068.
    pub fn subtitle(&self) -> &'static str {
        match (self.automation, self.step) {
            // SCHED-045
            (true, Step::Describe) => "Say what it should do and when. Claude writes the prompt each run follows.",
            (true, Step::Review) => {
                "Each time, a session runs this prompt in auto mode, beside its tile on the board. It goes when it’s done."
            }
            (false, Step::Describe) => "Say what the button should do. Claude writes the prompt each press runs.",
            (false, Step::Review) => {
                "Each press runs a session on this prompt in auto mode, beside its button. It goes when it’s done."
            }
        }
    }

    /// SCHED-045: an automation has no `Use in every project`.
    pub fn shows_scope(&self) -> bool {
        !self.automation
    }

    /// SCHED-045: nor the `When I press it` | `On a schedule` switch: it's
    /// always on a schedule.
    pub fn shows_when_switch(&self) -> bool {
        !self.automation
    }

    /// The example chips' labels.
    pub fn chip_labels(&self) -> Vec<&'static str> {
        if self.automation {
            AUTOMATION_EXAMPLES.iter().map(|e| e.0).collect()
        } else {
            EXAMPLES.iter().map(|e| e.0).collect()
        }
    }

    /// The description field's placeholder.
    pub fn description_placeholder(&self) -> &'static str {
        if self.automation {
            AUTOMATION_PLACEHOLDER
        } else {
            DESCRIPTION_PLACEHOLDER
        }
    }

    /// ACT-071.
    pub fn scope_hint(&self) -> &'static str {
        if self.global {
            "A global button, in the actions rail of all your projects. Claude writes its prompt to work in any repository."
        } else {
            "Only this project has it."
        }
    }

    /// ACT-071: the switch is off-limits while drafting or saving.
    pub fn scope_enabled(&self) -> bool {
        !self.drafting && !self.saving
    }

    /// ACT-070: a chip is highlighted while the description is its text.
    pub fn chip_selected(&self, index: usize) -> bool {
        if self.automation {
            return AUTOMATION_EXAMPLES
                .get(index)
                .is_some_and(|(_, text, when)| self.description == *text && self.when == *when);
        }
        EXAMPLES.get(index).is_some_and(|(_, text)| self.description == *text)
    }

    /// ACT-070: a chip replaces the description; chips do nothing while drafting.
    pub fn pick_chip(&mut self, index: usize) {
        if self.drafting {
            return;
        }
        // SCHED-046: an automation's chip sets when it runs too.
        if self.automation {
            if let Some((_, text, when)) = AUTOMATION_EXAMPLES.get(index) {
                (*text).clone_into(&mut self.description);
                self.set_when(when);
            }
            return;
        }
        if let Some((_, text)) = EXAMPLES.get(index) {
            (*text).clone_into(&mut self.description);
        }
    }

    /// ACT-074: the highlighted icon; none for a name that isn't one of the 26.
    pub fn selected_icon(&self) -> Option<ActionIcon> {
        ActionIcon::parse(&self.icon)
    }

    /// ACT-072: `Draft with Claude`. Nothing without a description or while a
    /// draft is in flight.
    pub fn begin_draft(&mut self) -> Option<DraftRequest> {
        let what = trim(&self.description);
        if what.is_empty() || self.drafting || (self.automation && self.when_unclear) {
            return None;
        }
        self.request += 1;
        self.drafting = true;
        self.error = None;
        // SCHED-047: an automation's drafter is told when it runs.
        let description = match (&self.schedule, self.automation) {
            (Some(s), true) => format!("{what}\n\nIt runs on its own: {}.", schedule::describe(s)),
            _ => what.to_owned(),
        };
        Some(DraftRequest {
            id: self.request,
            description,
            global: self.global,
        })
    }

    /// ACT-072, ACT-073: the draft's answer. `Err` is the full IPC error text
    /// (ACT-083). An answer for an older request, or after closing, is dropped.
    pub fn draft_done(&mut self, id: i64, result: Result<ActionDraft, String>) {
        if id != self.request {
            return;
        }
        match result {
            Ok(draft) => {
                self.name = draft.name;
                self.icon = draft.icon;
                self.prompt = draft.prompt;
                self.step = Step::Review;
                // SCHED-008: a time in the description becomes the schedule;
                // an automation keeps the one the user set (SCHED-047).
                if let (false, false, Some(found)) = (self.global, self.automation, schedule::parse(&draft.when)) {
                    self.scheduled = true;
                    self.when = draft.when;
                    self.when_unclear = false;
                    self.schedule = Some(ActionSchedule {
                        paused: self.schedule.as_ref().and_then(|s| s.paused),
                        ..found
                    });
                }
            }
            Err(message) => self.error = Some(format!("Claude couldn’t draft it: {message}")),
        }
        self.drafting = false;
    }

    /// ACT-073: closing drops any draft still to come.
    pub fn close(&mut self) {
        self.request = -1;
    }

    /// ACT-075: `Add action` / `Save`. Nothing without a prompt or while a save
    /// is in flight; otherwise what to send to `actions:save`. `new_id` is a
    /// fresh UUID for a new action, `now` its creation time in ms.
    pub fn begin_save(&mut self, new_id: &str, now: f64) -> Option<Value> {
        if trim(&self.prompt).is_empty() || self.saving {
            return None;
        }
        self.saving = true;
        self.error = None;
        let name = Some(trim(&self.name)).filter(|n| !n.is_empty()).unwrap_or("Action");
        let mut raw = Map::new();
        raw.insert(
            "id".into(),
            json!(self.action.as_ref().map_or(new_id, |a| a.id.as_str())),
        );
        raw.insert("name".into(), json!(name));
        raw.insert("icon".into(), json!(self.icon));
        raw.insert("description".into(), json!(trim(&self.description)));
        raw.insert("prompt".into(), json!(trim(&self.prompt)));
        if self.global {
            raw.insert("global".into(), json!(true));
        }
        raw.insert(
            "createdAt".into(),
            json!(self.action.as_ref().map_or(now, |a| a.created_at)),
        );
        // SCHED-043
        if let (true, false, Some(s)) = (self.scheduled, self.global, &self.schedule) {
            raw.insert("schedule".into(), serde_json::to_value(s).unwrap_or(Value::Null));
        }
        Some(Value::Object(raw))
    }

    /// SCHED-007, SCHED-040: the global switch. A global action can't have a
    /// schedule, so turning it on turns the schedule off.
    pub fn set_global(&mut self, on: bool) {
        self.global = on;
        if on {
            self.scheduled = false;
        }
    }

    /// SCHED-040: `When I press it` or `On a schedule`. A schedule turned on
    /// with nothing typed yet starts as every weekday at 9; it turns the global
    /// switch off.
    pub fn set_scheduled(&mut self, on: bool) {
        self.scheduled = on;
        if !on {
            return;
        }
        self.global = false;
        if self.schedule.is_none() {
            self.schedule = schedule::parse(DEFAULT_WHEN);
            self.when = DEFAULT_WHEN.to_owned();
            self.when_unclear = false;
        }
    }

    /// SCHED-041: the When field changed. Words that say when replace the
    /// schedule (keeping it paused if it was); others leave it and say so.
    pub fn set_when(&mut self, text: &str) {
        text.clone_into(&mut self.when);
        match schedule::parse(text) {
            Some(found) => {
                self.schedule = Some(ActionSchedule {
                    paused: self.schedule.as_ref().and_then(|s| s.paused),
                    ..found
                });
                self.when_unclear = false;
            }
            None => self.when_unclear = !trim(text).is_empty(),
        }
    }

    /// SCHED-042: whether a day chip is on; every chip is on for every day.
    pub fn day_on(&self, day: u8) -> bool {
        self.schedule
            .as_ref()
            .is_some_and(|s| s.days.is_empty() || s.days.contains(&day))
    }

    /// SCHED-042: a day chip pressed. The last day on stays on. The When
    /// field then says the schedule back, so the two agree; the UI puts
    /// [`Self::when`] into the field when this returns true.
    pub fn toggle_day(&mut self, day: u8) -> bool {
        let Some(s) = self.schedule.as_mut() else {
            return false;
        };
        let mut days: Vec<u8> = (0..7).filter(|d| s.days.is_empty() || s.days.contains(d)).collect();
        if days.contains(&day) {
            if days.len() == 1 {
                return false;
            }
            days.retain(|d| *d != day);
        } else {
            days.push(day);
            days.sort_unstable();
        }
        s.days = if days.len() == 7 { Vec::new() } else { days };
        self.said_back();
        true
    }

    /// SCHED-042: the time field: `9`, `9:30`, `17:05`. A time makes an
    /// every-so-often schedule run once a day at it. Returns false for text that
    /// isn't a time, leaving the schedule as it was.
    pub fn set_time(&mut self, text: &str) -> bool {
        let Some(found) = schedule::parse(&format!("at {}", trim(text))).and_then(|s| s.at) else {
            return false;
        };
        let Some(s) = self.schedule.as_mut() else {
            return false;
        };
        s.at = Some(found);
        s.every = None;
        self.said_back();
        true
    }

    /// The time field's text: `9:00`, or empty for an every-so-often schedule.
    pub fn time_text(&self) -> String {
        self.schedule
            .as_ref()
            .and_then(|s| s.at.as_deref())
            .and_then(schedule::at_minutes)
            .map(schedule::time_label)
            .unwrap_or_default()
    }

    fn said_back(&mut self) {
        if let Some(s) = self.schedule.as_mut() {
            let words = schedule::describe(s);
            s.words.clone_from(&words);
            self.when = words;
            self.when_unclear = false;
        }
    }

    /// SCHED-044: the line under the When row: the schedule said back and when
    /// it next runs (`Weekdays at 9:00 · next tomorrow at 9:00`), or why it
    /// can't be read.
    pub fn when_hint(&self, now_ms: f64) -> String {
        if self.when_unclear {
            return WHEN_UNCLEAR.to_owned();
        }
        let Some(s) = &self.schedule else {
            return String::new();
        };
        let said = schedule::describe(s);
        match schedule::next_label_ms(s, now_ms) {
            Some(next) => format!("{said} · next {next}"),
            None => said,
        }
    }

    /// ACT-075: a save that failed shows its message (the IPC error text).
    /// One that succeeded closes the dialog (the UI drops it).
    pub fn save_failed(&mut self, message: String) {
        self.error = Some(message);
        self.saving = false;
    }

    /// ACT-076: the footer's left button, if any. Pressing it goes to the other step.
    pub fn back_button(&self) -> Option<Button> {
        match self.step {
            Step::Review => Some(Button {
                label: if self.action.is_some() { "Redraft…" } else { "Back" },
                enabled: !self.saving,
            }),
            Step::Describe => (!self.prompt.is_empty()).then_some(Button {
                label: "Back to the draft",
                enabled: !self.drafting,
            }),
        }
    }

    /// ACT-076: goes back to describe, or on to review.
    pub fn back(&mut self) {
        self.step = match self.step {
            Step::Review => Step::Describe,
            Step::Describe => Step::Review,
        };
    }

    /// ACT-077: the primary button.
    pub fn primary_button(&self) -> Button {
        match self.step {
            Step::Describe => Button {
                label: if self.drafting {
                    "Drafting…"
                } else {
                    "Draft with Claude"
                },
                enabled: !trim(&self.description).is_empty()
                    && !self.drafting
                    && !(self.automation && self.when_unclear),
            },
            Step::Review => Button {
                label: match (self.action.is_some(), self.automation) {
                    (true, _) => "Save",
                    (false, true) => "Add automation",
                    (false, false) => "Add action",
                },
                enabled: !trim(&self.prompt).is_empty() && !self.saving,
            },
        }
    }

    /// ACT-078: ⌘/Ctrl+Enter anywhere in the dialog.
    pub fn submit(&mut self, new_id: &str, now: f64) -> Submit {
        let next = match self.step {
            Step::Describe => self.begin_draft().map(Submit::Draft),
            Step::Review => self.begin_save(new_id, now).map(Submit::Save),
        };
        next.unwrap_or(Submit::Nothing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing() -> ProjectAction {
        crate::normalize_action(
            &json!({ "id": "a1", "name": "Tidy", "icon": "broom", "description": "d", "prompt": "Tidy.", "global": true }),
            None,
            5.0,
        )
        .unwrap()
    }

    #[test]
    fn starts_where_the_action_is() {
        let new = ActionDialog::new(None);
        assert_eq!(
            (new.step, new.icon.as_str(), new.global),
            (Step::Describe, "zap", false)
        );
        assert_eq!(new.title(), "New action");
        assert_eq!(
            new.subtitle(),
            "Say what the button should do. Claude writes the prompt each press runs."
        );
        assert_eq!(new.back_button(), None);
        let edit = ActionDialog::new(Some(existing()));
        assert_eq!((edit.step, edit.global), (Step::Review, true));
        assert_eq!(edit.title(), "Edit action");
        // A stored icon that isn't one of the 26 highlights none.
        assert_eq!(edit.icon, "zap");
        let mut odd = existing();
        odd.icon = "broom".into();
        assert_eq!(ActionDialog::new(Some(odd)).selected_icon(), None);
        assert_eq!(
            edit.scope_hint(),
            "A global button, in the actions rail of all your projects. Claude writes its prompt to work in any repository."
        );
        assert_eq!(
            edit.back_button(),
            Some(Button {
                label: "Redraft…",
                enabled: true
            })
        );
    }

    #[test]
    fn drafts_and_drops_stale_answers() {
        let mut d = ActionDialog::new(None);
        assert_eq!(d.begin_draft(), None);
        assert!(!d.primary_button().enabled);
        d.pick_chip(2);
        assert!(d.chip_selected(2) && !d.chip_selected(0));
        d.global = true;
        let req = d.begin_draft().unwrap();
        assert_eq!(req.description, EXAMPLES[2].1);
        assert!(req.global);
        assert_eq!(
            d.primary_button(),
            Button {
                label: "Drafting…",
                enabled: false
            }
        );
        assert!(!d.scope_enabled());
        // Chips and a second draft wait.
        d.pick_chip(0);
        assert!(d.chip_selected(2));
        assert_eq!(d.begin_draft(), None);
        d.draft_done(
            req.id,
            Ok(ActionDraft {
                name: "Run Tests".into(),
                icon: "test-tube".into(),
                prompt: "Run them.".into(),
                when: String::new(),
            }),
        );
        assert_eq!(
            (d.step, d.name.as_str(), d.drafting),
            (Step::Review, "Run Tests", false)
        );
        // Redraft, fail, and see why.
        d.back();
        let req = d.begin_draft().unwrap();
        d.draft_done(
            req.id,
            Err("Error invoking remote method 'actions:draft': Error: Drafting the button took too long".into()),
        );
        assert_eq!(
            d.error.as_deref(),
            Some(
                "Claude couldn’t draft it: Error invoking remote method 'actions:draft': Error: Drafting the button took too long"
            )
        );
        assert_eq!(
            d.back_button(),
            Some(Button {
                label: "Back to the draft",
                enabled: true
            })
        );
        // Closed while drafting: the answer is dropped.
        let req = d.begin_draft().unwrap();
        assert_eq!(d.error, None);
        d.close();
        d.draft_done(
            req.id,
            Ok(ActionDraft {
                name: "Late".into(),
                icon: "zap".into(),
                prompt: "p".into(),
                when: String::new(),
            }),
        );
        assert_eq!(d.name, "Run Tests");
    }

    #[test]
    fn saves_what_was_reviewed() {
        let mut d = ActionDialog::new(None);
        d.step = Step::Review;
        assert_eq!(d.submit("new-id", 9.0), Submit::Nothing);
        d.prompt = "  Run them.  ".into();
        d.name = "  ".into();
        d.description = " tests ".into();
        let Submit::Save(raw) = d.submit("new-id", 9.0) else {
            panic!("saves");
        };
        assert_eq!(
            raw,
            json!({ "id": "new-id", "name": "Action", "icon": "zap", "description": "tests", "prompt": "Run them.", "createdAt": 9.0 })
        );
        assert!(d.saving);
        assert_eq!(d.begin_save("x", 1.0), None);
        d.save_failed("Error invoking remote method 'actions:save': Error: An action needs a prompt".into());
        assert!(!d.saving);
        assert_eq!(
            d.primary_button(),
            Button {
                label: "Add action",
                enabled: true
            }
        );
        // Editing keeps the id and creation time, and says Save.
        let mut e = ActionDialog::new(Some(existing()));
        assert_eq!(e.primary_button().label, "Save");
        let raw = e.begin_save("unused", 1.0).unwrap();
        assert_eq!(
            (raw["id"].clone(), raw["createdAt"].clone(), raw["global"].clone()),
            (json!("a1"), json!(5.0), json!(true))
        );
    }

    #[test]
    fn sets_a_schedule_in_words_or_with_the_chips() {
        let mut d = ActionDialog::new(None);
        d.step = Step::Review;
        d.prompt = "Sum up the PRs.".into();
        d.global = true;
        // SCHED-040: on, it starts as every weekday at 9 and isn't global.
        d.set_scheduled(true);
        assert_eq!(
            (d.global, d.when.as_str(), d.time_text().as_str()),
            (false, DEFAULT_WHEN, "9:00")
        );
        assert!((1..=5).all(|day| d.day_on(day)) && !d.day_on(0) && !d.day_on(6));
        // SCHED-041
        d.set_when("mondays at 5pm");
        assert_eq!(d.time_text(), "17:00");
        assert!(d.day_on(1) && !d.day_on(2) && !d.when_unclear);
        d.set_when("whenever");
        assert!(d.when_unclear);
        assert_eq!(d.when_hint(0.0), WHEN_UNCLEAR);
        assert_eq!(d.time_text(), "17:00", "keeps what it understood");
        // SCHED-042: chips and the time say the schedule back in the field.
        assert!(d.toggle_day(3));
        assert_eq!(d.when, "Mon and Wed at 17:00");
        assert!(!d.when_unclear);
        assert!(d.toggle_day(1) && !d.toggle_day(3), "the last day stays");
        assert!(d.set_time("8:30") && !d.set_time("soon"));
        assert_eq!(d.when, "Wednesdays at 8:30");
        d.set_when("every 2 hours");
        assert!(d.set_time("9"));
        assert_eq!(d.when, "Every day at 9:00");
        // SCHED-043
        let raw = d.begin_save("id", 1.0).unwrap();
        assert_eq!(raw["schedule"], json!({ "words": "Every day at 9:00", "at": "09:00" }));
        assert_eq!(raw.get("global"), None);
        let saved = crate::normalize_action(&raw, None, 1.0).unwrap();
        assert_eq!(
            schedule::describe(saved.schedule.as_ref().unwrap()),
            "Every day at 9:00"
        );
        // Editing it starts on a schedule; turning global on turns it off.
        let mut e = ActionDialog::new(Some(saved));
        assert!(e.scheduled);
        assert_eq!(e.when, "Every day at 9:00");
        e.set_global(true);
        assert!(!e.scheduled);
        assert_eq!(e.begin_save("x", 1.0).unwrap().get("schedule"), None);
    }

    #[test]
    fn takes_the_schedule_from_the_draft() {
        let mut d = ActionDialog::new(None);
        d.description = "Every weekday at 9, sum up the PRs".into();
        let request = d.begin_draft().unwrap();
        let draft = ActionDraft {
            name: "PR Sweep".into(),
            icon: "git-pull-request".into(),
            prompt: "Sum up the open PRs.".into(),
            when: "every weekday at 9".into(),
        };
        d.draft_done(request.id, Ok(draft.clone()));
        assert!(d.scheduled);
        assert_eq!(schedule::describe(d.schedule.as_ref().unwrap()), "Weekdays at 9:00");
        // Not for a global action.
        let mut g = ActionDialog::new(None);
        g.description = "x".into();
        g.global = true;
        let request = g.begin_draft().unwrap();
        g.draft_done(request.id, Ok(draft));
        assert!(!g.scheduled);
    }

    #[test]
    fn opens_as_an_automation() {
        // SCHED-045
        let mut d = ActionDialog::automation(None);
        assert_eq!(
            (d.title(), d.shows_scope(), d.shows_when_switch()),
            ("New automation", false, false)
        );
        assert!(d.subtitle().starts_with("Say what it should do and when."));
        assert_eq!(d.description_placeholder(), AUTOMATION_PLACEHOLDER);
        assert!(d.scheduled && !d.global);
        assert_eq!(schedule::describe(d.schedule.as_ref().unwrap()), "Weekdays at 9:00");
        assert_eq!(d.when, "", "the field shows its placeholder");
        // SCHED-046: a chip says what and when.
        d.pick_chip(2);
        assert!(d.chip_selected(2) && !d.chip_selected(0));
        assert_eq!(d.when, "every day at 2am");
        assert_eq!(d.time_text(), "2:00");
        assert_eq!(d.chip_labels()[0], "Morning PR sweep");
        // An unclear When holds the draft back.
        d.set_when("whenever");
        assert!(!d.primary_button().enabled && d.begin_draft().is_none());
        d.set_when("every weekday at 9");
        // SCHED-047: the drafter hears when it runs; the draft's own `when` doesn't win.
        let request = d.begin_draft().unwrap();
        assert!(
            request
                .description
                .ends_with("\n\nIt runs on its own: Weekdays at 9:00.")
        );
        d.draft_done(
            request.id,
            Ok(ActionDraft {
                name: "Nightly Tests".into(),
                icon: "test-tube".into(),
                prompt: "Run them.".into(),
                when: "every day at 2am".into(),
            }),
        );
        assert_eq!(schedule::describe(d.schedule.as_ref().unwrap()), "Weekdays at 9:00");
        assert_eq!(d.primary_button().label, "Add automation");
        let raw = d.begin_save("id", 1.0).unwrap();
        assert_eq!(raw["schedule"]["days"], json!([1, 2, 3, 4, 5]));
        // Editing one from its tile.
        let saved = crate::normalize_action(&raw, None, 1.0).unwrap();
        let e = ActionDialog::automation(Some(saved));
        assert_eq!(
            (e.title(), e.step, e.when.as_str()),
            ("Edit automation", Step::Review, "every weekday at 9")
        );
    }
}
