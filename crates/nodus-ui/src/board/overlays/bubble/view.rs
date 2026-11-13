//! Drawing asks on the spot (BUBBLE-004…BUBBLE-008, BUBBLE-030,
//! BUBBLE-043): the bubble, a frosted card whose square corner points at its
//! spot; the markers that wait on their spots while Claude decides; the
//! mic's pill while it listens; and the two tools' cursors, drawn from their
//! dock icons.

use std::sync::OnceLock;

use gpui::component::Sizable as _;
use gpui::component::input::Textarea;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, CursorBitmap, CursorImage, CursorStyle, CustomCursor, Focusable as _, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Rgba, StatefulInteractiveElement, Styled, Window, div, point, px, size,
};
use nodus_domain::js;
use nodus_voice::controller::Phase;

use super::super::ink::cursor::rasterize;
use super::super::{kbd, spinner};
use crate::board::canvas::BoardCanvas;
use crate::board::hud::rgba_mix;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::presence::{self, inert};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::theme::tokens::{DUR_FAST, TEXT_SM, Tokens};
use crate::theme::{self, motion};
use crate::voice::{NoticeAnchor, Z, voice_notice, voice_status};

/// How wide the bubble is, and how far its square corner sits from the spot.
const WIDTH: f32 = 360.;
const OFFSET: f32 = 6.;
/// The room it keeps from the board's edges.
const MARGIN: f32 = 12.;
/// About how tall it is with one line: under this much room above the
/// spot, it opens below it.
const ONE_LINE_H: f32 = 76.;
/// How wide a marker's words get.
const MARKER_W: f32 = 240.;

/// BUBBLE-005: the bubble tool's dock icon (lucide `message-circle-more`),
/// white with a dark outline so it reads on either board; the tip of its
/// tail is the hotspot.
pub(crate) const CURSOR_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><path d='M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0-4.777-4.719z' fill='white' stroke='black' stroke-opacity='0.85' stroke-width='1.5' stroke-linejoin='round'/><circle cx='8' cy='12' r='1.25' fill-opacity='0.85'/><circle cx='12' cy='12' r='1.25' fill-opacity='0.85'/><circle cx='16' cy='12' r='1.25' fill-opacity='0.85'/></svg>";

/// BUBBLE-005: the mic tool's dock icon (lucide `mic`) the same way, its
/// stand haloed in white; the foot of the stand is the hotspot.
pub(crate) const MIC_CURSOR_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><g fill='none' stroke='white' stroke-width='4' stroke-linecap='round'><path d='M19 10v2a7 7 0 0 1-14 0v-2'/><path d='M12 19v3'/></g><g fill='none' stroke='black' stroke-opacity='0.85' stroke-width='1.75' stroke-linecap='round'><path d='M19 10v2a7 7 0 0 1-14 0v-2'/><path d='M12 19v3'/></g><rect x='9' y='2' width='6' height='13' rx='3' fill='white' stroke='black' stroke-opacity='0.85' stroke-width='1.5'/></svg>";

/// An image cursor from a 24×24 SVG, registered once into `slot`; the
/// arrow where an image cursor can't be made.
fn image_cursor(slot: &'static OnceLock<CursorStyle>, svg: &str, hotspot: (f32, f32)) -> CursorStyle {
    *slot.get_or_init(|| {
        let bitmaps: Vec<CursorBitmap> = [1.0, 2.0].into_iter().filter_map(|s| rasterize(svg, 24, s)).collect();
        if bitmaps.is_empty() {
            return CursorStyle::Arrow;
        }
        CursorStyle::Custom(CustomCursor::register(CursorImage {
            size: size(24.0, 24.0),
            hotspot: point(hotspot.0, hotspot.1),
            bitmaps,
            fallback: CursorStyle::Arrow,
        }))
    })
}

/// The bubble tool's cursor.
pub(crate) fn cursor() -> CursorStyle {
    static CURSOR: OnceLock<CursorStyle> = OnceLock::new();
    image_cursor(&CURSOR, CURSOR_SVG, (2.5, 21.5))
}

/// The mic tool's cursor.
pub(crate) fn mic_cursor() -> CursorStyle {
    static CURSOR: OnceLock<CursorStyle> = OnceLock::new();
    image_cursor(&CURSOR, MIC_CURSOR_SVG, (12.0, 22.0))
}

impl BoardCanvas {
    /// Everything asked on the spot: the markers waiting on Claude, the
    /// mic's pill, and the bubble.
    pub(crate) fn render_bubble(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let mut out = self.render_asking(window, cx);
        out.extend(self.render_voice(window, cx));
        out.extend(self.render_bubble_card(window, cx));
        out
    }

    /// A board point on the canvas, in pixels.
    fn on_canvas(&self, p: nodus_board::geometry::Point) -> (f32, f32) {
        let vp = self.viewport;
        ((vp.x + p.x * vp.zoom) as f32, (vp.y + p.y * vp.zoom) as f32)
    }

