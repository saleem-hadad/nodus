//! The test driver (decision D11), off unless `NODUS_AUTOMATION` names a
//! Unix socket (a named pipe on Windows). It takes one JSON command per line and answers each with one
//! JSON line: `{"ok":true,…}` or `{"ok":false,"error":"…"}`.
//!
//! ```text
//! {"cmd":"resize","width":1440,"height":900}       the window's content size
//! {"cmd":"parity","frozen":true,"now":1792508400000} hold infinite animations, freeze "now"
//! {"cmd":"core","method":"settings.update","args":[{"theme":"dark"}]}
//! {"cmd":"menu","command":"settings"}               a menu command, as the menu sends it
//! {"cmd":"action","name":"nodus::ZoomIn"}            a GPUI action by name (a menu item's)
//! {"cmd":"key","keystroke":"cmd-o"}                 a keystroke through GPUI
//! {"cmd":"type","text":"Hello"}                    text typed into what has the keyboard
//! {"cmd":"modifiers","meta":true}                   modifiers held (⌘ for the rail's numbers)
//! {"cmd":"mouse","kind":"click","x":100,"y":200}    move | down | up | click | double | right
//! {"cmd":"scroll","x":720,"y":400,"dy":-600}       a trackpad scroll at a point
//! {"cmd":"scroll","x":720,"y":400,"dy":-1,"lines":true,"meta":true}   a mouse wheel's notch, ⌘ held
//! {"cmd":"pinch","x":720,"y":400,"delta":0.02}     a trackpad pinch step (+ zooms in)
//! {"cmd":"files","kind":"entered","x":700,"y":400,"paths":["/…"]}   entered | pending | submit | exited
//! {"cmd":"emit","event":"updateStatus","payload":{"state":"ready","version":"0.9.5","current":"0.9.4"}}
//! {"cmd":"connect","url":"http://127.0.0.1:7420","token":"…","link":["shop"]}
//!                                                  adds a server (SRV-060) and links its projects by
//!                                                  name or id ("link":"all" for every one); gives
//!                                                  {server, linked}
//! {"cmd":"open","project":"storefront"}          a project's board, by name
//! {"cmd":"board","project":"storefront"}         that board as the core has it (cards, notes, ink, views)
//! {"cmd":"pick","label":"Restore to board"}       the open context menu's row, chosen
//! {"cmd":"wait","ms":300}
//! {"cmd":"idle"}                                    until the window stops drawing
//! {"cmd":"screenshot","path":"/…/scene.light.rust.png"}
//! {"cmd":"window"}                                  the window's number and size
//! {"cmd":"stub","name":"summarize","phases":["done",null],"results":[{…}]}   what summarize answers (spec 11)
//! {"cmd":"stub","name":"pickFolder","path":"/…"}   the folder picker answers this at once
//! {"cmd":"terminal","action":"open",…}                the terminal's windows (terminal::driver)
//! {"cmd":"views"}                                   views' frames: live or snapshot, and why
//! {"cmd":"native-click","x":500,"y":300}            a click AppKit routes (into a page, or GPUI);
//!                                                   "button": "right" or "control" (⌃-click)
//! {"cmd":"hit","x":500,"y":300}                     the native view a click there reaches
//! {"cmd":"responder"}                               the class of the window's first responder
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
    // A driven app is mostly covered or off screen: a user-initiated
    // activity keeps App Nap from holding its timers back, and the Mac from
    // idling to sleep under a long run. (A Mac already asleep, with only
    // dark wakes, still needs `caffeinate -s`.)
    let awake = cx.prevent_idle_sleep("Nodus is driven by its test driver");
    cx.spawn(async move |_| {
        if let Ok(guard) = awake.await {
            std::mem::forget(guard);
        }
    })
    .detach();
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

