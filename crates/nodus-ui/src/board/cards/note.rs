//! Sticky notes (`NoteNode.tsx`; NOTEUI-001…NOTEUI-026): paper in the
//! note's colour, written in place, signed by the session that wrote it,
//! with ▶ to start a session on it; also the pad's top sheet, peeling up
//! under the pointer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::component::Sizable as _;
use gpui::component::input::{InputEvent, Position, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, Entity, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    linear_color_stop, linear_gradient, px, radians,
};
use nodus_domain::BoardNote;

use super::icons::{CardIcon, card_icon};
use super::paper::paper;
use super::para::{Space, para};
use super::present::{self, NOTE_MIN, Signature, SignatureTarget, can_start, note_size};
use super::resize::{Handles, resize_zones};
use super::{BoardEnv, Resized, Shade, Shadow, anim, ui_font};
use crate::board::nodes::NoteNode;
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::rgba;
use crate::theme::{self, fonts, motion};

/// NOTEUI-014: typing rests this long before it's saved.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// A note node (a free note, or the pad's top sheet when `node.draft`),
/// signed by `signature` if Claude wrote it.
pub fn render(
    env: &Rc<BoardEnv>,
    node: &NoteNode<'_>,
    signature: Option<Signature>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let size = (node.node.size.width, node.node.size.height);
    note(
        env,
        node.note,
        node.draft,
        node.node.selected,
        node.node.dragging,
        size,
        signature,
        window,
        cx,
    )
}

