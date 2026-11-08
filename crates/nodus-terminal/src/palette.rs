//! The terminal's colours: the fixed dark theme in registry.ts (`TERMINAL_THEME`),
//! which is the same in the light and the dark app theme (TERM-084), and how
//! xterm.js 6's WebGL renderer turns a cell's attributes into the colours it
//! draws (TERM-058, TERM-079): bold text in ANSI 0-7 drawn bright, dim at half
//! opacity, inverse, and the selection blended over the cell.

/// An opaque colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// `0xRRGGBB`.
    pub const fn hex(v: u32) -> Self {
        Self {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    /// With an alpha channel.
    pub const fn with_alpha(self, a: u8) -> Rgba {
        Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }
}

/// A colour with an alpha channel, as the theme's `#rrggbbaa` strings give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// `0xRRGGBBAA`.
    pub const fn hex(v: u32) -> Self {
        Self {
            r: (v >> 24) as u8,
            g: (v >> 16) as u8,
            b: (v >> 8) as u8,
            a: v as u8,
        }
    }

    pub const fn rgb(self) -> Rgb {
        Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }

    /// The alpha as a fraction, as the browser reads it.
    pub fn alpha(self) -> f32 {
        f32::from(self.a) / 255.0
    }
}

/// registry.ts `TERMINAL_THEME`, and the xterm defaults the app relies on.
pub mod theme {
    use super::{Rgb, Rgba};

    pub const BACKGROUND: Rgb = Rgb::hex(0x1e1e20);
    pub const FOREGROUND: Rgb = Rgb::hex(0xe6e6e6);
    pub const CURSOR: Rgb = Rgb::hex(0xe6e6e6);
    /// The glyph under a block cursor.
    pub const CURSOR_ACCENT: Rgb = Rgb::hex(0x1e1e20);
    /// The selection while the terminal has focus.
    pub const SELECTION_BACKGROUND: Rgba = Rgba::hex(0x0a84ff55);
    /// The selection while it doesn't.
    pub const SELECTION_INACTIVE_BACKGROUND: Rgba = Rgba::hex(0x8e8e9340);
    pub const SCROLLBAR_SLIDER_BACKGROUND: Rgba = Rgba::hex(0xffffff1f);
    pub const SCROLLBAR_SLIDER_HOVER_BACKGROUND: Rgba = Rgba::hex(0xffffff33);
    pub const SCROLLBAR_SLIDER_ACTIVE_BACKGROUND: Rgba = Rgba::hex(0xffffff47);

    /// ANSI 0-15: black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub const ANSI: [Rgb; 16] = [
        Rgb::hex(0x3a3a3c),
        Rgb::hex(0xff6b62),
        Rgb::hex(0x5fd068),
        Rgb::hex(0xf5c451),
        Rgb::hex(0x5aa9ff),
        Rgb::hex(0xd68cf7),
        Rgb::hex(0x5ad1e0),
        Rgb::hex(0xd8d8dc),
        Rgb::hex(0x6e6e73),
        Rgb::hex(0xff8a80),
        Rgb::hex(0x7fe08a),
        Rgb::hex(0xffd978),
        Rgb::hex(0x80bdff),
        Rgb::hex(0xe2a8fb),
        Rgb::hex(0x80e2ee),
        Rgb::hex(0xf5f5f7),
    ];

    /// SGR 2 draws the foreground at this opacity (`addon-webgl/src/Constants.ts`).
    pub const DIM_OPACITY: f32 = 0.5;

    /// IME composition text (TERM-055): `.xterm .composition-view` is white on black.
    pub const COMPOSITION_BACKGROUND: Rgb = Rgb::hex(0x000000);
    pub const COMPOSITION_FOREGROUND: Rgb = Rgb::hex(0xffffff);
}

/// The 256-colour palette entry `index` before any program changes it: the
/// theme's 16, xterm's 6×6×6 cube, then 24 greys (`browser/Types.ts`).
pub const fn default_color(index: u8) -> Rgb {
    const LEVELS: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    match index {
        0..=15 => theme::ANSI[index as usize],
        16..=231 => {
            let i = index - 16;
            Rgb {
                r: LEVELS[(i / 36) as usize],
                g: LEVELS[((i / 6) % 6) as usize],
                b: LEVELS[(i % 6) as usize],
            }
        }
        _ => {
            let v = 8 + (index - 232) * 10;
            Rgb { r: v, g: v, b: v }
        }
    }
}

