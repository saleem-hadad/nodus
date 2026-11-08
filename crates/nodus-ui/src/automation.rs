//! The test driver (decision D11), off unless `NODUS_AUTOMATION` names a
//! Unix socket. It takes one JSON command per line and answers each with one
//! JSON line: `{"ok":true,…}` or `{"ok":false,"error":"…"}`.
//!
//! ```text
//! {"cmd":"resize","width":1440,"height":900}       the window's content size
//! {"cmd":"parity","frozen":true,"now":1792508400000} hold infinite animations, freeze "now"
//! {"cmd":"core","method":"settings.update","args":[{"theme":"dark"}]}
//! {"cmd":"menu","command":"settings"}               a menu command, as the menu sends it
//! {"cmd":"key","keystroke":"cmd-o"}                 a keystroke through GPUI
//! {"cmd":"modifiers","meta":true}                   modifiers held (⌘ for the rail's numbers)
//! {"cmd":"mouse","kind":"click","x":100,"y":200}    move | down | up | click | double | right
//! {"cmd":"wait","ms":300}
//! {"cmd":"idle"}                                    until the window stops drawing
//! {"cmd":"screenshot","path":"/…/scene.light.rust.png"}
//! {"cmd":"window"}                                  the window's number and size
//! {"cmd":"quit"}
//! ```
//!
//! Input is synthetic (`Window::dispatch_event`): it never moves the user's
//! pointer, and the window ignores the real one while driven. Screenshots
//! are the patched GPUI's `Window::render_to_image`: the content only, as
//! the goldens are.

use std::time::Duration;

use gpui::{
    App, AsyncApp, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, point, px, size,
};
use nodus_domain::{Event, GroupPatch, MenuCommand, ProjectPatch, SettingsPatch};
use serde_json::{Value, json};

use crate::parity::Parity;
use crate::shell::window;

type Reply = std::sync::mpsc::Sender<Value>;

/// Starts listening when `NODUS_AUTOMATION` is set.
pub fn start(cx: &mut App) {
    let Some(path) = std::env::var_os("NODUS_AUTOMATION").filter(|p| !p.is_empty()) else {
        return;
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Value, Reply)>();
    std::thread::Builder::new()
        .name("nodus-automation".into())
        .spawn(move || listen(&path, &tx))
        .ok();
    cx.spawn(async move |cx| {
        while let Some((command, reply)) = rx.recv().await {
            let answer = match run(&command, cx).await {
                Ok(value) => {
                    let mut out = json!({ "ok": true });
                    if !value.is_null() {
                        out["result"] = value;
                    }
                    out
                }
                Err(error) => json!({ "ok": false, "error": error }),
            };
            let _ = reply.send(answer);
        }
    })
    .detach();
}

