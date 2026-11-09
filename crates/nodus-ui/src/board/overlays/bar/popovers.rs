//! Above the pill: the slash menu (BAR-014, BAR-015) and the note's colour
//! picker (BAR-022). Both sit with their left edge at the pill's and their
//! bottom 8px above its top, and rise in over 120ms.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_board::compose::Target;

use super::view::{note_gradient, view_hue};
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::color_swatches::color_swatches;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tween::enter;
use crate::theme::tokens::{DUR_FAST, RADIUS_LG, TEXT_SM, Tokens, inset_shadow, rgba};
use crate::theme::{fonts, motion};

impl BoardCanvas {
    /// The menu while it's open, else the picker while it's up.
    pub(crate) fn render_bar_popover(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let commands = self.bar_commands();
        let (key, body) = if !commands.is_empty() {
            ("bar-menu", self.render_slash_menu(t, &commands, cx))
        } else if self.overlays.bar.picking && self.overlays.bar.target == Some(Target::Note) {
            ("bar-picker", self.render_picker(window, cx))
        } else {
            return None;
        };
        let rise = enter(window, cx, key, DUR_FAST, motion::ease_out);
        Some(
            div()
                .absolute()
                .left_0()
                .bottom_full()
                .pb(px(8.))
                .child(
                    div()
                        .id(key)
                        .occlude()
                        .rounded(RADIUS_LG)
                        .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.96), 24., 1.8))
                        .shadow(t.lg())
                        .cursor(CursorStyle::Arrow)
                        .group_opacity(rise)
                        .translate(px(0.), px(6. * (1. - rise)))
                        // Keeps the keyboard in the field.
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .child(body)
                        .when(key == "bar-picker", |d| {
                            d.child(crate::board::overlays::bounds_probe(cx.entity().downgrade(), |c, b| {
                                c.overlays.bar.picker_area[0] = Some(b)
                            }))
                        }),
                )
                .into_any_element(),
        )
    }

    fn render_slash_menu(
        &mut self,
        t: &Tokens,
        commands: &[nodus_board::compose::SlashCommand],
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let highlighted = self.overlays.bar.active.min(commands.len() - 1);
        let color = self.overlays.bar.color;
        let (_, ink) = view_hue(t);
        let options = commands.iter().enumerate().map(|(i, c)| {
            let target = c.target;
            let (hover, pick) = (cx.entity().downgrade(), cx.entity().downgrade());
            let glyph = match target {
                Target::View => icon(Icon::LayoutDashboard, 15., 2., ink).into_any_element(),
                Target::Note => div()
                    .size(px(14.))
                    .rounded(px(3.))
                    .bg(note_gradient(color))
                    .shadow(vec![inset_shadow(0., 0., 0., 0.5, rgba(0, 0, 0, 0.2))])
                    .into_any_element(),
            };
            div()
                .id(("bar-option", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(6.))
                .px(px(8.))
                .rounded(px(8.))
                .text_color(t.text_primary)
                .when(i == highlighted, |d| d.bg(t.bg_selected))
                .on_hover(move |h, _, cx| {
                    if *h && let Some(c) = hover.upgrade() {
                        c.update(cx, |c, cx| {
                            c.overlays.bar.active = i;
                            cx.notify();
                        });
                    }
                })
                .on_click(move |_, window, cx| {
                    if let Some(c) = pick.upgrade() {
                        c.update(cx, |c, cx| c.bar_pick(target, window, cx));
                    }
                })
                .child(
                    div()
                        .flex_none()
                        .size(px(28.))
                        .rounded(px(7.))
                        .bg(t.bg_hover)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(glyph),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(52.))
                        .font_family(fonts::MONO)
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(c.command),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.text_secondary)
                        .child(c.description),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(px(340.))
            .p(px(5.))
            .children(options)
            .into_any_element()
    }

    /// The eight swatches; a pick sets the colour, closes, refocuses the field.
    fn render_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let on_pick: crate::components::ColorHandler = Rc::new(move |color, window, cx| {
            if let Some(c) = me.upgrade() {
                c.update(cx, |c, cx| {
                    c.overlays.bar.color = color;
                    c.overlays.bar.picking = false;
                    c.bar_focus_field(window, cx);
                    cx.notify();
                });
            }
        });
        div()
            .py(px(8.))
            .px(px(10.))
            .child(color_swatches(self.overlays.bar.color, &on_pick, window, cx))
            .into_any_element()
    }
}
