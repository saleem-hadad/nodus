//! Where it goes (`RouteMenu`, `.cap-menu`; CAP-107…CAP-112): what to make
//! on the left, the projects (or for into, the sessions) on the right.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, InteractiveElement, IntoElement, MouseDownEvent, ParentElement,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, px,
};
use nodus_core::capture_service::page::{self, kind_copy, sendable, sessions_count};
use nodus_domain::{CaptureKind, CaptureTargets};

use super::panel::CapturePanel;
use super::render::{dot, kbd, kind_icon, swatch, who_dot};
use super::style::{self, Inks};
use crate::components::icon::{Icon, icon};
use crate::components::tooltip::native_tooltip;
use crate::components::tween::enter;
use crate::theme::motion;
use crate::theme::tokens::{DUR_FAST, Tokens};

/// The menu's top and left in the card.
const MENU_TOP: f32 = 50.0;
const MENU_LEFT: f32 = 8.0;
const MENU_W: f32 = 524.0;
/// A column's tallest, before it scrolls.
const COLUMN_MAX: f32 = 300.0;
/// The scrollbar's gutter (`global.css`).
const GUTTER: f32 = 10.0;

impl CapturePanel {
    pub(super) fn render_menu(
        &mut self,
        t: Tokens,
        ink: Inks,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let Some(targets) = self.targets.clone() else {
            return div().into_any_element();
        };
        let route = self.route();
        // CAP-108: why Claude guessed so, while it's Claude's guess.
        let reason = (self.choice.is_none())
            .then(|| self.guess.guess.as_ref().map(|g| g.reason.clone()))
            .flatten()
            .filter(|r| !r.is_empty());
        let rise = enter(
            window,
            cx,
            ("cap-menu-rise", self.menu_epoch as usize),
            DUR_FAST,
            motion::ease_out,
        );
        let can_into = targets.sessions.iter().any(sendable);

        // CAP-109: what to make.
        let mut make: Vec<AnyElement> = vec![column_label(&t).child("Make").into_any_element()];
        for (n, kind) in CaptureKind::ALL.iter().copied().enumerate() {
            let copy = kind_copy(kind);
            let disabled = kind == CaptureKind::Into && !can_into;
            let hint = if disabled { "No sessions are running" } else { copy.hint };
            make.push(
                option(("cap-kind", n), route.kind == kind, disabled, t)
                    .items_start()
                    .gap(px(10.))
                    .when(!disabled, |d| {
                        d.on_click(cx.listener(move |p, _, window, cx| {
                            p.pick_kind(kind, cx);
                            p.focus_field(window, cx);
                        }))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .w(px(16.))
                            .mt(px(1.))
                            .child(kind_icon(kind, 16., t, ink)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .min_w_0()
                            .child(copy.label)
                            .child(div().text_size(px(11.)).text_color(t.text_tertiary).child(hint)),
                    )
                    .into_any_element(),
            );
        }

        let where_to = if route.kind == CaptureKind::Into {
            self.render_sessions(&targets, t, ink, cx)
        } else {
            self.render_projects(&targets, t, cx)
        };

        let route_bounds = self.route_bounds.clone();
        let menu_bottom = self.menu_bottom.clone();
        div()
            .id("cap-menu")
            .absolute()
            .top(px(MENU_TOP))
            .left(px(MENU_LEFT))
            .w(px(MENU_W))
            .overflow_hidden()
            .rounded(px(12.))
            .bg(t.bg_elevated)
            .shadow(t.lg())
            .when(rise < 1.0, |d| {
                d.group_opacity(rise)
                    .translate(px(0.), px(-6.0 * (1.0 - rise)))
                    .scale(0.985 + 0.015 * rise)
            })
            // CAP-107: a press outside the menu and the header closes it; the
            // press still goes on to what it hit.
            .on_mouse_down_out(cx.listener(move |p, e: &MouseDownEvent, _, cx| {
                if !route_bounds.get().is_some_and(|b| b.contains(&e.position)) {
                    p.close_menu(cx);
                }
            }))
            .children(reason.map(|reason| {
                let lead = "Claude's guess:";
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.separator)
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(t.text_secondary)
                    .child(div().mt(px(6.)).child(who_dot(t, None)))
                    .child(div().flex_1().min_w_0().child(style::spans(
                        window,
                        &[
                            (lead, t.text_primary, Some(FontWeight::SEMIBOLD)),
                            (&format!(" {reason}"), t.text_secondary, None),
                        ],
                    )))
            }))
            .child(
                div()
                    .flex()
                    .child(column("cap-menu-make", false, make, t, window, cx))
                    .child(column("cap-menu-where", true, where_to, t, window, cx)),
            )
            // CAP-112
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(14.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.separator)
                    .text_size(px(11.))
                    .text_color(t.text_tertiary)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(kbd("Tab", t))
                            .child("\u{a0}next kind"),
                    )
                    .child(div().flex().items_center().child(kbd("Esc", t)).child("\u{a0}close")),
            )
            .child(
                canvas(
                    move |b, _, _| menu_bottom.set(Some(f32::from(b.bottom()))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }

    /// CAP-110: every project, the route's ticked.
    fn render_projects(&self, targets: &CaptureTargets, t: Tokens, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let route = self.route();
        let mut items = vec![column_label(&t).child("Project").into_any_element()];
        for (i, project) in targets.projects.iter().enumerate() {
            let selected = route.project_id.as_deref() == Some(project.id.as_str());
            let id = project.id.clone();
            items.push(
                option(("cap-project", i), selected, false, t)
                    .items_center()
                    .gap(px(8.))
                    .on_click(cx.listener(move |p, _, window, cx| p.pick_project(&id, window, cx)))
                    .child(swatch(Some(project.color)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(project.name.clone())),
                    )
                    .when(selected, |d| d.child(check(t)))
                    .into_any_element(),
            );
        }
        items
    }

    /// CAP-111: the route project's sessions, waiting ones included, then
    /// every other project.
    fn render_sessions(
        &self,
        targets: &CaptureTargets,
        t: Tokens,
        ink: Inks,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let route = self.route();
        let project = targets
            .projects
            .iter()
            .find(|p| route.project_id.as_deref() == Some(p.id.as_str()));
        let mut items = vec![
            column_label(&t)
                .child(swatch(project.map(|p| p.color)))
                .child(SharedString::from(project.map(|p| p.name.clone()).unwrap_or_default()))
                .into_any_element(),
        ];
        let here: Vec<_> = targets
            .sessions
            .iter()
            .filter(|s| route.project_id.as_deref() == Some(s.project_id.as_str()))
            .take(page::MENU_SESSIONS)
            .cloned()
            .collect();
        for (i, session) in here.iter().enumerate() {
            let selected = route.session_id.as_deref() == Some(session.session_id.as_str());
            let open = sendable(session);
            let picked = session.clone();
            items.push(
                option(("cap-session", i), selected, !open, t)
                    .items_center()
                    .gap(px(8.))
                    .when(open, |d| {
                        d.on_click(cx.listener(move |p, _, window, cx| p.pick_session(&picked, window, cx)))
                    })
                    .when(!open, |d| {
                        d.tooltip(native_tooltip(
                            "It’s waiting for your answer. Answer it on the board first.".into(),
                        ))
                    })
                    .child(dot(Some(session.status), t))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(session.title.clone())),
                    )
                    .map(|d| {
                        if !open {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ink.waiting)
                                    .child("Needs you"),
                            )
                        } else if selected {
                            d.child(check(t))
                        } else {
                            d
                        }
                    })
                    .into_any_element(),
            );
        }
        if here.is_empty() {
            items.push(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(t.text_tertiary)
                    .child("No sessions on this board")
                    .into_any_element(),
            );
        }
        items.push(div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.separator).into_any_element());
        for (i, other) in targets
            .projects
            .iter()
            .filter(|p| route.project_id.as_deref() != Some(p.id.as_str()))
            .enumerate()
        {
            let count = targets.sessions.iter().filter(|s| s.project_id == other.id).count();
            let id = other.id.clone();
            items.push(
                option(("cap-other", i), false, false, t)
                    .items_center()
                    .gap(px(8.))
                    .on_click(cx.listener(move |p, _, window, cx| p.pick_project(&id, window, cx)))
                    .child(swatch(Some(other.color)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(other.name.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(t.text_tertiary)
                            .child(SharedString::from(sessions_count(count))),
                    )
                    .into_any_element(),
            );
        }
        items
    }
}

