//! Drawing the panel (`capture.css`): the see-through stage, the card with
//! its header, field, tiles and footer, the banner and the recents. The
//! menu is in `menu.rs`, the receipt in `receipt.rs`.

use std::time::Duration;

use gpui::component::Sizable as _;
use gpui::component::input::Textarea;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Div, ExternalPaths, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Transformation, Window, canvas, div, img,
    percentage, px, relative,
};
use nodus_core::capture_service::page::{self, bytes, host, kind_copy};
use nodus_domain::{
    CaptureAttachment, CaptureKind, CaptureRecent, CaptureShortcut, ClipboardOffer, FolderColor, SessionStatus,
};
use nodus_voice::controller::{MICROPHONE_SETTINGS_LABEL, Phase};

use super::icons::{CapIcon, cap_icon};
use super::panel::CapturePanel;
use super::style::{self, Inks};
use crate::components::folder_icon::folder_hues;
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::{enter, transition};
use crate::parity;
use crate::platform::Platform;
use crate::theme::tokens::{DUR_FAST, DUR_MED, Tokens, inset_shadow, shadow};
use crate::theme::{self, fonts, motion};

/// The stage's padding (`40px 56px 64px`): the room the card's shadow takes.
pub const STAGE_TOP: f32 = 40.0;
pub const STAGE_SIDE: f32 = 56.0;
pub const STAGE_BOTTOM: f32 = 64.0;
/// `--cap-w`.
pub const CARD_W: f32 = 540.0;

impl Render for CapturePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let ink = style::inks(&t);
        self.menu_bottom.set(None);
        let stage = match self.receipt.clone() {
            Some(shown) => self.render_receipt(&shown, t, ink, window, cx),
            None => self.render_stage(t, ink, window, cx),
        };
        // html.capture-window: see-through; body text as global.css has it.
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family(fonts::UI)
            .text_size(px(13.))
            .line_height(relative(1.4))
            .text_color(t.text_primary)
            .child(stage)
    }
}

impl CapturePanel {
    /// The stage (`.cap-stage`): the card, then the banner and the recents.
    fn render_stage(&mut self, t: Tokens, ink: Inks, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let card = self.render_card(t, ink, window, cx);
        let banner = self.render_banner(t, cx);
        let recents = self.render_recents(t, ink, window, cx);
        // Chromium's paint order: the banner, then the card (`position:
        // relative`) with its shadow over the banner, then the recents (their
        // filling animation makes each a layer) over the card's shadow. The
        // reversed column lays them out card, banner, recents.
        stage(STAGE_TOP)
            .flex_col_reverse()
            .children(recents.map(|r| gpui::deferred(r).into_any_element()))
            .children(banner)
            .child(card)
            .child(self.measure())
            .into_any_element()
    }

