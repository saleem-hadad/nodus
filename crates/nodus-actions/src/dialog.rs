//! The action dialog's logic, for nodus-ui to draw (`ActionDialog.tsx`,
//! ACT-067…ACT-080): describe, let Claude draft, review, save. The window's
//! calls (`actions:draft`, `actions:save`) are made by the UI with what this
//! hands it, and their answers handed back.

use nodus_domain::js::trim;
use nodus_domain::{ActionDraft, ActionIcon, ProjectAction};
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
    request: i64,
}

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
            request: 0,
            action,
        }
    }

    /// ACT-068.
    pub fn title(&self) -> &'static str {
        if self.action.is_some() {
            "Edit action"
        } else {
            "New action"
        }
    }

    /// ACT-068.
    pub fn subtitle(&self) -> &'static str {
        match self.step {
            Step::Describe => "Say what the button should do. Claude writes the prompt each press runs.",
            Step::Review => {
                "Each press runs a session on this prompt in auto mode, beside its button. It goes when it’s done."
            }
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
        EXAMPLES.get(index).is_some_and(|(_, text)| self.description == *text)
    }

    /// ACT-070: a chip replaces the description; chips do nothing while drafting.
    pub fn pick_chip(&mut self, index: usize) {
        if let (false, Some((_, text))) = (self.drafting, EXAMPLES.get(index)) {
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
        if what.is_empty() || self.drafting {
            return None;
        }
        self.request += 1;
        self.drafting = true;
        self.error = None;
        Some(DraftRequest {
            id: self.request,
            description: what.to_owned(),
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
        Some(Value::Object(raw))
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
                enabled: !trim(&self.description).is_empty() && !self.drafting,
            },
            Step::Review => Button {
                label: if self.action.is_some() { "Save" } else { "Add action" },
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
}