    /// BUBBLE-004: the bubble beside its spot, above and to the right of it,
    /// or below or to the left where the board runs out, with a dot on the
    /// spot; MOTION-011: a bubble closed or sent fades from its spot.
    fn render_bubble_card(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let leaving = presence::step(&mut self.overlays.bubble_leaving, window).map(|l| (l.what.clone(), l.shown()));
        if let Some((look, shown)) = leaving {
            out.extend(self.draw_bubble(&look, None, shown, cx));
        }
        // BUBBLE-003: the keyboard went elsewhere, as the frame has it (the
        // field's blur doesn't fire for a focus given from outside). An
        // empty bubble goes; one with words stays.
        let Some(bubble) = self.overlays.bubble.as_mut() else {
            return out;
        };
        let focused = bubble.field.read(cx).focus_handle(cx).is_focused(window);
        bubble.had_focus |= focused;
        if !focused && bubble.had_focus && js::trim(&bubble.text).is_empty() {
            self.close_bubble(None, cx);
            // Its exit starts this frame.
            if let Some(leaving) = &self.overlays.bubble_leaving {
                let (look, shown) = (leaving.what.clone(), leaving.shown());
                window.request_animation_frame();
                out.extend(self.draw_bubble(&look, None, shown, cx));
            }
            return out;
        }
        let Some(bubble) = self.overlays.bubble.as_ref() else {
            return out;
        };
        let look = bubble.look();
        let field = bubble.field.clone();
        let rise = enter(window, cx, ("bubble-in", look.key), DUR_FAST, motion::ease_out);
        out.extend(self.draw_bubble(&look, Some(field), rise, cx));
        out
    }

    /// A bubble `rise` of the way in: with its field while it's open, else
    /// its words as they were, inert, as it leaves.
    fn draw_bubble(
        &mut self,
        look: &super::BubbleLook,
        field: Option<gpui::Entity<gpui::component::input::TextareaState>>,
        rise: f32,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let t = theme::tokens(cx);
        let live = field.is_some();
        let key = look.key;
        let typed = !js::trim(&look.text).is_empty();
        let (w, h) = self.canvas_size();
        let (w, h) = (w as f32, h as f32);
        let (sx, sy) = self.on_canvas(look.at);
        let above = sy - OFFSET - ONE_LINE_H >= MARGIN;
        let right = sx + OFFSET + WIDTH <= w - MARGIN;
        let left = if right {
            sx + OFFSET
        } else {
            (sx - OFFSET - WIDTH).max(MARGIN)
        };
        let me = cx.entity().downgrade();
        let hint = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(TEXT_SM)
            .line_height(px(16.))
            .text_color(t.text_tertiary)
            .child(icon(Icon::Sparkles, 12., 2., t.text_tertiary))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child("Claude decides: a session, a note or a view"),
            )
            .when(typed, |d| d.child(kbd(&t, "↵")));
        let card = div()
            .id(if live { ("bubble", key) } else { ("bubble-leaving", key) })
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
            .cursor(CursorStyle::IBeam)
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
            .when(live, |d| {
                d.capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                    if this.bubble_key_down(e, window, cx) {
                        cx.stop_propagation();
                    }
                }))
            })
            .child(match field {
                // XSmall's editor insets are 4px across: taken back.
                Some(field) => div()
                    .mx(px(-4.))
                    .child(
                        Textarea::new(&field)
                            .appearance(false)
                            .xsmall()
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .py(px(4.))
                            .px(px(0.)),
                    )
                    .into_any_element(),
                // The words as the field showed them (it went as it closed).
                None => div()
                    .py(px(4.))
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(if typed { t.text_primary } else { t.text_tertiary })
                    .child(if typed {
                        look.text.clone()
                    } else {
                        "Ask Claude, jot a note, or describe a view".to_owned()
                    })
                    .into_any_element(),
            })
            .child(hint);
        let dot = spot_dot(sx, sy, t.accent, rise);
        if live {
            return vec![dot, card.into_any_element()];
        }
        vec![inert(dot).into_any_element(), inert(card).into_any_element()]
    }

    /// BUBBLE-030: each ask waiting on Claude, as a dot on its spot and a
    /// spinner beside the first line of what was asked.
    fn render_asking(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let t = theme::tokens(cx);
        let asks: Vec<(usize, (f32, f32), String)> = self
            .overlays
            .asking
            .iter()
            .map(|a| (a.id, self.on_canvas(a.at), a.text.clone()))
            .collect();
        let mut out = Vec::new();
        for (id, (sx, sy), text) in asks {
            let rise = enter(window, cx, ("bubble-asking-in", id), DUR_FAST, motion::ease_out);
            let first_line = text.lines().next().unwrap_or_default().to_owned();
            out.push(spot_dot(sx, sy, t.accent, rise));
            out.push(
                pill(&t, sx, sy, rise)
                    .id(("bubble-asking", id))
                    .max_w(px(MARKER_W))
                    .child(spinner("bubble-asking-spin", 12., 2., t.text_secondary, cx))
                    .child(div().min_w_0().overflow_hidden().text_ellipsis().child(first_line))
                    .into_any_element(),
            );
        }
        out
    }

    /// BUBBLE-043: while the mic tool listens or writes down, a pill on its
    /// spot with the clock and meter (or the spinner), and its notice under
    /// it when something went wrong.
    fn render_voice(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let (Some(mic), Some(at)) = (self.overlays.voice.clone(), self.overlays.voice_at) else {
            return Vec::new();
        };
        let t = theme::tokens(cx);
        let (sx, sy) = self.on_canvas(at);
        let phase = mic.read(cx).phase();
        let mut out = Vec::new();
        if phase != Phase::Idle {
            let recording = phase == Phase::Recording;
            let rise = enter(window, cx, "bubble-voice-in", DUR_FAST, motion::ease_out);
            let status = voice_status("bubble-voice-status", &mic, false, Z(1.), window, cx);
            let hint = match phase {
                Phase::Recording => Some("Click to send"),
                Phase::Opening => Some("Opening the mic…"),
                _ => None,
            };
            let cancel = mic.clone();
            let dot = if recording { t.status_waiting } else { t.accent };
            out.push(spot_dot(sx, sy, dot, rise));
            out.push(
                pill(&t, sx, sy, rise)
                    .id("bubble-voice")
                    .children(status)
                    .when_some(hint, |d, hint| d.child(div().text_color(t.text_tertiary).child(hint)))
                    .when(phase != Phase::Transcribing, |d| {
                        d.child(
                            div()
                                .id("bubble-voice-cancel")
                                .flex_none()
                                .size(px(20.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(t.bg_hover))
                                .cursor(CursorStyle::Arrow)
                                .tooltip(native_tooltip("Cancel (Esc)".into()))
                                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                    cx.stop_propagation();
                                    window.prevent_default();
                                })
                                .on_click(move |_, _, cx| cancel.read(cx).cancel())
                                .child(icon(Icon::X, 11., 2.4, t.text_tertiary)),
                        )
                    })
                    .into_any_element(),
            );
        }
        if let Some(notice) = voice_notice("bubble-voice-notice", &mic, NoticeAnchor::SessionMic, Z(1.), window, cx) {
            out.push(
                div()
                    .absolute()
                    .left(px(sx + 10.))
                    .top(px(sy + 20.))
                    .occlude()
                    .child(notice)
                    .into_any_element(),
            );
        }
        out
    }
}

