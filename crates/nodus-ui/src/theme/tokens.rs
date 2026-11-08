//! The design tokens of `tokens.css`, resolved for each appearance (spec 08,
//! Design tokens). Every component takes its colours, shadows, radii and
//! timings from here; component-local colours are in [`hard`].

use std::time::Duration;

use gpui::{BoxShadow, Hsla, Pixels, Rgba, point, px};

/// `rgba(r, g, b, a)`.
pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

/// `#rrggbb`.
pub const fn hex(rgb: u32) -> Rgba {
    rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 1.0)
}

/// A colour at a share of its alpha, as `color-mix(in srgb, c p%, transparent)`.
pub fn alpha(color: Rgba, share: f32) -> Hsla {
    Rgba {
        a: color.a * share,
        ..color
    }
    .into()
}

/// One CSS `box-shadow` layer: `x y blur spread colour`. GPUI's blur radius
/// is the Gaussian's σ, which is half of CSS's blur radius.
pub fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: Rgba) -> BoxShadow {
    BoxShadow {
        color: color.into(),
        offset: point(px(x), px(y)),
        blur_radius: px(blur / 2.0),
        spread_radius: px(spread),
        inset: false,
    }
}

/// An `inset` CSS `box-shadow` layer.
pub fn inset_shadow(x: f32, y: f32, blur: f32, spread: f32, color: Rgba) -> BoxShadow {
    BoxShadow {
        inset: true,
        ..shadow(x, y, blur, spread, color)
    }
}

/// A ring of `width` outside the box, following its corners (`0 0 0 w c`).
pub fn ring(width: f32, color: Rgba) -> BoxShadow {
    shadow(0.0, 0.0, 0.0, width, color)
}

/// The colours, shadows and derived surfaces of one appearance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub dark: bool,

    // Surfaces
    pub bg_window: Rgba,
    pub bg_toolbar: Rgba,
    pub bg_canvas: Rgba,
    pub canvas_dot: Rgba,
    pub bg_elevated: Rgba,
    pub bg_sunken: Rgba,
    pub bg_hover: Rgba,
    pub bg_pressed: Rgba,
    pub bg_selected: Rgba,

    // Text
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_tertiary: Rgba,
    pub text_on_accent: Rgba,

    // Lines
    pub border: Rgba,
    pub border_strong: Rgba,
    pub separator: Rgba,

    // Accent
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub focus_ring: Rgba,

    // Session status
    pub status_working: Rgba,
    pub status_done: Rgba,
    pub status_waiting: Rgba,

    // Search matches
    pub mark: Rgba,
    pub mark_strong: Rgba,

    // Diffs
    pub diff_add: Rgba,
    pub diff_del: Rgba,

    // Archive zone
    pub archive_bg: Rgba,
    pub archive_border: Rgba,
    pub archive_label: Rgba,
    pub archive_drop: Rgba,
    pub archive_panel: Rgba,

    // Sticky notes
    pub note_paper: Rgba,
    pub note_ink: Rgba,
    pub note_ink_faint: Rgba,

    pub folder_label_selected: Rgba,

    // Board ink
    pub ink_graphite: Rgba,
    pub ink_red: Rgba,
    pub ink_orange: Rgba,
    pub ink_green: Rgba,
    pub ink_blue: Rgba,
    pub ink_purple: Rgba,

    // Terminal
    pub term_bg: Rgba,
    pub term_fg: Rgba,

    // Shadows, outermost first: (x, y, blur, spread, colour)
    pub shadow_sm: [Layer; 2],
    pub shadow_md: [Layer; 2],
    pub shadow_lg: [Layer; 2],
    pub shadow_modal: [Layer; 2],
    pub note_shadow: [Layer; 2],
    pub note_shadow_lifted: [Layer; 2],

    // Derived (resolved composites)
    /// `--bg-toolbar` over `--bg-window`: the title bar, with nothing to blur behind it.
    pub titlebar_surface: Rgba,
    /// `--bg-elevated` at 88%: context menus.
    pub menu_surface: Rgba,
    /// `--bg-elevated` at 82%: the floating rail.
    pub rail_surface: Rgba,
    /// `--accent` at 55%: a section's drop-target ring.
    pub drop_ring: Rgba,
    /// The dim behind every modal.
    pub modal_backdrop: Rgba,
    /// `--status-waiting` 88% mixed with black: the danger button hovered.
    pub danger_hover: Rgba,
    /// Windows' caption buttons (`titleBarOverlay`): background and symbols.
    pub caption_bg: Rgba,
    pub caption_symbol: Rgba,
}

