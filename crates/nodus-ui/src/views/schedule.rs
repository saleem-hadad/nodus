//! A view's schedules on its caption (spec 19, SCHED-065…SCHED-070): a small
//! clock whose ring fills toward the soonest next run and when that is, and
//! the picker it opens to change how often each runs without asking Claude.
//! The picker is a menu in the window's menu layer, so it draws over the
//! page, which shows its snapshot meanwhile (decision D3).

use std::rc::Rc;
use std::time::Duration;

use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ElementId, Entity, Focusable as _, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, point, px,
};
use nodus_actions::schedule::{parse_interval, span_label};
use nodus_domain::{BoardView, ViewSchedule};

use super::card::{CardUi, tip_over_page};
use super::parts::{nodus, on_core};
use crate::board::nodes::{BoardActions, BoardCx};
use crate::board::units::fu;
use crate::components::clock_ring::clock_ring;
use crate::components::context_menu::{Menu, MenuItem};
use crate::components::text_field::{focus_all, new_input};
use crate::components::tooltip::TooltipPress;
use crate::theme::tokens::{TEXT_SM, TEXT_XS, Tokens};
use crate::theme::{self, fonts};

/// Read again at least this often, as runs move the times on (ms).
const READ_EVERY: f64 = 60_000.0;
/// While a run is due, read again this often until it has moved on (ms).
const READ_DUE: f64 = 5_000.0;
/// A command's chips (SCHED-067): minutes, or off.
const COMMAND_CHIPS: [(&str, Option<f64>); 8] = [
    ("1m", Some(1.0)),
    ("5m", Some(5.0)),
    ("15m", Some(15.0)),
    ("30m", Some(30.0)),
    ("1h", Some(60.0)),
    ("6h", Some(360.0)),
    ("1d", Some(1440.0)),
    ("Off", None),
];
/// A refresh's: it asks Claude at most every ten minutes (VIEW-011).
const REFRESH_CHIPS: [(&str, Option<f64>); 7] = [
    ("10m", Some(10.0)),
    ("15m", Some(15.0)),
    ("30m", Some(30.0)),
    ("1h", Some(60.0)),
    ("6h", Some(360.0)),
    ("1d", Some(1440.0)),
    ("Off", None),
];

/// What a view's caption knows of its schedules (a part of its [`CardUi`]).
#[derive(Default)]
pub struct Schedules {
    /// As the core last gave them.
    list: Vec<ViewSchedule>,
    /// The view's commands and refresh they were read for: they're read
    /// again when those change.
    read_for: Option<(
        Option<Vec<nodus_domain::ViewCommand>>,
        Option<nodus_domain::ViewRefresh>,
    )>,
    /// When they were last read (ms), and whether a read is under way.
    read_at: f64,
    reading: bool,
    /// What they say changed (a command ran, the user picked): read again.
    stale: bool,
    /// Bumped by each change from the picker, so a read that started before
    /// one doesn't undo it.
    changes: u64,
    /// Draws the caption again as time passes.
    tick: Option<Task<()>>,
    /// The clock's place in the window, for the picker to hang under.
    anchor: Option<Bounds<Pixels>>,
    /// The open picker's menu, by its serial.
    open: Option<u64>,
    /// The picker's words field, while it's open.
    field: Option<(Entity<InputState>, Subscription)>,
    /// The words typed couldn't be read.
    unreadable: bool,
}

impl Schedules {
    /// SCHED-070: they're read again when the view's commands or refresh
    /// change, when something changed what they say, every 5 s while a run
    /// is due, and at least every minute.
    fn wants_read(&self, view: &BoardView, now: f64) -> bool {
        if self.reading {
            return false;
        }
        let same = self
            .read_for
            .as_ref()
            .is_some_and(|(commands, refresh)| *commands == view.commands && *refresh == view.refresh);
        if !same || self.stale {
            return true;
        }
        let since = now - self.read_at;
        since >= READ_EVERY || (since >= READ_DUE && soonest(&self.list).is_some_and(|s| s.next_at <= now))
    }
}

