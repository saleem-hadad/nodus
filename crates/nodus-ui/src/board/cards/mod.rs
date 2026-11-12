//! The things on the board (spec 10): session cards, expanded cards' chrome,
//! the agent deck and its fan, the archive frame and the looks of its
//! stack, sticky notes, their stickers and the note pad.
//!
//! # How the canvas uses them
//!
//! Each node is an element the canvas places at its node's position, sized
//! to the node: [`session_card::SessionNode`] (compact and expanded cards,
//! with their deck, fan and stickers), [`archive_zone::ArchiveZone`],
//! [`note::NoteNode`] (free notes and the pad's top sheet) and
//! [`note_pad::NotePad`]. They draw at `env.scale` (1 when the canvas
//! scales the world itself, the zoom when it lays nodes out in screen
//! pixels, which keeps text sharp), take what the board knows from
//! [`BoardEnv`] and report what the user does through [`BoardActions`].
//!
//! What the canvas owns (spec 09) stays there: positions, glides between
//! them, z-order, selection, drag and drop, click routing (the 240ms title
//! click, opening, renaming on double-click), context menus and shortcuts.
//! Regions that must not start a node drag (`nodrag` in 0.9.4: buttons,
//! fields, the fan, the expanded card's body) stop the left mouse-down from
//! propagating, so the canvas should start node drags from a bubble-phase
//! mouse-down on the node.

pub mod agent_deck;
pub mod anim;
pub mod archive_zone;
pub mod border;
pub mod expanded_card;
pub mod icons;
pub mod install;
pub mod note;
pub mod note_pad;
pub mod paper;
pub mod para;
pub mod present;
pub mod resize;
pub mod session_card;
pub mod shape;
pub mod status_dot;
pub mod sticker;

use std::rc::Rc;

use gpui::{AnyElement, App, Bounds, BoxShadow, Hsla, Pixels, Rgba, Window, point, px};
use nodus_domain::BoardNote;

use crate::components::tween::Mix;
use crate::platform::Platform;
use crate::theme::tokens::Layer;

pub use install::install;
pub use present::{Phase, StackLook};

/// The scale the board's things are drawn at: every CSS length goes
/// through it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Z(pub f32);

impl Z {
    pub fn px(self, v: f32) -> Pixels {
        px(v * self.0)
    }
    pub fn f(self, v: f32) -> f32 {
        v * self.0
    }
}

/// One `box-shadow` layer (x, y, CSS blur, spread, colour, inset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
    pub inset: bool,
}

const CLEAR: Rgba = Rgba {
    r: 0.,
    g: 0.,
    b: 0.,
    a: 0.,
};

impl Shade {
    pub const NONE: Self = Self {
        x: 0.,
        y: 0.,
        blur: 0.,
        spread: 0.,
        color: CLEAR,
        inset: false,
    };

    pub fn ring(width: f32, color: Rgba) -> Self {
        Self {
            spread: width,
            color,
            ..Self::NONE
        }
    }

    pub fn inset(x: f32, y: f32, color: Rgba) -> Self {
        Self {
            x,
            y,
            color,
            inset: true,
            ..Self::NONE
        }
    }
}

impl From<Layer> for Shade {
    fn from((x, y, blur, spread, color): Layer) -> Self {
        Self {
            x,
            y,
            blur,
            spread,
            color,
            inset: false,
        }
    }
}

/// A `box-shadow` list of up to four layers, first on top as CSS writes
/// it, which transitions as CSS's do: layer by layer, the shorter list
/// padded with transparent ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow(pub [Shade; 4]);

impl Shadow {
    pub fn of(layers: &[Shade]) -> Self {
        let mut out = [Shade::NONE; 4];
        for (slot, layer) in out.iter_mut().zip(layers) {
            *slot = *layer;
        }
        Self(out)
    }

    /// A token's layers after `first` (a ring, say).
    pub fn with(first: &[Shade], token: &[Layer]) -> Self {
        let mut all: Vec<Shade> = first.to_vec();
        all.extend(token.iter().map(|&l| Shade::from(l)));
        Self::of(&all)
    }

    /// GPUI's shadows at `z` (GPUI paints its last on top).
    pub fn gpui(&self, z: Z) -> Vec<BoxShadow> {
        self.0
            .iter()
            .rev()
            .filter(|l| l.color.a > 0.0)
            .map(|l| BoxShadow {
                color: l.color.into(),
                offset: point(z.px(l.x), z.px(l.y)),
                blur_radius: z.px(l.blur / 2.0),
                spread_radius: z.px(l.spread),
                inset: l.inset,
            })
            .collect()
    }