/// One shadow layer: x, y, CSS blur, spread, colour.
pub type Layer = (f32, f32, f32, f32, Rgba);

/// GPUI shadows from a token's layers. CSS paints its first layer on top;
/// GPUI paints its last on top, so the list is reversed.
pub fn shadows(layers: &[Layer]) -> Vec<BoxShadow> {
    layers
        .iter()
        .rev()
        .map(|&(x, y, b, s, c)| shadow(x, y, b, s, c))
        .collect()
}

impl Tokens {
    pub const LIGHT: Self = Self {
        dark: false,
        bg_window: hex(0xf5f5f4),
        bg_toolbar: rgba(246, 246, 245, 0.82),
        bg_canvas: hex(0xf3f3f1),
        canvas_dot: rgba(0, 0, 0, 0.13),
        bg_elevated: hex(0xffffff),
        bg_sunken: rgba(0, 0, 0, 0.035),
        bg_hover: rgba(0, 0, 0, 0.05),
        bg_pressed: rgba(0, 0, 0, 0.09),
        bg_selected: rgba(0, 122, 255, 0.14),
        text_primary: rgba(0, 0, 0, 0.88),
        text_secondary: rgba(0, 0, 0, 0.56),
        text_tertiary: rgba(0, 0, 0, 0.36),
        text_on_accent: hex(0xffffff),
        border: rgba(0, 0, 0, 0.1),
        border_strong: rgba(0, 0, 0, 0.18),
        separator: rgba(0, 0, 0, 0.07),
        accent: hex(0x007aff),
        accent_hover: hex(0x0068d9),
        focus_ring: rgba(0, 122, 255, 0.5),
        status_working: hex(0xff9500),
        status_done: hex(0x007aff),
        status_waiting: hex(0xff3b30),
        mark: rgba(255, 196, 0, 0.32),
        mark_strong: rgba(255, 170, 0, 0.75),
        diff_add: hex(0x2da44e),
        diff_del: hex(0xcf222e),
        archive_bg: rgba(120, 120, 128, 0.07),
        archive_border: rgba(120, 120, 128, 0.35),
        archive_label: rgba(0, 0, 0, 0.42),
        archive_drop: rgba(0, 122, 255, 0.08),
        archive_panel: rgba(248, 248, 247, 0.8),
        note_paper: hex(0xffffff),
        note_ink: rgba(0, 0, 0, 0.82),
        note_ink_faint: rgba(0, 0, 0, 0.38),
        folder_label_selected: hex(0x0063e1),
        ink_graphite: hex(0x1d1d1f),
        ink_red: hex(0xff3b30),
        ink_orange: hex(0xff9500),
        ink_green: hex(0x28cd41),
        ink_blue: hex(0x007aff),
        ink_purple: hex(0xaf52de),
        term_bg: hex(0x1e1e20),
        term_fg: hex(0xe6e6e6),
        shadow_sm: [
            (0.0, 1.0, 2.0, 0.0, rgba(0, 0, 0, 0.06)),
            (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.08)),
        ],
        shadow_md: [
            (0.0, 4.0, 14.0, 0.0, rgba(0, 0, 0, 0.08)),
            (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.1)),
        ],
        shadow_lg: [
            (0.0, 18.0, 50.0, 0.0, rgba(0, 0, 0, 0.18)),
            (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.12)),
        ],
        shadow_modal: [
            (0.0, 30.0, 90.0, 0.0, rgba(0, 0, 0, 0.35)),
            (0.0, 0.0, 0.0, 0.5, rgba(0, 0, 0, 0.2)),
        ],
        note_shadow: [
            (0.0, 1.0, 1.5, 0.0, rgba(0, 0, 0, 0.08)),
            (0.0, 8.0, 16.0, -8.0, rgba(0, 0, 0, 0.3)),
        ],
        note_shadow_lifted: [
            (0.0, 2.0, 4.0, 0.0, rgba(0, 0, 0, 0.08)),
            (0.0, 20.0, 32.0, -12.0, rgba(0, 0, 0, 0.36)),
        ],
        titlebar_surface: hex(0xf6f6f5),
        menu_surface: rgba(255, 255, 255, 0.88),
        rail_surface: rgba(255, 255, 255, 0.82),
        drop_ring: rgba(0, 122, 255, 0.55),
        modal_backdrop: rgba(0, 0, 0, 0.28),
        danger_hover: hex(0xe0342a),
        caption_bg: hex(0xf6f6f5),
        caption_symbol: hex(0x1f1f1f),
    };

    pub const DARK: Self = Self {
        dark: true,
        bg_window: hex(0x1e1e20),
        bg_toolbar: rgba(38, 38, 41, 0.82),
        bg_canvas: hex(0x19191b),
        canvas_dot: rgba(255, 255, 255, 0.11),
        bg_elevated: hex(0x2a2a2d),
        bg_sunken: rgba(255, 255, 255, 0.04),
        bg_hover: rgba(255, 255, 255, 0.07),
        bg_pressed: rgba(255, 255, 255, 0.12),
        bg_selected: rgba(10, 132, 255, 0.24),
        text_primary: rgba(255, 255, 255, 0.9),
        text_secondary: rgba(255, 255, 255, 0.58),
        text_tertiary: rgba(255, 255, 255, 0.36),
        text_on_accent: hex(0xffffff),
        border: rgba(255, 255, 255, 0.1),
        border_strong: rgba(255, 255, 255, 0.18),
        separator: rgba(255, 255, 255, 0.07),
        accent: hex(0x0a84ff),
        accent_hover: hex(0x3396ff),
        focus_ring: rgba(10, 132, 255, 0.55),
        status_working: hex(0xff9f0a),
        status_done: hex(0x0a84ff),
        status_waiting: hex(0xff453a),
        mark: rgba(255, 214, 10, 0.26),
        mark_strong: rgba(255, 214, 10, 0.7),
        diff_add: hex(0x3fb950),
        diff_del: hex(0xf85149),
        archive_bg: rgba(160, 160, 170, 0.06),
        archive_border: rgba(160, 160, 170, 0.3),
        archive_label: rgba(255, 255, 255, 0.42),
        archive_drop: rgba(10, 132, 255, 0.12),
        archive_panel: rgba(38, 38, 41, 0.8),
        note_paper: hex(0xb4b4b8),
        note_ink: rgba(0, 0, 0, 0.82),
        note_ink_faint: rgba(0, 0, 0, 0.38),
        folder_label_selected: hex(0x0a84ff),
        ink_graphite: hex(0xf2f2f4),
        ink_red: hex(0xff453a),
        ink_orange: hex(0xff9f0a),
        ink_green: hex(0x32d74b),
        ink_blue: hex(0x0a84ff),
        ink_purple: hex(0xbf5af2),
        term_bg: hex(0x1e1e20),
        term_fg: hex(0xe6e6e6),
        shadow_sm: [
            (0.0, 1.0, 2.0, 0.0, rgba(0, 0, 0, 0.3)),
            (0.0, 0.0, 0.0, 0.5, rgba(255, 255, 255, 0.08)),
        ],
        shadow_md: [
            (0.0, 4.0, 14.0, 0.0, rgba(0, 0, 0, 0.35)),
            (0.0, 0.0, 0.0, 0.5, rgba(255, 255, 255, 0.1)),
        ],
        shadow_lg: [
            (0.0, 18.0, 50.0, 0.0, rgba(0, 0, 0, 0.5)),
            (0.0, 0.0, 0.0, 0.5, rgba(255, 255, 255, 0.12)),
        ],
        shadow_modal: [
            (0.0, 30.0, 90.0, 0.0, rgba(0, 0, 0, 0.6)),
            (0.0, 0.0, 0.0, 0.5, rgba(255, 255, 255, 0.14)),
        ],
        note_shadow: [
            (0.0, 1.0, 1.5, 0.0, rgba(0, 0, 0, 0.3)),
            (0.0, 8.0, 18.0, -8.0, rgba(0, 0, 0, 0.6)),
        ],
        note_shadow_lifted: [
            (0.0, 2.0, 4.0, 0.0, rgba(0, 0, 0, 0.3)),
            (0.0, 20.0, 34.0, -12.0, rgba(0, 0, 0, 0.7)),
        ],
        titlebar_surface: hex(0x252527),
        menu_surface: rgba(42, 42, 45, 0.88),
        rail_surface: rgba(42, 42, 45, 0.82),
        drop_ring: rgba(10, 132, 255, 0.55),
        modal_backdrop: rgba(0, 0, 0, 0.45),
        danger_hover: hex(0xe03d33),
        caption_bg: hex(0x252527),
        caption_symbol: hex(0xe6e6e6),
    };

    pub fn sm(&self) -> Vec<BoxShadow> {
        shadows(&self.shadow_sm)
    }
    pub fn md(&self) -> Vec<BoxShadow> {
        shadows(&self.shadow_md)
    }
    pub fn lg(&self) -> Vec<BoxShadow> {
        shadows(&self.shadow_lg)
    }
    pub fn modal(&self) -> Vec<BoxShadow> {
        shadows(&self.shadow_modal)
    }

    /// The folder colour, the status colour, or the ink colour a name refers to.
    pub fn status(&self, status: nodus_domain::SessionStatus) -> Rgba {
        match status {
            nodus_domain::SessionStatus::Working => self.status_working,
            nodus_domain::SessionStatus::Waiting => self.status_waiting,
            nodus_domain::SessionStatus::Done => self.status_done,
        }
    }
}

