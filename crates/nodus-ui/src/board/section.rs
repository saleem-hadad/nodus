//! A section's frame (`SectionNode.tsx`, `board.css` › sections): the tinted
//! frame around its cards, its header with the name, count and status, the
//! empty placeholder, the rename field, and the "Naming…" state while Claude
//! names it (BOARD-045, BOARD-080…BOARD-098; spec 09 § UI details › Sections).

use std::cell::Cell;
use std::rc::Rc;

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Rgba, Styled, Window, div,
    px,
};
use nodus_domain::{BoardSection, SessionStatus};

use super::canvas::BoardCanvas;
use super::layout::{SECTION_HEADER_H, SECTION_PAD};
use super::units::fu;
use crate::components::folder_icon::folder_hues;
use crate::components::status_dot::status_dot;
use crate::components::text_field::{focus_all, input, new_input};
use crate::components::tween::transition;
use crate::theme::tokens::DUR_MED;
use crate::theme::{self, motion};

/// `color-mix(in srgb, a p, b)`: premultiplied, as Chromium mixes.
pub fn mix(a: Rgba, p: f32, b: Rgba) -> Rgba {
    let alpha = p * a.a + (1.0 - p) * b.a;
    if alpha <= 0.0 {
        return Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        };
    }
    let ch = |x: f32, y: f32| (p * a.a * x + (1.0 - p) * b.a * y) / alpha;
    Rgba {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: alpha,
    }
}

/// `color-mix(in srgb, c p, transparent)`.
pub fn fade(c: Rgba, p: f32) -> Rgba {
    Rgba { a: c.a * p, ..c }
}

/// The label stays legible when zoomed out (`labelScale`).
pub fn label_scale(zoom: f32) -> f32 {
    (1.0 / zoom).clamp(1.0, 2.2)
}

/// What a section node shows.
pub(crate) struct SectionProps {
    pub section: BoardSection,
    pub count: usize,
    pub status: Option<SessionStatus>,
    pub hot: bool,
    pub zoom: f32,
    /// None while Claude names it, then the name it gave while that lands;
    /// absent when it isn't being named.
    pub naming: Option<Option<String>>,
}

