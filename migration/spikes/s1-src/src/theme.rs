//! A slice of Nodus' tokens.css (light + dark), as a GPUI global that follows the window appearance.
use gpui::*;

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub bg_window: Hsla,
    pub bg_toolbar: Hsla,
    pub bg_canvas: Hsla,
    pub canvas_dot: Hsla,
    pub bg_elevated: Hsla,
    pub bg_selected: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub note_paper: Hsla,
    pub note_ink: Hsla,
    pub shadow_a: Hsla,
    pub shadow_b: Hsla,
    pub term_bg: Hsla,
    pub term_fg: Hsla,
}

impl Global for Tokens {}

pub fn rgba_hex(hex: u32, a: f32) -> Hsla {
    let r = ((hex >> 16) & 0xff) as f32 / 255.;
    let g = ((hex >> 8) & 0xff) as f32 / 255.;
    let b = (hex & 0xff) as f32 / 255.;
    Rgba { r, g, b, a }.into()
}

impl Tokens {
    pub fn light() -> Self {
        Self {
            dark: false,
            bg_window: rgba_hex(0xf5f5f4, 1.),
            bg_toolbar: rgba_hex(0xf6f6f5, 0.82),
            bg_canvas: rgba_hex(0xf3f3f1, 1.),
            canvas_dot: rgba_hex(0x000000, 0.13),
            bg_elevated: rgba_hex(0xffffff, 1.),
            bg_selected: rgba_hex(0x007aff, 0.14),
            text_primary: rgba_hex(0x000000, 0.88),
            text_secondary: rgba_hex(0x000000, 0.56),
            border: rgba_hex(0x000000, 0.1),
            accent: rgba_hex(0x007aff, 1.),
            note_paper: rgba_hex(0xffffff, 1.),
            note_ink: rgba_hex(0x000000, 0.82),
            shadow_a: rgba_hex(0x000000, 0.08),
            shadow_b: rgba_hex(0x000000, 0.30),
            term_bg: rgba_hex(0x1e1e20, 1.),
            term_fg: rgba_hex(0xe6e6e6, 1.),
        }
    }
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg_window: rgba_hex(0x1e1e20, 1.),
            bg_toolbar: rgba_hex(0x262629, 0.82),
            bg_canvas: rgba_hex(0x19191b, 1.),
            canvas_dot: rgba_hex(0xffffff, 0.11),
            bg_elevated: rgba_hex(0x2a2a2d, 1.),
            bg_selected: rgba_hex(0x0a84ff, 0.24),
            text_primary: rgba_hex(0xffffff, 0.9),
            text_secondary: rgba_hex(0xffffff, 0.58),
            border: rgba_hex(0xffffff, 0.1),
            accent: rgba_hex(0x0a84ff, 1.),
            note_paper: rgba_hex(0xb4b4b8, 1.),
            note_ink: rgba_hex(0x000000, 0.82),
            shadow_a: rgba_hex(0x000000, 0.3),
            shadow_b: rgba_hex(0x000000, 0.6),
            term_bg: rgba_hex(0x1e1e20, 1.),
            term_fg: rgba_hex(0xe6e6e6, 1.),
        }
    }
}

pub fn init(cx: &mut App) {
    let dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    cx.set_global(if dark { Tokens::dark() } else { Tokens::light() });
}

/// Call from a root view: picks tokens + gpui-component theme from this window's (effective) appearance.
pub fn sync(window: &mut Window, cx: &mut App) {
    let dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    cx.set_global(if dark { Tokens::dark() } else { Tokens::light() });
    gpui::component::Theme::sync_system_appearance(Some(window), cx);
}

pub fn tokens(cx: &App) -> Tokens {
    *cx.global::<Tokens>()
}
