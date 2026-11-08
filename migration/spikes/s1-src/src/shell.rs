//! Main window: transparent title bar with inset traffic lights (Nodus: hiddenInset at 18,18,
//! 52px bar), theme following the window's effective appearance, gpui-component Button + Input.
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::component::button::{Button, ButtonVariants as _};
use gpui::component::input::{Input, InputState};
use gpui::component::{Sizable as _, TitleBar};

use crate::board::Board;
use crate::terminal::TerminalView;
use crate::theme::{self, tokens};

#[derive(Clone, Copy, PartialEq)]
pub enum Content {
    Window,
    Board,
}

pub struct Shell {
    pub content: Content,
    pub input: Entity<InputState>,
    pub board: Option<Entity<Board>>,
    pub side_term: Option<Entity<TerminalView>>,
    pub clicks: usize,
    pub show_side: bool,
    _subs: Vec<Subscription>,
}

impl Shell {
    fn new(content: Content, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
        let subs = vec![cx.observe_window_appearance(window, |_, window, cx| {
            theme::sync(window, cx);
            println!("S1 APPEARANCE {:?}", window.appearance());
        })];
        theme::sync(window, cx);
        let (board, side_term) = if content == Content::Board {
            (Some(cx.new(|cx| Board::new(window, cx))), Some(cx.new(TerminalView::new)))
        } else {
            (None, None)
        };
        Self { content, input, board, side_term, clicks: 0, show_side: true, _subs: subs }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        let title = TitleBar::new()
            .h(px(52.))
            .pl(px(88.))
            .bg(t.bg_toolbar)
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text_primary).child("Nodus · S1"))
                    .child(
                        Button::new("new-session")
                            .small()
                            .primary()
                            .label(format!("New session ({})", self.clicks))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clicks += 1;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("toggle-side")
                            .small()
                            .label(if self.show_side { "Hide terminal panel" } else { "Show terminal panel" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_side = !this.show_side;
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(240.)).child(Input::new(&self.input).small())),
            )
            .child(
                div()
                    .pr(px(14.))
                    .text_size(px(11.))
                    .text_color(t.text_secondary)
                    .child(format!("{:?} · scale {}", window.appearance(), window.scale_factor())),
            );

        let body: AnyElement = match self.content {
            Content::Window => div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(div().text_size(px(26.)).font_weight(FontWeight::BOLD).text_color(t.text_primary).child("Step 1: window + theme"))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(t.text_secondary)
                        .child(format!("appearance = {:?}, input = {:?}", window.appearance(), self.input.read(cx).value())),
                )
                .child(Button::new("big").primary().label("A gpui-component Button"))
                .child(div().w(px(320.)).child(Input::new(&self.input)))
                .into_any_element(),
            Content::Board => div()
                .flex_1()
                .flex()
                .flex_row()
                .min_h_0()
                .child(div().flex_1().min_w_0().h_full().children(self.board.clone()))
                .when(self.show_side, |d| {
                    d.child(
                        div()
                            .w(px(460.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .border_l_1()
                            .border_color(t.border)
                            .bg(t.bg_window)
                            .child(
                                div()
                                    .h(px(30.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(12.))
                                    .text_color(t.text_secondary)
                                    .child("Terminal panel (unscaled)"),
                            )
                            .child(div().flex_1().min_h_0().children(self.side_term.clone())),
                    )
                })
                .into_any_element(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg_window)
            .text_color(t.text_primary)
            .font_family(".SystemUIFont")
            .child(title)
            .child(body)
    }
}

pub fn open_main_window(content: Content, cx: &mut App) -> WindowHandle<gpui::base::Root> {
    let no_activate = crate::env_flag("S1_NO_ACTIVATE");
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1080.), px(740.)), cx))),
        titlebar: Some(TitlebarOptions {
            title: Some("Nodus S1".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(18.), px(18.))),
        }),
        app_owns_titlebar_drag: true,
        window_min_size: Some(size(px(960.), px(620.))),
        focus: !no_activate,
        show: !crate::env_flag("S1_STAGE_EXEMPT"),
        // Measure real frame pacing even while another app is frontmost.
        inactive_frame_interval: if crate::env_flag("S1_NO_THROTTLE") { None } else { WindowOptions::default().inactive_frame_interval },
        ..Default::default()
    };
    let mut shell_entity = None;
    let handle = cx
        .open_window(options, |window, cx| {
            let shell = cx.new(|cx| Shell::new(content, window, cx));
            shell_entity = Some(shell.clone());
            cx.new(|cx| gpui::base::Root::new(shell, window, cx))
        })
        .expect("open main window");
    let shell = shell_entity.unwrap();
    handle
        .update(cx, |_, window, cx| {
            #[cfg(target_os = "macos")]
            if let Ok(a) = std::env::var("S1_APPEARANCE") {
                crate::mac::force_appearance(window, a == "dark");
            }
            #[cfg(target_os = "macos")]
            if crate::env_flag("S1_STAGE_EXEMPT") {
                crate::mac::stage_exempt(window, crate::env_flag("S1_PARK"));
            }
            #[cfg(target_os = "macos")]
            println!("S1 MAIN_WINDOW {}", crate::mac::window_number(window));
            if let Some(board) = shell.read(cx).board.clone() {
                let f = board.read(cx).focus.clone();
                window.focus(&f, cx);
            }
        })
        .ok();
    if crate::env_flag("S1_AUTOPILOT") {
        crate::autopilot::run_main(handle.into(), shell, cx);
    }
    handle
}
