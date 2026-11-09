//! The merge dialog, "New session from context" (`MergeDialog.tsx`, spec 11
//! MERGE): each source summarizes itself in a forked copy, then the user
//! reviews the handoff and starts a session on it. The state machine is
//! `nodus_board::merge::Merge`; this file runs it (requests, progress
//! events, the fields), `view.rs` draws it. The board opens it with
//! `start_merge` and creates the session in `merge_start` (`board/bar.rs`).

mod grip;
mod view;

use std::time::Instant;

use gpui::component::input::{InputEvent, InputState, TextareaState};
use gpui::{AppContext as _, Context, Entity, Global, Subscription, Task, Window};
use nodus_board::merge::{Merge, Step};
use nodus_domain::{Event, SummarizePhase, SummaryResult};

use crate::board::bar::MergeStart;
use crate::board::canvas::BoardCanvas;

/// What a parity scene makes summarize answer, as 0.9.4's scenes replaced
/// the IPC handler: each source's phase, and the results (none: pending).
#[derive(Debug, Clone, Default)]
pub struct SummarizeStub {
    pub phases: Vec<Option<SummarizePhase>>,
    pub results: Option<Vec<SummaryResult>>,
}

impl Global for SummarizeStub {}

/// The open dialog.
pub(crate) struct MergeDialog {
    pub m: Merge,
    pub context: Entity<TextareaState>,
    pub instruction: Entity<TextareaState>,
    pub name: Entity<InputState>,
    pub starting: bool,
    pub start_error: Option<String>,
    /// When gathering started (the elapsed counter, MERGE-005).
    pub since: Instant,
    /// The instruction field's height once its grip was dragged, and the drag.
    pub instruction_h: Option<f32>,
    pub grip: Option<grip::GripDrag>,
    /// The instruction took the keyboard when review came (MERGE-013).
    focused_review: bool,
    /// The bar is setting the context field, not the user.
    setting: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl BoardCanvas {
    /// Opens the dialog when the board asks for one, and drops it when the
    /// board closes it (MERGE-001, MERGE-002).
    pub(crate) fn merge_sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        match (&self.merge, &self.overlays.merge) {
            (Some(sources), None) => {
                let sources: Vec<(String, String)> = sources
                    .iter()
                    .map(|s| (s.session_id.clone(), s.title.clone()))
                    .collect();
                let focus = self.merge_focus.clone();
                self.overlays.merge = Some(self.merge_open(&sources, focus.as_deref(), window, cx));
                let ids: Vec<String> = sources.into_iter().map(|(id, _)| id).collect();
                self.merge_run(ids, cx);
            }
            (None, Some(_)) => self.overlays.merge = None,
            _ => {}
        }
    }

