//! Claude's charts and forms in the chat (CHAT-026, CHAT-027).

use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window, div};
use serde_json::Value;

use super::view::ChatView;
use crate::theme::tokens::Tokens;

impl ChatView {
    pub(crate) fn render_visual(
        &mut self,
        _id: &str,
        spec: &Value,
        t: &Tokens,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let title = spec.get("title").and_then(Value::as_str).unwrap_or("A chart");
        div()
            .p(self.px(12.))
            .rounded(self.px(12.))
            .border_1()
            .border_color(t.border)
            .child(SharedString::from(title.to_owned()))
            .into_any_element()
    }

    pub(crate) fn render_questions(
        &mut self,
        _id: &str,
        spec: &Value,
        _answered: Option<&Value>,
        t: &Tokens,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let title = spec.get("title").and_then(Value::as_str).unwrap_or("Questions");
        div()
            .p(self.px(12.))
            .rounded(self.px(12.))
            .border_1()
            .border_color(t.border)
            .child(SharedString::from(title.to_owned()))
            .into_any_element()
    }
}
