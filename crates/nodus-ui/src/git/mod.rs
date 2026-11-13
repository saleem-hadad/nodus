//! The project's git status in the window (`GitControls.tsx`, `git.css`, the
//! store's `git`, `refreshGit`, `openDiff`/`closeDiff`; GIT-021…GIT-031): the
//! branch chip with its last commits dropping down, the Changes button, and
//! the Changes window ([`diff_modal`]).
//!
//! [`GitModel`] is the store's `git`: one per window, read on the open board's
//! project, shared by the title bar and the Changes window. The title bar
//! places [`title_bar_git`] after the project's name.

pub mod diff_modal;

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FontWeight, Global, InteractiveElement,
    IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, deferred, div, px,
};
use nodus_domain::{Event, GitCommit, GitStatus};
use nodus_git::labels;

use crate::actions::icons::{Glyph, glyph};
use crate::components::native_tooltip;
use crate::components::tween::enter;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

use self::diff_modal::DiffModal;

/// GIT-023, GIT-041: `GIT_POLL_MS`.
pub const GIT_POLL: Duration = labels::GIT_POLL;

/// Something the Changes window waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitEvent {
    /// A status read settled (whatever it found).
    Settled,
    /// The window gained focus.
    Focused,
}

/// The window's git status (the store's `git`), its reads, and the branch
/// log's popover.
pub struct GitModel {
    pub state: Entity<AppState>,
    /// GIT-024: none when the folder isn't a repository, git is missing, the
    /// read failed, or nothing has been read yet.
    pub status: Option<GitStatus>,
    /// The board the status belongs to.
    route_project: Option<String>,
    /// On a server board, the selected card whose worktree it's of
    /// (SRV-128); none for the project's own folder.
    pub card: Option<String>,
    /// GIT-021: the read in flight, shared by calls for the same project.
    reading: Option<(String, u64)>,
    next_read: u64,
    /// Reads settled so far: the Changes window waits for one (GIT-033).
    pub settled: u64,
    /// Whether the window has focus (`document.hasFocus()`).
    active: bool,
    /// GIT-027: the log popover, and its pending open or close.
    log_open: bool,
    log_timer: Option<Task<()>>,
    /// Whether the pointer is over the chip, and over the log: one area to
    /// the pointer, as the log is inside the chip's wrapper in 0.9.4.
    over_chip: bool,
    over_log: bool,
    /// GIT-028: the last log read, by `<projectId>:<oid>`, and the one in flight.
    log: Option<(String, Vec<GitCommit>)>,
    log_reading: Option<String>,
    /// The Changes window while it's open.
    diff: Option<Entity<DiffModal>>,
    _poll: Task<()>,
    _events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GitEvent> for GitModel {}

/// SRV-128: on a server board, the one selected card when it has a
/// worktree, whose git the chip and the Changes window show; none otherwise,
/// for the project's own folder.
fn worktree_card(state: &AppState) -> Option<String> {
    let [selected] = state.selection.as_slice() else {
        return None;
    };
    let board = state.board.as_ref()?;
    if board.project.server.is_none()
        && !state
            .projects
            .iter()
            .any(|p| p.id == board.project.id && p.server.is_some())
    {
        return None;
    }
    board
        .cards
        .iter()
        .find(|card| card.node.session_id == *selected)
        .filter(|card| card.node.worktree().is_some())
        .map(|card| card.node.session_id.clone())
}

/// The window's model.
struct GitGlobal(Entity<GitModel>);

impl Global for GitGlobal {}

impl GitModel {
    /// The window's model, made the first time the title bar asks for it.
    pub fn get(state: &Entity<AppState>, window: &mut Window, cx: &mut App) -> Entity<GitModel> {
        if let Some(model) = cx.try_global::<GitGlobal>() {
            return model.0.clone();
        }
        let state = state.clone();
        let model = cx.new(|cx| GitModel::new(state, window, cx));
        cx.set_global(GitGlobal(model.clone()));
        model
    }

    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let nodus = state.read(cx).nodus.clone();
        // GIT-023: every session status event re-reads it (Claude edits files
        // during a turn, so a status change usually means new changes).
        let mut rx = nodus.subscribe();
        let events = cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(Event::Status { .. }) => {
                        if this.update(cx, GitModel::refresh).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        // GIT-023: every 4 s while the window has focus, catching branch
        // switches and edits made in terminals or other apps.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GIT_POLL).await;
                let alive = this.update(cx, |m, cx| {
                    if m.active {
                        m.refresh(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let subscriptions = vec![
            // GIT-022, GIT-023: a new route resets it; a board reads it.
            cx.observe(&state, |m, state, cx| {
                let project = state.read(cx).route.project_id().map(str::to_owned);
                let card = worktree_card(state.read(cx));
                if project != m.route_project || card != m.card {
                    m.route_project = project;
                    m.card = card;
                    m.status = None;
                    m.log_open = false;
                    m.log_timer = None;
                    m.over_chip = false;
                    m.over_log = false;
                    m.refresh(cx);
                    cx.notify();
                }
                m.sync_diff(cx);
            }),
            // GIT-023: the window gaining focus re-reads it.
            cx.observe_window_activation(window, |m, window, cx| {
                m.active = window.is_window_active();
                if m.active {
                    m.refresh(cx);
                    cx.emit(GitEvent::Focused);
                }
            }),
        ];
        let route_project = state.read(cx).route.project_id().map(str::to_owned);
        let card = worktree_card(state.read(cx));
        let mut model = Self {
            state,
            status: None,
            route_project,
            card,
            reading: None,
            next_read: 0,
            settled: 0,
            active: window.is_window_active(),
            log_open: false,
            log_timer: None,
            over_chip: false,
            over_log: false,
            log: None,
            log_reading: None,
            diff: None,
            _poll: poll,
            _events: events,
            _subscriptions: subscriptions,
        };
        model.refresh(cx);
        model.sync_diff(cx);
        model
    }

    /// GIT-021 (`refreshGit`): reads the board's status. Off a board it does
    /// nothing; calls while a read for the same project is in flight share
    /// it; a failed read is none; an answer for a board the window has left
    /// is dropped; the status is replaced only when it changed.
    pub fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        let Some(project) = self.route_project.clone() else {
            return;
        };
        if self.reading.as_ref().is_some_and(|(p, _)| *p == project) {
            return;
        }
        self.next_read += 1;
        let id = self.next_read;
        self.reading = Some((project.clone(), id));
        let nodus = self.state.read(cx).nodus.clone();
        let card = self.card.clone();
        cx.spawn(async move |this, cx| {
            let git = nodus.git_for(card.as_deref()).status(&project).await.ok().flatten();
            let _ = this.update(cx, |m, cx| {
                if m.reading.as_ref().is_some_and(|(_, r)| *r == id) {
                    m.reading = None;
                }
                m.settled += 1;
                if m.route_project.as_deref() == Some(project.as_str()) && m.card == card && m.status != git {
                    m.status = git;
                    m.ensure_log(cx);
                    cx.notify();
                }
                cx.emit(GitEvent::Settled);
            });
        })
        .detach();
    }

    /// The pointer entered or left the chip (`log` false) or the log.
    fn set_over(&mut self, log: bool, over: bool, cx: &mut Context<'_, Self>) {
        let before = self.over_chip || self.over_log;
        if log {
            self.over_log = over;
        } else {
            self.over_chip = over;
        }
        let after = self.over_chip || self.over_log;
        if before != after {
            self.hover_log(after, cx);
        }
    }

    /// GIT-027: pointing at the chip opens the log after 150 ms, leaving
    /// closes it after 150 ms; each restarts the wait, so passing over does
    /// nothing and the pointer can cross the gap into the log.
    fn hover_log(&mut self, wanted: bool, cx: &mut Context<'_, Self>) {
        let delay = if wanted {
            labels::LOG_OPEN_DELAY
        } else {
            labels::LOG_CLOSE_DELAY
        };
        self.log_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |m, cx| {
                m.log_timer = None;
                if m.log_open != wanted {
                    m.log_open = wanted;
                    if !wanted {
                        m.over_log = false;
                    }
                    m.ensure_log(cx);
                    cx.notify();
                }
            });
        }));
    }

