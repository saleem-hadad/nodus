//! The action dialog (`ActionDialog.tsx`, `actions.css`; ACT-067…ACT-080):
//! describe the button, let Claude draft it, review the draft and save it.
//! [`nodus_actions::dialog::ActionDialog`] holds what it shows and decides
//! what each press does; this view keeps its three fields in step with it,
//! makes the `actions:draft` and `actions:save` calls and draws it
//! ([`super::dialog_view`]).

use std::rc::Rc;

use gpui::component::input::{InputEvent, InputState, TextareaState};
use gpui::{App, AppContext as _, Context, Entity, FocusHandle, Keystroke, Subscription, Window};
use nodus_actions::dialog::{ActionDialog, DESCRIPTION_PLACEHOLDER, Step, Submit};
use nodus_domain::ProjectAction;

use crate::platform::Platform;
use crate::state::AppState;

/// Closes the dialog (its owner drops it).
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// ACT-074: the name field's `maxLength`, in UTF-16 units.
pub const NAME_MAX: usize = 40;

pub struct ActionDialogView {
    pub(super) state: Entity<AppState>,
    pub(super) project_id: String,
    pub(super) model: ActionDialog,
    pub(super) on_close: OnClose,
    pub(super) description: Entity<TextareaState>,
    pub(super) name: Entity<InputState>,
    pub(super) prompt: Entity<TextareaState>,
    /// Spelling in the description, the name and the prompt (Chromium's
    /// default `spellcheck`; the name is a single-line field).
    pub(super) spell: [Entity<crate::components::spellcheck::Spellcheck>; 3],
    /// The dialog's own focus, for its keys when no field has them.
    pub(super) focus: FocusHandle,
    /// The step whose field was last given the keyboard (`autoFocus`).
    focused_step: Option<Step>,
    _subscriptions: Vec<Subscription>,
}

