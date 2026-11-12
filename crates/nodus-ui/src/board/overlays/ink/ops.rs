//! What the ink does: changes and their history, tools, colours, labels.

use super::*;

impl BoardCanvas {
    // ---- the strokes ------------------------------------------------------------------

    /// The board's ink as it stands (`strokesRef.current`).
    pub(crate) fn ink_strokes(&self) -> Vec<InkStroke> {
        self.board.as_ref().map(|b| b.ink.clone()).unwrap_or_default()
    }

    /// INKUI-024: the ink changes at once on the board and is saved.
    fn set_ink(&mut self, next: Vec<InkStroke>, cx: &mut Context<'_, Self>) {
        if let Some(b) = self.board.as_mut() {
            b.ink = next.clone();
        }
        if let Some(s) = self.source.as_mut() {
            s.ink = next.clone();
        }
        self.state.update(cx, |s, cx| s.save_ink(next, cx));
        cx.notify();
    }

    /// A change ⌘Z can take back (`commit`).
    pub(crate) fn ink_commit(&mut self, next: Vec<InkStroke>, cx: &mut Context<'_, Self>) {
        let current = self.ink_strokes();
        self.overlays.ink.history.commit(current);
        self.set_ink(next, cx);
    }

    // ---- the keys and the dock --------------------------------------------------------

    /// INKUI-003: choosing a tool finishes a label; any tool but the pointer
    /// lets go of the selected stroke.
    pub(crate) fn ink_set_tool(&mut self, tool: InkTool, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(window, cx);
        // BUBBLE-003, BUBBLE-044: what the tool left behind goes with it.
        if tool != InkTool::Bubble {
            self.close_empty_bubble(cx);
        }
        if tool != InkTool::Mic {
            self.voice_cancel(cx);
        }
        self.overlays.ink.tool = tool;
        if tool != InkTool::Select {
            self.overlays.ink.selected = None;
            // INKUI-017: switching away from the pointer drops the move cursor.
            self.overlays.ink.hover = false;
        }
        cx.notify();
    }