/// Takes clients one at a time: a Unix socket at `path`, or on Windows the
/// named pipe `path` names (`\\.\pipe\<name>`, as `packaging/smoke.py`
/// opens it).
fn listen(path: &std::ffi::OsStr, tx: &tokio::sync::mpsc::UnboundedSender<(Value, Reply)>) {
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(path);
        let Ok(listener) = std::os::unix::net::UnixListener::bind(path) else {
            return;
        };
        for stream in listener.incoming().flatten() {
            let Ok(writer) = stream.try_clone() else { continue };
            if !serve(stream, writer, tx) {
                return;
            }
        }
    }
    #[cfg(windows)]
    for pipe in nodus_platform::pipe::serve(&path.to_string_lossy()) {
        let Ok(pipe) = pipe else { continue };
        let Ok(writer) = pipe.try_clone() else { continue };
        if !serve(pipe, writer, tx) {
            return;
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (path, tx);
}

/// Answers one client's commands, a JSON line for each line, until it goes.
/// False once the app has gone.
#[cfg(any(unix, windows))]
fn serve(
    reader: impl std::io::Read,
    mut writer: impl std::io::Write,
    tx: &tokio::sync::mpsc::UnboundedSender<(Value, Reply)>,
) -> bool {
    use std::io::BufRead as _;
    for line in std::io::BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(command) => {
                let (reply, answer) = std::sync::mpsc::channel();
                if tx.send((command, reply)).is_err() {
                    return false;
                }
                answer
                    .recv()
                    .unwrap_or_else(|_| json!({ "ok": false, "error": "app gone" }))
            }
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        };
        if writeln!(writer, "{answer}").and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
    true
}

fn num(v: &Value, key: &str) -> f32 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

