//! The action dialog (`ActionDialog.tsx`; ACT-067…ACT-080). Work in progress.

use std::rc::Rc;

use gpui::{App, Context, Entity, IntoElement, Render, Window, div};
use nodus_domain::ProjectAction;

use crate::state::AppState;

/// Closes the dialog (its owner drops it).
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct ActionDialogView {
    _state: Entity<AppState>,
    _project_id: String,
    _action: Option<ProjectAction>,
    _on_close: OnClose,
}

impl ActionDialogView {
    pub fn new(
        state: Entity<AppState>,
        project_id: String,
        action: Option<ProjectAction>,
        on_close: OnClose,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Self {
        Self {
            _state: state,
            _project_id: project_id,
            _action: action,
            _on_close: on_close,
        }
    }

    /// ACT-073: closing drops any draft still to come.
    pub fn closed(&mut self, _cx: &mut Context<'_, Self>) {}
}

impl Render for ActionDialogView {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
    }
}
