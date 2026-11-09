//! The board's side of the overlays (spec 09 § Prompt bar wiring and
//! § Overlays; spec 11 draws them): what the prompt bar sends into or
//! starts, the merge dialog's sources and its new session, ⌘K search's open
//! state and query, and the New view dialog's point. The overlays read this
//! state from the canvas and call these commands.

use gpui::{Context, Task};
use nodus_board::compose::{ContextItem, with_context};
use nodus_board::geometry::{CARD_SIZE, Point};
use nodus_domain::{BoardNote, CreateSessionOptions, FolderColor, NodePatch, SessionStatus, js};

use super::canvas::BoardCanvas;
use super::layout::{
    NEW_CARD_SIZE, NOTE_SIZE, card_obstacles, card_size, card_title, find_free_spot, is_terminal, section_at,
    section_views, spot_below, spot_for_new_session, truncate,
};

/// What the prompt bar sends (`Submission`).
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    /// A new Claude session on the text, with the selection as context.
    Session { text: String },
    /// The text, sent into a session on the board.
    Into { session_id: String, text: String },
    /// A view made from the text.
    View { text: String },
    /// A sticky note of the text.
    Note { text: String, color: FolderColor },
}

/// A session the merge dialog draws from (`{ sessionId, title }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSource {
    pub session_id: String,
    pub title: String,
}

/// What the merge dialog starts its session with (`MergeStartOptions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeStart {
    pub name: String,
    pub initial_prompt: String,
    pub parents: Vec<String>,
}

/// The selection bar's counts (spec 09 § Selection bar wiring): selected
/// cards, the Claude sessions among them, the active ones, and the ones a
/// group would take.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionCounts {
    pub count: usize,
    pub session_count: usize,
    pub active_count: usize,
    pub group_count: usize,
}

fn message(err: &nodus_core::CommandError) -> String {
    err.ipc_message()
}

impl BoardCanvas {
    /// BOARD-131: the session the bar sends into: the one selected card,
    /// when it's an active Claude session and no written note or view is
    /// selected with it.
    pub fn bar_into(&self) -> Option<String> {
        let board = self.board.as_ref()?;
        let cards = self.selected_cards();
        let [id] = cards.as_slice() else { return None };
        let written = self.selected_notes().iter().any(|n| {
            board
                .notes
                .iter()
                .any(|note| &note.id == n && !js::trim(&note.text).is_empty())
        });
        if written || !self.selected_views().is_empty() {
            return None;
        }
        let card = board.cards.iter().find(|c| &c.node.session_id == id)?;
        (!card.node.archived && !is_terminal(card)).then(|| id.clone())
    }

    /// BOARD-132: what a new session from the bar would read first.
    pub fn bar_context_count(&self) -> usize {
        self.bar_context().1.len()
    }

    /// The selection as context, in board order: the non-terminal cards
    /// (and those cards), written notes, views (BOARD-133, BAR-025).
    fn bar_context(&self) -> (Vec<nodus_domain::SessionCard>, Vec<ContextItem>) {
        let Some(board) = &self.board else {
            return (Vec::new(), Vec::new());
        };
        let (card_ids, note_ids, view_ids) = (self.selected_cards(), self.selected_notes(), self.selected_views());
        let sources: Vec<nodus_domain::SessionCard> = board
            .cards
            .iter()
            .filter(|c| card_ids.contains(&c.node.session_id) && !is_terminal(c))
            .cloned()
            .collect();
        let mut items: Vec<ContextItem> = sources
            .iter()
            .map(|c| ContextItem::Session {
                title: card_title(c),
                branch: c.meta.as_ref().and_then(|m| m.git_branch.clone()),
                last_reply: c.meta.as_ref().and_then(|m| m.last_assistant_text.clone()),
                transcript: c.meta.as_ref().map(|m| m.transcript_path.clone()),
            })
            .collect();
        items.extend(
            board
                .notes
                .iter()
                .filter(|n| note_ids.contains(&n.id) && !js::trim(&n.text).is_empty())
                .map(|n| ContextItem::Note { text: n.text.clone() }),
        );
        items.extend(
            board
                .views
                .iter()
                .filter(|v| view_ids.contains(&v.id))
                .map(|v| ContextItem::View {
                    title: v.title.clone(),
                    description: v.description.clone(),
                }),
        );
        (sources, items)
    }

