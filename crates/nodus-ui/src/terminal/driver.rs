//! The test driver's `terminal` command (decision D11), for the parity
//! scenes and the end-to-end checks of this module:
//!
//! ```text
//! {"cmd":"terminal","action":"board","project":"<id>"}            open a board, loaded from the core
//! {"cmd":"terminal","action":"liven","project":"<id>","sessions":["<id>"],"cols":120,"rows":36}
//! {"cmd":"terminal","action":"reload"}                             the open board again (statuses, usage)
//! {"cmd":"terminal","action":"open","session":"<id>"}              the session window
//! {"cmd":"terminal","action":"preview","session":"<id>"}           the transcript preview
//! {"cmd":"terminal","action":"agents","session":"<id>","agent":"<id>"}
//! {"cmd":"terminal","action":"close"}                              ⌘W's layer
//! {"cmd":"terminal","action":"screen","session":"<id>"}            the terminal's visible text
//! {"cmd":"terminal","action":"type","session":"<id>","text":"…"}   typed into the terminal
//! ```
//!
//! Until the board's own track loads boards, `board` puts the core's
//! snapshot in the app state itself.

use gpui::AsyncApp;
use serde_json::{Value, json};

use super::actions;
use super::store::Terminals;
use crate::shell::window;
use crate::state::Route;

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

pub async fn run(command: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let action = text(command, "action");
    let state = cx
        .update(|cx| window::main_shell(cx).map(|s| s.read(cx).state.clone()))
        .ok_or("no main window")?;
    let nodus = cx.update(|cx| state.read(cx).nodus.clone());
    match action.as_str() {
        "board" | "reload" => {
            let project = if action == "board" {
                text(command, "project")
            } else {
                cx.update(|cx| state.read(cx).route.project_id().map(str::to_owned))
                    .ok_or("no board open")?
            };
            let board = nodus.board().load(&project).await.map_err(|e| e.ipc_message())?;
            cx.update(|cx| {
                state.update(cx, |s, cx| {
                    if action == "board" {
                        s.navigate(Route::Board(project.clone()), cx);
                    }
                    s.board = Some(board);
                    cx.notify();
                });
            });
            Ok(Value::Null)
        }
        "liven" => {
            let project = text(command, "project");
            let cols = command.get("cols").and_then(Value::as_f64).unwrap_or(120.0);
            let rows = command.get("rows").and_then(Value::as_f64).unwrap_or(36.0);
            let sessions: Vec<String> = command
                .get("sessions")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            for id in sessions {
                nodus
                    .sessions()
                    .open(&project, &id, cols, rows)
                    .await
                    .map_err(|e| e.ipc_message())?;
            }
            Ok(Value::Null)
        }
        "open" => {
            let id = text(command, "session");
            cx.update(|cx| actions::open_terminal(&state, &id, cx));
            Ok(Value::Null)
        }
        "preview" => {
            let id = text(command, "session");
            cx.update(|cx| actions::open_preview(&state, &id, None, cx));
            Ok(Value::Null)
        }
        "agents" => {
            let id = text(command, "session");
            let agent = command.get("agent").and_then(Value::as_str).map(str::to_owned);
            cx.update(|cx| actions::open_agents(&state, &id, agent.as_deref(), cx));
            Ok(Value::Null)
        }
        "close" => {
            cx.update(|cx| state.update(cx, |s, cx| s.close_top_layer(cx)));
            Ok(Value::Null)
        }
        "screen" => {
            let id = text(command, "session");
            let screen = cx.update(|cx| {
                let terminals = Terminals::try_global(cx)?;
                let registry = terminals.read(cx).registry.clone();
                registry.with_terminal(&id, |t| {
                    let s = t.snapshot();
                    let lines: Vec<String> = s
                        .cells
                        .iter()
                        .map(|row| row.iter().map(|c| if c.text.is_empty() && c.width != 0 { " ".to_owned() } else { c.text.clone() }).collect::<String>().trim_end().to_owned())
                        .collect();
                    json!({ "cols": s.cols, "rows": s.rows, "lines": lines, "cursor": s.cursor.map(|c| [c.row, c.col]) })
                })
            });
            Ok(screen.unwrap_or(Value::Null))
        }
        "type" => {
            let id = text(command, "session");
            let typed = text(command, "text");
            cx.update(|cx| {
                if let Some(terminals) = Terminals::try_global(cx) {
                    terminals.read(cx).registry.input_text(&id, &typed);
                }
            });
            Ok(Value::Null)
        }
        other => Err(format!("unknown terminal action {other}")),
    }
}