    /// Whether a layer is a ring: `0 0 0 <spread> <colour>`.
    fn is_ring(l: &Shade) -> bool {
        !l.inset && l.blur == 0.0 && l.x == 0.0 && l.y == 0.0 && l.spread > 0.0 && l.color.a > 0.0
    }

    /// GPUI's shadows for the blurred and inset layers; the rings are
    /// [`Shadow::rings`]'s to draw.
    pub fn soft(&self, z: Z) -> Vec<BoxShadow> {
        let mut soft = *self;
        for l in &mut soft.0 {
            if Self::is_ring(l) {
                *l = Shade::NONE;
            }
        }
        soft.gpui(z)
    }

    /// The ring layers drawn exactly round the box they're put in, whose
    /// corners are `radius` round (CSS px); `None` without rings.
    pub fn rings(&self, radius: f32, z: Z) -> Option<AnyElement> {
        // CSS paints its last layer first.
        let rings: Vec<(f32, Rgba)> = self
            .0
            .iter()
            .rev()
            .filter(|l| Self::is_ring(l))
            .map(|l| (l.spread, l.color))
            .collect();
        (!rings.is_empty()).then(|| gpui::IntoElement::into_any_element(border::rings(radius, rings, z)))
    }

    /// Every colour multiplied by `factor`, as `filter: brightness()` does.
    pub fn brightness(mut self, factor: f32) -> Self {
        for l in &mut self.0 {
            l.color = brightness(l.color, factor);
        }
        self
    }
}

impl Mix for Shadow {
    fn mix(self, to: Self, t: f32) -> Self {
        let mut out = self.0;
        for (o, (a, b)) in out.iter_mut().zip(self.0.iter().zip(to.0.iter())) {
            // A layer fading in or out keeps the other's geometry with a
            // transparent colour, as CSS pads the shorter list.
            let (a, b) = match (a.color.a == 0.0, b.color.a == 0.0) {
                (true, false) => (
                    &Shade {
                        color: Rgba { a: 0., ..b.color },
                        x: 0.,
                        y: 0.,
                        blur: 0.,
                        spread: 0.,
                        inset: b.inset,
                    },
                    b,
                ),
                (false, true) => (
                    a,
                    &Shade {
                        color: Rgba { a: 0., ..a.color },
                        x: 0.,
                        y: 0.,
                        blur: 0.,
                        spread: 0.,
                        inset: a.inset,
                    },
                ),
                _ => (a, b),
            };
            *o = Shade {
                x: a.x.mix(b.x, t),
                y: a.y.mix(b.y, t),
                blur: a.blur.mix(b.blur, t).max(0.0),
                spread: a.spread.mix(b.spread, t),
                color: premul_mix(a.color, b.color, t),
                inset: if t < 0.5 { a.inset } else { b.inset },
            };
        }
        Self(out)
    }
}

/// Colours mix in premultiplied space, as CSS transitions mix them.
pub fn premul_mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let alpha = a.a.mix(b.a, t);
    if alpha <= 0.0 {
        return Rgba { a: 0., ..b };
    }
    let ch = |x: f32, y: f32| (x * a.a).mix(y * b.a, t) / alpha;
    Rgba {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: alpha,
    }
}

/// `filter: brightness(f)` on one colour.
pub fn brightness(c: Rgba, f: f32) -> Rgba {
    Rgba {
        r: (c.r * f).min(1.0),
        g: (c.g * f).min(1.0),
        b: (c.b * f).min(1.0),
        a: c.a,
    }
}

/// `color-mix(in srgb, c share%, transparent)`.
pub fn fade(c: Rgba, share: f32) -> Rgba {
    Rgba { a: c.a * share, ..c }
}

/// `color-mix(in srgb, a p%, b)`, premultiplied as CSS mixes.
pub fn mix(a: Rgba, p: f32, b: Rgba) -> Rgba {
    premul_mix(b, a, p)
}

/// A `box-shadow` as CSS draws it, outside the border box only, for a box
/// `w`×`h` (flow units) with corners `radius` round. GPUI fills a shadow in
/// under the whole box, which shows through a translucent one (the open
/// archive panel, ARCH-004), so this one is drawn with the patched GPUI's
/// `shadow_outside` (PATCHES 14), to sit under the box.
pub fn outer_shadow(shadow: &Shadow, w: f32, h: f32, radius: f32, z: Z) -> Option<AnyElement> {
    use gpui::Styled as _;
    let layers = shadow.soft(z);
    if layers.is_empty() {
        return None;
    }
    Some(gpui::IntoElement::into_any_element(
        gpui::div()
            .absolute()
            .left_0()
            .top_0()
            .w(z.px(w))
            .h(z.px(h))
            .rounded(z.px(radius))
            .shadow(layers)
            .shadow_outside(),
    ))
}

