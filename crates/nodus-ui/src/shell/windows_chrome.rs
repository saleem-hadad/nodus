//! The window chrome Chromium drew for 0.9.4 off a Mac (spec 01 PLAT-018,
//! PLAT-060; spec 08 UI-099): the caption buttons of the title bar overlay
//! (`titleBarOverlay`), and the whole application menu as a popup from the
//! title bar's ☰ button.

use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Entity, FontWeight, Global, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled,
    Window, WindowControlArea, deferred, div, point, px, rgba,
};

use crate::platform::Platform;
use crate::shell::menus;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// Each caption button's width; three of them are the 138px the title bar
/// leaves free (`title_bar::CAPTION_W`).
const CAPTION_BUTTON_W: f32 = 46.0;

/// Windows' symbol font for the caption glyphs: Segoe Fluent Icons on
/// Windows 11, Segoe MDL2 Assets on Windows 10.
fn caption_font(window: &Window) -> &'static str {
    let fluent = "Segoe Fluent Icons";
    if window.text_system().all_font_names().iter().any(|f| f == fluent) {
        fluent
    } else {
        "Segoe MDL2 Assets"
    }
}

/// PLAT-018: minimize, maximize (restore while maximized) and close at the
/// title bar's right end, 52px tall, on the bar's colour with the theme's
/// symbol colour (`--bg-toolbar` over `--bg-window`; `#1f1f1f` / `#e6e6e6`).
/// The OS handles them (and Windows 11's snap layouts on maximize): each is
/// its window-control area. Hover and press tint the symbol colour as
/// Chromium's caption buttons do; close turns red.
pub fn caption_buttons(window: &mut Window, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let font = caption_font(window);
    let symbol = t.caption_symbol;
    let tint = |a: f32| gpui::Rgba { a, ..symbol };
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(CAPTION_BUTTON_W))
            .h_full()
            .font_family(font)
            .text_size(px(10.))
            .text_color(symbol)
            .window_control_area(area)
            .map(|d| {
                if close {
                    d.hover(|s| s.bg(rgba(0xe81123ff)).text_color(gpui::white()))
                        .active(|s| s.bg(rgba(0xe8112399)).text_color(gpui::white()))
                } else {
                    d.hover(|s| s.bg(tint(0x1a as f32 / 255.0)))
                        .active(|s| s.bg(tint(0x33 as f32 / 255.0)))
                }
            })
            .child(glyph)
    };
    let max_glyph = if window.is_maximized() { "\u{E923}" } else { "\u{E922}" };
    div()
        .id("caption-buttons")
        .absolute()
        .top_0()
        .right_0()
        .h(TITLEBAR_H)
        .flex()
        .bg(t.caption_bg)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(button("caption-min", "\u{E921}", WindowControlArea::Min, false))
        .child(button("caption-max", max_glyph, WindowControlArea::Max, false))
        .child(button("caption-close", "\u{E8BB}", WindowControlArea::Close, true))
        .into_any_element()
}

/// The ☰ popup's place: the title bar's menu button is 28px tall, 8px in,
/// centred in the 52px bar, and the menu opens 4px below its bottom-left
/// corner (UI-099, PLAT-060).
pub fn menu_origin() -> Point<Pixels> {
    let button_h = px(28.);
    point(px(8.), (TITLEBAR_H + button_h) / 2. + px(4.))
}

/// The application menu as a popup in the window (PLAT-060): one window's,
/// opened from the title bar's ☰ button off a Mac.
pub struct AppMenu {
    at: Option<Point<Pixels>>,
    /// The top-level menu whose items show beside it.
    open_menu: Option<usize>,
    /// When a press outside last closed it: that press on the ☰ button
    /// closes it, rather than opening it again.
    closed_at: Option<Instant>,
}

/// The window's [`AppMenu`].
pub struct AppMenuSlot(pub Entity<AppMenu>);

impl Global for AppMenuSlot {}

