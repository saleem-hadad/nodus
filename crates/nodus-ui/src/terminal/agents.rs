//! The agents viewer (`AgentViewer.tsx`; AGV-001…AGV-020): a session's
//! subagents, nested and newest first, beside the selected one's live
//! transcript and what it is doing right now.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
    relative,
};
use nodus_domain::{AgentInfo, SessionStatus};

use super::actions::{self, card};
use super::messages::{MessageProps, empty_note, message, skeleton};
use super::preview::Load;
use super::text::{POLL_MS, agent_author, agent_elapsed, default_agent, format_time, live_line, sticks};
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::components::scroll::scroll_area;
use crate::components::tooltip::native_tooltip;
use crate::components::{button, status_dot};
use crate::format::{agent_status_label, card_title, nest_agents};
use crate::parity;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// The selected agent's transcript.
struct Transcript {
    agent_id: String,
    /// What it was read for: a change of either reads it again (AGV-013).
    seen: (f64, SessionStatus),
    load: Load,
    expanded: HashSet<usize>,
    all_calls: HashSet<usize>,
    scroll: ScrollHandle,
    /// AGV-014: follow new steps unless scrolled up.
    stick: bool,
    /// The scroll position seen last frame.
    last_top: Option<f32>,
    _fetch: Option<Task<()>>,
}

pub struct AgentViewer {
    state: Entity<AppState>,
    pub session_id: String,
    project_id: String,
    agents: Option<Vec<AgentInfo>>,
    list_error: Option<String>,
    /// The card's agents as last read (id, status, updatedAt): a change reads the list again.
    card_agents: String,
    transcript: Option<Transcript>,
    focus: FocusHandle,
    _list: Option<Task<()>>,
    _poll: Option<Task<()>>,
    _tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// The card's agents in the shape the effect depended on.
fn card_agents_key(state: &AppState, session_id: &str) -> String {
    card(state, session_id)
        .and_then(|c| c.agents)
        .map(|agents| {
            agents
                .iter()
                .map(|a| format!("{}|{:?}|{}", a.agent_id, a.status, a.updated_at))
                .collect::<Vec<_>>()
                .join(";")
        })
        .unwrap_or_default()
}

impl AgentViewer {
    pub fn new(
        state: Entity<AppState>,
        session_id: String,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // AGV-002: nothing keeps the keyboard (Escape would interrupt Claude).
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let subscriptions = vec![cx.observe(&state, |this, _, cx| {
            let key = card_agents_key(this.state.read(cx), &this.session_id);
            if key != this.card_agents {
                this.card_agents = key;
                this.fetch_list(cx);
            }
            cx.notify();
        })];
        let card_agents = card_agents_key(state.read(cx), &session_id);
        let mut this = Self {
            state,
            session_id,
            project_id,
            agents: None,
            list_error: None,
            card_agents,
            transcript: None,
            focus,
            _list: None,
            _poll: None,
            _tick: None,
            _subscriptions: subscriptions,
        };
        this.fetch_list(cx);
        this
    }

