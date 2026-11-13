//! The session window (`TerminalModal.tsx`, `.term-*` in `terminal.css`;
//! MODAL-001…MODAL-032): a session's terminal in a modal over the board,
//! with its status, title, folder or branch, session id, usage, mic,
//! Resume/Restart, Stop, Archive/Remove and close.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window,
    div, px, relative,
};
use nodus_core::StopConfirmation;
use nodus_domain::{SessionCard, SessionKind, SessionStatus};
use nodus_terminal::host::Phase;

use super::actions::{self, card};
use super::confirm::confirm_dialog;
use super::element::{TERM_BG, rgb};
use super::host::{HostConfig, HostEvent, TerminalHost};
use super::icons::{TermIcon, term_icon};
use super::overlays::{error_overlay, loading_pill};
use crate::chat::ChatView;
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::components::tooltip::native_tooltip;
use crate::components::usage_stats::usage_stats;
use crate::components::{button, icon, status_dot};
use crate::format::{card_title, tilde_path};
use crate::platform::Platform;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// How long `Copied` shows (MODAL-017).
const COPIED_FOR: Duration = Duration::from_millis(1400);

/// MODAL-012: shell `Running`/`Exited`; Claude `Working`, `Needs you`, `Idle`, `Not running`.
pub fn status_label(card: Option<&SessionCard>) -> &'static str {
    let Some(card) = card else { return "" };
    if card.node.kind == Some(SessionKind::Terminal) {
        return if card.live { "Running" } else { "Exited" };
    }
    match card.status {
        SessionStatus::Working => "Working",
        SessionStatus::Waiting => "Needs you",
        SessionStatus::Done if card.live => "Idle",
        SessionStatus::Done => "Not running",
    }
}

/// MODAL-009, MODAL-010: the loading pill's text.
pub fn loading_label(shell: bool, started_live: bool) -> &'static str {
    if shell {
        "Starting shell…"
    } else if started_live {
        "Starting Claude…"
    } else {
        "Resuming session…"
    }
}

/// MODAL-027: the close button's tooltip.
pub fn close_tooltip(platform: Platform, shell: bool) -> String {
    let keys = if platform.is_mac() { "⌘W" } else { "Ctrl+Shift+W" };
    let what = if shell { "shell" } else { "session" };
    format!("Close ({keys}). The {what} keeps running.")
}

