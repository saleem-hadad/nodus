//! The chat's look (CHAT-035…CHAT-041, MOTION-039…MOTION-043): Claude's
//! mark, the dots and words of the working line, the light through them,
//! and the moves rows make as they arrive. It stays in the theme's
//! neutrals; motion and depth do the lifting, not colour.

use std::ops::Range;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::base::text::TextViewMotion;
use gpui::{
    AnyElement, App, Bounds, Div, HighlightStyle, IntoElement, ParentElement, Pixels, Rgba, SharedString, Styled,
    Window, canvas, div, fill, point, px, size,
};

use crate::components::icon::{Icon, icon};
use crate::theme::tokens::{Tokens, hex};
use crate::theme::{ai, motion};

/// Claude's mark: the sparkle on a quiet round tile.
pub(crate) fn avatar(size: f32, t: &Tokens) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.bg_sunken)
        .border_1()
        .border_color(t.separator)
        .child(ai::sparkle(size * 0.56, 2.0, t.text_secondary, 0.))
}

/// Ms on the clock the chat's loaders run on, one for them all so they keep
/// in step; asks for the next frame. Held at 0 in parity shots and under
/// reduced motion.
pub(crate) fn clock(window: &mut Window, cx: &App) -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    if crate::parity::frozen(cx) || cx.reduce_motion() {
        return 0.;
    }
    window.request_animation_frame();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}

/// The cells of the 3×3 grid, row by row, in the order the snake's head
/// visits them: round the ring clockwise from the top left, through the
/// middle, and back to the start.
const SNAKE: [usize; 9] = [0, 1, 2, 5, 8, 7, 6, 3, 4];
/// One lap of the snake.
const SNAKE_MS: f64 = 1600.;
/// How many cells the snake's tail trails behind its head.
const TAIL: f64 = 3.4;

/// How lit grid cell `cell` is (0…1) with the snake's head `head` cells
/// along its path: full at the head, fading over the tail behind it and
/// over the one cell ahead, so the light glides from dot to dot.
fn snake_light(cell: usize, head: f64) -> f32 {
    let at = SNAKE.iter().position(|&c| c == cell).unwrap_or(0) as f64;
    let n = SNAKE.len() as f64;
    let behind = (head - at).rem_euclid(n);
    let ahead = (at - head).rem_euclid(n);
    let tail = if behind < TAIL {
        (1. - behind / TAIL).powf(1.6)
    } else {
        0.
    };
    let front = if ahead < 1. { (1. - ahead).powi(2) } else { 0. };
    tail.max(front) as f32
}

/// The loader (MOTION-039): a 3×3 grid of dots `size` px square in
/// `color`, a light winding through it like a snake, a lap every 1.6s, at
/// `ms` on the [`clock`].
pub(crate) fn dots(size: f32, ms: f64, color: Rgba) -> Div {
    let head = (ms / SNAKE_MS).rem_euclid(1.) * SNAKE.len() as f64;
    let (dot, gap) = (size * 0.2, size * 0.1);
    let row = move |r: usize| {
        div().flex().gap(px(gap)).children((0..3).map(move |c| {
            let lit = snake_light(r * 3 + c, head);
            div()
                .flex_none()
                .size(px(dot))
                .rounded_full()
                .bg(color)
                .opacity(0.18 + 0.82 * lit)
                .scale(0.7 + 0.42 * lit)
        }))
    };
    div()
        .flex_none()
        .size(px(size))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(gap))
        .children((0..3).map(row))
}

/// What the working line says Claude does with the tool `name` on `detail`
/// (MOTION-040): a verb and what it works on, cut to 40 characters.
pub(crate) fn doing(name: &str, detail: &str) -> String {
    let line = detail.lines().next().unwrap_or("").trim();
    let what = clip(line, 40);
    let verb = |verb: &str, alone: &str| {
        if what.is_empty() {
            alone.to_owned()
        } else {
            format!("{verb} {what}")
        }
    };
    let quoted = |verb: &str, alone: &str| {
        if what.is_empty() {
            alone.to_owned()
        } else {
            format!("{verb} “{what}”")
        }
    };
    match name {
        "Read" | "NotebookRead" => verb("Reading", "Reading a file"),
        "Edit" | "MultiEdit" | "NotebookEdit" => verb("Editing", "Editing a file"),
        "Write" => verb("Writing", "Writing a file"),
        "Bash" | "BashOutput" => verb("Running", "Running a command"),
        "Grep" => quoted("Searching for", "Searching"),
        "Glob" | "LS" => verb("Finding", "Finding files"),
        "WebFetch" => {
            let host = line.split("://").nth(1).unwrap_or(line).split('/').next().unwrap_or("");
            if host.is_empty() {
                "Reading a page".to_owned()
            } else {
                format!("Reading {}", clip(host, 40))
            }
        }
        "WebSearch" => quoted("Searching the web for", "Searching the web"),
        "Task" | "Agent" => verb("Agent:", "Running an agent"),
        "TodoWrite" => "Planning".to_owned(),
        "ToolSearch" => "Finding tools".to_owned(),
        "Skill" => verb("Using", "Using a skill"),
        _ => format!("Using {}", name.replace('_', " ")),
    }
}

