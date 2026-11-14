//! The chat's look (CHAT-035…CHAT-041): Claude's mark, the light through the
//! working line, and the moves rows make as they arrive. It stays in the
//! theme's neutrals; motion and depth do the lifting, not colour.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    App, Div, HighlightStyle, ParentElement, Rgba, Styled, Window, div, linear_color_stop, linear_gradient, px,
};

use crate::components::icon::{Icon, icon};
use crate::theme::ai;
use crate::theme::tokens::{Tokens, hex, rgba};

/// Claude's mark: the sparkle on a quiet round tile. While Claude works
/// (`working` is ms into its turn) the sparkle twinkles.
pub(crate) fn avatar(size: f32, working: Option<f64>, t: &Tokens) -> Div {
    let tw = working.map_or(0., |ms| ai::twinkle(ms / ai::TWINKLE_MS));
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
        .child(ai::sparkle(size * 0.56, 2.0, t.text_secondary, tw))
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

/// Claude's words showing from the top down: a soft edge, `band` of the
/// height, sweeps down as `e` goes 0 → 1.
pub(crate) fn reveal(el: Div, e: f32) -> Div {
    if e >= 1.0 {
        return el;
    }
    const BAND: f32 = 0.35;
    let top = e * (1.0 + BAND) - BAND;
    el.mask(linear_gradient(
        180.0,
        linear_color_stop(rgba(0, 0, 0, 1.0), top),
        linear_color_stop(rgba(0, 0, 0, 0.0), top + BAND),
    ))
}

/// A tool's mark at the end of its line: a quiet tick, or a red cross.
pub(crate) fn outcome(failed: bool, size: f32, t: &Tokens) -> Div {
    let (glyph, color) = if failed {
        (Icon::X, t.status_waiting)
    } else {
        (Icon::Check, t.text_tertiary)
    };
    div().flex_none().child(icon(glyph, size, 2.4, color))
}