/// The note box.
#[allow(clippy::too_many_arguments)]
pub fn note(
    env: &Rc<BoardEnv>,
    note: &BoardNote,
    draft: bool,
    selected: bool,
    dragging: bool,
    size: (f32, f32),
    signature: Option<Signature>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = note.id.clone();
    let editing = !draft && env.renaming.as_deref() == Some(id.as_str());
    let (w, h) = size;
    let paper = paper(note.color, &t);
    let hover = window.use_keyed_state(anim::key(&id, "note-hover"), cx, |_, _| false);
    let hovered = *hover.read(cx);

    // NOTEUI-005…NOTEUI-007, NOTEUI-044
    let ring = selected || editing;
    let lifted = dragging || (draft && hovered);
    let layers = if lifted { &t.note_shadow_lifted } else { &t.note_shadow };
    let target = if ring && !(dragging && !selected) {
        Shadow::with(&[Shade::ring(2.0, t.accent)], layers)
    } else {
        Shadow::with(&[], layers)
    };
    let ease = motion::ease_out as fn(f32) -> f32;
    let ms = Duration::from_millis;
    let shadow = transition(window, cx, anim::key(&id, "note-shadow"), target, ms(220), ease);
    // NOTEUI-006: dragged, −1.5° and 1.02 about the centre; NOTEUI-044: the
    // pad's top sheet peels −2° and 2px up about its bottom-left corner.
    let (rot, ty, scale) = if dragging {
        (-1.5, 0.0, 1.02)
    } else if draft && hovered {
        (-2.0, -2.0, 1.0)
    } else {
        (0.0, 0.0, 1.0)
    };
    let rot = transition(window, cx, anim::key(&id, "note-rot"), rot, ms(220), ease);
    let ty = transition(window, cx, anim::key(&id, "note-ty"), ty, ms(220), ease);
    let scale = transition(window, cx, anim::key(&id, "note-scale"), scale, ms(220), ease);

    let signed = signature.is_some() && !editing;
    let content_h = h - 32.0 - if signed { 22.0 } else { 0.0 };
    let body: AnyElement = if editing {
        editor(env, note, window, cx)
    } else {
        // NOTEUI-008, NOTEUI-009
        let empty = note.text.is_empty();
        let text = if empty {
            if draft {
                String::new()
            } else {
                "Double-click to write".into()
            }
        } else {
            note.text.clone()
        };
        let fade_from = ((content_h - 18.0) / content_h.max(1.0)).clamp(0.0, 1.0);
        div()
            .w_full()
            .h(z.px(content_h.max(0.0)))
            .overflow_hidden()
            .mask(linear_gradient(
                180.,
                linear_color_stop(rgba(0, 0, 0, 1.0), fade_from),
                linear_color_stop(rgba(0, 0, 0, 0.0), 1.0),
            ))
            .child(
                para(
                    text,
                    ui_font(FontWeight::NORMAL),
                    z.px(15.),
                    z.px(21.),
                    if empty { t.note_ink_faint } else { t.note_ink },
                )
                .zoom(z.0)
                .space(Space::PreWrap),
            )
            .into_any_element()
    };

    let hs = hover.clone();
    let mut paper_el = div()
        .id(anim::key(&id, "note"))
        .absolute()
        .left_0()
        .top_0()
        .w(z.px(w))
        .h(z.px(h))
        .pt(z.px(16.))
        .px(z.px(18.))
        .rounded(z.px(4.))
        .bg(paper)
        .shadow(shadow.soft(z))
        .children(shadow.rings(4.0, z))
        .text_color(t.note_ink)
        .font_family(fonts::UI)
        .cursor(if draft {
            if dragging {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            }
        } else {
            CursorStyle::Arrow
        })
        // NOTEUI-004: padding 16/18, radius 4, the note shadow, the ink;
        // NOTEUI-043: the top sheet's grab cursor and tooltip.
        .when(draft, |d| {
            d.tooltip(native_tooltip(
                "Drag a note onto the board, or click for one beside the pad".into(),
            ))
        })
        // NOTEUI-045: on a server board, who wrote it.
        .when_some(
            present::note_author(note).filter(|_| !draft && !editing),
            |d, author| d.tooltip(native_tooltip(author.into())),
        )
        .on_hover(move |h, window, cx| {
            hs.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        // NOTEUI-003: a touch darker along the top, where it sticks.
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .w_full()
                .h(z.px(22.))
                .rounded_t(z.px(4.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0, 0, 0, 0.035), 0.),
                    linear_color_stop(rgba(0, 0, 0, 0.0), 1.),
                )),
        )
        .child(body);
    if rot != 0.0 || ty != 0.0 || scale != 1.0 {
        paper_el = if draft {
            paper_el.transform_origin(0.0, 1.0)
        } else {
            paper_el.transform_origin(0.5, 0.5)
        }
        .rotate(radians(rot.to_radians()))
        .translate(px(0.), z.px(ty))
        .scale(scale);
    }
    if let Some(sig) = signature.filter(|_| !editing) {
        paper_el = paper_el.child(signature_el(env, &id, sig, window, cx));
    }
    if can_start(note, draft, editing) {
        paper_el = paper_el.child(play(env, note, hovered, dragging, paper, window, cx));
    }

    let mut root = div().relative().w(z.px(w)).h(z.px(h)).child(paper_el);
    if !draft && selected && !editing {
        // NOTEUI-025, NOTEUI-026
        root = root.children(resize_zones(
            env,
            &id,
            Resized::Note(Box::new(note.clone())),
            (note.x as f32, note.y as f32, w, h),
            (NOTE_MIN, NOTE_MIN),
            // A 9px handle with its 1.5px border outside it (content-box).
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

/// The size a note draws at (NOTEUI-001).
pub fn size(note: &BoardNote) -> (f32, f32) {
    note_size(note)
}

/// The signature along the bottom (NOTEUI-018…NOTEUI-021).
/// NOTEUI-020: 18px in, 46px from the right, 13px up, faint, underlined
/// and inked on hover.
fn signature_el(env: &Rc<BoardEnv>, id: &str, sig: Signature, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let hover = window.use_keyed_state(anim::key(id, "sig-hover"), cx, |_, _| false);
    let on = *hover.read(cx);
    let color = transition(
        window,
        cx,
        anim::key(id, "sig-color"),
        if on { t.note_ink } else { t.note_ink_faint },
        Duration::from_millis(120),
        motion::ease,
    );
    let h2 = hover.clone();
    let env2 = env.clone();
    let target = sig.target.clone();
    div()
        .id(anim::key(id, "sig"))
        .absolute()
        .left(z.px(18.))
        .right(z.px(46.))
        .bottom(z.px(13.))
        .cursor(CursorStyle::Arrow)
        .tooltip(native_tooltip(sig.tooltip.clone().into()))
        .on_hover(move |h, window, cx| {
            h2.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            match &target {
                SignatureTarget::Here { session_id } => env2.actions.open_session(session_id, cx),
                SignatureTarget::Away { project_id, session_id } => {
                    env2.actions.show_card(project_id, session_id.as_deref(), cx);
                }
            }
        })
        .child(
            para(sig.text, ui_font(FontWeight::NORMAL), z.px(12.), z.px(16.), color)
                .zoom(z.0)
                .ellipsis()
                .underline(on.then_some(color.into())),
        )
        .into_any_element()
}

/// ▶: start a session on the note (NOTEUI-022…NOTEUI-024).
#[allow(clippy::too_many_arguments)]
/// NOTEUI-023: the 28px circle, the paper-coloured ▶, accent on hover,
/// 0.92 pressed.
fn play(
    env: &Rc<BoardEnv>,
    note: &BoardNote,
    node_hovered: bool,
    dragging: bool,
    paper: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = note.id.clone();
    let hover = window.use_keyed_state(anim::key(&id, "play-hover"), cx, |_, _| false);
    let pressed = window.use_keyed_state(anim::key(&id, "play-press"), cx, |_, _| false);
    let on = *hover.read(cx);
    let down = *pressed.read(cx);
    let ms = Duration::from_millis;
    let shown = node_hovered && !dragging;
    let opacity = transition(
        window,
        cx,
        anim::key(&id, "play-o"),
        if shown { 1.0 } else { 0.0 },
        ms(120),
        motion::ease,
    );
    let scale = transition(
        window,
        cx,
        anim::key(&id, "play-s"),
        if down && on {
            0.92
        } else if node_hovered {
            1.0
        } else {
            0.85
        },
        ms(220),
        motion::ease_spring,
    );
    let bg = transition(
        window,
        cx,
        anim::key(&id, "play-bg"),
        if on { t.accent } else { rgba(0, 0, 0, 0.656) },
        ms(120),
        motion::ease,
    );
    let icon_color = if on { t.text_on_accent } else { paper };
    let (h2, p_down, p_up) = (hover.clone(), pressed.clone(), pressed.clone());
    let env2 = env.clone();
    div()
        .id(anim::key(&id, "play"))
        .aria_label("Start a Claude session on this note")
        .absolute()
        .right(z.px(10.))
        .bottom(z.px(10.))
        .size(z.px(28.))
        .pl(z.px(2.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(bg)
        .shadow(
            Shadow::of(&[Shade {
                y: 2.0,
                blur: 6.0,
                color: rgba(0, 0, 0, 0.18),
                ..Shade::NONE
            }])
            .gpui(z),
        )
        // `.note-play`'s ink is see-through: its shadow only outside it.
        .shadow_outside()
        .opacity(opacity)
        .when(scale != 1.0, |d| d.scale(scale))
        .cursor(CursorStyle::Arrow)
        .tooltip(native_tooltip("Start a Claude session on this note".into()))
        .on_hover(move |h, window, cx| {
            h2.update(cx, |s, _| *s = *h);
            window.refresh();
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            p_down.update(cx, |s, _| *s = true);
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, move |_, window, cx| {
            p_up.update(cx, |s, _| *s = false);
            window.refresh();
        })
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            env2.actions.start_note(&id, cx);
        })
        .child(card_icon(CardIcon::PlayFilled, z.f(12.), 2.5, icon_color))
        .into_any_element()
}

/// Writing in a note (NOTEUI-011…NOTEUI-017).
pub struct NoteEditor {
    field: Entity<TextareaState>,
    /// The note's spelling (`.note-input` is a textarea, spellchecked).
    spell: Entity<crate::components::spellcheck::Spellcheck>,
    saver: Rc<Saver>,
    _subscriptions: Vec<Subscription>,
}

/// What's typed and what's saved, shared with the field's listeners.
struct Saver {
    env: Rc<BoardEnv>,
    project_id: String,
    note: RefCell<BoardNote>,
    saved: RefCell<String>,
    value: RefCell<String>,
    timer: RefCell<Option<Task<()>>>,
    done: Cell<bool>,
}

impl Saver {
    /// Saves the text if it changed since the last save.
    fn flush(&self, cx: &mut App) {
        self.timer.borrow_mut().take();
        let value = self.value.borrow().clone();
        if *self.saved.borrow() == value {
            return;
        }
        *self.saved.borrow_mut() = value.clone();
        let note = self.note.borrow().clone();
        self.env.actions.edit_note(&self.project_id, &note, &value, cx);
    }
}

fn editor(env: &Rc<BoardEnv>, note: &BoardNote, window: &mut Window, cx: &mut App) -> AnyElement {
    let z = env.z();
    let t = theme::tokens(cx);
    let id = note.id.clone();
    let editor = window.use_keyed_state(anim::key(&id, "note-editor"), cx, {
        let env = env.clone();
        let note = note.clone();
        move |window, cx: &mut Context<'_, NoteEditor>| {
            let text = note.text.clone();
            let field = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx)
                    .placeholder("Type a note…")
                    .context_menu(false);
                state.set_value(text.clone(), window, cx);
                // Caret at the end, focused.
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
            let saver = Rc::new(Saver {
                env: env.clone(),
                project_id: env.project_id.clone(),
                note: RefCell::new(note.clone()),
                saved: RefCell::new(text.clone()),
                value: RefCell::new(text),
                timer: RefCell::new(None),
                done: Cell::new(false),
            });
            let typing = saver.clone();
            let blurring = saver.clone();
            let sub = cx.subscribe_in(&field, window, move |_, field, e: &InputEvent, window, cx| match e {
                InputEvent::Change => {
                    *typing.value.borrow_mut() = field.read(cx).value().to_string();
                    // NOTEUI-014: saved 500ms after the last keystroke.
                    let later = typing.clone();
                    let task = window.spawn(cx, async move |cx| {
                        cx.background_executor().timer(SAVE_DELAY).await;
                        let _ = cx.update(|_, cx| later.flush(cx));
                    });
                    *typing.timer.borrow_mut() = Some(task);
                }
                // NOTEUI-013: leaving the field saves and ends writing.
                InputEvent::Blur if !blurring.done.replace(true) => {
                    blurring.flush(cx);
                    blurring.env.actions.set_renaming(None, cx);
                }
                _ => {}
            });
            // NOTEUI-014: the field going away for any other reason keeps what was typed.
            let releasing = saver.clone();
            let release = cx.on_release(move |_, cx| releasing.flush(cx));
            let focus = field.clone();
            window.defer(cx, move |window, cx| focus.update(cx, |f, cx| f.focus(window, cx)));
            let spell = crate::components::spellcheck::spellcheck(&field, false, window, cx);
            NoteEditor {
                field,
                spell,
                saver,
                _subscriptions: vec![sub, release],
            }
        }
    });
    let (field, saver, spell) = {
        let e = editor.read(cx);
        (e.field.clone(), e.saver.clone(), e.spell.clone())
    };
    // The note changes under the editor (a move, a recolour): save onto it.
    *saver.note.borrow_mut() = note.clone();
    let platform = env.platform;
    div()
        .id(anim::key(&id, "note-field"))
        .relative()
        .size_full()
        .font_family(fonts::UI)
        .text_size(z.px(15.))
        .line_height(z.px(21.))
        .text_color(t.note_ink)
        .cursor(CursorStyle::IBeam)
        // NOTEUI-012: Escape or ⌘Enter puts the note down.
        .capture_key_down(move |e: &KeyDownEvent, window, cx| {
            let k = &e.keystroke;
            if k.key == "escape" || (k.key == "enter" && platform.mod_key(&k.modifiers)) {
                cx.stop_propagation();
                window.blur(cx);
                if !saver.done.replace(true) {
                    saver.flush(cx);
                    saver.env.actions.set_renaming(None, cx);
                }
            }
        })
        // NOTEUI-016: every key, press and wheel stays in the field. A key
        // is marked, not stopped: a stopped key counts as handled, and the
        // platform then never types its character into the field.
        .on_key_down(|_, window, _| window.prevent_default())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_scroll(|_, _, cx| cx.stop_propagation())
        // The field's own 4px side padding (its smallest) sits outside the
        // note's, and so do the 10px it keeps clear at the right of every
        // line (gpui-component's `RIGHT_MARGIN`): it wraps where the note's
        // text does (NOTEUI-011).
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-4.))
                .right(px(-14.))
                .child(WithCaret {
                    color: t.note_ink.into(),
                    child: Textarea::new(&field)
                        .appearance(false)
                        .bordered(false)
                        .xsmall()
                        .h(gpui::relative(1.))
                        .text_size(z.px(15.))
                        .line_height(z.px(21.))
                        // At any zoom, lines as far apart as the note's text.
                        .fractional_line_height()
                        .font_family(fonts::UI)
                        .text_color(t.note_ink)
                        .into_any_element(),
                })
                .child(crate::components::spellcheck::underlines(
                    &spell,
                    &field,
                    z.px(15.),
                    z.f(1.),
                )),
        )
        .into_any_element()
}

/// Draws a field with the text caret in `color` (NOTEUI-011: the note's
/// ink): gpui-component takes the caret's colour from its theme as the
/// field lays out and paints, so the theme's is swapped for those moments.
pub(super) struct WithCaret {
    pub(super) color: gpui::Hsla,
    pub(super) child: AnyElement,
}

impl IntoElement for WithCaret {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for WithCaret {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let color = self.color;
        let id = with_caret(color, cx, |cx| self.child.request_layout(window, cx));
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = self.color;
        with_caret(color, cx, |cx| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = self.color;
        with_caret(color, cx, |cx| self.child.paint(window, cx));
    }
}

/// Runs `f` with the theme's caret in `color` (transparent in parity
/// shots, which hide the caret), then puts the theme's back.
fn with_caret<R>(color: gpui::Hsla, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
    use gpui::component::theme::Theme;
    let saved = Theme::global(cx).caret;
    Theme::global_mut(cx).caret = if crate::parity::frozen(cx) {
        gpui::transparent_black()
    } else {
        color
    };
    let out = f(cx);
    Theme::global_mut(cx).caret = saved;
    out
}
