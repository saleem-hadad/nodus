//! The proof of our GPUI patch set (third_party/gpui/PATCHES.md): backdrop
//! blur, element transforms, group opacity and window capture.
//!
//! ```sh
//! cargo run -p nodus-ui --example effects --release
//! ```
//!
//! It opens a non-activating pop-up window in the bottom right corner of the
//! main display, so it never takes the focus or the menu bar, animates for a
//! few seconds, prints what it checked and how long frames took, writes the
//! window's frame to a PNG and quits.
//!
//! Environment:
//!
//! - `EFFECTS_SECONDS=8`: how long to run.
//! - `EFFECTS_CAPTURE=<file.png>`: where to write the capture (default
//!   `effects.png` in the system temp directory).
//! - `EFFECTS_STRESS=1`: six blurred panels and twenty rotated notes.
//! - `EFFECTS_OFF=1`: the same scene with every effect off, to compare frame
//!   times against.
//! - `EFFECTS_STRICT=1`: exit with status 1 when a check fails (CI).
#![allow(clippy::print_stdout, reason = "an example reports on stdout")]

use gpui_kit::*;
use std::{
    cell::RefCell,
    f32::consts::PI,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

const WIDTH: f32 = 960.;
const HEIGHT: f32 = 600.;

/// What the frames recorded: when each one painted, and how long the CPU
/// spent from `render` to the end of paint.
#[derive(Default)]
struct Stats {
    painted: Vec<Instant>,
    cpu: Vec<Duration>,
    render_started: Option<Instant>,
}

struct Effects {
    started: Instant,
    stats: Rc<RefCell<Stats>>,
    stress: bool,
    off: bool,
    /// Whether the rotated note says it's hovered.
    note_hovered: Rc<RefCell<bool>>,
}

/// Degrees, the unit the CSS uses, as radians.
fn deg(degrees: f32) -> Radians {
    radians(degrees * PI / 180.)
}

/// A soft drop shadow like the one Nodus gives its notes.
fn note_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.28),
        offset: point(px(0.), px(10.)),
        blur_radius: px(24.),
        spread_radius: px(-4.),
        inset: false,
    }]
}

impl Effects {
    /// What moves behind the frosted panels: cards, text and a gradient pill.
    fn backdrop(&self, t: f32) -> impl IntoElement {
        let colors = [0xff453a, 0x30d158, 0x0a84ff, 0xffd60a, 0xbf5af2, 0xff9f0a];
        let mut board = div().absolute().size_full().bg(rgb(0xf2f2f7));
        // Lanes across the whole window, so cards pass under every panel.
        let lanes = [20., 130., 240., 330., 440., 520.];
        for (index, color) in colors.iter().enumerate() {
            let speed = 70. + index as f32 * 23.;
            let x = (t * speed + index as f32 * 260.).rem_euclid(WIDTH + 160.) - 140.;
            let y = lanes[index];
            board = board.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(130.))
                    .h(px(86.))
                    .rounded(px(12.))
                    .bg(rgb(*color))
                    .p(px(10.))
                    .text_size(px(15.))
                    .text_color(white())
                    .font_weight(FontWeight::BOLD)
                    .child(format!("card {index}")),
            );
        }
        let pill_x = (t * 140.).rem_euclid(WIDTH + 200.) - 200.;
        board.child(
            div()
                .absolute()
                .left(px(pill_x))
                .top(px(300.))
                .w(px(200.))
                .h(px(44.))
                .rounded(px(22.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(0xff2d55), 0.),
                    linear_color_stop(rgb(0x5856d6), 1.),
                ))
                .text_color(white())
                .text_size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .child("moving under the glass"),
        )
    }

    /// A frosted panel: CSS `backdrop-filter: blur(<blur>px) saturate(<saturate>)`.
    fn frosted(&self, blur: f32, saturate: f32) -> Div {
        let panel = div()
            .absolute()
            .rounded(px(14.))
            .border_1()
            .border_color(hsla(0., 0., 1., 0.5))
            .bg(hsla(0., 0., 1., 0.55))
            .p(px(14.))
            .text_size(px(13.))
            .text_color(rgb(0x1c1c1e));
        if self.off {
            panel
        } else {
            panel.backdrop_blur(px(blur)).backdrop_saturate(saturate)
        }
    }

    /// A sticky note tilted by `angle`, its text drawn by the system's text
    /// renderer.
    fn note(&self, id: &'static str, angle: f32) -> Stateful<Div> {
        let note = div()
            .id(id)
            .absolute()
            .w(px(170.))
            .h(px(150.))
            .p(px(14.))
            .rounded(px(4.))
            .bg(rgb(0xfff0a3))
            .shadow(note_shadow())
            .text_size(px(15.))
            .line_height(px(20.))
            .text_color(rgb(0x3a3000))
            .child("Buy milk, then ship the GPUI patch set before Friday.");
        if self.off {
            note
        } else {
            note.rotate(deg(angle))
        }
    }

    /// Two overlapping squares faded together, as one picture: where they
    /// overlap only the top one shows, as with CSS `opacity`.
    fn fade_pair(&self, label: &'static str, opacity: f32, group: bool) -> Div {
        let squares = div()
            .relative()
            .w(px(120.))
            .h(px(90.))
            .child(div().absolute().left_0().top_0().size(px(70.)).rounded(px(8.)).bg(rgb(0x0a84ff)))
            .child(
                div()
                    .absolute()
                    .left(px(40.))
                    .top(px(20.))
                    .size(px(70.))
                    .rounded(px(8.))
                    .bg(rgb(0xff375f)),
            );
        let squares = if group && !self.off {
            squares.group_opacity(opacity)
        } else {
            squares.opacity(opacity)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(rgb(0x3a3a3c))
            .child(squares)
            .child(label)
    }
}

