//! Links in the ink layer (spec 14, LINK): drawn above the cards with the
//! ink, under its strokes; their labels, and the label being written.

use nodus_board::geometry::Point;
use nodus_domain::{BoardLink, Rect};

use super::draw::paint_dashed_rect;
use super::*;
use crate::board::shapes::{self, LINK_HEAD, LINK_WIDTH};

/// A label's words: the UI font, semibold, 13 board units, a line 22 tall
/// with 8 units each side.
pub(crate) const LINK_LABEL_FONT: f64 = 13.0;
pub(crate) const LINK_LABEL_H: f64 = 22.0;
pub(crate) const LINK_LABEL_PAD: f64 = 8.0;

/// How wide words are in a label, in board units.
pub(crate) fn link_label_measure(window: &Window) -> impl Fn(&str) -> f64 + use<> {
    let ts: Rc<WindowTextSystem> = window.text_system().clone();
    move |text: &str| {
        if text.is_empty() {
            return 0.0;
        }
        let font = gpui::Font {
            weight: FontWeight::SEMIBOLD,
            ..gpui::font(fonts::UI)
        };
        let run = TextRun {
            len: text.len(),
            font,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = ts.shape_line(
            SharedString::from(text.to_owned()),
            px(LINK_LABEL_FONT as f32),
            &[run],
            None,
        );
        f64::from(f32::from(shaped.width))
    }
}

/// A label's box, centred on the middle of its link.
pub(crate) fn label_rect(curve: &[Point; 4], text: &str, measure: &dyn Fn(&str) -> f64) -> Rect {
    let mid = shapes::at(curve, 0.5);
    let w = measure(text) + 2.0 * LINK_LABEL_PAD;
    Rect {
        x: mid.x - w / 2.0,
        y: mid.y - LINK_LABEL_H / 2.0,
        w,
        h: LINK_LABEL_H,
    }
}

/// A link as painted this frame.
pub(crate) struct LinkPaint {
    pub curve: [Point; 4],
    pub color: Rgba,
    pub dashed: bool,
    /// The selected link shows its ends, to take elsewhere.
    pub handles: bool,
}

impl BoardCanvas {
    /// The label under a board point, top first.
    pub(crate) fn link_label_at(&self, p: Pt, measure: &dyn Fn(&str) -> f64) -> Option<String> {
        self.links()
            .iter()
            .rev()
            .filter(|l| !l.label.is_empty())
            .find(|l| {
                self.link_curve(l).is_some_and(|c| {
                    let r = label_rect(&c, &l.label, measure);
                    p[0] >= r.x && p[0] <= r.x + r.w && p[1] >= r.y && p[1] <= r.y + r.h
                })
            })
            .map(|l| l.id.clone())
    }

    /// LINK-001…LINK-005, LINK-010, LINK-014: the links to paint, the one
    /// being drawn or taken elsewhere included, and the box a new end would
    /// hold on to.
    pub(crate) fn link_paints(&self, t: &Tokens) -> (Vec<LinkPaint>, Option<Rect>) {
        let ink = &self.overlays.ink;
        let erasing: &[String] = match &ink.gesture {
            Some(InkGesture::Erase { links, .. }) => links,
            _ => &[],
        };
        let mut out = Vec::new();
        let mut target = None;
        for link in self.links() {
            let mut shown = link.clone();
            if let Some(InkGesture::Reattach {
                id,
                head,
                end,
                target: over,
            }) = &ink.gesture
                && id == &link.id
            {
                let moved = LinkEnd {
                    node: over.clone(),
                    x: end[0],
                    y: end[1],
                };
                if *head {
                    shown.to = moved;
                } else {
                    shown.from = moved;
                }
                target = over.as_deref().and_then(|id| self.attach_rect(id));
            }
            let Some(curve) = self.link_curve(&shown) else { continue };
            let selected = ink.link.as_deref() == Some(link.id.as_str());
            let mut color = if selected { t.accent } else { ink_rgba(t, link.color) };
            if erasing.contains(&link.id) {
                color.a *= 0.25;
            }
            out.push(LinkPaint {
                curve,
                color,
                dashed: false,
                handles: selected,
            });
        }
        if let Some(InkGesture::Link {
            from,
            end,
            target: over,
            ..
        }) = &ink.gesture
        {
            let to = LinkEnd {
                node: over.clone(),
                x: end[0],
                y: end[1],
            };
            let live = BoardLink {
                id: String::new(),
                from: from.clone(),
                to,
                color: ink.color.unwrap_or(InkColor::Red),
                label: String::new(),
                by: None,
                created_at: 0.0,
                extra: serde_json::Map::new(),
            };
            if let Some(curve) = self.link_curve(&live) {
                out.push(LinkPaint {
                    curve,
                    color: ink_rgba(t, live.color),
                    dashed: over.is_none(),
                    handles: false,
                });
            }
            target = over.as_deref().and_then(|id| self.attach_rect(id));
        }
        (out, target)
    }

    /// LINK-020: writing a link's label, at its middle.
    pub(crate) fn ink_open_link_label(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ink_finish_link_label(Some(window), cx);
        let text = self.link(&id).map(|l| l.label.clone()).unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Label").context_menu(false);
            state.set_value(text, window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Blur => this.ink_finish_link_label(Some(window), cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        ];
        self.overlays.ink.link_label = Some(LinkLabelDraft {
            id,
            input: input.clone(),
            _subscriptions: subscriptions,
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        window.on_next_frame(move |window, _| {
            window.refresh();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |s, cx| {
                    s.focus(window, cx);
                    let end = s.value().len();
                    s.set_selected_range(0..end, cx);
                });
            });
        });
        cx.notify();
    }

    /// Return or leaving the field keeps the label (emptied, it goes).
    pub(crate) fn ink_finish_link_label(&mut self, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        let Some(draft) = self.overlays.ink.link_label.take() else {
            return;
        };
        if let Some(window) = window
            && draft.input.read(cx).focus_handle(cx).is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
        let text = draft.input.read(cx).value().to_string();
        if self.link(&draft.id).is_some_and(|l| l.label != js::trim(&text)) {
            self.set_link_label(&draft.id, &text, cx);
        }
        cx.notify();
    }

    /// Esc leaves the label as it was.
    pub(crate) fn ink_discard_link_label(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.link_label.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// The links' labels, and the one being written.
    pub(crate) fn render_link_labels(
        &self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let vp = self.viewport;
        let z = vp.zoom;
        let measure = link_label_measure(window);
        let editing = self.overlays.ink.link_label.as_ref().map(|d| d.id.clone());
        let mut out = Vec::new();
        for link in self.links() {
            let Some(curve) = self.link_curve(link) else { continue };
            let is_editing = editing.as_deref() == Some(link.id.as_str());
            if link.label.is_empty() && !is_editing {
                continue;
            }
            let text = match (&self.overlays.ink.link_label, is_editing) {
                (Some(d), true) => d.input.read(cx).value().to_string(),
                _ => link.label.clone(),
            };
            let shown = if text.is_empty() { "Label" } else { text.as_str() };
            let r = label_rect(&curve, shown, &measure);
            let selected = self.overlays.ink.link.as_deref() == Some(link.id.as_str());
            let edge = if selected { t.accent } else { ink_rgba(t, link.color) };
            let pill = div()
                .absolute()
                .left(px((vp.x + r.x * z) as f32))
                .top(px((vp.y + r.y * z) as f32))
                .w(px((r.w * z) as f32 + if is_editing { 24.0 } else { 0.0 }))
                .h(px((r.h * z) as f32))
                .px(px((LINK_LABEL_PAD * z) as f32))
                .flex()
                .items_center()
                .rounded(px((6.0 * z) as f32))
                .bg(t.bg_elevated)
                .border(px((1.0 * z).max(0.5) as f32))
                .border_color(Rgba {
                    a: edge.a * 0.6,
                    ..edge
                })
                .font_family(fonts::UI)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px((LINK_LABEL_FONT * z) as f32))
                .line_height(px((LINK_LABEL_H * z) as f32))
                .text_color(t.text_primary)
                .whitespace_nowrap();
            let pill = match (&self.overlays.ink.link_label, is_editing) {
                (Some(draft), true) => pill
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                        match e.keystroke.key.as_str() {
                            "enter" => {
                                cx.stop_propagation();
                                this.ink_finish_link_label(Some(window), cx);
                            }
                            "escape" => {
                                cx.stop_propagation();
                                this.ink_discard_link_label(window, cx);
                            }
                            _ => {}
                        }
                    }))
                    .child(Styled::h(
                        Input::new(&draft.input)
                            .appearance(false)
                            .px(px(0.))
                            .py(px(0.))
                            .text_size(px((LINK_LABEL_FONT * z) as f32))
                            .line_height(px((LINK_LABEL_H * z) as f32)),
                        px((LINK_LABEL_H * z) as f32),
                    ))
                    .into_any_element(),
                _ => pill.child(text).into_any_element(),
            };
            out.push(pill);
        }
        out
    }
}