/// `Math.round`: halves go up, toward +∞.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// xterm's `color.blend(bg, fg)`: `fg` drawn over the opaque `bg`, each channel
/// rounded as `Math.round` does.
pub fn blend(bg: Rgb, fg: Rgba) -> Rgb {
    if fg.a == 0xff {
        return fg.rgb();
    }
    let a = f64::from(fg.a) / 255.0;
    let channel = |b: u8, f: u8| (f64::from(b) + js_round((f64::from(f) - f64::from(b)) * a)) as u8;
    Rgb {
        r: channel(bg.r, fg.r),
        g: channel(bg.g, fg.g),
        b: channel(bg.b, fg.b),
    }
}

/// The colours in force: the theme, with whatever OSC 4/10/11/12 changed since
/// (and OSC 104/110/111/112 restored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    pub ansi: [Rgb; 256],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

impl Default for Colors {
    fn default() -> Self {
        let mut ansi = [Rgb::hex(0); 256];
        for (i, slot) in ansi.iter_mut().enumerate() {
            *slot = default_color(i as u8);
        }
        Self {
            ansi,
            foreground: theme::FOREGROUND,
            background: theme::BACKGROUND,
            cursor: theme::CURSOR,
        }
    }
}

impl Colors {
    /// The theme with alacritty's record of the colours a program set on top.
    pub fn with_overrides(overrides: &alacritty_terminal::term::color::Colors) -> Self {
        use alacritty_terminal::vte::ansi::NamedColor;
        let mut colors = Self::default();
        let get = |i: usize| overrides[i].map(|c| Rgb { r: c.r, g: c.g, b: c.b });
        for i in 0..256 {
            if let Some(c) = get(i) {
                colors.ansi[i] = c;
            }
        }
        if let Some(c) = get(NamedColor::Foreground as usize) {
            colors.foreground = c;
        }
        if let Some(c) = get(NamedColor::Background as usize) {
            colors.background = c;
        }
        if let Some(c) = get(NamedColor::Cursor as usize) {
            colors.cursor = c;
        }
        colors
    }

    /// The selection colour made opaque over the background, as xterm's theme
    /// service precomputes it (`selectionBackgroundOpaque`).
    pub fn selection_opaque(&self, focused: bool) -> Rgb {
        let selection = if focused {
            theme::SELECTION_BACKGROUND
        } else {
            theme::SELECTION_INACTIVE_BACKGROUND
        };
        blend(self.background, selection)
    }

    fn resolve(&self, color: CellColor, default: Rgb) -> Rgb {
        match color {
            CellColor::Default => default,
            CellColor::Indexed(i) => self.ansi[i as usize],
            CellColor::Rgb(c) => c,
        }
    }
}

/// A cell's colour as xterm stores it: the default, a palette index (SGR 30-37,
/// 90-97, 38;5;n) or a truecolour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellColor {
    Default,
    Indexed(u8),
    Rgb(Rgb),
}

impl CellColor {
    /// From alacritty's cell colour. The parser only ever stores the default
    /// foreground/background, ANSI 0-15, an index or an RGB.
    pub fn from_alacritty(color: alacritty_terminal::vte::ansi::Color) -> Self {
        use alacritty_terminal::vte::ansi::Color;
        match color {
            Color::Named(n) if (n as usize) < 16 => Self::Indexed(n as u8),
            Color::Named(_) => Self::Default,
            Color::Indexed(i) => Self::Indexed(i),
            Color::Spec(c) => Self::Rgb(Rgb { r: c.r, g: c.g, b: c.b }),
        }
    }
}

/// What a cell's colours depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttrs {
    pub bold: bool,
    pub dim: bool,
    pub inverse: bool,
}

/// The colours to draw a cell with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellColors {
    /// The glyph's colour, drawn at [`theme::DIM_OPACITY`] when `dim`.
    pub fg: Rgb,
    pub dim: bool,
    /// The cell's background.
    pub bg: Rgb,
    /// Whether `bg` is just the terminal's background (nothing to draw).
    pub bg_is_default: bool,
}