fn listen(path: &std::ffi::OsStr, tx: &tokio::sync::mpsc::UnboundedSender<(Value, Reply)>) {
    #[cfg(unix)]
    {
        use std::io::{BufRead, BufReader, Write};
        let _ = std::fs::remove_file(path);
        let Ok(listener) = std::os::unix::net::UnixListener::bind(path) else {
            return;
        };
        for stream in listener.incoming().flatten() {
            let Ok(mut writer) = stream.try_clone() else { continue };
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let answer = match serde_json::from_str::<Value>(&line) {
                    Ok(command) => {
                        let (reply, answer) = std::sync::mpsc::channel();
                        if tx.send((command, reply)).is_err() {
                            return;
                        }
                        answer
                            .recv()
                            .unwrap_or_else(|_| json!({ "ok": false, "error": "app gone" }))
                    }
                    Err(err) => json!({ "ok": false, "error": err.to_string() }),
                };
                if writeln!(writer, "{answer}").is_err() {
                    break;
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (path, tx);
}

fn num(v: &Value, key: &str) -> f32 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

async fn run(command: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let name = command.get("cmd").and_then(Value::as_str).unwrap_or_default();
    let nodus = cx.update(|cx| cx.global::<window::Core>().0.clone());
    match name {
        "resize" => {
            let (w, h) = (num(command, "width"), num(command, "height"));
            with_window(cx, move |window, _| {
                // A screen smaller than the shot keeps a titled window inside it.
                #[cfg(target_os = "macos")]
                crate::mac::make_borderless(window);
                window.resize(size(px(w), px(h)));
            })?;
            Ok(Value::Null)
        }
        "parity" => {
            let frozen = command.get("frozen").and_then(Value::as_bool).unwrap_or(true);
            let now = command.get("now").and_then(Value::as_f64);
            cx.update(|cx| cx.set_global(Parity { frozen, now_ms: now }));
            with_window(cx, |window, _| window.refresh())?;
            Ok(Value::Null)
        }
        "core" => core(&nodus, command).await,
        "menu" => {
            let command = command.get("command").and_then(Value::as_str).unwrap_or_default();
            let command = MenuCommand::parse(command).ok_or("unknown menu command")?;
            nodus.emit(Event::Menu(command));
            Ok(Value::Null)
        }
        "key" => {
            let spec = command.get("keystroke").and_then(Value::as_str).unwrap_or_default();
            let spec: &'static str = Box::leak(spec.to_owned().into_boxed_str());
            let keystroke = Keystroke::parse(spec).map_err(|e| e.to_string())?;
            with_window(cx, move |window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            })?;
            Ok(Value::Null)
        }
        "modifiers" => {
            let modifiers = Modifiers {
                platform: command.get("meta").and_then(Value::as_bool).unwrap_or(false),
                control: command.get("ctrl").and_then(Value::as_bool).unwrap_or(false),
                alt: command.get("alt").and_then(Value::as_bool).unwrap_or(false),
                shift: command.get("shift").and_then(Value::as_bool).unwrap_or(false),
                function: false,
            };
            with_window(cx, move |window, cx| {
                window.dispatch_event(
                    PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                        modifiers,
                        capslock: Default::default(),
                    }),
                    cx,
                );
            })?;
            Ok(Value::Null)
        }
        "mouse" => {
            let kind = command
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("click")
                .to_owned();
            let position = point(px(num(command, "x")), px(num(command, "y")));
            let modifiers = Modifiers {
                platform: command.get("meta").and_then(Value::as_bool).unwrap_or(false),
                shift: command.get("shift").and_then(Value::as_bool).unwrap_or(false),
                ..Modifiers::default()
            };
            with_window(cx, move |window, cx| {
                let mv = |button| {
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: button,
                        modifiers,
                    })
                };
                let down = |button, count| {
                    PlatformInput::MouseDown(MouseDownEvent {
                        button,
                        position,
                        modifiers,
                        click_count: count,
                        first_mouse: false,
                    })
                };
                let up = |button, count| {
                    PlatformInput::MouseUp(MouseUpEvent {
                        button,
                        position,
                        modifiers,
                        click_count: count,
                    })
                };
                let events = match kind.as_str() {
                    "move" => vec![mv(None)],
                    "down" => vec![mv(None), down(MouseButton::Left, 1)],
                    "up" => vec![up(MouseButton::Left, 1)],
                    "drag" => vec![mv(Some(MouseButton::Left))],
                    "right" => vec![mv(None), down(MouseButton::Right, 1), up(MouseButton::Right, 1)],
                    "double" => vec![
                        mv(None),
                        down(MouseButton::Left, 1),
                        up(MouseButton::Left, 1),
                        down(MouseButton::Left, 2),
                        up(MouseButton::Left, 2),
                    ],
                    _ => vec![mv(None), down(MouseButton::Left, 1), up(MouseButton::Left, 1)],
                };
                for event in events {
                    window.dispatch_event(event, cx);
                }
            })?;
            Ok(Value::Null)
        }
        "wait" => {
            let ms = command.get("ms").and_then(Value::as_u64).unwrap_or(100);
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            Ok(Value::Null)
        }
        "idle" => {
            // Every finite animation here is shorter than this.
            cx.background_executor().timer(Duration::from_millis(900)).await;
            Ok(Value::Null)
        }
        "window" => {
            let info = with_window(cx, |window, _| {
                let size = window.viewport_size();
                #[cfg(target_os = "macos")]
                let number = crate::mac::window_number(window);
                #[cfg(not(target_os = "macos"))]
                let number: Option<isize> = None;
                json!({ "number": number, "width": f32::from(size.width), "height": f32::from(size.height), "scale": window.scale_factor() })
            })?;
            Ok(info)
        }
        "screenshot" => {
            let path = command
                .get("path")
                .and_then(Value::as_str)
                .ok_or("screenshot needs a path")?
                .to_owned();
            screenshot(cx, &path).await
        }
        "quit" => {
            cx.update(|cx| cx.quit());
            Ok(Value::Null)
        }
        other => Err(format!("unknown command {other}")),
    }
}

