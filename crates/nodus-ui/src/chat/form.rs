//! A form of questions in the chat (spec 16, FORM): each question's choices
//! lettered a, b, c…, a note per question, the form's own notes, and a Send
//! button that hands the answers over as JSON. Once answered, the form shows
//! what was sent and takes nothing more.

use std::rc::Rc;

use gpui::component::Sizable as _;
use gpui::component::input::{Input, InputState, Textarea, TextareaState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_visual::questions::{self, Form, FormState, Question, letter};
use serde_json::Value;

use crate::components::icon::{Icon, icon};
use crate::components::text_field::new_input;
use crate::theme::tokens::Tokens;
use crate::theme::{self, fonts};

/// Told the answers when the user sends them.
pub type OnSend = Rc<dyn Fn(Value, &mut Window, &mut App)>;

/// A form in the chat, `scale` times its natural size.
pub struct FormView {
    form: Form,
    state: FormState,
    /// The answers sent; the form is read-only once there are some.
    answered: Option<Value>,
    scale: f32,
    on_send: OnSend,
    /// A text question's field, by question.
    texts: Vec<Option<Entity<TextareaState>>>,
    /// A choice question's note, by question.
    notes: Vec<Option<Entity<InputState>>>,
    /// The form's own notes, with two questions or more.
    extra: Option<Entity<TextareaState>>,
}

impl FormView {
    pub fn new(
        form: Form,
        answered: Option<Value>,
        scale: f32,
        on_send: OnSend,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let state = FormState::new(&form);
        let texts = form
            .questions
            .iter()
            .map(|q| {
                q.text.then(|| {
                    let placeholder = q.placeholder.clone().unwrap_or_else(|| "Write your answer".into());
                    cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .auto_grow(2, 8)
                            .placeholder(placeholder)
                            .context_menu(false)
                    })
                })
            })
            .collect();
        let notes = form
            .questions
            .iter()
            .map(|q| (!q.text).then(|| new_input("", "Add a note (optional)", false, window, cx)))
            .collect();
        let extra = form.has_notes().then(|| {
            cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 8)
                    .placeholder("Anything else Claude should know")
                    .context_menu(false)
            })
        });
        Self {
            form,
            state,
            answered,
            scale,
            on_send,
            texts,
            notes,
            extra,
        }
    }

    /// FORM-020: shows the form as answered (read-only), or open again.
    pub fn set_answered(&mut self, answered: Option<Value>, cx: &mut Context<'_, Self>) {
        if self.answered != answered {
            self.answered = answered;
            cx.notify();
        }
    }

    /// The scale it draws at.
    pub fn set_scale(&mut self, scale: f32, cx: &mut Context<'_, Self>) {
        if self.scale != scale {
            self.scale = scale;
            cx.notify();
        }
    }

    /// FORM-012: the answers as they stand, read from the fields.
    fn collect(&mut self, cx: &App) -> Value {
        for (i, field) in self.texts.iter().enumerate() {
            if let Some(f) = field {
                self.state.texts[i] = f.read(cx).value().to_string();
            }
        }
        for (i, field) in self.notes.iter().enumerate() {
            if let Some(f) = field {
                self.state.notes[i] = f.read(cx).value().to_string();
            }
        }
        if let Some(f) = &self.extra {
            self.state.extra = f.read(cx).value().to_string();
        }
        questions::answers_json(&self.form, &self.state)
    }

    /// FORM-012, FORM-020: sends the answers and goes read-only.
    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.answered.is_some() {
            return;
        }
        let answers = self.collect(cx);
        self.answered = Some(answers.clone());
        cx.notify();
        (self.on_send)(answers, window, cx);
    }

    fn toggle(&mut self, question: usize, choice: usize, cx: &mut Context<'_, Self>) {
        if self.answered.is_none() {
            self.state.toggle(&self.form, question, choice);
            cx.notify();
        }
    }
}

