//! Claude's charts and forms in the chat (CHAT-009, CHAT-010, CHAT-027):
//! each spec read once and kept by its entry's id, a form as an entity of
//! its own so what the user picked survives the list's re-renders.

use std::rc::Rc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div,
};
use nodus_visual::{Form, Visual};
use serde_json::Value;

use super::form::{FormView, OnSend};
use super::view::ChatView;
use super::visual::visual_element;
use crate::theme::tokens::Tokens;

/// What the chat keeps of Claude's visuals and forms, by entry id.
#[derive(Default)]
pub(crate) struct Shows {
    visuals: std::collections::HashMap<String, Rc<Result<Rc<Visual>, String>>>,
    forms: std::collections::HashMap<String, Result<Entity<FormView>, String>>,
}

impl ChatView {
    pub(crate) fn render_visual(
        &mut self,
        id: &str,
        spec: &Value,
        t: &Tokens,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let read = self
            .shows
            .visuals
            .entry(id.to_owned())
            .or_insert_with(|| Rc::new(nodus_visual::parse_visual(spec).map(Rc::new)))
            .clone();
        match &*read {
            Ok(visual) => div()
                .w_full()
                .child(visual_element(
                    SharedString::from(format!("chat-visual-{id}")),
                    Rc::clone(visual),
                    self.scale,
                    cx,
                ))
                .into_any_element(),
            Err(err) => self.cant_show("This chart can’t be shown", err, t),
        }
    }

    pub(crate) fn render_questions(
        &mut self,
        id: &str,
        spec: &Value,
        answered: Option<&Value>,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let scale = self.scale;
        if !self.shows.forms.contains_key(id) {
            let made = nodus_visual::parse_form(spec).map(|form| self.new_form(id, form, answered, scale, window, cx));
            self.shows.forms.insert(id.to_owned(), made);
        }
        match self.shows.forms.get(id).cloned() {
            Some(Ok(form)) => {
                // The form locks itself on Send; the transcript only ever
                // confirms it (CHAT-007), it never opens one again.
                let answered = answered.cloned();
                form.update(cx, |f, cx| {
                    if answered.is_some() {
                        f.set_answered(answered, cx);
                    }
                    f.set_scale(scale, cx);
                });
                div().w_full().child(form).into_any_element()
            }
            Some(Err(err)) => self.cant_show("These questions can’t be shown", &err, t),
            None => div().into_any_element(),
        }
    }

    /// CHAT-027: a form whose Send goes to Claude as an answers envelope.
    fn new_form(
        &mut self,
        id: &str,
        form: Form,
        answered: Option<&Value>,
        scale: f32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<FormView> {
        let me = cx.entity().downgrade();
        let (to, title) = (id.to_owned(), form.title.clone());
        let on_send: OnSend = Rc::new(move |answers, _, cx| {
            let _ = me.update(cx, |this, cx| this.send_answers(&to, &title, answers, cx));
        });
        let answered = answered.cloned();
        cx.new(|cx| FormView::new(form, answered, scale, on_send, window, cx))
    }

    fn cant_show(&self, what: &'static str, why: &str, t: &Tokens) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(self.px(2.))
            .p(self.px(10.))
            .rounded(self.px(10.))
            .border_1()
            .border_color(t.border)
            .text_size(self.px(12.))
            .child(div().text_color(t.text_secondary).child(what))
            .child(
                div()
                    .text_color(t.text_tertiary)
                    .child(SharedString::from(why.to_owned())),
            )
            .into_any_element()
    }
}
