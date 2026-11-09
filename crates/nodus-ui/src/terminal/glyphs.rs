//! Box drawing, block elements, shades and powerline symbols drawn as shapes
//! that fill their cell, as xterm.js 6's WebGL renderer draws them
//! (`addon-webgl/src/CustomGlyphs.ts`, `customGlyphs: true`, TERM-080):
//! the font never draws these, so neighbouring cells join with no gaps at
//! line-height 1.2. Claude Code's boxes and logo rely on it.
//!
//! Everything here is in device pixels, relative to the cell's top-left,
//! so the element only scales it to logical pixels.

/// A path command, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    Cubic([f32; 2], [f32; 2], [f32; 2]),
}

/// One shape of a glyph.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// A filled rectangle at a share of the glyph's colour (its edges may
    /// sit between device pixels: they're drawn antialiased, as a canvas
    /// `fillRect` is).
    Rect { x: f32, y: f32, w: f32, h: f32, alpha: f32 },
    /// A path, filled, or stroked `width` wide.
    Path { cmds: Vec<Cmd>, stroke: Option<f32> },
}

/// A glyph's shapes, and whether they're clipped to the cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomGlyph {
    pub prims: Vec<Prim>,
    pub clip: bool,
}

/// The shapes for `c` in a cell `cell_w` × `cell_h` device pixels, at
/// `dpr` device pixels per CSS pixel with the terminal's `font_size` (CSS
/// px); `None` when the font draws it.
pub fn custom_glyph(c: char, cell_w: f32, cell_h: f32, dpr: f32, font_size: f32) -> Option<CustomGlyph> {
    if let Some(boxes) = block_element(c) {
        let (xe, ye) = (cell_w / 8.0, cell_h / 8.0);
        let prims = boxes
            .iter()
            .map(|&(x, y, w, h)| Prim::Rect {
                x: f32::from(x) * xe,
                y: f32::from(y) * ye,
                w: f32::from(w) * xe,
                h: f32::from(h) * ye,
                alpha: 1.0,
            })
            .collect();
        return Some(CustomGlyph { prims, clip: false });
    }
    if let Some(pattern) = shade_pattern(c) {
        return Some(CustomGlyph {
            prims: pattern_rects(pattern, cell_w as usize, cell_h as usize),
            clip: false,
        });
    }
    if let Some(def) = box_drawing(c) {
        return Some(CustomGlyph {
            prims: box_drawing_prims(&def, cell_w, cell_h, dpr),
            clip: false,
        });
    }
    if let Some(def) = powerline(c) {
        return Some(CustomGlyph {
            prims: powerline_prims(&def, cell_w, cell_h, dpr, font_size),
            clip: true,
        });
    }
    None
}

/// Whether `c` is one of the characters drawn here.
pub fn is_custom(c: char) -> bool {
    block_element(c).is_some() || shade_pattern(c).is_some() || box_drawing(c).is_some() || powerline(c).is_some()
}

