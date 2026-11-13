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
use crate::terminal::actions::PreviewFind;

pub(crate) use text::{MARK_RADIUS, UNDERLINE_OFFSET, highlighted, time_ago};

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
    /// The results' scroll, so the highlighted row stays in view (SRCH-012).
    pub scroll: gpui::ScrollHandle,
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

/// SRCH-005: live before archived, then the most recently updated first,
/// at most [`RECENT`] (a stable sort, as `Array.prototype.sort` is).
pub(crate) fn latest(mut cards: Vec<SessionCard>) -> Vec<SessionCard> {
    cards.sort_by(|a, b| {
        a.node.archived.cmp(&b.node.archived).then(
            updated_at(b)
                .partial_cmp(&updated_at(a))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    cards.truncate(RECENT);
    cards
}

/// BOARD-037: what a row's transcript opens at, `{query: trimmed search
/// query, snippet}`, the snippet being the hit's best (first) one.
pub(crate) fn row_find(query: &str, row: &Row) -> PreviewFind {
    PreviewFind {
        query: js::trim(query).to_owned(),
        snippet: row.hit.as_ref().and_then(|h| h.snippets.first().cloned()),
    }
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
            return latest(cards).into_iter().map(|card| Row { card, hit: None }).collect();
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
        // Focused now, so the next frame draws it with the keyboard; its
        // text all selected once drawn.
        field.update(cx, |s, cx| s.focus(window, cx));
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
        self.search_reset(cx);
        self.close_search(cx);
    }

    /// A fresh state in place of the search's, and the old one kept for its
    /// exit (MOTION-011). Its field, its subscriptions and the request on
    /// its way go now, as they always went: nothing that's leaving keeps
    /// the keyboard or answers.
    fn search_reset(&mut self, cx: &mut Context<'_, Self>) {
        let mut was = std::mem::take(&mut self.overlays.search);
        was.field = None;
        was.pending = None;
        was._subscriptions.clear();
        self.overlays.search_leaving = crate::components::presence::leave(was, cx);
    }

    /// SRCH-013…SRCH-015: opens a row, through the board's jump (BOARD-036,
    /// BOARD-037), which closes the search when the card is on the board.
    pub(crate) fn search_open_row(&mut self, row: &Row, cx: &mut Context<'_, Self>) {
        let find = row_find(&self.search_query, row);
        self.jump_to_card(&row.card.node.session_id, Some(find), cx);
        if !self.search_open {
            self.search_reset(cx);
        }
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
        self.search_reset(cx);
        self.search_start(&ids, cx);
    }

    /// SRCH-012, SRCH-013, SRCH-016, SRCH-017, SRCH-003: the dialog's keys.
    pub(crate) fn search_key_down(&mut self, e: &gpui::KeyDownEvent, cx: &mut Context<'_, Self>) -> bool {
        let rows = self.search_rows();
        let last = rows.len().saturating_sub(1);
        let current = self.overlays.search.active.min(last);
        let moved = matches!(e.keystroke.key.as_str(), "down" | "up" | "tab");
        let handled = self.search_key(e, &rows, current, last, cx);
        if handled && moved {
            // The highlighted row scrolls into view, the least it can.
            let header = usize::from(js::trim(&self.search_query).is_empty() && !rows.is_empty());
            let row = self.overlays.search.active.min(last);
            self.overlays.search.scroll.scroll_to_item(row + header);
        }
        handled
    }

    fn search_key(
        &mut self,
        e: &gpui::KeyDownEvent,
        rows: &[Row],
        current: usize,
        last: usize,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let m = e.keystroke.modifiers;
        let modk = crate::platform::Platform::CURRENT.mod_key(&m);
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

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNode, SessionStatus};
    use serde_json::Map;

    fn card(id: &str, archived: bool, created_at: f64) -> SessionCard {
        SessionCard {
            node: BoardNode {
                session_id: id.into(),
                kind: None,
                x: 0.0,
                y: 0.0,
                archived,
                title: None,
                parents: None,
                created_by_app: None,
                run_of: None,
                expanded: None,
                view_of: None,
                auto_mode: None,
                from_view: None,
                created_at,
                w: None,
                h: None,
                archived_at: None,
                section_id: None,
                extra: Map::new(),
            },
            meta: None,
            status: SessionStatus::Done,
            live: false,
            agents: None,
            agent_count: None,
            usage: None,
        }
    }

    #[test]
    fn lists_live_sessions_first_then_the_latest() {
        let cards = vec![
            card("old", false, 1.0),
            card("gone", true, 9.0),
            card("new", false, 5.0),
            card("tie", false, 5.0),
        ];
        let ids: Vec<String> = latest(cards).into_iter().map(|c| c.node.session_id).collect();
        assert_eq!(ids, ["new", "tie", "old", "gone"]);
        let many: Vec<SessionCard> = (0..40).map(|i| card(&i.to_string(), false, f64::from(i))).collect();
        let kept = latest(many);
        assert_eq!(kept.len(), RECENT);
        assert_eq!(kept[0].node.session_id, "39");
    }

    #[test]
    fn a_row_opens_at_its_best_match_for_the_trimmed_query() {
        // BOARD-037: `{query: searchQuery.trim(), snippet: hit.snippets[0]}`.
        let snippet = |text: &str, timestamp: f64| nodus_domain::SearchSnippet {
            kind: nodus_domain::SearchSnippetKind::Reply,
            tool: None,
            file: None,
            agent: None,
            text: text.into(),
            ranges: vec![[0, 4]],
            timestamp,
        };
        let row = Row {
            card: card("gone", true, 1.0),
            hit: Some(SessionSearchHit {
                session_id: "gone".into(),
                matches: 2,
                snippets: vec![snippet("best", 20.0), snippet("next", 10.0)],
            }),
        };
        let find = row_find("  tax rounding \n", &row);
        assert_eq!(find.query, "tax rounding");
        assert_eq!(find.snippet, Some(snippet("best", 20.0)));
        // A title-only hit, or the latest sessions' rows, have no snippet.
        let bare = Row {
            card: card("gone", true, 1.0),
            hit: None,
        };
        assert_eq!(
            row_find("", &bare),
            PreviewFind {
                query: String::new(),
                snippet: None
            }
        );
    }
}
