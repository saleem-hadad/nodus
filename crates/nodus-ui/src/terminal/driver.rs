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
//! {"cmd":"terminal","action":"route"}                              what's open (windows, search, find, cards, views)
//! {"cmd":"terminal","action":"save-nodes","project":"<id>","patches":[…]}   changed from outside, then board-changed
//! {"cmd":"terminal","action":"run-finished","run":{"projectId":…,"sessionId":…,"actionName":…}}
//! {"cmd":"terminal","action":"remove-view","view":"<id>"}          the store's removeView
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

/// A stand-in expanded card (the `card` action).
#[derive(Debug, Clone)]
struct TestCard {
    session_id: String,
    shell: bool,
    project_id: String,
    /// The card's flow rect.
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// The board's viewport: flow (0, 0) lands at (vx, 52 + vy).
    vx: f32,
    vy: f32,
    zoom: f32,
}

#[derive(Default)]
struct TestCards(Vec<TestCard>);

impl gpui::Global for TestCards {}

/// The stand-in cards' bodies, as `.card-term` lays them out: below the 40 px
/// title bar, padding 8 4 8 12, the terminal background, all at the zoom.
pub fn test_cards(window: &mut gpui::Window, cx: &mut gpui::App) -> Vec<gpui::AnyElement> {
    use gpui::{IntoElement, ParentElement, Styled, div, px};
    let Some(cards) = cx.try_global::<TestCards>().map(|c| c.0.clone()) else {
        return Vec::new();
    };
    cards
        .into_iter()
        .map(|c| {
            let z = c.zoom;
            let terminals = super::card::CardTerminals::new(c.project_id.clone(), z, cx);
            terminals.set_enabled(&c.session_id, c.shell, true, cx);
            let el = terminals.element(&c.session_id, window, cx);
            let body = div()
                .w(px(c.w * z))
                .h(px((c.h - 40.0) * z))
                .pt(px(8.0 * z))
                .pr(px(4.0 * z))
                .pb(px(8.0 * z))
                .pl(px(12.0 * z))
                .bg(super::element::rgb(super::element::TERM_BG))
                .children(el.map(|el| div().relative().size_full().child(el)));
            // Placed in the window, under the windows over the board.
            gpui::deferred(
                gpui::anchored()
                    .position(gpui::point(px(c.vx + c.x * z), px(52.0 + c.vy + (c.y + 40.0) * z)))
                    .child(body),
            )
            .with_priority(1)
            .into_any_element()
        })
        .collect()
}

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
            let runtime = cx.update(|cx| super::store::core_runtime(cx));
            for id in sessions {
                let (nodus, project) = (nodus.clone(), project.clone());
                runtime
                    .spawn(async move { nodus.sessions().open(&project, &id, cols, rows).await.map(|_| ()) })
                    .await
                    .map_err(|e| e.to_string())?
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
            cx.update(|cx| state.update(cx, crate::state::AppState::close_top_layer));
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
        // A frame drawn now, even while the window is covered (macOS stops
        // asking for frames then), so a screenshot shows the current state.
        "draw" => {
            cx.update(|cx| {
                if let Some(handle) = window::main_window(cx) {
                    let _ = gpui::AppContext::update_window(cx, handle.into(), |_, window, cx| {
                        window.run_next_frame_callbacks(cx);
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                }
            });
            Ok(Value::Null)
        }
        // Reloads the board until the session's card has this status (and is live).
        "await-status" => {
            let id = text(command, "session");
            let want = text(command, "status");
            let project = cx
                .update(|cx| state.read(cx).route.project_id().map(str::to_owned))
                .ok_or("no board open")?;
            for _ in 0..50 {
                let board = nodus.board().load(&project).await.map_err(|e| e.ipc_message())?;
                let ok = board
                    .cards
                    .iter()
                    .any(|c| c.node.session_id == id && c.live && c.status.as_str() == want);
                cx.update(|cx| {
                    state.update(cx, |s, cx| {
                        s.board = Some(board);
                        cx.notify();
                    });
                });
                if ok {
                    return Ok(Value::Null);
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            Err(format!("{id} never became {want}"))
        }
        // A stand-in for an expanded card's body on a zoomed board, until the
        // board's cards host terminals: the flow rect of the card's body
        // (under its 40 px title bar), the viewport's offset and zoom.
        "card" => {
            let id = text(command, "session");
            let num = |k: &str, d: f64| command.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
            let card = TestCard {
                session_id: id,
                shell: command.get("shell").and_then(Value::as_bool).unwrap_or(false),
                project_id: text(command, "project"),
                x: num("x", 0.0),
                y: num("y", 0.0),
                w: num("w", 720.0),
                h: num("h", 480.0),
                vx: num("vx", 0.0),
                vy: num("vy", 0.0),
                zoom: num("zoom", 1.0),
            };
            cx.update(|cx| {
                cx.default_global::<TestCards>().0.push(card);
                cx.refresh_windows();
            });
            Ok(Value::Null)
        }
        "route" => Ok(cx.update(|cx| {
            let confirm = window::main_shell(cx)
                .and_then(|shell| shell.read(cx).terminal_layers.read(cx).session_window().cloned())
                .is_some_and(|w| w.read(cx).asking());
            let search = window::main_shell(cx)
                .and_then(|shell| shell.read(cx).board().cloned())
                .is_some_and(|b| b.read(cx).search_open);
            let find = actions::preview_find(cx).map(|f| {
                json!({ "query": f.query, "snippet": f.snippet.map(|s| json!({ "text": s.text, "timestamp": s.timestamp })) })
            });
            let s = state.read(cx);
            json!({
                "confirm": confirm,
                "route": s.route.project_id(),
                "board": s.board.as_ref().map(|b| b.project.id.clone()),
                "terminal": s.terminal_session_id,
                "preview": s.preview_session_id,
                "agents": s.agent_view.as_ref().map(|(id, _)| id.clone()),
                "search": search,
                "find": find,
                "cards": s.board.as_ref().map(|b| b.cards.iter().map(|c| json!({ "id": c.node.session_id, "archived": c.node.archived })).collect::<Vec<_>>()),
                "views": s.board.as_ref().map(|b| b.views.iter().map(|v| json!({ "id": v.id, "title": v.title, "brain": v.session_id })).collect::<Vec<_>>()),
            })
        })),
        // The board as another process changes it (an MCP tool's
        // `archive_self`): saved through the core, then `board-changed`.
        "save-nodes" => {
            let project = text(command, "project");
            let patches: Vec<nodus_domain::NodePatch> =
                serde_json::from_value(command.get("patches").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
            let runtime = cx.update(|cx| super::store::core_runtime(cx));
            let (core, id) = (nodus.clone(), project.clone());
            runtime
                .spawn(async move { core.board().save_nodes(&id, &patches).await })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.ipc_message())?;
            nodus.emit(nodus_domain::Event::BoardChanged { project_id: project });
            Ok(Value::Null)
        }
        // An action run finished, as the core reports it (MODAL-029). Says
        // whether the run's terminal was disposed: the registry's change for
        // it with no terminal left, seen before a frame could attach it again
        // (closing the window expands a card still on the board, BOARD-103).
        "run-finished" => {
            let run: nodus_domain::ActionRunResult =
                serde_json::from_value(command.get("run").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
            let disposed = std::rc::Rc::new(std::cell::Cell::new(false));
            let watch = cx.update(|cx| {
                let terminals = Terminals::try_global(cx)?;
                let (flag, id) = (disposed.clone(), run.session_id.clone());
                Some(cx.subscribe(&terminals, move |t, event, cx| {
                    if let super::store::TerminalsEvent::Changed(changed) = event
                        && changed.as_ref() == id
                        && !t.read(cx).registry.contains(&id)
                    {
                        flag.set(true);
                    }
                }))
            });
            nodus.emit(nodus_domain::Event::RunFinished(run));
            for _ in 0..40 {
                if disposed.get() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            drop(watch);
            Ok(json!({ "disposed": disposed.get() }))
        }
        // The board's `removeView`, as Delete does once confirmed (MODAL-030).
        "remove-view" => {
            let id = text(command, "view");
            let removed = cx.update(|cx| state.update(cx, |s, cx| s.remove_view(&id, cx)));
            removed.await;
            Ok(Value::Null)
        }
        other => Err(format!("unknown terminal action {other}")),
    }
}
