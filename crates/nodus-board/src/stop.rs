//! Stopping sessions from the board or the terminal modal: which cards have a
//! process to stop, and what to ask first. Stopping a session ends its Claude
//! process, and the MCP servers it started, but keeps its card: opening the
//! card resumes the session. A port of `stopSessions.ts`; the kill itself
//! (all at once, waiting for each, SESS-153) is the core's.

use nodus_domain::{SessionCard, SessionKind, SessionStatus};

/// What to ask before stopping: the confirm dialog's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopConfirmation {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub danger: bool,
}

/// The Claude sessions among `cards` with a process to stop. Terminals are
/// removed instead, and an action run has no card to keep: it leaves the
/// board once its process ends. Views' brains can be stopped (SESS-149).
pub fn stoppable(cards: &[SessionCard]) -> Vec<&SessionCard> {
    cards
        .iter()
        .filter(|c| c.live && c.node.kind != Some(SessionKind::Terminal) && !truthy(c.node.run_of.as_deref()))
        .collect()
}

/// What to ask before stopping `cards`, or none when none is mid-turn, so
/// nothing is cut off (SESS-150…152). `title_of` is the card's display title.
pub fn stop_confirmation(cards: &[SessionCard], title_of: impl Fn(&SessionCard) -> String) -> Option<StopConfirmation> {
    let busy: Vec<&SessionCard> = cards.iter().filter(|c| c.status != SessionStatus::Done).collect();
    if busy.is_empty() {
        return None;
    }
    let several = cards.len() > 1;
    // A session alone in being mid-turn is named, even among several.
    let cut_off = if let [only] = busy[..] {
        let doing = if only.status == SessionStatus::Waiting {
            "waiting for you"
        } else {
            "working"
        };
        format!("“{}” is still {doing}. Stopping it cuts off this turn.", title_of(only))
    } else {
        "Some of these sessions are still working. Stopping them cuts off their turns.".into()
    };
    let stays = if several {
        "The transcripts stay, so you can resume them."
    } else {
        "The transcript stays, so you can resume it."
    };
    Some(StopConfirmation {
        title: if several {
            format!("Stop {} sessions?", cards.len())
        } else {
            "Stop a running session?".into()
        },
        message: format!("{cut_off} {stays}"),
        confirm_label: "Stop".into(),
        danger: true,
    })
}

fn truthy(s: Option<&str>) -> bool {
    s.is_some_and(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn card(session_id: &str, patch: &Value) -> SessionCard {
        let mut v = json!({ "sessionId": session_id, "x": 0, "y": 0, "archived": false, "createdAt": 0, "status": "done", "live": true });
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn title_of(c: &SessionCard) -> String {
        c.node.title.clone().unwrap_or_else(|| c.node.session_id.clone())
    }

    fn ids(cards: &[&SessionCard]) -> Vec<String> {
        cards.iter().map(|c| c.node.session_id.clone()).collect()
    }

    #[test]
    fn keeps_the_claude_sessions_that_are_running() {
        let cards = [
            card("a", &json!({})),
            card("b", &json!({ "live": false })),
            card("c", &json!({ "kind": "terminal" })),
            card("d", &json!({ "kind": "claude" })),
        ];
        assert_eq!(ids(&stoppable(&cards)), ["a", "d"]);
    }

    #[test]
    fn leaves_out_action_runs_which_have_no_card_to_keep() {
        let cards = [
            card("a", &json!({ "runOf": "deploy" })),
            card("b", &json!({ "viewOf": "stats" })),
        ];
        assert_eq!(ids(&stoppable(&cards)), ["b"]);
    }

    #[test]
    fn doesnt_ask_when_every_session_is_between_turns() {
        assert_eq!(
            stop_confirmation(&[card("a", &json!({})), card("b", &json!({}))], title_of),
            None
        );
    }

    #[test]
    fn asks_for_a_session_thats_working_by_name() {
        let ask = stop_confirmation(
            &[card("a", &json!({ "status": "working", "title": "Fix login" }))],
            title_of,
        )
        .unwrap();
        assert_eq!(ask.title, "Stop a running session?");
        assert_eq!(ask.confirm_label, "Stop");
        assert!(ask.danger);
        assert!(ask.message.contains("“Fix login” is still working"));
        assert_eq!(
            ask.message,
            "“Fix login” is still working. Stopping it cuts off this turn. The transcript stays, so you can resume it."
        );
    }

    #[test]
    fn says_when_a_session_is_waiting_for_you() {
        let ask = stop_confirmation(&[card("a", &json!({ "status": "waiting" }))], title_of).unwrap();
        assert!(ask.message.contains("waiting for you"));
    }

    #[test]
    fn asks_once_for_several_when_any_of_them_is_mid_turn() {
        let cards = [
            card("a", &json!({})),
            card("b", &json!({ "status": "working" })),
            card("c", &json!({ "status": "working" })),
        ];
        let ask = stop_confirmation(&cards, title_of).unwrap();
        assert_eq!(ask.title, "Stop 3 sessions?");
        assert!(ask.message.contains("Some of these sessions are still working"));
    }

    #[test]
    fn names_the_one_session_among_several_thats_mid_turn() {
        let cards = [
            card("a", &json!({})),
            card("b", &json!({ "status": "waiting", "title": "Fix login" })),
            card("c", &json!({})),
        ];
        let ask = stop_confirmation(&cards, title_of).unwrap();
        assert_eq!(ask.title, "Stop 3 sessions?");
        assert!(ask.message.contains("“Fix login” is still waiting for you"));
        assert!(ask.message.contains("The transcripts stay"));
    }
}