/// Paints a link: its curve, its head, dashes while it's loose, and the
/// selected one's ends as handles.
pub(crate) fn paint_link(
    link: &LinkPaint,
    zoom: f64,
    handle_fill: Rgba,
    at: &dyn Fn(Pt) -> GPoint<Pixels>,
    window: &mut Window,
) {
    let p = |q: Point| at([q.x, q.y]);
    let width = (LINK_WIDTH * zoom) as f32;
    let c = &link.curve;
    let head = shapes::head(c, LINK_HEAD);
    // The line stops short of the tip, under the head.
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    let mut path = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    if link.dashed {
        path = path.dash_array(&[px(6.0 * zoom as f32), px(4.0 * zoom as f32)]);
    }
    let base = Point {
        x: (head[1].x + head[2].x) / 2.0,
        y: (head[1].y + head[2].y) / 2.0,
    };
    path.move_to(p(c[0]));
    path.cubic_bezier_to(p(base), p(c[1]), p(c[2]));
    if let Ok(path) = path.build() {
        window.paint_path(path, link.color);
    }
    let mut tri = PathBuilder::fill();
    tri.move_to(p(head[0]));
    tri.line_to(p(head[1]));
    tri.line_to(p(head[2]));
    tri.close();
    if let Ok(tri) = tri.build() {
        window.paint_path(tri, link.color);
    }
    if link.handles {
        for end in [c[0], c[3]] {
            let centre = p(end);
            let r = px(5.0);
            let b = Bounds::new(point(centre.x - r, centre.y - r), gpui::size(r * 2.0, r * 2.0));
            window.paint_quad(gpui::fill(b, link.color).corner_radii(r));
            let inner = px(3.5);
            let b = Bounds::new(
                point(centre.x - inner, centre.y - inner),
                gpui::size(inner * 2.0, inner * 2.0),
            );
            window.paint_quad(gpui::fill(b, handle_fill).corner_radii(inner));
        }
    }
}

