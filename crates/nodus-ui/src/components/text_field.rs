//! Text fields: gpui-component's editing engine (selection, IME, undo),
//! drawn without its own chrome inside the frame each place styles itself
//! (rename fields, the group name, the xAI key). Text in them selects and
//! edits normally (UI-007); no native right-click menu (PLAT-138).

use gpui::component::input::{Input, InputState};
use gpui::{App, AppContext as _, Entity, IntoElement, Pixels, Styled, Window, px};

/// A field holding `value`, showing `placeholder` while empty; `masked`
/// hides what's typed (a password field).
pub fn new_input(
    value: &str,
    placeholder: &str,
    masked: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = value.to_owned();
    let placeholder = placeholder.to_owned();
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder).context_menu(false);
        if masked {
            state = state.masked(true);
        }
        state.set_value(value, window, cx);
        state
    })
}

/// The field's text, laid out in its frame: no border, no fill, no padding,
/// `height` tall.
pub fn input(state: &Entity<InputState>, height: Pixels, centered: bool) -> impl IntoElement {
    let field = Input::new(state)
        .appearance(false)
        .px(px(0.))
        .py(px(0.))
        .h(height)
        .line_height(height);
    if centered { field.text_center() } else { field }
}

/// Focuses the field with all its text selected (UI-237).
pub fn focus_all(state: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.focus(window, cx);
        s.select_all(window, cx);
    });
}
