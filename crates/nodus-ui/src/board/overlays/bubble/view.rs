//! Drawing the bubble (BUBBLE-004…BUBBLE-008): a frosted card whose square
//! corner points at the spot it was opened on, its field, and a footer
//! saying what it will make; and the chat-bubble cursor over the empty board.

use std::sync::OnceLock;

use gpui::component::Sizable as _;
use gpui::component::input::{IndentInline, Textarea};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, CursorBitmap, CursorImage, CursorStyle, CustomCursor, Focusable as _, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Rgba, StatefulInteractiveElement, Styled, Window, div, point, px, size,
};
use nodus_domain::{BubbleRoute, FolderColor, js};

use super::super::bar::{note_gradient, view_hue};
use super::super::ink::cursor::rasterize;
use super::super::{kbd, spinner};
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::theme::tokens::{DUR_FAST, TEXT_SM, Tokens, inset_shadow, rgba};
use crate::theme::{self, motion};

/// How wide it is, and how far its square corner sits from the spot.
const WIDTH: f32 = 360.;
const OFFSET: f32 = 6.;
/// The room it keeps from the board's edges.
const MARGIN: f32 = 12.;
/// About how tall it is with one line: under this much room above the
/// spot, it opens below it.
const ONE_LINE_H: f32 = 84.;

/// BUBBLE-005: a speech bubble, white with a dark outline so it reads on
/// either board, its tail's tip the hotspot.
pub(crate) const CURSOR_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><path d='M9 3.5h9.5a3.5 3.5 0 0 1 3.5 3.5v5a3.5 3.5 0 0 1-3.5 3.5h-7.5l-7.5 5.5 2.6-6.1A3.5 3.5 0 0 1 5.5 12V7A3.5 3.5 0 0 1 9 3.5z' fill='white' stroke='black' stroke-opacity='0.85' stroke-width='1.4' stroke-linejoin='round'/><circle cx='10' cy='9.5' r='1.1'/><circle cx='13.75' cy='9.5' r='1.1'/><circle cx='17.5' cy='9.5' r='1.1'/></svg>";

/// The bubble's cursor, registered the first time it's asked for; the arrow
/// where an image cursor can't be made.
pub(crate) fn cursor() -> CursorStyle {
    static CURSOR: OnceLock<CursorStyle> = OnceLock::new();
    *CURSOR.get_or_init(|| {
        let bitmaps: Vec<CursorBitmap> = [1.0, 2.0]
            .into_iter()
            .filter_map(|s| rasterize(CURSOR_SVG, 24, s))
            .collect();
        if bitmaps.is_empty() {
            return CursorStyle::Arrow;
        }
        CursorStyle::Custom(CustomCursor::register(CursorImage {
            size: size(24.0, 24.0),
            hotspot: point(3.5, 21.0),
            bitmaps,
            fallback: CursorStyle::Arrow,
        }))
    })
}