    /// CAP-010: the window as tall as the stage, or the open menu and the
    /// room for its shadow, whichever reaches further down.
    pub(super) fn measure(&self) -> impl IntoElement {
        let menu = self.menu_bottom.clone();
        let sent = self.sent_height.clone();
        let nodus = self.nodus.clone();
        canvas(
            move |bounds, _, _| {
                let mut bottom = f32::from(bounds.bottom());
                if let Some(menu) = menu.get() {
                    bottom = bottom.max(menu + page::MENU_ROOM as f32);
                }
                let height = f64::from(bottom.ceil());
                if height != sent.get() {
                    sent.set(height);
                    nodus.capture().resize(height);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// The card (`.cap-card`, role dialog `Nodus capture`).
    fn render_card(&mut self, t: Tokens, ink: Inks, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        // `cap-rise` when the card is made: on first show, and after a receipt.
        let rise = enter(
            window,
            cx,
            ("cap-card-rise", self.card_epoch as usize),
            DUR_MED,
            motion::ease_out,
        );
        let head = self.render_head(t, window, cx);
        let field = self.render_field(window, cx);
        let tiles = self.render_tiles(t, window, cx);
        let foot = self.render_foot(t, window, cx);
        let menu = self.menu_open.then(|| self.render_menu(t, ink, window, cx));
        let error = self.error.clone().map(|e| {
            div()
                .mx(px(16.))
                .mb(px(12.))
                .text_size(px(12.))
                .text_color(ink.waiting)
                .child(e)
        });
        let notice = self.shot_blocked.then(|| self.render_shot_notice(t, cx));
        let voice_notice = self.render_voice_notice(t, cx);
        let dropping = style::card_shadow(&t, true);
        div()
            .id("cap-card")
            .relative()
            .w(px(CARD_W))
            .flex_none()
            .rounded(px(14.))
            .bg(t.bg_elevated)
            .shadow(style::card_shadow(&t, false))
            .when(rise < 1.0, |d| {
                d.group_opacity(rise)
                    .translate(px(0.), px(-6.0 * (1.0 - rise)))
                    .scale(0.985 + 0.015 * rise)
            })
            // CAP-097: files dragged over the card ring it in the accent.
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.shadow(dropping.clone()))
            // CAP-062: and dropped on it, they come along.
            .on_drop::<ExternalPaths>(cx.listener(|p, paths: &ExternalPaths, window, cx| {
                let paths: Vec<String> = paths
                    .paths()
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .filter(|p| !p.is_empty())
                    .collect();
                p.drop_files(paths, window, cx);
            }))
            .child(head)
            .child(field)
            .children(tiles)
            .children(error)
            .children(notice)
            .children(voice_notice)
            .child(foot)
            .children(menu)
            .into_any_element()
    }

    /// The header (`.cap-head`): where it goes, and who said so.
    fn render_head(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let route = self.route();
        let has_text = self.has_text();
        let targets_loaded = self.targets.is_some();
        let project = self
            .targets
            .as_ref()
            .and_then(|ts| ts.projects.iter().find(|p| Some(&p.id) == route.project_id.as_ref()))
            .cloned();
        let thinking = self.guess.guessing && self.guess.guess.is_none() && self.choice.is_none();
        let frozen = parity::frozen(cx);
        let ink = style::inks(&t);

        // CAP-086
        let content: Vec<AnyElement> = if !has_text || !targets_loaded {
            vec![
                swatch(project.as_ref().map(|p| p.color)).into_any_element(),
                div()
                    .child(SharedString::from(
                        project
                            .as_ref()
                            .map_or("No projects yet".to_owned(), |p| p.name.clone()),
                    ))
                    .into_any_element(),
                icon(Icon::ChevronDown, 14., 2., t.text_tertiary).into_any_element(),
            ]
        } else if thinking {
            vec![
                pulsing(
                    "cap-thinking",
                    1200,
                    frozen,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(t.text_secondary)
                        .child(spinner("cap-thinking-spin", 14., t.text_secondary, frozen))
                        .child("Working out where this goes…"),
                )
                .into_any_element(),
            ]
        } else {
            let mut parts: Vec<AnyElement> = vec![
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .rounded(px(6.))
                    .bg(t.bg_sunken)
                    .child(kind_icon(route.kind, 14., t, ink))
                    .into_any_element(),
            ];
            if route.kind == CaptureKind::Into {
                let title = self
                    .sessions()
                    .iter()
                    .find(|s| Some(&s.session_id) == route.session_id.as_ref())
                    .map_or("a session".to_owned(), |s| s.title.clone());
                parts.push(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(format!("Into “{title}”")))
                        .into_any_element(),
                );
            } else {
                let label = match route.kind {
                    CaptureKind::Note => "Note",
                    CaptureKind::View => "View",
                    _ => "New session",
                };
                parts.push(div().font_weight(FontWeight::SEMIBOLD).child(label).into_any_element());
                parts.push(
                    div()
                        .text_color(t.text_tertiary)
                        .child(if route.kind == CaptureKind::Note { "on" } else { "in" })
                        .into_any_element(),
                );
                parts.push(swatch(project.as_ref().map(|p| p.color)).into_any_element());
                parts.push(
                    div()
                        .child(SharedString::from(
                            project.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                        ))
                        .into_any_element(),
                );
            }
            parts.push(icon(Icon::ChevronDown, 14., 2., t.text_tertiary).into_any_element());
            parts
        };

        // CAP-087: off until the targets are in, and with no projects.
        let enabled = targets_loaded && !self.no_projects();
        let bounds = self.route_bounds.clone();
        let button = div()
            .id("cap-route")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .max_w(px(360.))
            .h(px(32.))
            .pl(px(6.))
            .pr(px(8.))
            .rounded(px(8.))
            .whitespace_nowrap()
            .text_size(px(13.))
            .when(self.menu_open, |d| d.bg(t.bg_selected))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(t.bg_hover))
                    .on_click(cx.listener(|p, _, window, cx| p.toggle_menu(window, cx)))
            })
            .children(content)
            .child(
                canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            );