/// Whether a view has schedules to show: a command that runs every so
/// often, or a refresh (SCHED-065).
pub fn scheduled(view: &BoardView) -> bool {
    view.refresh.is_some()
        || view
            .commands
            .iter()
            .flatten()
            .any(|c| c.every.is_some_and(|e| e != 0.0 && !e.is_nan()))
}

/// The schedule that runs next, of those that run by themselves.
pub fn soonest(list: &[ViewSchedule]) -> Option<&ViewSchedule> {
    list.iter()
        .filter(|s| s.runs())
        .min_by(|a, b| a.next_at.total_cmp(&b.next_at))
}

/// When a run is, from `now`: `in 12m`, `in 40s`, or `due` once its time
/// has come and the 30 s ticker hasn't started it yet (SCHED-065).
pub fn when(next_at: f64, now: f64) -> String {
    if next_at <= now {
        "due".to_owned()
    } else {
        format!("in {}", span_label(next_at - now))
    }
}

/// How full the ring is: how far `s` is from its last run to its next (SCHED-065).
pub fn progress(s: &ViewSchedule, now: f64) -> f32 {
    let span = s.every * 60_000.0;
    if span <= 0.0 {
        return 0.0;
    }
    ((now - s.last_at()) / span).clamp(0.0, 1.0) as f32
}

/// How soon the caption is drawn again (SCHED-070): every second while the
/// ring moves visibly or seconds show, else every 5 s; never with nothing
/// running.
fn redraw_after(list: &[ViewSchedule], now: f64) -> Option<Duration> {
    let next = soonest(list)?;
    let quick = next.every <= 5.0 || next.next_at - now < 90_000.0;
    Some(Duration::from_secs(if quick { 1 } else { 5 }))
}

/// What the words field says (SCHED-068): minutes between runs, or
/// `Some(None)` for off; None for words that say neither.
pub fn read_words(words: &str) -> Option<Option<f64>> {
    let words = words.trim().to_lowercase();
    if matches!(words.as_str(), "off" | "never" | "pause" | "stop") {
        return Some(None);
    }
    parse_interval(&words).map(Some)
}

/// Whether a chip is the schedule's interval as it is (SCHED-067).
fn picked(s: &ViewSchedule, chip: Option<f64>) -> bool {
    match chip {
        None => s.paused,
        Some(every) => !s.paused && s.every == every,
    }
}

/// What a row says of its schedule: `every 15 minutes`, `Off`, or that the
/// user hasn't allowed its command (SCHED-067).
fn state(s: &ViewSchedule) -> String {
    if s.paused {
        "Off".to_owned()
    } else if !s.allowed {
        "Not allowed to run".to_owned()
    } else {
        super::every_label(s.every)
    }
}

/// The picker's last line (SCHED-069).
fn next_line(list: &[ViewSchedule], now: f64) -> String {
    match soonest(list) {
        Some(s) if s.next_at <= now => "Due now".to_owned(),
        Some(s) => format!("Next {}", when(s.next_at, now)),
        None => "Nothing runs on its own".to_owned(),
    }
}

/// A command of the view ran: its schedule moved on, so they're read again
/// (SCHED-062, SCHED-070).
pub fn ran(view_id: &str, cx: &mut App) {
    if let Some(ui) = super::card::find(view_id, cx) {
        ui.update(cx, |u, cx| {
            u.schedules.stale = true;
            cx.notify();
        });
    }
}