/// The dashed accent box around what a new end would hold on to.
pub(crate) fn paint_target(r: Rect, zoom: f64, color: Rgba, at: &dyn Fn(Pt) -> GPoint<Pixels>, window: &mut Window) {
    let pad = 6.0;
    let origin = at([r.x - pad / zoom, r.y - pad / zoom]);
    let (w, h) = ((r.w * zoom + 2.0 * pad) as f32, (r.h * zoom + 2.0 * pad) as f32);
    paint_dashed_rect(origin, w, h, 10.0, 2.0, (6.0, 4.0), color, window);
}

/// A shape being drawn, as an outline (SHAPE-002).
pub(crate) fn paint_shape_outline(
    kind: ShapeKind,
    r: Rect,
    zoom: f64,
    color: Rgba,
    at: &dyn Fn(Pt) -> GPoint<Pixels>,
    window: &mut Window,
) {
    let width = (2.0 * zoom) as f32;
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_join(LineJoin::Round);
    let mut path = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    shape_path(&mut path, kind, r, at);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A rectangle's, ellipse's or diamond's outline, for a path builder.
pub(crate) fn shape_path(path: &mut PathBuilder, kind: ShapeKind, r: Rect, at: &dyn Fn(Pt) -> GPoint<Pixels>) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    match kind {
        ShapeKind::Ellipse => {
            // Four quarter arcs as cubics.
            let k = 0.552_284_75;
            let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
            path.move_to(at([cx + rx, cy]));
            path.cubic_bezier_to(
                at([cx, cy + ry]),
                at([cx + rx, cy + k * ry]),
                at([cx + k * rx, cy + ry]),
            );
            path.cubic_bezier_to(
                at([cx - rx, cy]),
                at([cx - k * rx, cy + ry]),
                at([cx - rx, cy + k * ry]),
            );
            path.cubic_bezier_to(
                at([cx, cy - ry]),
                at([cx - rx, cy - k * ry]),
                at([cx - k * rx, cy - ry]),
            );
            path.cubic_bezier_to(
                at([cx + rx, cy]),
                at([cx + k * rx, cy - ry]),
                at([cx + rx, cy - k * ry]),
            );
            path.close();
        }
        ShapeKind::Diamond => {
            path.move_to(at([x + w / 2.0, y]));
            path.line_to(at([x + w, y + h / 2.0]));
            path.line_to(at([x + w / 2.0, y + h]));
            path.line_to(at([x, y + h / 2.0]));
            path.close();
        }
        _ => {
            let rad = (10.0f64).min(w / 2.0).min(h / 2.0);
            let k = 0.552_284_75 * rad;
            path.move_to(at([x + rad, y]));
            path.line_to(at([x + w - rad, y]));
            path.cubic_bezier_to(at([x + w, y + rad]), at([x + w - rad + k, y]), at([x + w, y + rad - k]));
            path.line_to(at([x + w, y + h - rad]));
            path.cubic_bezier_to(
                at([x + w - rad, y + h]),
                at([x + w, y + h - rad + k]),
                at([x + w - rad + k, y + h]),
            );
            path.line_to(at([x + rad, y + h]));
            path.cubic_bezier_to(at([x, y + h - rad]), at([x + rad - k, y + h]), at([x, y + h - rad + k]));
            path.line_to(at([x, y + rad]));
            path.cubic_bezier_to(at([x + rad, y]), at([x, y + rad - k]), at([x + rad - k, y]));
            path.close();
        }
    }
}
