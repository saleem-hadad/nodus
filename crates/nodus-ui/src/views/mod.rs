//! Views on the board (spec 04's UI): the view card (`ViewNode.tsx`,
//! `views.css`), its live page in a native webview (`ViewFrame`), the relay
//! between pages and the app (`viewRelay.ts`), the large window
//! (`ViewModal`) and the New view dialog (`ViewDialog.tsx`).
//!
//! - [`card`]: the card: title bar, tools, approvals, body, error, ask bar.
//! - [`frames`]: one webview per frame (the card's, and the large window's),
//!   kept for every view with a page while its board is open, placed over
//!   the card's body, and swapped for a snapshot whenever GPUI must draw over
//!   it (decision D3).
//! - [`webview`]: a frame's webview: the shell page, the custom scheme, IPC.
//! - [`ink`]: the board's ink over live pages, drawn in a native overlay
//!   above each page, as 0.9.4's ink layer drew over its iframes.
//! - [`relay`]: what pages ask for, and what the board tells them.
//! - [`large`], [`dialog`]: the large window and the New view dialog.

pub mod card;
pub mod dialog;
pub mod frames;
pub mod icons;
pub mod ink;
mod inputs;
pub mod large;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mac_ink;
mod parts;
pub mod relay;
mod resize;
pub mod shell;
pub mod swap;
mod webview;

use std::rc::Rc;

use gpui::{AnyElement, App, Context, Window};

use crate::board::canvas::BoardCanvas;
use crate::board::nodes::ViewRenderer;
#[cfg(target_os = "macos")]
pub use mac::{hit_class, native_click, native_keys, responder_class};

/// A press anywhere GPUI draws gives GPUI the keyboard back from the page
/// the user last clicked, as a click outside a frame blurs it (VIEW-051,
/// VIEW-149: the board's keys are the board's again). The window's root calls
/// it on every press: a page keeps the keyboard while none of its frames is
/// drawn (off screen, its board closed), and a page closed while it had the
/// keyboard leaves it with the window.
pub fn keyboard_to_board(window: &Window) {
    #[cfg(target_os = "macos")]
    mac::keyboard_to_board(window);
    // WebView2: the shell's own check (its frame has the focus) stands.
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Sets views up once at start-up: the view renderer the canvas draws view
/// nodes with.
pub fn install(cx: &mut App) {
    cx.set_global(ViewRenderer(Rc::new(card::render)));
}

/// What the board draws for views besides their cards, each frame: keeps a
/// webview for every view with a page (VIEW-029, VIEW-030, R13), and draws
/// the large window (VIEW-147) and the New view dialog (VIEW-151…154).
pub fn board_layer(
    canvas: &mut BoardCanvas,
    window: &mut Window,
    cx: &mut Context<'_, BoardCanvas>,
) -> Vec<AnyElement> {
    frames::sync_board(canvas, window, cx);
    ink::sync(canvas, window, cx);
    relay::board_changed(canvas, cx);
    let mut out = Vec::new();
    out.extend(large::render(canvas, window, cx));
    out.extend(dialog::render(canvas, window, cx));
    out
}

/// IPC errors arrive as "Error invoking remote method '…': Error: …"; what
/// the page, the card and the dialog show is the last part (VIEW-053).
pub fn reason(message: &str) -> String {
    let Some(rest) = message.strip_prefix("Error invoking remote method '") else {
        return message.to_owned();
    };
    let Some(end) = rest.find('\'') else {
        return message.to_owned();
    };
    // `[^']+` needs at least one character before the quote.
    if end == 0 {
        return message.to_owned();
    }
    let Some(rest) = rest[end + 1..].strip_prefix(": ") else {
        return message.to_owned();
    };
    rest.strip_prefix("Error: ").unwrap_or(rest).to_owned()
}

/// A command's schedule, as the approvals strip says it (`everyLabel`,
/// VIEW-136): "every 10 minutes", "every hour", "every 2 days".
pub fn every_label(minutes: f64) -> String {
    let n = nodus_domain::json::number_to_string;
    if minutes % 60.0 != 0.0 {
        return if minutes == 1.0 {
            "every minute".to_owned()
        } else {
            format!("every {} minutes", n(minutes))
        };
    }
    let hours = minutes / 60.0;
    if hours % 24.0 == 0.0 {
        return if hours == 24.0 {
            "every day".to_owned()
        } else {
            format!("every {} days", n(hours / 24.0))
        };
    }
    if hours == 1.0 {
        "every hour".to_owned()
    } else {
        format!("every {} hours", n(hours))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_ipc_prefix_from_errors() {
        assert_eq!(
            reason("Error invoking remote method 'views:call': Error: Asked too soon"),
            "Asked too soon"
        );
        assert_eq!(reason("Error invoking remote method 'views:ask': Nope"), "Nope");
        assert_eq!(reason("Plain"), "Plain");
        assert_eq!(
            reason("Error invoking remote method '': Error: x"),
            "Error invoking remote method '': Error: x"
        );
        // Only the leading `Error: ` goes.
        assert_eq!(
            reason("Error invoking remote method 'a': Error: Error: twice"),
            "Error: twice"
        );
    }

    #[test]
    fn labels_schedules_as_0_9_4_did() {
        assert_eq!(every_label(1.0), "every minute");
        assert_eq!(every_label(10.0), "every 10 minutes");
        assert_eq!(every_label(30.0), "every 30 minutes");
        assert_eq!(every_label(90.0), "every 90 minutes");
        assert_eq!(every_label(60.0), "every hour");
        assert_eq!(every_label(120.0), "every 2 hours");
        assert_eq!(every_label(1440.0), "every day");
        assert_eq!(every_label(2880.0), "every 2 days");
        assert_eq!(every_label(10080.0), "every 7 days");
    }
}