impl ActionDialogView {
    /// ACT-067: a new action starts on describe with nothing filled in; an
    /// existing one on review with its own.
    pub fn new(
        state: Entity<AppState>,
        project_id: String,
        action: Option<ProjectAction>,
        on_close: OnClose,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let model = ActionDialog::new(action);
        let description = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx)
                .rows(5)
                .soft_wrap(true)
                .placeholder(DESCRIPTION_PLACEHOLDER)
                .context_menu(false);
            s.set_value(model.description.clone(), window, cx);
            s
        });
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .context_menu(false)
                .validate(|text, _| nodus_domain::js::utf16_len(text) <= NAME_MAX);
            s.set_value(model.name.clone(), window, cx);
            s
        });
        let prompt = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).soft_wrap(true).context_menu(false);
            s.set_value(model.prompt.clone(), window, cx);
            s
        });
        let subscriptions = vec![
            cx.subscribe(&description, |this, field, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.model.description = field.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&name, |this, field, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.model.name = field.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&prompt, |this, field, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.model.prompt = field.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];
        // ACT-078, ACT-079: ⌘/Ctrl+Enter and Esc are the dialog's wherever
        // the keyboard is in it, before a field's own bindings see them.
        let window_id = window.window_handle().window_id();
        let weak = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |e, window, cx| {
            if window.window_handle().window_id() != window_id {
                return;
            }
            let Some(view) = weak.upgrade() else { return };
            if view.update(cx, |v, cx| v.on_key(&e.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        let mut subscriptions = subscriptions;
        subscriptions.push(keys);
        let spell = {
            use crate::components::spellcheck::spellcheck;
            [
                spellcheck(&description, false, window, cx),
                spellcheck(&name, true, window, cx),
                spellcheck(&prompt, false, window, cx),
            ]
        };
        Self {
            state,
            project_id,
            model,
            on_close,
            description,
            name,
            prompt,
            spell,
            focus: cx.focus_handle(),
            focused_step: None,
            _subscriptions: subscriptions,
        }
    }

    /// ACT-073: closing drops any draft still to come.
    pub fn closed(&mut self, _cx: &mut Context<'_, Self>) {
        self.model.close();
    }

    /// ACT-079: Esc, the backdrop or Cancel: closes without saving.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.model.close();
        let on_close = self.on_close.clone();
        cx.defer_in(window, move |_, window, cx| on_close(window, cx));
    }

    /// `autoFocus`: the step's first field takes the keyboard once it's
    /// drawn (a focus handle no frame has drawn yet doesn't keep it).
    pub(super) fn autofocus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.focused_step == Some(self.model.step) {
            return;
        }
        self.focused_step = Some(self.model.step);
        let this = cx.entity();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                this.update(cx, |v, cx| {
                    match v.model.step {
                        Step::Describe => v.description.update(cx, |s, cx| s.focus(window, cx)),
                        Step::Review => v.name.update(cx, |s, cx| s.focus(window, cx)),
                    }
                    cx.notify();
                });
            });
            // A driven window that's not in front draws only when asked.
            window.refresh();
        });
        window.refresh();
    }

    /// ACT-070: a chip's text replaces the description.
    pub(super) fn pick_chip(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.model.pick_chip(index);
        let text = self.model.description.clone();
        self.description.update(cx, |s, cx| s.set_value(text, window, cx));
        cx.notify();
    }

    /// ACT-074: an icon is picked.
    pub(super) fn pick_icon(&mut self, icon: &str, cx: &mut Context<'_, Self>) {
        icon.clone_into(&mut self.model.icon);
        cx.notify();
    }

    /// ACT-071: the scope switch (no redraft on review).
    pub(super) fn set_global(&mut self, global: bool, cx: &mut Context<'_, Self>) {
        self.model.global = global;
        cx.notify();
    }

    /// ACT-076: `Back`, `Redraft…` or `Back to the draft`.
    pub(super) fn back(&mut self, cx: &mut Context<'_, Self>) {
        self.model.back();
        cx.notify();
    }

    /// ACT-072, ACT-075, ACT-078: the primary button, or ⌘/Ctrl+Enter.
    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let new_id = uuid::Uuid::new_v4().to_string();
        let now = crate::parity::now_ms(cx);
        match self.model.submit(&new_id, now) {
            Submit::Draft(request) => {
                let nodus = self.state.read(cx).nodus.clone();
                let project = self.project_id.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let drafted = nodus
                        .actions()
                        .draft(&project, &request.description, request.global)
                        .await
                        .map_err(|e| e.ipc_message());
                    let _ = this.update_in(cx, |v, window, cx| v.drafted(request.id, drafted, window, cx));
                })
                .detach();
            }
            Submit::Save(raw) => {
                let nodus = self.state.read(cx).nodus.clone();
                let project = self.project_id.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let saved = nodus.actions().save(&project, &raw).await;
                    let _ = this.update_in(cx, |v, window, cx| match saved {
                        // ACT-017: the store takes the lists; the dialog goes.
                        Ok(lists) => {
                            v.state.update(cx, |s, cx| super::set_actions(s, lists, cx));
                            v.close(window, cx);
                        }
                        Err(e) => {
                            v.model.save_failed(e.ipc_message());
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            Submit::Nothing => {}
        }
        cx.notify();
    }

    /// ACT-072: the draft's answer fills the review step.
    fn drafted(
        &mut self,
        id: i64,
        result: Result<nodus_domain::ActionDraft, String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let step = self.model.step;
        self.model.draft_done(id, result);
        if self.model.step != step || self.model.step == Step::Review {
            let (name, prompt) = (self.model.name.clone(), self.model.prompt.clone());
            self.name.update(cx, |s, cx| s.set_value(name, window, cx));
            self.prompt.update(cx, |s, cx| s.set_value(prompt, window, cx));
        }
        cx.notify();
    }

    /// ACT-078, ACT-079: ⌘/Ctrl+Enter submits and Esc closes, from anywhere
    /// in the dialog (taken before the fields see them).
    fn on_key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let m = &keystroke.modifiers;
        match keystroke.key.as_str() {
            "escape" if !m.modified() => {
                self.close(window, cx);
                true
            }
            "enter" if Platform::CURRENT.mod_key(m) => {
                self.submit(window, cx);
                true
            }
            _ => false,
        }
    }
}