impl Render for FormView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let s = self.scale;
        let read_only = self.answered.is_some();
        // What to show: the answers sent, or what's picked so far.
        let shown = match &self.answered {
            Some(a) => FormState::from_answers(&self.form, a),
            None => self.state.clone(),
        };
        let questions: Vec<AnyElement> = self
            .form
            .questions
            .iter()
            .enumerate()
            .map(|(i, q)| self.question(i, q, &shown, &t, window, cx))
            .collect();
        let extra = self.extra.clone().map(|field| {
            let body = if read_only {
                answered_text(&shown.extra, s, &t)
            } else {
                text_box(field_area(&field, s), &field, s, &t, window, cx)
            };
            div()
                .flex()
                .flex_col()
                .gap(px(6. * s))
                .child(
                    div()
                        .text_size(px(12. * s))
                        .line_height(px(16. * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_secondary)
                        .child(questions::NOTES),
                )
                .child(body)
        });
        let extra = extra.filter(|_| !read_only || !shown.extra.trim().is_empty());
        let footer = (!read_only).then(|| {
            let label: SharedString = self.form.submit_label().to_owned().into();
            div().flex().justify_end().child(
                div()
                    .id("form-send")
                    .h(px(30. * s))
                    .px(px(14. * s))
                    .flex()
                    .items_center()
                    .gap(px(6. * s))
                    .rounded(px(8. * s))
                    .bg(t.accent)
                    .hover(|d| d.bg(t.accent_hover))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(13. * s))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_on_accent)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                    .child(label),
            )
        });
        div()
            .id("form")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(18. * s))
            .p(px(16. * s))
            .rounded(px(14. * s))
            .bg(t.bg_elevated)
            .border_1()
            .border_color(t.border)
            .font_family(fonts::UI)
            .text_color(t.text_primary)
            // FORM-030: typing stays in the form. Keys are marked, not
            // stopped, so the fields still get their characters, and the
            // board's shortcuts see the mark and stand aside.
            .on_key_down(|_, window, _| window.prevent_default())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(12. * s))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15. * s))
                            .line_height(px(20. * s))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.form.title.clone()),
                    )
                    .when(read_only, |d| d.child(answered_badge(s, &t))),
            )
            .children(questions)
            .children(extra)
            .children(footer)
    }
}

impl FormView {
    /// FORM-002…FORM-011: one question: its tag, title and detail, then
    /// its choices or its field, and its note.
    fn question(
        &self,
        i: usize,
        q: &Question,
        shown: &FormState,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let s = self.scale;
        let read_only = self.answered.is_some();
        // Rows of text never wrap as flex rows: in the chat's list they'd
        // shrink to their narrowest; the text takes the rest of the row.
        let head = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8. * s))
            .when_some(q.tag.clone().filter(|t| !t.is_empty()), |d, tag| {
                d.child(
                    div()
                        .flex_none()
                        .h(px(18. * s))
                        .px(px(6. * s))
                        .flex()
                        .items_center()
                        .rounded(px(5. * s))
                        .bg(t.bg_sunken)
                        .border_1()
                        .border_color(t.border)
                        .text_size(px(10.5 * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_secondary)
                        .child(tag),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14. * s))
                    .line_height(px(19. * s))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(q.title.clone()),
            )
            .when(q.multiple && !q.text, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.5 * s))
                        .text_color(t.text_tertiary)
                        .child("Pick any"),
                )
            });
        let detail = q.detail.clone().filter(|d| !d.trim().is_empty()).map(|detail| {
            div()
                .text_size(px(12.5 * s))
                .line_height(px(17. * s))
                .text_color(t.text_secondary)
                .child(detail)
        });
        let body: AnyElement = if q.text {
            if read_only {
                answered_text(&shown.texts[i], s, t)
            } else if let Some(field) = self.texts[i].clone() {
                text_box(field_area(&field, s), &field, s, t, window, cx)
            } else {
                div().into_any_element()
            }
        } else {
            let choices = q.shown_choices();
            let rows: Vec<AnyElement> = choices
                .iter()
                .enumerate()
                .map(|(c, choice)| {
                    let picked = shown.picked(i, c);
                    self.choice_row(i, c, choice, q.multiple, picked, t, cx)
                })
                .collect();
            let note = if read_only {
                let note = shown.notes[i].trim().to_owned();
                (!note.is_empty()).then(|| {
                    div()
                        .text_size(px(12.5 * s))
                        .line_height(px(17. * s))
                        .text_color(t.text_secondary)
                        .child(format!("Note: {note}"))
                        .into_any_element()
                })
            } else {
                self.notes[i].clone().map(|field| {
                    let input = Styled::h(
                        Input::new(&field)
                            .appearance(false)
                            .px(px(0.))
                            .py(px(0.))
                            .text_size(px(12.5 * s))
                            .line_height(px(28. * s)),
                        px(28. * s),
                    );
                    text_box(input.into_any_element(), &field, s, t, window, cx)
                })
            };
            div()
                .flex()
                .flex_col()
                .gap(px(6. * s))
                .children(rows)
                .children(note)
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8. * s))
            .child(head)
            .children(detail)
            .child(body)
            .into_any_element()
    }

    /// FORM-003…FORM-005: a choice: its letter, label, detail and
    /// "Recommended", highlighted when picked.
    #[allow(clippy::too_many_arguments)]
    fn choice_row(
        &self,
        q: usize,
        c: usize,
        choice: &questions::Choice,
        multiple: bool,
        picked: bool,
        t: &Tokens,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let s = self.scale;
        let read_only = self.answered.is_some();
        let badge = div()
            .flex_none()
            .mt(px(0.5 * s))
            .size(px(20. * s))
            .flex()
            .items_center()
            .justify_center()
            // One answer: round letters; several: square ones.
            .rounded(px(if multiple { 5. } else { 10. } * s))
            .when(picked, |d| d.bg(t.accent).text_color(t.text_on_accent))
            .when(!picked, |d| {
                d.bg(t.bg_sunken)
                    .border_1()
                    .border_color(t.border)
                    .text_color(t.text_secondary)
            })
            .text_size(px(11. * s))
            .font_weight(FontWeight::SEMIBOLD)
            .child(letter(c).to_string());
        let label = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(6. * s))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13. * s))
                    .line_height(px(19. * s))
                    .font_weight(FontWeight::MEDIUM)
                    .child(choice.label.clone()),
            )
            .when(choice.recommended, |d| {
                d.child(
                    div()
                        .flex_none()
                        .h(px(17. * s))
                        .px(px(6. * s))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(t.bg_selected)
                        .text_size(px(10.5 * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.folder_label_selected)
                        .child("Recommended"),
                )
            });
        let detail = choice.detail.clone().map(|d| {
            div()
                .text_size(px(12. * s))
                .line_height(px(16. * s))
                .text_color(t.text_secondary)
                .child(d)
        });
        div()
            .id(("form-choice", q * 32 + c))
            .flex()
            .items_start()
            .gap(px(10. * s))
            .px(px(10. * s))
            .py(px(7. * s))
            .rounded(px(9. * s))
            .border_1()
            .border_color(if picked { t.accent } else { t.border })
            .when(picked, |d| d.bg(t.bg_selected))
            // Answered, the picks stand out and the rest step back.
            .when(read_only && !picked, |d| d.opacity(0.5))
            .when(!read_only, |d| {
                d.cursor(CursorStyle::PointingHand)
                    .when(!picked, |d| d.hover(|h| h.bg(t.bg_hover)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(q, c, cx)))
            })
            .child(badge)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1. * s))
                    .child(label)
                    .children(detail),
            )
            .into_any_element()
    }
}