/// `.cap-menu-label`.
fn column_label(t: &Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .pt(px(2.))
        .px(px(8.))
        .pb(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_tertiary)
}

/// `.cap-option`: hover, selected and disabled looks.
fn option(id: impl Into<ElementId>, selected: bool, disabled: bool, t: Tokens) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .px(px(8.))
        .py(px(7.))
        .rounded(px(8.))
        .text_size(px(13.))
        .when(selected, |d| d.bg(t.bg_selected))
        .when(disabled, |d| d.opacity(0.55))
        .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(t.bg_hover)))
}

/// The tick on what's picked (`Check` 14 in `--into-ink`).
fn check(t: Tokens) -> impl IntoElement {
    let ink = style::inks(&t);
    icon(Icon::Check, 14., 2., ink.into)
}

/// `.cap-menu-col`: a column of options, scrolling past 300 px with the
/// app's 10 px scrollbar gutter; the second has a hairline on its left.
fn column(
    id: &'static str,
    divided: bool,
    items: Vec<AnyElement>,
    t: Tokens,
    window: &mut Window,
    cx: &mut Context<'_, CapturePanel>,
) -> AnyElement {
    let handle = window
        .use_keyed_state(ElementId::Name(format!("{id}-scroll").into()), cx, |_, _| {
            ScrollHandle::new()
        })
        .read(cx)
        .clone();
    let overflow = window.use_keyed_state(ElementId::Name(format!("{id}-overflow").into()), cx, |_, _| false);
    let overflowing = *overflow.read(cx);
    let max = handle.max_offset().y;
    let offset = -handle.offset().y;
    let viewport = handle.bounds().size.height;
    let watch = handle.clone();
    let view = window.current_view();
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .max_h(px(COLUMN_MAX))
        .when(divided, |d| d.border_l_1().border_color(t.separator))
        .child(
            div()
                .id(id)
                .flex()
                .flex_col()
                .flex_auto()
                .min_h_0()
                .gap(px(2.))
                .pt(px(8.))
                .pb(px(8.))
                .pl(px(8.))
                .pr(px(8. + if overflowing { GUTTER } else { 0. }))
                .overflow_y_scroll()
                .track_scroll(&handle)
                .children(items),
        )
        .when(overflowing && viewport > px(0.), |d| {
            let content = viewport + max;
            let thumb = (viewport * (viewport / content)).max(px(18.));
            let travel = viewport - thumb;
            let top = if max > px(0.) {
                travel * (f32::from(offset) / f32::from(max))
            } else {
                px(0.)
            };
            d.child(
                div()
                    .absolute()
                    .top(top + px(3.))
                    .right(px(3.))
                    .w(px(GUTTER - 6.))
                    .h(thumb - px(6.))
                    .rounded(px(10.))
                    .bg(t.border_strong),
            )
        })
        .child(
            canvas(
                move |_, _, cx| {
                    let now = watch.max_offset().y > px(0.5);
                    let changed = overflow.update(cx, |o, _| std::mem::replace(o, now) != now);
                    if changed {
                        cx.notify(view);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}
