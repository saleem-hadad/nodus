//! Between views' pages and the app (`viewRelay.ts`, VIEW-044…055). A page
//! posts its requests to the board; the relay answers them for the view
//! whose frame sent them, which it knows by the frame, never by anything the
//! page says (VIEW-044). Most go to the core, which checks them; opening and
//! starting sessions happen on the board. The core's view events, and the
//! board's `board` and `git` news, go back to the pages.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Global, Modifiers, ScrollDelta, ScrollEvent, Task, TouchPhase, WeakEntity, point, px};
use nodus_domain::{BoardNote, BoardView, Event, SessionCard, ViewReport};
use serde_json::{Value, json};

use super::frames::{Frames, Slot, existing};
use super::webview::Incoming;
use crate::board::canvas::BoardCanvas;

/// `{event: 'board'}` goes out this long after the board last changed (VIEW-055).
const BOARD_DEBOUNCE: Duration = Duration::from_millis(300);
/// How often the board's git status is read again while the window has focus
/// (`GIT_POLL_MS`).
const GIT_POLL: Duration = Duration::from_millis(4000);

/// What a page posted, as the relay sorts it (VIEW-044…048).
#[derive(Debug, Clone, PartialEq)]
pub enum PageMessage {
    /// `{error}`: a JavaScript error, for `views.report`.
    Error(String),
    /// `{loaded}`: the page loaded.
    Loaded,
    /// `{wheel}`: a scroll the page had no use for.
    Wheel(PageWheel),
    /// `{id, method, args}`: a request, answered by `id`.
    Request { id: Value, method: String, args: Value },
}

/// `{dx, dy, mode, x, y, ctrl, shift, alt, meta}`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageWheel {
    pub dx: f64,
    pub dy: f64,
    pub mode: f64,
    pub x: f64,
    pub y: f64,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// `num`: a finite number, else 0.
fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0)
}

/// JavaScript truthiness, for `msg.loaded`.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Sorts what a page posted, in 0.9.4's order: anything that isn't
/// `{nodus: 1, …}` is ignored; then an error, `loaded`, a wheel, a request.
pub fn parse(msg: &Value) -> Option<PageMessage> {
    let obj = msg.as_object()?;
    if obj.get("nodus").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    if let Some(Value::String(error)) = obj.get("error") {
        return Some(PageMessage::Error(error.clone()));
    }
    if obj.get("loaded").is_some_and(truthy) {
        return Some(PageMessage::Loaded);
    }
    if let Some(Value::Object(w)) = obj.get("wheel") {
        return Some(PageMessage::Wheel(PageWheel {
            dx: num(w.get("dx")),
            dy: num(w.get("dy")),
            mode: num(w.get("mode")),
            x: num(w.get("x")),
            y: num(w.get("y")),
            ctrl: w.get("ctrl") == Some(&Value::Bool(true)),
            shift: w.get("shift") == Some(&Value::Bool(true)),
            alt: w.get("alt") == Some(&Value::Bool(true)),
            meta: w.get("meta") == Some(&Value::Bool(true)),
        }));
    }
    let id = obj.get("id").filter(|v| v.is_number())?.clone();
    let method = obj.get("method").and_then(Value::as_str)?.to_owned();
    let args = obj.get("args").cloned().unwrap_or(Value::Null);
    Some(PageMessage::Request { id, method, args })
}

/// The reply to a request (VIEW-048).
pub fn reply(id: &Value, answer: Result<Value, String>) -> Value {
    match answer {
        Ok(result) => json!({ "nodus": 1, "id": id, "result": result }),
        Err(error) => json!({ "nodus": 1, "id": id, "error": error }),
    }
}

/// An event for pages (VIEW-054, VIEW-055); `payload: undefined` is left out.
pub fn event(name: &str, payload: Option<Value>) -> Value {
    match payload {
        Some(payload) => json!({ "nodus": 1, "event": name, "payload": payload }),
        None => json!({ "nodus": 1, "event": name }),
    }
}

