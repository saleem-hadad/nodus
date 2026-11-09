//! The field's keys (BAR-009, BAR-015, BAR-016, BAR-021, BAR-023), seen
//! before the field's own editing; nothing while an input method composes.

use gpui::{Context, EntityInputHandler as _, KeyDownEvent, Window};

use crate::board::canvas::BoardCanvas;

impl BoardCanvas {
    /// The bar's field's key handler, in the capture phase. True when the
    /// key was the bar's (the field doesn't see it).
    pub(crate) fn bar_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some(field) = self.overlays.bar.field.clone() else {
            return false;
        };
        if field.update(cx, |s, cx| s.marked_text_range(window, cx)).is_some() {
            return false;
        }
        let k = &e.keystroke;
        let m = k.modifiers;
        let key = k.key.as_str();
        let commands = self.bar_commands();
        let menu_open = !commands.is_empty();
        let highlighted = self.overlays.bar.active.min(commands.len().saturating_sub(1));
        if menu_open && (key == "down" || key == "up") {
            // BAR-015: both wrap.
            let step = if key == "down" { 1 } else { commands.len() - 1 };
            self.overlays.bar.active = (highlighted + step) % commands.len();
            cx.notify();
            return true;
        }
        if menu_open && (key == "enter" || key == "tab") {
            self.bar_pick(commands[highlighted].target, window, cx);
            return true;
        }
        if menu_open && key == "escape" {
            // BAR-016: closed, the field keeps the keyboard.
            self.overlays.bar.menu_dismissed = true;
            cx.notify();
            return true;
        }
        if key == "enter" && !m.shift && !m.platform && !m.control && !m.alt {
            self.bar_submit(cx);
            return true;
        }
        // BAR-021: Backspace in an empty field takes the token off.
        if key == "backspace" && self.overlays.bar.text.is_empty() {
            if self.overlays.bar.target.is_some() {
                self.bar_clear_target(cx);
                return true;
            }
            if self.bar_sends_into().is_some() {
                self.bar_drop_into(cx);
                return true;
            }
            return false;
        }
        // BAR-023: Esc closes the picker, else lets go of the keyboard.
        if key == "escape" {
            if self.overlays.bar.picking {
                self.overlays.bar.picking = false;
                cx.notify();
            } else {
                window.focus(&self.focus, cx);
            }
            return true;
        }
        false
    }
}
