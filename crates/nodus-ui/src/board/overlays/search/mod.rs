//! ⌘K session search (`SessionSearch.tsx`, spec 11 SRCH): what was said
//! and done in every session on the board, live and archived. With nothing
//! typed it lists the latest sessions. This file holds its state and what it
//! does; `view.rs` draws it, `text.rs` holds the highlighting and times.
//! The board keeps it open (`search_open`) and keeps its query.

mod text;
mod view;

use std::time::Duration;

use gpui::component::input::{InputEvent, InputState};
use gpui::{AppContext as _, Context, Entity, Subscription, Task, Window};
use nodus_domain::{SessionCard, SessionSearchHit, js};

use crate::board::canvas::BoardCanvas;
use crate::board::layout::is_terminal;
use crate::terminal::actions::{PreviewFind, open_preview};

pub(crate) use text::{highlighted, time_ago};

/// SRCH-006: the query is sent this long after the last edit.
pub const DEBOUNCE_MS: u64 = 120;
/// SRCH-005: how many latest sessions an empty query lists.
pub const RECENT: usize = 30;

/// The open search's state; it goes when the search closes.
#[derive(Default)]
pub(crate) struct Search {
    pub field: Option<Entity<InputState>>,
    /// The answer last received, and the query it answers.
    pub answer: Option<(String, Vec<SessionSearchHit>)>,
    pub error: Option<String>,
    pub active: usize,
    /// Marked sessions, in marking order (SRCH-016).
    pub marked: Vec<String>,
    /// The query the latest request is for (stale answers are dropped).
    pub asking: Option<String>,
    pending: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// A row: a card, and its hit when a query has answered.
#[derive(Clone)]
pub(crate) struct Row {
    pub card: SessionCard,
    pub hit: Option<SessionSearchHit>,
}

/// `card.meta?.updatedAt ?? card.createdAt`.
pub(crate) fn updated_at(card: &SessionCard) -> f64 {
    card.meta.as_ref().map_or(card.node.created_at, |m| m.updated_at)
}

impl BoardCanvas {
    /// SRCH-018: the board's cards but terminals (runs and view brains are
    /// already off the board).
    fn searchable(&self) -> Vec<SessionCard> {
        self.cards().iter().filter(|c| !is_terminal(c)).cloned().collect()
    }