/// Box drawing, block elements and powerline symbols take the selection as
/// their glyph colour too (`treatGlyphAsBackgroundColor`).
pub fn glyph_is_background(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0xE0A4..=0xE0D6)
}

/// How xterm 6's WebGL renderer colours a cell (`CellColorResolver.resolve`,
/// `TextureAtlas._getForegroundColor`, `RectangleRenderer._updateRectangle`).
/// `selected` is `Some(focused)` when the cell is inside the selection.
pub fn resolve_cell_colors(
    mut fg: CellColor,
    mut bg: CellColor,
    attrs: CellAttrs,
    glyph: char,
    selected: Option<bool>,
    colors: &Colors,
) -> CellColors {
    let mut inverse = attrs.inverse;
    if let Some(focused) = selected {
        let opaque = colors.selection_opaque(focused);
        let half = opaque.with_alpha(0x80);
        // The selection replaces the background, blended with the cell's own
        // colour when it has one.
        let new_bg = if inverse || bg != CellColor::Default {
            let base = if inverse {
                colors.resolve(fg, colors.foreground)
            } else {
                colors.resolve(bg, colors.background)
            };
            blend(base, half)
        } else {
            opaque
        };
        let mut new_fg = None;
        if glyph_is_background(glyph) {
            new_fg = Some(if inverse && bg == CellColor::Default {
                CellColor::Rgb(opaque)
            } else {
                let base = if inverse {
                    colors.resolve(bg, colors.background)
                } else {
                    colors.resolve(fg, colors.foreground)
                };
                CellColor::Rgb(blend(base, half))
            });
        }
        // Inverse with only the background overridden: the glyph takes the
        // cell's background colour (the theme's when it has none).
        if inverse && new_fg.is_none() {
            new_fg = Some(if bg == CellColor::Default {
                CellColor::Rgb(colors.background)
            } else {
                bg
            });
        }
        if let Some(f) = new_fg {
            fg = f;
            inverse = false;
        }
        bg = CellColor::Rgb(new_bg);
    }

    // The background rectangle.
    let (bg_rgb, bg_is_default) = if inverse {
        (colors.resolve(fg, colors.foreground), false)
    } else {
        (colors.resolve(bg, colors.background), bg == CellColor::Default)
    };

    // The glyph: inverse swaps first, then bold brightens ANSI 0-7.
    let (glyph_color, default) = if inverse {
        (bg, colors.background)
    } else {
        (fg, colors.foreground)
    };
    let fg_rgb = match glyph_color {
        CellColor::Indexed(i) if attrs.bold && i < 8 => colors.ansi[i as usize + 8],
        other => colors.resolve(other, default),
    };
    CellColors {
        fg: fg_rgb,
        dim: attrs.dim,
        bg: bg_rgb,
        bg_is_default,
    }
}