        // CAP-088
        let who = if has_text && (self.guess.guess.is_some() || self.choice.is_some()) {
            let mut who = who_row(&t);
            if self.choice.is_some() {
                who = who.child("Your choice");
            } else {
                who = who
                    .child(who_dot(t, self.guess.guessing, frozen))
                    .child("Claude's guess");
            }
            Some(who.child(kbd("Tab", t)).into_any_element())
        } else if !has_text {
            Some(
                who_row(&t)
                    .child("Claude works out what it is as you type")
                    .into_any_element(),
            )
        } else {
            None
        };
        let _ = window;

        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_h(px(48.))
            .pt(px(8.))
            .pb(px(8.))
            .pl(px(10.))
            .pr(px(12.))
            .border_b_1()
            .border_color(t.separator)
            .child(button)
            .child(div().flex_1().min_w_0())
            .children(who)
            .into_any_element()
    }

    /// The field (`.cap-field`, CAP-089, CAP-090): 15/22, padded 14 16 10,
    /// 3 rows growing to 8.
    fn render_field(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let placeholder = if self.no_projects() {
            "Add a project in Nodus to capture into"
        } else {
            "Ask Claude to do something, or jot a note"
        };
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.field
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        div()
            .pt(px(14.))
            .pb(px(10.))
            .px(px(12.))
            .child(
                Textarea::new(&self.field)
                    .appearance(false)
                    .xsmall()
                    .aria_label("What to capture")
                    .text_size(px(15.))
                    .line_height(px(22.)),
            )
            // PLAT-137: the capture box asks for spellcheck explicitly.
            .child(crate::components::spellcheck::underlines(
                "cap-field-spelling".into(),
                &self.field,
                px(15.),
                px(22.),
                window,
                cx,
            ))
            .into_any_element()
    }

    /// The tiles (`.cap-tiles`, CAP-091): each attachment, then the offer.
    fn render_tiles(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let offer = self.offer().cloned();
        if self.attachments.is_empty() && offer.is_none() {
            return None;
        }
        let mut tiles: Vec<AnyElement> = Vec::new();
        for (i, attachment) in self.attachments.clone().iter().enumerate() {
            tiles.push(self.render_tile(i, attachment, t, window, cx));
        }
        if let Some(offer) = offer {
            tiles.push(self.render_offer(&offer, t, cx));
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .px(px(16.))
                .pb(px(14.))
                .children(tiles)
                .into_any_element(),
        )
    }

    /// One attachment's tile (CAP-092…CAP-096).
    fn render_tile(
        &mut self,
        i: usize,
        attachment: &CaptureAttachment,
        t: Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let group: SharedString = format!("cap-tile-{i}").into();
        let hovered = window.use_keyed_state(("cap-tile-hover", i), cx, |_, _| false);
        let is_hovered = *hovered.read(cx);
        let image = matches!(attachment, CaptureAttachment::Image(_));
        // CAP-096: the × fades in on hover (120 ms).
        let shown = transition(
            window,
            cx,
            ("cap-tile-x", i),
            if is_hovered { 1.0 } else { 0.0 },
            DUR_FAST,
            motion::ease,
        );
        let remove = div()
            .id(("cap-tile-x", i))
            .absolute()
            .top(px(4.))
            .right(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(px(5.))
            .bg(if image {
                style::IMAGE_X_BG
            } else {
                style::elevated_at(&t, 0.8)
            })
            .opacity(shown)
            .cursor_pointer()
            .tooltip(native_tooltip("Remove".into()))
            .on_click(cx.listener(move |p, _, window, cx| p.detach(i, window, cx)))
            .child(icon(
                Icon::X,
                11.,
                2.,
                if image { style::WHITE } else { t.text_secondary },
            ));
        let tile = tile_box(&t).id(("cap-tile", i)).group(group).on_hover({
            let hovered = hovered.clone();
            move |over: &bool, _, cx| {
                hovered.update(cx, |h, cx| {
                    *h = *over;
                    cx.notify();
                });
            }
        });
        let tile = match attachment {
            // CAP-092
            CaptureAttachment::Tab(tab) => tile
                .w(px(168.))
                .tooltip(native_tooltip(tab.url.clone().into()))
                .child(
                    tile_top(&t)
                        .child(cap_icon(CapIcon::Globe, 12., t.text_tertiary))
                        .child(div().min_w_0().truncate().child(SharedString::from(host(&tab.url)))),
                )
                .child(two_lines(if tab.title.is_empty() { &tab.url } else { &tab.title })),
            // CAP-093
            CaptureAttachment::Image(shot) => {
                let thumb = (!shot.thumb.is_empty()).then(|| self.thumb(&shot.thumb)).flatten();
                tile.w(px(96.))
                    .p_0()
                    .bg(t.bg_window)
                    .tooltip(native_tooltip(shot.path.clone().into()))
                    .children(thumb.map(|thumb| cover(thumb, 96., 70.)))
                    .child(
                        div()
                            .absolute()
                            .left(px(6.))
                            .bottom(px(5.))
                            .px(px(5.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(style::LABEL_BG)
                            .text_color(style::WHITE)
                            .text_size(px(10.))
                            .line_height(px(14.))
                            .child(if shot.copied == Some(true) {
                                "Copied image"
                            } else {
                                "Screenshot"
                            }),
                    )
            }
            // CAP-094
            CaptureAttachment::File(file) => tile
                .flex_row()
                .items_center()
                .gap(px(8.))
                .h(px(52.))
                .tooltip(native_tooltip(file.path.clone().into()))
                .child(cap_icon(CapIcon::FileText, 18., t.text_tertiary))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().truncate().child(SharedString::from(file.name.clone())))
                        .when(file.size > 0, |d| {
                            d.child(
                                div()
                                    .text_color(t.text_tertiary)
                                    .child(SharedString::from(bytes(file.size))),
                            )
                        }),
                ),
            // CAP-095
            CaptureAttachment::Text(text) => tile
                .child(tile_top(&t).child("Copied text"))
                .child(mono(two_lines(&text.text))),
        };
        tile.child(remove).into_any_element()
    }

    /// CAP-098: the dashed tile offering what's on the clipboard.
    fn render_offer(&mut self, offer: &ClipboardOffer, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let (label, body) = match offer {
            ClipboardOffer::Text { text } => ("Copied text", mono(two_lines(text)).into_any_element()),
            ClipboardOffer::Image { thumb } => (
                "Copied image",
                match self.thumb(thumb) {
                    Some(image) => {
                        let (w, h) = (image.size(0).width.0 as f32, image.size(0).height.0 as f32);
                        let width = if h > 0.0 { (w * 34.0 / h).min(130.) } else { 0.0 };
                        div()
                            .h(px(34.))
                            .w(px(width))
                            .max_w_full()
                            .rounded(px(3.))
                            .overflow_hidden()
                            .child(cover(image, width, 34.))
                            .into_any_element()
                    }
                    None => div().into_any_element(),
                },
            ),
        };
        tile_box(&t)
            .id("cap-offer")
            .border_1()
            .border_dashed()
            .border_color(t.border_strong)
            .bg(gpui::transparent_black())
            .shadow(Vec::new())
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover))
            .on_click(cx.listener(|p, _, window, cx| p.take_offer(window, cx)))
            .child(
                tile_top(&t)
                    .text_color(t.text_secondary)
                    .child(icon(Icon::Plus, 12., 2., t.text_secondary))
                    .child(label),
            )
            .child(body)
            .into_any_element()
    }

    /// CAP-101: macOS kept the screenshot from Nodus.
    fn render_shot_notice(&self, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .mx(px(16.))
            .mb(px(12.))
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(t.text_secondary)
            .child(cap_icon(CapIcon::CircleAlert, 14., t.status_waiting))
            .child(div().flex_1().min_w_0().child(
                "macOS kept the screenshot from Nodus. Turn Nodus on in Screen & System Audio Recording, then take it again.",
            ))
            .child(
                banner_button("cap-allow-screen", "Open System Settings", t)
                    .on_click(cx.listener(|p, _, _, cx| p.allow_screen(cx))),
            )
            .into_any_element()
    }

    /// The footer (`.cap-foot`): screenshot, files, mic, the hint and send.
    fn render_foot(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        self.sync_voice_project();
        let route = self.route();
        let frozen = parity::frozen(cx);
        let enabled = self.has_text() && !self.sending && !self.no_projects();
        // CAP-106
        let label = if self.working_it_out() {
            "Working it out…"
        } else {
            kind_copy(route.kind).verb
        };
        let send_key_bg = if enabled { style::SEND_KEY_BG } else { t.bg_sunken };
        let send = div()
            .id("cap-send")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(32.))
            .pl(px(14.))
            .pr(px(8.))
            .rounded(px(8.))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| {
                if enabled {
                    d.bg(t.accent)
                        .text_color(t.text_on_accent)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.accent_hover))
                        .on_click(cx.listener(|p, _, window, cx| p.send(window, cx)))
                } else {
                    d.bg(t.bg_sunken).text_color(t.text_tertiary)
                }
            })
            .when(self.sending, |d| {
                let color = if enabled { t.text_on_accent } else { t.text_tertiary };
                d.child(spinner("cap-send-spin", 14., color, frozen))
            })
            .child(label)
            .child(
                // `.cap-send-key` under the global `kbd` rule.
                div()
                    .flex_none()
                    .mx(px(1.))
                    .px(px(5.))
                    .py(px(1.))
                    .min_w(px(18.))
                    .rounded(px(4.))
                    .bg(send_key_bg)
                    .border(px(0.5))
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(0.0, 1.0, 0.0, 0.0, t.border)])
                    .text_size(px(11.))
                    .line_height(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.text_secondary)
                    .flex()
                    .justify_center()
                    .child(SharedString::from(Platform::CURRENT.keys("↩"))),
            );
        // CAP-105, CAP-103: the hint, or what the mic is doing.
        let hint = if self.voice_state.phase == Phase::Idle {
            div()
                .flex()
                .flex_none()
                .items_center()
                .mr(px(10.))
                .text_size(px(12.))
                .text_color(t.text_tertiary)
                .child(kbd(Platform::CURRENT.keys("⇧↩"), t))
                .child(div().child("\u{a0}new line"))
                .into_any_element()
        } else {
            self.render_voice_status(window, cx)
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .py(px(8.))
            .px(px(10.))
            .border_t_1()
            .border_color(t.separator)
            .child(
                icon_button("cap-shot", CapIcon::Scan, 16., "Take a screenshot", t)
                    .on_click(cx.listener(|p, _, window, cx| p.screenshot(window, cx))),
            )
            .child(
                icon_button("cap-files", CapIcon::Paperclip, 16., "Attach files", t)
                    .on_click(cx.listener(|p, _, window, cx| p.pick_files(window, cx))),
            )
            .child(self.render_mic(t, cx))
            .child(div().flex_1().min_w_0())
            .child(hint)
            .child(send)
            .into_any_element()
    }

    /// CAP-103: the mic. `Speak`; while recording `Stop and transcribe` on
    /// red with a filled square; off while it opens or transcribes.
    fn render_mic(&self, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let phase = self.voice_state.phase;
        let recording = phase == Phase::Recording;
        let enabled = !matches!(phase, Phase::Opening | Phase::Transcribing);
        if !recording {
            let button = icon_button("cap-mic", CapIcon::Mic, 16., "Speak", t);
            return if enabled {
                button
                    .on_click(cx.listener(|p, _, _, cx| p.press_mic(cx)))
                    .into_any_element()
            } else {
                // Disabled: no hover, nothing else changes.
                div()
                    .id("cap-mic")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .tooltip(native_tooltip("Speak".into()))
                    .child(cap_icon(CapIcon::Mic, 16., t.text_secondary))
                    .into_any_element()
            };
        }
        div()
            .id("cap-mic")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(8.))
            .cursor_pointer()
            .bg(t.status_waiting)
            .tooltip(native_tooltip("Stop and transcribe".into()))
            .child(cap_icon(CapIcon::SquareFilled, 13., style::WHITE))
            .on_click(cx.listener(|p, _, _, cx| p.press_mic(cx)))
            .into_any_element()
    }

    /// 05's `VoiceStatus` in the hint's place (`.voice-status`, margin-right
    /// 10): the blinking dot, the clock and the meter while recording,
    /// `Transcribing…` while transcribing, nothing while the mic opens.
    fn render_voice_status(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        // The meter moves every frame while it listens (the shared status asks).
        let levels = self.voice.levels().map(|(bars, _)| bars);
        let status = crate::voice::voice_status_of(
            "cap-voice",
            self.voice_state.phase,
            self.voice_state.elapsed,
            levels,
            false,
            crate::voice::Z(1.),
            window,
            cx,
        );
        div().flex().flex_none().mr(px(10.)).children(status).into_any_element()
    }

    /// 05's `VoiceNotice` inside the card, under the error and the
    /// screenshot notice. The panel gives `.voice-notice` no layout of its
    /// own (its pill is `.sidekick`'s and `.session-mic`'s): the icon, the
    /// words and the buttons run on in a line, and the dismiss button, a
    /// grid, takes the line under them (spec 07, "UI details").
    fn render_voice_notice(&self, t: Tokens, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let notice = self.voice_state.notice.clone()?;
        let action = |id: &'static str, glyph: Option<CapIcon>, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .border(px(0.5))
                .border_color(t.border_strong)
                .bg(t.bg_elevated)
                .shadow(t.sm())
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .hover(|s| s.bg(t.bg_hover))
                .active(|s| s.bg(t.bg_pressed))
                .children(glyph.map(|g| cap_icon(g, 12., t.text_primary)))
                .child(label)
        };
        let mut line = div()
            .flex()
            .flex_wrap()
            .items_center()
            .child(cap_icon(CapIcon::CircleAlert, 14., t.status_waiting))
            .child(
                div()
                    .min_w_0()
                    .line_height(relative(1.35))
                    .text_color(t.text_primary)
                    .child(notice.message.clone()),
            );
        if notice.settings {
            line = line.child(
                action("cap-voice-settings", Some(CapIcon::Settings), "Settings").on_click(cx.listener(
                    |p, _, _, cx| {
                        let nodus = p.nodus.clone();
                        cx.spawn(async move |_, _| nodus.capture().open_settings().await)
                            .detach();
                    },
                )),
            );
        }
        if notice.microphone {
            line = line.child(
                action("cap-voice-mic", None, MICROPHONE_SETTINGS_LABEL).on_click(cx.listener(|p, _, _, cx| {
                    let nodus = p.nodus.clone();
                    cx.spawn(async move |_, _| nodus.voice().open_microphone_settings().await)
                        .detach();
                })),
            );
        }
        if notice.retry {
            line = line.child(
                action("cap-voice-retry", Some(CapIcon::RotateCcw), "Retry")
                    .tooltip(native_tooltip("Send the same recording again".into()))
                    .on_click(cx.listener(|p, _, _, cx| p.retry_voice(cx))),
            );
        }
        let dismiss = div()
            .id("cap-voice-dismiss")
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded_full()
            .text_color(t.text_tertiary)
            .hover(|s| s.bg(t.bg_hover).text_color(t.text_primary))
            .tooltip(native_tooltip("Dismiss".into()))
            .child(icon(Icon::X, 13., 2., t.text_tertiary))
            .on_click(cx.listener(|p, _, _, cx| {
                p.voice.dismiss();
                cx.notify();
            }));
        Some(
            div()
                .mx(px(12.))
                .mb(px(10.))
                .child(line)
                .child(dismiss)
                .into_any_element(),
        )
    }

    /// CAP-117: the banner under the card.
    fn render_banner(&self, t: Tokens, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let shortcut = self.banner()?;
        let row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .w(px(CARD_W))
            .pt(px(10.))
            .pb(px(10.))
            .pl(px(14.))
            .pr(px(12.))
            .rounded(px(10.))
            .bg(t.bg_elevated)
            .shadow(t.md())
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(t.text_secondary);
        Some(match shortcut {
            CaptureShortcut::NeedsPermission => row
                .child(div().flex_1().min_w_0().child(
                    "To open this from any app with a double tap of ⇧, allow Nodus in Accessibility. Already on there? Click Allow… anyway.",
                ))
                .child(
                    banner_button("cap-allow", "Allow…", t).on_click(cx.listener(|p, _, _, cx| p.allow(cx))),
                )
                .into_any_element(),
            _ => row
                .child(div().child(
                    "The double tap of ⇧ couldn’t start on this computer. Use File › Capture… instead.",
                ))
                .into_any_element(),
        })
    }

    /// CAP-152, CAP-153: the last three captures and how they're doing.
    fn render_recents(
        &mut self,
        t: Tokens,
        ink: Inks,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        if self.has_text() {
            return None;
        }
        let targets = self.targets.as_ref()?;
        if targets.recents.is_empty() {
            return None;
        }
        let now = parity::now_ms(cx);
        let epoch = self.recents_epoch as usize;
        let recents: Vec<CaptureRecent> = targets.recents.clone();
        let names: Vec<(String, String)> = targets
            .projects
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let mut cards: Vec<AnyElement> = Vec::new();
        for (i, recent) in recents.iter().take(3).enumerate() {
            let project = names
                .iter()
                .find(|(id, _)| *id == recent.project_id)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            cards.push(recent_card(i, epoch, recent, project, now, t, ink, window, cx));
        }
        let more = page::more_recents(&recents).map(|more| {
            div()
                .flex_none()
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(style::elevated_at(&t, 0.86))
                .shadow(t.sm())
                // See-through: its shadow only outside it, as CSS draws it.
                .shadow_outside()
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .child(style::spans(
                    window,
                    &[
                        (&more, t.text_secondary, None),
                        (" · ↑ for earlier captures", t.text_tertiary, None),
                    ],
                ))
        });
        Some(
            div()
                .flex()
                .flex_none()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .w(px(CARD_W))
                .child(div().grid().grid_cols(3).gap(px(8.)).w_full().children(cards))
                .children(more)
                .into_any_element(),
        )
    }
}

