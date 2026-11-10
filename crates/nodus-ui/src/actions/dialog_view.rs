//! Drawing the action dialog (`.action-*` in `actions.css`; ACT-068…ACT-080).

use gpui::component::Sizable as _;
use gpui::component::input::{Input, Textarea};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, Focusable as _, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, linear_color_stop, linear_gradient, px,
};
use nodus_actions::dialog::{EXAMPLES, Step};
use nodus_domain::ActionIcon;

use super::dialog::{ActionDialogView, NAME_MAX};
use super::icons::{Glyph, glyph};
use crate::components::modal::Modal;
use crate::components::native_tooltip;
use crate::components::spellcheck;
use crate::components::switch::switch;
use crate::components::{Kind, button, child_id};
use crate::theme::tokens::*;
use crate::theme::{self, Tokens};

/// The description's line height (`line-height: 1.45` at 13px).
const DESCRIPTION_LH: f32 = 13. * 1.45;
/// The prompt's line height (`line-height: 1.55` at 12px).
const PROMPT_LH: f32 = 12. * 1.55;
/// What gpui-component's text fields leave unused at the right of a line.
const WRAP_MARGIN: f32 = 10.;

/// `.action-input`: the frame every field shares; focused, an accent border
/// and a 3px `--focus-ring` around it; disabled, at 60%.
fn field_frame(t: &Tokens, focused: bool, disabled: bool) -> Div {
    div()
        .relative()
        .w_full()
        .border(px(0.5))
        .border_color(if focused { t.accent } else { t.border_strong })
        .rounded(px(8.))
        .bg(t.bg_window)
        .when(focused, |d| d.shadow(vec![ring(3., t.focus_ring)]))
        .when(disabled, |d| d.opacity(0.6))
}

/// `.action-label`.
fn label(t: &Tokens, text: &'static str) -> Div {
    div()
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_secondary)
        .child(text)
}

impl ActionDialogView {
    /// ACT-068: the gradient tile, the title and what this step is for.
    fn header(&self, t: &Tokens) -> Div {
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .flex_none()
            .pt(px(20.))
            .px(px(22.))
            .pb(px(16.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .size(px(32.))
                    .rounded(px(9.))
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(t.accent, 0.),
                        linear_color_stop(hex(0x5e5ce6), 1.),
                    ))
                    .child(glyph(Glyph::Sparkles, 16., 2.2, hard::WHITE)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(TEXT_LG)
                            .line_height(px(21.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.model.title()),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(TEXT_SM)
                            .line_height(px(16.8))
                            .text_color(t.text_secondary)
                            .child(self.model.subtitle()),
                    ),
            )
    }

