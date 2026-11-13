//! The pointer, taken ahead of the board (INKUI-006…INKUI-020), and the
//! tools that draw shapes and links (spec 14).

use nodus_board::geometry::Point;
use nodus_domain::Rect;

use super::*;
use crate::board::shapes;

/// A board point as the shapes have it.
fn pt(p: Pt) -> Point {
    Point { x: p[0], y: p[1] }
}

impl BoardCanvas {
    // ---- the pointer ------------------------------------------------------------------

    /// The pointer's place in board units.
    pub(crate) fn ink_point(&self, p: GPoint<Pixels>) -> Pt {
        let f = self.to_flow(p);
        [f.x, f.y]
    }

    /// Whether `p` is on the label being typed.
    pub(crate) fn on_label_input(&self, p: GPoint<Pixels>, window: &Window, cx: &App) -> bool {
        let Some(label) = &self.overlays.ink.label else {
            return false;
        };
        let text = label.input.read(cx).value();
        let w = label_width(
            window.text_system(),
            if text.is_empty() { "Label" } else { &text },
            label.size,
        )
        .max(48.0);
        let q = self.ink_point(p);
        let pad = 3.0;
        q[0] >= label.x - pad
            && q[0] <= label.x + w + pad
            && q[1] >= label.y - pad
            && q[1] <= label.y + LABEL_LINE * label.size + pad
    }

    /// INKUI-007: the press is the ink's: nothing under it sees it, a
    /// label being typed is finished, and the keyboard comes back to the board.
    pub(crate) fn ink_take(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.stop_propagation();
        window.prevent_default();
        self.ink_finish_label(Some(window), cx);
        if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
    }

    /// INKUI-016: with the pointer tool and no button held, ink under the
    /// pointer shows it can be picked up (the move cursor over the whole
    /// board surface). GPUI hands over one move a frame, as 0.9.4 checked
    /// once per animation frame.
    fn ink_hover(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.tool != InkTool::Select || e.pressed_button.is_some() {
            return;
        }
        let strokes = self.ink_strokes();
        let over = !strokes.is_empty() && self.overlays.ink.hitbox.as_ref().is_some_and(|h| h.is_hovered(window)) && {
            let p = self.ink_point(e.position);
            let measure = measurer(window);
            stroke_at(&strokes, p, HIT_PX / self.viewport.zoom, &measure).is_some()
        };
        let over_link = !over && self.overlays.ink.hitbox.as_ref().is_some_and(|h| h.is_hovered(window)) && {
            let p = self.ink_point(e.position);
            let measure = links::link_label_measure(window);
            self.link_label_at(p, &measure).is_some()
                || self.link_at(pt(p), shapes::LINK_HIT_PX / self.viewport.zoom).is_some()
        };
        if over != self.overlays.ink.hover || over_link != self.overlays.ink.hover_link {
            self.overlays.ink.hover = over;
            self.overlays.ink.hover_link = over_link;
            cx.notify();
        }
    }

