//! Shapes on the board (spec 14, SHAPE-001, SHAPE-012…SHAPE-016): a
//! rectangle, ellipse or diamond tinted with its colour and outlined in
//! it, words centred in it; words on their own; a sticker's emoji; a
//! picture. Selected, a shape shows an accent ring and resizes from its
//! edges; a double-click writes in it.

use std::cell::Cell;
use std::rc::Rc;

use gpui::component::Sizable as _;
use gpui::component::input::{InputEvent, Position, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, Entity, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ObjectFit, ParentElement, PathBuilder, PathStyle, Rgba, StatefulInteractiveElement,
    Styled, StyledImage as _, Subscription, Window, canvas, div, img, point, px,
};
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};
use nodus_domain::ShapeKind;

use super::note::WithCaret;
use super::resize::{Handles, resize_zones};
use super::{BoardEnv, Resized, anim, mix};
use crate::board::nodes::ShapeNode;
use crate::board::overlays::ink::{ink_rgba, shape_path};
use crate::board::shapes;
use crate::theme::{self, fonts, motion};

/// The smallest a shape resizes to, in board units.
pub const SHAPE_MIN: f32 = 16.0;
/// Words in a shape: 15 units on lines of 21; on their own, 18 on 24.
const TEXT: (f32, f32) = (15.0, 21.0);
const WORDS: (f32, f32) = (18.0, 24.0);
/// How far words keep in from a box's edges.
const INSET: f32 = 12.0;

