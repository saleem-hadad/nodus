//! The board's chrome over the canvas: the zoom pill (`BoardControls.tsx`),
//! the minimap, the empty-board hint, the toast and the confirm dialog
//! (`ConfirmDialog.tsx`) — spec 09 § UI details.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PathBuilder, Pixels, StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, size,
};
use nodus_domain::{Rect, SessionStatus, Viewport};

use super::canvas::BoardCanvas;
use super::scene::{FlowNode, NodeKind};
use crate::components::button::{Kind, button};
use crate::components::folder_icon::folder_hues;
use crate::components::frosted::frosted;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

/// The minimap's size (`style={{ width: 176, height: 118 }}`).
const MINIMAP_W: f32 = 176.0;
const MINIMAP_H: f32 = 118.0;

/// One of the zoom pill's buttons (`.board-control`).
fn control(
    id: &'static str,
    tooltip: &'static str,
    content: AnyElement,
    min_w: f32,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let hovered = window.use_keyed_state(crate::components::child_id(id, "hover"), cx, |_, _| false);
    let on = *hovered.read(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(min_w))
        .h(px(28.))
        .px(px(4.))
        .rounded(px(8.))
        .text_color(if on { t.text_primary } else { t.text_secondary })
        .when(on, |d| d.bg(t.bg_hover))
        .active(move |s| s.bg(t.bg_pressed))
        .on_hover(move |h, _, cx| {
            hovered.update(cx, |s, cx| {
                *s = *h;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| on_click(window, cx))
        .tooltip(native_tooltip(tooltip.into()))
        .child(content)
        .into_any_element()
}

impl BoardCanvas {
    /// The zoom pill, 16px from the bottom-left (z 5).
    pub(crate) fn render_zoom_pill(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let zoom = self.viewport.zoom;
        let me = cx.entity().downgrade();
        let (out, reset, zin, fit) = (me.clone(), me.clone(), me.clone(), me);
        let color = |hover: bool| if hover { t.text_primary } else { t.text_secondary };
        let minus = control(
            "zoom-out",
            "Zoom out",
            icon(Icon::Minus, 14., 2.2, color(false)).into_any_element(),
            28.,
            move |_, cx| {
                if let Some(c) = out.upgrade() {
                    c.update(cx, |c, cx| c.zoom_by(1.0 / 1.2, 160.0, cx));
                }
            },
            window,
            cx,
        );
        let label = control(
            "zoom-reset",
            "Reset to 100%",
            div()
                .text_size(TEXT_SM)
                .font_weight(FontWeight::MEDIUM)
                .child(format!("{}%", nodus_domain::js::round(zoom * 100.0)))
                .into_any_element(),
            50.,
            move |_, cx| {
                if let Some(c) = reset.upgrade() {
                    c.update(cx, |c, cx| c.zoom_to(1.0, 200.0, cx));
                }
            },
            window,
            cx,
        );
        let plus = control(
            "zoom-in",
            "Zoom in",
            icon(Icon::Plus, 14., 2.2, color(false)).into_any_element(),
            28.,
            move |_, cx| {
                if let Some(c) = zin.upgrade() {
                    c.update(cx, |c, cx| c.zoom_by(1.2, 160.0, cx));
                }
            },
            window,
            cx,
        );
        // BOARD-033: the click event is no duration, so this fit is instant.
        let fit = control(
            "zoom-fit",
            "Zoom to fit",
            icon(Icon::Maximize, 13., 2.2, color(false)).into_any_element(),
            28.,
            move |_, cx| {
                if let Some(c) = fit.upgrade() {
                    c.update(cx, |c, cx| c.fit_active(0.0, cx));
                }
            },
            window,
            cx,
        );
        div()
            .id("board-controls")
            .absolute()
            .left(px(16.))
            .bottom(px(16.))
            .occlude()
            .flex()
            .items_center()
            .h(px(34.))
            .px(px(3.))
            .rounded(px(11.))
            .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.86), 20., 1.6))
            .shadow(t.md())
            .font_family(fonts::UI)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(minus)
            .child(label)
            .child(plus)
            .child(div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(t.separator))
            .child(fit)
            .into_any_element()
    }

    /// The minimap, 16px from the bottom-right (z 5).
    pub(crate) fn render_minimap(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let (w, h) = self.canvas_size();
        let vp = self.viewport;
        let board = self.board.clone();
        let nodes: Vec<FlowNode> = self.nodes.iter().filter(|n| !n.hidden).cloned().collect();
        let me = cx.entity();
        let wheel = me.clone();
        div()
            .id("board-minimap")
            .absolute()
            .right(px(16.))
            .bottom(px(16.))
            .w(px(MINIMAP_W))
            .h(px(MINIMAP_H))
            .occlude()
            .rounded(px(12.))
            .overflow_hidden()
            .clip_rounded()
            .bg(t.bg_elevated)
            .shadow(t.md())
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                cx.stop_propagation();
                me.update(cx, |c, cx| c.minimap_press(e.position, cx));
            })
            .on_scroll(move |e, _, cx| {
                cx.stop_propagation();
                wheel.update(cx, |c, cx| c.minimap_wheel(e, cx));
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        paint_minimap(bounds, &nodes, board.as_ref(), vp, (w, h), &t, window);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    /// The empty-board hint (BOARD-146).
    pub(crate) fn render_hint(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let rise = enter(window, cx, "board-hint-rise", DUR_MED, motion::ease_out);
        let kbd = |k: &'static str| {
            div()
                .flex_none()
                .min_w(px(18.))
                .px(px(5.))
                .mx(px(1.))
                .rounded(px(4.))
                .border(px(0.5))
                .border_color(t.border_strong)
                .bg(t.bg_elevated)
                .shadow(vec![shadow(0., 1., 0., 0., t.border)])
                .text_size(TEXT_XS)
                .line_height(px(17.))
                .flex()
                .justify_center()
                .text_color(t.text_secondary)
                .child(k)
                .into_any_element()
        };
        // The body as words that wrap, with the key caps among them.
        // A fragment after a key cap starts with the space after it.
        let text = |s: &'static str| -> Vec<AnyElement> {
            let lead = s.starts_with(' ');
            s.split(' ')
                .filter(|w| !w.is_empty())
                .enumerate()
                .map(|(i, w)| {
                    let w = if i == 0 && lead {
                        format!("\u{a0}{w}")
                    } else {
                        w.to_owned()
                    };
                    div().flex_none().child(format!("{w}\u{a0}")).into_any_element()
                })
                .collect()
        };
        let mut body: Vec<AnyElement> = Vec::new();
        body.extend(text("Type in the bar below, double-click anywhere or press"));
        body.push(kbd("N"));
        body.extend(text(" to start a Claude session,"));
        body.push(kbd("V"));
        body.extend(text(" to speak one, or"));
        body.push(kbd("T"));
        body.extend(text(
            " for a terminal. Drag cards out of the archive to reactivate them.",
        ));
        div()
            .absolute()
            .top(px(22. + 6. * (1. - rise)))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .max_w(px(440.))
                    .py(px(12.))
                    .px(px(18.))
                    .rounded(RADIUS_LG)
                    .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.8), 20., 1.6))
                    .shadow(t.sm())
                    .opacity(rise)
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .mb(px(2.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_primary)
                            .child("No active sessions"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_center()
                            .items_center()
                            .text_size(TEXT_SM)
                            .line_height(px(16.8))
                            .text_color(t.text_secondary)
                            .children(body),
                    ),
            )
            .into_any_element()
    }

    /// The confirm dialog (BOARD-140, BOARD-141).
    pub(crate) fn render_confirm(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let t = theme::tokens(cx);
        let me = cx.entity().downgrade();
        let (cancel, ok, close) = (me.clone(), me.clone(), me);
        let body = div()
            .flex()
            .flex_col()
            .w(px(380.))
            .pt(px(22.))
            .px(px(22.))
            .pb(px(18.))
            .child(
                div()
                    .mb(px(6.))
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(confirm.title.clone()),
            )
            .child(
                div()
                    .mb(px(20.))
                    .text_color(t.text_secondary)
                    .line_height(px(13. * 1.45))
                    .child(confirm.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(("confirm-cancel", confirm.serial as usize))
                            .label("Cancel")
                            .on_click(move |_, _, cx| {
                                if let Some(c) = cancel.upgrade() {
                                    c.update(cx, |c, cx| c.resolve_confirm(false, cx));
                                }
                            }),
                    )
                    .child(
                        button(("confirm-ok", confirm.serial as usize))
                            .kind(if confirm.danger { Kind::Danger } else { Kind::Primary })
                            .label(confirm.confirm_label.clone())
                            .on_click(move |_, _, cx| {
                                if let Some(c) = ok.upgrade() {
                                    c.update(cx, |c, cx| c.resolve_confirm(true, cx));
                                }
                            }),
                    ),
            );
        Some(
            Modal::new(("confirm-dialog", confirm.serial as usize), px(380.), move |_, cx| {
                if let Some(c) = close.upgrade() {
                    c.update(cx, |c, cx| c.resolve_confirm(false, cx));
                }
            })
            .render(body, window, cx),
        )
    }
}