    /// INKUI-023 (BOARD-160): back to the pointer, or lets go of the
    /// selected stroke. False when there was nothing to do.
    pub(crate) fn ink_escape(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.overlays.ink.tool != InkTool::Select {
            self.ink_set_tool(InkTool::Select, None, cx);
            return true;
        }
        if self.overlays.ink.selected.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// INKUI-022 (BOARD-163): ⌫ deletes the selected stroke.
    pub(crate) fn ink_delete_selected(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(id) = self.overlays.ink.selected.take() else {
            return false;
        };
        let next = self.ink_strokes().into_iter().filter(|s| s.id != id).collect();
        self.ink_commit(next, cx);
        true
    }

    /// INKUI-021 (BOARD-159): ⌘Z. False when there's nothing to undo.
    pub(crate) fn ink_undo(&mut self, cx: &mut Context<'_, Self>) -> bool {
        self.ink_finish_label(None, cx);
        let current = self.ink_strokes();
        let Some(prev) = self.overlays.ink.history.undo(current) else {
            return false;
        };
        self.overlays.ink.selected = None;
        self.set_ink(prev, cx);
        true
    }

    /// INKUI-021: ⇧⌘Z.
    pub(crate) fn ink_redo(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let current = self.ink_strokes();
        let Some(next) = self.overlays.ink.history.redo(current) else {
            return false;
        };
        self.overlays.ink.selected = None;
        self.set_ink(next, cx);
        true
    }

    /// INKUI-004: a colour from the palette; a selected stroke takes it too.
    pub(crate) fn ink_pick_color(&mut self, color: InkColor, cx: &mut Context<'_, Self>) {
        self.overlays.ink.color = Some(color);
        self.overlays.ink.palette = false;
        self.state
            .read(cx)
            .nodus
            .local_storage()
            .set(COLOR_KEY, Some(color.as_str()));
        let current = self.ink_strokes();
        if let Some(id) = self.overlays.ink.selected.clone()
            && current.iter().any(|s| s.id == id && s.color != color)
        {
            let next = current
                .into_iter()
                .map(|s| if s.id == id { InkStroke { color, ..s } } else { s })
                .collect();
            self.ink_commit(next, cx);
        }
        cx.notify();
    }

    /// INKUI-005.
    pub(crate) fn ink_clear_all(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(Some(window), cx);
        if self.ink_strokes().is_empty() {
            return;
        }
        self.overlays.ink.selected = None;
        self.ink_commit(Vec::new(), cx);
        self.show_toast(format!("Cleared the ink. Press {} to bring it back.", keys("⌘Z")), cx);
    }

    // ---- labels -----------------------------------------------------------------------

    /// INKUI-013, INKUI-014: puts a label being typed on the board (or
    /// takes an edited one off, emptied).
    pub(crate) fn ink_finish_label(&mut self, window: Option<&mut Window>, cx: &mut Context<'_, Self>) {
        let Some(draft) = self.overlays.ink.label.take() else {
            return;
        };
        if let Some(window) = window
            && draft.input.read(cx).focus_handle(cx).is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
        let text = js::trim(&draft.input.read(cx).value()).to_owned();
        let current = self.ink_strokes();
        match &draft.id {
            Some(id) => {
                let Some(old) = current.iter().find(|s| &s.id == id) else {
                    cx.notify();
                    return;
                };
                if old.text.as_deref() == Some(text.as_str()) {
                    cx.notify();
                    return;
                }
                let next = if text.is_empty() {
                    current.into_iter().filter(|s| &s.id != id).collect()
                } else {
                    current
                        .into_iter()
                        .map(|s| {
                            if &s.id == id {
                                InkStroke {
                                    text: Some(text.clone()),
                                    ..s
                                }
                            } else {
                                s
                            }
                        })
                        .collect()
                };
                self.ink_commit(next, cx);
            }
            None if !text.is_empty() => {
                let mut next = current;
                next.push(InkStroke {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: InkKind::Text,
                    color: draft.color,
                    points: flatten(&[[draft.x, draft.y]]),
                    size: draft.size,
                    text: Some(text),
                    extra: serde_json::Map::new(),
                });
                self.ink_commit(next, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// Esc in a label: a new one stays off the board, an edited one as it was.
    pub(crate) fn ink_discard_label(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.overlays.ink.label.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Return in a label.
    pub(crate) fn ink_commit_label(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ink_finish_label(Some(window), cx);
    }

    /// Whether an input method is composing in the label.
    pub(crate) fn ink_label_composing(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        use gpui::EntityInputHandler as _;
        let Some(label) = &self.overlays.ink.label else {
            return false;
        };
        label
            .input
            .update(cx, |s, cx| s.marked_text_range(window, cx))
            .is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ink_open_label(
        &mut self,
        id: Option<String>,
        at: Pt,
        size: f64,
        color: InkColor,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = text.to_owned();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Label").context_menu(false);
            state.set_value(text, window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Blur => this.ink_finish_label(Some(window), cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        ];
        self.overlays.ink.label_key += 1;
        self.overlays.ink.label = Some(LabelDraft {
            id,
            x: at[0],
            y: at[1],
            size,
            color,
            input: input.clone(),
            _subscriptions: subscriptions,
        });
        // Focused once drawn, with the caret at the end.
        // Focused now, so the next frame draws it with the keyboard; and
        // again once drawn, with the caret at the end.
        input.update(cx, |s, cx| s.focus(window, cx));
        window.on_next_frame(move |window, _| {
            window.refresh();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |s, cx| {
                    s.focus(window, cx);
                    let end = s.value().len();
                    s.set_selected_range(end..end, cx);
                });
            });
        });
        cx.notify();
    }
}
