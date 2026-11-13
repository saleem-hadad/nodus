//! The chat's visuals and forms (spec 16), each drawn in the light and dark
//! themes at scale 1 and 0.75, and saved as PNGs to look over.
//!
//! ```sh
//! cargo run -p nodus-ui --example chat_visuals -- <out dir> [scene…]
//! ```
//!
//! The window is a non-activating panel that never takes the focus, and the
//! pointer and keys are simulated: the hover scene rests the pointer on a
//! chart, and the form flow picks choices, writes a note and presses Send,
//! printing the answers it sent (`SENT {...}`). Each PNG is cropped to what
//! was drawn: `<scene>.<theme>.<scale>.png`.
#![allow(clippy::print_stdout, reason = "an example reports on stdout")]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;
use nodus_domain::ThemePreference;
use nodus_ui::chat::form::FormView;
use nodus_ui::chat::visual::visual_element;
use nodus_ui::components::icon::Assets;
use nodus_ui::theme;
use nodus_visual::{Visual, parse_form, parse_visual};
use serde_json::{Value, json};

const W: f32 = 680.;
const H: f32 = 1000.;
/// The card's width at scale 1, as in a chat column.
const CARD_W: f32 = 600.;
const PAD: f32 = 24.;

/// What a scene shows.
#[derive(Clone)]
enum Show {
    Visual(Rc<Visual>),
    /// A form; answered or not.
    Form(bool),
}