impl BoardCanvas {
    /// BUBBLE-004: the bubble beside the spot it was opened on, above and to
    /// the right of it, or below or to the left where the board runs out,
    /// with a dot on the spot.
    pub(crate) fn render_bubble(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        // BUBBLE-003: the keyboard went elsewhere, as the frame has it (the
        // field's blur doesn't fire for a focus given from outside, such as
        // a new note's).
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return Vec::new();
        };
        if bubble.field.read(cx).focus_handle(cx).is_focused(window) {
            bubble.had_focus = true;
        } else if bubble.had_focus && !bubble.sending {
            self.close_bubble(None, cx);
            return Vec::new();
        }
        let Some(bubble) = self.overlays.bubble.as_ref() else {
            return Vec::new();
        };
        let t = theme::tokens(cx);
        let key = bubble.key;
        let field = bubble.field.clone();
        let (w, h) = self.canvas_size();
        let (w, h) = (w as f32, h as f32);
        let vp = self.viewport;
        let sx = (vp.x + bubble.at.x * vp.zoom) as f32;
        let sy = (vp.y + bubble.at.y * vp.zoom) as f32;
        let above = sy - OFFSET - ONE_LINE_H >= MARGIN;
        let right = sx + OFFSET + WIDTH <= w - MARGIN;
        let left = if right {
            sx + OFFSET
        } else {
            (sx - OFFSET - WIDTH).max(MARGIN)
        };
        let rise = enter(window, cx, ("bubble-in", key), DUR_FAST, motion::ease_out);
        let footer = self.render_bubble_footer(&t, window, cx);
        let me = cx.entity().downgrade();
        let card = div()
            .id(("bubble", key))
            .absolute()
            .left(px(left))
            .map(|d| {
                if above {
                    d.bottom(px(h - (sy - OFFSET) - 4. * (1. - rise)))
                } else {
                    d.top(px(sy + OFFSET + 4. * (1. - rise)))
                }
            })
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .gap(px(6.))
            .pt(px(8.))
            .pb(px(8.))
            .px(px(12.))
            .rounded(px(16.))
            // The corner nearest the spot is the bubble's point.
            .map(|d| match (above, right) {
                (true, true) => d.rounded_bl(px(4.)),
                (true, false) => d.rounded_br(px(4.)),
                (false, true) => d.rounded_tl(px(4.)),
                (false, false) => d.rounded_tr(px(4.)),
            })
            .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.92), 24., 1.8))
            .shadow(t.lg())
            .opacity(rise)
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                if let Some(c) = me.upgrade() {
                    c.update(cx, |c, cx| {
                        if let Some(b) = &c.overlays.bubble {
                            let handle = b.field.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        }
                    });
                }
            })
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                if this.bubble_key_down(e, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // BUBBLE-015: Tab comes as the field's IndentInline.
            .capture_action(cx.listener(|this, _: &IndentInline, _, cx| {
                this.bubble_cycle(cx);
                cx.stop_propagation();
            }))
            .child(
                // XSmall's editor insets are 4px across: taken back.
                div().mx(px(-4.)).cursor(CursorStyle::IBeam).child(
                    Textarea::new(&field)
                        .appearance(false)
                        .xsmall()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .py(px(4.))
                        .px(px(0.)),
                ),
            )
            .child(footer);
        let dot = div()
            .absolute()
            .left(px(sx - 4.))
            .top(px(sy - 4.))
            .size(px(8.))
            .rounded_full()
            .bg(t.accent)
            .shadow(vec![theme::tokens::ring(3., rgba_mix(t.accent, 0.25))])
            .opacity(rise);
        vec![dot.into_any_element(), card.into_any_element()]
    }

    /// BUBBLE-006, BUBBLE-007: what it will make, and why.
    fn render_bubble_footer(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let Some(bubble) = self.overlays.bubble.as_ref() else {
            return div().into_any_element();
        };
        let typed = !js::trim(&bubble.text).is_empty();
        let route = bubble.shown();
        // BUBBLE-007: Claude is looking at it, and nothing the user picked stands.
        let thinking = bubble.choice.is_none() && (bubble.guessing || bubble.sending);
        let (bg, fg, lead, label): (Rgba, Rgba, AnyElement, String) = match route {
            Some(BubbleRoute::Session(model)) => (
                rgba_mix(t.accent, 0.12),
                t.accent,
                icon(Icon::Sparkles, 12., 2., t.accent).into_any_element(),
                format!("Session · {}", model.label()),
            ),
            Some(BubbleRoute::Note) => (
                t.bg_hover,
                t.text_primary,
                div()
                    .flex_none()
                    .size(px(11.))
                    .rounded(px(3.))
                    .bg(note_gradient(FolderColor::Yellow))
                    .shadow(vec![inset_shadow(0., 0., 0., 0.5, rgba(0, 0, 0, 0.2))])
                    .into_any_element(),
                "Note".into(),
            ),
            Some(BubbleRoute::View) => {
                let (hue, ink) = view_hue(t);
                (
                    Rgba { a: 0.15, ..hue },
                    ink,
                    icon(Icon::LayoutDashboard, 12., 2., ink).into_any_element(),
                    "View".into(),
                )
            }
            None if thinking => (
                t.bg_hover,
                t.text_secondary,
                spinner("bubble-thinking", 12., 2., t.text_secondary, cx),
                "Thinking…".into(),
            ),
            None => (
                t.bg_hover,
                t.text_secondary,
                icon(Icon::Sparkles, 12., 2., t.text_secondary).into_any_element(),
                "Auto".into(),
            ),
        };
        // A guess on screen that's being asked again turns its mark to the spinner.
        let lead = if thinking && route.is_some() {
            spinner("bubble-thinking", 12., 2., fg, cx)
        } else {
            lead
        };
        let why = if !typed {
            "Claude picks a session, note or view".to_owned()
        } else if bubble.choice.is_some() {
            "Your pick".to_owned()
        } else if bubble.sending {
            "Starts once Claude picks".to_owned()
        } else {
            bubble.guess.as_ref().map(|g| g.reason.clone()).unwrap_or_default()
        };
        let hovered = window.use_keyed_state("bubble-chip-hover", cx, |_, _| false);
        let hover = *hovered.read(cx);
        let me = cx.entity().downgrade();
        let chip = div()
            .id("bubble-route")
            .flex_none()
            .h(px(22.))
            .pl(px(7.))
            .pr(px(9.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(11.))
            .bg(bg)
            .when(hover, |d| {
                d.shadow(vec![inset_shadow(0., 0., 0., 1., rgba_mix(fg, 0.35))])
            })
            .text_color(fg)
            .font_weight(gpui::FontWeight::MEDIUM)
            .whitespace_nowrap()
            .cursor(CursorStyle::PointingHand)
            .tooltip(native_tooltip("Change (Tab)".into()))
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
            })
            .on_click(move |_, _, cx| {
                if let Some(c) = me.upgrade() {
                    c.update(cx, BoardCanvas::bubble_cycle);
                }
            })
            .child(lead)
            .child(label);
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(TEXT_SM)
            .line_height(px(16.))
            .child(chip)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(t.text_tertiary)
                    .child(why),
            )
            .when(typed, |d| d.child(kbd(t, "↵")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_is_drawn_at_each_scale() {
        let one = rasterize(CURSOR_SVG, 24, 1.0).unwrap();
        assert_eq!((one.width, one.height), (24, 24));
        let two = rasterize(CURSOR_SVG, 24, 2.0).unwrap();
        assert_eq!((two.width, two.height), (48, 48));
        let at = |x: u32, y: u32| {
            let i = ((y * two.width + x) * 4) as usize;
            [two.rgba[i], two.rgba[i + 1], two.rgba[i + 2], two.rgba[i + 3]]
        };
        // Inside the bubble, between the dots: white.
        assert_eq!(at(24, 26), [255, 255, 255, 255]);
        // A dot: dark.
        assert!(at(20, 19)[0] < 60);
        // Outside it, top left: clear.
        assert_eq!(at(2, 2)[3], 0);
    }
}