impl BoardCanvas {
    /// The rename field for a section, made when its renaming starts.
    fn section_field(
        &mut self,
        section: &BoardSection,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<InputState> {
        if let Some((id, field)) = &self.section_field
            && id == &section.id
        {
            return field.clone();
        }
        let field = new_input(&section.name, "Section name", false, window, cx);
        let initial = section.name.clone();
        let id = section.id.clone();
        let done = Rc::new(Cell::new(false));
        // BOARD-093: Enter or leaving commits once; Escape cancels.
        let sub = cx.subscribe_in(&field, window, move |canvas, field, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) && !done.replace(true) {
                let value = field.read(cx).value().to_string();
                let trimmed = nodus_domain::js::trim(&value).to_owned();
                if trimmed != initial {
                    canvas.rename_section(&id, &trimmed, cx);
                }
                canvas.section_field = None;
                canvas.set_renaming(None, cx);
            }
        });
        self.section_field_sub = Some(sub);
        focus_all(&field, window, cx);
        self.section_field = Some((section.id.clone(), field.clone()));
        field
    }

    /// Escape in the section's rename field.
    pub(crate) fn cancel_section_rename(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.section_field.take().is_some() {
            self.section_field_sub = None;
            self.set_renaming(None, cx);
            window.blur(cx);
        }
    }

    /// The section's frame and header, in flow units (drawn in `Zoomed`).
    pub(crate) fn render_section(
        &mut self,
        props: &SectionProps,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = theme::tokens(cx);
        let hue = folder_hues(props.section.color).back_bottom;
        let id = props.section.id.clone();
        let renaming = self.renaming.as_deref() == Some(id.as_str());
        let s = label_scale(props.zoom);
        let border = transition(
            window,
            cx,
            crate::components::child_id(gpui::ElementId::Name(id.clone().into()), "border"),
            if props.hot { hue } else { fade(hue, 0.3) },
            DUR_MED,
            motion::ease,
        );
        let fill = transition(
            window,
            cx,
            crate::components::child_id(gpui::ElementId::Name(id.clone().into()), "fill"),
            fade(hue, if props.hot { 0.12 } else { 0.06 }),
            DUR_MED,
            motion::ease,
        );
        let header_color = mix(hue, 0.68, t.text_primary);

        let name: AnyElement = if renaming {
            let field = self.section_field(&props.section, window, cx);
            let value = field.read(cx).value().to_string();
            let chars = nodus_domain::js::utf16_len(&value).max(13) + 1;
            div()
                .flex_none()
                .h(fu(24. * s))
                .my(fu(-2. * s))
                .px(fu(6. * s))
                .rounded(fu(6. * s))
                .bg(t.bg_window)
                .shadow(vec![theme::tokens::ring(2. * props.zoom * s, hue)])
                .text_color(t.text_primary)
                // `size = max(14, length + 1)`: Chromium sizes it by the font's average
                // character width (SF Pro at 13px: ~10.3px), plus the padding.
                .w(fu((chars as f32 * 10.3 + 12.) * s))
                // BOARD-093: the field keeps its keys and presses; Escape cancels.
                .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                    // Marked, not stopped, so the field still types the key
                    // (cards/note.rs).
                    window.prevent_default();
                    if e.keystroke.key == "escape" {
                        this.cancel_section_rename(window, cx);
                    }
                }))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(input(&field, px(24. * props.zoom * s), false))
                .into_any_element()
        } else {
            match &props.naming {
                Some(None) => super::naming::naming_label(&id, props.zoom, s, header_color, window, cx),
                Some(Some(landed)) if !landed.is_empty() => super::naming::reveal(&id, landed, props.zoom, window, cx),
                _ => {
                    let untitled = props.section.name.is_empty();
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .when(untitled, |d| d.font_weight(FontWeight::MEDIUM).opacity(0.7))
                        .child(if untitled {
                            "Untitled section".to_owned()
                        } else {
                            props.section.name.clone()
                        })
                        .into_any_element()
                }
            }
        };

        let label = div()
            .flex()
            .items_center()
            .gap(fu(8. * s))
            .min_w_0()
            .text_size(fu(13. * s))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(name)
            .child(
                div()
                    .flex_none()
                    .px(fu(7. * s))
                    .rounded_full()
                    .bg(fade(hue, 0.14))
                    .text_size(fu(11. * s))
                    .line_height(fu(18. * s))
                    .child(props.count.to_string()),
            )
            .when_some(props.status, |d, status| {
                d.child(
                    status_dot(
                        crate::components::child_id(gpui::ElementId::Name(id.clone().into()), "status"),
                        status,
                    )
                    .size(px(8. * props.zoom * s)),
                )
            });

        let glow = props
            .naming
            .as_ref()
            .map(|naming| super::naming::glow(&id, props.zoom, naming.is_some(), window, cx));
        div()
            .relative()
            .size_full()
            .rounded(fu(22.))
            .border(fu(1.5))
            .border_color(border)
            .bg(fill)
            .children(glow)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(fu(SECTION_HEADER_H as f32))
                    .px(fu(18.))
                    .text_color(header_color)
                    .child(label),
            )
            .when(props.count == 0, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(fu(SECTION_PAD as f32))
                        .top(fu(SECTION_HEADER_H as f32))
                        .w(fu(288.))
                        .h(fu(216.))
                        .rounded(fu(14.))
                        .border(fu(1.5))
                        .border_dashed()
                        .border_color(fade(hue, 0.35))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(fu(12.))
                        .text_color(mix(hue, 0.55, t.text_tertiary))
                        .child("Drag cards here"),
                )
            })
            .into_any_element()
    }
}