/// A recent capture's card (CAP-152): rises in turn, 30 ms apart.
#[allow(clippy::too_many_arguments)]
fn recent_card(
    i: usize,
    epoch: usize,
    recent: &CaptureRecent,
    project: String,
    now: f64,
    t: Tokens,
    ink: Inks,
    window: &mut Window,
    cx: &mut Context<'_, CapturePanel>,
) -> AnyElement {
    let delay = Duration::from_millis(30 * i as u64);
    let rise = enter(window, cx, ("cap-recent-rise", epoch * 8 + i), DUR_MED + delay, |x| x);
    // `animation-fill-mode: both`: still at the start through the delay.
    let total = (DUR_MED + delay).as_secs_f32();
    let elapsed = rise * total;
    let local = ((elapsed - delay.as_secs_f32()) / DUR_MED.as_secs_f32()).clamp(0.0, 1.0);
    let rise = motion::ease_out(local);
    let session = !matches!(recent.kind, CaptureKind::Note | CaptureKind::View);
    let waiting = session && recent.status == Some(SessionStatus::Waiting);
    let hovered = window.use_keyed_state(("cap-recent-hover", i), cx, |_, _| false);
    let is_hovered = *hovered.read(cx);
    let status_color = match (session, recent.status) {
        (true, Some(SessionStatus::Working)) => ink.working,
        (true, Some(SessionStatus::Waiting)) => ink.waiting,
        _ => t.text_tertiary,
    };
    let (project_id, session_id) = (recent.project_id.clone(), recent.session_id.clone());
    div()
        .id(("cap-recent", i))
        .flex()
        .flex_col()
        .gap(px(5.))
        .min_w_0()
        .h(px(104.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(t.bg_elevated)
        .shadow(style::recent_shadow(&t, is_hovered, waiting))
        .cursor_pointer()
        .tooltip(native_tooltip("Open it on the board".into()))
        .on_hover(move |over: &bool, _, cx| {
            hovered.update(cx, |h, cx| {
                *h = *over;
                cx.notify();
            });
        })
        .on_click(cx.listener(move |p, _, _, cx| p.open(project_id.clone(), session_id.clone(), cx)))
        .when(rise < 1.0, |d| {
            d.group_opacity(rise)
                .translate(px(0.), px(-6.0 * (1.0 - rise)))
                .scale(0.985 + 0.015 * rise)
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(t.text_tertiary)
                .child(mark(recent.kind, recent.status, t))
                .child(div().flex_1().min_w_0().truncate().child(SharedString::from(project)))
                .child(SharedString::from(page::ago(now, recent.created_at))),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(17.))
                .line_clamp(2)
                .text_ellipsis()
                .child(SharedString::from(recent.title.clone())),
        )
        .child(
            div()
                .mt_auto()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(status_color)
                .child(page::recent_status(recent)),
        )
        .into_any_element()
}

/// `.cap-stage`: a centred column, the card's shadow room around it.
pub(super) fn stage(top: f32) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .w_full()
        .pt(px(top))
        .px(px(STAGE_SIDE))
        .pb(px(STAGE_BOTTOM))
        .overflow_hidden()
}

/// `.cap-folder`: the project's colour as a small folder tab (blue when none).
pub(super) fn swatch(color: Option<FolderColor>) -> Div {
    let hue = folder_hues(color.unwrap_or(FolderColor::Blue)).front_bottom;
    div()
        .flex_none()
        .w(px(13.))
        .h(px(10.))
        .rounded_tl(px(2.))
        .rounded_tr(px(2.))
        .rounded_br(px(3.))
        .rounded_bl(px(3.))
        .bg(hue)
}

/// A kind's icon (`KindIcon`): a terminal, an arrow into, a note's yellow, a view.
pub(super) fn kind_icon(kind: CaptureKind, size: f32, t: Tokens, ink: Inks) -> AnyElement {
    match kind {
        CaptureKind::Session => icon(Icon::SquareTerminal, size, 2., t.text_primary).into_any_element(),
        CaptureKind::Into => cap_icon(CapIcon::CornerDownRight, size, ink.into).into_any_element(),
        CaptureKind::Note => div()
            .flex_none()
            .size(px(size - 2.))
            .rounded(px(3.))
            .bg(style::NOTE_YELLOW)
            .into_any_element(),
        CaptureKind::View => cap_icon(CapIcon::LayoutDashboard, size, ink.view).into_any_element(),
    }
}

/// `.cap-dot`: done blue, working orange and waiting red with a 3px ring.
pub(super) fn dot(status: Option<SessionStatus>, t: Tokens) -> Div {
    let status = status.unwrap_or(SessionStatus::Done);
    let color = t.status(status);
    div()
        .flex_none()
        .size(px(8.))
        .rounded_full()
        .bg(color)
        .when(status != SessionStatus::Done, |d| {
            d.shadow(vec![shadow(0.0, 0.0, 0.0, 3.0, style::ring_of(color))])
        })
}

/// A recent's mark (`Mark`): a session's dot, a note's or a view's square.
fn mark(kind: CaptureKind, status: Option<SessionStatus>, t: Tokens) -> Div {
    match kind {
        CaptureKind::Note => div().flex_none().size(px(9.)).rounded(px(2.)).bg(style::NOTE_YELLOW),
        CaptureKind::View => div().flex_none().size(px(9.)).rounded(px(2.)).bg(style::VIEW_PURPLE),
        _ => dot(status, t),
    }
}

/// `.cap-kbd` over the global `kbd` rule: 20 high, a sunken fill, an inset
/// ring and a hairline border.
pub(super) fn kbd(text: impl Into<SharedString>, t: Tokens) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .mx(px(1.))
        .min_w(px(20.))
        .h(px(20.))
        .px(px(5.))
        .rounded(px(5.))
        .bg(t.bg_sunken)
        .border(px(0.5))
        .border_color(t.border_strong)
        .shadow(vec![inset_shadow(0.0, 0.0, 0.0, 1.0, t.border)])
        .text_size(px(11.))
        .line_height(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .font_family(fonts::UI)
        .text_color(t.text_secondary)
        .child(text.into())
}

/// `.cap-who`.
fn who_row(t: &Tokens) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .text_color(t.text_tertiary)
}