/// SCHED-065: after the view's title, a 14-unit clock whose ring fills
/// toward the soonest next run, and when that is (`in 12m`), or a clock
/// without a ring and `Off` when nothing runs by itself. A click opens the
/// picker under it. None for a view with no schedules, or until they're read.
pub fn clock(
    view: &BoardView,
    board: &BoardCx,
    ui: &Entity<CardUi>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    if board.project_id.is_empty() || !scheduled(view) {
        return None;
    }
    let now = crate::parity::now_ms(cx);
    if ui.read(cx).schedules.wants_read(view, now) {
        read(ui, &board.project_id, view, now, cx);
    }
    let list = ui.read(cx).schedules.list.clone();
    if list.is_empty() {
        return None;
    }
    if !crate::parity::frozen(cx)
        && ui.read(cx).schedules.tick.is_none()
        && let Some(after) = redraw_after(&list, now)
    {
        ui.update(cx, |u, cx| {
            u.schedules.tick = Some(cx.spawn(async move |ui, cx| {
                cx.background_executor().timer(after).await;
                let _ = ui.update(cx, |u, cx| {
                    u.schedules.tick = None;
                    cx.notify();
                });
            }));
        });
    }
    let t = theme::tokens(cx);
    let (ring, label) = match soonest(&list) {
        Some(s) => (clock_ring(fu(14.), progress(s, now)), when(s.next_at, now)),
        None => (clock_ring(fu(14.), 0.0).without_ring(), "Off".to_owned()),
    };
    let id = view.id.clone();
    let open = is_open(ui, cx);
    let press = window.use_keyed_state(ElementId::Name(format!("view-clock-tip-{id}").into()), cx, |_, _| {
        TooltipPress::default()
    });
    let (click_ui, project, view_id, actions) = (
        ui.clone(),
        board.project_id.to_string(),
        id.clone(),
        board.actions.clone(),
    );
    let leave = id.clone();
    Some(
        div()
            .id(ElementId::Name(format!("view-clock-{id}").into()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(fu(4.))
            .h(fu(20.))
            .px(fu(4.))
            .rounded(fu(5.))
            .when(open, |d| d.bg(t.bg_hover))
            .hover(|d| d.bg(t.bg_hover))
            .text_size(fu(11.))
            .line_height(fu(14.))
            .font_features(fonts::tabular())
            .text_color(t.text_tertiary)
            .whitespace_nowrap()
            // The caption is the view's drag handle: the clock is a button.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |e, window, cx| {
                actions.click_select(&view_id, &e.modifiers(), cx);
                toggle(&click_ui, &project, &view_id, &actions, window, cx);
            })
            .on_hover(move |h, _, cx| {
                if !*h {
                    super::swap::set_tooltip(&leave, false, cx);
                }
            })
            .tooltip(tip_over_page(&id, "Change how often it updates".into(), press))
            .child(ring)
            .child(label)
            .child(crate::model_menu::probe(ui.downgrade(), |u: &mut CardUi, b| {
                u.schedules.anchor = Some(b);
            }))
            .into_any_element(),
    )
}

/// Reads the view's schedules from the core (SCHED-061).
fn read(ui: &Entity<CardUi>, project_id: &str, view: &BoardView, now: f64, cx: &mut App) {
    let changes = ui.update(cx, |u, _| {
        let s = &mut u.schedules;
        s.reading = true;
        s.stale = false;
        s.read_at = now;
        s.read_for = Some((view.commands.clone(), view.refresh.clone()));
        s.changes
    });
    let nodus = nodus(cx);
    let (p, v) = (project_id.to_owned(), view.id.clone());
    let ui = ui.clone();
    on_core(
        cx,
        async move { nodus.views().schedules(&p, &v).await },
        move |result, cx| {
            ui.update(cx, |u, cx| {
                let s = &mut u.schedules;
                s.reading = false;
                // A change picked meanwhile is newer; the read after it says.
                if let Ok(list) = result
                    && s.changes == changes
                {
                    s.list = list;
                }
                cx.notify();
            });
        },
    );
}

/// Whether this view's picker is the menu that's open.
fn is_open(ui: &Entity<CardUi>, cx: &App) -> bool {
    let open = ui.read(cx).schedules.open;
    open.is_some()
        && crate::shell::window::main_shell(cx)
            .and_then(|shell| shell.read(cx).menus.read(cx).menu.as_ref().map(|m| m.serial))
            == open
}

