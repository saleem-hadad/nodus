//! Scroll areas with Chromium's classic scrollbar as `global.css` styles it
//! (UI-011): while the content overflows, a 10px gutter that takes layout
//! space, with a 4px `--border-strong` pill thumb inset 3px; no track, no
//! arrows. Areas marked "scrollbar hidden" scroll with none.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, ScrollHandle,
    StatefulInteractiveElement, Styled, Window, canvas, div, px,
};

use crate::theme;

/// The gutter's width.
pub const GUTTER: f32 = 10.0;

/// Whether the area overflowed last frame, kept per area.
#[derive(Default, Clone, Copy, PartialEq)]
struct Overflow {
    overflowing: bool,
}

/// A vertically scrolling area that takes the rest of its flex column:
/// `content` is laid out in `padding` (top, right, bottom, left), and the
/// scrollbar takes its gutter at the right edge while the content overflows.
pub fn scroll_area(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    padding: [f32; 4],
    content: impl IntoElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id = id.into();
    let t = theme::tokens(cx);
    let state = window.use_keyed_state((id.clone(), "overflow"), cx, |_, _| Overflow::default());
    let overflowing = state.read(cx).overflowing;
    let [top, right, bottom, left] = padding;
    let max = handle.max_offset().y;
    let offset = -handle.offset().y;
    let viewport = handle.bounds().size.height;
    let watch = handle.clone();
    let view = window.current_view();

    div()
        .relative()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .id(id.clone())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(handle)
                .pt(px(top))
                .pb(px(bottom))
                .pl(px(left))
                .pr(px(right + if overflowing { GUTTER } else { 0. }))
                .child(content),
        )
        .when(overflowing && viewport > px(0.), |d| {
            let content_h = viewport + max;
            let thumb_h = (viewport * (viewport / content_h)).max(px(18.));
            let travel = viewport - thumb_h;
            let thumb_top = if max > px(0.) {
                travel * (f32::from(offset) / f32::from(max))
            } else {
                px(0.)
            };
            d.child(
                div()
                    .absolute()
                    .top(thumb_top + px(3.))
                    .right(px(3.))
                    .w(px(GUTTER - 6.))
                    .h(thumb_h - px(6.))
                    .rounded(px(10.))
                    .bg(t.border_strong),
            )
        })
        // Re-renders when the content starts or stops overflowing.
        .child(
            canvas(
                move |_: Bounds<Pixels>, _, cx| {
                    let now = watch.max_offset().y > px(0.5);
                    let changed = state.update(cx, |s, _| {
                        let changed = s.overflowing != now;
                        s.overflowing = now;
                        changed
                    });
                    if changed {
                        cx.notify(view);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}
