//! The report dialog drawn: a modal over the main window, one step at a time
//! (RPT-003…RPT-043).

use gpui::component::Sizable as _;
use gpui::component::input::{Enter, Escape, IndentInline, OutdentInline, Paste, Textarea};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Corners, Div, ExternalPaths, Focusable as _, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ObjectFit, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    StyledImage as _, Window, anchored, deferred, div, img, point, px,
};

use super::{Notice, ReportDialog, Shot, Step};
use crate::components::button::button;
use crate::components::focus_ring::focus_ring;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::text_field::input;
use crate::platform::Platform;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// Where every report goes, as the dialog names it.
const REPO_LABEL: &str = "github.com/saleem-hadad/nodus";

impl Render for ReportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // RPT-007, RPT-008: out of the way while a screenshot is taken. A
        // clear cover stays, so the board's pages keep drawing as pictures.
        if self.shooting {
            let size = window.viewport_size();
            return deferred(
                anchored()
                    .position(point(px(0.), px(0.)))
                    .child(div().id("report-cover").occlude().w(size.width).h(size.height)),
            )
            .with_priority(500)
            .into_any_element();
        }
        let t = theme::tokens(cx);
        let (width, content) = match &self.step {
            Step::Write => (560., self.render_write(t, window, cx)),
            Step::Review => (600., self.render_review(t, window, cx)),
            Step::Done(_) | Step::Browser { .. } => (520., self.render_done(t, cx)),
        };
        let dropping = t.archive_drop;
        let panel = div()
            .id("report-panel")
            .track_focus(&self.panel)
            .flex()
            .flex_col()
            .min_h_0()
            .text_size(TEXT_MD)
            .text_color(t.text_primary)
            // RPT-004: ⌘↩ does the main button, from any field.
            .capture_action(cx.listener(|d, e: &Enter, window, cx| {
                if e.secondary {
                    cx.stop_propagation();
                    d.primary(window, cx);
                }
            }))
            // RPT-002: Esc closes, even from a field.
            .capture_action(cx.listener(|d, _: &Escape, _, cx| {
                cx.stop_propagation();
                d.close(cx);
            }))
            // RPT-003: Tab goes from field to field rather than indenting.
            .capture_action(cx.listener(|d, _: &IndentInline, window, cx| {
                cx.stop_propagation();
                d.focus_next(false, window, cx);
            }))
            .capture_action(cx.listener(|d, _: &OutdentInline, window, cx| {
                cx.stop_propagation();
                d.focus_next(true, window, cx);
            }))
            .on_key_down(cx.listener(|d, e: &KeyDownEvent, window, cx| d.on_key_down(e, window, cx)))
            // RPT-011: images dropped anywhere on the dialog.
            .when(matches!(self.step, Step::Write), |d| {
                d.drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(dropping))
                    .on_drop::<ExternalPaths>(cx.listener(|d, paths: &ExternalPaths, _, cx| {
                        d.drop_files(paths.paths(), cx);
                    }))
            })
            .child(content);
        let this = cx.entity();
        let mut modal = Modal::new("report-dialog", px(width), move |_, cx| this.update(cx, Self::close));
        // RPT-002: a stray click outside doesn't throw a draft away.
        modal.close_on_backdrop = false;
        modal.render(panel, window, cx)
    }
}