// Shape, motion and layout tokens: the same in both themes.
pub const RADIUS_SM: Pixels = px(6.0);
pub const RADIUS_MD: Pixels = px(10.0);
pub const RADIUS_LG: Pixels = px(14.0);
pub const RADIUS_XL: Pixels = px(18.0);
pub const DUR_FAST: Duration = Duration::from_millis(120);
pub const DUR_MED: Duration = Duration::from_millis(220);
pub const TITLEBAR_H: Pixels = px(52.0);
pub const RAIL_W: Pixels = px(52.0);
pub const RAIL_INSET: Pixels = px(8.0);
pub const CARD_W: Pixels = px(288.0);
pub const CARD_H: Pixels = px(216.0);

// Type sizes.
pub const TEXT_XS: Pixels = px(11.0);
pub const TEXT_SM: Pixels = px(12.0);
pub const TEXT_MD: Pixels = px(13.0);
pub const TEXT_LG: Pixels = px(15.0);
pub const TEXT_XL: Pixels = px(20.0);
pub const TEXT_2XL: Pixels = px(26.0);

/// Colours components hard-code in their own CSS, the same in both themes
/// unless named for one.
pub mod hard {
    use super::{Rgba, hex, rgba};

    pub const SWITCH_KNOB: Rgba = hex(0xffffff);
    pub const SWITCH_KNOB_SHADOW: Rgba = rgba(0, 0, 0, 0.25);
    pub const SWATCH_RING: Rgba = rgba(0, 0, 0, 0.18);
    pub const WHITE: Rgba = hex(0xffffff);