    /// GIT-028: an open log reads the branch's last five commits, unless the
    /// ones it has are for this HEAD; an answer that lands after it closed or
    /// HEAD moved is dropped, and a failure leaves the head line alone.
    fn ensure_log(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(project), Some(git)) = (self.route_project.clone(), self.status.as_ref()) else {
            return;
        };
        if !self.log_open || git.oid.is_none() {
            return;
        }
        let key = labels::log_key(&project, git);
        if self.log.as_ref().is_some_and(|(k, _)| *k == key) || self.log_reading.as_deref() == Some(key.as_str()) {
            return;
        }
        self.log_reading = Some(key.clone());
        let nodus = self.state.read(cx).nodus.clone();
        let card = self.card.clone();
        cx.spawn(async move |this, cx| {
            let commits = nodus.git_for(card.as_deref()).log(&project, None).await;
            let _ = this.update(cx, |m, cx| {
                if m.log_reading.as_deref() == Some(key.as_str()) {
                    m.log_reading = None;
                }
                let current = m
                    .route_project
                    .as_deref()
                    .zip(m.status.as_ref())
                    .map(|(p, g)| labels::log_key(p, g));
                if let (Ok(commits), true, true) = (commits, m.log_open, current.as_deref() == Some(key.as_str())) {
                    m.log = Some((key, commits));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The commits the popover lists, when they're for this HEAD (GIT-030).
    fn commits(&self) -> Option<&[GitCommit]> {
        let project = self.route_project.as_deref()?;
        let git = self.status.as_ref()?;
        git.oid.as_ref()?;
        let key = labels::log_key(project, git);
        self.log
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, c)| c.as_slice())
            .filter(|c| !c.is_empty())
    }

    /// GIT-032: the Changes window shows while `diffOpen` is set on a board.
    fn sync_diff(&mut self, cx: &mut Context<'_, Self>) {
        let (open, project) = {
            let s = self.state.read(cx);
            (s.diff_open, s.route.project_id().map(str::to_owned))
        };
        let reopen = match (open, &project, &self.diff) {
            (true, Some(_), None) => true,
            (true, Some(project), Some(diff)) => diff.read(cx).project_id != *project,
            _ => false,
        };
        if reopen && let Some(project) = project {
            let model = cx.entity();
            let settled = self.settled;
            self.diff = Some(cx.new(|cx| DiffModal::new(model, project, settled, cx)));
            // GIT-033: the status may be a few seconds old; it's read again
            // before the folder is called clean.
            self.refresh(cx);
            cx.notify();
            return;
        }
        if let (false, Some(diff)) = (open && project.is_some(), &self.diff) {
            // Focus goes back where it was, however it closed (⌘W too).
            if let Some(previous) = diff.read(cx).previous_focus() {
                cx.defer(move |cx| {
                    if let Some(handle) = crate::shell::window::main_window(cx) {
                        let _ = handle.update(cx, |_, window, cx| {
                            if window.focused(cx).is_none() {
                                window.focus(&previous, cx);
                            }
                        });
                    }
                });
            }
            self.diff = None;
            cx.notify();
        }
    }
}

/// `openDiff`.
pub fn open_diff(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        s.diff_open = true;
        cx.notify();
    });
}

/// `closeDiff`.
pub fn close_diff(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        if s.diff_open {
            s.diff_open = false;
            cx.notify();
        }
    });
}