/// `color-mix(in srgb, c p%, transparent)`.
pub(crate) fn rgba_mix(c: gpui::Rgba, share: f32) -> gpui::Rgba {
    gpui::Rgba { a: c.a * share, ..c }
}

/// The minimap's picture: its nodes, and the view rect cut out of a mask.
fn paint_minimap(
    bounds: Bounds<Pixels>,
    nodes: &[FlowNode],
    board: Option<&nodus_domain::BoardSnapshot>,
    vp: Viewport,
    canvas: (f64, f64),
    t: &Tokens,
    window: &mut Window,
) {
    let view = Rect {
        x: -vp.x / vp.zoom,
        y: -vp.y / vp.zoom,
        w: canvas.0 / vp.zoom,
        h: canvas.1 / vp.zoom,
    };
    let mut rects: Vec<Rect> = nodes.iter().map(FlowNode::rect).collect();
    let bounding = if rects.is_empty() {
        view
    } else {
        rects.push(view);
        super::layout::bounds_of(&rects)
    };
    let (ew, eh) = (f64::from(MINIMAP_W), f64::from(MINIMAP_H));
    let view_scale = (bounding.w / ew).max(bounding.h / eh);
    let (vw, vh) = (view_scale * ew, view_scale * eh);
    let offset = 5.0 * view_scale;
    let vb = Rect {
        x: bounding.x - (vw - bounding.w) / 2.0 - offset,
        y: bounding.y - (vh - bounding.h) / 2.0 - offset,
        w: vw + offset * 2.0,
        h: vh + offset * 2.0,
    };
    // preserveAspectRatio xMidYMid meet.
    let s = (ew / vb.w).min(eh / vb.h);
    let ox = (ew - vb.w * s) / 2.0;
    let oy = (eh - vb.h * s) / 2.0;
    let map = |x: f64, y: f64| {
        point(
            bounds.origin.x + px((ox + (x - vb.x) * s) as f32),
            bounds.origin.y + px((oy + (y - vb.y) * s) as f32),
        )
    };
    let quad = |r: Rect| Bounds::from_corners(map(r.x, r.y), map(r.x + r.w, r.y + r.h));
    let radius = px((6.0 * s) as f32);
    for n in nodes {
        let (fill_color, stroke): (gpui::Rgba, Option<gpui::Rgba>) = match &n.kind {
            NodeKind::Archive { .. } | NodeKind::Pad => (t.archive_bg, Some(t.archive_border)),
            NodeKind::Section { index, .. } => {
                let hue = board
                    .and_then(|b| b.sections.get(*index))
                    .map_or(t.text_tertiary, |s| folder_hues(s.color).back_bottom);
                (
                    gpui::Rgba {
                        a: f32::from(0x24u8) / 255.0,
                        ..hue
                    },
                    Some(hue),
                )
            }
            NodeKind::Note { index } => {
                let color = index
                    .and_then(|i| board.and_then(|b| b.notes.get(i)))
                    .map_or(nodus_domain::FolderColor::Yellow, |n| n.color);
                (folder_hues(color).front_top, None)
            }
            NodeKind::View { .. } => (folder_hues(nodus_domain::FolderColor::Purple).front_top, None),
            NodeKind::Shape { index } => {
                let color = board
                    .and_then(|b| b.shapes.get(*index))
                    .map_or(nodus_domain::InkColor::Graphite, |s| s.color);
                (super::overlays::ink::ink_rgba(t, color), None)
            }
            NodeKind::Card { index, .. } => {
                let card = board.and_then(|b| b.cards.get(*index));
                match card {
                    Some(c) if super::layout::is_terminal(c) => (t.text_tertiary, None),
                    Some(c) => (t.status(c.status), None),
                    None => (t.status(SessionStatus::Done), None),
                }
            }
        };
        let b = quad(n.rect());
        let mut q = fill(b, fill_color).corner_radii(radius);
        if let Some(stroke) = stroke {
            q = q.border_widths(px((3.0 * s) as f32)).border_color(stroke);
        }
        window.paint_quad(q);
    }
    // The mask: everything outside the view, then its edge.
    let mask = rgba_mix(t.bg_canvas, 0.7);
    let v = quad(view);
    let outer = Bounds::new(bounds.origin, size(px(MINIMAP_W), px(MINIMAP_H)));
    let bands = [
        Bounds::from_corners(outer.origin, point(outer.right(), v.top().max(outer.top()))),
        Bounds::from_corners(
            point(outer.left(), v.bottom().min(outer.bottom())),
            outer.bottom_right(),
        ),
        Bounds::from_corners(
            point(outer.left(), v.top().max(outer.top())),
            point(v.left().max(outer.left()), v.bottom().min(outer.bottom())),
        ),
        Bounds::from_corners(
            point(v.right().min(outer.right()), v.top().max(outer.top())),
            point(outer.right(), v.bottom().min(outer.bottom())),
        ),
    ];
    for band in bands {
        if band.size.width > px(0.) && band.size.height > px(0.) {
            window.paint_quad(fill(band, mask));
        }
    }
    let stroke_w = px((2.0 * s) as f32);
    let mut edge = PathBuilder::stroke(stroke_w);
    edge.move_to(v.origin);
    edge.line_to(v.top_right());
    edge.line_to(v.bottom_right());
    edge.line_to(v.bottom_left());
    edge.close();
    if let Ok(path) = edge.build() {
        window.paint_path(path, rgba_mix(t.accent, 0.5));
    }
}