/// The session window's content for one session.
pub struct SessionWindow {
    state: Entity<AppState>,
    pub session_id: String,
    project_id: String,
    kind: SessionKind,
    pub host: Entity<TerminalHost>,
    /// CHAT-029: a Claude session's chat, with the terminal docked in it.
    pub chat: Option<Entity<ChatView>>,
    /// MODAL-010: the loading label is decided when the window opens.
    started_live: bool,
    copied: bool,
    copied_timer: Option<Task<()>>,
    confirm: Option<StopConfirmation>,
    confirm_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SessionWindow {
    pub fn new(
        state: Entity<AppState>,
        session_id: String,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let card = card(state.read(cx), &session_id);
        let kind = card.as_ref().and_then(|c| c.node.kind).unwrap_or(SessionKind::Claude);
        // MODAL-007: the window attaches the terminal with focus.
        let host = cx.new(|cx| {
            let mut host = TerminalHost::new(
                session_id.clone(),
                project_id.clone(),
                kind,
                HostConfig {
                    focus_on_attach: true,
                    card: false,
                },
                window,
                cx,
            );
            host.set_enabled(true, cx);
            host
        });
        let subscriptions = vec![
            cx.subscribe(&host, |_, _, _: &HostEvent, cx| cx.notify()),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        let chat = (kind == SessionKind::Claude).then(|| {
            let (state, session_id, project_id, host) =
                (state.clone(), session_id.clone(), project_id.clone(), host.clone());
            cx.new(|cx| {
                let chat = ChatView::new(state, session_id, project_id, host, false, window, cx);
                chat.focus_composer(window, cx);
                chat
            })
        });
        Self {
            state,
            session_id,
            project_id,
            kind,
            host,
            chat,
            started_live: card.is_some_and(|c| c.live),
            copied: false,
            copied_timer: None,
            confirm: None,
            confirm_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether Stop's confirm is up (MODAL-022).
    pub fn asking(&self) -> bool {
        self.confirm.is_some()
    }

    fn shell(&self) -> bool {
        self.kind == SessionKind::Terminal
    }

    /// Resume / Restart / Try again: reattach, which resumes (MODAL-020).
    fn resume(&mut self, cx: &mut Context<'_, Self>) {
        self.started_live = false;
        self.host.update(cx, TerminalHost::reattach);
        // CHAT-029: the terminal attaches where it's drawn, so it docks.
        if let Some(chat) = &self.chat {
            chat.update(cx, |c, cx| c.set_dock(true, cx));
        }
        cx.notify();
    }

    /// MODAL-019: what's said waits at the prompt; a session that isn't
    /// running resumes to take it.
    pub fn type_in(&mut self, text: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        // CHAT-030: into the chat's composer.
        if let Some(chat) = &self.chat {
            chat.update(cx, |c, cx| c.insert_text(text, window, cx));
            return;
        }
        let card = card(self.state.read(cx), &self.session_id);
        let live = card.as_ref().is_some_and(|c| c.live);
        self.host.read(cx).type_in(text, !live);
        let loading = self.host.read(cx).is_loading();
        if !live && !loading {
            self.resume(cx);
        }
        self.host.update(cx, |h, cx| h.focus_terminal(window, cx));
    }

    fn copy_id(&mut self, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.session_id.clone()));
        self.copied = true;
        self.copied_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// MODAL-022: between turns at once, else after asking.
    fn stop(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(card) = card(self.state.read(cx), &self.session_id) else {
            return;
        };
        match nodus_core::stop_confirmation(std::slice::from_ref(&card), |c| card_title(Some(c))) {
            None => actions::stop_sessions(&self.state, vec![self.session_id.clone()], cx),
            Some(ask) => {
                self.confirm = Some(ask);
                window.focus(&self.confirm_focus, cx);
                cx.notify();
            }
        }
    }

    fn resolve_stop(&mut self, ok: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.confirm = None;
        if ok {
            actions::stop_sessions(&self.state, vec![self.session_id.clone()], cx);
        }
        self.host.update(cx, |h, cx| h.focus_terminal(window, cx));
        cx.notify();
    }

    /// MODAL-026: archived, and the window closes.
    fn archive(&mut self, cx: &mut Context<'_, Self>) {
        actions::patch_cards(
            &self.state,
            vec![nodus_domain::NodePatch {
                session_id: self.session_id.clone(),
                archived: Some(true),
                ..Default::default()
            }],
            cx,
        );
        actions::close_terminal(&self.state, cx);
    }

    /// MODAL-025: the window closes and the card goes, stopping its shell.
    fn remove(&mut self, cx: &mut Context<'_, Self>) {
        actions::close_terminal(&self.state, cx);
        actions::remove_card(&self.state, &self.session_id, cx);
    }

    fn header(&mut self, card: Option<&SessionCard>, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let shell = self.shell();
        let status = card.map_or(SessionStatus::Done, |c| c.status);
        let label = status_label(card);
        let live = card.is_some_and(|c| c.live);
        let phase = self.host.read(cx).phase.phase.clone();
        let can_resume = !live && phase != Phase::Loading;
        let can_stop = card.is_some_and(|c| !nodus_core::stoppable(std::slice::from_ref(c)).is_empty());
        let repo_path = self.state.read(cx).board.as_ref().map(|b| b.project.repo_path.clone());
        let branch = card.and_then(|c| c.meta.as_ref()).and_then(|m| m.git_branch.clone());
        let view_of = card.and_then(|c| c.node.view_of.clone()).is_some_and(|v| !v.is_empty());
        let label_color = match status {
            SessionStatus::Working if !shell => t.status_working,
            SessionStatus::Waiting if !shell => t.status_waiting,
            _ => t.text_secondary,
        };
        let this = cx.entity();

        let status_el = div()
            .id("term-status")
            .flex()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .tooltip(native_tooltip(label.into()))
            .map(|d| {
                if shell {
                    d.child(term_icon(
                        TermIcon::SquareTerminal,
                        14.,
                        2.,
                        if live { t.text_secondary } else { t.text_tertiary },
                    ))
                } else {
                    d.child(status_dot("term-status-dot", status))
                }
            })
            .child(
                div()
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(label_color)
                    .child(label),
            );

        let id_hover = window.use_keyed_state("term-id-hover", cx, |_, _| false);
        let id_hovered = *id_hover.read(cx);
        let sub = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .mt(px(1.))
            .text_size(TEXT_XS)
            .line_height(relative(1.4))
            .text_color(t.text_tertiary)
            .min_w_0()
            // A server project's folder is on its server: no path here (SRV-163).
            .when_some(repo_path.filter(|p| shell && !p.is_empty()), |d, path| {
                d.child(
                    div()
                        .id("term-cwd")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .min_w_0()
                        .font_family(fonts::mono())
                        .cursor_text()
                        .tooltip(native_tooltip(path.clone().into()))
                        .child(term_icon(TermIcon::Folder, 12., 2., t.text_tertiary))
                        // `.term-cwd.selectable`.
                        .child(div().truncate().child(crate::components::selectable::selectable_text(
                            "term-cwd-text",
                            tilde_path(&path),
                        ))),
                )
            })
            .when_some(branch, |d, branch| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .max_w(px(280.))
                        .overflow_hidden()
                        .font_family(fonts::mono())
                        .child(term_icon(TermIcon::GitBranch, 12., 2., t.text_tertiary))
                        .child(div().truncate().child(branch)),
                )
            })
            .when(!shell, |d| {
                let copied = self.copied;
                let id_hover = id_hover.clone();
                d.child(
                    div()
                        .id("term-id")
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .px(px(4.))
                        .mx(px(-4.))
                        .rounded(px(4.))
                        .font_family(fonts::mono())
                        .text_size(TEXT_XS)
                        .when(id_hovered, |d| d.bg(t.bg_hover).text_color(t.text_secondary))
                        .on_hover(move |h, _, cx| {
                            id_hover.update(cx, |v, cx| {
                                *v = *h;
                                cx.notify();
                            });
                        })
                        .tooltip(native_tooltip("Copy session ID".into()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy_id(cx)))
                        .map(|d| {
                            if copied {
                                d.child(icon(
                                    Icon::Check,
                                    11.,
                                    2.,
                                    if id_hovered { t.text_secondary } else { t.text_tertiary },
                                ))
                                .child("Copied")
                            } else {
                                d.child(SharedString::from(self.session_id.chars().take(8).collect::<String>()))
                            }
                        }),
                )
            })
            .when(!shell && card.is_some(), |d| {
                d.child(usage_stats(
                    &format!("term-usage-{}", self.session_id),
                    card.and_then(|c| c.usage.as_ref()),
                    live,
                    window,
                    cx,
                ))
            });

        let heading = div()
            .flex()
            .flex_col()
            .justify_center()
            .min_w_0()
            .flex_1()
            .child(
                div()
                    .font_family(fonts::DISPLAY)
                    .text_size(TEXT_MD)
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(card_title(card)),
            )
            .child(sub);

        let actions_el = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .flex_none()
            .when(!shell, |d| {
                // MODAL-019, VOICE-041: the mic; what's said is typed in at
                // the session's prompt (resuming it if it's stopped), not sent.
                let typing_into = this.clone();
                let handle = window.window_handle();
                let on_text: crate::voice::OnText = std::rc::Rc::new(move |text: String, cx: &mut gpui::App| {
                    let view = typing_into.clone();
                    let _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |w, cx| w.type_in(&text, window, cx));
                    });
                });
                d.child(crate::voice::session_mic(
                    SharedString::from(format!("term-mic-{}", self.session_id)),
                    crate::voice::SessionMicProps {
                        project_id: self.project_id.clone(),
                        variant: crate::voice::MicVariant::Window,
                        scale: 1.,
                        on_text,
                    },
                    window,
                    cx,
                ))
            })
            .when(can_resume, |d| {
                let resume = this.clone();
                if shell {
                    d.child(
                        button("term-restart")
                            .tooltip("Start a new shell in the project folder")
                            .child(term_icon(TermIcon::RotateCcw, 13., 2., t.text_primary))
                            .child("Restart")
                            .on_click(move |_, _, cx| resume.update(cx, Self::resume)),
                    )
                } else {
                    d.child(
                        button("term-resume")
                            .child(term_icon(TermIcon::Play, 13., 2., t.text_primary))
                            .child("Resume")
                            .on_click(move |_, _, cx| resume.update(cx, Self::resume)),
                    )
                }
            })
            .when(can_stop, |d| {
                let stop = this.clone();
                d.child(
                    button("term-stop")
                        .tooltip("Stop the session. The card stays, and Resume picks it up.")
                        .child(term_icon(TermIcon::Square, 12., 2.4, t.text_primary))
                        .child("Stop")
                        .on_click(move |_, window, cx| stop.update(cx, |w, cx| w.stop(window, cx))),
                )
            })
            .map(|d| {
                if shell {
                    let remove = this.clone();
                    d.child(
                        button("term-remove")
                            .icon(Icon::Trash2, 13., 2.)
                            .label("Remove")
                            .tooltip("Stop the shell and remove this terminal from the board")
                            .on_click(move |_, _, cx| remove.update(cx, Self::remove)),
                    )
                } else if view_of {
                    // A view's brain goes with its view, never to the archive.
                    d
                } else {
                    let archive = this.clone();
                    d.child(
                        button("term-archive")
                            .icon(Icon::Archive, 13., 2.)
                            .label("Archive")
                            .tooltip("Stop and move to the archive")
                            .on_click(move |_, _, cx| archive.update(cx, Self::archive)),
                    )
                }
            })
            .child(div().w(px(1.)).h(px(18.)).mx(px(2.)).bg(t.separator))
            .child({
                let state = self.state.clone();
                button("term-close")
                    .ghost()
                    .square()
                    .icon(Icon::X, 16., 2.)
                    .tooltip(close_tooltip(Platform::CURRENT, shell))
                    .on_click(move |_, _, cx| actions::close_terminal(&state, cx))
            });

        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .flex_none()
            .h(px(48.))
            .pl(px(16.))
            .pr(px(10.))
            .bg(t.bg_elevated)
            // The panel clips to its rounded corners (`overflow: hidden`).
            .rounded_t(RADIUS_XL)
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(status_el)
            .child(heading)
            .child(actions_el)
            .into_any_element()
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(chat) = &self.chat {
            return div()
                .id("chat-body")
                .flex_1()
                .min_h_0()
                .rounded_b(RADIUS_XL)
                .overflow_hidden()
                .child(chat.clone())
                .into_any_element();
        }
        let shell = self.shell();
        let phase = self.host.read(cx).phase.clone();
        let this = cx.entity();
        let host = self.host.clone();
        div()
            .id("term-body")
            .relative()
            .flex_1()
            .min_h_0()
            .pt(px(12.))
            .pr(px(6.))
            .pb(px(10.))
            .pl(px(14.))
            .bg(rgb(TERM_BG))
            .rounded_b(RADIUS_XL)
            // MODAL-008: a press anywhere in the body, padding too, focuses the terminal.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                host.update(cx, |h, cx| h.focus_terminal(window, cx));
            })
            .child(div().relative().size_full().child(self.host.clone()))
            .when(phase.phase == Phase::Loading, |d| {
                d.child(loading_pill(
                    ("term-loading", self.host.entity_id().as_u64() as usize),
                    loading_label(shell, self.started_live),
                    window,
                    cx,
                ))
            })
            .when(phase.phase == Phase::Error, |d| {
                let title = if shell {
                    "Couldn’t start the shell"
                } else {
                    "Couldn’t start Claude"
                };
                d.child(error_overlay(
                    "term-error",
                    title,
                    phase.error.as_deref(),
                    std::rc::Rc::new(move |_, cx| this.update(cx, Self::resume)),
                ))
            })
            .into_any_element()
    }
}