/// What the title bar shows after the project's name on a board: the branch
/// chip and the Changes button (GIT-024: neither without a status), and the
/// Changes window while it's open.
pub fn title_bar_git(state: &Entity<AppState>, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let model = GitModel::get(state, window, cx);
    let mut out = Vec::new();
    let m = model.read(cx);
    if let Some(git) = m.status.clone() {
        out.push(branch_chip(&model, &git, window, cx));
        out.push(changes_button(state, git.files.len(), window, cx));
    }
    let diff = model.read(cx).diff.clone();
    if let Some(diff) = diff {
        // Out of the title bar's flow: the window is drawn over everything.
        out.push(div().absolute().child(diff).into_any_element());
    }
    out
}

/// The branch chip and its log (GIT-026…GIT-031).
fn branch_chip(model: &Entity<GitModel>, git: &GitStatus, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let m = model.read(cx);
    let open = m.log_open;
    let commits: Option<Vec<GitCommit>> = m.commits().map(<[GitCommit]>::to_vec);
    let hover = |model: &Entity<GitModel>, log: bool| {
        let model = model.clone();
        move |h: &bool, _: &mut Window, cx: &mut App| {
            let over = *h;
            model.update(cx, |m, cx| m.set_over(log, over, cx));
        }
    };

    // `.git-branch`: 22px tall, rounded, sunken, with a hairline ring.
    let chip = div()
        .id("git-branch")
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w_0()
        .max_w(px(260.))
        .h(px(22.))
        .px(px(8.))
        .rounded(px(11.))
        .bg(t.bg_sunken)
        .shadow(vec![inset_shadow(0., 0., 0., 0.5, t.border)])
        .text_color(t.text_secondary)
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .font_weight(FontWeight::MEDIUM)
        .child(glyph(Glyph::GitBranch, 13., 2.2, t.text_secondary))
        .child(div().min_w_0().truncate().child(labels::branch_label(git)))
        .children(labels::ahead_behind(git).into_iter().map(|ab| {
            div()
                .flex_none()
                .text_color(t.text_tertiary)
                .font_features(fonts::tabular())
                .child(ab)
        }));

    let popover = open.then(|| {
        // GIT-029: the head, then (GIT-030) the commits for this HEAD.
        let now = crate::parity::now_ms(cx);
        let rise = enter(window, cx, "git-log-in", Duration::from_millis(90), motion::ease_out);
        let rows: Vec<AnyElement> = commits
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                div()
                    .id(("git-log-commit", i))
                    .flex()
                    .flex_col()
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .hover(|s| s.bg(t.bg_hover))
                    .child(
                        div()
                            .flex()
                            .gap(px(10.))
                            .child(
                                div()
                                    .id(("git-log-subject", i))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .tooltip(native_tooltip(SharedString::from(c.subject.clone())))
                                    .child(c.subject.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(t.text_tertiary)
                                    .font_features(fonts::tabular())
                                    .whitespace_nowrap()
                                    .child(time_ago(c.time, now)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .text_size(TEXT_XS)
                            .line_height(px(15.4))
                            .text_color(t.text_tertiary)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(fonts::mono())
                                    .child(nodus_domain::js::slice_utf16(&c.oid, 0, 7)),
                            )
                            .child(div().min_w_0().truncate().child(format!(" · {}", c.author))),
                    )
                    .into_any_element()
            })
            .collect();
        deferred(
            div()
                .id("git-log")
                .absolute()
                .top(px(22.))
                .left_0()
                // The 6px above it bridges to the chip, so the pointer can
                // cross into the log without leaving.
                .pt(px(6.))
                .occlude()
                .on_hover(hover(model, true))
                .child(
                    div()
                        .w(px(360.))
                        .p(px(5.))
                        .rounded(RADIUS_MD)
                        .bg(t.bg_elevated)
                        .shadow(t.lg())
                        .text_color(t.text_primary)
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .font_weight(FontWeight::NORMAL)
                        .translate(px(0.), px(-2. * (1. - rise)))
                        .opacity(rise)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .pt(px(4.))
                                .px(px(8.))
                                .pb(px(6.))
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(labels::log_branch(git).to_owned()),
                                )
                                .child(div().text_color(t.text_secondary).child(labels::tracking_label(git))),
                        )
                        .when(!rows.is_empty(), |d| {
                            d.child(
                                div()
                                    .relative()
                                    .pt(px(4.))
                                    .child(div().absolute().top_0().left_0().right_0().h(px(0.5)).bg(t.separator))
                                    .children(rows),
                            )
                        }),
                ),
        )
        .with_priority(50)
    });

    div()
        .id("git-branch-wrap")
        .relative()
        .flex()
        .min_w_0()
        .ml(px(6.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_hover(hover(model, false))
        .child(chip)
        .children(popover)
        .into_any_element()
}

/// GIT-025: the Changes button.
fn changes_button(state: &Entity<AppState>, count: usize, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let hovered = window.use_keyed_state("git-changes-hover", cx, |_, _| false);
    let on = *hovered.read(cx);
    let color = if on { t.text_primary } else { t.text_secondary };
    let state = state.clone();
    div()
        .id("git-changes")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .h(px(24.))
        .ml(px(2.))
        .px(px(8.))
        .rounded(RADIUS_SM)
        .text_color(color)
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .font_weight(FontWeight::MEDIUM)
        .when(on, |d| d.bg(t.bg_hover))
        .active(|s| s.bg(t.bg_pressed))
        .on_hover(move |h, _, cx| {
            hovered.update(cx, |s, cx| {
                *s = *h;
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(native_tooltip(labels::changes_title(count).into()))
        .on_click(move |_, _, cx| open_diff(&state, cx))
        .child(glyph(Glyph::FileDiff, 14., 2., color))
        .child("Changes")
        .children(labels::changes_badge(count).map(|badge| {
            div()
                .min_w(px(18.))
                .px(px(5.))
                .rounded(px(9.))
                .bg(t.accent)
                .text_color(t.text_on_accent)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(16.))
                .font_features(fonts::tabular())
                .flex()
                .justify_center()
                .child(badge)
        }))
        .into_any_element()
}

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// GIT-031: `timeAgo` (CARD-041's), worked out when drawn: `just now`,
/// `<n>m ago`, `<n>h ago`, `<n>d ago`, then a short date in the local time
/// zone as en-US writes it (`Oct 3`, with the year past 300 days).
pub fn time_ago(ts: f64, now: f64) -> String {
    time_ago_with(ts, now, local_date)
}

pub fn time_ago_with(ts: f64, now: f64, date: impl Fn(f64) -> (i64, u32, u32)) -> String {
    if ts == 0.0 || ts.is_nan() {
        return String::new();
    }
    let diff = (now - ts).max(0.0);
    if diff < MINUTE {
        return "just now".into();
    }
    if diff < HOUR {
        return format!("{}m ago", (diff / MINUTE).floor());
    }
    if diff < DAY {
        return format!("{}h ago", (diff / HOUR).floor());
    }
    if diff < 7.0 * DAY {
        return format!("{}d ago", (diff / DAY).floor());
    }
    let (y, m, d) = date(ts);
    let month = MONTHS[(m.clamp(1, 12) - 1) as usize];
    if now - ts > 300.0 * DAY {
        format!("{month} {d}, {y}")
    } else {
        format!("{month} {d}")
    }
}

/// A time's date in the local time zone: (year, month 1–12, day).
pub fn local_date(ms: f64) -> (i64, u32, u32) {
    let secs = (ms / 1000.0).floor() as i64;
    #[cfg(unix)]
    {
        // SAFETY: localtime_r writes only into `tm`, which outlives the call.
        unsafe {
            let t = secs as libc::time_t;
            let mut tm: libc::tm = std::mem::zeroed();
            if !libc::localtime_r(&t, &mut tm).is_null() {
                return (
                    i64::from(tm.tm_year) + 1900,
                    u32::try_from(tm.tm_mon + 1).unwrap_or(1),
                    u32::try_from(tm.tm_mday).unwrap_or(1),
                );
            }
        }
    }
    civil_from_days(secs.div_euclid(86_400))
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_ago_a_commit_was() {
        // GIT-031.
        let now = 1_792_508_400_000.0; // 2026-10-20 15:00 UTC
        let utc = |ms: f64| civil_from_days((ms / 1000.0).floor() as i64 / 86_400);
        let ago = |ms: f64| time_ago_with(now - ms, now, utc);
        assert_eq!(ago(59_000.0), "just now");
        assert_eq!(ago(15.0 * MINUTE), "15m ago");
        assert_eq!(ago(3.0 * HOUR + 5.0), "3h ago");
        assert_eq!(ago(2.0 * DAY), "2d ago");
        assert_eq!(ago(10.0 * DAY), "Oct 10");
        assert_eq!(ago(400.0 * DAY), "Sep 15, 2025");
        assert_eq!(time_ago_with(0.0, now, utc), "");
        // A time from the future reads as just now.
        assert_eq!(ago(-5.0 * MINUTE), "just now");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