impl AppMenu {
    /// The window's popup, made the first time it's asked for.
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if let Some(slot) = cx.try_global::<AppMenuSlot>() {
            return slot.0.clone();
        }
        let menu = cx.new(|_| AppMenu {
            at: None,
            open_menu: None,
            closed_at: None,
        });
        cx.set_global(AppMenuSlot(menu.clone()));
        menu
    }

    /// Opens it at `at` (window coordinates), or closes it when it's open.
    pub fn toggle(at: Point<Pixels>, cx: &mut App) {
        let menu = Self::entity(cx);
        menu.update(cx, |m, cx| {
            let just_closed = m.closed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(300));
            m.at = if m.at.is_some() || just_closed { None } else { Some(at) };
            m.open_menu = None;
            m.closed_at = None;
            cx.notify();
        });
    }

    pub fn is_open(&self) -> bool {
        self.at.is_some()
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.at = None;
        self.open_menu = None;
        cx.notify();
    }
}

/// The shortcut a Windows popup shows beside an item: Electron's text for
/// the menu's accelerators (PLAT-040…PLAT-043), shown whether or not it's
/// registered (PLAT-055).
pub fn windows_accelerator(item: &str) -> Option<&'static str> {
    Some(match item {
        "New Session" => "Ctrl+N",
        "New Terminal" => "Ctrl+T",
        "New Project…" => "Ctrl+Shift+N",
        "Close" => "Ctrl+W",
        "Settings…" => "Ctrl+,",
        "Undo" => "Ctrl+Z",
        "Redo" => "Ctrl+Y",
        "Cut" => "Ctrl+X",
        "Copy" => "Ctrl+C",
        "Paste" => "Ctrl+V",
        "Select All" => "Ctrl+A",
        "Search Sessions…" => "Ctrl+K",
        "Focus Mode" => "Ctrl+Shift+F",
        "Reload" => "Ctrl+R",
        "Actual Size" => "Ctrl+0",
        // Electron's `CommandOrControl+Plus` is Shift and the = key.
        "Zoom In" => "Ctrl+Shift+=",
        "Zoom Out" => "Ctrl+-",
        "Toggle Full Screen" => "F11",
        "Minimize" => "Ctrl+M",
        _ => return None,
    })
}

/// A row of the popup: a submenu or an item.
struct Row {
    label: SharedString,
    accelerator: Option<&'static str>,
    checked: bool,
    disabled: bool,
    action: Option<Box<dyn Action>>,
}