    /// The Settings theme previews' miniature windows, in each tone.
    #[derive(Debug, Clone, Copy)]
    pub struct MiniTone {
        pub canvas: Rgba,
        pub bar: Rgba,
        pub card: Rgba,
        pub border: Rgba,
        pub line: Rgba,
        pub dot: Rgba,
    }

    pub const MINI_LIGHT: MiniTone = MiniTone {
        canvas: hex(0xf3f3f1),
        bar: hex(0xebebea),
        card: hex(0xffffff),
        border: rgba(0, 0, 0, 0.09),
        line: rgba(0, 0, 0, 0.12),
        dot: rgba(0, 0, 0, 0.13),
    };

    pub const MINI_DARK: MiniTone = MiniTone {
        canvas: hex(0x19191b),
        bar: hex(0x262629),
        card: hex(0x2a2a2d),
        border: rgba(255, 255, 255, 0.09),
        line: rgba(255, 255, 255, 0.16),
        dot: rgba(255, 255, 255, 0.11),
    };

    pub const MINI_LIGHTS: [Rgba; 3] = [hex(0xff5f57), hex(0xfebc2e), hex(0x28c840)];
    pub const MINI_CARD_SHADOW: Rgba = rgba(0, 0, 0, 0.08);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_surfaces_are_the_composites_the_spec_resolves() {
        // --bg-toolbar over --bg-window, rounded to 8 bits.
        for t in [Tokens::LIGHT, Tokens::DARK] {
            let over = |top: f32, below: f32| (top * t.bg_toolbar.a + below * (1.0 - t.bg_toolbar.a)) * 255.0;
            let mixed = (
                over(t.bg_toolbar.r, t.bg_window.r).round(),
                over(t.bg_toolbar.g, t.bg_window.g).round(),
                over(t.bg_toolbar.b, t.bg_window.b).round(),
            );
            let surface = (
                (t.titlebar_surface.r * 255.0).round(),
                (t.titlebar_surface.g * 255.0).round(),
                (t.titlebar_surface.b * 255.0).round(),
            );
            assert_eq!(mixed, surface, "dark={}", t.dark);
            assert_eq!(t.caption_bg, t.titlebar_surface);
        }
    }

    #[test]
    fn css_blur_is_twice_gpuis_sigma() {
        assert_eq!(shadow(0.0, 4.0, 14.0, 0.0, hex(0)).blur_radius, px(7.0));
    }
}
