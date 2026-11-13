//! What's new (spec 06, UPD-096…UPD-101): after an update, a small card in
//! the window's bottom-right corner walks through what the new versions
//! brought, a slide at a time, until it's closed.

pub mod slides;

use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Image, ImageFormat, InteractiveElement, IntoElement, ObjectFit, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledImage as _, Window, div, img, px,
};

use self::slides::{SLIDES, Slide};
use crate::components::button::button;
use crate::components::icon::{Icon, icon};
use crate::components::presence::{Presence, inert};
use crate::components::tween::enter;
use crate::platform::Platform;
use crate::shell::root::Shell;
use crate::theme::{self, motion, tokens::*};

/// The card's width, and how far it sits from the window's edges.
const WIDTH: f32 = 300.;
const INSET: f32 = 16.;
/// The screenshot's height: 16:10 at the card's width.
const PICTURE_H: f32 = 188.;
/// At most this many slides, the newest versions' first.
const MOST: usize = 6;

/// A slide with its screenshots, decoded once.
struct Shown {
    slide: &'static Slide,
    light: Arc<Image>,
    dark: Arc<Image>,
}

/// The slides on the card and the one showing.
#[derive(Clone)]
pub struct Deck {
    slides: Rc<Vec<Shown>>,
    index: usize,
}

/// The card, while it's open, and its exit once closed.
#[derive(Default)]
pub struct WhatsNewSlot {
    deck: Option<Deck>,
    presence: Presence<Deck>,
}

impl Shell {
    /// UPD-096: at launch, the slides of the versions the user hasn't seen.
    pub(crate) fn ask_whats_new(&mut self, cx: &mut Context<'_, Self>) {
        let Some(unseen) = self.state.read(cx).nodus.updates().whats_new() else {
            return;
        };
        let slides = SLIDES.iter().filter(|slide| unseen.includes(slide.version));
        self.show_whats_new(slides.collect(), cx);
    }

    /// Opens the card on `slides`; nothing with none.
    pub fn show_whats_new(&mut self, slides: Vec<&'static Slide>, cx: &mut Context<'_, Self>) {
        let slides: Vec<Shown> = slides
            .into_iter()
            .take(MOST)
            .map(|slide| Shown {
                slide,
                light: Arc::new(Image::from_bytes(ImageFormat::Jpeg, slide.light.to_vec())),
                dark: Arc::new(Image::from_bytes(ImageFormat::Jpeg, slide.dark.to_vec())),
            })
            .collect();
        self.whats_new.deck = (!slides.is_empty()).then(|| Deck {
            slides: Rc::new(slides),
            index: 0,
        });
        cx.notify();
    }

    /// UPD-099: Back and Next, or a dot.
    fn go_to_slide(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if let Some(deck) = &mut self.whats_new.deck {
            deck.index = index.min(deck.slides.len() - 1);
            cx.notify();
        }
    }

    /// UPD-097: × or Done; it won't show again until the next update.
    fn close_whats_new(&mut self, cx: &mut Context<'_, Self>) {
        if self.whats_new.deck.take().is_some() {
            self.state.read(cx).nodus.updates().whats_new_seen();
            cx.notify();
        }
    }

