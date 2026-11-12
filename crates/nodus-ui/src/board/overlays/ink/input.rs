//! The pointer, taken ahead of the board (INKUI-006…INKUI-020).

use super::*;

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
        if over != self.overlays.ink.hover {
            self.overlays.ink.hover = over;
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
                if self.overlays.ink.selected.take().is_some() {
                    cx.notify();
                }
                return;
            };
            self.ink_take(window, cx);
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
        match tool {
            InkTool::Pen | InkTool::Arrow | InkTool::Rect => {
                self.overlays.ink.gesture = Some(InkGesture::Draw {
                    base: InkStroke {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: match tool {
                            InkTool::Pen => InkKind::Pen,
                            InkTool::Arrow => InkKind::Arrow,
                            _ => InkKind::Rect,
                        },
                        color,
                        points: Vec::new(),
                        size: size_at(zoom),
                        text: None,
                        extra: serde_json::Map::new(),
                    },
                    pts: vec![p],
                    zoom,
                });
            }
            InkTool::Eraser => {
                let tolerance = HIT_PX / zoom;
                self.overlays.ink.gesture = Some(InkGesture::Erase {
                    gone: Vec::new(),
                    last: p,
                    tolerance,
                });
                self.ink_erase_along(p, p, window, cx);
            }
            InkTool::Text => {
                let size = size_at(zoom);
                // The click lands mid-line, where the caret shows.
                let at = [p[0], p[1] - LABEL_LINE * size / 2.0];
                self.ink_open_label(None, at, size, color, "", window, cx);
            }
            InkTool::Select | InkTool::Bubble | InkTool::Mic => {}
        }
        cx.notify();
    }

    /// INKUI-011: the strokes the eraser passed over from `a` to `b`.
    pub(crate) fn ink_erase_along(&mut self, a: Pt, b: Pt, window: &Window, cx: &mut Context<'_, Self>) {
        let strokes = self.ink_strokes();
        let measure = measurer(window);
        let Some(InkGesture::Erase { gone, tolerance, .. }) = &mut self.overlays.ink.gesture else {
            return;
        };
        let left: Vec<InkStroke> = strokes.into_iter().filter(|s| !gone.contains(&s.id)).collect();
        let hit = strokes_along(&left, a, b, *tolerance, &measure);
        if !hit.is_empty() {
            gone.extend(hit);
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
        match &mut self.overlays.ink.gesture {
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
            InkGesture::Erase { gone, .. } => {
                if gone.is_empty() {
                    cx.notify();
                    return;
                }
                if self.overlays.ink.selected.as_ref().is_some_and(|s| gone.contains(s)) {
                    self.overlays.ink.selected = None;
                }
                let next = current.into_iter().filter(|s| !gone.contains(&s.id)).collect();
                self.ink_commit(next, cx);
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