async fn run(command: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let name = command.get("cmd").and_then(Value::as_str).unwrap_or_default();
    let nodus = cx.update(|cx| cx.global::<window::Core>().0.clone());
    // `"window": "capture"` sends input to, and shoots, the capture panel.
    let panel = command.get("window").and_then(Value::as_str) == Some("capture");
    cx.update(|cx| cx.set_global(Target { panel }));
    if let Some(answer) = crate::capture::automation::run(name, command, &nodus, cx).await {
        return answer;
    }
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
        // As 0.9.4's scenes replaced `projects:add`: the folder picker
        // answers this path at once instead of showing (projects-toast).
        "stub" if command.get("name").and_then(Value::as_str) == Some("pickFolder") => {
            let path = command
                .get("path")
                .and_then(Value::as_str)
                .map(std::path::PathBuf::from);
            cx.update(|cx| cx.set_global(crate::app::PickFolderStub(path)));
            Ok(Value::Null)
        }
        // As 0.9.4's scenes replaced an IPC handler: what the merge dialog's
        // summarize answers (each source's phase; results, or none: pending).
        "stub" => {
            let phases = command
                .get("phases")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|p| p.as_str().and_then(nodus_domain::SummarizePhase::parse))
                        .collect()
                })
                .unwrap_or_default();
            let results = command
                .get("results")
                .map(|r| serde_json::from_value(r.clone()).map_err(|e| e.to_string()))
                .transpose()?;
            cx.update(|cx| {
                cx.set_global(crate::board::overlays::merge::SummarizeStub { phases, results });
            });
            Ok(Value::Null)
        }
        "core" => core(&nodus, command).await,
        // A GPUI action by name, as its menu item sends it
        // (`{"cmd":"action","name":"nodus::DeleteSelection"}`).
        "action" => {
            let name = command
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let action = cx
                .update(|cx| cx.build_action(&name, None))
                .map_err(|e| e.to_string())?;
            with_window(cx, move |window, cx| window.dispatch_action(action, cx))?;
            Ok(Value::Null)
        }
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
        "type" => {
            // Text typed into whatever has the keyboard, a character at a time.
            let text = command
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            with_window(cx, move |window, cx| {
                for ch in text.chars() {
                    let s = ch.to_string();
                    let key = if ch == ' ' {
                        "space".to_owned()
                    } else {
                        s.to_lowercase()
                    };
                    window.dispatch_keystroke(
                        Keystroke {
                            modifiers: Modifiers {
                                shift: ch.is_uppercase(),
                                ..Modifiers::default()
                            },
                            key,
                            key_char: Some(s),
                        },
                        cx,
                    );
                }
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
            let (dx, dy) = (num(command, "dx"), num(command, "dy"));
            let flag = |key: &str| command.get(key).and_then(Value::as_bool).unwrap_or(false);
            let delta = if flag("lines") {
                ScrollDelta::Lines(point(dx, dy))
            } else {
                ScrollDelta::Pixels(point(px(dx), px(dy)))
            };
            let modifiers = Modifiers {
                platform: flag("meta"),
                control: flag("ctrl"),
                alt: flag("alt"),
                shift: flag("shift"),
                function: false,
            };
            with_window(cx, move |window, cx| {
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position,
                        delta,
                        modifiers,
                        touch_phase: TouchPhase::Moved,
                        source: Default::default(),
                    }),
                    cx,
                );
            })?;
            Ok(Value::Null)
        }
        "pinch" => {
            let position = point(px(num(command, "x")), px(num(command, "y")));
            let delta = num(command, "delta");
            with_window(cx, move |window, cx| {
                window.dispatch_event(
                    PlatformInput::Pinch(gpui::PinchEvent {
                        position,
                        delta,
                        modifiers: Modifiers::default(),
                        phase: TouchPhase::Moved,
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
                // A session's status line, as Claude reports it (board-expanded).
                "usage" => Event::Usage {
                    session_id: payload["sessionId"].as_str().unwrap_or_default().to_owned(),
                    usage: serde_json::from_value(payload["usage"].clone()).map_err(|e| e.to_string())?,
                },
                "captureContext" => Event::CaptureContext(crate::capture::automation::context_patch(&payload)?),
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
        "connect" => connect(&nodus, command, cx).await,
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
        "terminal" => crate::terminal::driver::run(command, cx).await,
        // The open context menu's row with this label, chosen as a click
        // chooses it (closes, then runs); the labels when there's none.
        "pick" => {
            let want = command
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            with_window(cx, move |window, cx| {
                let shell = window::main_shell(cx).ok_or("no main window")?;
                let layer = shell.read(cx).menus.clone();
                let items = layer
                    .read(cx)
                    .menu
                    .as_ref()
                    .map(|m| m.items.clone())
                    .unwrap_or_default();
                let mut labels = Vec::new();
                for item in items {
                    if let crate::components::context_menu::MenuItem::Item {
                        label,
                        disabled,
                        on_select,
                        ..
                    } = item
                    {
                        if label.as_ref() == want && !disabled {
                            layer.update(cx, |l, cx| l.close(window, cx));
                            on_select(window, cx);
                            return Ok(Value::Null);
                        }
                        labels.push(label.to_string());
                    }
                }
                Err(format!("no item {want:?} in {labels:?}"))
            })?
        }
        // A project's board as the core has it now (by name or id).
        "board" => {
            let key = command.get("project").and_then(Value::as_str).unwrap_or_default();
            let projects = nodus.projects().list().await;
            let id = projects
                .iter()
                .find(|p| p.name == key || p.id == key)
                .map(|p| p.id.clone())
                .ok_or("no such project")?;
            let board = nodus.board().load(&id).await.map_err(|e| e.ipc_message())?;
            let mut out = serde_json::to_value(board).map_err(|e| e.to_string())?;
            // Where the board is on screen now (its saved viewport lags).
            let live = cx.update(|cx| {
                let shell = window::main_shell(cx)?;
                let open = shell.read(cx).state.read(cx).route.project_id() == Some(id.as_str());
                let canvas = shell.read(cx).board()?.read(cx);
                open.then(|| serde_json::to_value(canvas.viewport).ok()).flatten()
            });
            out["liveViewport"] = live.unwrap_or(Value::Null);
            Ok(out)
        }
        // The menu bar as AppKit shows it, its own additions included.
        "menus" => {
            #[cfg(target_os = "macos")]
            // Outside an update: AppKit asks the app to validate the items.
            let menus = crate::mac::menu_bar();
            #[cfg(not(target_os = "macos"))]
            let menus = Value::Null;
            Ok(menus)
        }
        // Views' frames: live or behind a snapshot, and why (spec 04).
        "views" => Ok(cx.update(|cx| crate::views::frames::describe(cx))),
        // A click AppKit routes itself (into a page's webview, or GPUI), and
        // which view a click at a point would reach.
        #[cfg(target_os = "macos")]
        "native-click" => {
            let (x, y) = (f64::from(num(command, "x")), f64::from(num(command, "y")));
            let number = with_window(cx, |window, _| crate::mac::window_number(window))?.ok_or("no window")?;
            let button = command.get("button").and_then(Value::as_str).unwrap_or("left");
            crate::views::native_click(number, x, y, button == "right", button == "control");
            Ok(Value::Null)
        }
        // Keys typed as AppKit delivers them (the platform's path, not GPUI's
        // dispatch): lowercase letters and spaces.
        #[cfg(target_os = "macos")]
        "native-keys" => {
            let text = command
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let number = with_window(cx, |window, _| crate::mac::window_number(window))?.ok_or("no window")?;
            crate::views::native_keys(number, text);
            Ok(Value::Null)
        }
        #[cfg(target_os = "macos")]
        "responder" => Ok(Value::String(with_window(cx, |window, _| {
            crate::views::responder_class(window)
        })?)),
        #[cfg(target_os = "macos")]
        "hit" => {
            let (x, y) = (f64::from(num(command, "x")), f64::from(num(command, "y")));
            Ok(Value::String(with_window(cx, move |window, _| {
                crate::views::hit_class(window, x, y)
            })?))
        }
        "quit" => {
            cx.update(|cx| cx.quit());
            Ok(Value::Null)
        }
        other => Err(format!("unknown command {other}")),
    }
}

/// Which window a command is for: the main one, or the capture panel.
#[derive(Default)]
struct Target {
    panel: bool,
}

impl gpui::Global for Target {}

fn with_window<R>(cx: &mut AsyncApp, f: impl FnOnce(&mut gpui::Window, &mut App) -> R) -> Result<R, String> {
    cx.update(|cx| {
        if cx.try_global::<Target>().is_some_and(|t| t.panel) {
            let handle = crate::capture::PanelSlot::live(cx).ok_or("no capture panel")?;
            return gpui::AppContext::update_window(cx, handle.into(), |_, window, cx| f(window, cx))
                .map_err(|e| e.to_string());
        }
        let handle = window::main_window(cx).ok_or("no main window")?;
        // Through the window, not its root view: input dispatched here
        // reaches views that update the root themselves.
        gpui::AppContext::update_window(cx, handle.into(), |_, window, cx| {
            // A driven window is often covered by others, and GPUI stops
            // drawing a covered window: draw the current state first, so
            // input hits what's there now and a shot shows it, running what
            // the last frames left for the next one (a field taking the
            // focus two frames on), as those frames would.
            for _ in 0..2 {
                window.run_next_frame_callbacks(cx);
                window.draw(cx).clear(cx);
            }
            f(window, cx)
        })
        .map_err(|e| e.to_string())
    })
}

/// Saves the window's last frame as a PNG, at the drawable's size (device
/// pixels), drawn again offscreen by the patched GPUI: what GPUI draws and
/// nothing else, so no traffic lights, as the goldens have none.
async fn screenshot(cx: &mut AsyncApp, path: &str) -> Result<Value, String> {
    // Views' pages are WebKit's, which can be slow to start: pages that
    // should show as pictures get theirs first (up to 5s), or the shot holds
    // their empty bodies.
    for _ in 0..100 {
        if cx.update(|cx| crate::views::swap::pictures_ready(cx)) {
            break;
        }
        cx.background_executor().timer(Duration::from_millis(50)).await;
    }
    // Let the frame in flight land first.
    cx.background_executor().timer(Duration::from_millis(50)).await;
    let mut image = with_window(cx, |window, _| window.render_to_image().map_err(|e| e.to_string()))??;
    // GPUI draws sRGB values and the window shows them as sRGB (the patched
    // GPUI tags its layer so); the goldens hold what Chromium sent a Display P3
    // screen. Converted, the two compare value for value.
    for pixel in image.pixels_mut() {
        // A see-through window (the capture panel) reads back premultiplied;
        // the goldens' PNGs are straight alpha. Opaque pixels don't change.
        let a = pixel[3];
        if a > 0 && a < 255 {
            for c in 0..3 {
                pixel[c] = ((u16::from(pixel[c]) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8;
            }
        }
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
/// `connect`: adds the server as Connect to Server… does, then links the
/// projects asked for, by name or id, or all of them.
async fn connect(nodus: &nodus_core::Nodus, command: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let field = |key: &str| command.get(key).and_then(Value::as_str).unwrap_or_default();
    let server = nodus
        .servers()
        .add(field("url"), field("token"))
        .await
        .map_err(|e| e.message)?;
    // Its projects can be asked for once it's connected.
    for _ in 0..150 {
        if nodus.servers().status(&server.id).is_some_and(|s| s.state == nodus_domain::ServerState::Connected) {
            break;
        }
        cx.background_executor().timer(Duration::from_millis(100)).await;
    }
    let wanted: Option<Vec<String>> = match command.get("link") {
        Some(Value::String(all)) if all == "all" => None,
        Some(Value::Array(names)) => Some(names.iter().filter_map(Value::as_str).map(str::to_owned).collect()),
        _ => Some(Vec::new()),
    };
    let mut linked = Vec::new();
    if wanted.as_ref().is_none_or(|w| !w.is_empty()) {
        let theirs = nodus.servers().projects(&server.id).await.map_err(|e| e.message)?;
        for project in theirs {
            let pick = wanted
                .as_ref()
                .is_none_or(|w| w.iter().any(|n| *n == project.name || *n == project.id));
            if pick {
                let project = nodus
                    .servers()
                    .link(&server.id, &project.id)
                    .await
                    .map_err(|e| e.message)?;
                linked.push(project);
            }
        }
    }
    Ok(serde_json::json!({ "server": server, "linked": linked }))
}

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
        "projects.stats" => serde_json::to_value(nodus.projects().stats().await).map_err(|e| e.to_string()),
        "sessions.create" => {
            let opts: nodus_domain::CreateSessionOptions = serde_json::from_value(arg(1)).map_err(|e| e.to_string())?;
            serde_json::to_value(nodus.sessions().create(&text(0), &opts).await.map_err(err)?)
                .map_err(|e| e.to_string())
        }
        "sessions.kill" => {
            nodus.sessions().kill(&text(0)).await;
            Ok(Value::Null)
        }
        "updates.status" => serde_json::to_value(nodus.updates().status().await).map_err(|e| e.to_string()),
        "voice.keySource" => serde_json::to_value(nodus.voice().key_source().await).map_err(|e| e.to_string()),
        "servers.list" => serde_json::to_value(nodus.servers().list().await).map_err(|e| e.to_string()),
        "servers.projects" => {
            serde_json::to_value(nodus.servers().projects(&text(0)).await.map_err(err)?).map_err(|e| e.to_string())
        }
        "servers.link" => serde_json::to_value(nodus.servers().link(&text(0), &text(1)).await.map_err(err)?)
            .map_err(|e| e.to_string()),
        "servers.unlink" => {
            nodus.servers().unlink(&text(0)).await.map_err(err)?;
            Ok(Value::Null)
        }
        "servers.remove" => {
            nodus.servers().remove(&text(0)).await;
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