    /// SRCH-005, SRCH-007: the rows shown.
    pub(crate) fn search_rows(&self) -> Vec<Row> {
        let trimmed = js::trim(&self.search_query);
        let cards = self.searchable();
        if trimmed.is_empty() {
            let mut cards = cards;
            cards.sort_by(|a, b| {
                a.node.archived.cmp(&b.node.archived).then(
                    updated_at(b)
                        .partial_cmp(&updated_at(a))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
            });
            return cards
                .into_iter()
                .take(RECENT)
                .map(|card| Row { card, hit: None })
                .collect();
        }
        let Some((_, hits)) = &self.overlays.search.answer else {
            return Vec::new();
        };
        hits.iter()
            .filter_map(|hit| {
                let card = cards.iter().find(|c| c.node.session_id == hit.session_id)?;
                Some(Row {
                    card: card.clone(),
                    hit: Some(hit.clone()),
                })
            })
            .collect()
    }

    /// SRCH-007: a query whose answer hasn't come, and no error showing.
    pub(crate) fn search_loading(&self) -> bool {
        let trimmed = js::trim(&self.search_query);
        let s = &self.overlays.search;
        !trimmed.is_empty() && s.answer.as_ref().map(|(q, _)| q.as_str()) != Some(trimmed) && s.error.is_none()
    }

    /// The field, made when the search opens, with the kept query all
    /// selected (SRCH-004).
    pub(crate) fn search_field(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<InputState> {
        if let Some(field) = &self.overlays.search.field {
            return field.clone();
        }
        let query = self.search_query.clone();
        let field = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("Search what every session said and did")
                .context_menu(false);
            state.set_value(query, window, cx);
            state
        });
        let sub = cx.subscribe_in(&field, window, |this, field, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                let value = field.read(cx).value().to_string();
                this.search_query_changed(value, cx);
            }
        });
        self.overlays.search._subscriptions.push(sub);
        self.overlays.search.field = Some(field.clone());
        crate::components::text_field::focus_all(&field, window, cx);
        // An open with a query asks again at once (the answer shown meanwhile is the last one).
        let trimmed = js::trim(&self.search_query).to_owned();
        if !trimmed.is_empty() {
            self.search_ask(trimmed, cx);
        }
        field
    }

    /// SRCH-006, SRCH-012: an edit.
    fn search_query_changed(&mut self, value: String, cx: &mut Context<'_, Self>) {
        let before = js::trim(&self.search_query).to_owned();
        self.set_search_query(value, cx);
        let trimmed = js::trim(&self.search_query).to_owned();
        if trimmed != before {
            self.overlays.search.active = 0;
            if trimmed.is_empty() {
                self.overlays.search.pending = None;
                self.overlays.search.asking = None;
            } else {
                self.search_ask(trimmed, cx);
            }
        }
        cx.notify();
    }

    /// Asks the backend for `query` after the debounce.
    fn search_ask(&mut self, query: String, cx: &mut Context<'_, Self>) {
        self.overlays.search.asking = Some(query.clone());
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        self.overlays.search.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(DEBOUNCE_MS)).await;
            let q = query.clone();
            let found = crate::board::on_core(&rt, async move { nodus.sessions().search(&project_id, &q).await }).await;
            let _ = this.update(cx, |c, cx| {
                let s = &mut c.overlays.search;
                if !c.search_open || s.asking.as_deref() != Some(query.as_str()) {
                    return;
                }
                match found {
                    Ok(hits) => {
                        s.answer = Some((query, hits));
                        s.error = None;
                        s.active = 0;
                    }
                    Err(err) => s.error = Some(err.ipc_message()),
                }
                cx.notify();
            });
        }));
    }

    /// SRCH-003: closes, dropping its state (the query stays).
    pub(crate) fn search_close(&mut self, cx: &mut Context<'_, Self>) {
        self.overlays.search = Search::default();
        self.close_search(cx);
    }

    /// SRCH-013…SRCH-015: opens a row.
    pub(crate) fn search_open_row(&mut self, row: &Row, cx: &mut Context<'_, Self>) {
        let id = row.card.node.session_id.clone();
        if row.card.node.archived {
            let find = PreviewFind {
                query: js::trim(&self.search_query).to_owned(),
                snippet: row.hit.as_ref().and_then(|h| h.snippets.first().cloned()),
            };
            self.search_close(cx);
            let state = self.state.clone();
            open_preview(&state, &id, Some(find), cx);
            return;
        }
        self.overlays.search = Search::default();
        self.jump_to_card(&id, cx);
    }

    /// SRCH-016.
    pub(crate) fn search_toggle(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let marked = &mut self.overlays.search.marked;
        match marked.iter().position(|m| m == id) {
            Some(i) => {
                marked.remove(i);
            }
            None => marked.push(id.to_owned()),
        }
        cx.notify();
    }

    /// SRCH-017: from the marks, or the highlighted row.
    pub(crate) fn search_start_merge(&mut self, cx: &mut Context<'_, Self>) {
        let rows = self.search_rows();
        let current = self.overlays.search.active.min(rows.len().saturating_sub(1));
        let ids = if self.overlays.search.marked.is_empty() {
            rows.get(current)
                .map(|r| vec![r.card.node.session_id.clone()])
                .unwrap_or_default()
        } else {
            self.overlays.search.marked.clone()
        };
        if ids.is_empty() {
            return;
        }
        self.overlays.search = Search::default();
        self.search_start(&ids, cx);
    }

    /// SRCH-012, SRCH-013, SRCH-016, SRCH-017, SRCH-003: the dialog's keys.
    pub(crate) fn search_key_down(&mut self, e: &gpui::KeyDownEvent, cx: &mut Context<'_, Self>) -> bool {
        let m = e.keystroke.modifiers;
        let modk = crate::platform::Platform::CURRENT.mod_key(&m);
        let rows = self.search_rows();
        let last = rows.len().saturating_sub(1);
        let current = self.overlays.search.active.min(last);
        match e.keystroke.key.as_str() {
            "escape" => self.search_close(cx),
            "down" => self.overlays.search.active = (current + 1).min(last),
            "up" => self.overlays.search.active = current.saturating_sub(1),
            "enter" if modk => self.search_start_merge(cx),
            "enter" => {
                if let Some(row) = rows.get(current) {
                    self.search_open_row(row, cx);
                }
            }
            "tab" => {
                // Like fzf: Tab marks and moves on.
                if let Some(row) = rows.get(current) {
                    self.search_toggle(&row.card.node.session_id.clone(), cx);
                    self.overlays.search.active = if m.shift {
                        current.saturating_sub(1)
                    } else {
                        (current + 1).min(last)
                    };
                }
            }
            "k" if modk => self.search_close(cx),
            _ => return false,
        }
        cx.notify();
        true
    }
}
