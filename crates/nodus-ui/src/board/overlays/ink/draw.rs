//! Drawing the ink (INKUI-025, INKUI-026) and the label being typed.

use super::*;

impl BoardCanvas {
    // ---- drawing ----------------------------------------------------------------------

    /// The listeners that take the pointer ahead of the board: drawn first,
    /// so the window's capture phase runs them before any other.
    pub(crate) fn render_ink_input(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let me = cx.entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_down(e, window, cx));
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_move(e, window, cx));
                    }
                });
                let m = me.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        m.update(cx, |c, cx| c.ink_mouse_up(e, cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    /// INKUI-006, INKUI-016, INKUI-017: the board's surface, above every
    /// node (the open archive panel's too) and under the HUD and the bar: a
    /// press is the ink's when this is hovered, and the tool's cursor wins
    /// over the cards' own: crosshair, I-beam, the eraser's circle, or with
    /// the pointer over a stroke, `move`.
    pub(crate) fn render_ink_hitbox(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let tool = self.overlays.ink.tool;
        let hover = self.overlays.ink.hover;
        let me = cx.entity();
        canvas(
            move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, cx| {
                match tool {
                    InkTool::Pen | InkTool::Arrow => window.set_cursor_style(CursorStyle::Crosshair, &hitbox),
                    InkTool::Eraser => window.set_cursor_style(cursor::eraser(), &hitbox),
                    InkTool::Text => window.set_cursor_style(CursorStyle::IBeam, &hitbox),
                    InkTool::Select if hover => window.set_cursor_style(CursorStyle::Move, &hitbox),
                    InkTool::Select => {}
                }
                me.update(cx, |c, _| c.overlays.ink.hitbox = Some(hitbox));
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// INKUI-025, INKUI-026: lines in stored order, the live stroke, the
    /// selection outline, then labels and the label being typed; above every
    /// card, under the open archive panel.
    pub(crate) fn render_ink_layer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let ink = &self.overlays.ink;
        let mut shown = self.ink_strokes();
        if let Some(InkGesture::Move {
            id,
            dx,
            dy,
            moved: true,
            ..
        }) = &ink.gesture
        {
            for s in &mut shown {
                if &s.id == id {
                    *s = translate(s, *dx, *dy);
                }
            }
        }
        let erasing: Vec<String> = match &ink.gesture {
            Some(InkGesture::Erase { gone, .. }) => gone.clone(),
            _ => Vec::new(),
        };
        let live = match &ink.gesture {
            Some(InkGesture::Draw { base, pts, .. }) => Some(InkStroke {
                points: flatten(pts),
                ..base.clone()
            }),
            _ => None,
        };
        let editing = ink.label.as_ref().and_then(|l| l.id.clone());
        let picked = {
            let measure = measurer(window);
            ink.selected
                .as_ref()
                .and_then(|id| shown.iter().find(|s| &s.id == id))
                .map(|s| (stroke_box(s, &measure), s.size))
        };
        let vp = self.viewport;
        let z = vp.zoom;

        let lines: Vec<(Vec<Seg>, f64, Rgba)> = shown
            .iter()
            .filter(|s| s.kind != InkKind::Text)
            .chain(live.iter())
            .map(|s| {
                let mut color = ink_rgba(&t, s.color);
                if erasing.contains(&s.id) {
                    color.a *= 0.25;
                }
                (ink_draw::stroke_path(s), PEN_WIDTH * s.size, color)
            })
            .collect();
        let accent = t.accent;
        let layer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let at = |p: Pt| {
                    point(
                        bounds.origin.x + px((vp.x + p[0] * z) as f32),
                        bounds.origin.y + px((vp.y + p[1] * z) as f32),
                    )
                };
                for (segs, width, color) in &lines {
                    paint_line(segs, (width * z) as f32, *color, &at, window);
                }
                if let Some((b, size)) = picked {
                    let pad = 6.0 * size;
                    let r = (4.0 * size * z) as f32;
                    let origin = at([b.x - pad, b.y - pad]);
                    let (w, h) = (((b.w + 2.0 * pad) * z) as f32, ((b.h + 2.0 * pad) * z) as f32);
                    paint_dashed_rect(origin, w, h, r, 1.5, (5.0, 4.0), accent, window);
                }
            },
        )
        .absolute()
        .size_full();

        let labels: Vec<AnyElement> = shown
            .iter()
            .filter(|s| s.kind == InkKind::Text && Some(&s.id) != editing.as_ref())
            .map(|s| {
                let mut color = ink_rgba(&t, s.color);
                if erasing.contains(&s.id) {
                    color.a *= 0.25;
                }
                label_element(
                    (vp.x + s.points[0] * z) as f32,
                    (vp.y + s.points.get(1).copied().unwrap_or(0.0) * z) as f32,
                    (s.size * z) as f32,
                    color,
                )
                .child(s.text.clone().unwrap_or_default())
                .into_any_element()
            })
            .collect();

        let input = self.render_label_input(&t, window, cx);
        div()
            .absolute()
            .size_full()
            .child(layer)
            .children(labels)
            .children(input)
            .into_any_element()
    }

    /// The label being typed (`.ink-label-input`): in the label's font and
    /// colour, as wide as its text (at least 48 units), a dashed accent
    /// outline 3 units outside it.
    pub(crate) fn render_label_input(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let label = self.overlays.ink.label.as_ref()?;
        let vp = self.viewport;
        let z = vp.zoom as f32;
        let size = label.size as f32;
        let color = ink_rgba(t, label.color);
        let text = label.input.read(cx).value().to_string();
        let measured = label_width(
            window.text_system(),
            if text.is_empty() { "Label" } else { &text },
            label.size,
        ) as f32;
        let w = measured.max(48.0) * z + 2.0;
        let h = 20.0 * size * z;
        let (left, top) = ((vp.x as f32) + label.x as f32 * z, (vp.y as f32) + label.y as f32 * z);
        let off = 3.0 * z;
        let outline = 1.5 * z;
        let accent = t.accent;
        let ring = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                paint_dashed_rect(
                    point(o.x - px(off), o.y - px(off)),
                    w + 2.0 * off,
                    h + 2.0 * off,
                    (2.0 + 3.0) * z,
                    outline,
                    (2.0 * outline, outline),
                    accent,
                    window,
                );
            },
        )
        .absolute()
        .size_full();
        let placeholder = text.is_empty().then(|| {
            label_element(
                0.0,
                0.0,
                size * z,
                Rgba {
                    a: color.a * 0.4,
                    ..color
                },
            )
            .child("Label")
        });
        Some(
            div()
                .id(("ink-label-input", self.overlays.ink.label_key))
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .occlude()
                .font_family(fonts::DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(LABEL_FONT as f32 * size * z))
                .line_height(px(h))
                .text_color(color)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // INKUI-013: Return puts it on the board, Esc leaves it off,
                // and neither reaches the board.
                .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                    if this.ink_label_composing(window, cx) {
                        return;
                    }
                    match e.keystroke.key.as_str() {
                        "enter" => {
                            cx.stop_propagation();
                            this.ink_commit_label(window, cx);
                        }
                        "escape" => {
                            cx.stop_propagation();
                            this.ink_discard_label(window, cx);
                        }
                        _ => {}
                    }
                }))
                .child(ring)
                .children(placeholder)
                .child(
                    // Wider than its text, so it never scrolls sideways
                    // (`field-sizing: content` fits the text exactly).
                    div().absolute().left_0().top_0().w(px(w + 96.0)).h(px(h)).child(
                        // `Input::h` sizes a multi-line field; a single line takes the style's.
                        Styled::h(
                            Input::new(&label.input)
                                .appearance(false)
                                .px(px(0.))
                                .py(px(0.))
                                .text_size(px(LABEL_FONT as f32 * size * z))
                                .line_height(px(h)),
                            px(h),
                        ),
                    ),
                )
                .into_any_element(),
        )
    }
}