/// The clock's click: opens the picker under it, or closes it (SCHED-066).
fn toggle(
    ui: &Entity<CardUi>,
    project_id: &str,
    view_id: &str,
    actions: &BoardActions,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(menus) = crate::shell::window::main_shell(cx).map(|shell| shell.read(cx).menus.clone()) else {
        return;
    };
    if is_open(ui, cx) {
        menus.update(cx, |m, cx| m.close(window, cx));
        return;
    }
    let Some(anchor) = ui.read(cx).schedules.anchor else {
        return;
    };
    let (row_ui, p, v) = (ui.clone(), project_id.to_owned(), view_id.to_owned());
    let (close_ui, actions) = (ui.clone(), actions.clone());
    let menu = Menu {
        at: point(anchor.left(), anchor.bottom() + px(4.)),
        align_right: false,
        items: vec![MenuItem::Custom(Rc::new(move |window, cx| {
            picker(&row_ui, &p, &v, window, cx)
        }))],
        // A press on the clock is the clock's: it closes the picker.
        anchor: Some(anchor),
        on_close: Some(Rc::new(move |window, cx| closed(&close_ui, &actions, window, cx))),
        serial: 0,
    };
    menus.update(cx, |m, cx| m.open(menu, cx));
    let serial = menus.read(cx).menu.as_ref().map(|m| m.serial);
    ui.update(cx, |u, cx| {
        u.schedules.open = serial;
        u.schedules.unreadable = false;
        cx.notify();
    });
}

/// The picker closed, however it closed: the board has the keyboard back
/// if its field had it.
fn closed(ui: &Entity<CardUi>, actions: &BoardActions, window: &mut Window, cx: &mut App) {
    let field = ui.update(cx, |u, cx| {
        u.schedules.open = None;
        u.schedules.unreadable = false;
        cx.notify();
        u.schedules.field.take()
    });
    if let Some((field, _)) = field
        && field.read(cx).focus_handle(cx).is_focused(window)
    {
        actions.focus_board(window, cx);
    }
}