    /// ACT-069, ACT-070: the description, the example chips and the scope.
    fn describe(&self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let drafting = self.model.drafting;
        let focused = !drafting && self.description.read(cx).focus_handle(cx).is_focused(window);
        // `rows="5"`: five lines and the padding (10px 12px, Size::Large's).
        let textarea = Textarea::new(&self.description)
            .appearance(false)
            .with_size(gpui::component::Size::Large)
            .disabled(drafting)
            .text_size(TEXT_MD)
            .line_height(px(DESCRIPTION_LH))
            .h(px(5. * DESCRIPTION_LH + 20.))
            // The field wraps 10px short of its width (gpui-component's
            // RIGHT_MARGIN); Chromium wraps at the padding.
            .mr(px(-WRAP_MARGIN));
        let field = field_frame(t, focused, drafting)
            .child(textarea)
            // PLAT-137: Chromium's spellcheck underlines.
            .child(spellcheck::underlines(&self.spell[0], &self.description, TEXT_MD, 1.))
            // `resize: vertical`: Chromium's grip in the corner.
            .child(div().absolute().right(px(0.5)).bottom(px(0.5)).child(glyph(
                Glyph::ResizeGrip,
                10.,
                2.4,
                t.text_secondary,
            )));
        let chips: Vec<AnyElement> = EXAMPLES
            .iter()
            .enumerate()
            .map(|(i, (name, text))| self.chip(i, name, text, t, window, cx))
            .collect();
        let examples = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(label(t, "Start from an example"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(chips));
        vec![
            field.into_any_element(),
            examples.into_any_element(),
            self.scope(t, cx).into_any_element(),
        ]
    }

    /// ACT-070: an example chip, highlighted while the description is its text.
    fn chip(
        &self,
        index: usize,
        name: &'static str,
        text: &'static str,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let active = self.model.chip_selected(index);
        let id = SharedString::from(format!("act-chip-{index}"));
        let hovered = window.use_keyed_state(child_id(id.clone(), "hover"), cx, |_, _| false);
        let hover = *hovered.read(cx);
        let bg = if active {
            alpha(t.accent, 0.12)
        } else if hover {
            t.bg_hover.into()
        } else {
            t.bg_elevated.into()
        };
        let view = cx.entity();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .h(px(26.))
            .px(px(11.))
            .border(px(0.5))
            .border_color(if active { t.accent } else { t.border_strong })
            .rounded(px(13.))
            .bg(bg)
            .text_size(TEXT_SM)
            .line_height(px(16.8))
            .text_color(if active { t.accent } else { t.text_primary })
            .whitespace_nowrap()
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .tooltip(native_tooltip(text.into()))
            .on_click(move |_, window, cx| view.update(cx, |v, cx| v.pick_chip(index, window, cx)))
            .child(name)
            .into_any_element()
    }

    /// ACT-071: `Use in every project`, its hint and the switch.
    fn scope(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> Div {
        let view = cx.entity();
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .flex_none()
            .pt(px(12.))
            .border_t(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(TEXT_MD)
                            .line_height(px(18.2))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Use in every project"),
                    )
                    .child(
                        div()
                            .text_size(TEXT_SM)
                            .line_height(px(16.8))
                            .text_color(t.text_secondary)
                            .child(self.model.scope_hint()),
                    ),
            )
            .child(
                switch("act-scope", self.model.global)
                    .disabled(!self.model.scope_enabled())
                    .on_change(move |on, _, cx| view.update(cx, |v, cx| v.set_global(on, cx))),
            )
    }
}

impl ActionDialogView {
    /// ACT-074: the name, the 26 icons, the prompt and the scope.
    fn review(&self, t: &Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let name_focused = self.name.read(cx).focus_handle(cx).is_focused(window);
        let name = div().flex().flex_col().gap(px(6.)).child(label(t, "Name")).child(
            field_frame(t, name_focused, false)
                // `maxLength`: a long paste is cut to fit, not refused.
                .capture_action(crate::components::text_field::paste_within(&self.name, NAME_MAX))
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .text_size(TEXT_MD)
                .child(
                    Input::new(&self.name)
                        .appearance(false)
                        .px(px(0.))
                        .py(px(0.))
                        .text_size(TEXT_MD)
                        .line_height(px(31.))
                        .h(px(31.)),
                )
                .child(spellcheck::underlines(&self.spell[1], &self.name, TEXT_MD, 1.)),
        );
        let selected = self.model.selected_icon();
        let icons: Vec<AnyElement> = ActionIcon::ALL
            .iter()
            .map(|icon| self.icon_option(*icon, selected == Some(*icon), t, window, cx))
            .collect();
        let icon = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(t, "Icon"))
            .child(div().flex().flex_wrap().gap(px(4.)).children(icons));
        let prompt_focused = self.prompt.read(cx).focus_handle(cx).is_focused(window);
        // `.action-prompt`: 12px 14px around the text; Size::Large gives the
        // field 10px 12px, the frame the rest.
        let prompt = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .min_h(px(180.))
            .child(label(t, "Prompt"))
            .child(
                field_frame(t, prompt_focused, false)
                    .flex_1()
                    .h(px(160.))
                    .p(px(2.))
                    .font_family(theme::fonts::mono())
                    .child(
                        Textarea::new(&self.prompt)
                            .appearance(false)
                            .with_size(gpui::component::Size::Large)
                            .font_family(theme::fonts::mono())
                            .text_size(TEXT_SM)
                            .line_height(px(PROMPT_LH))
                            .h(px(160. - 1. - 4.))
                            .mr(px(-WRAP_MARGIN)),
                    )
                    .child(spellcheck::underlines(&self.spell[2], &self.prompt, TEXT_SM, 1.)),
            );
        vec![
            name.into_any_element(),
            icon.into_any_element(),
            prompt.into_any_element(),
            self.scope(t, cx).into_any_element(),
        ]
    }