    /// INKUI-006…INKUI-015, INKUI-019, INKUI-020: a press, seen before
    /// anything on the board.
    pub(crate) fn ink_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.bar_outside_press(e.position, cx);
        // INKUI-004: the palette closes on a press anywhere else.
        if self.overlays.ink.palette && !self.overlays.ink.color_area.iter().any(|b| b.contains(&e.position)) {
            self.overlays.ink.palette = false;
            cx.notify();
        }
        // SHAPE-003: so does a picker, unless the press is its own button's.
        if self.overlays.ink.picker.is_some()
            && !self.overlays.ink.picker_area.iter().any(|b| b.contains(&e.position))
            && !self.on_dock(e.position)
        {
            self.overlays.ink.picker = None;
            cx.notify();
        }
        if e.button != MouseButton::Left || self.space {
            return;
        }
        let on_board = self.overlays.ink.hitbox.as_ref().is_some_and(|h| h.is_hovered(window));
        if !on_board || self.on_label_input(e.position, window, cx) {
            return;
        }
        let p = self.ink_point(e.position);
        let zoom = self.viewport.zoom;
        let tool = self.overlays.ink.tool;
        if tool == InkTool::Select {
            let strokes = self.ink_strokes();
            let hit = {
                let measure = measurer(window);
                stroke_at(&strokes, p, HIT_PX / zoom, &measure).cloned()
            };
            let Some(hit) = hit else {
                // LINK-012, LINK-014: the selected link's ends, then any link.
                if self.link_press(p, e.click_count, window, cx) {
                    return;
                }
                if self.overlays.ink.selected.take().is_some() || self.overlays.ink.link.take().is_some() {
                    cx.notify();
                }
                return;
            };
            self.ink_take(window, cx);
            self.overlays.ink.link = None;
            // INKUI-014: a double-click on a label edits it in place.
            if e.click_count >= 2 && hit.kind == InkKind::Text {
                self.overlays.ink.selected = None;
                let at = points_of(&hit)[0];
                let text = hit.text.clone().unwrap_or_default();
                self.ink_open_label(Some(hit.id.clone()), at, hit.size, hit.color, &text, window, cx);
                return;
            }
            // INKUI-015: selected, the cards let go, and it follows the pointer.
            self.overlays.ink.selected = Some(hit.id.clone());
            self.clear_selection(cx);
            self.overlays.ink.gesture = Some(InkGesture::Move {
                id: hit.id,
                start: p,
                dx: 0.0,
                dy: 0.0,
                moved: false,
                zoom,
            });
            cx.notify();
            return;
        }
        self.ink_take(window, cx);
        // BUBBLE-001, BUBBLE-040: asking Claude on the spot.
        match tool {
            InkTool::Bubble => return self.bubble_press(e.position, window, cx),
            InkTool::Mic => return self.voice_press(e.position, window, cx),
            _ => {}
        }
        let color = self.overlays.ink.color(cx);
        let scale = size_at(zoom);
        match tool {
            InkTool::Pen => {
                self.overlays.ink.gesture = Some(InkGesture::Draw {
                    base: InkStroke {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: InkKind::Pen,
                        color,
                        points: Vec::new(),
                        size: scale,
                        text: None,
                        extra: serde_json::Map::new(),
                    },
                    pts: vec![p],
                    zoom,
                });
            }
            // SHAPE-002: from corner to corner.
            InkTool::Shape => {
                self.overlays.ink.gesture = Some(InkGesture::Shape {
                    kind: self.overlays.ink.shape_kind(),
                    start: p,
                    end: p,
                    square: e.modifiers.shift,
                    zoom,
                });
            }
            // LINK-010: from what's pressed, or from the spot.
            InkTool::Arrow => {
                let from = self.attachable_at(pt(p), None);
                self.overlays.ink.gesture = Some(InkGesture::Link {
                    from: LinkEnd {
                        node: from,
                        x: p[0],
                        y: p[1],
                    },
                    start: p,
                    end: p,
                    target: None,
                    zoom,
                });
            }
            InkTool::Eraser => {
                let tolerance = HIT_PX / zoom;
                self.overlays.ink.gesture = Some(InkGesture::Erase {
                    gone: Vec::new(),
                    shapes: Vec::new(),
                    links: Vec::new(),
                    last: p,
                    tolerance,
                });
                self.ink_erase_along(p, p, window, cx);
            }
            // SHAPE-004: words on their own, written at once, the line
            // centred on the press.
            InkTool::Text => {
                let (w, h) = shapes::default_size(ShapeKind::Text);
                let (w, h) = (w * scale, h * scale);
                let rect = Rect {
                    x: p[0],
                    y: p[1] - h / 2.0,
                    w,
                    h,
                };
                self.ink_set_tool(InkTool::Select, Some(window), cx);
                self.add_shape(ShapeKind::Text, rect, String::new(), None, true, cx);
            }
            // SHAPE-005: the sticker picked, centred on the press.
            InkTool::Sticker => {
                let side = shapes::default_size(ShapeKind::Sticker).0 * scale;
                let rect = Rect {
                    x: p[0] - side / 2.0,
                    y: p[1] - side / 2.0,
                    w: side,
                    h: side,
                };
                let emoji = self.overlays.ink.sticker().to_owned();
                self.overlays.ink.picker = None;
                self.add_shape(ShapeKind::Sticker, rect, emoji, None, false, cx);
            }
            InkTool::Select | InkTool::Bubble | InkTool::Mic => {}
        }
        cx.notify();
    }

    /// LINK-012…LINK-014: a press with the pointer tool on the selected
    /// link's end (taking it elsewhere), or on a link or its label
    /// (selecting it; twice, writing its label). False when it missed.
    fn link_press(&mut self, p: Pt, clicks: usize, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let zoom = self.viewport.zoom;
        let tol = shapes::LINK_HIT_PX / zoom;
        if let Some((id, head)) = self.link_end_at(p, tol * 1.5) {
            self.ink_take(window, cx);
            self.overlays.ink.gesture = Some(InkGesture::Reattach {
                id,
                head,
                end: p,
                target: None,
            });
            cx.notify();
            return true;
        }
        let measure = links::link_label_measure(window);
        let Some(id) = self.link_label_at(p, &measure).or_else(|| self.link_at(pt(p), tol)) else {
            return false;
        };
        self.ink_take(window, cx);
        self.overlays.ink.selected = None;
        self.clear_selection(cx);
        self.overlays.ink.link = Some(id.clone());
        if clicks >= 2 {
            self.ink_open_link_label(id, window, cx);
        }
        cx.notify();
        true
    }

    /// The selected link's end near a point: which link, and whether it's
    /// the end with the head.
    fn link_end_at(&self, p: Pt, tol: f64) -> Option<(String, bool)> {
        let id = self.overlays.ink.link.clone()?;
        let curve = self.link(&id).and_then(|l| self.link_curve(l))?;
        let near = |q: Point| (q.x - p[0]).hypot(q.y - p[1]) <= tol;
        if near(curve[3]) {
            Some((id, true))
        } else if near(curve[0]) {
            Some((id, false))
        } else {
            None
        }
    }

    /// INKUI-011: the strokes the eraser passed over from `a` to `b`.
    /// SHAPE-021, LINK-015: and the shapes and links it passed over.
    pub(crate) fn ink_erase_along(&mut self, a: Pt, b: Pt, window: &Window, cx: &mut Context<'_, Self>) {
        let strokes = self.ink_strokes();
        let measure = measurer(window);
        let Some(InkGesture::Erase { tolerance, .. }) = &self.overlays.ink.gesture else {
            return;
        };
        let tol = *tolerance;
        // Along the way, every half tolerance.
        let steps = (distance(a, b) / (tol / 2.0).max(0.5)).ceil().max(1.0) as usize;
        let along: Vec<Point> = (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                Point {
                    x: a[0] + (b[0] - a[0]) * t,
                    y: a[1] + (b[1] - a[1]) * t,
                }
            })
            .collect();
        let shape_hits: Vec<String> = self
            .shapes()
            .iter()
            .filter(|s| {
                along
                    .iter()
                    .any(|q| q.x >= s.x - tol && q.x <= s.x + s.w + tol && q.y >= s.y - tol && q.y <= s.y + s.h + tol)
            })
            .map(|s| s.id.clone())
            .collect();
        let link_hits: Vec<String> = self
            .links()
            .iter()
            .filter(|l| {
                self.link_curve(l)
                    .is_some_and(|c| along.iter().any(|q| shapes::distance(&c, *q) <= tol))
            })
            .map(|l| l.id.clone())
            .collect();
        let Some(InkGesture::Erase {
            gone,
            shapes: gone_shapes,
            links: gone_links,
            ..
        }) = &mut self.overlays.ink.gesture
        else {
            return;
        };
        let left: Vec<InkStroke> = strokes.into_iter().filter(|s| !gone.contains(&s.id)).collect();
        let hit = strokes_along(&left, a, b, tol, &measure);
        let mut changed = !hit.is_empty();
        gone.extend(hit);
        for id in shape_hits {
            if !gone_shapes.contains(&id) {
                gone_shapes.push(id);
                changed = true;
            }
        }
        for id in link_hits {
            if !gone_links.contains(&id) {
                gone_links.push(id);
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// The pointer moving while the ink holds it (INKUI-007).
    pub(crate) fn ink_mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.gesture.is_none() {
            self.ink_hover(e, window, cx);
            return;
        }
        cx.stop_propagation();
        let q = self.ink_point(e.position);
        let link_from = match &self.overlays.ink.gesture {
            Some(InkGesture::Link { from, .. }) => from.node.clone(),
            Some(InkGesture::Reattach { id, head, .. }) => self
                .link(id)
                .and_then(|l| if *head { l.from.node.clone() } else { l.to.node.clone() }),
            _ => None,
        };
        let over = self.attachable_at(pt(q), link_from.as_deref());
        match &mut self.overlays.ink.gesture {
            Some(InkGesture::Shape { end, square, .. }) => {
                *end = q;
                *square = e.modifiers.shift;
            }
            Some(InkGesture::Link { end, target, .. } | InkGesture::Reattach { end, target, .. }) => {
                *end = q;
                *target = over;
            }
            Some(InkGesture::Draw { base, pts, zoom }) => {
                if matches!(base.kind, InkKind::Arrow | InkKind::Rect) {
                    // INKUI-027: Shift keeps a rectangle square.
                    let q = if base.kind == InkKind::Rect && e.modifiers.shift {
                        ink_draw::square_corner(pts[0], q)
                    } else {
                        q
                    };
                    if pts.len() < 2 {
                        pts.push(q);
                    } else {
                        pts[1] = q;
                    }
                } else if pts.last().is_some_and(|&last| distance(q, last) * *zoom >= MIN_STEP_PX) {
                    pts.push(q);
                }
            }
            Some(InkGesture::Erase { last, .. }) => {
                let from = std::mem::replace(last, q);
                self.ink_erase_along(from, q, window, cx);
            }
            Some(InkGesture::Move {
                start,
                dx,
                dy,
                moved,
                zoom,
                ..
            }) => {
                *dx = q[0] - start[0];
                *dy = q[1] - start[1];
                if !*moved && dx.hypot(*dy) * *zoom >= MOVE_PX {
                    *moved = true;
                }
            }
            None => {}
        }
        cx.notify();
    }

    /// The pointer lifting: the gesture's change goes in (INKUI-008…INKUI-015).
    pub(crate) fn ink_mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<'_, Self>) {
        let Some(gesture) = self.overlays.ink.gesture.take() else {
            return;
        };
        cx.stop_propagation();
        let current = self.ink_strokes();
        match gesture {
            InkGesture::Shape {
                kind,
                start,
                end,
                square,
                zoom,
            } => {
                // SHAPE-002: a click puts one down at its usual size, centred.
                let dragged = distance(start, end) * zoom >= MIN_ARROW_PX;
                let rect = if !dragged {
                    let s = size_at(zoom);
                    let (w, h) = shapes::default_size(kind);
                    let (w, h) = (w * s, h * s);
                    Rect {
                        x: start[0] - w / 2.0,
                        y: start[1] - h / 2.0,
                        w,
                        h,
                    }
                } else {
                    shapes::box_between(pt(start), pt(end), square)
                };
                self.overlays.ink.tool = InkTool::Select;
                self.overlays.ink.picker = None;
                let id = self.add_shape(kind, rect, String::new(), None, true, cx);
                // MOTION-010: one drawn corner to corner was seen as it grew.
                if dragged {
                    self.moments.hush(&id);
                }
            }
            InkGesture::Link {
                from,
                start,
                end,
                target,
                zoom,
            } => {
                // LINK-011: a slip, unless it reached something to hold.
                if target.is_none() && distance(start, end) * zoom < shapes::MIN_LINK_PX {
                    cx.notify();
                    return;
                }
                let link = nodus_domain::BoardLink {
                    id: uuid::Uuid::new_v4().to_string(),
                    from,
                    to: LinkEnd {
                        node: target,
                        x: end[0],
                        y: end[1],
                    },
                    color: self.overlays.ink.color(cx),
                    label: String::new(),
                    by: None,
                    created_at: crate::parity::now_ms(cx),
                    extra: serde_json::Map::new(),
                };
                let mut change = nodus_board::drawing::Change::default();
                change.link(None, Some(link));
                self.commit_change(change, cx);
            }
            InkGesture::Reattach { id, head, end, target } => {
                let Some(before) = self.link(&id).cloned() else {
                    cx.notify();
                    return;
                };
                let moved = LinkEnd {
                    node: target,
                    x: end[0],
                    y: end[1],
                };
                let mut after = before.clone();
                if head {
                    after.to = moved;
                } else {
                    after.from = moved;
                }
                if after.from.node.is_some() && after.from.node == after.to.node {
                    cx.notify();
                    return;
                }
                let mut change = nodus_board::drawing::Change::default();
                change.link(Some(before), Some(after));
                self.commit_change(change, cx);
            }
            InkGesture::Draw { base, pts, zoom } => {
                // INKUI-009, INKUI-027: a short arrow or rectangle is a slip.
                if matches!(base.kind, InkKind::Arrow | InkKind::Rect)
                    && (pts.len() < 2 || distance(pts[0], pts[1]) * zoom < MIN_ARROW_PX)
                {
                    cx.notify();
                    return;
                }
                let kept = if base.kind == InkKind::Pen {
                    simplify(&pts, 0.5 / zoom)
                } else {
                    pts
                };
                let mut next = current;
                next.push(InkStroke {
                    points: flatten(&kept),
                    ..base
                });
                self.ink_commit(next, cx);
            }
            InkGesture::Erase {
                gone,
                shapes: gone_shapes,
                links: gone_links,
                ..
            } => {
                if gone.is_empty() && gone_shapes.is_empty() && gone_links.is_empty() {
                    cx.notify();
                    return;
                }
                if self.overlays.ink.selected.as_ref().is_some_and(|s| gone.contains(s)) {
                    self.overlays.ink.selected = None;
                }
                // A shape's links go with it.
                let mut change = nodus_board::drawing::Change::default();
                for s in self.shapes().iter().filter(|s| gone_shapes.contains(&s.id)) {
                    change.shape(Some(s.clone()), None);
                }
                for l in self
                    .links()
                    .iter()
                    .filter(|l| gone_links.contains(&l.id) || gone_shapes.iter().any(|id| l.touches(id)))
                {
                    change.link(Some(l.clone()), None);
                }
                let next = current.into_iter().filter(|s| !gone.contains(&s.id)).collect();
                self.ink_commit_with(next, change, cx);
            }
            InkGesture::Move { id, dx, dy, moved, .. } => {
                if !moved {
                    cx.notify();
                    return;
                }
                let next = current
                    .into_iter()
                    .map(|s| if s.id == id { translate(&s, dx, dy) } else { s })
                    .collect();
                self.ink_commit(next, cx);
            }
        }
        cx.notify();
    }
}