/// `.cap-who-dot`: the accent dot, pulsing while Claude thinks.
pub(super) fn who_dot(t: Tokens, thinking: bool, frozen: bool) -> AnyElement {
    let d = div().flex_none().size(px(6.)).rounded_full().bg(t.accent);
    if thinking {
        pulsing("cap-who-dot", 900, frozen, d).into_any_element()
    } else {
        d.into_any_element()
    }
}

/// `Loader2` turning once every 0.9 s (`.cap-spin`), held still in parity mode.
pub(super) fn spinner(id: &'static str, size: f32, color: impl Into<gpui::Rgba>, frozen: bool) -> AnyElement {
    let svg = cap_icon(CapIcon::LoaderCircle, size, color);
    if frozen {
        return svg.into_any_element();
    }
    svg.with_animation(id, Animation::new(Duration::from_millis(900)).repeat(), |svg, delta| {
        svg.with_transformation(Transformation::rotate(percentage(delta)))
    })
    .into_any_element()
}

/// `cap-pulse`: opacity 1 → 0.55 → 1, ease-in-out, forever.
fn pulsing(id: &'static str, ms: u64, frozen: bool, element: Div) -> AnyElement {
    if frozen {
        return element.into_any_element();
    }
    element
        .with_animation(id, Animation::new(Duration::from_millis(ms)).repeat(), |d, delta| {
            let half = if delta < 0.5 { delta * 2.0 } else { (1.0 - delta) * 2.0 };
            d.opacity(1.0 - 0.45 * motion::ease_in_out(half))
        })
        .into_any_element()
}