/// What the simulated hand does in a scene, in px from the card's top
/// left at scale 1.
#[derive(Clone, Copy)]
enum Step {
    /// Rests the pointer there.
    Point(f32, f32),
    /// Presses and lets go there.
    Click(f32, f32),
    /// Types into what has the focus.
    Type(&'static str),
    /// Saves a picture, named after the scene and this.
    Shot(&'static str),
}

#[derive(Clone)]
struct Scene {
    name: &'static str,
    show: Show,
    /// Nothing but a picture at the end, unless it says otherwise.
    steps: Vec<Step>,
}

fn visual(spec: &Value) -> Rc<Visual> {
    Rc::new(parse_visual(spec).unwrap_or_else(|e| panic!("{e}")))
}

fn scenes() -> Vec<Scene> {
    let grouped = json!({
        "title": "Crash-free sessions by platform",
        "mark": "bar",
        "data": {"values": [
            {"q": "Q1", "platform": "iOS", "rate": 97.2}, {"q": "Q1", "platform": "Android", "rate": 95.1}, {"q": "Q1", "platform": "Web", "rate": 98.4},
            {"q": "Q2", "platform": "iOS", "rate": 98.1}, {"q": "Q2", "platform": "Android", "rate": 96.0}, {"q": "Q2", "platform": "Web", "rate": 98.9},
            {"q": "Q3", "platform": "iOS", "rate": 98.6}, {"q": "Q3", "platform": "Android", "rate": 96.8}, {"q": "Q3", "platform": "Web", "rate": 99.1},
            {"q": "Q4", "platform": "iOS", "rate": 99.0}, {"q": "Q4", "platform": "Android", "rate": 97.5}, {"q": "Q4", "platform": "Web", "rate": 99.3}
        ]},
        "encoding": {
            "x": {"field": "q", "type": "nominal", "title": "Quarter"},
            "y": {"field": "rate", "type": "quantitative", "title": "Crash-free %", "scale": {"domain": [90, 100]}},
            "xOffset": {"field": "platform"},
            "color": {"field": "platform"}
        }
    });
    let stacked = json!({
        "title": "Build minutes by runner",
        "mark": "bar",
        "data": {"values": [
            {"month": "Jun", "runner": "macOS", "min": 4200}, {"month": "Jun", "runner": "Linux", "min": 8100}, {"month": "Jun", "runner": "Windows", "min": 1900},
            {"month": "Jul", "runner": "macOS", "min": 4800}, {"month": "Jul", "runner": "Linux", "min": 9300}, {"month": "Jul", "runner": "Windows", "min": 2100},
            {"month": "Aug", "runner": "macOS", "min": 5100}, {"month": "Aug", "runner": "Linux", "min": 8800}, {"month": "Aug", "runner": "Windows", "min": 2600},
            {"month": "Sep", "runner": "macOS", "min": 6100}, {"month": "Sep", "runner": "Linux", "min": 10400}, {"month": "Sep", "runner": "Windows", "min": 2400},
            {"month": "Oct", "runner": "macOS", "min": 6900}, {"month": "Oct", "runner": "Linux", "min": 11800}, {"month": "Oct", "runner": "Windows", "min": 3100},
            {"month": "Nov", "runner": "macOS", "min": 5600}, {"month": "Nov", "runner": "Linux", "min": 9900}, {"month": "Nov", "runner": "Windows", "min": 2800}
        ]},
        "encoding": {
            "x": {"field": "month", "type": "ordinal", "axis": {"title": null}},
            "y": {"field": "min", "type": "quantitative", "title": "Minutes"},
            "color": {"field": "runner"}
        }
    });
    let horizontal = json!({
        "title": "Slowest test files",
        "mark": "bar",
        "data": {"values": [
            {"file": "checkout/payment.spec.ts", "s": 48.2},
            {"file": "search/results.spec.ts", "s": 31.7},
            {"file": "auth/session-refresh.spec.ts", "s": 27.9},
            {"file": "cart/store.spec.ts", "s": 19.4},
            {"file": "profile/avatar-upload.spec.ts", "s": 12.1},
            {"file": "home.spec.ts", "s": 6.3}
        ]},
        "encoding": {
            "y": {"field": "file", "type": "nominal", "sort": "-x", "axis": {"title": null}},
            "x": {"field": "s", "type": "quantitative", "title": "Seconds"}
        }
    });
    let days: Vec<Value> = (1..=14)
        .flat_map(|d| {
            let p50 = 120.0 + (d as f64 * 0.9).sin() * 14.0 + d as f64 * 1.5;
            let p95 = 310.0 + (d as f64 * 0.7).cos() * 40.0 + d as f64 * 6.0;
            [
                json!({"day": format!("2026-10-{d:02}"), "pct": "p50", "ms": p50.round()}),
                json!({"day": format!("2026-10-{d:02}"), "pct": "p95", "ms": p95.round()}),
            ]
        })
        .collect();
    let line = json!({
        "title": "API latency",
        "mark": {"type": "line", "point": true},
        "data": {"values": days},
        "encoding": {
            "x": {"field": "day", "type": "temporal", "axis": {"title": null}},
            "y": {"field": "ms", "type": "quantitative", "title": "ms"},
            "color": {"field": "pct", "title": "Percentile"}
        }
    });
    let hours: Vec<Value> = (0..48)
        .map(|h| {
            let v = 1800.0
                + 900.0 * ((h as f64 - 14.0) / 24.0 * std::f64::consts::TAU).sin().max(-0.7)
                + (h % 5) as f64 * 40.0;
            json!({"t": format!("2026-10-{:02}T{:02}:00", 3 + h / 24, h % 24), "rps": v.round()})
        })
        .collect();
    let area = json!({
        "title": "Requests per second",
        "mark": "area",
        "height": 180,
        "data": {"values": hours},
        "encoding": {
            "x": {"field": "t", "type": "temporal", "axis": {"title": null}},
            "y": {"field": "rps", "type": "quantitative", "axis": {"title": null}}
        }
    });
    let donut = json!({
        "title": "Where the bundle's weight goes",
        "mark": {"type": "arc", "innerRadius": 56},
        "data": {"values": [
            {"part": "React + DOM", "kb": 132}, {"part": "Charts", "kb": 88}, {"part": "Date utils", "kb": 41},
            {"part": "App code", "kb": 214}, {"part": "Icons", "kb": 27}
        ]},
        "encoding": {
            "theta": {"field": "kb", "type": "quantitative", "title": "KB"},
            "color": {"field": "part", "type": "nominal"}
        }
    });
    let composition = json!({
        "title": "Release 1.0.12 at a glance",
        "rows": [
            {"stats": [
                {"label": "Downloads", "value": 18342, "note": "+12% on 1.0.11", "tone": "good"},
                {"label": "Crash-free", "value": "99.3%", "note": "+0.4 pts", "tone": "good"},
                {"label": "Open issues", "value": 27, "note": "6 new this week", "tone": "bad"},
                {"label": "Median start", "value": "412 ms"}
            ]},
            {"title": "Top crashes", "table": {"rows": [
                {"signature": "TerminalElement::paint", "count": 41, "first seen": "Oct 28", "status": "Fixed in 1.0.13"},
                {"signature": "BoardCanvas::drop_gesture", "count": 17, "first seen": "Nov 2", "status": "Investigating"},
                {"signature": "wry::WebView::snapshot", "count": 9, "first seen": "Nov 9", "status": "Needs repro"}
            ]}}
        ]
    });
    let columns = json!({
        "title": "Test suite",
        "rows": [
            {"columns": [
                {"chart": {
                    "title": "Pass rate by suite",
                    "mark": "bar",
                    "height": 160,
                    "data": {"values": [
                        {"suite": "unit", "pass": 99.6}, {"suite": "integration", "pass": 97.1},
                        {"suite": "e2e", "pass": 91.4}, {"suite": "visual", "pass": 95.8}
                    ]},
                    "encoding": {"x": {"field": "suite", "type": "nominal", "axis": {"title": null}},
                                  "y": {"field": "pass", "type": "quantitative", "title": "%", "scale": {"domain": [80, 100]}}}
                }},
                {"text": "### What changed\n\nThe **e2e** suite lost 4 points after the checkout rewrite: two specs wait on a spinner that no longer exists.\n\n- Fix the selectors in `checkout/*.spec.ts`\n- Re-record the visual baselines"}
            ]}
        ]
    });
    vec![
        Scene {
            name: "grouped-bars",
            show: Show::Visual(visual(&grouped)),
            steps: Vec::new(),
        },
        Scene {
            name: "stacked-bars",
            show: Show::Visual(visual(&stacked)),
            steps: Vec::new(),
        },
        Scene {
            name: "horizontal-bars",
            show: Show::Visual(visual(&horizontal)),
            steps: Vec::new(),
        },
        Scene {
            name: "line",
            show: Show::Visual(visual(&line)),
            steps: Vec::new(),
        },
        Scene {
            name: "line-hover",
            show: Show::Visual(visual(&line)),
            steps: vec![Step::Point(330., 160.)],
        },
        Scene {
            name: "area",
            show: Show::Visual(visual(&area)),
            steps: Vec::new(),
        },
        Scene {
            name: "donut",
            show: Show::Visual(visual(&donut)),
            steps: Vec::new(),
        },
        Scene {
            name: "stats-table",
            show: Show::Visual(visual(&composition)),
            steps: Vec::new(),
        },
        Scene {
            name: "columns",
            show: Show::Visual(visual(&columns)),
            steps: Vec::new(),
        },
        Scene {
            name: "form",
            show: Show::Form(false),
            steps: Vec::new(),
        },
        // FORM-003, FORM-010, FORM-012: picking, unpicking, a note, Send.
        Scene {
            name: "form-flow",
            show: Show::Form(false),
            steps: vec![
                Step::Click(294., 191.),
                Step::Click(294., 401.),
                Step::Click(294., 528.),
                Step::Click(294., 528.),
                Step::Click(294., 528.),
                Step::Click(294., 323.),
                Step::Type("Keep search on top"),
                Step::Shot("picked"),
                Step::Click(521., 847.),
                Step::Shot("sent"),
            ],
        },
        Scene {
            name: "form-answered",
            show: Show::Form(true),
            steps: Vec::new(),
        },
    ]
}

fn form_spec() -> Value {
    json!({
        "title": "Before I restructure the settings screen",
        "submit": "Send to Claude",
        "questions": [
            {"tag": "Q1", "title": "How should the sections be laid out?", "name": "layout",
             "detail": "There are seven sections today; two of them are rarely opened.",
             "choices": [
                {"label": "A sidebar of sections", "detail": "Like System Settings: one section at a time", "recommended": true},
                {"label": "One long page", "detail": "Everything visible, with headings to jump to"},
                {"label": "Tabs across the top"}
             ]},
            {"tag": "Q2", "title": "Which settings should move to Advanced?", "multiple": true,
             "choices": ["Session host", "Telemetry", "Voice model", "Experimental board tools"]},
            {"tag": "Q3", "title": "Anything the new screen must keep?", "type": "text",
             "placeholder": "A shortcut, a default, a habit…"}
        ]
    })
}

fn answered() -> Value {
    json!({
        "layout": "A sidebar of sections",
        "layout (note)": "Keep the search field at the top",
        "Which settings should move to Advanced?": ["Session host", "Experimental board tools"],
        "Anything the new screen must keep?": "⌘, still opens it, on the last section I had open.",
        "Additional notes": "No rush on this one."
    })
}

struct Gallery {
    scene: Rc<RefCell<(Scene, f32)>>,
    forms: Vec<(bool, f32, Entity<FormView>)>,
    /// The form the flow scene works, made anew each time it starts.
    flow: Option<Entity<FormView>>,
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let (scene, scale) = self.scene.borrow().clone();
        let content: AnyElement = match &scene.show {
            Show::Visual(v) => visual_element(scene.name, v.clone(), scale, cx),
            Show::Form(_) if scene.name == "form-flow" => self
                .flow
                .clone()
                .map_or_else(|| div().into_any_element(), IntoElement::into_any_element),
            Show::Form(done) => self
                .forms
                .iter()
                .find(|(d, s, _)| d == done && *s == scale)
                .map_or_else(|| div().into_any_element(), |(_, _, f)| f.clone().into_any_element()),
        };
        div()
            .size_full()
            .bg(t.bg_window)
            .p(px(PAD))
            .child(div().w(px(CARD_W * scale)).child(content))
    }
}

fn in_window<R>(
    cx: &mut AsyncApp,
    handle: WindowHandle<Gallery>,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R> {
    cx.update(|cx| cx.update_window(handle.into(), |_, window, cx| f(window, cx)))
}

/// Draws a frame now, even while the window is covered.
fn draw(window: &mut Window, cx: &mut App) {
    window.refresh();
    window.draw(cx).clear(cx);
}

fn move_pointer(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    draw(window, cx);
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }),
        cx,
    );
    draw(window, cx);
}