/// SCHED-066…SCHED-069: the picker: a words field for a view with one
/// schedule, a row of chips for each schedule, and when the next run is.
fn picker(ui: &Entity<CardUi>, project_id: &str, view_id: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme::tokens(cx);
    let now = crate::parity::now_ms(cx);
    let (list, unreadable) = {
        let s = &ui.read(cx).schedules;
        (s.list.clone(), s.unreadable)
    };
    let field = match list.as_slice() {
        [only] => Some(words_field(ui, project_id, view_id, &only.key, &t, window, cx)),
        _ => None,
    };
    let rows: Vec<AnyElement> = list.iter().map(|s| row(ui, project_id, view_id, s, &t)).collect();
    let last = if unreadable {
        "Try “every 2 hours”, “15m” or “daily”".to_owned()
    } else {
        next_line(&list, now)
    };
    div()
        .w(px(268.))
        .p(px(6.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .children(field)
        .children(rows)
        .child(
            div()
                .pt(px(8.))
                .border_t_1()
                .border_color(t.separator)
                .text_size(TEXT_XS)
                .font_features(fonts::tabular())
                .text_color(if unreadable { t.text_secondary } else { t.text_tertiary })
                .child(last),
        )
        .into_any_element()
}

/// SCHED-068: how often, in words, for the view's one schedule; Enter sets it.
#[allow(clippy::too_many_arguments)]
fn words_field(
    ui: &Entity<CardUi>,
    project_id: &str,
    view_id: &str,
    key: &str,
    t: &Tokens,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let frame = div()
        .flex()
        .items_center()
        .h(px(28.))
        .px(px(8.))
        .rounded(px(6.))
        .border_1()
        .bg(t.bg_window)
        .text_size(TEXT_SM)
        .text_color(t.text_primary);
    let (existing, open) = {
        let s = &ui.read(cx).schedules;
        (s.field.as_ref().map(|(field, _)| field.clone()), s.open.is_some())
    };
    let field = match existing {
        Some(field) => field,
        // The picker fading out after it closed (MOTION-011) gets no new field.
        None if !open => return frame.border_color(t.border_strong).into_any_element(),
        None => {
            let field = new_input("", "Type how often: every 2 hours", false, window, cx);
            let (ui2, p, v, k) = (ui.clone(), project_id.to_owned(), view_id.to_owned(), key.to_owned());
            let sub = window.subscribe(&field, cx, move |field, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { .. } => {
                    let words = field.read(cx).value().to_string();
                    match read_words(&words) {
                        Some(every) => {
                            field.update(cx, |f, cx| f.set_value("", window, cx));
                            set_every(&ui2, &p, &v, &k, every, cx);
                        }
                        None => ui2.update(cx, |u, cx| {
                            u.schedules.unreadable = true;
                            cx.notify();
                        }),
                    }
                }
                InputEvent::Change => ui2.update(cx, |u, cx| {
                    if u.schedules.unreadable {
                        u.schedules.unreadable = false;
                        cx.notify();
                    }
                }),
                _ => {}
            });
            focus_all(&field, window, cx);
            ui.update(cx, |u, _| u.schedules.field = Some((field.clone(), sub)));
            field
        }
    };
    let focused = field.read(cx).focus_handle(cx).is_focused(window);
    frame
        .border_color(if focused { t.accent } else { t.border_strong })
        .child(
            Input::new(&field)
                .appearance(false)
                .px(px(0.))
                .py(px(0.))
                .text_size(TEXT_SM)
                .line_height(px(18.))
                .h(px(18.)),
        )
        .into_any_element()
}

/// SCHED-067: a schedule's name and how often it runs now, over its chips.
fn row(ui: &Entity<CardUi>, project_id: &str, view_id: &str, s: &ViewSchedule, t: &Tokens) -> AnyElement {
    let chips: &[(&str, Option<f64>)] = if s.key == ViewSchedule::REFRESH {
        &REFRESH_CHIPS
    } else {
        &COMMAND_CHIPS
    };
    let chips = chips.iter().enumerate().map(|(i, &(name, every))| {
        let on = picked(s, every);
        let (ui, p, v, k) = (ui.clone(), project_id.to_owned(), view_id.to_owned(), s.key.clone());
        div()
            .id(ElementId::Name(format!("view-every-{view_id}-{}-{i}", s.key).into()))
            .flex_1()
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .border_1()
            .text_size(TEXT_XS)
            .font_weight(FontWeight::MEDIUM)
            .map(|d| {
                if on {
                    d.border_color(t.accent).text_color(t.accent)
                } else {
                    d.border_color(t.border_strong)
                        .text_color(t.text_secondary)
                        .hover(|d| d.bg(t.bg_hover).text_color(t.text_primary))
                }
            })
            .on_click(move |_, _, cx| {
                if !on {
                    set_every(&ui, &p, &v, &k, every, cx);
                }
            })
            .child(name)
    });
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text_primary)
                        .child(s.label.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(TEXT_XS)
                        .text_color(t.text_tertiary)
                        .child(state(s)),
                ),
        )
        .child(div().flex().gap(px(4.)).children(chips))
        .into_any_element()
}