impl Render for Effects {
    fn render(&mut self, window: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        self.stats.borrow_mut().render_started = Some(Instant::now());
        window.request_animation_frame();
        let t = self.started.elapsed().as_secs_f32();
        // The drag tilt of Nodus' notes, swinging between 3 and 7 degrees.
        let tilt = 5. + 2. * (t * 2.).sin();
        let fade = 0.25 + 0.75 * (0.5 + 0.5 * (t * 1.7).cos());
        let stats = self.stats.clone();
        let hovered = self.note_hovered.clone();

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family(".SystemUIFont")
            .child(self.backdrop(t))
            // A rotated note, with a hover state through the transform.
            .child(
                self.note("note", tilt)
                    .left(px(70.))
                    .top(px(150.))
                    .hover(|style| style.bg(rgb(0xffe066)))
                    .on_hover(move |is_hovered, _, _| *hovered.borrow_mut() = *is_hovered),
            )
            // A sticker, tilted 3 degrees, as Nodus tilts them.
            .child({
                let sticker = div()
                    .absolute()
                    .left(px(300.))
                    .top(px(380.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .border_2()
                    .border_color(white())
                    .bg(rgb(0x34c759))
                    .shadow(note_shadow())
                    .text_color(white())
                    .text_size(px(18.))
                    .font_weight(FontWeight::BOLD)
                    .child("SHIPPED");
                if self.off { sticker } else { sticker.rotate(deg(-3.)) }
            })
            // The pad's peeling top sheet: scaled and turned around its top edge.
            .child({
                let sheet = div()
                    .absolute()
                    .left(px(70.))
                    .top(px(370.))
                    .w(px(170.))
                    .h(px(120.))
                    .p(px(12.))
                    .rounded(px(4.))
                    .bg(rgb(0xa0e7ff))
                    .shadow(note_shadow())
                    .text_size(px(14.))
                    .child("The pad's top sheet");
                if self.off {
                    sheet
                } else {
                    sheet
                        .transform_origin(0.5, 0.)
                        .rotate(deg(-4. + 3. * (t * 1.3).sin()))
                        .scale(0.96 + 0.04 * (t * 1.3).cos())
                }
            })
            // Group opacity against GPUI's own per-primitive opacity.
            .child(
                div()
                    .absolute()
                    .left(px(300.))
                    .top(px(150.))
                    .p(px(14.))
                    .rounded(px(12.))
                    .bg(white())
                    .shadow(note_shadow())
                    .flex()
                    .gap(px(18.))
                    .child(self.fade_pair("group_opacity", fade, true))
                    .child(self.fade_pair("opacity", fade, false)),
            )
            // The archive panel: blur(30px) saturate(1.8).
            .child(
                self.frosted(30., 1.8)
                    .right(px(24.))
                    .top(px(24.))
                    .w(px(260.))
                    .h(px(400.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Archive"))
                    .children((1..=6).map(|i| {
                        div()
                            .rounded(px(8.))
                            .bg(hsla(0., 0., 1., 0.6))
                            .p(px(8.))
                            .child(format!("Archived session {i}"))
                    })),
            )
            // A toast: blur(20px) saturate(1.6).
            .child(
                self.frosted(20., 1.6)
                    .left(px(300.))
                    .bottom(px(24.))
                    .w(px(340.))
                    .child("Toast over moving content, blur(20px) saturate(1.6)"),
            )
            // The prompt bar.
            .child(
                self.frosted(24., 1.6)
                    .left(px(300.))
                    .top(px(24.))
                    .w(px(360.))
                    .h(px(52.))
                    .rounded(px(26.))
                    .child("Ask Claude…"),
            );

        if self.stress {
            for i in 0..3 {
                root = root.child(
                    self.frosted(30., 1.8)
                        .left(px(40. + i as f32 * 190.))
                        .top(px(470.))
                        .w(px(170.))
                        .h(px(90.))
                        .child(format!("Blurred {i}")),
                );
            }
            for i in 0..20 {
                let angle = (t * 2. + i as f32).sin() * 7.;
                let id: &'static str = Box::leak(format!("stress-note-{i}").into_boxed_str());
                root = root.child(
                    self.note(id, angle)
                        .left(px(20. + (i % 10) as f32 * 64.))
                        .top(px(90. + (i / 10) as f32 * 160.))
                        .w(px(110.))
                        .h(px(100.)),
                );
            }
        }

        root.child(
            canvas(
                |_, _, _| {},
                move |_, _, _, _| {
                    let mut stats = stats.borrow_mut();
                    let now = Instant::now();
                    if let Some(started) = stats.render_started.take() {
                        stats.cpu.push(now - started);
                    }
                    stats.painted.push(now);
                },
            )
            .absolute()
            .size_full(),
        )
    }
}

/// p50, p95 and max of some durations, in milliseconds.
fn percentiles(mut values: Vec<f64>) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0., 0., 0.);
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.95), values[values.len() - 1])
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value != "0")
}