/// A left press (`down`) or release at `at`, then a frame.
fn press(window: &mut Window, cx: &mut App, at: Point<Pixels>, down: bool) {
    let event = if down {
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        })
    } else {
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        })
    };
    window.dispatch_event(event, cx);
    draw(window, cx);
}

/// The image cut down to what differs from its corner's colour, with a margin.
fn crop(image: &image::RgbaImage, margin: u32) -> image::RgbaImage {
    let bg = *image.get_pixel(1, 1);
    let (w, h) = image.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in image.enumerate_pixels() {
        let differs = p.0.iter().zip(bg.0.iter()).any(|(a, b)| a.abs_diff(*b) > 3);
        if differs {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 || y1 < y0 {
        return image.clone();
    }
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(w - 1), (y1 + margin).min(h - 1));
    image::imageops::crop_imm(image, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(
        args.first()
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().join("chat-visuals").display().to_string()),
    );
    let filter: Vec<String> = args.iter().skip(1).cloned().collect();
    let all = scenes();
    let chosen: Vec<Scene> = all
        .into_iter()
        .filter(|s| filter.is_empty() || filter.iter().any(|f| s.name.contains(f.as_str())))
        .collect();
    std::fs::create_dir_all(&out).expect("out dir");

    application().with_assets(Assets).run(move |cx: &mut App| {
        init(cx);
        theme::fonts::register(cx);
        theme::apply_preference(ThemePreference::Light, cx);
        let first = chosen.first().cloned().expect("a scene");
        let state = Rc::new(RefCell::new((first, 1.0f32)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(40.), px(40.)),
                size(px(W), px(H)),
            ))),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            is_movable: false,
            ..WindowOptions::default()
        };
        let handle = cx
            .open_window(options, {
                let state = state.clone();
                move |window, cx| {
                    #[cfg(target_os = "macos")]
                    nodus_ui::mac::ignore_real_mouse(window);
                    let form = parse_form(&form_spec()).expect("the form parses");
                    let mut forms = Vec::new();
                    for scale in [1.0f32, 0.75] {
                        for done in [false, true] {
                            let sent = done.then(answered);
                            let f = form.clone();
                            let view = cx.new(|cx| {
                                FormView::new(
                                    f,
                                    sent,
                                    scale,
                                    Rc::new(|answers, _, _| println!("sent {answers}")),
                                    window,
                                    cx,
                                )
                            });
                            forms.push((done, scale, view));
                        }
                    }
                    cx.new(|_| Gallery {
                        scene: state,
                        forms,
                        flow: None,
                    })
                }
            })
            .expect("window");
        cx.spawn(async move |cx| {
            for (pref, theme_name) in [(ThemePreference::Light, "light"), (ThemePreference::Dark, "dark")] {
                cx.update(|cx| theme::apply_preference(pref, cx));
                for scene in &chosen {
                    for (scale, scale_name) in [(1.0f32, "1x"), (0.75, "0.75x")] {
                        *state.borrow_mut() = (scene.clone(), scale);
                        if scene.name == "form-flow" {
                            let _ = cx.update(|cx| {
                                handle.update(cx, |g, window, cx| {
                                    let form = parse_form(&form_spec()).expect("the form parses");
                                    g.flow = Some(cx.new(|cx| {
                                        FormView::new(
                                            form,
                                            None,
                                            scale,
                                            Rc::new(|answers, _, _| println!("SENT {answers}")),
                                            window,
                                            cx,
                                        )
                                    }));
                                })
                            });
                        }
                        // Away first, so nothing starts hovered.
                        let _ = in_window(cx, handle, |window, cx| move_pointer(window, cx, point(px(2.), px(2.))));
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        let at = |x: f32, y: f32| point(px(PAD + x * scale), px(PAD + y * scale));
                        let mut steps = scene.steps.clone();
                        if !steps.iter().any(|s| matches!(s, Step::Shot(_))) {
                            steps.push(Step::Shot(""));
                        }
                        for step in steps {
                            match step {
                                Step::Point(x, y) => {
                                    // Two moves a beat apart, as a hand makes them.
                                    for nudge in [1., 0.] {
                                        let to = at(x + nudge, y);
                                        let _ = in_window(cx, handle, |window, cx| move_pointer(window, cx, to));
                                        cx.background_executor().timer(Duration::from_millis(80)).await;
                                    }
                                }
                                Step::Click(x, y) => {
                                    let to = at(x, y);
                                    let _ = in_window(cx, handle, |window, cx| {
                                        move_pointer(window, cx, to);
                                        press(window, cx, to, true);
                                    });
                                    cx.background_executor().timer(Duration::from_millis(90)).await;
                                    let _ = in_window(cx, handle, |window, cx| press(window, cx, to, false));
                                    cx.background_executor().timer(Duration::from_millis(60)).await;
                                }
                                Step::Type(text) => {
                                    let _ = in_window(cx, handle, |window, cx| {
                                        for c in text.chars() {
                                            let key = if c == ' ' { "space".into() } else { c.to_string() };
                                            let keystroke = Keystroke {
                                                modifiers: Modifiers::default(),
                                                key,
                                                key_char: Some(c.to_string()),
                                            };
                                            window.dispatch_keystroke(keystroke, cx);
                                        }
                                        draw(window, cx);
                                    });
                                    cx.background_executor().timer(Duration::from_millis(60)).await;
                                }
                                Step::Shot(suffix) => {
                                    cx.background_executor().timer(Duration::from_millis(250)).await;
                                    let image = in_window(cx, handle, |window, cx| {
                                        draw(window, cx);
                                        window.render_to_image()
                                    });
                                    let name = if suffix.is_empty() {
                                        scene.name.to_owned()
                                    } else {
                                        format!("{}-{suffix}", scene.name)
                                    };
                                    match image {
                                        Ok(Ok(image)) => {
                                            let path = out.join(format!("{name}.{theme_name}.{scale_name}.png"));
                                            crop(&image, 12).save(&path).expect("save");
                                            println!("{}", path.display());
                                        }
                                        other => println!("capture failed for {name}: {:?}", other.err()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