/// A multi-line field, auto-growing. XSmall's editor keeps 4px clear on
/// the left and 14px on the right: taken back, so its text lines up with
/// the single-line notes and wraps at the frame's padding.
fn field_area(field: &Entity<TextareaState>, s: f32) -> AnyElement {
    div()
        .ml(px(-4.))
        .mr(px(-14.))
        .child(
            Textarea::new(field)
                .appearance(false)
                .xsmall()
                .px(px(0.))
                .py(px(6. * s))
                .text_size(px(13. * s))
                .line_height(px(19. * s)),
        )
        .into_any_element()
}

/// A field in its frame: a hairline box, the accent once focused.
fn text_box<F: gpui::Focusable>(
    input: AnyElement,
    field: &Entity<F>,
    s: f32,
    t: &Tokens,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let focused = field.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w_full()
        .px(px(10. * s))
        .rounded(px(8. * s))
        .border_1()
        .border_color(if focused { t.accent } else { t.border })
        .bg(t.bg_elevated)
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(input)
        .into_any_element()
}

/// Words that were sent, in a sunken box; "Skipped" when there were none.
fn answered_text(text: &str, s: f32, t: &Tokens) -> AnyElement {
    let text = text.trim();
    div()
        .w_full()
        .px(px(10. * s))
        .py(px(7. * s))
        .rounded(px(8. * s))
        .bg(t.bg_sunken)
        .text_size(px(13. * s))
        .line_height(px(19. * s))
        .when(text.is_empty(), |d| d.text_color(t.text_tertiary).child("Skipped"))
        .when(!text.is_empty(), |d| {
            d.text_color(t.text_primary).child(text.to_owned())
        })
        .into_any_element()
}

/// FORM-020: "Answered", with a check.
fn answered_badge(s: f32, t: &Tokens) -> AnyElement {
    div()
        .flex_none()
        .h(px(22. * s))
        .px(px(8. * s))
        .flex()
        .items_center()
        .gap(px(5. * s))
        .rounded_full()
        .bg(t.bg_sunken)
        .text_size(px(11.5 * s))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_secondary)
        .child(icon(Icon::Check, 12. * s, 2.4, t.diff_add))
        .child("Answered")
        .into_any_element()
}