/// `.cap-tile`: 150 × 70, padded 8 10, sunken with a hairline ring, 12/16.
fn tile_box(t: &Tokens) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(3.))
        .w(px(150.))
        .h(px(70.))
        .px(px(10.))
        .py(px(8.))
        .overflow_hidden()
        .rounded(px(9.))
        .bg(t.bg_sunken)
        .shadow(vec![inset_shadow(0.0, 0.0, 0.0, 1.0, t.border)])
        .text_size(px(12.))
        .line_height(px(16.))
}

/// `.cap-tile-top`.
fn tile_top(t: &Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w_0()
        .text_size(px(11.))
        .text_color(t.text_tertiary)
}

/// `.cap-two-lines`: at most two lines, cut with `…`, broken anywhere.
fn two_lines(text: &str) -> Div {
    div()
        .line_clamp(2)
        .text_ellipsis()
        .child(SharedString::from(text.to_owned()))
}

/// `.cap-mono`: SF Mono 11.
fn mono(d: Div) -> Div {
    d.font_family(fonts::MONO).text_size(px(11.))
}

/// An image filling `w × h`, cut from its top left (`object-fit: cover;
/// object-position: top left`).
fn cover(image: std::sync::Arc<gpui::RenderImage>, w: f32, h: f32) -> AnyElement {
    let size = image.size(0);
    let (iw, ih) = (size.width.0 as f32, size.height.0 as f32);
    if iw <= 0.0 || ih <= 0.0 {
        return div().into_any_element();
    }
    let scale = (w / iw).max(h / ih);
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .child(
            img(gpui::ImageSource::Render(image))
                .absolute()
                .top_0()
                .left_0()
                .w(px(iw * scale))
                .h(px(ih * scale)),
        )
        .into_any_element()
}

/// `.cap-banner-btn`: the accent button of the banner and the notice.
pub(super) fn banner_button(id: &'static str, label: &'static str, t: Tokens) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(28.))
        .px(px(12.))
        .rounded(px(7.))
        .bg(t.accent)
        .text_color(t.text_on_accent)
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .child(label)
}

/// `.cap-icon`: a 32px square button, secondary ink, darker on hover.
fn icon_button(id: &'static str, which: CapIcon, size: f32, label: &'static str, t: Tokens) -> gpui::Stateful<Div> {
    let group: SharedString = format!("{id}-group").into();
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(|s| s.bg(t.bg_hover))
        .tooltip(native_tooltip(label.into()))
        .child(cap_icon(which, size, t.text_secondary).group_hover(group, |s| s.text_color(t.text_primary)))
}