/// A label's box (`.ink-label`): display font, weight 600, one line.
pub(crate) fn label_element(left: f32, top: f32, scale: f32, color: Rgba) -> gpui::Div {
    let h = LABEL_LINE as f32 * scale;
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .h(px(h))
        .font_family(fonts::DISPLAY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(LABEL_FONT as f32 * scale))
        .line_height(px(h))
        .text_color(color)
        .whitespace_nowrap()
}

/// Strokes a path of segments in window coordinates, round caps and joins.
pub(crate) fn paint_line(
    segs: &[Seg],
    width: f32,
    color: Rgba,
    at: &dyn Fn(Pt) -> GPoint<Pixels>,
    window: &mut Window,
) {
    if segs.is_empty() || width <= 0.0 {
        return;
    }
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    let mut path = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    for seg in segs {
        match *seg {
            Seg::Move(p) => path.move_to(at(p)),
            Seg::Line(p) => path.line_to(at(p)),
            Seg::Quad(c, p) => path.curve_to(at(p), at(c)),
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A dashed rounded rectangle (`stroke-dasharray`, non-scaling), the dash
/// starting at the top edge after the corner as SVG's `rect` does.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_dashed_rect(
    origin: GPoint<Pixels>,
    w: f32,
    h: f32,
    r: f32,
    width: f32,
    dash: (f32, f32),
    color: Rgba,
    window: &mut Window,
) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (x, y) = (f32::from(origin.x), f32::from(origin.y));
    let options = StrokeOptions::default().with_line_width(width);
    let mut path = PathBuilder::stroke(px(width))
        .with_style(PathStyle::Stroke(options))
        .dash_array(&[px(dash.0), px(dash.1)]);
    let p = |px_: f32, py: f32| point(px(px_), px(py));
    // Quarter circles as cubic curves.
    let k = 0.5523;
    path.move_to(p(x + r, y));
    path.line_to(p(x + w - r, y));
    path.cubic_bezier_to(p(x + w, y + r), p(x + w - r + k * r, y), p(x + w, y + r - k * r));
    path.line_to(p(x + w, y + h - r));
    path.cubic_bezier_to(
        p(x + w - r, y + h),
        p(x + w, y + h - r + k * r),
        p(x + w - r + k * r, y + h),
    );
    path.line_to(p(x + r, y + h));
    path.cubic_bezier_to(p(x, y + h - r), p(x + r - k * r, y + h), p(x, y + h - r + k * r));
    path.line_to(p(x, y + r));
    path.cubic_bezier_to(p(x + r, y), p(x, y + r - k * r), p(x + r - k * r, y));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