/// A shape node.
pub fn render(env: &Rc<BoardEnv>, node: &ShapeNode<'_>, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let shape = node.shape;
    let (w, h) = (node.node.size.width, node.node.size.height);
    let color = ink_rgba(&t, shape.color);
    let editing = env.renaming.as_deref() == Some(shape.id.as_str()) && shape.kind.has_text();
    let selected = node.node.selected;

    // MOTION-021: Claude drawing it: the outline traced, then the fill and
    // the words; words on their own typed; a sticker stamped; a picture
    // coming into focus.
    let drawing = node.drawing.filter(|_| !editing);
    let body: AnyElement = match shape.kind {
        ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Diamond => {
            let kind = shape.kind;
            let share = drawing.map(shapes::drawn_share);
            let mut fill = mix(color, 0.14, t.bg_elevated);
            if let Some((_, f, _)) = share {
                fill.a *= f;
            }
            let stroke = z.f(2.0);
            let frame = canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let o = bounds.origin;
                    let r = nodus_domain::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: f64::from(f32::from(bounds.size.width)),
                        h: f64::from(f32::from(bounds.size.height)),
                    };
                    let at = |p: [f64; 2]| point(o.x + px(p[0] as f32), o.y + px(p[1] as f32));
                    if fill.a > 0.0 {
                        let mut body = PathBuilder::fill();
                        shape_path(&mut body, kind, r, &at);
                        if let Ok(body) = body.build() {
                            window.paint_path(body, fill);
                        }
                    }
                    // The outline inside the box, so its edge is the box's.
                    let half = f64::from(stroke) / 2.0;
                    let inner = nodus_domain::Rect {
                        x: half,
                        y: half,
                        w: (r.w - 2.0 * half).max(0.0),
                        h: (r.h - 2.0 * half).max(0.0),
                    };
                    let options = StrokeOptions::default()
                        .with_line_width(stroke)
                        .with_line_join(LineJoin::Round)
                        .with_line_cap(LineCap::Round);
                    let mut line = PathBuilder::stroke(px(stroke)).with_style(PathStyle::Stroke(options));
                    match share {
                        Some((traced, _, _)) => {
                            let (part, _) = shapes::line_part(&shapes::outline_line(kind, inner), f64::from(traced));
                            let mut points = part.into_iter().map(|p| at([p.x, p.y]));
                            let Some(first) = points.next() else { return };
                            line.move_to(first);
                            for p in points {
                                line.line_to(p);
                            }
                        }
                        None => shape_path(&mut line, kind, inner, &at),
                    }
                    if let Ok(line) = line.build() {
                        window.paint_path(line, color);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full();
            // A diamond's words keep to its middle half.
            let (iw, ih) = match kind {
                ShapeKind::Diamond => (w / 2.0, h / 2.0),
                _ => ((w - 2.0 * INSET).max(0.0), (h - 2.0 * INSET).max(0.0)),
            };
            let words = words(
                env,
                node,
                TEXT,
                FontWeight::MEDIUM,
                t.text_primary,
                (iw, ih),
                editing,
                share.map(|(_, _, w)| w),
                window,
                cx,
            );
            div()
                .relative()
                .size_full()
                .child(frame)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(words),
                )
                .into_any_element()
        }
        ShapeKind::Text => {
            let words = words(
                env,
                node,
                WORDS,
                FontWeight::SEMIBOLD,
                color,
                (w, h),
                editing,
                drawing,
                window,
                cx,
            );
            div().size_full().flex().items_center().child(words).into_any_element()
        }
        ShapeKind::Sticker => {
            let stamp = div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(z.px(w.min(h) * 0.78))
                .line_height(z.px(h))
                .child(shape.text.clone());
            match drawing {
                // Stamped: in from small and turned, on the spring.
                Some(p) => {
                    let e = motion::ease_spring(p);
                    stamp
                        .opacity(motion::ease_out((p / 0.3).min(1.0)))
                        .rotate(gpui::radians((-14.0 * (1.0 - e)).to_radians()))
                        .scale(0.2 + 0.8 * e)
                        .into_any_element()
                }
                None => stamp.into_any_element(),
            }
        }
        ShapeKind::Image => {
            let radius = z.px(8.0);
            match &node.image {
                Some(path) => {
                    let picture = div()
                        .size_full()
                        .rounded(radius)
                        .overflow_hidden()
                        .shadow(t.sm())
                        .child(
                            img(path.clone())
                                .size_full()
                                .rounded(radius)
                                .object_fit(ObjectFit::Contain),
                        );
                    match drawing {
                        // Coming into focus as it fades in.
                        Some(p) => {
                            let e = motion::ease_out(p);
                            let blur = 10.0 * z.0 * (1.0 - e);
                            let picture = picture.scale(0.96 + 0.04 * e).group_opacity(e);
                            if blur > 0.05 {
                                picture.blur(px(blur)).into_any_element()
                            } else {
                                picture.into_any_element()
                            }
                        }
                        None => picture.into_any_element(),
                    }
                }
                None => div()
                    .size_full()
                    .rounded(radius)
                    .bg(t.bg_elevated)
                    .border(z.px(1.0))
                    .border_color(t.border)
                    .into_any_element(),
            }
        }
    };

    // SHAPE-013: selected, an accent ring just outside it.
    let ring = (selected || editing).then(|| {
        let gap = z.f(3.0);
        div()
            .absolute()
            .left(px(-gap))
            .top(px(-gap))
            .w(z.px(w) + px(2.0 * gap))
            .h(z.px(h) + px(2.0 * gap))
            .rounded(z.px(6.0))
            .border(px(1.5))
            .border_color(t.accent)
    });
    let mut root = div()
        .relative()
        .w(z.px(w))
        .h(z.px(h))
        .when(shape.kind == ShapeKind::Text && !selected && !editing, |d| {
            d.cursor(CursorStyle::Arrow)
        })
        .child(body)
        .children(ring);
    if selected && !editing {
        // SHAPE-011: from its edges and corners, as notes resize.
        root = root.children(resize_zones(
            env,
            &shape.id,
            Resized::Shape(shape.id.clone()),
            (shape.x as f32, shape.y as f32, w, h),
            (SHAPE_MIN, SHAPE_MIN),
            12.0,
            Some(Handles {
                size: 9.0,
                border: 1.5,
                border_color: t.accent,
                fill: t.bg_elevated,
                radius: 2.0,
            }),
            window,
            cx,
        ));
    }
    root.into_any_element()
}

