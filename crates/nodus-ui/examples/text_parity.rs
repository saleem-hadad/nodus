//! Draws text cases with the patched GPUI, to compare with Chromium (track
//! gpui-text).
//!
//! usage: `cargo run -p nodus-ui --example text_parity -- <cases.json> <out.png>`
//!
//! Each case is `{x, y, w?, size, weight, lh, text, align, overflowWrap?,
//! wordBreak?, clamp?, ellipsis?, bar?, box?}` in logical pixels, black system-font
//! text on white, as a page of absolutely positioned divs would draw it in
//! Chromium. The window is a non-activating pop-up (it never takes focus);
//! the frame is saved with `Window::render_to_image` (2x on Retina) and the
//! example quits.

#![allow(clippy::print_stdout, reason = "an example reports on stdout")]

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;

const WIDTH: f32 = 1200.;
const HEIGHT: f32 = 760.;

struct Cases(Vec<serde_json::Value>);

impl Render for Cases {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .relative()
            .bg(white())
            .text_color(black())
            .font_family(nodus_ui::theme::fonts::UI);
        for case in &self.0 {
            let number = |key: &str| case[key].as_f64().map(|v| v as f32);
            let text = case["text"].as_str().unwrap_or_default().to_owned();
            let (x, y) = (number("x").unwrap_or_default(), number("y").unwrap_or_default());
            let size = number("size").unwrap_or(13.);
            let lh = number("lh").unwrap_or(size * 1.4);
            let mut el = div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .text_size(px(size))
                .font_weight(FontWeight(number("weight").unwrap_or(400.)))
                .line_height(px(lh));
            if let Some(w) = number("w") {
                el = el.w(px(w));
            }
            el = match case["align"].as_str() {
                Some("center") => el.text_center(),
                Some("right") => el.text_right(),
                _ => el,
            };
            if case["overflowWrap"].as_str() == Some("anywhere") {
                el = el.overflow_wrap(OverflowWrap::Anywhere);
            }
            if case["wordBreak"].as_str() == Some("break-all") {
                el = el.word_break(WordBreak::BreakAll);
            }
            if let Some(lines) = case["clamp"].as_u64() {
                el = el.line_clamp(lines as usize).text_ellipsis();
            }
            if case["ellipsis"].as_bool() == Some(true) {
                el = el.truncate();
            }
            if case["kind"].as_str() == Some("stack") {
                el = el.whitespace_nowrap();
            }
            el = match (case["kind"].as_str(), number("box")) {
                // Text in a flex row too narrow for it, then a fixed box.
                (Some("flex"), Some(width)) => el
                    .flex()
                    .items_start()
                    .child(div().child(text))
                    .child(div().flex_none().w(px(width)).h(px(10.)).bg(black())),
                _ => el.child(text),
            };
            root = root.child(el);
            if case["bar"].as_bool() == Some(true) {
                root = root.child(div().absolute().left(px(x + 30.)).top(px(y + lh)).w(px(20.)).h(px(2.)).bg(black()));
            }
        }
        root
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cases: Vec<serde_json::Value> = args
        .next()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let out = args.next().map_or_else(|| std::env::temp_dir().join("text_parity.png"), PathBuf::from);
    application().run(move |cx: &mut App| {
        cx.set_font_smoothing(FontSmoothing::Antialiased);
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
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            is_movable: false,
            ..WindowOptions::default()
        };
        let Ok(handle) = cx.open_window(options, move |_, cx| cx.new(|_| Cases(cases))) else {
            println!("TEXT PARITY no window");
            cx.quit();
            return;
        };
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            match handle.update(cx, |_, window, _| window.render_to_image()) {
                Ok(Ok(image)) => match image.save(&out) {
                    Ok(()) => println!("TEXT PARITY {}x{} -> {}", image.width(), image.height(), out.display()),
                    Err(error) => println!("TEXT PARITY save failed: {error}"),
                },
                Ok(Err(error)) => println!("TEXT PARITY capture failed: {error:#}"),
                Err(error) => println!("TEXT PARITY capture failed: {error:#}"),
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