fn main() {
    let seconds: f64 = std::env::var("EFFECTS_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8.);
    let capture_path = std::env::var_os("EFFECTS_CAPTURE")
        .map_or_else(|| std::env::temp_dir().join("effects.png"), PathBuf::from);
    let stress = env_flag("EFFECTS_STRESS");
    let off = env_flag("EFFECTS_OFF");
    let strict = env_flag("EFFECTS_STRICT");

    application().run(move |cx: &mut App| {
        let stats = Rc::new(RefCell::new(Stats::default()));
        let note_hovered = Rc::new(RefCell::new(false));
        let size = size(px(WIDTH), px(HEIGHT));
        let origin = cx
            .primary_display()
            .map(|display| {
                let screen = display.bounds();
                point(
                    screen.origin.x + screen.size.width - size.width - px(16.),
                    screen.origin.y + screen.size.height - size.height - px(16.),
                )
            })
            .unwrap_or_default();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size })),
            titlebar: None,
            // A non-activating panel, shown without taking the focus.
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            is_movable: false,
            inactive_frame_interval: None,
            low_power_frame_interval: None,
            thermal_frame_interval: None,
            ..WindowOptions::default()
        };
        let handle = cx
            .open_window(options, {
                let stats = stats.clone();
                let note_hovered = note_hovered.clone();
                move |_, cx| {
                    cx.new(|_| Effects {
                        started: Instant::now(),
                        stats,
                        stress,
                        off,
                        note_hovered,
                    })
                }
            })
            .expect("the window opens");

        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;

            // Hit testing follows the transform. The note (70, 150, 170 by
            // 150) turns 3 to 7 degrees clockwise about its centre, so its
            // right edge swings out past x = 240 near the top, and its top
            // edge drops below y = 150 on the right.
            let mut failures = 0;
            let mut hit_checks = Vec::new();
            for (label, position, expect) in [
                ("inside the turned note, outside its box", point(px(241.5), px(175.)), !off),
                ("outside the turned note, inside its box", point(px(200.), px(151.)), off),
                ("the note's centre", point(px(155.), px(225.)), true),
            ] {
                let moved = handle.update(cx, |_, window, cx| {
                    window.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }),
                        cx,
                    );
                    window.refresh();
                });
                cx.background_executor().timer(Duration::from_millis(80)).await;
                let hovered = *note_hovered.borrow();
                let pass = moved.is_ok() && hovered == expect;
                failures += usize::from(!pass);
                hit_checks.push(format!(
                    "{label}: hovered={hovered} {}",
                    if pass { "pass" } else { "FAIL" }
                ));
            }
            for check in &hit_checks {
                println!("EFFECTS HIT {check}");
            }

            // Window capture, with no screen recording permission.
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let started = Instant::now();
            let image = handle.update(cx, |_, window, _| window.render_to_image());
            let took = started.elapsed();
            match image {
                Ok(Ok(image)) => {
                    let (width, height) = image.dimensions();
                    // A frame that drew anything has more than one colour.
                    let first = image.get_pixel(0, 0);
                    let drew = image.pixels().any(|pixel| pixel != first);
                    failures += usize::from(!drew || width == 0);
                    match image.save(&capture_path) {
                        Ok(()) => println!(
                            "EFFECTS CAPTURE {width}x{height} in {:.1} ms -> {}{}",
                            took.as_secs_f64() * 1000.,
                            capture_path.display(),
                            if drew { "" } else { " FAIL: one colour" }
                        ),
                        Err(error) => {
                            failures += 1;
                            println!("EFFECTS CAPTURE FAIL saving: {error}");
                        }
                    }
                }
                Ok(Err(error)) => {
                    failures += 1;
                    println!("EFFECTS CAPTURE FAIL: {error:#}");
                }
                Err(error) => {
                    failures += 1;
                    println!("EFFECTS CAPTURE FAIL: {error:#}");
                }
            }

            // Frame times over the rest of the run, after a warm-up.
            stats.borrow_mut().painted.clear();
            stats.borrow_mut().cpu.clear();
            cx.background_executor()
                .timer(Duration::from_secs_f64((seconds - 2.).max(2.)))
                .await;
            {
                let stats = stats.borrow();
                let frames = &stats.painted;
                if frames.len() > 2 {
                    let gaps: Vec<f64> = frames
                        .windows(2)
                        .map(|pair| (pair[1] - pair[0]).as_secs_f64() * 1000.)
                        .collect();
                    let over = gaps.iter().filter(|gap| **gap > 25.).count();
                    let fps = (frames.len() - 1) as f64
                        / (frames[frames.len() - 1] - frames[0]).as_secs_f64();
                    let (p50, p95, max) = percentiles(gaps);
                    let (cpu50, cpu95, cpu_max) = percentiles(
                        stats.cpu.iter().map(|cpu| cpu.as_secs_f64() * 1000.).collect(),
                    );
                    println!(
                        "EFFECTS FRAMES stress={stress} off={off} frames={} fps={fps:.1} \
                         gap_p50={p50:.2} gap_p95={p95:.2} gap_max={max:.2} gaps_over_25ms={over} \
                         cpu_p50={cpu50:.2} cpu_p95={cpu95:.2} cpu_max={cpu_max:.2}",
                        frames.len()
                    );
                }
            }
            if strict && failures > 0 {
                println!("EFFECTS FAILED {failures} checks");
                std::process::exit(1);
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
