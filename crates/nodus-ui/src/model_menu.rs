//! Choosing a Claude session's model and how hard it thinks (spec 16,
//! MODEL): the chip that names them and the menu it opens, which the chat's
//! composer and the board's prompt bar share, and switching a session.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, SharedString, Stateful, StatefulInteractiveElement, Styled, Task, Window, div,
    px,
};
use nodus_domain::{ModelSwitch, SessionCard, SessionEffort, SessionModel};

use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::state::AppState;
use crate::theme::tokens::Tokens;

/// Told the model picked; None is Claude Code's own.
pub type OnModel = Rc<dyn Fn(Option<SessionModel>, &mut Window, &mut App)>;
/// Told the effort picked; None is the model's own.
pub type OnEffort = Rc<dyn Fn(Option<SessionEffort>, &mut Window, &mut App)>;

/// The model and effort a chip names and its menu ticks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelChoice {
    /// None is Claude Code's default.
    pub model: Option<SessionModel>,
    /// None is the model's default.
    pub effort: Option<SessionEffort>,
    /// The model running, as Claude Code names it (`Opus 5.5`), when known.
    pub running: Option<String>,
}

impl ModelChoice {
    /// MODEL-030: the chip's words, `Opus 5.5 · High`, and `Default` for
    /// Claude Code's own model.
    pub fn label(&self) -> String {
        let model = self
            .running
            .clone()
            .or_else(|| self.model.map(|m| m.label().to_owned()))
            .unwrap_or_else(|| "Default".to_owned());
        match self.effort.filter(|_| self.takes_effort()) {
            Some(effort) => format!("{model} · {}", effort.label()),
            None => model,
        }
    }

    /// Whether its model thinks with an effort level; Claude Code's default may.
    pub fn takes_effort(&self) -> bool {
        self.model.is_none_or(SessionModel::takes_effort)
    }
}

/// MODEL-031: what a session runs on: the model and effort the card was
/// started on or switched to, and what its status line reports while it
/// runs. A running model of another family than the card's is one a switch
/// hasn't reached yet (MODEL-011), so the card's is named.
pub fn choice_of(card: &SessionCard) -> ModelChoice {
    let usage = card.usage.as_ref().filter(|_| card.live);
    let running = usage.and_then(|u| u.model.clone());
    let family = running.as_deref().and_then(SessionModel::of_name);
    let model = card.node.model().and_then(SessionModel::parse);
    ModelChoice {
        model: model.or(family),
        effort: card
            .node
            .effort()
            .and_then(SessionEffort::parse)
            .or_else(|| usage.and_then(|u| u.effort.as_deref()).and_then(SessionEffort::parse)),
        running: running.filter(|_| model.is_none() || model == family),
    }
}

/// MODEL-031: a running Claude still on another model or effort than its
/// card's, which it's started again on when its turn ends (MODEL-011).
pub fn pending(card: &SessionCard) -> bool {
    let Some(usage) = card.usage.as_ref().filter(|_| card.live) else {
        return false;
    };
    let model = card.node.model().and_then(SessionModel::parse);
    let effort = card.node.effort().and_then(SessionEffort::parse);
    let running = usage.model.as_deref().and_then(SessionModel::of_name);
    let thinking = usage.effort.as_deref().and_then(SessionEffort::parse);
    (model.is_some() && running.is_some() && model != running)
        || (effort.is_some() && thinking.is_some() && effort != thinking)
}

/// MODEL-032: switches a session's model or effort: on its card at once,
/// then in the core, which saves it and starts a running Claude again on it.
/// Gives back why it couldn't, the card as it was.
pub fn switch(
    state: &Entity<AppState>,
    project_id: &str,
    session_id: &str,
    switch: ModelSwitch,
    cx: &mut App,
) -> Task<Result<(), String>> {
    let set = |card: &mut SessionCard, switch: ModelSwitch| match switch {
        ModelSwitch::Model(model) => card.node.set_model(model),
        ModelSwitch::Effort(effort) => card.node.set_effort(effort),
    };
    let before = crate::terminal::actions::card(state.read(cx), session_id).map(|card| match switch {
        ModelSwitch::Model(_) => ModelSwitch::Model(card.node.model().and_then(SessionModel::parse)),
        ModelSwitch::Effort(_) => ModelSwitch::Effort(card.node.effort().and_then(SessionEffort::parse)),
    });
    let put = move |state: &Entity<AppState>, session_id: &str, switch: ModelSwitch, cx: &mut App| {
        state.update(cx, |s, cx| {
            if let Some(mut card) = crate::terminal::actions::card(s, session_id) {
                set(&mut card, switch);
                s.upsert_card(card, cx);
            }
        });
    };
    put(state, session_id, switch, cx);
    let nodus = state.read(cx).nodus.clone();
    let (state, project_id, session_id) = (state.clone(), project_id.to_owned(), session_id.to_owned());
    cx.spawn(async move |cx| {
        let result = nodus
            .sessions()
            .set_model(&project_id, &session_id, switch)
            .await
            .map_err(|err| err.ipc_message());
        if result.is_err()
            && let Some(before) = before
        {
            cx.update(|cx| put(&state, &session_id, before, cx));
        }
        result
    })
}