/// `blockElementDefinitions`: boxes in eighths of the cell.
fn block_element(c: char) -> Option<&'static [(u8, u8, u8, u8)]> {
    Some(match c {
        '▀' => &[(0, 0, 8, 4)],
        '▁' => &[(0, 7, 8, 1)],
        '▂' => &[(0, 6, 8, 2)],
        '▃' => &[(0, 5, 8, 3)],
        '▄' => &[(0, 4, 8, 4)],
        '▅' => &[(0, 3, 8, 5)],
        '▆' => &[(0, 2, 8, 6)],
        '▇' => &[(0, 1, 8, 7)],
        '█' => &[(0, 0, 8, 8)],
        '▉' => &[(0, 0, 7, 8)],
        '▊' => &[(0, 0, 6, 8)],
        '▋' => &[(0, 0, 5, 8)],
        '▌' => &[(0, 0, 4, 8)],
        '▍' => &[(0, 0, 3, 8)],
        '▎' => &[(0, 0, 2, 8)],
        '▏' => &[(0, 0, 1, 8)],
        '▐' => &[(4, 0, 4, 8)],
        '▔' => &[(0, 0, 8, 1)],
        '▕' => &[(7, 0, 1, 8)],
        '▖' => &[(0, 4, 4, 4)],
        '▗' => &[(4, 4, 4, 4)],
        '▘' => &[(0, 0, 4, 4)],
        '▙' => &[(0, 0, 4, 8), (0, 4, 8, 4)],
        '▚' => &[(0, 0, 4, 4), (4, 4, 4, 4)],
        '▛' => &[(0, 0, 4, 8), (4, 0, 4, 4)],
        '▜' => &[(0, 0, 8, 4), (4, 0, 4, 8)],
        '▝' => &[(4, 0, 4, 4)],
        '▞' => &[(4, 0, 4, 4), (0, 4, 4, 4)],
        '▟' => &[(4, 0, 4, 8), (0, 4, 8, 4)],
        '\u{1FB70}' => &[(1, 0, 1, 8)],
        '\u{1FB71}' => &[(2, 0, 1, 8)],
        '\u{1FB72}' => &[(3, 0, 1, 8)],
        '\u{1FB73}' => &[(4, 0, 1, 8)],
        '\u{1FB74}' => &[(5, 0, 1, 8)],
        '\u{1FB75}' => &[(6, 0, 1, 8)],
        '\u{1FB76}' => &[(0, 1, 8, 1)],
        '\u{1FB77}' => &[(0, 2, 8, 1)],
        '\u{1FB78}' => &[(0, 3, 8, 1)],
        '\u{1FB79}' => &[(0, 4, 8, 1)],
        '\u{1FB7A}' => &[(0, 5, 8, 1)],
        '\u{1FB7B}' => &[(0, 6, 8, 1)],
        '\u{1FB7C}' => &[(0, 0, 1, 8), (0, 7, 8, 1)],
        '\u{1FB7D}' => &[(0, 0, 1, 8), (0, 0, 8, 1)],
        '\u{1FB7E}' => &[(7, 0, 1, 8), (0, 0, 8, 1)],
        '\u{1FB7F}' => &[(7, 0, 1, 8), (0, 7, 8, 1)],
        '\u{1FB80}' => &[(0, 0, 8, 1), (0, 7, 8, 1)],
        '\u{1FB81}' => &[(0, 0, 8, 1), (0, 2, 8, 1), (0, 4, 8, 1), (0, 7, 8, 1)],
        '\u{1FB82}' => &[(0, 0, 8, 2)],
        '\u{1FB83}' => &[(0, 0, 8, 3)],
        '\u{1FB84}' => &[(0, 0, 8, 5)],
        '\u{1FB85}' => &[(0, 0, 8, 6)],
        '\u{1FB86}' => &[(0, 0, 8, 7)],
        '\u{1FB87}' => &[(6, 0, 2, 8)],
        '\u{1FB88}' => &[(5, 0, 3, 8)],
        '\u{1FB89}' => &[(3, 0, 5, 8)],
        '\u{1FB8A}' => &[(2, 0, 6, 8)],
        '\u{1FB8B}' => &[(1, 0, 7, 8)],
        '\u{1FB95}' => &[
            (0, 0, 2, 2),
            (4, 0, 2, 2),
            (2, 2, 2, 2),
            (6, 2, 2, 2),
            (0, 4, 2, 2),
            (4, 4, 2, 2),
            (2, 6, 2, 2),
            (6, 6, 2, 2),
        ],
        '\u{1FB96}' => &[
            (2, 0, 2, 2),
            (6, 0, 2, 2),
            (0, 2, 2, 2),
            (4, 2, 2, 2),
            (2, 4, 2, 2),
            (6, 4, 2, 2),
            (0, 6, 2, 2),
            (4, 6, 2, 2),
        ],
        '\u{1FB97}' => &[(0, 2, 8, 2), (0, 6, 8, 2)],
        _ => return None,
    })
}

/// `patternCharacterDefinitions`: device pixels set in a repeating pattern.
fn shade_pattern(c: char) -> Option<&'static [&'static [u8]]> {
    Some(match c {
        '░' => &[&[1, 0, 0, 0], &[0, 0, 0, 0], &[0, 0, 1, 0], &[0, 0, 0, 0]],
        '▒' => &[&[1, 0], &[0, 0], &[0, 1], &[0, 0]],
        '▓' => &[&[0, 1], &[1, 1], &[1, 0], &[1, 1]],
        _ => return None,
    })
}

/// The pattern tiled over the cell, as runs of set pixels per row.
fn pattern_rects(pattern: &[&[u8]], w: usize, h: usize) -> Vec<Prim> {
    let mut out = Vec::new();
    for y in 0..h {
        let row = pattern[y % pattern.len()];
        let mut x = 0;
        while x < w {
            if row[x % row.len()] == 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x % row.len()] != 0 {
                x += 1;
            }
            out.push(Prim::Rect {
                x: start as f32,
                y: y as f32,
                w: (x - start) as f32,
                h: 1.0,
                alpha: 1.0,
            });
        }
    }
    out
}