    /// ACT-074: one icon to pick; its name is its tooltip.
    fn icon_option(
        &self,
        icon: ActionIcon,
        selected: bool,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let id = SharedString::from(format!("act-icon-{}", icon.as_str()));
        let hovered = window.use_keyed_state(child_id(id.clone(), "hover"), cx, |_, _| false);
        let hover = *hovered.read(cx);
        let (bg, color) = if selected {
            (Some(t.accent), t.text_on_accent)
        } else if hover {
            (Some(t.bg_hover), t.text_primary)
        } else {
            (None, t.text_secondary)
        };
        let view = cx.entity();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(30.))
            .rounded(px(7.))
            .when_some(bg, Styled::bg)
            .on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .tooltip(native_tooltip(icon.as_str().into()))
            .on_click(move |_, _, cx| view.update(cx, |v, cx| v.pick_icon(icon.as_str(), cx)))
            .child(glyph(Glyph::action(icon.as_str()), 15., 2., color))
            .into_any_element()
    }

    /// ACT-076, ACT-077: back on the left; Cancel and the step's button on the right.
    fn footer(&self, t: &Tokens, cx: &mut Context<'_, Self>) -> Div {
        let back = self.model.back_button().map(|b| {
            let view = cx.entity();
            button("act-back")
                .ghost()
                .label(b.label)
                .disabled(!b.enabled)
                .on_click(move |_, _, cx| view.update(cx, ActionDialogView::back))
        });
        let cancel = {
            let view = cx.entity();
            button("act-cancel")
                .label("Cancel")
                .on_click(move |_, window, cx| view.update(cx, |v, cx| v.close(window, cx)))
        };
        let primary = self.model.primary_button();
        let view = cx.entity();
        let mut primary_button = button("act-primary").kind(Kind::Primary).disabled(!primary.enabled);
        match self.model.step {
            Step::Describe if self.model.drafting => {
                primary_button =
                    primary_button.child(crate::voice::spinner("act-drafting", 14., 2., t.text_on_accent, cx));
            }
            Step::Describe => {
                primary_button = primary_button.child(glyph(Glyph::Sparkles, 14., 2.2, t.text_on_accent));
            }
            Step::Review => {}
        }
        let primary_button = primary_button
            .child(primary.label)
            .on_click(move |_, window, cx| view.update(cx, |v, cx| v.submit(window, cx)));
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .px(px(22.))
            .py(px(12.))
            .border_t(px(0.5))
            .border_color(t.separator)
            .children(back)
            .child(div().flex_1())
            .child(cancel)
            .child(primary_button)
    }
}

impl Render for ActionDialogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.autofocus(window, cx);
        let t = theme::tokens(cx);
        let viewport = window.viewport_size();
        let body = match self.model.step {
            Step::Describe => self.describe(&t, window, cx),
            Step::Review => self.review(&t, window, cx),
        };
        // ACT-080: between the body and the footer until the next draft or save.
        let error = self.model.error.clone().map(|e| {
            div()
                .mx(px(22.))
                .mb(px(12.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(alpha(t.status_waiting, 0.10))
                .text_color(t.text_primary)
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                // `.action-error.selectable`.
                .child(crate::components::selectable::selectable("action-error", e, window, cx))
        });
        let content = div()
            .id("act-dialog")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .min_h_0()
            // `.action-dialog { max-height: min(86vh, 760px) }`.
            .max_h((viewport.height * 0.86).min(px(760.)))
            .text_color(t.text_primary)
            .child(self.header(&t))
            .child(
                div()
                    .id("act-body")
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .flex_1()
                    .min_h_0()
                    .px(px(22.))
                    .py(px(16.))
                    .overflow_y_scroll()
                    .children(body),
            )
            .children(error)
            .child(self.footer(&t, cx));
        let view = cx.entity();
        Modal::new("act-dialog-modal", px(620.), move |window, cx| {
            view.update(cx, |v, cx| v.close(window, cx));
        })
        .render(content, window, cx)
    }
}