impl Render for AppMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let Some(at) = self.at else {
            return div().into_any_element();
        };
        let t = theme::tokens(cx);
        let settings = crate::shell::window::main_shell(cx)
            .and_then(|shell| shell.read(cx).state.read(cx).settings.clone())
            .unwrap_or_default();
        let menus = menus::app_menus(&settings, Platform::Windows);
        let panel = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .flex_col()
                .min_w(px(220.))
                .py(px(4.))
                .rounded(px(8.))
                .bg(t.bg_elevated)
                .border_1()
                .border_color(t.border)
                .shadow(t.md())
                .font_family(fonts::UI)
                .text_size(TEXT_SM)
                .text_color(t.text_primary)
        };
        let row = |id: SharedString, label: SharedString, right: Option<AnyElement>, hovered: bool, checked: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(28.))
                .mx(px(4.))
                .pl(px(8.))
                .pr(px(10.))
                .rounded(px(4.))
                .when(hovered, |d| d.bg(t.bg_hover))
                .hover(|s| s.bg(t.bg_hover))
                .child(
                    div()
                        .w(px(20.))
                        .flex_none()
                        .text_color(t.text_primary)
                        .when(checked, |d| d.child("✓")),
                )
                .child(div().flex_1().whitespace_nowrap().child(label))
                .children(right)
        };
        let separator = || div().my(px(4.)).mx(px(8.)).h(px(1.)).bg(t.separator).into_any_element();

        let this = cx.entity();
        let top = panel("app-menu".into()).children(menus.iter().enumerate().map(|(ix, menu)| {
            let open = self.open_menu == Some(ix);
            let enter = this.clone();
            row(
                SharedString::from(format!("app-menu-{ix}")),
                menu.name.clone(),
                Some(div().text_color(t.text_tertiary).child("›").into_any_element()),
                open,
                false,
            )
            .on_hover(move |hovered, _, cx| {
                if *hovered {
                    enter.update(cx, |m, cx| {
                        m.open_menu = Some(ix);
                        cx.notify();
                    });
                }
            })
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |m, cx| {
                        m.open_menu = Some(ix);
                        cx.notify();
                    });
                }
            })
            .into_any_element()
        }));

        let sub = self.open_menu.and_then(|ix| menus.into_iter().nth(ix).map(|m| (ix, m)));
        let sub = sub.map(|(ix, menu)| {
            let rows: Vec<Row> = menu
                .items
                .into_iter()
                .filter_map(|item| match item {
                    gpui::MenuItem::Separator => Some(Row {
                        label: SharedString::default(),
                        accelerator: None,
                        checked: false,
                        disabled: false,
                        action: None,
                    }),
                    gpui::MenuItem::Action {
                        name,
                        action,
                        checked,
                        disabled,
                        ..
                    } => Some(Row {
                        accelerator: windows_accelerator(&name),
                        label: name,
                        checked,
                        disabled,
                        action: Some(action),
                    }),
                    _ => None,
                })
                .collect();
            let items = rows.into_iter().enumerate().map(|(n, r)| {
                let Some(action) = r.action else {
                    return separator();
                };
                let close = this.clone();
                row(
                    SharedString::from(format!("app-menu-{ix}-{n}")),
                    r.label,
                    r.accelerator.map(|a| {
                        div()
                            .pl(px(24.))
                            .text_color(t.text_tertiary)
                            .child(a)
                            .into_any_element()
                    }),
                    false,
                    r.checked,
                )
                .when(r.disabled, |d| d.opacity(0.4))
                .on_click(move |_, window, cx| {
                    close.update(cx, AppMenu::close);
                    // As the menu bar's item: to whatever has the keyboard.
                    window.dispatch_action(action.boxed_clone(), cx);
                })
                .into_any_element()
            });
            // Beside its row in the top-level list.
            let top = px(4. + 28. * ix as f32 - 4.);
            div()
                .absolute()
                .left(px(220. - 4.))
                .top(top)
                .child(panel(SharedString::from(format!("app-menu-sub-{ix}"))).children(items))
        });

        let close_out = this.clone();
        let close_esc = this.clone();
        deferred(
            div()
                .id("app-menu-layer")
                .absolute()
                .left(at.x)
                .top(at.y)
                .on_mouse_down_out(move |_, _, cx| {
                    close_out.update(cx, |m, cx| {
                        m.close(cx);
                        m.closed_at = Some(Instant::now());
                    });
                })
                .on_key_down(move |e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" {
                        cx.stop_propagation();
                        close_esc.update(cx, AppMenu::close);
                    }
                })
                .font_weight(FontWeight::NORMAL)
                .child(top)
                .children(sub),
        )
        .with_priority(1000)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::AppSettings;

    #[test]
    fn shows_every_item_of_the_windows_menu_with_electrons_shortcuts() {
        let menus = menus::app_menus(&AppSettings::default(), Platform::Windows);
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(names, ["File", "Edit", "View", "Window", "Help"]);
        assert_eq!(windows_accelerator("New Project…"), Some("Ctrl+Shift+N"));
        assert_eq!(windows_accelerator("Exit"), None);
        assert_eq!(windows_accelerator("Toggle Full Screen"), Some("F11"));
        // 4px below a 28px button centred in the 52px bar, at its left edge.
        assert_eq!(menu_origin(), point(px(8.), px(44.)));
    }
}