pub fn hsla(c: Rgba) -> Hsla {
    c.into()
}

/// Where the first baseline of a line `line_height` tall of `font` at
/// `size` sits, as CSS centres the font's ascent and descent in it.
pub fn baseline(window: &Window, font: &gpui::Font, size: Pixels, line_height: Pixels) -> Pixels {
    let ts = window.text_system();
    let id = ts.resolve_font(font);
    let ascent = ts.ascent(id, size);
    let descent = ts.descent(id, size);
    (line_height - ascent - descent) / 2. + ascent
}

/// One line in the UI font at the board's scale, `size` CSS px, its line
/// 1.4 times that (the body's).
pub fn text(z: Z, s: impl Into<gpui::SharedString>, size: f32, weight: gpui::FontWeight, color: Rgba) -> para::Para {
    para::label(s, ui_font(weight), size, size * 1.4, color, z.0)
}

/// [`text`] with tabular figures.
pub fn text_tab(
    z: Z,
    s: impl Into<gpui::SharedString>,
    size: f32,
    weight: gpui::FontWeight,
    color: Rgba,
) -> para::Para {
    let mut font = ui_font(weight);
    font.features = crate::theme::fonts::tabular();
    para::label(s, font, size, size * 1.4, color, z.0)
}

/// A text field's line at the board's scale: gpui-component's editing
/// engine, no chrome, `height` tall, its text `size` (both scaled).
pub fn field_line(
    state: &gpui::Entity<gpui::component::input::InputState>,
    height: Pixels,
    size: Pixels,
    family: &'static str,
    weight: gpui::FontWeight,
) -> impl gpui::IntoElement {
    use gpui::Styled as _;
    let field = gpui::component::input::Input::new(state)
        .appearance(false)
        .px(px(0.))
        .py(px(0.))
        .line_height(height)
        .text_size(size)
        .font_family(family)
        .font_weight(weight);
    gpui::Styled::h(field, height)
}

/// A UI-font face at a weight.
pub fn ui_font(weight: gpui::FontWeight) -> gpui::Font {
    para::font(crate::theme::fonts::UI, weight, false)
}

/// What the archive's search and stack are doing (`ArchiveStackContext`,
/// BOARD-079).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StackView {
    pub open: bool,
    /// The pointer is over the stack and the search isn't dismissed.
    pub hover: bool,
    pub query: String,
    pub result_count: usize,
    pub scroll: f32,
    pub scroll_max: f32,
}

/// What a resize is resizing.
#[derive(Debug, Clone)]
pub enum Resized {
    Card(String),
    Note(Box<BoardNote>),
    /// A shape, by id (SHAPE-011).
    Shape(String),
}

/// The board actions cards, notes and the archive use (`BoardActions` in
/// BoardContext.ts, plus the store actions they call). `install.rs` maps
/// them onto the canvas's `board::nodes::BoardActions`.
pub trait CardActions {
    /// Commits a card's title (empty clears the override).
    fn rename(&self, session_id: &str, title: &str, cx: &mut App);
    /// Starts or stops renaming a card or section, or writing in a note.
    fn set_renaming(&self, id: Option<&str>, cx: &mut App);
    fn expand(&self, session_ids: &[String], expanded: bool, cx: &mut App);
    /// Archives, asking first when a session is running.
    fn archive(&self, session_ids: &[String], cx: &mut App);
    /// Stops sessions and keeps their cards, asking first mid-turn.
    fn stop(&self, session_ids: &[String], cx: &mut App);
    /// Saves a note's text; `project_id` is the board it was written on
    /// (NOTEUI-015: if the board has changed since, save to that project).
    fn edit_note(&self, project_id: &str, note: &BoardNote, text: &str, cx: &mut App);
    fn start_note(&self, note_id: &str, cx: &mut App);
    /// Puts words in a shape (SHAPE-012).
    fn edit_shape(&self, shape_id: &str, text: &str, cx: &mut App);
    /// Gives the keyboard back to the board.
    fn focus_board(&self, window: &mut Window, cx: &mut App);
    /// Opens a card's session as clicking the card does.
    fn open_session(&self, session_id: &str, cx: &mut App);
    /// Switches to a project and shows a session's card (`showCard`).
    fn show_card(&self, project_id: &str, session_id: Option<&str>, cx: &mut App);
    /// The agents viewer, on an agent or none.
    fn open_agents(&self, session_id: &str, agent_id: Option<&str>, cx: &mut App);
    /// The terminal modal.
    fn open_terminal(&self, session_id: &str, cx: &mut App);
    fn remove_notes(&self, note_ids: &[String], cx: &mut App);
    /// A node is being resized (`done` = false, the canvas shows the box
    /// under the pointer) or was (`done`, the rect rounded, to save).
    fn resize(&self, node: &Resized, rect: (f32, f32, f32, f32), done: bool, cx: &mut App);
    /// A card's title text was pressed (`.card-title`, BOARD-043/047).
    fn press_title(&self);
    /// Raises a card while its agent fan is out (z 1400, DECK-014).
    fn raise(&self, session_id: &str, raised: bool, cx: &mut App);
    // The archive stack (ArchiveStackContext).
    fn set_stack_query(&self, query: &str, cx: &mut App);
    fn open_stack(&self, cx: &mut App);
    fn close_stack(&self, cx: &mut App);
    fn submit_stack(&self, window: &mut Window, cx: &mut App);
}

