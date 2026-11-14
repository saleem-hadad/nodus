//! The tools that show something in the session's chat (spec 16): a chart,
//! stats, a table or a text layout (`show_visual`), questions for the user
//! (`ask`), and a page on the board (`show_page`). The chat draws them from
//! the transcript, where Claude Code records each call, so the first two
//! only check what Claude wrote and say what happens next.

use serde_json::{Map, Value, json};

use crate::server::ToolError;

/// TOOLCHAT-001: a visual the chat can draw, or exactly why not.
pub(crate) fn show_visual(args: &Value) -> Result<Value, ToolError> {
    let Some(visual) = args.get("visual") else {
        return Err("visual is required: the chart, stats, table or layout, as JSON".into());
    };
    nodus_visual::parse_visual(visual).map_err(|err| ToolError(format!("The visual can’t be shown: {err}")))?;
    Ok(json!({
        "shown": true,
        "note": "It’s in the user’s chat now. Don’t repeat in words what it already shows: add only what it doesn’t, such as the conclusion or the next step."
    }))
}

/// TOOLCHAT-002: a form the chat can draw, or exactly why not.
pub(crate) fn ask(args: &Value) -> Result<Value, ToolError> {
    let form =
        nodus_visual::parse_form(args).map_err(|err| ToolError(format!("The questions can’t be shown: {err}")))?;
    Ok(json!({
        "shown": true,
        "questions": form.questions.len(),
        "note": "The form is in the user’s chat. End your turn now, and don’t ask the same in words: the answers arrive as the user’s next message, in a <nodus-answers> block keyed by question, with a question left blank as \"skipped\"."
    }))
}

/// TOOLCHAT-003: what of `show_page`'s arguments go on to `create_view`. A
/// page has no commands, schedule or brain of its own to set up, and it's
/// always the session's to write: without html it fails as a blank page
/// does, rather than going to a brain (VIEW-161).
pub(crate) fn page_args(args: &Value) -> Value {
    let mut out = Map::new();
    for key in ["title", "html", "description", "data", "width", "height"] {
        match args.get(key) {
            None | Some(Value::Null) if key == "html" => {
                out.insert(key.to_owned(), Value::String(String::new()));
            }
            Some(value) => {
                out.insert(key.to_owned(), value.clone());
            }
            None => {}
        }
    }
    Value::Object(out)
}

/// TOOLCHAT-003: `create_view`'s answer, with what happens next.
pub(crate) fn page_shown(mut created: Value) -> Value {
    if let Value::Object(out) = &mut created {
        out.insert(
            "note".into(),
            json!("The page is on the board beside your card, and the user’s chat has a card for it. If it asks for answers, they arrive as the user’s next message, in a <nodus-answers> block, once the page calls nodus.submit(answers). Change it with update_view and its view_id."),
        );
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visual_is_checked_and_why_it_fails_is_said() {
        let ok = json!({ "visual": { "mark": "bar", "data": { "values": [{ "a": "x", "b": 1 }] }, "encoding": { "x": { "field": "a", "type": "nominal" }, "y": { "field": "b", "type": "quantitative" } } } });
        assert!(show_visual(&ok).is_ok());
        let missing = show_visual(&json!({})).unwrap_err();
        assert!(missing.0.starts_with("visual is required"));
        let bad = show_visual(&json!({ "visual": { "mark": "rule" } })).unwrap_err();
        assert!(bad.0.starts_with("The visual can’t be shown: "), "{}", bad.0);
    }

    #[test]
    fn questions_are_checked() {
        let ok = json!({ "title": "Layout", "questions": [{ "title": "Which?", "choices": ["Tabs", "One page"] }] });
        assert_eq!(ask(&ok).unwrap()["questions"], json!(1));
        assert!(ask(&json!({ "questions": [] })).is_err());
    }

    #[test]
    fn a_page_takes_only_a_pages_arguments() {
        let args = json!({ "title": "T", "html": "<p>", "commands": [], "refresh": 10, "width": 600 });
        assert_eq!(page_args(&args), json!({ "title": "T", "html": "<p>", "width": 600 }));
    }

    #[test]
    fn a_page_without_html_stays_the_sessions_and_never_goes_to_a_brain() {
        // VIEW-161: create_view without html hands a view to its brain; a
        // page is blank instead, which create_view turns down.
        for args in [
            json!({ "title": "Mockups" }),
            json!({ "title": "Mockups", "html": null }),
        ] {
            assert_eq!(page_args(&args), json!({ "title": "Mockups", "html": "" }));
        }
    }
}
