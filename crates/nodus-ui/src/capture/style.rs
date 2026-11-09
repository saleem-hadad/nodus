//! The panel's own colours and shadows (`capture.css`), on top of the
//! shared tokens: the stage's inks, the card's and the receipt's shadows,
//! the note yellows and the status marks.

use gpui::{BoxShadow, Rgba};
use nodus_domain::FolderColor;

use crate::components::folder_icon::folder_hues;
use crate::theme::tokens::{Tokens, hex, rgba, shadow};

/// `.cap-stage`'s inks, per theme.
#[derive(Debug, Clone, Copy)]
pub struct Inks {
    /// `--into-ink`: `--accent` in light, `#5eacff` in dark.
    pub into: Rgba,
    pub view: Rgba,
    pub working: Rgba,
    pub waiting: Rgba,
}

pub fn inks(t: &Tokens) -> Inks {
    if t.dark {
        Inks {
            into: hex(0x5eacff),
            view: hex(0xd9a3f7),
            working: hex(0xffb340),
            waiting: hex(0xff7a72),
        }
    } else {
        Inks {
            into: t.accent,
            view: hex(0x8944ab),
            working: hex(0xb25000),
            waiting: hex(0xd70015),
        }
    }
}

/// The note swatch and a note's mark (`#ffcc00`).
pub const NOTE_YELLOW: Rgba = hex(0xffcc00);
/// A view's mark (`#af52de`).
pub const VIEW_PURPLE: Rgba = hex(0xaf52de);
/// The image tile's label and its remove button.
pub const LABEL_BG: Rgba = rgba(0, 0, 0, 0.6);
pub const IMAGE_X_BG: Rgba = rgba(0, 0, 0, 0.5);
pub const WHITE: Rgba = hex(0xffffff);
/// The send button's key cap.
pub const SEND_KEY_BG: Rgba = rgba(255, 255, 255, 0.22);
/// The note receipt's top shade, `linear-gradient(rgba(0,0,0,.035), transparent 22px)`.
pub const NOTE_SHADE: Rgba = rgba(0, 0, 0, 0.035);

/// `.cap-card`'s `--cap-shadow`, CSS order (outermost first).
fn card_layers(t: &Tokens) -> [(f32, f32, f32, f32, Rgba); 2] {
    if t.dark {
        [
            (0.0, 16.0, 40.0, 0.0, rgba(0, 0, 0, 0.55)),
            (0.0, 0.0, 0.0, 0.5, rgba(255, 255, 255, 0.16)),
        ]
    } else {
        [
            (0.0, 16.0, 40.0, 0.0, rgba(0, 0, 0, 0.24)),
            (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.16)),
        ]
    }
}

/// GPUI shadows from CSS layers: CSS paints its first on top, GPUI its last.
fn shadows(layers: &[(f32, f32, f32, f32, Rgba)]) -> Vec<BoxShadow> {
    layers
        .iter()
        .rev()
        .map(|&(x, y, b, s, c)| shadow(x, y, b, s, c))
        .collect()
}

/// The card's shadow, and with files dragged over it the 2px accent ring
/// outside it (CAP-097).
pub fn card_shadow(t: &Tokens, dropping: bool) -> Vec<BoxShadow> {
    let mut layers = card_layers(t).to_vec();
    if dropping {
        layers.push((0.0, 0.0, 0.0, 2.0, t.accent));
    }
    shadows(&layers)
}

/// `.cap-receipt`'s shadow: the light card's, in both themes.
pub fn receipt_shadow() -> Vec<BoxShadow> {
    shadows(&[
        (0.0, 16.0, 40.0, 0.0, rgba(0, 0, 0, 0.24)),
        (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.16)),
    ])
}

/// A recent capture's shadow: `--shadow-md`, `--shadow-lg` hovered, and
/// a waiting session's 2px ring on top of `--shadow-md`.
pub fn recent_shadow(t: &Tokens, hovered: bool, waiting: bool) -> Vec<BoxShadow> {
    let mut layers: Vec<_> = Vec::new();
    if waiting {
        layers.push((0.0, 0.0, 0.0, 2.0, t.status_waiting));
        layers.extend_from_slice(&t.shadow_md);
    } else if hovered {
        layers.extend_from_slice(&t.shadow_lg);
    } else {
        layers.extend_from_slice(&t.shadow_md);
    }
    shadows(&layers)
}

/// `--note-shadow`, `--note-shadow-lifted`.
pub fn note_shadow(t: &Tokens, lifted: bool) -> Vec<BoxShadow> {
    shadows(if lifted { &t.note_shadow_lifted } else { &t.note_shadow })
}

/// `color-mix(in srgb, <yellow frontTop> 72%, var(--note-paper))`: the
/// receipt's paper (`#ffea9f` / `#ead58b`).
pub fn note_paper(t: &Tokens) -> Rgba {
    let hue = folder_hues(FolderColor::Yellow).front_top;
    let paper = t.note_paper;
    let mix = |a: f32, b: f32| ((a * 0.72 + b * 0.28) * 255.0).round() / 255.0;
    Rgba {
        r: mix(hue.r, paper.r),
        g: mix(hue.g, paper.g),
        b: mix(hue.b, paper.b),
        a: 1.0,
    }
}

/// `--bg-elevated` at `share` (`color-mix(in srgb, … p%, transparent)`).
pub fn elevated_at(t: &Tokens, share: f32) -> Rgba {
    Rgba {
        a: t.bg_elevated.a * share,
        ..t.bg_elevated
    }
}

/// `--status-*` at 28%: the working and waiting dots' rings.
pub fn ring_of(color: Rgba) -> Rgba {
    Rgba { a: 0.28, ..color }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_hex(c: Rgba) -> u32 {
        let b = |v: f32| (v * 255.0).round() as u32;
        (b(c.r) << 16) | (b(c.g) << 8) | b(c.b)
    }

    #[test]
    fn mixes_the_receipt_paper_as_chromium_does() {
        assert_eq!(to_hex(note_paper(&Tokens::LIGHT)), 0xffea9f);
        let dark = to_hex(note_paper(&Tokens::DARK));
        // `#ead58b` in the spec; the last channel rounds either way.
        assert!(dark == 0xead58b || dark == 0xead58a, "{dark:x}");
    }

    #[test]
    fn rings_a_dropping_card() {
        assert_eq!(card_shadow(&Tokens::LIGHT, false).len(), 2);
        assert_eq!(card_shadow(&Tokens::LIGHT, true).len(), 3);
    }
}
