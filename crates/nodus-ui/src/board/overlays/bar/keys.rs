//! The field's keys (BAR-009, BAR-015, BAR-016, BAR-021, BAR-023), seen
//! before the field's own editing; nothing while an input method composes.
//! ↑, ↓ and Tab are the field's own bindings, so the menu takes them as
//! actions ([`BoardCanvas::bar_menu_move`], [`BoardCanvas::bar_menu_tab`]).

use gpui::{Context, EntityInputHandler as _, KeyDownEvent, Window};
use nodus_board::compose::SlashCommand;

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
        // SRCH-001: ⌘K opens search from a text field too (not a terminal).
        if key == "k" && (m.platform || m.control) {
            return self.on_key_down(e, false, cx);
        }
        let commands = self.bar_commands();
        let menu_open = !commands.is_empty();
        let highlighted = self.overlays.bar.active.min(commands.len().saturating_sub(1));
        if menu_open && key == "enter" {
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

    /// BAR-015: ↓ or ↑ while the slash menu is open moves the highlight,
    /// both wrapping. They come as the field's `MoveDown` and `MoveUp`,
    /// whose bindings run before any key handler, so the pill takes them in
    /// the capture phase. True when the menu took the key.
    pub(crate) fn bar_menu_move(&mut self, down: bool, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let count = self.bar_menu_keyed(window, cx).len();
        if count == 0 {
            return false;
        }
        let highlighted = self.overlays.bar.active.min(count - 1);
        let step = if down { 1 } else { count - 1 };
        self.overlays.bar.active = (highlighted + step) % count;
        cx.notify();
        true
    }

    /// BAR-015: Tab while the slash menu is open picks the highlighted
    /// command. It comes as the field's `IndentInline` (the field indents),
    /// taken as ↓ and ↑ are. True when the menu took the key.
    pub(crate) fn bar_menu_tab(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let commands = self.bar_menu_keyed(window, cx);
        let Some(command) = commands.get(self.overlays.bar.active.min(commands.len().saturating_sub(1))) else {
            return false;
        };
        self.bar_pick(command.target, window, cx);
        true
    }

    /// The menu's commands while its keys are the menu's: none while an
    /// input method composes.
    fn bar_menu_keyed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<SlashCommand> {
        let Some(field) = self.overlays.bar.field.clone() else {
            return Vec::new();
        };
        if field.update(cx, |s, cx| s.marked_text_range(window, cx)).is_some() {
            return Vec::new();
        }
        self.bar_commands()
    }
}