    fn merge_open(
        &mut self,
        sources: &[(String, String)],
        focus: Option<&str>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> MergeDialog {
        let m = Merge::new(sources, focus);
        let context = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("No summaries yet. Retry the failed sessions or write the context yourself.")
                .context_menu(false)
        });
        let instruction = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 3)
                .placeholder("e.g. Combine both approaches and implement …")
                .context_menu(false)
        });
        let default_name = m.default_name();
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx).context_menu(false);
            s.set_value(default_name, window, cx);
            s
        });
        let subs = vec![cx.subscribe_in(&context, window, |this, field, e: &InputEvent, _, cx| {
            if let (InputEvent::Change, Some(d)) = (e, this.overlays.merge.as_mut())
                && !d.setting
            {
                // MERGE-011: an edit stops it following the generated text.
                let value = field.read(cx).value().to_string();
                d.m.edit_context(value);
                cx.notify();
            }
        })];
        // MERGE-004: progress events for this dialog's requests.
        let mut rx = self.state.read(cx).nodus.subscribe();
        let events = cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(Event::SummarizeProgress(p)) => {
                        let gone = this
                            .update(cx, |c, cx| {
                                if let Some(d) = c.overlays.merge.as_mut()
                                    && d.m.progress(&p)
                                {
                                    d.m.settle();
                                    cx.notify();
                                }
                            })
                            .is_err();
                        if gone {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        // MERGE-005: the elapsed counter ticks once a second while gathering.
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let gathering = this
                    .update(cx, |c, cx| {
                        let g = c.overlays.merge.as_ref().is_some_and(|d| d.m.step == Step::Gathering);
                        if g {
                            cx.notify();
                        }
                        g
                    })
                    .unwrap_or(false);
                if !gathering {
                    break;
                }
            }
        });
        MergeDialog {
            m,
            context,
            instruction,
            name,
            starting: false,
            start_error: None,
            since: Instant::now(),
            instruction_h: None,
            grip: None,
            focused_review: false,
            setting: false,
            _tasks: vec![events, tick],
            _subscriptions: subs,
        }
    }

    /// MERGE-004, MERGE-009: one summarize request for `ids`.
    pub(crate) fn merge_run(&mut self, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        let Some(d) = self.overlays.merge.as_mut() else { return };
        let request_id = uuid::Uuid::new_v4().to_string();
        d.m.begin(&ids, &request_id);
        let focus = d.m.focus.clone();
        cx.notify();
        if let Some(stub) = cx.try_global::<SummarizeStub>().cloned() {
            self.merge_run_stubbed(&ids, &request_id, stub, cx);
            return;
        }
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let asked = ids.clone();
            let answer = crate::board::on_core(&rt, async move {
                nodus
                    .sessions()
                    .summarize(&project_id, &asked, &request_id, focus.as_deref())
                    .await
            })
            .await
            .map_err(|e| e.ipc_message());
            let _ = this.update(cx, |c, cx| {
                // Closed meanwhile: the summaries are simply dropped.
                if let Some(d) = c.overlays.merge.as_mut() {
                    d.m.results(&ids, answer);
                    cx.notify();
                }
            });
        });
        if let Some(d) = self.overlays.merge.as_mut() {
            d._tasks.push(task);
        }
    }

    /// A parity scene's summarize: its phases as progress, then its results.
    fn merge_run_stubbed(&mut self, ids: &[String], request_id: &str, stub: SummarizeStub, cx: &mut Context<'_, Self>) {
        let Some(d) = self.overlays.merge.as_mut() else { return };
        for (i, id) in ids.iter().enumerate() {
            if let Some(Some(phase)) = stub.phases.get(i) {
                d.m.progress(&nodus_domain::SummarizeProgress {
                    request_id: request_id.to_owned(),
                    session_id: id.clone(),
                    phase: *phase,
                    error: (*phase == SummarizePhase::Error).then(|| "claude exited with code 1".to_owned()),
                });
            }
        }
        if let Some(results) = stub.results {
            let results = ids
                .iter()
                .zip(results)
                .map(|(id, r)| SummaryResult {
                    session_id: id.clone(),
                    ..r
                })
                .collect();
            d.m.results(ids, Ok(results));
        } else {
            d.m.settle();
        }
        cx.notify();
    }

    /// Before drawing: the context field follows the generated text until
    /// edited, and the instruction takes the keyboard once review comes.
    pub(crate) fn merge_prepare(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(d) = self.overlays.merge.as_mut() else { return };
        if !d.m.dirty && d.context.read(cx).value().as_ref() != d.m.context.as_str() {
            d.setting = true;
            let text = d.m.context.clone();
            d.context.update(cx, |s, cx| s.set_value(text, window, cx));
            d.setting = false;
        }
        if d.m.step == Step::Review && !d.focused_review {
            d.focused_review = true;
            let field = d.instruction.clone();
            window.on_next_frame(move |window, cx| field.update(cx, |s, cx| s.focus(window, cx)));
        }
    }

    /// MERGE-015, MERGE-016: starts the session; an error stays in the footer.
    pub(crate) fn merge_begin(&mut self, cx: &mut Context<'_, Self>) {
        let Some(d) = self.overlays.merge.as_mut() else { return };
        let instruction = d.instruction.read(cx).value().to_string();
        if !d.m.can_start(&instruction, d.starting) {
            return;
        }
        d.starting = true;
        d.start_error = None;
        let typed = d.name.read(cx).value().to_string();
        let name = match nodus_domain::js::trim(&typed) {
            "" => d.m.default_name(),
            n => n.to_owned(),
        };
        let start = MergeStart {
            name,
            initial_prompt: nodus_board::merge::initial_prompt(&d.m.context, &instruction),
            parents: d.m.succeeded(),
        };
        cx.notify();
        let created = self.merge_start(start, cx);
        cx.spawn(async move |this, cx| {
            if let Err(err) = created.await {
                let _ = this.update(cx, |c, cx| {
                    if let Some(d) = c.overlays.merge.as_mut() {
                        d.start_error = Some(err.ipc_message());
                        d.starting = false;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// MERGE-009.
    pub(crate) fn merge_retry(&mut self, id: String, cx: &mut Context<'_, Self>) {
        self.merge_run(vec![id], cx);
    }

    /// MERGE-010.
    pub(crate) fn merge_skip(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if let Some(d) = self.overlays.merge.as_mut() {
            d.m.skip(id);
            cx.notify();
        }
    }

    /// MERGE-011: back to the generated text, following again.
    pub(crate) fn merge_reset_context(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(d) = self.overlays.merge.as_mut() {
            d.m.reset_context();
            cx.notify();
        }
    }

    /// MERGE-002: Esc or Cancel.
    pub(crate) fn merge_cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.overlays.merge = None;
        self.close_merge(cx);
    }
}