/// Changes how often one schedule runs (SCHED-063): at once in the caption
/// and the picker, then in the core, whose answer is read back; a failure
/// shows on the view's error line.
fn set_every(ui: &Entity<CardUi>, project_id: &str, view_id: &str, key: &str, every: Option<f64>, cx: &mut App) {
    ui.update(cx, |u, cx| {
        let s = &mut u.schedules;
        s.changes += 1;
        s.unreadable = false;
        if let Some(found) = s.list.iter_mut().find(|x| x.key == key) {
            let last = found.last_at();
            found.paused = every.is_none();
            found.every = every.unwrap_or(found.every);
            found.next_at = last + found.every * 60_000.0;
        }
        cx.notify();
    });
    let nodus = nodus(cx);
    let (p, v, k) = (project_id.to_owned(), view_id.to_owned(), key.to_owned());
    let ui = ui.clone();
    on_core(
        cx,
        async move {
            nodus
                .views()
                .set_every(&p, &v, &k, every)
                .await
                .map_err(|e| super::reason(&e.ipc_message()))
        },
        move |result, cx| {
            if let Err(reason) = result {
                CardUi::set_error(&ui, reason, cx);
            }
            ui.update(cx, |u, cx| {
                u.schedules.stale = true;
                cx.notify();
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000_000.0;
    const MIN: f64 = 60_000.0;

    fn schedule(key: &str, every: f64, next_at: f64) -> ViewSchedule {
        ViewSchedule {
            key: key.into(),
            label: key.into(),
            every,
            paused: false,
            allowed: true,
            next_at,
        }
    }

    #[test]
    fn sched_065_says_when_the_soonest_running_schedule_runs() {
        let mut list = vec![
            schedule("command:a", 30.0, NOW + 20.0 * MIN),
            schedule("refresh", 60.0, NOW + 12.0 * MIN),
            schedule("command:b", 1.0, NOW + 30_000.0),
        ];
        // Turned off, or not allowed: not counted.
        list[2].paused = true;
        assert_eq!(soonest(&list).unwrap().key, "refresh");
        assert_eq!(when(list[1].next_at, NOW), "in 12m");
        assert_eq!(when(NOW + 40_000.0, NOW), "in 40s");
        assert_eq!(when(NOW + 2.0 * 3_600_000.0 + 14.0 * MIN, NOW), "in 2h 14m");
        assert_eq!(when(NOW - 1.0, NOW), "due");
        list[1].allowed = false;
        assert_eq!(soonest(&list).unwrap().key, "command:a");
        list[0].paused = true;
        assert!(soonest(&list).is_none());
        assert_eq!(next_line(&list, NOW), "Nothing runs on its own");
    }

    #[test]
    fn sched_065_fills_the_ring_from_the_last_run_to_the_next() {
        let s = schedule("refresh", 60.0, NOW + 15.0 * MIN);
        assert_eq!(progress(&s, NOW), 0.75);
        assert_eq!(progress(&s, NOW + 20.0 * MIN), 1.0);
        assert_eq!(progress(&s, NOW - 50.0 * MIN), 0.0);
        assert_eq!(
            redraw_after(std::slice::from_ref(&s), NOW),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            redraw_after(&[schedule("command:a", 1.0, NOW + 30_000.0)], NOW),
            Some(Duration::from_secs(1))
        );
        assert_eq!(next_line(&[s], NOW), "Next in 15m");
    }

    #[test]
    fn sched_067_marks_the_chip_of_the_interval_it_has() {
        let mut s = schedule("command:a", 15.0, NOW);
        assert!(picked(&s, Some(15.0)) && !picked(&s, Some(5.0)) && !picked(&s, None));
        assert_eq!(state(&s), "every 15 minutes");
        s.paused = true;
        assert!(picked(&s, None) && !picked(&s, Some(15.0)));
        assert_eq!(state(&s), "Off");
        s.paused = false;
        s.allowed = false;
        assert_eq!(state(&s), "Not allowed to run");
    }

    #[test]
    fn sched_068_reads_words_as_minutes_or_off() {
        assert_eq!(read_words("every 15 minutes"), Some(Some(15.0)));
        assert_eq!(read_words(" Hourly "), Some(Some(60.0)));
        assert_eq!(read_words("2h"), Some(Some(120.0)));
        assert_eq!(read_words("Off"), Some(None));
        assert_eq!(read_words("never"), Some(None));
        assert_eq!(read_words("whenever"), None);
    }
}