impl ReportDialog {
    /// Keys while the dialog itself has the focus (no field does).
    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let m = &e.keystroke.modifiers;
        match e.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.close(cx);
            }
            "enter" if Platform::CURRENT.mod_key(m) || matches!(self.step, Step::Done(_) | Step::Browser { .. }) => {
                cx.stop_propagation();
                self.primary(window, cx);
            }
            _ => {}
        }
    }

    /// RPT-003: Tab and ⇧Tab between the step's fields.
    fn focus_next(&mut self, back: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let handles = match self.step {
            Step::Write => vec![self.message.focus_handle(cx), self.username.focus_handle(cx)],
            Step::Review => vec![self.title.focus_handle(cx), self.body.focus_handle(cx)],
            _ => return,
        };
        let n = handles.len();
        let at = handles.iter().position(|h| h.is_focused(window));
        let next = match (at, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, _) => 0,
        };
        match (&self.step, next) {
            (Step::Write, 0) => self.message.update(cx, |f, cx| f.focus(window, cx)),
            (Step::Write, _) => self.username.update(cx, |f, cx| f.focus(window, cx)),
            (Step::Review, 0) => self.title.update(cx, |f, cx| f.focus(window, cx)),
            _ => self.body.update(cx, |f, cx| f.focus(window, cx)),
        }
    }

    fn render_write(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let message_focused = self.message.focus_handle(cx).is_focused(window);
        let name_focused = self.username.focus_handle(cx).is_focused(window);
        let shots =
            (!self.shots.is_empty()).then(|| {
                div().flex().flex_wrap().gap(px(8.)).px(px(10.)).pb(px(10.)).children(
                    self.shots.iter().enumerate().map(|(i, shot)| {
                        thumb(shot, 96., 60., t).child(
                            div()
                                .id(("report-shot-remove", i))
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .size(px(18.))
                                .rounded_full()
                                .bg(Rgba {
                                    r: 0.,
                                    g: 0.,
                                    b: 0.,
                                    a: 0.6,
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .tooltip(crate::components::tooltip::native_tooltip("Remove".into()))
                                .on_click(cx.listener(move |d, _, _, cx| d.remove_shot(i, cx)))
                                .child(icon(
                                    Icon::X,
                                    10.,
                                    3.,
                                    Rgba {
                                        r: 1.,
                                        g: 1.,
                                        b: 1.,
                                        a: 1.,
                                    },
                                )),
                        )
                    }),
                )
            });
        let message = div()
            .relative()
            .flex()
            .flex_col()
            .rounded(RADIUS_MD)
            .border(px(0.5))
            .border_color(if message_focused {
                transparent()
            } else {
                t.border_strong
            })
            .bg(t.bg_elevated)
            .capture_action(cx.listener(|d, _: &Paste, _, cx| {
                if d.paste(cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div().px(px(12.)).pt(px(10.)).pb(px(8.)).child(
                    Textarea::new(&self.message)
                        .appearance(false)
                        .xsmall()
                        .aria_label("What happened?")
                        .text_size(px(13.5))
                        .line_height(px(20.)),
                ),
            )
            .children(shots)
            .when(message_focused, |d| {
                d.child(focus_ring(Corners::all(RADIUS_MD), px(0.), t.focus_ring))
            });
        let full = self.full();
        let paste_key = if Platform::CURRENT.is_mac() { "⌘V" } else { "Ctrl+V" };
        let shot_buttons = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .when(Platform::CURRENT.is_mac(), |d| {
                d.child(
                    button("report-shot-window")
                        .icon(Icon::AppWindowMac, 14., 2.)
                        .label("Screenshot of Nodus")
                        .disabled(full)
                        .on_click(cx.listener(|d, _, window, cx| d.shoot_window(window, cx))),
                )
            })
            .child(
                button("report-shot-area")
                    .icon(Icon::SquareDashed, 14., 2.)
                    .label(if Platform::CURRENT.is_mac() {
                        "Part of the screen"
                    } else {
                        "Screenshot of the screen"
                    })
                    .disabled(full)
                    .on_click(cx.listener(|d, _, window, cx| d.shoot_area(window, cx))),
            )
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(t.text_secondary)
                    .child(format!("or paste with {paste_key}, or drop images. Up to 5.")),
            );
        let notice = self.notice.map(|notice| self.render_notice(notice, t, cx));
        let name_frame = div()
            .relative()
            .flex()
            .items_center()
            .h(px(32.))
            .px(px(12.))
            .rounded(RADIUS_SM)
            .border(px(0.5))
            .border_color(if name_focused {
                transparent()
            } else if self.username_invalid {
                t.status_waiting
            } else {
                t.border_strong
            })
            .bg(t.bg_elevated)
            .child(div().text_color(t.text_secondary).pr(px(2.)).child("@"))
            .child(div().flex_1().min_w_0().child(input(&self.username, px(18.), false)))
            .when(name_focused, |d| {
                d.child(focus_ring(Corners::all(RADIUS_SM), px(0.), t.focus_ring))
            });
        let name_hint = if self.username_invalid {
            hint_row(
                Icon::CircleAlert,
                "That isn't a GitHub username: letters, digits and single hyphens, up to 39.",
                t.status_waiting,
                t.diff_del,
            )
        } else {
            hint_row(
                Icon::Bell,
                "The issue mentions you, so GitHub emails you when Saleem replies or ships a fix. No sign-in.",
                t.text_secondary,
                t.text_secondary,
            )
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .px(px(20.))
            .pt(px(16.))
            .pb(px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("What happened?", t))
                    .child(message)
                    .child(shot_buttons.mt(px(2.)))
                    .children(notice),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div().flex().gap(px(4.)).child(label("Your GitHub username", t)).child(
                            div()
                                .text_size(TEXT_SM)
                                .text_color(t.text_secondary)
                                .child("· optional"),
                        ),
                    )
                    .child(name_frame)
                    .child(name_hint),
            );
        let can_review = self.can_review(cx);
        let footer = footer(t)
            .child(hint_row(
                Icon::Globe,
                &format!("Public on {REPO_LABEL}"),
                t.text_secondary,
                t.text_secondary,
            ))
            .child(div().flex_1())
            .child(
                button("report-cancel")
                    .label("Cancel")
                    .on_click(cx.listener(|d, _, _, cx| d.close(cx))),
            )
            .child(
                button("report-review")
                    .primary()
                    .label("Review")
                    .tooltip(if Platform::CURRENT.is_mac() {
                        "⌘↩"
                    } else {
                        "Ctrl+Enter"
                    })
                    .disabled(!can_review)
                    .on_click(cx.listener(|d, _, window, cx| d.review(window, cx))),
            );
        div()
            .flex()
            .flex_col()
            .child(header(
                "Report a problem",
                "Tell Saleem what's broken, confusing or missing. You'll see the issue before it's posted publicly on GitHub.",
                t,
                cx,
            ))
            .child(steps(1, t))
            .child(body)
            .child(footer)
            .into_any_element()
    }

    fn render_notice(&self, notice: Notice, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let text = match notice {
            Notice::Full => "That's 5 screenshots, the most one report takes.",
            Notice::Unsupported => "Only pictures can be attached: PNG, JPEG, GIF, WebP, BMP or TIFF.",
            Notice::TooLarge => "That picture is too large to attach, even made smaller.",
            Notice::NeedsPermission => {
                "macOS kept the screen from Nodus. Turn Nodus on in Screen & System Audio Recording, then try again."
            }
            Notice::NoWindowShot => "Nodus couldn't take a picture of its window. Try Part of the screen.",
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(hint_row(Icon::CircleAlert, text, t.status_waiting, t.text_primary).flex_1())
            .when(notice == Notice::NeedsPermission, |d| {
                d.child(
                    button("report-allow-screen")
                        .label("Open Settings")
                        .on_click(cx.listener(|d, _, _, cx| d.open_screen_settings(cx))),
                )
            })
            .into_any_element()
    }

    fn render_review(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let title_focused = self.title.focus_handle(cx).is_focused(window);
        let body_focused = self.body.focus_handle(cx).is_focused(window);
        let title = div()
            .relative()
            .flex()
            .items_center()
            .h(px(32.))
            .px(px(12.))
            .rounded(RADIUS_SM)
            .border(px(0.5))
            .border_color(if title_focused { transparent() } else { t.border_strong })
            .bg(t.bg_elevated)
            .child(div().flex_1().min_w_0().child(input(&self.title, px(18.), false)))
            .when(title_focused, |d| {
                d.child(focus_ring(Corners::all(RADIUS_SM), px(0.), t.focus_ring))
            });
        let body = div()
            .relative()
            .px(px(12.))
            .pt(px(10.))
            .pb(px(8.))
            .rounded(RADIUS_MD)
            .border(px(0.5))
            .border_color(if body_focused { transparent() } else { t.border_strong })
            .bg(t.bg_elevated)
            .child(
                Textarea::new(&self.body)
                    .appearance(false)
                    .xsmall()
                    .aria_label("Description")
                    .text_size(px(13.5))
                    .line_height(px(20.)),
            )
            .when(body_focused, |d| {
                d.child(focus_ring(Corners::all(RADIUS_MD), px(0.), t.focus_ring))
            });
        let client = super::client_info();
        let footer_line = nodus_feedback::footer(&client.version, &client.os, &client.arch);
        let shots = (!self.shots.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label("Screenshots, added to the issue", t))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .children(self.shots.iter().map(|shot| thumb(shot, 120., 75., t))),
                )
        });
        let computer = if Platform::CURRENT.is_mac() { "Mac" } else { "PC" };
        let privacy = div()
            .flex()
            .gap(px(8.))
            .px(px(11.))
            .py(px(9.))
            .rounded(RADIUS_SM)
            .bg(t.bg_sunken)
            .child(hint_row(
                Icon::Shield,
                &format!(
                    "Your home folder shows as ~ and your {computer}'s name is taken out. Nothing else from your {computer} is sent."
                ),
                t.text_secondary,
                t.text_secondary,
            ));
        let failure = self.failure.as_ref().map(|f| {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .px(px(14.))
                .py(px(12.))
                .rounded(RADIUS_MD)
                .bg(Rgba {
                    a: 0.07,
                    ..t.status_waiting
                })
                .border(px(1.))
                .border_color(Rgba {
                    a: 0.25,
                    ..t.status_waiting
                })
                .child(
                    div()
                        .flex()
                        .gap(px(9.))
                        .child(icon(Icon::CircleAlert, 16., 2., t.status_waiting).mt(px(1.)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.diff_del)
                                        .child(f.title),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .line_height(px(18.))
                                        .text_color(t.text_secondary)
                                        .child(f.message.clone()),
                                ),
                        ),
                )
                .when(f.browser, |d| {
                    d.child(
                        div().flex().pl(px(25.)).child(
                            button("report-browser")
                                .icon(Icon::ExternalLink, 13., 2.)
                                .label("Open the issue in your browser")
                                .on_click(cx.listener(|d, _, window, cx| d.open_in_browser(window, cx))),
                        ),
                    )
                })
        });
        let body_block = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .px(px(20.))
            .pt(px(16.))
            .pb(px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("Title", t))
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("Description", t))
                    .child(body)
                    .child(
                        div()
                            .text_size(TEXT_SM)
                            .text_color(t.text_secondary)
                            .child(format!("Added under it: {footer_line}")),
                    ),
            )
            .children(shots)
            .child(privacy)
            .children(failure);
        let can_post = self.can_post(cx);
        let post_label = if self.sending {
            "Posting…"
        } else if self.failure.is_some() {
            "Try again"
        } else {
            "Post issue"
        };
        let footer = footer(t)
            .child(
                button("report-back")
                    .label("Back")
                    .disabled(self.sending)
                    .on_click(cx.listener(|d, _, window, cx| d.back(window, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(t.text_secondary)
                    .child("Opened on GitHub under Saleem's account"),
            )
            .child(
                button("report-post")
                    .primary()
                    .label(post_label)
                    .tooltip(if Platform::CURRENT.is_mac() {
                        "⌘↩"
                    } else {
                        "Ctrl+Enter"
                    })
                    .disabled(!can_post)
                    .on_click(cx.listener(|d, _, window, cx| d.post(window, cx))),
            );
        div()
            .flex()
            .flex_col()
            .child(header(
                "Review the issue",
                "This is exactly what will be posted publicly on GitHub. Change anything you like.",
                t,
                cx,
            ))
            .child(steps(2, t))
            .child(body_block)
            .child(footer)
            .into_any_element()
    }

    fn render_done(&mut self, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let green = t.diff_add;
        let (badge, heading, text, link): (AnyElement, &str, String, Option<(String, String)>) = match &self.step {
            Step::Done(posted) => {
                let text = match &self.reporter {
                    Some(name) => format!(
                        "Issue #{} is posted. GitHub will email @{name} when Saleem replies or ships a fix.",
                        posted.number
                    ),
                    None => format!(
                        "Issue #{} is posted. Follow it on GitHub to see replies.",
                        posted.number
                    ),
                };
                (
                    badge(Icon::Check, green, t),
                    "Thanks, it's on GitHub",
                    text,
                    Some((format!("Open issue #{}", posted.number), posted.url.clone())),
                )
            }
            Step::Browser { clipboard, folder } => {
                let mut text =
                    "The issue is open in your browser, filled in. Press Create there to post it.".to_owned();
                if *folder {
                    let place = if Platform::CURRENT.is_mac() {
                        "Finder"
                    } else {
                        "Explorer"
                    };
                    text.push_str(&format!(
                        " Your screenshots are in the {place} window: drag them into the description."
                    ));
                }
                if *clipboard {
                    text.push_str(" The description was too long for the link, so it's on the clipboard: paste it in.");
                }
                (badge(Icon::ExternalLink, t.accent, t), "Finish on GitHub", text, None)
            }
            _ => return div().into_any_element(),
        };
        let link = link.map(|(label, url)| {
            button("report-open-issue")
                .ghost()
                .icon(Icon::ExternalLink, 13., 2.)
                .label(label)
                .colors(t.accent, t.accent_hover)
                .on_click(move |_, _, _| {
                    nodus_platform::shell::open_external(&url);
                })
        });
        let footer = footer(t).child(div().flex_1()).child(
            button("report-done")
                .primary()
                .label("Done")
                .on_click(cx.listener(|d, _, _, cx| d.close(cx))),
        );
        div()
            .flex()
            .flex_col()
            .child(steps(3, t).pt(px(18.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .px(px(28.))
                    .pt(px(22.))
                    .pb(px(22.))
                    .child(badge)
                    .child(
                        div()
                            .font_family(fonts::DISPLAY)
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(heading.to_owned()),
                    )
                    .child(
                        div()
                            .max_w(px(420.))
                            .text_center()
                            .text_size(TEXT_MD)
                            .line_height(px(19.5))
                            .text_color(t.text_secondary)
                            .child(text),
                    )
                    .children(link),
            )
            .child(footer)
            .into_any_element()
    }
}

/// The title row: icon, title and line under it, and the close button.
fn header(title: &'static str, sub: &'static str, t: Tokens, cx: &mut Context<'_, ReportDialog>) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .pt(px(20.))
        .px(px(20.))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(px(8.))
                .bg(t.bg_selected)
                .child(icon(Icon::MessageSquareWarning, 17., 2., t.accent)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(
                    div()
                        .font_family(fonts::DISPLAY)
                        .text_size(TEXT_LG)
                        .line_height(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(t.text_secondary)
                        .child(sub),
                ),
        )
        .child(
            button("report-close")
                .ghost()
                .square()
                .icon(Icon::X, 16., 2.)
                .colors(t.text_primary, t.text_primary)
                .tooltip("Close (Esc)")
                .on_click(cx.listener(|d, _, _, cx| d.close(cx))),
        )
}

/// RPT-003: Write · Review · Done, `at` the current one.
fn steps(at: usize, t: Tokens) -> Div {
    let step = |n: usize, name: &'static str| {
        let (dot, ink, weight): (AnyElement, Rgba, FontWeight) = if n < at {
            (
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded_full()
                    .bg(t.bg_selected)
                    .child(icon(Icon::Check, 10., 3., t.accent))
                    .into_any_element(),
                t.text_secondary,
                FontWeight::NORMAL,
            )
        } else if n == at {
            (
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded_full()
                    .bg(t.accent)
                    .text_color(t.text_on_accent)
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(n.to_string())
                    .into_any_element(),
                t.text_primary,
                FontWeight::SEMIBOLD,
            )
        } else {
            (
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded_full()
                    .border_1()
                    .border_color(t.border_strong)
                    .text_size(TEXT_XS)
                    .text_color(t.text_secondary)
                    .child(n.to_string())
                    .into_any_element(),
                t.text_secondary,
                FontWeight::NORMAL,
            )
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_color(ink)
            .font_weight(weight)
            .child(dot)
            .child(name)
    };
    let line = || div().w(px(28.)).h(px(1.)).bg(t.border_strong);
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(20.))
        .pt(px(14.))
        .text_size(TEXT_SM)
        .child(step(1, "Write"))
        .child(line())
        .child(step(2, "Review"))
        .child(line())
        .child(step(3, "Done"))
}

fn footer(t: Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(20.))
        .py(px(14.))
        .border_t(px(0.5))
        .border_color(t.separator)
}

fn label(text: &'static str, t: Tokens) -> Div {
    div()
        .text_size(TEXT_SM)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_secondary)
        .child(text)
}

/// A small icon and a line of text beside it.
fn hint_row(which: Icon, text: &str, icon_ink: Rgba, ink: Rgba) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(7.))
        .min_w_0()
        .text_size(TEXT_SM)
        .line_height(px(17.))
        .text_color(ink)
        .child(icon(which, 13., 2., icon_ink).mt(px(2.)))
        .child(div().min_w_0().child(SharedString::from(text.to_owned())))
}

/// A screenshot, cut to `w × h` from its middle.
fn thumb(shot: &Shot, w: f32, h: f32, t: Tokens) -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(7.))
        .overflow_hidden()
        .border(px(0.5))
        .border_color(t.border_strong)
        .bg(t.bg_sunken)
        .child(img(shot.image.clone()).size_full().object_fit(ObjectFit::Cover))
}

/// The Done screen's round badge.
fn badge(which: Icon, ink: Rgba, _t: Tokens) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(44.))
        .rounded_full()
        .bg(Rgba { a: 0.14, ..ink })
        .child(icon(which, 22., 2.4, ink))
        .into_any_element()
}

fn transparent() -> Rgba {
    crate::components::button::transparent()
}