/// xterm's reply to an OSC 4/10/11/12 query: `rgb:rrrr/gggg/bbbb`, each 8-bit
/// channel written twice (`toRgbString` with 16 bits).
pub fn rgb_string(c: Rgb) -> String {
    format!("rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}", c.r, c.g, c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_084_the_theme_is_registry_ts_terminal_theme() {
        assert_eq!(theme::BACKGROUND, Rgb { r: 30, g: 30, b: 32 });
        assert_eq!(theme::FOREGROUND, Rgb { r: 230, g: 230, b: 230 });
        assert_eq!(theme::ANSI[1], Rgb { r: 255, g: 107, b: 98 });
        assert_eq!(theme::ANSI[15], Rgb { r: 245, g: 245, b: 247 });
        assert_eq!(
            theme::SELECTION_BACKGROUND,
            Rgba {
                r: 10,
                g: 132,
                b: 255,
                a: 0x55
            }
        );
        assert_eq!(
            theme::SELECTION_INACTIVE_BACKGROUND,
            Rgba {
                r: 142,
                g: 142,
                b: 147,
                a: 0x40
            }
        );
    }

    #[test]
    fn the_256_palette_is_xterms_cube_and_greys() {
        assert_eq!(default_color(16), Rgb::hex(0x000000));
        assert_eq!(default_color(17), Rgb::hex(0x00005f));
        assert_eq!(default_color(196), Rgb::hex(0xff0000));
        assert_eq!(default_color(208), Rgb::hex(0xff8700));
        assert_eq!(default_color(231), Rgb::hex(0xffffff));
        assert_eq!(default_color(232), Rgb::hex(0x080808));
        assert_eq!(default_color(255), Rgb::hex(0xeeeeee));
    }

    #[test]
    fn term_078_colour_replies_double_each_channel() {
        assert_eq!(rgb_string(theme::BACKGROUND), "rgb:1e1e/1e1e/2020");
        assert_eq!(rgb_string(theme::FOREGROUND), "rgb:e6e6/e6e6/e6e6");
    }

    #[test]
    fn blend_rounds_as_math_round() {
        // #0a84ff at 0x55 over #1e1e20.
        assert_eq!(
            blend(theme::BACKGROUND, theme::SELECTION_BACKGROUND),
            Rgb { r: 23, g: 64, b: 106 }
        );
        assert_eq!(
            blend(theme::BACKGROUND, theme::SELECTION_INACTIVE_BACKGROUND),
            Rgb { r: 58, g: 58, b: 61 }
        );
    }

    #[test]
    fn term_079_bold_brightens_ansi_0_to_7_only() {
        let c = Colors::default();
        let bold = CellAttrs {
            bold: true,
            ..Default::default()
        };
        let r = resolve_cell_colors(CellColor::Indexed(1), CellColor::Default, bold, 'a', None, &c);
        assert_eq!(r.fg, theme::ANSI[9]);
        let r = resolve_cell_colors(CellColor::Indexed(9), CellColor::Default, bold, 'a', None, &c);
        assert_eq!(r.fg, theme::ANSI[9]);
        let r = resolve_cell_colors(CellColor::Indexed(100), CellColor::Default, bold, 'a', None, &c);
        assert_eq!(r.fg, default_color(100));
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, bold, 'a', None, &c);
        assert_eq!(r.fg, theme::FOREGROUND);
        assert!(r.bg_is_default);
    }

    #[test]
    fn term_079_inverse_swaps_and_brightens_the_swapped_colour() {
        let c = Colors::default();
        let inv = CellAttrs {
            inverse: true,
            ..Default::default()
        };
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, inv, 'a', None, &c);
        assert_eq!(
            (r.fg, r.bg, r.bg_is_default),
            (theme::BACKGROUND, theme::FOREGROUND, false)
        );
        let bold_inv = CellAttrs {
            inverse: true,
            bold: true,
            ..Default::default()
        };
        let r = resolve_cell_colors(CellColor::Indexed(2), CellColor::Indexed(1), bold_inv, 'a', None, &c);
        assert_eq!((r.fg, r.bg), (theme::ANSI[9], theme::ANSI[2]));
    }

    #[test]
    fn term_058_selection_replaces_or_blends_the_background() {
        let c = Colors::default();
        let none = CellAttrs::default();
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, none, 'a', Some(true), &c);
        assert_eq!(r.bg, c.selection_opaque(true));
        assert_eq!(r.fg, theme::FOREGROUND);
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, none, 'a', Some(false), &c);
        assert_eq!(r.bg, c.selection_opaque(false));
        let r = resolve_cell_colors(CellColor::Default, CellColor::Indexed(1), none, 'a', Some(true), &c);
        assert_eq!(r.bg, blend(theme::ANSI[1], c.selection_opaque(true).with_alpha(0x80)));
        // A selected inverse cell keeps its look: its glyph in the old background.
        let inv = CellAttrs {
            inverse: true,
            ..Default::default()
        };
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, inv, 'a', Some(true), &c);
        assert_eq!(r.fg, theme::BACKGROUND);
        assert_eq!(
            r.bg,
            blend(theme::FOREGROUND, c.selection_opaque(true).with_alpha(0x80))
        );
        // Box drawing takes the selection colour as its glyph colour.
        let r = resolve_cell_colors(CellColor::Default, CellColor::Default, none, '─', Some(true), &c);
        assert_eq!(
            r.fg,
            blend(theme::FOREGROUND, c.selection_opaque(true).with_alpha(0x80))
        );
    }
}