    /// UPD-098…UPD-101: the card, rising into the corner as the toasts do
    /// and sinking back once closed (MOTION-011).
    pub(crate) fn render_whats_new(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let open = self.whats_new.deck.clone();
        let (deck, leaving) = self.whats_new.presence.frame("whats-new-drawn", open, window, cx)?;
        let t = theme::tokens(cx);
        let rise = leaving.unwrap_or_else(|| enter(window, cx, "whats-new-rise", DUR_MED, motion::ease_out));
        let (index, count) = (deck.index, deck.slides.len());
        let shown = &deck.slides[index];
        let slide = shown.slide;
        // A new slide fades in where the last one was.
        let fade = enter(window, cx, ("whats-new-slide", index), DUR_MED, motion::ease_out);
        let last = index + 1 == count;

        let close = div()
            .id("whats-new-close")
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .size(px(22.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_0073))
            .hover(|d| d.bg(gpui::rgba(0x0000_00a6)))
            .cursor_pointer()
            .child(icon(Icon::X, 12., 2.4, gpui::white()))
            .on_click(cx.listener(|shell, _, _, cx| shell.close_whats_new(cx)));
        let picture = div()
            .relative()
            .h(px(PICTURE_H))
            .border_b_1()
            .border_color(t.border)
            .child(
                img(if t.dark {
                    shown.dark.clone()
                } else {
                    shown.light.clone()
                })
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded_t(RADIUS_LG - px(1.))
                .opacity(fade),
            )
            .child(close);
        let eyebrow = if count > 1 {
            format!("NEW IN {} · {} OF {count}", slide.version, index + 1)
        } else {
            format!("NEW IN {}", slide.version)
        };
        let words = div()
            .px(px(14.))
            .pt(px(11.))
            .opacity(fade)
            .child(
                div()
                    .text_size(px(10.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text_tertiary)
                    .child(SharedString::from(eyebrow)),
            )
            .child(
                div()
                    .mt(px(3.))
                    .text_size(px(13.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(slide.title),
            )
            .child(
                // Two lines' room, so the card keeps its height from slide to slide.
                div()
                    .mt(px(3.))
                    .min_h(px(2. * 17.5))
                    .text_size(TEXT_SM)
                    .line_height(px(17.5))
                    .text_color(t.text_secondary)
                    .child(SharedString::from(with_keys(slide.text, Platform::CURRENT))),
            );
        let dots = div().flex().gap(px(5.)).when(count > 1, |d| {
            d.children((0..count).map(|i| {
                div()
                    .id(("whats-new-dot", i))
                    .size(px(6.))
                    .rounded_full()
                    .bg(if i == index { t.text_secondary } else { t.border_strong })
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _, _, cx| shell.go_to_slide(i, cx)))
            }))
        });
        let buttons = div()
            .flex()
            .gap(px(4.))
            .when(index > 0, |d| {
                d.child(
                    button("whats-new-back")
                        .ghost()
                        .label("Back")
                        .on_click(cx.listener(move |shell, _, _, cx| shell.go_to_slide(index - 1, cx))),
                )
            })
            .child(
                button("whats-new-next")
                    .primary()
                    .label(if last { "Done" } else { "Next" })
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        if last {
                            shell.close_whats_new(cx);
                        } else {
                            shell.go_to_slide(index + 1, cx);
                        }
                    })),
            );
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(14.))
            .pr(px(10.))
            .pt(px(12.))
            .pb(px(10.))
            .child(dots)
            .child(buttons);
        let card = div()
            .id("whats-new")
            .occlude()
            .absolute()
            .right(px(INSET))
            .bottom(px(INSET))
            .w(px(WIDTH).min(window.viewport_size().width - px(2. * INSET)))
            .translate(px(0.), px(8. * (1. - rise)))
            .opacity(rise)
            .flex()
            .flex_col()
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(t.border)
            .bg(t.bg_elevated)
            .shadow(t.lg())
            .child(picture)
            .child(words)
            .child(footer);
        Some(match leaving {
            None => card.into_any_element(),
            Some(_) => inert(card).into_any_element(),
        })
    }
}

/// A slide's sentence with each `{⌥⌘G}` written as `platform` writes it
/// (UI-303): Ctrl+Alt+G off a Mac.
fn with_keys(text: &str, platform: Platform) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        out.push_str(&rest[..open]);
        out.push_str(&platform.keys(&rest[open + 1..open + close]));
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_slides_shortcuts_as_the_platform_does() {
        let text = "Press {⌥⌘G}, then {⌘G}.";
        assert_eq!(with_keys(text, Platform::Mac), "Press ⌥⌘G, then ⌘G.");
        assert_eq!(with_keys(text, Platform::Windows), "Press Ctrl+Alt+G, then Ctrl+G.");
        assert_eq!(with_keys("No keys {here", Platform::Windows), "No keys {here");
    }
}
