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
//! {"cmd":"scroll","x":720,"y":400,"dy":-600}       a wheel scroll at a point
//! {"cmd":"files","kind":"entered","x":700,"y":400,"paths":["/…"]}   entered | pending | submit | exited
//! {"cmd":"emit","event":"updateStatus","payload":{"state":"ready","version":"0.9.5","current":"0.9.4"}}
//! {"cmd":"open","project":"storefront"}          a project's board, by name
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
    App, AsyncApp, ExternalPaths, FileDropEvent, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollEvent, TouchPhase, point, px, size,
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
        "scroll" => {
            let position = point(px(num(command, "x")), px(num(command, "y")));
            let delta = point(px(num(command, "dx")), px(num(command, "dy")));
            with_window(cx, move |window, cx| {
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position,
                        delta: ScrollDelta::Pixels(delta),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Moved,
                        source: Default::default(),
                    }),
                    cx,
                );
            })?;
            Ok(Value::Null)
        }
        "files" => {
            // Files from the OS dragged over the window: entered, over, gone.
            let kind = command
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("entered")
                .to_owned();
            let position = point(px(num(command, "x")), px(num(command, "y")));
            let paths: Vec<std::path::PathBuf> = command
                .get("paths")
                .and_then(Value::as_array)
                .map(|p| p.iter().filter_map(Value::as_str).map(Into::into).collect())
                .unwrap_or_default();
            with_window(cx, move |window, cx| {
                let event = match kind.as_str() {
                    "pending" => FileDropEvent::Pending { position },
                    "exited" => FileDropEvent::Exited,
                    "submit" => FileDropEvent::Submit { position },
                    _ => FileDropEvent::Entered {
                        position,
                        paths: ExternalPaths(paths.into()),
                    },
                };
                window.dispatch_event(PlatformInput::FileDrop(event), cx);
            })?;
            Ok(Value::Null)
        }
        "emit" => {
            // An event as a domain would send it (the updater's status, until
            // nodus-updater is wired in).
            let what = command.get("event").and_then(Value::as_str).unwrap_or_default();
            let payload = command.get("payload").cloned().unwrap_or(Value::Null);
            let event = match what {
                "updateStatus" => Event::UpdateStatus(serde_json::from_value(payload).map_err(|e| e.to_string())?),
                "projectsChanged" => Event::ProjectsChanged,
                // A session's status, as the session host reports it.
                "status" => Event::Status {
                    session_id: payload["sessionId"].as_str().unwrap_or_default().to_owned(),
                    status: serde_json::from_value(payload["status"].clone()).map_err(|e| e.to_string())?,
                    live: payload["live"].as_bool().unwrap_or(true),
                },
                other => return Err(format!("unknown event {other}")),
            };
            nodus.emit(event);
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
        "open" => {
            // A project's board by name, as a double-click on its folder opens it.
            let name = command.get("project").and_then(Value::as_str).unwrap_or_default();
            let projects = nodus.projects().list().await;
            let id = projects
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.id.clone())
                .ok_or("no such project")?;
            cx.update(|cx| {
                if let Some(shell) = window::main_shell(cx) {
                    let state = shell.read(cx).state.clone();
                    state.update(cx, |s, cx| s.navigate(crate::state::Route::Board(id), cx));
                }
            });
            Ok(Value::Null)
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
    let mut image = with_window(cx, |window, _| window.render_to_image().map_err(|e| e.to_string()))??;
    // GPUI draws sRGB values and the window shows them as sRGB (its layer
    // is tagged so); the goldens hold what Chromium sent a Display P3
    // screen. Converted, the two compare value for value.
    for pixel in image.pixels_mut() {
        let [r, g, b] = srgb_to_display_p3([pixel[0], pixel[1], pixel[2]]);
        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
    }
    image.save(path).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path, "width": image.width(), "height": image.height() }))
}

/// An sRGB colour in Display P3 (same white point and transfer curve; the
/// primaries differ).
pub fn srgb_to_display_p3(rgb: [u8; 3]) -> [u8; 3] {
    fn linear(v: u8) -> f64 {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    fn encode(v: f64) -> u8 {
        let v = v.clamp(0.0, 1.0);
        let v = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round() as u8
    }
    let [r, g, b] = rgb.map(linear);
    [
        encode(0.822_462_1 * r + 0.177_538 * g),
        encode(0.033_194_1 * r + 0.966_805_8 * g),
        encode(0.017_082_7 * r + 0.072_397_4 * g + 0.910_519_9 * b),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_srgb_to_display_p3_as_chromium_shows_it() {
        // --accent, as the goldens hold it.
        let [r, g, b] = srgb_to_display_p3([0x00, 0x7a, 0xff]);
        assert!(
            r.abs_diff(0x35) <= 1 && g.abs_diff(0x79) <= 1 && b.abs_diff(0xfa) <= 4,
            "{r:x} {g:x} {b:x}"
        );
        // Greys don't move.
        assert_eq!(srgb_to_display_p3([0xf5, 0xf5, 0xf4]), [0xf5, 0xf5, 0xf4]);
    }
}