/// A shape's words, centred, or the field they're written in.
#[allow(clippy::too_many_arguments)]
fn words(
    env: &Rc<BoardEnv>,
    node: &ShapeNode<'_>,
    (size, line): (f32, f32),
    weight: FontWeight,
    color: Rgba,
    (w, h): (f32, f32),
    editing: bool,
    typed: Option<f32>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let shape = node.shape;
    let text_only = shape.kind == ShapeKind::Text;
    if editing {
        return editor(env, node, (size, line), weight, color, (w, h), window, cx);
    }
    // MOTION-021: Claude typing them.
    let text: gpui::SharedString = match typed {
        Some(share) => {
            let n = (shape.text.chars().count() as f32 * share).ceil() as usize;
            shape.text.chars().take(n).collect::<String>().into()
        }
        None => shape.text.clone().into(),
    };
    div()
        .w(z.px(w))
        .max_h(z.px(h))
        .overflow_hidden()
        .font_family(fonts::UI)
        .font_weight(weight)
        .text_size(z.px(size))
        .line_height(z.px(line))
        .text_color(color)
        .when(!text_only, Styled::text_center)
        .child(text)
        .into_any_element()
}

/// Writing in a shape (SHAPE-012): Esc, ⌘Return or leaving the field puts
/// the words in.
struct ShapeEditor {
    field: Entity<TextareaState>,
    done: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

#[allow(clippy::too_many_arguments)]
fn editor(
    env: &Rc<BoardEnv>,
    node: &ShapeNode<'_>,
    (size, line): (f32, f32),
    weight: FontWeight,
    color: Rgba,
    (w, h): (f32, f32),
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let id = node.shape.id.clone();
    let editor = window.use_keyed_state(anim::key(&id, "shape-editor"), cx, {
        let env = env.clone();
        let id = id.clone();
        let text = node.shape.text.clone();
        move |window, cx: &mut Context<'_, ShapeEditor>| {
            let field = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx).placeholder("Type…").context_menu(false);
                state.set_value(text.clone(), window, cx);
                let lines: Vec<&str> = text.split('\n').collect();
                let last = lines.last().map_or(0, |l| l.encode_utf16().count());
                state.set_cursor_position(
                    Position {
                        line: lines.len().saturating_sub(1) as u32,
                        character: last as u32,
                    },
                    window,
                    cx,
                );
                state
            });
            let done = Rc::new(Cell::new(false));
            let finishing = done.clone();
            let sub = cx.subscribe_in(&field, window, move |_, field, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Blur) && !finishing.replace(true) {
                    let text = field.read(cx).value().to_string();
                    env.actions.edit_shape(&id, &text, cx);
                    env.actions.set_renaming(None, cx);
                }
            });
            let focus = field.clone();
            window.defer(cx, move |window, cx| focus.update(cx, |f, cx| f.focus(window, cx)));
            ShapeEditor {
                field,
                done,
                _subscriptions: vec![sub],
            }
        }
    });
    let (field, done) = {
        let e = editor.read(cx);
        (e.field.clone(), e.done.clone())
    };
    let finish_field = field.clone();
    let env2 = env.clone();
    let platform = env.platform;
    let finish_id = id.clone();
    div()
        .id(anim::key(&id, "shape-field"))
        .relative()
        .w(z.px(w))
        .h(z.px(h))
        .font_family(fonts::UI)
        .font_weight(weight)
        .text_size(z.px(size))
        .line_height(z.px(line))
        .text_color(color)
        .cursor(CursorStyle::IBeam)
        .capture_key_down(move |e: &KeyDownEvent, window, cx| {
            let k = &e.keystroke;
            if k.key == "escape" || (k.key == "enter" && platform.mod_key(&k.modifiers)) {
                cx.stop_propagation();
                if !done.replace(true) {
                    let text = finish_field.read(cx).value().to_string();
                    env2.actions.edit_shape(&finish_id, &text, cx);
                    env2.actions.set_renaming(None, cx);
                }
                // The board's keys work again at once (A for an arrow…).
                env2.actions.focus_board(window, cx);
            }
        })
        // Keys, presses and the wheel stay in the field (NOTEUI-016's way).
        .on_key_down(|_, window, _| window.prevent_default())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-4.))
                .right(px(-14.))
                .child(WithCaret {
                    color: color.into(),
                    child: Textarea::new(&field)
                        .appearance(false)
                        .bordered(false)
                        .xsmall()
                        .h(gpui::relative(1.))
                        .text_size(z.px(size))
                        .line_height(z.px(line))
                        .fractional_line_height()
                        .font_family(fonts::UI)
                        .text_color(color)
                        .into_any_element(),
                }),
        )
        .into_any_element()
}
