//! The title bar (`TitleBar.tsx`, `TitleBar.css`, `RunningCount.tsx`,
//! `UpdateButton.tsx`; UI-088…UI-108).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, deferred, div, px,
};
use nodus_domain::{SettingsPatch, UpdateState};

use crate::components::{Icon, button, folder_icon, icon, native_tooltip, spaced_text, switch};
use crate::platform::{Platform, keys};
use crate::state::{AppState, Route};
use crate::theme::tokens::*;
use crate::theme::{self, fonts};
use nodus_sessions::running::{running_by_project, running_total};

/// The windows caption buttons' width, which the bar leaves free off a Mac.
pub const CAPTION_W: f32 = 138.0;

/// The 52px title bar. `actions` are the open view's own buttons (UI-098).
pub fn title_bar(state: &Entity<AppState>, actions: Vec<AnyElement>, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let platform = Platform::CURRENT;
    let s = state.read(cx);
    let route = s.route.clone();
    let project = s.route_project().cloned();
    let focus_feature = s.focus_feature();
    let focus_mode = s.settings.as_ref().is_some_and(|s| s.focus_mode);
    let total = running_total(s.stats.values());
    let breakdown: Vec<_> = running_by_project(&s.projects, &s.stats)
        .into_iter()
        .map(|row| {
            let color = s.projects.iter().find(|p| p.id == row.id).map(|p| p.color);
            (row, color)
        })
        .collect();
    let open_project = s.route.project_id().map(str::to_owned);
    let ready_version = match &s.update_status {
        Some(status) => match &status.state {
            UpdateState::Ready { version } => Some(version.clone()),
            _ => None,
        },
        None => None,
    };

    let name_font = gpui::Font {
        family: fonts::DISPLAY.into(),
        weight: FontWeight::SEMIBOLD,
        ..gpui::Font::default()
    };
    // `--text-lg`, 600, −0.01em.
    let current_name =
        |text: String| spaced_text(text, name_font.clone(), TEXT_LG, px(-0.15), px(21.), t.text_primary).truncate();

    let go_to_projects = {
        let state = state.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            state.update(cx, |s, cx| s.navigate(Route::Projects, cx));
        }
    };

    let crumbs = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .min_w_0()
        .when(!platform.is_mac(), Styled::mr_auto)
        .map(|d| match &route {
            Route::Projects => d.child(
                // UI-089: the word Projects, part of the drag region.
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .min_w_0()
                    .pl(px(4.))
                    .child(current_name("Projects".into())),
            ),
            Route::Board(_) => {
                let tooltip = project.as_ref().map(|p| {
                    format!(
                        "{}\nDouble-click to {}",
                        p.repo_path,
                        if platform.is_mac() {
                            "reveal in Finder"
                        } else {
                            "show in File Explorer"
                        }
                    )
                });
                let reveal = {
                    let state = state.clone();
                    let id = project.as_ref().map(|p| p.id.clone());
                    move |e: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                        // UI-094: reveals, and never zooms the window.
                        if e.click_count() == 2
                            && let Some(id) = id.clone()
                        {
                            state.update(cx, |s, cx| s.reveal(id, cx));
                        }
                    }
                };
                d.child(no_drag(
                    div()
                        .id("titlebar-crumb-link")
                        .flex()
                        .items_center()
                        .h(px(24.))
                        .px(px(6.))
                        .rounded(RADIUS_SM)
                        .text_color(t.text_secondary)
                        .font_weight(FontWeight::MEDIUM)
                        .hover(|s| s.bg(t.bg_hover).text_color(t.text_primary))
                        .active(|s| s.bg(t.bg_pressed))
                        .on_click(go_to_projects)
                        .child("Projects"),
                ))
                .child(icon(Icon::ChevronRight, 13., 2.2, t.text_tertiary))
                .child(
                    no_drag(
                        div()
                            .id("titlebar-current")
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .min_w_0()
                            .pl(px(4.))
                            .when_some(tooltip, |d, tip| d.tooltip(native_tooltip(tip.into())))
                            .on_click(reveal)
                            .child(folder_icon(18., project.as_ref().map(|p| p.color)))
                            .child(current_name(
                                project.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                            )),
                    )
                    .min_w_0(),
                )
                // The git controls (05), and the Changes window.
                .children(crate::git::title_bar_git(state, window, cx))
            }
        });

    let running = (total > 0).then(|| {
        // Over the count, and over its list: the list stays while the
        // pointer is on either, so its projects can be clicked.
        let hover = window.use_keyed_state("titlebar-running", cx, |_, _| (false, false));
        let (on_count, on_list) = *hover.read(cx);
        let list_hover = hover.clone();
        let rows = breakdown
            .into_iter()
            .enumerate()
            .map(|(i, (row, color))| {
                let state = state.clone();
                let hover = hover.clone();
                let open = open_project.as_deref() != Some(row.id.as_str());
                div()
                    .id(("titlebar-running-row", i))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(24.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(t.bg_hover))
                    .active(|s| s.bg(t.bg_pressed))
                    .on_click(move |_, _, cx| {
                        hover.update(cx, |s, cx| {
                            *s = (false, false);
                            cx.notify();
                        });
                        if open {
                            let id = row.id.clone();
                            state.update(cx, |s, cx| s.navigate(Route::Board(id), cx));
                        }
                    })
                    .child(folder_icon(14., color))
                    .child(div().flex_1().min_w_0().truncate().child(row.name))
                    .child(
                        div()
                            .flex_none()
                            .text_color(t.text_tertiary)
                            .child(row.count.to_string()),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .id("titlebar-running")
            .relative()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(24.))
            .px(px(6.))
            .text_size(TEXT_SM)
            .font_features(fonts::tabular())
            .text_color(t.text_secondary)
            .whitespace_nowrap()
            .on_hover(move |h, _, cx| {
                hover.update(cx, |s, cx| {
                    s.0 = *h;
                    cx.notify();
                });
            })
            .child(icon(Icon::SquareTerminal, 13., 2., t.text_secondary))
            .child(format!("{total} running"))
            .when(on_count || on_list, |d| {
                // UI-105: hangs 6px below, right edges lined up, over the main
                // area. The 6px gap is part of the list, so the pointer keeps
                // it open on the way down; occluding keeps its clicks from
                // moving the window.
                d.child(
                    deferred(
                        div()
                            .id("titlebar-running-list")
                            .occlude()
                            .absolute()
                            .top(px(24.))
                            .right(px(0.))
                            .pt(px(6.))
                            .on_hover(move |h, _, cx| {
                                list_hover.update(cx, |s, cx| {
                                    s.1 = *h;
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .min_w(px(160.))
                                    .max_w(px(280.))
                                    .p(px(4.))
                                    .rounded(RADIUS_SM)
                                    .bg(t.bg_elevated)
                                    .shadow(t.md())
                                    .text_size(TEXT_SM)
                                    .text_color(t.text_primary)
                                    .children(rows),
                            ),
                    )
                    .with_priority(50),
                )
            })
    });

    let focus_switch = focus_feature.then(|| {
        let toggle = {
            let state = state.clone();
            move |checked: bool, cx: &mut App| {
                state.update(cx, |s, cx| {
                    s.update_settings(
                        SettingsPatch {
                            focus_mode: Some(checked),
                            ..SettingsPatch::default()
                        },
                        cx,
                    );
                });
            }
        };
        let label_toggle = toggle.clone();
        div()
            .id("titlebar-focus")
            .flex()
            .items_center()
            .gap(px(7.))
            .px(px(4.))
            .text_color(t.text_secondary)
            .font_weight(FontWeight::MEDIUM)
            .tooltip(native_tooltip(
                format!("Show only the projects in focus ({})", keys("⇧⌘F")).into(),
            ))
            .child(
                div()
                    .id("titlebar-focus-label")
                    .on_click(move |_, _, cx| label_toggle(!focus_mode, cx))
                    .child("Focus"),
            )
            .child(switch("titlebar-focus-switch", focus_mode).on_change(move |checked, _, cx| toggle(checked, cx)))
    });

    let update_button = ready_version.map(|version| {
        let state = state.clone();
        let tip = if platform.is_mac() {
            format!("Nodus {version} is ready. Sessions keep running while Nodus restarts.")
        } else {
            format!("Nodus {version} is ready. Restarting stops the sessions; you can resume them after.")
        };
        button("titlebar-update")
            .primary()
            .icon(Icon::RotateCw, 14., 2.2)
            .label("Restart to Update")
            .tooltip(tip)
            // UI-108
            .on_click(move |_, _, cx| {
                let nodus = state.read(cx).nodus.clone();
                crate::shell::core_shell::install_update(nodus, cx);
            })
    });

    let right = no_drag(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .children(running)
            .children(focus_switch)
            .children(update_button)
            .child(div().flex().items_center().gap(px(8.)).children(actions)),
    )
    .flex_none();

    div()
        .id("titlebar")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .h(TITLEBAR_H)
        .map(|d| {
            if platform.is_mac() {
                d.pl(px(84.)).pr(px(14.))
            } else {
                d.pl(px(8.)).pr(px(14. + CAPTION_W))
            }
        })
        // `--bg-toolbar` over a 24px blur of the window behind it, which is the
        // plain window background: the composite, without a layer to pay for.
        .bg(t.titlebar_surface)
        // The hairline is drawn over the bar's bottom rather than taking
        // layout room, so the controls centre on the whole 52px and land on
        // the device pixels Chromium snaps them to.
        .child(div().absolute().left_0().right_0().bottom_0().h(px(0.5)).bg(t.border))
        // UI-014: the empty bar moves the window; a double-click does the
        // OS's title-bar action.
        .on_mouse_down(MouseButton::Left, |e, window, _| {
            if e.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .when(!platform.is_mac(), |d| {
            d.child(
                no_drag(
                    button("titlebar-menu")
                        .ghost()
                        .square()
                        .icon(Icon::Menu, 16., 2.)
                        .colors(t.text_secondary, t.text_primary)
                        .tooltip("Menu")
                        // UI-099, PLAT-060: the whole application menu.
                        .on_click(|_, _, cx| {
                            crate::shell::windows_chrome::AppMenu::toggle(
                                crate::shell::windows_chrome::menu_origin(),
                                cx,
                            );
                        }),
                )
                .mr(px(-8.)),
            )
        })
        // PLAT-018: the caption buttons in the room the bar leaves them.
        .when(!platform.is_mac(), |d| {
            d.child(crate::shell::windows_chrome::caption_buttons(window, cx))
        })
        .child(crumbs)
        .child(right)
        .into_any_element()
}

/// A part of the bar that takes its own clicks instead of moving the window.
fn no_drag(child: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(child)
}

/// The grid's New Project button (UI-186).
pub fn new_project_button(state: &Entity<AppState>) -> AnyElement {
    let state = state.clone();
    button("new-project")
        .icon(Icon::Plus, 15., 2.2)
        .label("New Project")
        .tooltip(format!("New Project ({})", keys("⇧⌘N")))
        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.add_project(None, false, cx)))
        .into_any_element()
}