/// `text` cut to `max` characters, an ellipsis ending what was cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// A turn's time as the working line shows it: `14s`, `2m 5s`.
pub(crate) fn elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// The working line's words and the ones they took over from (MOTION-040).
pub(crate) struct Roll {
    pub(crate) now: SharedString,
    pub(crate) was: Option<SharedString>,
    /// When `now` took over.
    pub(crate) at: Option<Instant>,
}

impl Roll {
    pub(crate) fn new(words: &str) -> Self {
        Self {
            now: SharedString::from(words.to_owned()),
            was: None,
            at: None,
        }
    }

    /// Shows `words`, rolling them in if they aren't the ones showing.
    pub(crate) fn show(&mut self, words: &str) {
        if self.now != words {
            self.was = Some(std::mem::replace(&mut self.now, SharedString::from(words.to_owned())));
            self.at = Some(Instant::now());
        }
    }
}

/// How long Claude's reply takes to arrive word by word (MOTION-041):
/// 24ms a word, between 300ms and 1.4s.
pub(crate) fn words_duration(text: &str) -> Duration {
    Duration::from_millis((text.split_whitespace().count() as u64 * 24).clamp(300, 1400))
}

/// The first `share` (0…1) of `text`'s words, cut at a word's end.
pub(crate) fn words(text: &str, share: f32) -> &str {
    if share >= 1. {
        return text;
    }
    let want = (text.split_whitespace().count() as f32 * share.max(0.)).ceil() as usize;
    if want == 0 {
        return "";
    }
    let mut seen = 0;
    let mut in_word = false;
    for (i, ch) in text.char_indices() {
        if !ch.is_whitespace() {
            in_word = true;
        } else if in_word {
            in_word = false;
            seen += 1;
            if seen == want {
                return &text[..i];
            }
        }
    }
    text
}

/// How the reply's words settle in as they arrive (MOTION-041): each fades
/// up over 420ms, 28ms behind the one before it.
pub(crate) fn settle() -> TextViewMotion {
    TextViewMotion::default()
        .with_stream_fade(Duration::from_millis(420))
        .with_stream_fade_stagger(Duration::from_millis(28))
}

/// One lap of the light round the composer: slow, so it doesn't pull the eye.
const LIGHT_MS: f64 = 4500.;
/// How strong the light is at its head, of `color`'s own alpha: barely more
/// than the field's border, a hint that Claude is busy and no more.
const LIGHT_STRENGTH: f32 = 0.3;

