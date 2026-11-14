//! What's stacked above the bar's row (BAR-002, BOARD-138): the toast, then
//! the selection bar (`SelectionBar.tsx`, spec 11 SEL). Each rises in from
//! 6px below over 220ms, and sinks back as it goes (MOTION-011).

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window, div, px};

use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::button::button;
use crate::components::frosted::frosted;
use crate::components::icon::Icon;
use crate::components::presence::inert;
use crate::components::tween::enter;
use crate::platform::keys;
use crate::theme::tokens::{DUR_MED, RADIUS_MD, TEXT_MD, TEXT_SM};
use crate::theme::{self, fonts, motion};

impl BoardCanvas {
    /// The toast and the selection bar, top to bottom.
    pub(crate) fn render_above(
        &mut self,
        col_w: f32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(toast) = self.render_bar_toast(col_w, window, cx) {
            out.push(toast);
        }
        if let Some(bar) = self.render_selection_bar(window, cx) {
            out.push(bar);
        }
        if let Some(notice) = self.render_bar_notice(window, cx) {
            out.push(notice);
        }
        out
    }

    /// BAR-032: the board's toast, at the top of the column.
    fn render_bar_toast(&mut self, col_w: f32, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let open = self.toast.clone();
        let ((message, seq), leaving) = self.overlays.toast.frame("bar-toast-drawn", open, window, cx)?;
        let t = theme::tokens(cx);
        let rise = leaving.unwrap_or_else(|| enter(window, cx, ("bar-toast", seq as usize), DUR_MED, motion::ease_out));
        let toast = div()
            .id(("bar-toast", seq as usize))
            .occlude()
            .max_w(px(560f32.min(col_w - 48.)))
            .py(px(9.))
            .px(px(14.))
            .rounded(RADIUS_MD)
            .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.92), 20., 1.0))
            .shadow(t.lg())
            .text_size(TEXT_SM)
            .line_height(px(16.8))
            .text_color(t.text_primary)
            .group_opacity(rise)
            .translate(px(0.), px(6. * (1. - rise)))
            .child(message);
        Some(if leaving.is_some() {
            inert(toast).into_any_element()
        } else {
            toast.into_any_element()
        })
    }

    /// SEL-001…SEL-006.
    fn render_selection_bar(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let open = self.selection_bar();
        let (counts, leaving) = self
            .overlays
            .selection_bar
            .frame("selection-bar-drawn", open, window, cx)?;
        let t = theme::tokens(cx);
        let rise = leaving.unwrap_or_else(|| enter(window, cx, "selection-bar-rise", DUR_MED, motion::ease_out));
        let me = cx.entity().downgrade();
        let (merge, group, frame, archive, clear) = (me.clone(), me.clone(), me.clone(), me.clone(), me);
        let bar = div()
            .id(if leaving.is_none() {
                "selection-bar"
            } else {
                "selection-bar-leaving"
            })
            .occlude()
            .flex()
            .items_center()
            .gap(px(4.))
            .pt(px(6.))
            .pb(px(6.))
            .pr(px(6.))
            .pl(px(16.))
            .rounded(px(14.))
            .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.86), 24., 1.8))
            .shadow(t.lg())
            .group_opacity(rise)
            .translate(px(0.), px(6. * (1. - rise)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .font_family(fonts::UI)
                    .text_size(TEXT_MD)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_features(fonts::tabular())
                    .whitespace_nowrap()
                    .child(format!("{} selected", counts.count)),
            )
            .child(div().w(px(1.)).h(px(18.)).ml(px(10.)).mr(px(8.)).bg(t.separator))
            .child(
                div().mr(px(2.)).child(
                    button("selection-merge")
                        .primary()
                        .height(px(30.))
                        .icon(Icon::Sparkles, 14., 2.2)
                        .label("New session from context")
                        .disabled(counts.session_count == 0)
                        .on_click(move |_, _, cx| {
                            if let Some(c) = merge.upgrade() {
                                c.update(cx, |c, cx| {
                                    let ids = c.selected_cards();
                                    c.start_merge(&ids, None, cx);
                                });
                            }
                        }),
                ),
            )
            .child(if counts.one_group {
                // GROUP-021: one group selected: it comes apart.
                button("selection-group")
                    .ghost()
                    .icon(Icon::Ungroup, 14., 2.)
                    .label("Ungroup")
                    .tooltip(format!("Ungroup ({})", keys("⇧⌘G")))
                    .on_click(move |_, _, cx| {
                        if let Some(c) = group.upgrade() {
                            c.update(cx, |c, cx| {
                                let ids = c.selected_things();
                                c.ungroup_things(&ids, cx);
                            });
                        }
                    })
            } else {
                button("selection-group")
                    .ghost()
                    .icon(Icon::Group, 14., 2.)
                    .label("Group")
                    .tooltip(format!("Group, to move them together ({})", keys("⌘G")))
                    .disabled(counts.count < 2)
                    .on_click(move |_, _, cx| {
                        if let Some(c) = group.upgrade() {
                            c.update(cx, |c, cx| {
                                let ids = c.selected_things();
                                c.group_things(&ids, cx);
                            });
                        }
                    })
            })
            .child(
                button("selection-frame")
                    .ghost()
                    .icon(Icon::Frame, 14., 2.)
                    .label("Frame")
                    .tooltip(format!("Put in a frame Claude names ({})", keys("⌥⌘G")))
                    .on_click(move |_, _, cx| {
                        if let Some(c) = frame.upgrade() {
                            c.update(cx, |c, cx| {
                                let ids = c.selected_things();
                                c.frame_things(&ids, cx);
                            });
                        }
                    }),
            )
            .child(
                button("selection-archive")
                    .ghost()
                    .icon(Icon::Archive, 14., 2.)
                    .label("Archive")
                    .tooltip(format!("Archive selected ({})", keys("⌫")))
                    .disabled(counts.active_count == 0)
                    .on_click(move |_, _, cx| {
                        if let Some(c) = archive.upgrade() {
                            c.update(cx, |c, cx| {
                                let ids = c.selected_cards();
                                c.archive_cards(&ids, cx);
                            });
                        }
                    }),
            )
            .child(
                button("selection-clear")
                    .ghost()
                    .square()
                    .icon(Icon::X, 15., 2.)
                    .tooltip("Clear selection (Esc)")
                    .on_click(move |_, _, cx| {
                        if let Some(c) = clear.upgrade() {
                            c.update(cx, BoardCanvas::clear_selection);
                        }
                    }),
            );
        Some(if leaving.is_some() {
            inert(bar).into_any_element()
        } else {
            bar.into_any_element()
        })
    }
}