    /// AGV-004: the list, read on open, on card changes and while any is at work.
    fn fetch_list(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        let (project, session) = (self.project_id.clone(), self.session_id.clone());
        self._list = Some(cx.spawn(async move |this, cx| {
            let result = nodus.sessions().agents(&project, &session).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        this.agents = Some(list);
                        this.list_error = None;
                    }
                    // AGV-005: the error over any list already shown.
                    Err(e) => this.list_error = Some(e.ipc_message()),
                }
                this.after_list(cx);
                cx.notify();
            });
        }));
    }

    fn after_list(&mut self, cx: &mut Context<'_, Self>) {
        let running = self
            .agents
            .as_ref()
            .is_some_and(|a| a.iter().any(|a| a.status != SessionStatus::Done));
        if running && !parity::frozen(cx) {
            if self._poll.is_none() {
                self._poll = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
                        if this.update(cx, Self::fetch_list).is_err() {
                            break;
                        }
                    }
                }));
            }
        } else {
            self._poll = None;
        }
        self.sync_transcript(cx);
    }

    /// AGV-009: the requested agent if listed, else the default one.
    fn selected(&self, cx: &gpui::App) -> Option<AgentInfo> {
        let agents = self.agents.as_ref()?;
        let requested = self.state.read(cx).agent_view.as_ref().and_then(|(_, a)| a.clone());
        requested
            .and_then(|id| agents.iter().find(|a| a.agent_id == id))
            .or_else(|| default_agent(agents))
            .cloned()
    }

    /// AGV-013: read the selected agent's transcript when it's picked and
    /// again when its updatedAt or status changes.
    fn sync_transcript(&mut self, cx: &mut Context<'_, Self>) {
        let Some(agent) = self.selected(cx) else {
            self.transcript = None;
            return;
        };
        let seen = (agent.updated_at, agent.status);
        let same_agent = self.transcript.as_ref().is_some_and(|t| t.agent_id == agent.agent_id);
        if same_agent && self.transcript.as_ref().is_some_and(|t| t.seen == seen) {
            return;
        }
        if !same_agent {
            self.transcript = Some(Transcript {
                agent_id: agent.agent_id.clone(),
                seen,
                load: Load::Loading,
                expanded: HashSet::new(),
                all_calls: HashSet::new(),
                scroll: ScrollHandle::new(),
                stick: true,
                last_top: None,
                _fetch: None,
            });
        }
        let nodus = self.state.read(cx).nodus.clone();
        let (project, session, agent_id) = (self.project_id.clone(), self.session_id.clone(), agent.agent_id.clone());
        let task = cx.spawn(async move |this, cx| {
            let result = nodus.sessions().agent_transcript(&project, &session, &agent_id).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(t) = this.transcript.as_mut().filter(|t| t.agent_id == agent_id) {
                    // Later reads replace the content without flashing.
                    t.load = match result {
                        Ok(messages) => Load::Ready(messages),
                        Err(e) => Load::Error(e.ipc_message()),
                    };
                    if t.stick {
                        t.scroll.scroll_to_bottom();
                    }
                }
                cx.notify();
            });
        });
        if let Some(t) = self.transcript.as_mut() {
            t.seen = seen;
            t._fetch = Some(task);
        }
    }

    /// AGV-011: the elapsed time counts up each second while at work.
    fn tick_while_working(&mut self, selected: Option<&AgentInfo>, cx: &mut Context<'_, Self>) {
        let working = selected.is_some_and(|a| a.status != SessionStatus::Done);
        if !working || parity::frozen(cx) {
            self._tick = None;
            return;
        }
        if self._tick.is_none() {
            self._tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
    }

    fn header(&self, selected: Option<&AgentInfo>, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let card = card(self.state.read(cx), &self.session_id);
        let now = parity::now_ms(cx);
        let state = self.state.clone();
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .flex_none()
            .pt(px(16.))
            .pr(px(12.))
            .pb(px(14.))
            .pl(px(22.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(fonts::DISPLAY)
                            .text_size(TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(selected.map_or_else(|| "Agents".to_owned(), |a| a.description.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(px(14.))
                            .gap_y(px(4.))
                            .mt(px(3.))
                            .text_size(TEXT_SM)
                            .text_color(t.text_secondary)
                            .when_some(selected, |d, a| {
                                let color = match a.status {
                                    SessionStatus::Working => t.status_working,
                                    SessionStatus::Waiting => t.status_waiting,
                                    SessionStatus::Done => t.text_secondary,
                                };
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .text_color(color)
                                        .child(status_dot("av-status-dot", a.status).small())
                                        .child(agent_status_label(a)),
                                )
                                .child(a.agent_type.clone())
                                .child(format!(
                                    "{} · {}",
                                    format_time(a.started_at, now),
                                    agent_elapsed(a, now)
                                ))
                                .when(a.tool_count > 0, |d| {
                                    d.child(format!(
                                        "{} tool {}",
                                        a.tool_count,
                                        if a.tool_count == 1 { "call" } else { "calls" }
                                    ))
                                })
                            })
                            .child(
                                div()
                                    .id("av-session")
                                    .max_w(px(300.))
                                    .truncate()
                                    .text_color(t.text_tertiary)
                                    .tooltip(native_tooltip("The session that started these agents".into()))
                                    .child(card_title(card.as_ref())),
                            ),
                    ),
            )
            .child(
                div().flex().items_center().gap(px(6.)).flex_none().child(
                    button("av-close")
                        .ghost()
                        .square()
                        .icon(Icon::X, 16., 2.)
                        .tooltip("Close (Esc)")
                        .on_click(move |_, _, cx| actions::close_agents(&state, cx)),
                ),
            )
            .into_any_element()
    }

    fn list(&self, selected: Option<&AgentInfo>, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let now = parity::now_ms(cx);
        let note = |text: String| {
            div()
                .p(px(12.))
                .text_size(TEXT_SM)
                .text_color(t.text_tertiary)
                .child(text)
        };
        let nested = self.agents.as_ref().map(|a| nest_agents(a, true)).unwrap_or_default();
        div()
            .id("av-list")
            .flex()
            .flex_col()
            .gap(px(2.))
            .flex_none()
            .w(px(272.))
            .p(px(8.))
            .overflow_y_scroll()
            .border_r(px(0.5))
            .border_color(t.separator)
            .bg(t.bg_sunken)
            .when(self.agents.is_none() && self.list_error.is_none(), |d| {
                d.child(note("Loading…".into()))
            })
            .when_some(self.list_error.clone(), |d, e| d.child(note(e)))
            .when(self.agents.as_ref().is_some_and(Vec::is_empty), |d| {
                d.child(note("No agents yet.".into()))
            })
            .children(nested.into_iter().map(|(agent, depth)| {
                let current = selected.is_some_and(|s| s.agent_id == agent.agent_id);
                let (state, session, id) = (self.state.clone(), self.session_id.clone(), agent.agent_id.clone());
                div()
                    .id(SharedString::from(format!("av-item-{}", agent.agent_id)))
                    .flex()
                    .items_start()
                    .gap(px(9.))
                    .flex_none()
                    .w_full()
                    .py(px(7.))
                    .pr(px(12.))
                    .pl(px(12. + depth.min(3) as f32 * 14.))
                    .rounded(RADIUS_SM)
                    .when(current, |d| d.bg(t.bg_selected))
                    .when(!current, |d| d.hover(|s| s.bg(t.bg_hover)))
                    // AGV-008: picks that agent.
                    .on_click(move |_, _, cx| actions::open_agents(&state, &session, Some(&id), cx))
                    .child(div().mt(px(5.)).child(
                        status_dot(SharedString::from(format!("av-dot-{}", agent.agent_id)), agent.status).small(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(TEXT_MD)
                                    .text_color(t.text_primary)
                                    .line_clamp(2)
                                    .child(agent.description.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(TEXT_XS)
                                    .text_color(t.text_tertiary)
                                    .child(format!("{} · {}", agent.agent_type, format_time(agent.started_at, now))),
                            ),
                    )
            }))
            .into_any_element()
    }

    fn detail(&mut self, selected: Option<&AgentInfo>, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let Some(agent) = selected else {
            // AGV-012
            let text = if self.agents.is_some() {
                "This session hasn’t started any agents."
            } else {
                ""
            };
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(empty_note(text, None, None, cx))
                .into_any_element();
        };
        let this = cx.entity();
        let at_work = agent.status != SessionStatus::Done;
        let Some(transcript) = self.transcript.as_ref() else {
            return div().flex_1().into_any_element();
        };
        let content: AnyElement = match &transcript.load {
            Load::Loading => skeleton(cx),
            // AGV-016: no retry; the next change reads it again.
            Load::Error(e) => empty_note("Couldn’t read this agent’s transcript.", Some(e), None, cx),
            // AGV-017
            Load::Ready(messages) if messages.is_empty() => empty_note(
                if at_work {
                    "Starting…"
                } else {
                    "This agent left no transcript."
                },
                None,
                None,
                cx,
            ),
            Load::Ready(messages) => div()
                .flex()
                .flex_col()
                .gap(px(18.))
                .pt(px(12.))
                .children(messages.iter().enumerate().map(|(i, m)| {
                    let (toggle, all) = (this.clone(), this.clone());
                    message(
                        &MessageProps {
                            id: ("av-msg", i).into(),
                            message: m,
                            author: Some(agent_author(m, i, &agent.agent_type)),
                            highlight: &[],
                            found: false,
                            found_call: None,
                            expanded: transcript.expanded.contains(&i),
                            all_calls: transcript.all_calls.contains(&i),
                            on_toggle: Rc::new(move |_, cx| {
                                toggle.update(cx, |v, cx| {
                                    if let Some(t) = v.transcript.as_mut()
                                        && !t.expanded.remove(&i)
                                    {
                                        t.expanded.insert(i);
                                    }
                                    cx.notify();
                                });
                            }),
                            on_all_calls: Rc::new(move |_, cx| {
                                all.update(cx, |v, cx| {
                                    if let Some(t) = v.transcript.as_mut() {
                                        t.all_calls.insert(i);
                                    }
                                    cx.notify();
                                });
                            }),
                            on_bounds: None,
                        },
                        cx,
                    )
                }))
                .into_any_element(),
        };
        // AGV-018, AGV-019: what it's doing, red while it waits.
        let live = (matches!(transcript.load, Load::Ready(_)) && at_work).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(18.))
                .px(px(12.))
                .py(px(8.))
                .rounded(RADIUS_MD)
                .bg(t.bg_sunken)
                .font_family(super::MONO)
                .text_size(TEXT_XS)
                .text_color(if agent.status == SessionStatus::Waiting {
                    t.status_waiting
                } else {
                    t.text_secondary
                })
                .child(status_dot("av-live-dot", agent.status).small())
                .child(div().min_w_0().truncate().child(live_line(agent)))
        });
        let body = div().flex().flex_col().child(content).children(live);
        let area = scroll_area("av-body", &transcript.scroll, [8., 22., 24., 22.], body, window, cx);
        // AGV-014: scrolled up more than 48 px, it stops following. Only a
        // scroll (the user's, or following) changes that, not new content.
        if let Some(t) = self.transcript.as_mut() {
            let top = -f32::from(t.scroll.offset().y);
            if t.last_top != Some(top) {
                t.last_top = Some(top);
                t.stick = sticks(f32::from(t.scroll.max_offset().y), top, 0.0);
            }
        }
        div()
            .id("av-detail")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(area)
            .into_any_element()
    }
}

impl Render for AgentViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        // min(1080px, 94vw) × 82vh.
        let width = px(1080.).min(viewport.width * 0.94);
        let height = viewport.height * 0.82;
        self.sync_transcript(cx);
        let selected = self.selected(cx);
        self.tick_while_working(selected.as_ref(), cx);
        let header = self.header(selected.as_ref(), cx);
        let list = self.list(selected.as_ref(), cx);
        let detail = self.detail(selected.as_ref(), window, cx);
        let state = self.state.clone();
        let close = self.state.clone();
        let content = div()
            .id("av-panel")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .w_full()
            .h(height)
            .line_height(relative(1.4))
            // AGV-003: Escape closes it.
            .on_key_down(move |e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    actions::close_agents(&close, cx);
                }
            })
            .child(header)
            .child(div().flex().flex_1().min_h_0().child(list).child(detail));
        Modal::new("av-modal", width, move |_, cx| actions::close_agents(&state, cx)).render(content, window, cx)
    }
}
