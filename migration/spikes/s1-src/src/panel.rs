//! Floating capture panel: borderless, transparent, rounded card with its own shadow, over every
//! Space and full-screen apps, takes the keyboard without activating the app, hides on blur.
use gpui::*;
use gpui::component::input::{Input, InputState};

use crate::theme::{self, tokens};

pub const PANEL_W: f32 = 652.;
pub const PANEL_H: f32 = 320.;
pub const MARGIN: f32 = 26.;

pub struct Panel {
    pub input: Entity<InputState>,
    pub blurs: usize,
    pub hidden: bool,
    _subs: Vec<Subscription>,
}

impl Panel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("What should Claude do with this?"));
        let subs = vec![
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                println!("S1 PANEL_ACTIVE {active}");
                if !active && !this.hidden {
                    // Electron version waits 50ms and re-checks; same idea.
                    this.blurs += 1;
                    this.hidden = true;
                    #[cfg(target_os = "macos")]
                    crate::mac::panel_hide(window);
                    println!("S1 PANEL_HIDDEN_ON_BLUR");
                    cx.notify();
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::sync(window, cx)),
        ];
        input.update(cx, |state, cx| state.focus(window, cx));
        Self { input, blurs: 0, hidden: false, _subs: subs }
    }
}

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        // Transparent stage with a margin the shadow fits in (Electron: .cap-stage).
        div().size_full().p(px(MARGIN)).child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(16.))
                .rounded(px(18.))
                .bg(t.bg_elevated.opacity(0.97))
                .border(px(0.5))
                .border_color(t.border)
                .shadow(vec![
                    BoxShadow { color: t.shadow_b.opacity(0.55), offset: point(px(0.), px(12.)), blur_radius: px(24.), spread_radius: px(-4.), inset: false },
                    BoxShadow { color: t.shadow_a, offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
                ])
                .text_color(t.text_primary)
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Capture"))
                .child(div().text_size(px(12.)).text_color(t.text_secondary).child("Floating NSPanel · level 101 · all Spaces + full-screen auxiliary · hides on blur"))
                .child(Input::new(&self.input))
                .child(div().flex_1())
                .child(div().text_size(px(11.)).text_color(t.text_secondary).child(format!("blurs: {}", self.blurs))),
        )
    }
}

pub fn open_panel(cx: &mut App) -> WindowHandle<gpui::base::Root> {
    let no_activate = crate::env_flag("S1_NO_ACTIVATE");
    // Centred on the primary display, its card a fifth of the way down.
    let display = cx.primary_display();
    let screen = display.as_ref().map(|d| d.bounds()).unwrap_or(Bounds::new(point(px(0.), px(0.)), size(px(1440.), px(900.))));
    let origin = point(
        screen.origin.x + (screen.size.width - px(PANEL_W)) / 2.,
        screen.origin.y + screen.size.height * 0.2 - px(MARGIN),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size(px(PANEL_W), px(PANEL_H))))),
        titlebar: None,
        // Shown below, without activating the app.
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        display_id: display.map(|d| d.id()),
        ..Default::default()
    };
    let mut panel_entity = None;
    let handle = cx
        .open_window(options, |window, cx| {
            let panel = cx.new(|cx| Panel::new(window, cx));
            panel_entity = Some(panel.clone());
            cx.new(|cx| gpui::base::Root::new(panel, window, cx))
        })
        .expect("open panel");
    handle
        .update(cx, |_, window, cx| {
            #[cfg(target_os = "macos")]
            {
                crate::mac::configure_floating_panel(window);
                if !no_activate {
                    let w = crate::mac::ns_window(window);
                    // AppKit calls back into GPUI (windowDidBecomeKey): run it outside this update.
                    window.defer(cx, move |_, _| crate::mac::panel_show_ns(&w));
                } else {
                    crate::mac::ns_window(window).orderFrontRegardless();
                }
                if let Ok(a) = std::env::var("S1_APPEARANCE") {
                    crate::mac::force_appearance(window, a == "dark");
                }
                println!("S1 PANEL_WINDOW {}", crate::mac::window_number(window));
                println!("S1 PANEL_DESC {}", crate::mac::describe_window(window));
            }
        })
        .ok();
    if crate::env_flag("S1_AUTOPILOT") {
        crate::autopilot::run_panel(handle.into(), panel_entity.unwrap(), cx);
    }
    handle
}