impl Render for SessionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let card = card(self.state.read(cx), &self.session_id);
        let viewport = window.viewport_size();
        // MODAL-032: min(1400px, 92vw) × 88vh.
        let width = px(1400.).min(viewport.width * 0.92);
        let height = viewport.height * 0.88;
        let header = self.header(card.as_ref(), window, cx);
        let body = self.body(window, cx);
        let content = div()
            .flex()
            .flex_col()
            .w_full()
            .h(height)
            .line_height(relative(1.4))
            .child(header)
            .child(body);
        let state = self.state.clone();
        // MODAL-003: Escape is the terminal's; MODAL-004: the backdrop closes it.
        let modal = Modal::new("term-modal", width, move |_, cx| actions::close_terminal(&state, cx));
        let mut out = vec![modal.render(content, window, cx)];
        if let Some(ask) = self.confirm.clone() {
            let this = cx.entity();
            let resolve: super::confirm::Resolve =
                std::rc::Rc::new(move |ok, window, cx| this.update(cx, |w, cx| w.resolve_stop(ok, window, cx)));
            out.push(confirm_dialog(
                "term-stop-confirm",
                &ask,
                &self.confirm_focus,
                &resolve,
                window,
                cx,
            ));
        }
        let _ = &self.project_id;
        div().children(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_009_modal_010_loading_labels() {
        assert_eq!(loading_label(true, true), "Starting shell…");
        assert_eq!(loading_label(false, true), "Starting Claude…");
        assert_eq!(loading_label(false, false), "Resuming session…");
    }

    #[test]
    fn modal_027_close_tooltips() {
        assert_eq!(
            close_tooltip(Platform::Mac, false),
            "Close (⌘W). The session keeps running."
        );
        assert_eq!(
            close_tooltip(Platform::Windows, true),
            "Close (Ctrl+Shift+W). The shell keeps running."
        );
    }
}
