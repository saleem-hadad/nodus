//! Step 8 proof: per-element backdrop blur (CSS `backdrop-filter: blur(30px) saturate(1.8)`) that
//! composites GPUI content behind it, plus a GPU-side group opacity check. Built on the closed
//! upstream "effect layers" PR stack (zed#63773..#63780).
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, App, Bounds, ColorMatrix, Context, Hsla, Rgba, Window, WindowBounds,
    WindowOptions, canvas, div, linear_color_stop, linear_gradient, prelude::*, px, rgb, rgba, size,
};
use gpui_platform::application;

/// CSS `saturate(s)` as a 4x5 colour matrix (Filter Effects spec).
fn saturate(s: f32) -> ColorMatrix {
    ColorMatrix([
        0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0., 0., //
        0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0., 0., //
        0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0., 0., //
        0., 0., 0., 1., 0.,
    ])
}

fn hex(c: u32, a: f32) -> Hsla {
    Rgba { r: ((c >> 16) & 0xff) as f32 / 255., g: ((c >> 8) & 0xff) as f32 / 255., b: (c & 0xff) as f32 / 255., a }.into()
}

struct Demo {
    t0: Instant,
    frames: Vec<Instant>,
    blur: bool,
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t0.elapsed().as_secs_f32();
        if self.t0.elapsed() < Duration::from_secs(8) {
            window.request_animation_frame();
        }
        let this = cx.entity();
        let x = 60. + 220. * (t * 1.3).sin().abs();
        // Board-like content: dot grid, coloured cards, text.
        let cards = (0..6).map(|i| {
            let colors = [0xff3b30, 0xff9500, 0x28cd41, 0x007aff, 0xaf52de, 0x5ac8fa];
            div()
                .absolute()
                .left(px(40. + 150. * i as f32))
                .top(px(80. + 40. * (i % 2) as f32))
                .w(px(130.))
                .h(px(170.))
                .rounded(px(12.))
                .bg(rgb(colors[i]))
                .shadow_lg()
                .p(px(10.))
                .text_color(rgb(0xffffff))
                .text_size(px(15.))
                .child(format!("Card {i}\nsharp text\nunder the blur"))
        });
        let panel = |id: &'static str, left: f32, top: f32, w: f32, h: f32, sigma: f32, sat: f32, label: &str| {
            let mut d = div()
                .id(id)
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .rounded(px(14.))
                .border_1()
                .border_color(hex(0xffffff, 0.35))
                .bg(hex(0xf6f6f5, 0.55))
                .p(px(14.))
                .text_size(px(14.))
                .text_color(hex(0x000000, 0.85))
                .child(label.to_string());
            if self.blur {
                d = d.backdrop_blur(px(sigma)).backdrop_matrix(saturate(sat));
            }
            d
        };
        div()
            .size_full()
            .relative()
            .bg(rgb(0xf3f3f1))
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| {
                    let mut y = 0.;
                    while y < f32::from(bounds.size.height) {
                        let mut x = 0.;
                        while x < f32::from(bounds.size.width) {
                            window.paint_quad(gpui::fill(Bounds::new(bounds.origin + gpui::point(px(x), px(y)), size(px(2.), px(2.))), hex(0, 0.2)));
                            x += 24.;
                        }
                        y += 24.;
                    }
                })
                .absolute()
                .size_full(),
            )
            .children(cards)
            .child(
                // moving content under the panels
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(330.))
                    .w(px(260.))
                    .h(px(70.))
                    .rounded(px(35.))
                    .bg(linear_gradient(90., linear_color_stop(rgb(0x007aff), 0.), linear_color_stop(rgb(0xff2d55), 1.)))
                    .text_color(rgb(0xffffff))
                    .text_size(px(28.))
                    .pl(px(24.))
                    .pt(px(16.))
                    .child("moving"),
            )
            .child(panel("archive", 520., 230., 400., 260., 30., 1.8, "Archive panel · backdrop-filter: blur(30px) saturate(1.8)"))
            .child(panel("toast", 90., 300., 300., 80., 20., 1.6, "Toast · blur(20px) saturate(1.6)"))
            .child(
                // group opacity check: a fading box with an overlapping child
                div()
                    .absolute()
                    .left(px(60.))
                    .top(px(430.))
                    .w(px(200.))
                    .h(px(90.))
                    .bg(rgb(0x1d1d1f))
                    .rounded(px(10.))
                    .opacity(0.5)
                    .child(div().ml(px(40.)).mt(px(20.)).w(px(120.)).h(px(50.)).bg(rgb(0xffd60a)).rounded(px(8.))),
            )
            .child(
                div()
                    .absolute()
                    .left(px(300.))
                    .top(px(430.))
                    .w(px(200.))
                    .h(px(90.))
                    .bg(rgb(0x1d1d1f))
                    .rounded(px(10.))
                    .with_animation("fade", Animation::new(Duration::from_millis(900)), |el, d| el.opacity(0.3 + 0.2 * d)),
            )
            .child(canvas(|_, _, _| {}, move |_, _, _, cx| {
                this.update(cx, |d, _| d.frames.push(Instant::now()));
            }))
            .child(div().absolute().left(px(12.)).bottom(px(10.)).text_size(px(12.)).text_color(rgba(0x00000099)).child(if self.blur { "backdrop blur ON" } else { "backdrop blur OFF" }))
    }
}

fn main() {
    let blur = std::env::var("BLUR").map(|v| v != "0").unwrap_or(true);
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(980.), px(560.)), cx);
        let handle = cx
            .open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), focus: false, show: false, inactive_frame_interval: None, ..Default::default() },
                |_, cx| cx.new(|_| Demo { t0: Instant::now(), frames: Vec::new(), blur }),
            )
            .unwrap();
        // Spike-only: keep the window out of Stage Manager and show it without activating the app.
        handle
            .update(cx, |_, window, _| {
                use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let h = HasWindowHandle::window_handle(window).unwrap();
                let RawWindowHandle::AppKit(h) = h.as_raw() else { panic!() };
                let view = unsafe { &*(h.ns_view.as_ptr() as *const NSView) };
                let w = view.window().unwrap();
                w.setCollectionBehavior(NSWindowCollectionBehavior::Auxiliary | NSWindowCollectionBehavior::Transient);
                w.setLevel(3);
                w.orderFrontRegardless();
                println!("BLUR_WINDOW {}", w.windowNumber());
            })
            .ok();
        let entity = handle.update(cx, |_, _, cx| cx.entity()).ok();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(8500)).await;
            if let Some(e) = entity {
                let s = e.read_with(cx, |d, _| {
                    let f = &d.frames;
                    let n = f.len();
                    let gaps: Vec<f64> = f.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.).collect();
                    let mut sorted = gaps.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let fps = (n - 1) as f64 / (f[n - 1] - f[0]).as_secs_f64();
                    format!("frames={n} fps={fps:.1} gap_p50={:.2} gap_p95={:.2} gap_max={:.2}", sorted[sorted.len() / 2], sorted[sorted.len() * 95 / 100], sorted[sorted.len() - 1])
                });
                println!("BLUR_FRAMES {s}");
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