    /// `submitFromBar`: true when the bar should clear (BOARD-133…BOARD-137).
    pub fn submit_from_bar(&mut self, submission: Submission, cx: &mut Context<'_, Self>) -> Task<bool> {
        match submission {
            Submission::Session { text } => self.start_bar_session(text, cx),
            Submission::Into { session_id, text } => self.send_to_session(session_id, text, cx),
            Submission::View { text } => {
                // BOARD-136.
                let made = self.create_view(text, None, cx);
                cx.spawn(async move |this, cx| match made.await {
                    Ok(()) => true,
                    Err(err) => {
                        let _ = this.update(cx, |c, cx| {
                            c.show_toast(format!("Couldn't create the view: {}", message(&err)), cx);
                        });
                        false
                    }
                })
            }
            Submission::Note { text, color } => {
                // BOARD-137.
                let spot = self.free_note_spot(self.viewport_centre(NOTE_SIZE));
                let note = BoardNote {
                    id: uuid::Uuid::new_v4().to_string(),
                    text,
                    color,
                    x: js::round(spot.x),
                    y: js::round(spot.y),
                    w: None,
                    h: None,
                    session_id: None,
                    from: None,
                    from_project: None,
                    from_title: None,
                    created_at: crate::parity::now_ms(cx),
                    extra: serde_json::Map::new(),
                };
                self.state.update(cx, |s, cx| s.save_note(note, cx));
                self.ensure_visible(spot, NOTE_SIZE, cx);
                Task::ready(true)
            }
        }
    }

