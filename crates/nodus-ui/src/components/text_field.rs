//! Text fields: gpui-component's editing engine (selection, IME, undo),
//! drawn without its own chrome inside the frame each place styles itself
//! (rename fields, the group name, the xAI key). Text in them selects and
//! edits normally (UI-007); no native right-click menu (PLAT-138).

use gpui::component::input::{Input, InputState, Paste};
use gpui::{App, AppContext as _, Entity, Focusable as _, IntoElement, Pixels, Styled, Window, px};

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
        .line_height(height);
    // `Input::h` sizes a multi-line field; a single line takes the style's.
    let field = Styled::h(field, height);
    if centered { field.text_center() } else { field }
}

/// Focuses the field with all its text selected (UI-237), once it has been
/// drawn: a focus handle no frame has drawn yet doesn't keep the focus.
pub fn focus_all(state: &Entity<InputState>, window: &mut Window, _: &mut App) {
    let state = state.clone();
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| {
            state.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
        });
    });
}

/// What a paste puts in a field with HTML's `maxLength` (`max` UTF-16
/// units): the clipboard text without line breaks, cut to what fits beside
/// the text that stays (`len` units, `selected` of them replaced), never
/// splitting a surrogate pair, as Blink's `LimitLength` cuts it. Empty when
/// nothing fits.
pub fn fit_max_length(text: &str, len: usize, selected: usize, max: usize) -> String {
    let room = max.saturating_sub(len.saturating_sub(selected));
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars().filter(|c| !matches!(c, '\n' | '\r')) {
        used += c.len_utf16();
        if used > room {
            break;
        }
        out.push(c);
    }
    out
}

/// A `Paste` handler for a field with HTML's `maxLength`: the clipboard's
/// text is cut to fit ([`fit_max_length`]) where gpui-component's
/// validation would refuse the whole paste. Hang it with `capture_action`
/// on an element around the field.
pub fn paste_within(state: &Entity<InputState>, max: usize) -> impl Fn(&Paste, &mut Window, &mut App) + 'static {
    let state = state.clone();
    move |_, window, cx| {
        if !state.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        cx.stop_propagation();
        state.update(cx, |s, cx| {
            let value = s.value();
            let range = s.selected_range();
            let len = nodus_domain::js::utf16_len(&value);
            let selected = value.get(range).map_or(0, nodus_domain::js::utf16_len);
            let fitted = fit_max_length(&text, len, selected, max);
            if !fitted.is_empty() || selected > 0 {
                s.replace(fitted, window, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::fit_max_length;

    #[test]
    fn cuts_a_paste_to_what_fits_as_max_length_does() {
        let long = "Deploy the storefront to staging and run the smoke tests";
        assert_eq!(fit_max_length(long, 0, 0, 40), &long[..40]);
        // Beside text that stays, only the rest fits.
        assert_eq!(fit_max_length("abcdef", 36, 0, 40), "abcd");
        // A selection's units are freed.
        assert_eq!(fit_max_length("abcdef", 40, 3, 40), "abc");
        // Already full: nothing goes in.
        assert_eq!(fit_max_length("abc", 40, 0, 40), "");
        // Line breaks go, as a single-line field drops them.
        assert_eq!(fit_max_length("a\r\nb", 0, 0, 40), "ab");
        // A surrogate pair isn't split: the emoji is two units.
        assert_eq!(fit_max_length("ab\u{1F600}c", 37, 0, 40), "ab");
        assert_eq!(fit_max_length("ab\u{1F600}c", 36, 0, 40), "ab\u{1F600}");
    }
}