/// A very faint light circling the composer while Claude works (MOTION-043):
/// a head of `color` and a tail fading behind it, along the edge of a field
/// with corners of `radius`, `lit` (0…1) of full strength. It fills the
/// field it's put in. None in parity shots or under reduced motion.
pub(crate) fn light(radius: f32, lit: f32, color: Rgba, window: &mut Window, cx: &App) -> Option<AnyElement> {
    if crate::parity::frozen(cx) || cx.reduce_motion() {
        return None;
    }
    let head = (clock(window, cx) / LIGHT_MS).rem_euclid(1.) as f32;
    Some(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint_light(bounds, radius, head, lit, color, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
}

/// Paints the light as a run of small dots along the middle of the field's
/// 1px border, half a pixel out from the padding box `bounds`: a soft front
/// a fifth as long as the tail, which reaches at most 15% of the way round,
/// or 180px.
fn paint_light(bounds: Bounds<Pixels>, radius: f32, head: f32, lit: f32, color: Rgba, window: &mut Window) {
    const DOT: f32 = 1.2;
    let edge = bounds.dilate(px(0.5));
    let (w, h) = (f32::from(edge.size.width), f32::from(edge.size.height));
    let r = (radius - 0.5).max(0.).min(w / 2.).min(h / 2.);
    let around = 2. * (w + h) - (8. - 2. * std::f32::consts::PI) * r;
    if around <= 0. {
        return;
    }
    let tail = (0.15f32).min(180. / around);
    let front = tail * 0.2;
    let count = ((tail + front) * around).ceil().min(480.) as usize;
    for k in 0..count {
        // How far behind the head (ahead of it when negative), round the field.
        let back = k as f32 / count as f32 * (tail + front) - front;
        let strength = if back < 0. {
            let x = 1. + back / front;
            x * x * (3. - 2. * x)
        } else {
            (1. - back / tail).powi(2)
        };
        let at = ai::outline(edge, r, head - back);
        let alpha = color.a * LIGHT_STRENGTH * lit * strength;
        window.paint_quad(
            fill(
                Bounds::new(point(at.x - px(DOT / 2.), at.y - px(DOT / 2.)), size(px(DOT), px(DOT))),
                Rgba { a: alpha, ..color },
            )
            .corner_radii(px(DOT / 2.)),
        );
    }
}

/// A light passing through `text`, in its own colour: each letter faint at
/// rest, `color` as the light passes (MOTION-001's sweep without its tint).
pub(crate) fn shimmer(text: &str, ms: f64, color: Rgba) -> Vec<(Range<usize>, HighlightStyle)> {
    let faint = Rgba {
        a: color.a * 0.45,
        ..color
    };
    let n = text.chars().count().max(1);
    text.char_indices()
        .enumerate()
        .map(|(i, (b, ch))| {
            let g = ai::sweep_at((i as f64 + 0.5) / n as f64, ms / ai::SWEEP_MS);
            let tint = match g {
                g if g < 0.4 => ai::lerp(faint, color, (g / 0.4) as f32),
                g if g < 0.6 => color,
                g => ai::lerp(color, faint, ((g - 0.6) / 0.4).min(1.0) as f32),
            };
            (
                b..b + ch.len_utf8(),
                HighlightStyle {
                    color: Some(Rgba { a: 1.0, ..tint }.into()),
                    fade_out: Some(1.0 - tint.a),
                    ..HighlightStyle::default()
                },
            )
        })
        .collect()
}

/// The composer's field: white on the light theme, a step up from the
/// chat on the dark one.
pub(crate) fn field_bg(t: &Tokens) -> Rgba {
    if t.dark { hex(0x323236) } else { t.bg_elevated }
}

/// The purple of code's keywords and of searches (CHAT-047), the page
/// theme's `--nodus-purple`; not Claude's sparkle.
pub(crate) fn purple(t: &Tokens) -> Rgba {
    if t.dark { hex(0xa371f7) } else { hex(0x8250df) }
}

/// A tool's glyph colour by what it does (CHAT-047): reading blue, searching
/// purple, editing orange, running green, anything else quiet.
pub(crate) fn tool_tone(name: &str, t: &Tokens) -> Rgba {
    match name {
        "Read" | "NotebookRead" | "WebFetch" | "WebSearch" => t.accent,
        "Grep" | "Glob" | "LS" | "ToolSearch" => purple(t),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => t.status_working,
        "Bash" | "BashOutput" | "KillShell" | "KillBash" => t.diff_add,
        _ => t.text_tertiary,
    }
}

/// The user's bubble (CHAT-037, CHAT-047): a light wash of the accent.
pub(crate) fn your_bubble(t: &Tokens) -> Rgba {
    Rgba {
        a: if t.dark { 0.2 } else { 0.11 },
        ..t.accent
    }
}

/// A tool's glyph by its name, as Claude Code names it.
pub(crate) fn tool_icon(name: &str) -> Icon {
    match name {
        "Bash" | "BashOutput" | "KillShell" | "KillBash" => Icon::SquareTerminal,
        "Read" | "NotebookRead" => Icon::Eye,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Icon::Pencil,
        "Grep" | "Glob" | "LS" | "ToolSearch" => Icon::Search,
        "WebFetch" | "WebSearch" => Icon::Globe,
        "Task" | "Agent" => Icon::Bot,
        "TodoWrite" => Icon::Check,
        _ => Icon::Sparkles,
    }
}

/// How far into its arrival a row is (0…1, linear): rows that came in
/// while the chat was up move in, and rows it opened with, parity shots and
/// reduced motion are already there. Asks for frames until it's done.
pub(crate) fn arrival(born: Option<Instant>, duration: Duration, window: &mut Window, cx: &App) -> f32 {
    let Some(born) = born else {
        return 1.0;
    };
    if crate::parity::frozen(cx) || cx.reduce_motion() {
        return 1.0;
    }
    let t = born.elapsed().as_secs_f32() / duration.as_secs_f32();
    if t >= 1.0 {
        return 1.0;
    }
    window.request_animation_frame();
    t.max(0.0)
}

/// A row easing in: faded from nothing, risen from `dy` below, at `e`
/// (eased progress).
pub(crate) fn rise(el: Div, e: f32, dy: f32) -> Div {
    if e >= 1.0 {
        return el;
    }
    el.opacity(e).translate(px(0.), px(dy * (1.0 - e)))
}

/// A tool's mark at the end of its line: a green tick, or a red cross. `p`
/// (0…1) into its arrival, it springs in from a fifth of its size, turned
/// back 25° (MOTION-042).
pub(crate) fn outcome(failed: bool, size: f32, p: f32, t: &Tokens) -> Div {
    let (glyph, color) = if failed {
        (Icon::X, t.status_waiting)
    } else {
        (Icon::Check, t.diff_add)
    };
    let mark = div().flex_none().child(icon(glyph, size, 2.4, color));
    if p >= 1.0 {
        return mark;
    }
    let spring = motion::ease_spring(p);
    mark.opacity(motion::ease_out(p))
        .scale(0.2 + 0.8 * spring)
        .rotate(gpui::radians((-25.0 * (1.0 - spring)).to_radians()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lights_the_snake_from_its_head_back() {
        // The head on the top-left cell: it's full, the cells behind it on
        // the path (the middle, then the left) fade, and the far ones rest.
        assert!((snake_light(0, 0.) - 1.).abs() < 1e-6);
        assert!(snake_light(4, 0.) > snake_light(3, 0.));
        assert!(snake_light(3, 0.) > 0.);
        assert_eq!(snake_light(8, 0.), 0.);
        // Halfway to the next cell, the light is on both, more on the one it left.
        let (left, next) = (snake_light(0, 0.5), snake_light(1, 0.5));
        assert!(left > next && next > 0.);
        // A lap later it's where it started.
        assert_eq!(snake_light(5, 3.), snake_light(5, 12.));
    }

    #[test]
    fn says_what_the_tool_does() {
        assert_eq!(doing("Read", "look.rs"), "Reading look.rs");
        assert_eq!(doing("Edit", ""), "Editing a file");
        assert_eq!(doing("Grep", "spinner"), "Searching for “spinner”");
        assert_eq!(doing("Bash", "cargo test\necho done"), "Running cargo test");
        assert_eq!(doing("WebFetch", "https://docs.rs/gpui/latest"), "Reading docs.rs");
        assert_eq!(doing("draw_on_board", ""), "Using draw on board");
        let long = doing("Bash", &"x".repeat(80));
        assert_eq!(long.chars().count(), "Running ".len() + 40);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn times_the_turn() {
        assert_eq!(elapsed(0), "0s");
        assert_eq!(elapsed(59), "59s");
        assert_eq!(elapsed(125), "2m 5s");
    }

    #[test]
    fn rolls_only_when_the_words_change() {
        let mut roll = Roll::new("Thinking…");
        roll.show("Thinking…");
        assert!(roll.was.is_none() && roll.at.is_none());
        roll.show("Reading look.rs");
        assert_eq!(roll.now.as_ref(), "Reading look.rs");
        assert_eq!(roll.was.as_ref().map(AsRef::as_ref), Some("Thinking…"));
        assert!(roll.at.is_some());
    }

    #[test]
    fn cuts_words_at_their_end() {
        let text = "Swapped the  avatar\nfor dots.";
        assert_eq!(words(text, 0.), "");
        assert_eq!(words(text, 0.2), "Swapped");
        assert_eq!(words(text, 0.6), "Swapped the  avatar");
        assert_eq!(words(text, 0.99), text);
        assert_eq!(words(text, 1.), text);
        assert_eq!(words_duration("one two"), Duration::from_millis(300));
        assert_eq!(words_duration(&"word ".repeat(1000)), Duration::from_millis(1400));
    }
}