/// MODEL-030: the chip, `scale` times its size: the model and effort with a
/// chevron, quiet until hovered or open.
pub fn chip(
    t: &Tokens,
    id: impl Into<ElementId>,
    choice: &ModelChoice,
    open: bool,
    scale: f32,
    tooltip: impl Into<SharedString>,
) -> Stateful<Div> {
    let s = |v: f32| px(v * scale);
    let fg = if open { t.text_primary } else { t.text_secondary };
    div()
        .id(id.into())
        .flex_none()
        .flex()
        .items_center()
        .gap(s(3.))
        .h(s(22.))
        .pl(s(7.))
        .pr(s(5.))
        .rounded(s(7.))
        .cursor(CursorStyle::Arrow)
        .text_size(s(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .whitespace_nowrap()
        .when(open, |d| d.bg(t.bg_hover))
        .hover(|d| d.bg(t.bg_hover).text_color(t.text_primary))
        .tooltip(native_tooltip(tooltip.into()))
        .child(SharedString::from(choice.label()))
        .child(icon(Icon::ChevronDown, 11. * scale, 2.2, fg))
}

/// MODEL-033: the menu's body, `scale` times its size: the models, then the
/// effort levels. The caller puts it on a surface and closes it.
pub fn menu(
    t: &Tokens,
    id: &'static str,
    choice: &ModelChoice,
    scale: f32,
    on_model: &OnModel,
    on_effort: &OnEffort,
) -> AnyElement {
    let s = |v: f32| px(v * scale);
    let header = |text: &'static str| {
        div()
            .px(s(8.))
            .pt(s(4.))
            .pb(s(3.))
            .text_size(s(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text_tertiary)
            .child(text)
    };
    let models = std::iter::once(None)
        .chain(SessionModel::ALL.iter().copied().map(Some))
        .enumerate()
        .map(|(i, model)| {
            let on_model = on_model.clone();
            let picked = choice.model == model;
            let (name, blurb) = match model {
                Some(model) => (model.label(), model.blurb()),
                None => ("Default", "Claude Code’s own setting"),
            };
            div()
                .id((id, i))
                .flex()
                .items_center()
                .gap(s(10.))
                .h(s(30.))
                .px(s(8.))
                .rounded(s(8.))
                .cursor(CursorStyle::Arrow)
                .hover(|d| d.bg(t.bg_hover))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, window, cx| on_model(model, window, cx))
                .child(
                    div()
                        .flex_none()
                        .w(s(58.))
                        .text_size(s(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text_primary)
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(s(12.))
                        .text_color(t.text_tertiary)
                        .child(blurb),
                )
                .child(
                    div()
                        .flex_none()
                        .w(s(14.))
                        .when(picked, |d| d.child(icon(Icon::Check, 14. * scale, 2.4, t.text_primary))),
                )
        });
    let takes = choice.takes_effort();
    let levels = std::iter::once(None)
        .chain(SessionEffort::ALL.iter().copied().map(Some))
        .enumerate()
        .map(|(i, effort)| {
            let on_effort = on_effort.clone();
            let picked = takes && choice.effort == effort;
            let (name, tip) = match effort {
                Some(effort) => (effort.short(), effort.label()),
                None => ("Auto", "The model’s own default"),
            };
            div()
                .id((id, 100 + i))
                .flex_1()
                .h(s(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(s(6.))
                .text_size(s(11.5))
                .cursor(CursorStyle::Arrow)
                .map(|d| {
                    if picked {
                        d.bg(t.bg_elevated)
                            .shadow(t.sm())
                            .text_color(t.text_primary)
                            .font_weight(FontWeight::MEDIUM)
                    } else {
                        d.text_color(t.text_secondary)
                    }
                })
                .when(takes, |d| {
                    d.hover(|d| d.text_color(t.text_primary))
                        .tooltip(native_tooltip(tip.into()))
                        .on_click(move |_, window, cx| on_effort(effort, window, cx))
                })
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .child(name)
        });
    div()
        .w(s(296.))
        .p(s(6.))
        .flex()
        .flex_col()
        .child(header("Model"))
        .children(models)
        .child(div().h(px(1.)).mx(s(8.)).my(s(6.)).bg(t.separator))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pr(s(8.))
                .child(header("Effort"))
                .when(!takes, |d| {
                    d.child(
                        div()
                            .text_size(s(11.))
                            .text_color(t.text_tertiary)
                            .child("Haiku doesn’t take one"),
                    )
                }),
        )
        .child(
            div()
                .mx(s(4.))
                .mb(s(4.))
                .mt(s(2.))
                .p(s(2.))
                .flex()
                .gap(s(2.))
                .rounded(s(8.))
                .bg(t.bg_sunken)
                .when(!takes, |d| d.opacity(0.45))
                .children(levels),
        )
        .into_any_element()
}

/// Keeps where something was drawn, for a press outside the menu that
/// should leave it alone (its chip's own, which toggles it).
pub fn probe<T: 'static>(entity: gpui::WeakEntity<T>, put: fn(&mut T, Bounds<Pixels>)) -> AnyElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, _, cx| {
            if let Some(e) = entity.upgrade() {
                e.update(cx, |e, _| put(e, bounds));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_030_the_chip_names_the_model_and_its_effort() {
        let choice = |model, effort, running: Option<&str>| ModelChoice {
            model,
            effort,
            running: running.map(str::to_owned),
        };
        assert_eq!(choice(None, None, None).label(), "Default");
        assert_eq!(
            choice(Some(SessionModel::Opus), Some(SessionEffort::XHigh), None).label(),
            "Opus · Extra high"
        );
        assert_eq!(
            choice(Some(SessionModel::Opus), Some(SessionEffort::High), Some("Opus 5.5")).label(),
            "Opus 5.5 · High"
        );
        // Haiku thinks without an effort level.
        assert_eq!(
            choice(Some(SessionModel::Haiku), Some(SessionEffort::High), Some("Haiku 4.5")).label(),
            "Haiku 4.5"
        );
        assert_eq!(choice(None, Some(SessionEffort::Max), None).label(), "Default · Max");
    }
}
