//! An action from the window, checked and trimmed (`normalizeAction`,
//! ACT-001…ACT-012).

use nodus_domain::js::{slice_utf16, trim};
use nodus_domain::{ActionIcon, ProjectAction};
use serde_json::{Map, Value};

use crate::ActionError;

/// The longest name an action keeps, in UTF-16 units.
pub const MAX_NAME_CHARS: usize = 40;
/// The longest description an action keeps.
pub const MAX_DESCRIPTION_CHARS: usize = 4000;
/// The longest prompt an action keeps.
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// ACT-004: `clip(s, max)`: `''` for anything but a string, else the string
/// trimmed and cut to its first `max` UTF-16 units.
pub fn clip(value: Option<&Value>, max: usize) -> String {
    match value {
        Some(Value::String(s)) => slice_utf16(trim(s), 0, max),
        _ => String::new(),
    }
}

/// ACT-003: `isActionIcon`: one of the 26 icon names.
pub fn is_action_icon(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::String(s)) if ActionIcon::parse(s).is_some())
}

/// The icon a value names, or `zap` (ACT-008, ACT-026).
pub(crate) fn icon_or_default(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) if ActionIcon::parse(s).is_some() => s.clone(),
        _ => ActionIcon::DEFAULT.as_str().to_owned(),
    }
}

/// `normalizeAction(raw, existing)`: an action from the window (any JSON, as
/// IPC delivers it), checked and trimmed. `now` is the creation time of a new
/// action, in ms. Fails without a prompt (ACT-005).
pub fn normalize_action(raw: &Value, existing: Option<&ProjectAction>, now: f64) -> Result<ProjectAction, ActionError> {
    let field = |name: &str| raw.as_object().and_then(|o| o.get(name));
    // ACT-005
    let prompt = clip(field("prompt"), MAX_PROMPT_CHARS);
    if prompt.is_empty() {
        return Err(ActionError::NoPrompt);
    }
    // ACT-006
    let id = match (existing, field("id")) {
        (Some(existing), _) => existing.id.clone(),
        (None, Some(Value::String(id))) if !id.is_empty() => id.clone(),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    // ACT-007
    let name = Some(clip(field("name"), MAX_NAME_CHARS))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Action".to_owned());
    // ACT-012: the fields are written in this order, and nothing else is kept.
    Ok(ProjectAction {
        id,
        name,
        // ACT-008
        icon: icon_or_default(field("icon")),
        // ACT-009
        description: clip(field("description"), MAX_DESCRIPTION_CHARS),
        prompt,
        // ACT-010: only `true` itself.
        global: (field("global") == Some(&Value::Bool(true))).then_some(true),
        // ACT-011
        created_at: existing.map_or(now, |e| e.created_at),
        extra: Map::new(),
    })
}

#[cfg(test)]
mod tests {
    //! actions.test.ts › normalizeAction.
    use super::*;
    use serde_json::json;

    const NOW: f64 = 1_700_000_000_000.0;

    #[test]
    fn needs_a_prompt() {
        for raw in [
            json!({ "name": "Test" }),
            json!({ "prompt": "   " }),
            json!(null),
            json!({ "prompt": 5 }),
        ] {
            let err = normalize_action(&raw, None, NOW).unwrap_err();
            assert_eq!(err.to_string(), "An action needs a prompt");
        }
    }

    #[test]
    fn fills_in_a_new_action() {
        let action = normalize_action(&json!({ "prompt": "  Run the tests.  " }), None, NOW).unwrap();
        assert_eq!(action.id.len(), 36);
        assert!(
            action
                .id
                .chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_eq!(
            nodus_domain::json::to_string(&action).unwrap(),
            format!(
                r#"{{"id":"{}","name":"Action","icon":"zap","description":"","prompt":"Run the tests.","createdAt":1700000000000}}"#,
                action.id
            )
        );
    }

    #[test]
    fn trims_and_clips_what_the_renderer_sends() {
        let action = normalize_action(
            &json!({
                "name": format!("  {}  ", "N".repeat(60)),
                "prompt": "p".repeat(25_000),
                "description": "d".repeat(5000),
                "icon": "not-an-icon",
                "global": "yes",
            }),
            None,
            NOW,
        )
        .unwrap();
        assert_eq!(action.name.len(), 40);
        assert_eq!(action.prompt.len(), 20_000);
        assert_eq!(action.description.len(), 4000);
        assert_eq!(action.icon, "zap");
        assert_eq!(action.global, None);
    }

    #[test]
    fn keeps_an_existing_actions_id_and_creation_time() {
        let existing = normalize_action(&json!({ "prompt": "Old", "icon": "bug", "global": true }), None, NOW).unwrap();
        let edited = normalize_action(
            &json!({ "id": "other", "prompt": "New", "icon": "rocket", "global": true }),
            Some(&existing),
            NOW + 5.0,
        )
        .unwrap();
        assert_eq!(edited.id, existing.id);
        assert_eq!(edited.created_at, existing.created_at);
        assert_eq!(edited.prompt, "New");
        assert_eq!(edited.icon, "rocket");
        assert_eq!(edited.global, Some(true));
    }

    // Beyond actions.test.ts.

    #[test]
    fn takes_the_ids_the_window_gives_and_clips_in_utf16() {
        let given = normalize_action(&json!({ "id": "a1", "prompt": "x" }), None, NOW).unwrap();
        assert_eq!(given.id, "a1");
        let blank = normalize_action(&json!({ "id": "", "prompt": "x" }), None, NOW).unwrap();
        assert_eq!(blank.id.len(), 36);
        let number = normalize_action(&json!({ "id": 7, "prompt": "x", "name": 3 }), None, NOW).unwrap();
        assert_eq!((number.id.len(), number.name.as_str()), (36, "Action"));
        // Forty units: the emoji is two, so the name stops after it.
        let name = format!("{}😀b", "a".repeat(38));
        let clipped = normalize_action(&json!({ "prompt": "x", "name": name }), None, NOW).unwrap();
        assert_eq!(clipped.name, format!("{}😀", "a".repeat(38)));
        // An existing action's unknown fields aren't carried over.
        let mut existing = given.clone();
        existing.extra.insert("pinned".into(), json!(true));
        let edited = normalize_action(&json!({ "prompt": "y" }), Some(&existing), NOW).unwrap();
        assert!(edited.extra.is_empty());
        assert!(is_action_icon(Some(&json!("message-square"))));
        assert!(!is_action_icon(Some(&json!("Zap"))));
        assert!(!is_action_icon(None));
    }
}