/// The board relay's own state.
#[derive(Default)]
struct Relay {
    canvas: Option<WeakEntity<BoardCanvas>>,
    /// What the board last told pages about: its cards, notes and views.
    last: Option<(Vec<SessionCard>, Vec<BoardNote>, Vec<BoardView>)>,
    board_timer: Option<Task<()>>,
    git: Option<String>,
    git_reading: bool,
}

#[derive(Clone)]
struct RelayGlobal(Rc<RefCell<Relay>>);

impl Global for RelayGlobal {}

fn relay(cx: &mut App) -> Rc<RefCell<Relay>> {
    if let Some(g) = cx.try_global::<RelayGlobal>() {
        return g.0.clone();
    }
    let r = Rc::new(RefCell::new(Relay::default()));
    cx.set_global(RelayGlobal(r.clone()));
    r
}

/// Starts relaying once: what the frames' shells post, and the core's events.
pub fn start(frames: &Rc<RefCell<Frames>>, cx: &mut App) {
    let Some(mut rx) = frames.borrow_mut().take_rx() else {
        return;
    };
    cx.spawn(async move |cx| {
        while let Some(msg) = rx.recv().await {
            cx.update(|cx| incoming(&msg, cx));
        }
    })
    .detach();
    let nodus = cx.global::<crate::shell::window::Core>().0.clone();
    let mut events = nodus.subscribe();
    cx.spawn(async move |cx| {
        loop {
            match events.recv().await {
                Ok(Event::View {
                    view_id,
                    event,
                    payload,
                    ..
                }) => {
                    let _ = cx.update(|cx| view_event(&view_id, event.as_str(), payload, cx));
                }
                Ok(Event::Status { .. }) => {
                    // Claude edits files during a turn: read the git status again.
                    let _ = cx.update(read_git);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .detach();
    // GIT_POLL_MS: catches branch switches and edits made elsewhere.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(GIT_POLL).await;
            let focused = cx.update(|cx| {
                crate::shell::window::main_window(cx)
                    .and_then(|h| h.update(cx, |_, window, _| window.is_window_active()).ok())
                    .unwrap_or(false)
            });
            if focused {
                cx.update(read_git);
            }
        }
    })
    .detach();
}

/// A frame's page posted something (VIEW-044…052).
fn incoming(msg: &Incoming, cx: &mut App) {
    let Some(rc) = existing(cx) else { return };
    let (project_id, view_id, version, slot, zoom, origin, keyboard) = {
        let f = rc.borrow();
        let Some(fr) = f.by_id(msg.frame) else { return };
        (
            fr.project_id.clone(),
            fr.view_id.clone(),
            fr.version,
            fr.slot,
            fr.geometry.map_or(1.0, |g| g.zoom),
            fr.geometry.map(|g| g.document.origin),
            fr.web.has_keyboard(),
        )
    };
    let Ok(body) = serde_json::from_str::<Value>(&msg.body) else {
        return;
    };
    // Hover over the page, from the shell itself.
    if let Some(hover) = body.get("h").and_then(Value::as_bool) {
        set_hovered(msg.frame, hover, cx);
        return;
    }
    // VIEW-051: the user just clicked or pressed something in this page (the
    // shell's transient activation, with its frame focused), and nothing
    // else on the board since.
    let activated = keyboard && body.get("activated") == Some(&Value::Bool(true));
    let Some(message) = body.get("m").and_then(parse) else {
        return;
    };
    super::swap::page_changed(msg.frame, cx);
    let nodus = cx.global::<crate::shell::window::Core>().0.clone();
    match message {
        PageMessage::Error(error) => nodus.views().report(
            &project_id,
            &view_id,
            &ViewReport {
                error: Some(error),
                loaded: None,
            },
        ),
        PageMessage::Loaded => nodus.views().report(
            &project_id,
            &view_id,
            &ViewReport {
                error: None,
                loaded: Some(version),
            },
        ),
        PageMessage::Wheel(w) => {
            // VIEW-047: played on the card as if the frame weren't there; the
            // large window's frame sits outside the board.
            if slot == Slot::Card
                && let Some(origin) = origin
            {
                replay_wheel(&w, origin, zoom, cx);
            }
        }
        PageMessage::Request { id, method, args } => {
            let frame = msg.frame;
            let answer = answer(&nodus, &project_id, &view_id, &method, args, activated, cx);
            cx.spawn(async move |cx| {
                let result = answer.await;
                let _ = cx.update(|cx| deliver_to(frame, &reply(&id, result), cx));
            })
            .detach();
        }
    }
}

fn set_hovered(frame: u64, hovered: bool, cx: &mut App) {
    let Some(rc) = existing(cx) else { return };
    let window = {
        let mut f = rc.borrow_mut();
        let Some(fr) = f.by_id_mut(frame) else { return };
        if fr.hovered == hovered {
            return;
        }
        fr.hovered = hovered;
        f.window
    };
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

/// The board the relay acts on.
fn canvas(cx: &mut App) -> Option<gpui::Entity<BoardCanvas>> {
    relay(cx).borrow().canvas.as_ref().and_then(WeakEntity::upgrade)
}

/// VIEW-049…052: what the board answers itself, and what goes to the core.
fn answer(
    nodus: &nodus_core::Nodus,
    project_id: &str,
    view_id: &str,
    method: &str,
    args: Value,
    activated: bool,
    cx: &mut App,
) -> Task<Result<Value, String>> {
    let a = if args.is_object() { args.clone() } else { json!({}) };
    match method {
        "openSession" => {
            let Some(id) = a.get("id").and_then(Value::as_str) else {
                return Task::ready(Err("nodus.openSession needs a session id".into()));
            };
            if let Some(board) = canvas(cx) {
                let id = id.to_owned();
                board.update(cx, |c, cx| {
                    if c.board()
                        .is_some_and(|b| b.cards.iter().any(|card| card.node.session_id == id))
                    {
                        c.open_card(&id, cx);
                    }
                });
            }
            Task::ready(Ok(Value::Null))
        }
        "startSession" => {
            let prompt = a
                .get("prompt")
                .and_then(Value::as_str)
                .map(|p| nodus_domain::js::trim(p).to_owned())
                .unwrap_or_default();
            if prompt.is_empty() {
                return Task::ready(Err("nodus.startSession needs a prompt".into()));
            }
            if !activated {
                return Task::ready(Err(
                    "A view can only start a session within a few seconds of the user clicking something in it".into(),
                ));
            }
            let Some(board) = canvas(cx) else {
                return Task::ready(Err("The board isn’t open".into()));
            };
            let title = a
                .get("title")
                .and_then(Value::as_str)
                .map(|t| nodus_domain::js::trim(t).to_owned());
            let view_id = view_id.to_owned();
            let started = board.update(cx, |c, cx| c.start_view_session(&view_id, prompt, title, cx));
            Task::ready(started.map(|()| Value::Null))
        }
        _ => {
            let rt = crate::board::core_rt(cx);
            let (nodus, project_id, view_id, method) = (
                nodus.clone(),
                project_id.to_owned(),
                view_id.to_owned(),
                method.to_owned(),
            );
            cx.background_executor().spawn(async move {
                crate::board::on_core(&rt, async move {
                    nodus
                        .views()
                        .call(&project_id, &view_id, &method, &args)
                        .await
                        .map_err(|e| super::reason(&e.ipc_message()))
                })
                .await
            })
        }
    }
}

/// VIEW-047: the scroll at `origin + (x, y) × zoom`, through the board's own
/// wheel handling (pan, or zoom with ctrl).
fn replay_wheel(w: &PageWheel, origin: gpui::Point<gpui::Pixels>, zoom: f32, cx: &mut App) {
    let Some(board) = canvas(cx) else { return };
    let position = point(
        origin.x + px((w.x * f64::from(zoom)) as f32),
        origin.y + px((w.y * f64::from(zoom)) as f32),
    );
    let (dx, dy) = (-w.dx as f32, -w.dy as f32);
    let delta = match w.mode as i64 {
        1 => ScrollDelta::Lines(point(dx, dy)),
        2 => ScrollDelta::Pixels(point(px(dx * 800.0), px(dy * 800.0))),
        _ => ScrollDelta::Pixels(point(px(dx), px(dy))),
    };
    let e = ScrollEvent {
        position,
        delta,
        modifiers: Modifiers {
            control: w.ctrl,
            shift: w.shift,
            alt: w.alt,
            platform: w.meta,
            function: false,
        },
        touch_phase: TouchPhase::Moved,
        source: Default::default(),
    };
    board.update(cx, |c, cx| c.wheel(&e, cx));
}

/// Posts a message to one frame.
fn deliver_to(frame: u64, message: &Value, cx: &mut App) {
    let Some(rc) = existing(cx) else { return };
    if let Some(fr) = rc.borrow().by_id(frame) {
        fr.web.deliver(message);
    }
    super::swap::page_changed(frame, cx);
}

/// Posts to every frame that `to` picks.
fn deliver_where(message: &Value, to: impl Fn(&super::frames::Frame) -> bool, cx: &mut App) {
    let Some(rc) = existing(cx) else { return };
    let ids: Vec<u64> = {
        let f = rc.borrow();
        f.frames
            .iter()
            .filter(|fr| to(fr))
            .map(|fr| {
                fr.web.deliver(message);
                fr.id
            })
            .collect()
    };
    for id in ids {
        super::swap::page_changed(id, cx);
    }
}

/// VIEW-054: the core's `ev:view` to every frame of that view; new data
/// also tells every other view's frames the board changed.
pub fn view_event(view_id: &str, name: &str, payload: Value, cx: &mut App) {
    deliver_where(&event(name, Some(payload)), |fr| fr.view_id == view_id, cx);
    if name == "data" {
        deliver_where(&event("board", None), |fr| fr.view_id != view_id, cx);
    }
}

/// The board mounted: `board` 300ms later, and `git` with what's known, which
/// is nothing yet (VIEW-055).
pub fn board_mounted(cx: &mut App) {
    let r = relay(cx);
    {
        let mut r = r.borrow_mut();
        r.last = None;
        r.git = None;
    }
    deliver_where(&event("git", Some(Value::Null)), |_| true, cx);
    read_git(cx);
}

/// Every board render: `board` to every frame 300ms after its cards, notes
/// or views last changed (VIEW-055, a trailing debounce).
pub fn board_changed(canvas: &BoardCanvas, cx: &mut Context<'_, BoardCanvas>) {
    let r = relay(cx);
    let mut rel = r.borrow_mut();
    rel.canvas = Some(cx.entity().downgrade());
    let Some(board) = canvas.board() else { return };
    let same = rel
        .last
        .as_ref()
        .is_some_and(|(c, n, v)| c == &board.cards && n == &board.notes && v == &board.views);
    if same {
        return;
    }
    rel.last = Some((board.cards.clone(), board.notes.clone(), board.views.clone()));
    rel.board_timer = Some(cx.spawn(async move |_, cx| {
        cx.background_executor().timer(BOARD_DEBOUNCE).await;
        let _ = cx.update(|cx| deliver_where(&event("board", None), |_| true, cx));
    }));
}

/// Reads the board's git status, and tells pages when it changed (VIEW-055).
/// Only while some view has a page.
fn read_git(cx: &mut App) {
    let Some(rc) = existing(cx) else { return };
    let project_id = {
        let f = rc.borrow();
        if f.frames.is_empty() {
            return;
        }
        let Some(id) = f.project_id.clone() else { return };
        id
    };
    let r = relay(cx);
    if std::mem::replace(&mut r.borrow_mut().git_reading, true) {
        return;
    }
    let nodus = cx.global::<crate::shell::window::Core>().0.clone();
    let rt = crate::board::core_rt(cx);
    cx.spawn(async move |cx| {
        let status = crate::board::on_core(&rt, {
            let project_id = project_id.clone();
            async move { nodus.git().status(&project_id).await.ok().flatten() }
        })
        .await;
        let _ = cx.update(|cx| {
            let r = relay(cx);
            r.borrow_mut().git_reading = false;
            let still = existing(cx).is_some_and(|f| f.borrow().project_id.as_deref() == Some(project_id.as_str()));
            if !still {
                return;
            }
            let payload = serde_json::to_value(&status).unwrap_or(Value::Null);
            let text = nodus_domain::json::to_string(&payload).unwrap_or_default();
            let changed = r.borrow().git.as_deref() != Some(text.as_str());
            if changed {
                r.borrow_mut().git = Some(text);
                deliver_where(&event("git", Some(payload)), |_| true, cx);
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_what_isnt_for_the_board() {
        assert_eq!(parse(&json!("hi")), None);
        assert_eq!(parse(&json!({ "id": 1, "method": "sessions" })), None);
        assert_eq!(parse(&json!({ "nodus": 2, "loaded": true })), None);
        // A request needs a numeric id and a string method (VIEW-048).
        assert_eq!(parse(&json!({ "nodus": 1, "id": "1", "method": "notes" })), None);
        assert_eq!(parse(&json!({ "nodus": 1, "id": 1, "method": 3 })), None);
    }

    #[test]
    fn sorts_page_messages_in_0_9_4_order() {
        assert_eq!(
            parse(&json!({ "nodus": 1, "error": "boom", "loaded": true })),
            Some(PageMessage::Error("boom".into()))
        );
        assert_eq!(parse(&json!({ "nodus": 1, "loaded": 1 })), Some(PageMessage::Loaded));
        assert_eq!(
            parse(&json!({ "nodus": 1, "loaded": 0, "id": 2, "method": "git" }))
                .map(|m| matches!(m, PageMessage::Request { .. })),
            Some(true)
        );
        let wheel =
            parse(&json!({ "nodus": 1, "wheel": { "dx": 1, "dy": "x", "x": 5, "y": 6, "ctrl": true, "shift": 1 } }));
        assert_eq!(
            wheel,
            Some(PageMessage::Wheel(PageWheel {
                dx: 1.0,
                x: 5.0,
                y: 6.0,
                ctrl: true,
                ..PageWheel::default()
            }))
        );
        assert_eq!(
            parse(&json!({ "nodus": 1, "id": 3, "method": "run", "args": { "name": "builds" } })),
            Some(PageMessage::Request {
                id: json!(3),
                method: "run".into(),
                args: json!({ "name": "builds" })
            })
        );
    }

    #[test]
    fn replies_and_events_as_the_bridge_reads_them() {
        assert_eq!(
            nodus_domain::json::to_string(&reply(&json!(4), Ok(Value::Null))).unwrap(),
            r#"{"nodus":1,"id":4,"result":null}"#
        );
        assert_eq!(
            nodus_domain::json::to_string(&reply(&json!(5), Err("nope".into()))).unwrap(),
            r#"{"nodus":1,"id":5,"error":"nope"}"#
        );
        assert_eq!(
            nodus_domain::json::to_string(&event("board", None)).unwrap(),
            r#"{"nodus":1,"event":"board"}"#
        );
        assert_eq!(
            nodus_domain::json::to_string(&event("data", Some(json!({ "n": 1 })))).unwrap(),
            r#"{"nodus":1,"event":"data","payload":{"n":1}}"#
        );
    }
}