/// The terminal registry, as an expanded card needs it (spec 12).
pub trait TerminalHost {
    /// `registry.needsOpen`.
    fn needs_open(&self, session_id: &str) -> bool;
    /// Attaches or detaches the card's terminal (`useAttachedTerminal`).
    fn set_enabled(&self, session_id: &str, kind_terminal: bool, enabled: bool, cx: &mut App);
    fn phase(&self, session_id: &str, cx: &App) -> Phase;
    fn reattach(&self, session_id: &str, cx: &mut App);
    fn focus(&self, session_id: &str, window: &mut Window, cx: &mut App);
    /// Types text at the prompt without sending it.
    fn type_in(&self, session_id: &str, text: &str, starting: bool, cx: &mut App);
    /// The terminal element filling the card's body.
    fn element(&self, session_id: &str, window: &mut Window, cx: &mut App) -> Option<AnyElement>;
    /// The mic for the card's title bar (spec 05, XCARD-010): what's said is
    /// typed at the prompt (`starting` while the session isn't `live`) and
    /// the card wakes.
    fn mic(&self, session_id: &str, live: bool, scale: f32, window: &mut Window, cx: &mut App) -> Option<AnyElement>;
}

/// What the board knows that its things draw from (the React contexts and
/// store reads of 0.9.4).
pub struct BoardEnv {
    /// The scale everything draws at.
    pub scale: Z,
    /// The viewport's zoom (labels that stay legible, the fan's side).
    pub zoom: f32,
    pub project_id: String,
    pub repo_path: Option<String>,
    /// `RenamingContext`: the card, section or note being renamed or written in.
    pub renaming: Option<String>,
    /// `ArchiveHotContext`.
    pub archive_hot: bool,
    pub stack: StackView,
    /// The board canvas on screen (window coordinates), for the fan.
    pub board_bounds: Bounds<Pixels>,
    /// `Date.now()`, frozen in parity mode.
    pub now_ms: f64,
    /// A text field (rename, note, section name, a terminal) has the focus:
    /// the archive's search mustn't take it (ARCH-020).
    pub text_focus: bool,
    /// The session the terminal modal shows, if open.
    pub terminal_modal: Option<String>,
    pub platform: Platform,
    pub actions: Rc<dyn CardActions>,
    pub terminals: Option<Rc<dyn TerminalHost>>,
}

impl BoardEnv {
    pub fn z(&self) -> Z {
        self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::{Tokens, rgba};

    #[test]
    fn transitions_shadows_layer_by_layer() {
        let t = Tokens::LIGHT;
        let sm = Shadow::with(&[], &t.shadow_sm);
        let md = Shadow::with(&[], &t.shadow_md);
        let half = sm.mix(md, 0.5);
        assert!((half.0[0].y - 2.5).abs() < 1e-5);
        assert!((half.0[0].blur - 8.0).abs() < 1e-5);
        let sel = Shadow::with(&[Shade::ring(2.0, t.accent)], &t.shadow_md);
        assert_eq!(sel.0[0].spread, 2.0);
        assert_eq!(sel.gpui(Z(1.0)).len(), 3);
        // The third layer fades in from nothing.
        let mid = sm.mix(sel, 0.5);
        assert!((mid.0[2].color.a - 0.05).abs() < 1e-5);
    }

    #[test]
    fn mixes_colours_premultiplied() {
        let a = rgba(255, 0, 0, 1.0);
        let clear = rgba(0, 0, 255, 0.0);
        let m = premul_mix(a, clear, 0.5);
        assert!((m.r - 1.0).abs() < 1e-5 && m.b.abs() < 1e-5 && (m.a - 0.5).abs() < 1e-5);
        // CARD-021: --text-primary 7% into --bg-elevated.
        let hover = mix(rgba(0, 0, 0, 0.88), 0.07, rgba(255, 255, 255, 1.0));
        assert!((hover.a - 0.9916).abs() < 1e-3, "{hover:?}");
        assert!((hover.r * 255.0 - 239.6).abs() < 0.6, "{hover:?}");
    }
}