    /// `startBarSession` (BOARD-133, BOARD-134): a Claude session on the
    /// prompt, expanded and in auto mode, below the selected sessions or in
    /// view; the prompt rides on it as a note, or stays as one if it can't
    /// start.
    fn start_bar_session(&mut self, text: String, cx: &mut Context<'_, Self>) -> Task<bool> {
        let (sources, context) = self.bar_context();
        let dock = self.dock();
        let preferred = if sources.is_empty() {
            self.viewport_centre(NEW_CARD_SIZE)
        } else {
            spot_below(&sources.iter().collect::<Vec<_>>(), dock, NEW_CARD_SIZE)
        };
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        let spot = find_free_spot(preferred, &obstacles, Some(dock), NEW_CARD_SIZE);
        let pos = Point {
            x: js::round(spot.x),
            y: js::round(spot.y),
        };
        let joins = self
            .board
            .as_ref()
            .and_then(|b| section_at(pos, &section_views(&b.sections, &b.cards), None, CARD_SIZE));
        let note = BoardNote {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.clone(),
            color: FolderColor::Yellow,
            x: pos.x,
            y: pos.y,
            w: None,
            h: None,
            session_id: None,
            from: None,
            from_project: None,
            from_title: None,
            created_at: crate::parity::now_ms(cx),
            extra: serde_json::Map::new(),
        };
        let opts = CreateSessionOptions {
            x: pos.x,
            y: pos.y,
            initial_prompt: Some(with_context(&context, &text)),
            parents: (!sources.is_empty()).then(|| sources.iter().map(|c| c.node.session_id.clone()).collect()),
            // Its prompt opens with the context, so it's named for what was asked.
            name: (!context.is_empty()).then(|| truncate(text.split('\n').next().unwrap_or_default(), 64)),
            expanded: Some(true),
            auto_mode: Some(true),
            ..CreateSessionOptions::default()
        };
        let had_context = !context.is_empty();
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created =
                crate::board::on_core(&rt, async move { nodus.sessions().create(&project_id, &opts).await }).await;
            let _ = this.update(cx, |c, cx| match created {
                Ok(card) => {
                    let id = card.node.session_id.clone();
                    c.state.update(cx, |s, cx| {
                        s.upsert_card(card, cx);
                        if let Some(joins) = joins {
                            s.patch_cards(
                                vec![NodePatch {
                                    session_id: id.clone(),
                                    section_id: Some(Some(joins)),
                                    ..NodePatch::default()
                                }],
                                cx,
                            );
                        }
                        s.save_note(
                            BoardNote {
                                session_id: Some(id.clone()),
                                ..note
                            },
                            cx,
                        );
                    });
                    if had_context {
                        c.clear_selection(cx);
                    }
                    c.ensure_visible(pos, NEW_CARD_SIZE, cx);
                }
                Err(err) => {
                    c.state.update(cx, |s, cx| s.save_note(note, cx));
                    c.ensure_visible(pos, NOTE_SIZE, cx);
                    c.show_toast(
                        format!(
                            "Couldn't start a session, so your prompt is on a note: {}",
                            message(&err)
                        ),
                        cx,
                    );
                }
            });
            true
        })
    }

    /// `sendToSession` (BOARD-135).
    fn send_to_session(&mut self, session_id: String, text: String, cx: &mut Context<'_, Self>) -> Task<bool> {
        let card = self.cards().iter().find(|c| c.node.session_id == session_id).cloned();
        let Some(card) = card.filter(|c| !c.node.archived) else {
            self.show_toast("That session is no longer on the board.".to_owned(), cx);
            return Task::ready(false);
        };
        // Return would answer its question, so it's shown for that instead.
        if card.status == SessionStatus::Waiting {
            self.show_toast(
                format!(
                    "“{}” is waiting for your answer. Answer it, then send this.",
                    card_title(&card)
                ),
                cx,
            );
            self.open_card(&session_id, cx);
            return Task::ready(false);
        }
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let sent = crate::board::on_core(&rt, {
                let session_id = session_id.clone();
                async move { nodus.sessions().send(&project_id, &session_id, &text).await }
            })
            .await;
            this.update(cx, |c, cx| match sent {
                Ok(()) => {
                    let at = Point {
                        x: card.node.x,
                        y: card.node.y,
                    };
                    c.ensure_visible(at, card_size(&card), cx);
                    true
                }
                Err(err) => {
                    c.show_toast(format!("Couldn't send it: {}", message(&err)), cx);
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    /// BOARD-143: the selection bar shows with these counts while a card is
    /// selected and the merge dialog is closed (`None` otherwise). Its
    /// buttons call [`Self::start_merge`], `group_cards`, `archive_cards`
    /// and `clear_selection` with [`Self::selected_cards`].
    pub fn selection_bar(&self) -> Option<SelectionCounts> {
        let selected = self.selected_cards();
        if selected.is_empty() || self.merge.is_some() {
            return None;
        }
        let cards: Vec<&nodus_domain::SessionCard> = self
            .cards()
            .iter()
            .filter(|c| selected.contains(&c.node.session_id))
            .collect();
        Some(SelectionCounts {
            count: cards.len(),
            session_count: cards.iter().filter(|c| !is_terminal(c)).count(),
            active_count: cards.iter().filter(|c| !c.node.archived && !is_terminal(c)).count(),
            group_count: cards.iter().filter(|c| !c.node.archived).count(),
        })
    }

    // ---- the merge dialog ------------------------------------------------------------

    /// `startMerge` (BOARD-109): the non-terminal cards among `ids`, and the
    /// focus a search gave; none → nothing. The merge dialog (spec 11) opens
    /// on [`Self::merge`].
    pub fn start_merge(&mut self, ids: &[String], focus: Option<&str>, cx: &mut Context<'_, Self>) {
        let sources: Vec<MergeSource> = ids
            .iter()
            .filter_map(|id| self.cards().iter().find(|c| &c.node.session_id == id))
            .filter(|c| !is_terminal(c))
            .map(|c| MergeSource {
                session_id: c.node.session_id.clone(),
                title: card_title(c),
            })
            .collect();
        if sources.is_empty() {
            return;
        }
        self.merge_focus = focus.map(js::trim).filter(|f| !f.is_empty()).map(str::to_owned);
        self.merge = Some(sources);
        cx.notify();
    }

    /// The merge dialog closed without starting anything.
    pub fn close_merge(&mut self, cx: &mut Context<'_, Self>) {
        self.merge = None;
        self.merge_focus = None;
        cx.notify();
    }

    /// `onMergeStart` (BOARD-108): the new session goes below its sources
    /// (or where a new session would), clear of everything; then the dialog
    /// closes, the selection clears and its window opens. Errors go back to
    /// the dialog.
    pub fn merge_start(
        &mut self,
        start: MergeStart,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<(), nodus_core::CommandError>> {
        let dock = self.dock();
        let all = self.cards().to_vec();
        let sources: Vec<&nodus_domain::SessionCard> = all
            .iter()
            .filter(|c| self.merge.iter().flatten().any(|s| s.session_id == c.node.session_id))
            .collect();
        let others = self.notes_in_the_way();
        let preferred = if sources.is_empty() {
            spot_for_new_session(dock, &all, NEW_CARD_SIZE, &others)
        } else {
            spot_below(&sources, dock, NEW_CARD_SIZE)
        };
        let mut obstacles = card_obstacles(&all, &[]);
        obstacles.extend(others);
        let spot = find_free_spot(preferred, &obstacles, Some(dock), NEW_CARD_SIZE);
        let pos = Point {
            x: js::round(spot.x),
            y: js::round(spot.y),
        };
        let opts = CreateSessionOptions {
            x: pos.x,
            y: pos.y,
            name: Some(start.name),
            initial_prompt: Some(start.initial_prompt),
            parents: Some(start.parents),
            ..CreateSessionOptions::default()
        };
        let nodus = self.state.read(cx).nodus.clone();
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let card =
                crate::board::on_core(&rt, async move { nodus.sessions().create(&project_id, &opts).await }).await?;
            let _ = this.update(cx, |c, cx| {
                let id = card.node.session_id.clone();
                c.state.update(cx, |s, cx| s.upsert_card(card, cx));
                c.close_merge(cx);
                c.clear_selection(cx);
                c.ensure_visible(pos, NEW_CARD_SIZE, cx);
                c.state.update(cx, |s, cx| s.open_terminal(&id, cx));
            });
            Ok(())
        })
    }

    // ---- ⌘K search and the New view dialog ---------------------------------------------

    /// Opens ⌘K search (BOARD-157, BOARD-171; spec 11 draws it).
    pub fn open_search(&mut self, cx: &mut Context<'_, Self>) {
        self.search_open = true;
        cx.notify();
    }

    pub fn close_search(&mut self, cx: &mut Context<'_, Self>) {
        self.search_open = false;
        cx.notify();
    }

    /// Search's query, kept between openings (BOARD-145).
    pub fn set_search_query(&mut self, query: String, cx: &mut Context<'_, Self>) {
        self.search_query = query;
        cx.notify();
    }

    /// Search's `onStart` (BOARD-145): a merge of those sessions, focused
    /// on the query.
    pub fn search_start(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        self.search_open = false;
        let query = self.search_query.clone();
        self.start_merge(ids, Some(&query), cx);
    }

    /// The New view dialog (BOARD-144; spec 04 draws it): centred on a flow
    /// point (the pane menu) or in view (the title bar).
    pub fn open_view_dialog(&mut self, at: Option<Point>, cx: &mut Context<'_, Self>) {
        self.view_dialog = Some(at);
        cx.notify();
    }

    pub fn close_view_dialog(&mut self, cx: &mut Context<'_, Self>) {
        self.view_dialog = None;
        cx.notify();
    }
}