/// A box drawing character's instructions, per stroke weight (1 normal, 3 bold).
enum Instr {
    Fixed(&'static str),
    /// Double lines and rounded corners, given `xp` and `yp`.
    Func(fn(f32, f32) -> String),
}

const TOP_TO_BOTTOM: &str = "M.5,0 L.5,1";
const LEFT_TO_RIGHT: &str = "M0,.5 L1,.5";
const TOP_TO_RIGHT: &str = "M.5,0 L.5,.5 L1,.5";
const TOP_TO_LEFT: &str = "M.5,0 L.5,.5 L0,.5";
const LEFT_TO_BOTTOM: &str = "M0,.5 L.5,.5 L.5,1";
const RIGHT_TO_BOTTOM: &str = "M0.5,1 L.5,.5 L1,.5";
const MIDDLE_TO_TOP: &str = "M.5,.5 L.5,0";
const MIDDLE_TO_LEFT: &str = "M.5,.5 L0,.5";
const MIDDLE_TO_RIGHT: &str = "M.5,.5 L1,.5";
const MIDDLE_TO_BOTTOM: &str = "M.5,.5 L.5,1";
const T_TOP: &str = "M0,.5 L1,.5 M.5,.5 L.5,0";
const T_LEFT: &str = "M.5,0 L.5,1 M.5,.5 L0,.5";
const T_RIGHT: &str = "M.5,0 L.5,1 M.5,.5 L1,.5";
const T_BOTTOM: &str = "M0,.5 L1,.5 M.5,.5 L.5,1";
const CROSS: &str = "M0,.5 L1,.5 M.5,0 L.5,1";
const TWO_DASHES_HORIZONTAL: &str = "M.1,.5 L.4,.5 M.6,.5 L.9,.5";
const THREE_DASHES_HORIZONTAL: &str = "M.0667,.5 L.2667,.5 M.4,.5 L.6,.5 M.7333,.5 L.9333,.5";
const FOUR_DASHES_HORIZONTAL: &str = "M.05,.5 L.2,.5 M.3,.5 L.45,.5 M.55,.5 L.7,.5 M.8,.5 L.95,.5";
const TWO_DASHES_VERTICAL: &str = "M.5,.1 L.5,.4 M.5,.6 L.5,.9";
const THREE_DASHES_VERTICAL: &str = "M.5,.0667 L.5,.2667 M.5,.4 L.5,.6 M.5,.7333 L.5,.9333";
const FOUR_DASHES_VERTICAL: &str = "M.5,.05 L.5,.2 M.5,.3 L.5,.45 L.5,.55 M.5,.7 L.5,.95";
const TOP_TO_BOTTOM_MIDDLE_TO_RIGHT: &str = "M.5,0 L.5,1 M.5,.5 L1,.5";
const TOP_TO_BOTTOM_MIDDLE_TO_LEFT: &str = "M.5,0 L.5,1 M.5,.5 L0,.5";
const LEFT_TO_RIGHT_MIDDLE_TO_BOTTOM: &str = "M0,.5 L1,.5 M.5,.5 L.5,1";
const MIDDLE_TO_TOP_LEFT_TO_RIGHT: &str = "M.5,.5 L.5,0 M0,.5 L1,.5";

/// `boxDrawingDefinitions`, as (weight, instructions) pairs in xterm's order.
fn box_drawing(c: char) -> Option<Vec<(u8, Instr)>> {
    use Instr::{Fixed as F, Func};
    let n = |s| vec![(1, F(s))];
    let b = |s| vec![(3, F(s))];
    let nb = |normal, bold| vec![(1, F(normal)), (3, F(bold))];
    let d = |f: fn(f32, f32) -> String| vec![(1, Func(f))];
    Some(match c {
        '─' => n(LEFT_TO_RIGHT),
        '━' => b(LEFT_TO_RIGHT),
        '│' => n(TOP_TO_BOTTOM),
        '┃' => b(TOP_TO_BOTTOM),
        '┌' => n(RIGHT_TO_BOTTOM),
        '┏' => b(RIGHT_TO_BOTTOM),
        '┐' => n(LEFT_TO_BOTTOM),
        '┓' => b(LEFT_TO_BOTTOM),
        '└' => n(TOP_TO_RIGHT),
        '┗' => b(TOP_TO_RIGHT),
        '┘' => n(TOP_TO_LEFT),
        '┛' => b(TOP_TO_LEFT),
        '├' => n(T_RIGHT),
        '┣' => b(T_RIGHT),
        '┤' => n(T_LEFT),
        '┫' => b(T_LEFT),
        '┬' => n(T_BOTTOM),
        '┳' => b(T_BOTTOM),
        '┴' => n(T_TOP),
        '┻' => b(T_TOP),
        '┼' => n(CROSS),
        '╋' => b(CROSS),
        '╴' => n(MIDDLE_TO_LEFT),
        '╸' => b(MIDDLE_TO_LEFT),
        '╵' => n(MIDDLE_TO_TOP),
        '╹' => b(MIDDLE_TO_TOP),
        '╶' => n(MIDDLE_TO_RIGHT),
        '╺' => b(MIDDLE_TO_RIGHT),
        '╷' => n(MIDDLE_TO_BOTTOM),
        '╻' => b(MIDDLE_TO_BOTTOM),
        '═' => d(|_, yp| format!("M0,{} L1,{} M0,{} L1,{}", 0.5 - yp, 0.5 - yp, 0.5 + yp, 0.5 + yp)),
        '║' => d(|xp, _| format!("M{},0 L{},1 M{},0 L{},1", 0.5 - xp, 0.5 - xp, 0.5 + xp, 0.5 + xp)),
        '╒' => d(|_, yp| format!("M.5,1 L.5,{} L1,{} M.5,{} L1,{}", 0.5 - yp, 0.5 - yp, 0.5 + yp, 0.5 + yp)),
        '╓' => d(|xp, _| format!("M{},1 L{},.5 L1,.5 M{},.5 L{},1", 0.5 - xp, 0.5 - xp, 0.5 + xp, 0.5 + xp)),
        '╔' => d(|xp, yp| {
            format!(
                "M1,{} L{},{} L{},1 M1,{} L{},{} L{},1",
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp,
                0.5 + yp,
                0.5 + xp,
                0.5 + yp,
                0.5 + xp
            )
        }),
        '╕' => d(|_, yp| format!("M0,{} L.5,{} L.5,1 M0,{} L.5,{}", 0.5 - yp, 0.5 - yp, 0.5 + yp, 0.5 + yp)),
        '╖' => d(|xp, _| format!("M{},1 L{},.5 L0,.5 M{},.5 L{},1", 0.5 + xp, 0.5 + xp, 0.5 - xp, 0.5 - xp)),
        '╗' => d(|xp, yp| {
            format!(
                "M0,{} L{},{} L{},1 M0,{} L{},{} L{},1",
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 - xp,
                0.5 - yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp
            )
        }),
        '╘' => d(|_, yp| format!("M.5,0 L.5,{} L1,{} M.5,{} L1,{}", 0.5 + yp, 0.5 + yp, 0.5 - yp, 0.5 - yp)),
        '╙' => d(|xp, _| format!("M1,.5 L{},.5 L{},0 M{},.5 L{},0", 0.5 - xp, 0.5 - xp, 0.5 + xp, 0.5 + xp)),
        '╚' => d(|xp, yp| {
            format!(
                "M1,{} L{},{} L{},0 M1,{} L{},{} L{},0",
                0.5 - yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp,
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 - xp
            )
        }),
        '╛' => d(|_, yp| format!("M0,{} L.5,{} L.5,0 M0,{} L.5,{}", 0.5 + yp, 0.5 + yp, 0.5 - yp, 0.5 - yp)),
        '╜' => d(|xp, _| format!("M0,.5 L{},.5 L{},0 M{},.5 L{},0", 0.5 + xp, 0.5 + xp, 0.5 - xp, 0.5 - xp)),
        '╝' => d(|xp, yp| {
            format!(
                "M0,{} L{},{} L{},0 M0,{} L{},{} L{},0",
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp,
                0.5 + yp,
                0.5 + xp,
                0.5 + yp,
                0.5 + xp
            )
        }),
        '╞' => d(|_, yp| {
            format!(
                "{TOP_TO_BOTTOM} M.5,{} L1,{} M.5,{} L1,{}",
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 + yp
            )
        }),
        '╟' => d(|xp, _| {
            format!(
                "M{},0 L{},1 M{},0 L{},1 M{},.5 L1,.5",
                0.5 - xp,
                0.5 - xp,
                0.5 + xp,
                0.5 + xp,
                0.5 + xp
            )
        }),
        '╠' => d(|xp, yp| {
            format!(
                "M{},0 L{},1 M1,{} L{},{} L{},1 M1,{} L{},{} L{},0",
                0.5 - xp,
                0.5 - xp,
                0.5 + yp,
                0.5 + xp,
                0.5 + yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp
            )
        }),
        '╡' => d(|_, yp| {
            format!(
                "{TOP_TO_BOTTOM} M0,{} L.5,{} M0,{} L.5,{}",
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 + yp
            )
        }),
        '╢' => d(|xp, _| {
            format!(
                "M0,.5 L{},.5 M{},0 L{},1 M{},0 L{},1",
                0.5 - xp,
                0.5 - xp,
                0.5 - xp,
                0.5 + xp,
                0.5 + xp
            )
        }),
        '╣' => d(|xp, yp| {
            format!(
                "M{},0 L{},1 M0,{} L{},{} L{},1 M0,{} L{},{} L{},0",
                0.5 + xp,
                0.5 + xp,
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp
            )
        }),
        '╤' => d(|_, yp| {
            format!(
                "M0,{} L1,{} M0,{} L1,{} M.5,{} L.5,1",
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 + yp,
                0.5 + yp
            )
        }),
        '╥' => d(|xp, _| {
            format!(
                "{LEFT_TO_RIGHT} M{},.5 L{},1 M{},.5 L{},1",
                0.5 - xp,
                0.5 - xp,
                0.5 + xp,
                0.5 + xp
            )
        }),
        '╦' => d(|xp, yp| {
            format!(
                "M0,{} L1,{} M0,{} L{},{} L{},1 M1,{} L{},{} L{},1",
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 + xp,
                0.5 + yp,
                0.5 + xp
            )
        }),
        '╧' => d(|_, yp| {
            format!(
                "M.5,0 L.5,{} M0,{} L1,{} M0,{} L1,{}",
                0.5 - yp,
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 + yp
            )
        }),
        '╨' => d(|xp, _| {
            format!(
                "{LEFT_TO_RIGHT} M{},.5 L{},0 M{},.5 L{},0",
                0.5 - xp,
                0.5 - xp,
                0.5 + xp,
                0.5 + xp
            )
        }),
        '╩' => d(|xp, yp| {
            format!(
                "M0,{} L1,{} M0,{} L{},{} L{},0 M1,{} L{},{} L{},0",
                0.5 + yp,
                0.5 + yp,
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp
            )
        }),
        '╪' => d(|_, yp| {
            format!(
                "{TOP_TO_BOTTOM} M0,{} L1,{} M0,{} L1,{}",
                0.5 - yp,
                0.5 - yp,
                0.5 + yp,
                0.5 + yp
            )
        }),
        '╫' => d(|xp, _| {
            format!(
                "{LEFT_TO_RIGHT} M{},0 L{},1 M{},0 L{},1",
                0.5 - xp,
                0.5 - xp,
                0.5 + xp,
                0.5 + xp
            )
        }),
        '╬' => d(|xp, yp| {
            format!(
                "M0,{} L{},{} L{},1 M1,{} L{},{} L{},1 M0,{} L{},{} L{},0 M1,{} L{},{} L{},0",
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 - xp,
                0.5 + yp,
                0.5 + xp,
                0.5 + yp,
                0.5 + xp,
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 - xp,
                0.5 - yp,
                0.5 + xp,
                0.5 - yp,
                0.5 + xp
            )
        }),
        '╱' => n("M1,0 L0,1"),
        '╲' => n("M0,0 L1,1"),
        '╳' => n("M1,0 L0,1 M0,0 L1,1"),
        '╼' => nb(MIDDLE_TO_LEFT, MIDDLE_TO_RIGHT),
        '╽' => nb(MIDDLE_TO_TOP, MIDDLE_TO_BOTTOM),
        '╾' => nb(MIDDLE_TO_RIGHT, MIDDLE_TO_LEFT),
        '╿' => nb(MIDDLE_TO_BOTTOM, MIDDLE_TO_TOP),
        '┍' => nb(MIDDLE_TO_BOTTOM, MIDDLE_TO_RIGHT),
        '┎' => nb(MIDDLE_TO_RIGHT, MIDDLE_TO_BOTTOM),
        '┑' => nb(MIDDLE_TO_BOTTOM, MIDDLE_TO_LEFT),
        '┒' => nb(MIDDLE_TO_LEFT, MIDDLE_TO_BOTTOM),
        '┕' => nb(MIDDLE_TO_TOP, MIDDLE_TO_RIGHT),
        '┖' => nb(MIDDLE_TO_RIGHT, MIDDLE_TO_TOP),
        '┙' => nb(MIDDLE_TO_TOP, MIDDLE_TO_LEFT),
        '┚' => nb(MIDDLE_TO_LEFT, MIDDLE_TO_TOP),
        '┝' => nb(TOP_TO_BOTTOM, MIDDLE_TO_RIGHT),
        '┞' => nb(RIGHT_TO_BOTTOM, MIDDLE_TO_TOP),
        '┟' => nb(TOP_TO_RIGHT, MIDDLE_TO_BOTTOM),
        '┠' => nb(MIDDLE_TO_RIGHT, TOP_TO_BOTTOM),
        '┡' => nb(MIDDLE_TO_BOTTOM, TOP_TO_RIGHT),
        '┢' => nb(MIDDLE_TO_TOP, RIGHT_TO_BOTTOM),
        '┥' => nb(TOP_TO_BOTTOM, MIDDLE_TO_LEFT),
        '┦' => nb(LEFT_TO_BOTTOM, MIDDLE_TO_TOP),
        '┧' => nb(TOP_TO_LEFT, MIDDLE_TO_BOTTOM),
        '┨' => nb(MIDDLE_TO_LEFT, TOP_TO_BOTTOM),
        '┩' => nb(MIDDLE_TO_BOTTOM, TOP_TO_LEFT),
        '┪' => nb(MIDDLE_TO_TOP, LEFT_TO_BOTTOM),
        '┭' => nb(RIGHT_TO_BOTTOM, MIDDLE_TO_LEFT),
        '┮' => nb(LEFT_TO_BOTTOM, MIDDLE_TO_RIGHT),
        '┯' => nb(MIDDLE_TO_BOTTOM, LEFT_TO_RIGHT),
        '┰' => nb(LEFT_TO_RIGHT, MIDDLE_TO_BOTTOM),
        '┱' => nb(MIDDLE_TO_RIGHT, LEFT_TO_BOTTOM),
        '┲' => nb(MIDDLE_TO_LEFT, RIGHT_TO_BOTTOM),
        '┵' => nb(TOP_TO_RIGHT, MIDDLE_TO_LEFT),
        '┶' => nb(TOP_TO_LEFT, MIDDLE_TO_RIGHT),
        '┷' => nb(MIDDLE_TO_TOP, LEFT_TO_RIGHT),
        '┸' => nb(LEFT_TO_RIGHT, MIDDLE_TO_TOP),
        '┹' => nb(MIDDLE_TO_RIGHT, TOP_TO_LEFT),
        '┺' => nb(MIDDLE_TO_LEFT, TOP_TO_RIGHT),
        '┽' => nb(TOP_TO_BOTTOM_MIDDLE_TO_RIGHT, MIDDLE_TO_LEFT),
        '┾' => nb(TOP_TO_BOTTOM_MIDDLE_TO_LEFT, MIDDLE_TO_RIGHT),
        '┿' => nb(TOP_TO_BOTTOM, LEFT_TO_RIGHT),
        '╀' => nb(LEFT_TO_RIGHT_MIDDLE_TO_BOTTOM, MIDDLE_TO_TOP),
        '╁' => nb(MIDDLE_TO_TOP_LEFT_TO_RIGHT, MIDDLE_TO_BOTTOM),
        '╂' => nb(LEFT_TO_RIGHT, TOP_TO_BOTTOM),
        '╃' => nb(RIGHT_TO_BOTTOM, TOP_TO_LEFT),
        '╄' => nb(LEFT_TO_BOTTOM, TOP_TO_RIGHT),
        '╅' => nb(TOP_TO_RIGHT, LEFT_TO_BOTTOM),
        '╆' => nb(TOP_TO_LEFT, RIGHT_TO_BOTTOM),
        '╇' => nb(MIDDLE_TO_BOTTOM, MIDDLE_TO_TOP_LEFT_TO_RIGHT),
        '╈' => nb(MIDDLE_TO_TOP, LEFT_TO_RIGHT_MIDDLE_TO_BOTTOM),
        '╉' => nb(MIDDLE_TO_RIGHT, TOP_TO_BOTTOM_MIDDLE_TO_LEFT),
        '╊' => nb(MIDDLE_TO_LEFT, TOP_TO_BOTTOM_MIDDLE_TO_RIGHT),
        '╌' => n(TWO_DASHES_HORIZONTAL),
        '╍' => b(TWO_DASHES_HORIZONTAL),
        '┄' => n(THREE_DASHES_HORIZONTAL),
        '┅' => b(THREE_DASHES_HORIZONTAL),
        '┈' => n(FOUR_DASHES_HORIZONTAL),
        '┉' => b(FOUR_DASHES_HORIZONTAL),
        '╎' => n(TWO_DASHES_VERTICAL),
        '╏' => b(TWO_DASHES_VERTICAL),
        '┆' => n(THREE_DASHES_VERTICAL),
        '┇' => b(THREE_DASHES_VERTICAL),
        '┊' => n(FOUR_DASHES_VERTICAL),
        '┋' => b(FOUR_DASHES_VERTICAL),
        '╭' => d(|_, yp| {
            let y = 0.5 + yp / 0.15 * 0.5;
            format!("M.5,1 L.5,{y} C.5,{y},.5,.5,1,.5")
        }),
        '╮' => d(|_, yp| {
            let y = 0.5 + yp / 0.15 * 0.5;
            format!("M.5,1 L.5,{y} C.5,{y},.5,.5,0,.5")
        }),
        '╯' => d(|_, yp| {
            let y = 0.5 - yp / 0.15 * 0.5;
            format!("M.5,0 L.5,{y} C.5,{y},.5,.5,0,.5")
        }),
        '╰' => d(|_, yp| {
            let y = 0.5 - yp / 0.15 * 0.5;
            format!("M.5,0 L.5,{y} C.5,{y},.5,.5,1,.5")
        }),
        _ => return None,
    })
}

/// One parsed instruction: its letter and numbers.
fn parse(instructions: &str) -> Vec<(char, Vec<f32>)> {
    instructions
        .split(' ')
        .filter_map(|ins| {
            let mut chars = ins.chars();
            let kind = chars.next()?;
            let args: Vec<f32> = chars.as_str().split(',').map(|a| a.parse::<f32>().unwrap_or(0.0)).collect();
            // xterm skips an instruction missing either of its first two numbers.
            (args.len() >= 2).then_some((kind, args))
        })
        .collect()
}

/// `translateArgs`: 0–1 to the cell, with box drawing's rounding to the
/// nearest half pixel (clamped to the cell) so lines are crisp.
fn translate(args: &[f32], cell_w: f32, cell_h: f32, clamp: bool, dpr: f32, left: f32, right: f32) -> Vec<f32> {
    let js_round = |v: f32| (v + 0.5).floor();
    args.iter()
        .enumerate()
        .map(|(i, &a)| {
            if i % 2 == 0 {
                let mut x = a * (cell_w - left * dpr - right * dpr);
                if clamp && x != 0.0 {
                    x = (js_round(x + 0.5) - 0.5).clamp(0.0, cell_w);
                }
                x + left * dpr
            } else {
                let mut y = a * cell_h;
                if clamp && y != 0.0 {
                    y = (js_round(y + 0.5) - 0.5).clamp(0.0, cell_h);
                }
                y
            }
        })
        .collect()
}

fn to_cmds(parsed: &[(char, Vec<f32>)], map: impl Fn(&[f32]) -> Vec<f32>) -> Vec<Cmd> {
    parsed
        .iter()
        .filter_map(|(kind, args)| {
            let p = map(args);
            match kind {
                'M' => Some(Cmd::Move(p[0], p[1])),
                'L' => Some(Cmd::Line(p[0], p[1])),
                'C' if p.len() >= 6 => Some(Cmd::Cubic([p[0], p[1]], [p[2], p[3]], [p[4], p[5]])),
                _ => None,
            }
        })
        .collect()
}

/// `drawBoxDrawingChar`: each weight's path stroked `dpr × weight` wide.
/// Paths made only of horizontal and vertical lines become rectangles
/// (butt ends, mitred corners), which is what a canvas stroke covers.
fn box_drawing_prims(def: &[(u8, Instr)], cell_w: f32, cell_h: f32, dpr: f32) -> Vec<Prim> {
    let mut out = Vec::new();
    for (weight, instr) in def {
        let width = dpr * f32::from(*weight);
        let text = match instr {
            Instr::Fixed(s) => (*s).to_owned(),
            Instr::Func(f) => f(0.15, 0.15 / cell_h * cell_w),
        };
        let cmds = to_cmds(&parse(&text), |a| translate(a, cell_w, cell_h, true, dpr, 0.0, 0.0));
        match axis_aligned_rects(&cmds, width) {
            Some(rects) => out.extend(rects),
            None => out.push(Prim::Path {
                cmds,
                stroke: Some(width),
            }),
        }
    }
    out
}

/// The rectangles a stroke of `width` covers along `cmds`, if every
/// segment is horizontal or vertical.
fn axis_aligned_rects(cmds: &[Cmd], width: f32) -> Option<Vec<Prim>> {
    let half = width / 2.0;
    let mut rects = Vec::new();
    // Subpaths as point lists.
    let mut paths: Vec<Vec<(f32, f32)>> = Vec::new();
    for cmd in cmds {
        match *cmd {
            Cmd::Move(x, y) => paths.push(vec![(x, y)]),
            Cmd::Line(x, y) => paths.last_mut()?.push((x, y)),
            Cmd::Cubic(..) => return None,
        }
    }
    for points in &paths {
        for (i, pair) in points.windows(2).enumerate() {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            if x0 != x1 && y0 != y1 {
                return None;
            }
            // A joint with the next or previous segment is mitred: extend
            // into the corner by half the width.
            let start_joint = i > 0;
            let end_joint = i + 2 < points.len();
            let (a, b) = if x0 == x1 { (y0, y1) } else { (x0, x1) };
            let dir = if b >= a { 1.0 } else { -1.0 };
            let a = a - if start_joint { half * dir } else { 0.0 };
            let b = b + if end_joint { half * dir } else { 0.0 };
            let (lo, hi) = (a.min(b), a.max(b));
            let rect = if x0 == x1 {
                Prim::Rect {
                    x: x0 - half,
                    y: lo,
                    w: width,
                    h: hi - lo,
                    alpha: 1.0,
                }
            } else {
                Prim::Rect {
                    x: lo,
                    y: y0 - half,
                    w: hi - lo,
                    h: width,
                    alpha: 1.0,
                }
            };
            rects.push(rect);
        }
    }
    Some(rects)
}

/// A powerline symbol: its path, filled or stroked, and the padding its
/// path leaves (`IVectorShape`).
struct Powerline {
    d: &'static str,
    stroke: bool,
    left: f32,
    right: f32,
}

/// `powerlineDefinitions`.
fn powerline(c: char) -> Option<Powerline> {
    let p = |d, stroke, left, right| Powerline { d, stroke, left, right };
    Some(match c {
        '\u{E0A0}' => p(
            "M.3,1 L.03,1 L.03,.88 C.03,.82,.06,.78,.11,.73 C.15,.7,.2,.68,.28,.65 L.43,.6 C.49,.58,.53,.56,.56,.53 C.59,.5,.6,.47,.6,.43 L.6,.27 L.4,.27 L.69,.1 L.98,.27 L.78,.27 L.78,.46 C.78,.52,.76,.56,.72,.61 C.68,.66,.63,.67,.56,.7 L.48,.72 C.42,.74,.38,.76,.35,.78 C.32,.8,.31,.84,.31,.88 L.31,1 M.3,.5 L.03,.59 L.03,.09 L.3,.09 L.3,.655",
            false,
            0.0,
            0.0,
        ),
        '\u{E0A1}' => p(
            "M.7,.4 L.7,.47 L.2,.47 L.2,.03 L.355,.03 L.355,.4 L.705,.4 M.7,.5 L.86,.5 L.86,.95 L.69,.95 L.44,.66 L.46,.86 L.46,.95 L.3,.95 L.3,.49 L.46,.49 L.71,.78 L.69,.565 L.69,.5",
            false,
            0.0,
            0.0,
        ),
        '\u{E0A2}' => p(
            "M.25,.94 C.16,.94,.11,.92,.11,.87 L.11,.53 C.11,.48,.15,.455,.23,.45 L.23,.3 C.23,.25,.26,.22,.31,.19 C.36,.16,.43,.15,.51,.15 C.59,.15,.66,.16,.71,.19 C.77,.22,.79,.26,.79,.3 L.79,.45 C.87,.45,.91,.48,.91,.53 L.91,.87 C.91,.92,.86,.94,.77,.94 L.24,.94 M.53,.2 C.49,.2,.45,.21,.42,.23 C.39,.25,.38,.27,.38,.3 L.38,.45 L.68,.45 L.68,.3 C.68,.27,.67,.25,.64,.23 C.61,.21,.58,.2,.53,.2 M.58,.82 L.58,.66 C.63,.65,.65,.63,.65,.6 C.65,.58,.64,.57,.61,.56 C.58,.55,.56,.54,.52,.54 C.48,.54,.46,.55,.43,.56 C.4,.57,.39,.59,.39,.6 C.39,.63,.41,.64,.46,.66 L.46,.82 L.57,.82",
            false,
            0.0,
            0.0,
        ),
        '\u{E0B0}' => p("M0,0 L1,.5 L0,1", false, 0.0, 2.0),
        '\u{E0B1}' => p("M-1,-.5 L1,.5 L-1,1.5", true, 1.0, 1.0),
        '\u{E0B2}' => p("M1,0 L0,.5 L1,1", false, 2.0, 0.0),
        '\u{E0B3}' => p("M2,-.5 L0,.5 L2,1.5", true, 1.0, 1.0),
        '\u{E0B4}' => p("M0,0 L0,1 C0.552,1,1,0.776,1,.5 C1,0.224,0.552,0,0,0", false, 0.0, 1.0),
        '\u{E0B5}' => p("M.2,1 C.422,1,.8,.826,.78,.5 C.8,.174,0.422,0,.2,0", true, 0.0, 1.0),
        '\u{E0B6}' => p("M1,0 L1,1 C0.448,1,0,0.776,0,.5 C0,0.224,0.448,0,1,0", false, 1.0, 0.0),
        '\u{E0B7}' => p("M.8,1 C0.578,1,0.2,.826,.22,.5 C0.2,0.174,0.578,0,0.8,0", true, 1.0, 0.0),
        '\u{E0B8}' => p("M-.5,-.5 L1.5,1.5 L-.5,1.5", false, 0.0, 0.0),
        '\u{E0B9}' | '\u{E0BF}' => p("M-.5,-.5 L1.5,1.5", true, 1.0, 1.0),
        '\u{E0BA}' => p("M1.5,-.5 L-.5,1.5 L1.5,1.5", false, 0.0, 0.0),
        '\u{E0BC}' => p("M1.5,-.5 L-.5,1.5 L-.5,-.5", false, 0.0, 0.0),
        '\u{E0BD}' | '\u{E0BB}' => p("M1.5,-.5 L-.5,1.5", true, 1.0, 1.0),
        '\u{E0BE}' => p("M-.5,-.5 L1.5,1.5 L1.5,-.5", false, 0.0, 0.0),
        _ => return None,
    })
}

/// `drawPowerlineChar`: the path scaled into the cell (less its padding),
/// stroked `fontSize / 12` CSS px wide or filled, clipped to the cell.
fn powerline_prims(def: &Powerline, cell_w: f32, cell_h: f32, dpr: f32, font_size: f32) -> Vec<Prim> {
    let css_line = font_size / 12.0;
    let (left, right) = (def.left * css_line / 2.0, def.right * css_line / 2.0);
    let cmds = to_cmds(&parse(def.d), |a| translate(a, cell_w, cell_h, false, dpr, left, right));
    vec![Prim::Path {
        cmds,
        stroke: def.stroke.then_some(dpr * css_line),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(c: char) -> Vec<(f32, f32, f32, f32)> {
        custom_glyph(c, 15.0, 36.0, 2.0, 13.0)
            .unwrap()
            .prims
            .into_iter()
            .map(|p| match p {
                Prim::Rect { x, y, w, h, .. } => (x, y, w, h),
                Prim::Path { .. } => panic!("{c} is a path"),
            })
            .collect()
    }

    #[test]
    fn term_080_a_light_horizontal_line_spans_the_cell_at_its_middle() {
        // Centred on 18.5, 2 device px wide: rows 17.5..19.5, joining both neighbours.
        assert_eq!(rects('─'), vec![(0.0, 17.5, 15.0, 2.0)]);
        // Bold is three times as wide.
        assert_eq!(rects('━'), vec![(0.0, 15.5, 15.0, 6.0)]);
        // Vertical: x 7.5, top to bottom.
        assert_eq!(rects('│'), vec![(6.5, 0.0, 2.0, 36.0)]);
    }

    #[test]
    fn term_080_corners_are_mitred() {
        // ┌: from the bottom up to the middle, then right; both reach into the corner.
        assert_eq!(
            rects('┌'),
            vec![(6.5, 17.5, 2.0, 18.5), (6.5, 17.5, 8.5, 2.0)]
        );
    }

    #[test]
    fn term_080_block_elements_fill_eighths() {
        assert_eq!(rects('█'), vec![(0.0, 0.0, 15.0, 36.0)]);
        assert_eq!(rects('▄'), vec![(0.0, 18.0, 15.0, 18.0)]);
        assert_eq!(rects('▏'), vec![(0.0, 0.0, 1.875, 36.0)]);
    }

    #[test]
    fn term_080_shades_are_pixel_patterns() {
        let light = rects('░');
        // One pixel in sixteen: rows 0 and 2 of every 4 have pixels every 4 columns.
        assert_eq!(light.iter().filter(|r| r.1 == 0.0).count(), 4);
        assert!(light.iter().all(|r| r.2 == 1.0 && r.3 == 1.0));
        assert_eq!(light.len(), 4 * 9 + 4 * 9);
    }

    #[test]
    fn term_080_curves_diagonals_and_powerline_are_paths() {
        let round = custom_glyph('╭', 15.0, 36.0, 2.0, 13.0).unwrap();
        assert!(matches!(&round.prims[0], Prim::Path { stroke: Some(w), .. } if *w == 2.0));
        let tri = custom_glyph('\u{E0B0}', 15.0, 36.0, 2.0, 13.0).unwrap();
        assert!(tri.clip);
        match &tri.prims[0] {
            Prim::Path { cmds, stroke: None } => {
                // Right padding of 2 × (13/12)/2 CSS px narrows the triangle.
                assert_eq!(cmds[0], Cmd::Move(0.0, 0.0));
                let Cmd::Line(x, y) = cmds[1] else { panic!() };
                assert!((x - (15.0 - 2.0 * 13.0 / 12.0) ).abs() < 1e-4 && y == 18.0);
            }
            other => panic!("{other:?}"),
        }
        assert!(custom_glyph('a', 15.0, 36.0, 2.0, 13.0).is_none());
        assert!(custom_glyph('⎿', 15.0, 36.0, 2.0, 13.0).is_none());
        assert!(is_custom('╯') && !is_custom('⏺'));
    }
}