fn with_window<R>(cx: &mut AsyncApp, f: impl FnOnce(&mut gpui::Window, &mut App) -> R) -> Result<R, String> {
    cx.update(|cx| {
        let handle = window::main_window(cx).ok_or("no main window")?;
        // Through the window, not its root view: input dispatched here
        // reaches views that update the root themselves.
        gpui::AppContext::update_window(cx, handle.into(), |_, window, cx| f(window, cx)).map_err(|e| e.to_string())
    })
}

/// Saves the window's last frame as a PNG, at the drawable's size (device
/// pixels), drawn again offscreen by the patched GPUI: what GPUI draws and
/// nothing else, so no traffic lights, as the goldens have none.
async fn screenshot(cx: &mut AsyncApp, path: &str) -> Result<Value, String> {
    // Let the frame in flight land first.
    cx.background_executor().timer(Duration::from_millis(50)).await;
    let image = with_window(cx, |window, _| window.render_to_image().map_err(|e| e.to_string()))??;
    image.save(path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path, "width": image.width(), "height": image.height() }))
}

/// Calls a core command by its `window.api` name.
async fn core(nodus: &nodus_core::Nodus, command: &Value) -> Result<Value, String> {
    let method = command.get("method").and_then(Value::as_str).unwrap_or_default();
    let args = command
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let text = |i: usize| arg(i).as_str().unwrap_or_default().to_owned();
    let err = |e: nodus_core::CommandError| e.ipc_message();
    match method {
        "settings.get" => serde_json::to_value(nodus.settings().get().await).map_err(|e| e.to_string()),
        "settings.update" => {
            let patch: SettingsPatch = serde_json::from_value(arg(0)).map_err(|e| e.to_string())?;
            serde_json::to_value(nodus.settings().update(patch).await).map_err(|e| e.to_string())
        }
        "projects.list" => serde_json::to_value(nodus.projects().list().await).map_err(|e| e.to_string()),
        "projects.add" => {
            let path = arg(0).as_str().map(str::to_owned);
            serde_json::to_value(nodus.projects().add(path).await.map_err(err)?).map_err(|e| e.to_string())
        }
        "projects.update" => {
            let patch: ProjectPatch = serde_json::from_value(arg(1)).map_err(|e| e.to_string())?;
            serde_json::to_value(nodus.projects().update(&text(0), patch).await.map_err(err)?)
                .map_err(|e| e.to_string())
        }
        "projects.remove" => {
            nodus.projects().remove(&text(0)).await;
            Ok(Value::Null)
        }
        "groups.list" => serde_json::to_value(nodus.groups().list().await).map_err(|e| e.to_string()),
        "groups.create" => serde_json::to_value(nodus.groups().create(&text(0)).await).map_err(|e| e.to_string()),
        "groups.update" => {
            let patch: GroupPatch = serde_json::from_value(arg(1)).map_err(|e| e.to_string())?;
            serde_json::to_value(nodus.groups().update(&text(0), patch).await.map_err(err)?).map_err(|e| e.to_string())
        }
        "groups.remove" => {
            nodus.groups().remove(&text(0)).await;
            Ok(Value::Null)
        }
        "menuCommand" => {
            nodus.app().menu_command(&text(0)).await;
            Ok(Value::Null)
        }
        other => Err(format!("unknown core method {other}")),
    }
}