/// A dot on a spot: 8px, with a 3px ring of its colour at 25%.
fn spot_dot(sx: f32, sy: f32, color: Rgba, rise: f32) -> AnyElement {
    div()
        .absolute()
        .left(px(sx - 4.))
        .top(px(sy - 4.))
        .size(px(8.))
        .rounded_full()
        .bg(color)
        .shadow(vec![theme::tokens::ring(3., rgba_mix(color, 0.25))])
        .opacity(rise)
        .into_any_element()
}

/// A small frosted pill just right of a spot, level with it.
fn pill(t: &Tokens, sx: f32, sy: f32, rise: f32) -> gpui::Div {
    div()
        .absolute()
        .left(px(sx + 12. + 4. * (1. - rise)))
        .top(px(sy - 14.))
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .rounded(px(14.))
        .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.92), 24., 1.8))
        .shadow(t.md())
        .text_size(TEXT_SM)
        .text_color(t.text_secondary)
        .whitespace_nowrap()
        .opacity(rise)
        .occlude()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(b: &CursorBitmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * b.width + x) * 4) as usize;
        [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2], b.rgba[i + 3]]
    }

    #[test]
    fn the_bubble_cursor_is_the_dock_icon_filled_white() {
        let two = rasterize(CURSOR_SVG, 24, 2.0).unwrap();
        assert_eq!((two.width, two.height), (48, 48));
        // Inside the bubble, between the dots: white.
        assert_eq!(at(&two, 20, 17), [255, 255, 255, 255]);
        // A dot: dark.
        assert!(at(&two, 24, 24)[0] < 80);
        // Outside it, top left: clear.
        assert_eq!(at(&two, 1, 1)[3], 0);
    }

    #[test]
    fn the_mic_cursor_is_the_dock_icon_filled_white() {
        let two = rasterize(MIC_CURSOR_SVG, 24, 2.0).unwrap();
        // Inside the capsule: white.
        assert_eq!(at(&two, 24, 12), [255, 255, 255, 255]);
        // Its outline, and the stand: dark.
        assert!(at(&two, 18, 12)[0] < 100);
        assert!(at(&two, 24, 42)[0] < 100);
        // Beside the capsule, above the stand's arc: clear.
        assert_eq!(at(&two, 4, 4)[3], 0);
    }

    #[test]
    fn each_cursor_is_an_image_falling_back_to_the_arrow() {
        for (style, hotspot) in [(cursor(), point(2.5, 21.5)), (mic_cursor(), point(12.0, 22.0))] {
            let CursorStyle::Custom(c) = style else {
                panic!("not an image cursor")
            };
            let image = c.image().unwrap();
            assert_eq!(image.hotspot, hotspot);
            assert_eq!(image.fallback, CursorStyle::Arrow);
        }
    }
}
