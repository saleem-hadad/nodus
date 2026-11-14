//! The board's side of the overlays (spec 09 § Prompt bar wiring and
//! § Overlays; spec 11 draws them): what the prompt bar sends into or
//! starts, the merge dialog's sources and its new session, ⌘K search's open
//! state and query, and the New view dialog's point. The overlays read this
//! state from the canvas and call these commands.

use gpui::{Context, Task};
use nodus_board::compose::{ContextItem, with_context};
use nodus_board::geometry::Point;
use nodus_board::ink_draw::{fallback_width, stroke_box};
use nodus_domain::{
    BoardNote, BubbleRoute, CreateSessionOptions, FolderColor, Rect, SessionEffort, SessionModel, SessionStatus,
    ShapeKind, js,
};

use super::canvas::BoardCanvas;
use super::layout::{
    NEW_CARD_SIZE, NOTE_SIZE, card_obstacles, card_size, card_title, find_free_spot, is_terminal, spot_below,
    spot_for_new_session, truncate,
};

/// How much of the board around a bubble's spot a session is pointed at
/// (BUBBLE-033), in board units.
const SPOT_AREA: (f64, f64) = (1400.0, 1000.0);

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

/// The selection bar's counts (spec 09 § Selection bar wiring, spec 17):
/// the things selected, the Claude sessions among them, the active ones,
/// and whether they're one group exactly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionCounts {
    pub count: usize,
    pub session_count: usize,
    pub active_count: usize,
    /// GROUP-021: the selection is one group: its button takes it apart.
    pub one_group: bool,
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

    /// BOARD-132: what a new session from the bar would read first, each
    /// shape, picture and ink stroke counted.
    pub fn bar_context_count(&self) -> usize {
        self.bar_context().1.iter().map(ContextItem::count).sum()
    }

    /// BAR-042: the shapes, pictures and ink stroke selected, as the area
    /// they take, for a session to look at (TOOL-111).
    fn selected_area(&self) -> Option<ContextItem> {
        let shapes: Vec<&nodus_domain::BoardShape> = {
            let ids = self.selected_shapes();
            self.shapes().iter().filter(|s| ids.contains(&s.id)).collect()
        };
        let stroke = self
            .overlays
            .ink
            .selected
            .as_ref()
            .and_then(|id| self.ink_strokes().into_iter().find(|s| &s.id == id));
        let mut rects: Vec<Rect> = shapes.iter().map(|s| s.rect()).collect();
        if let Some(stroke) = &stroke {
            let b = stroke_box(stroke, &fallback_width);
            rects.push(Rect {
                x: b.x,
                y: b.y,
                w: b.w,
                h: b.h,
            });
        }
        let first = *rects.first()?;
        let (x, y) = rects
            .iter()
            .fold((first.x, first.y), |(x, y), r| (x.min(r.x), y.min(r.y)));
        let right = rects.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
        let bottom = rects.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
        let pictures = shapes.iter().filter(|s| s.kind == ShapeKind::Image).count();
        let counts = [
            (pictures, "picture", "pictures"),
            (shapes.len() - pictures, "shape", "shapes"),
            (usize::from(stroke.is_some()), "ink mark", "ink marks"),
        ];
        let parts: Vec<String> = counts
            .iter()
            .filter(|(n, ..)| *n > 0)
            .map(|(n, one, many)| format!("{n} {}", if *n == 1 { one } else { many }))
            .collect();
        let what = match parts.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => return None,
        };
        // A little room around them, so what they mark shows too.
        let pad = 40.0;
        Some(ContextItem::Area {
            what,
            count: shapes.len() + usize::from(stroke.is_some()),
            x: x - pad,
            y: y - pad,
            w: right - x + pad * 2.0,
            h: bottom - y + pad * 2.0,
        })
    }

    /// BUBBLE-033: where a bubble asked, when something on the board is
    /// near enough that it may be what the user pointed at.
    fn bubble_spot(&self, at: Point) -> Option<ContextItem> {
        let area = Rect {
            x: at.x - SPOT_AREA.0 / 2.0,
            y: at.y - SPOT_AREA.1 / 2.0,
            w: SPOT_AREA.0,
            h: SPOT_AREA.1,
        };
        let meets =
            |r: Rect| r.x < area.x + area.w && area.x < r.x + r.w && r.y < area.y + area.h && area.y < r.y + r.h;
        let near = self.nodes.iter().any(|n| !n.hidden && meets(n.rect()))
            || self.ink_strokes().iter().any(|s| {
                let b = stroke_box(s, &fallback_width);
                meets(Rect {
                    x: b.x,
                    y: b.y,
                    w: b.w,
                    h: b.h,
                })
            });
        near.then_some(ContextItem::Spot {
            x: at.x,
            y: at.y,
            area: [area.x, area.y, area.w, area.h],
        })
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
        items.extend(self.selected_area());
        (sources, items)
    }

    /// `submitFromBar`: true when the bar should clear (BOARD-133…BOARD-137).
    pub fn submit_from_bar(&mut self, submission: Submission, cx: &mut Context<'_, Self>) -> Task<bool> {
        match submission {
            // MODEL-022: on the model and effort the bar's chip names.
            Submission::Session { text } => {
                let choice = self.bar_model_choice(cx);
                self.start_bar_session(&text, None, choice.model, choice.effort, cx)
            }
            // BAR-042: shapes, pictures and ink selected with it go along.
            Submission::Into { session_id, text } => {
                let text = with_context(&self.selected_area().into_iter().collect::<Vec<_>>(), &text);
                self.send_to_session(session_id, text, cx)
            }
            // BOARD-136.
            Submission::View { text } => {
                let made = self.create_view(text, None, cx);
                self.toast_view_error(made, cx)
            }
            Submission::Note { text, color } => {
                // BOARD-137.
                let spot = self.free_note_spot(self.viewport_centre(NOTE_SIZE));
                self.put_note(text, color, spot, true, cx);
                Task::ready(true)
            }
        }
    }

    /// BUBBLE-030…BUBBLE-032: what the bubble made of its text, its top
    /// left on the spot it was asked at, the board left where it is: a
    /// session as the bar starts one, on the model Claude picked (Claude
    /// Code's own with none), a yellow note, or a view.
    pub fn submit_from_bubble(
        &mut self,
        text: String,
        route: Option<BubbleRoute>,
        at: Point,
        cx: &mut Context<'_, Self>,
    ) -> Task<bool> {
        match route {
            Some(BubbleRoute::Session(model)) => self.start_bar_session(&text, Some(at), Some(model), None, cx),
            None => self.start_bar_session(&text, Some(at), None, None, cx),
            Some(BubbleRoute::Note) => {
                self.put_note(text, FolderColor::Yellow, at, false, cx);
                Task::ready(true)
            }
            Some(BubbleRoute::View) => {
                let made = self.create_view_at(text, at, cx);
                self.toast_view_error(made, cx)
            }
        }
    }

    /// True once the view is made; a toast says why when it can't be.
    fn toast_view_error(
        &mut self,
        made: Task<Result<(), nodus_core::CommandError>>,
        cx: &mut Context<'_, Self>,
    ) -> Task<bool> {
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

    /// A sticky note of the text at `spot`, brought into view when `follow`.
    fn put_note(&mut self, text: String, color: FolderColor, spot: Point, follow: bool, cx: &mut Context<'_, Self>) {
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
        if follow {
            self.ensure_visible(spot, NOTE_SIZE, cx);
        }
    }

    /// `startBarSession` (BOARD-133, BOARD-134): a Claude session on the
    /// prompt, expanded and in auto mode, below the selected sessions or in
    /// view; the prompt rides on it as a note, or stays as one if it can't
    /// start. With `at` (the bubble's spot), its top left is there and the
    /// board stays where it is; `model` and `effort` are those it runs on.
    fn start_bar_session(
        &mut self,
        text: &str,
        at: Option<Point>,
        model: Option<SessionModel>,
        effort: Option<SessionEffort>,
        cx: &mut Context<'_, Self>,
    ) -> Task<bool> {
        let (sources, mut context) = self.bar_context();
        let had_context = !context.is_empty();
        context.extend(at.and_then(|at| self.bubble_spot(at)));
        let follow = at.is_none();
        let spot = at.unwrap_or_else(|| {
            let dock = self.dock();
            let preferred = if sources.is_empty() {
                self.viewport_centre(NEW_CARD_SIZE)
            } else {
                spot_below(&sources.iter().collect::<Vec<_>>(), dock, NEW_CARD_SIZE)
            };
            let mut obstacles = card_obstacles(self.cards(), &[]);
            obstacles.extend(self.notes_in_the_way());
            find_free_spot(preferred, &obstacles, Some(dock), NEW_CARD_SIZE)
        });
        let pos = Point {
            x: js::round(spot.x),
            y: js::round(spot.y),
        };
        let note = BoardNote {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.to_owned(),
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
            initial_prompt: Some(with_context(&context, text)),
            parents: (!sources.is_empty()).then(|| sources.iter().map(|c| c.node.session_id.clone()).collect()),
            // Its prompt opens with the context, so it's named for what was asked.
            name: (!context.is_empty()).then(|| truncate(text.split('\n').next().unwrap_or_default(), 64)),
            expanded: Some(true),
            auto_mode: Some(true),
            model: model.map(|m| m.as_str().to_owned()),
            effort: effort.map(|e| e.as_str().to_owned()),
            ..CreateSessionOptions::default()
        };
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
                    if follow {
                        c.ensure_visible(pos, NEW_CARD_SIZE, cx);
                    }
                }
                Err(err) => {
                    c.state.update(cx, |s, cx| s.save_note(note, cx));
                    if follow {
                        c.ensure_visible(pos, NOTE_SIZE, cx);
                    }
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

    /// BOARD-143, GROUP-021: the selection bar shows with these counts
    /// while a card, or two or more things, are selected and the merge
    /// dialog is closed (`None` otherwise). Its buttons call
    /// [`Self::start_merge`] and `archive_cards` with
    /// [`Self::selected_cards`], `group_things` (or `ungroup_things`) and
    /// `frame_things` with `selected_things`, and `clear_selection`.
    pub fn selection_bar(&self) -> Option<SelectionCounts> {
        let selected = self.selected_cards();
        let things = self.selected_things();
        if (selected.is_empty() && things.len() < 2) || self.merge.is_some() {
            return None;
        }
        let cards: Vec<&nodus_domain::SessionCard> = self
            .cards()
            .iter()
            .filter(|c| selected.contains(&c.node.session_id))
            .collect();
        let archived = cards.iter().filter(|c| c.node.archived).count();
        Some(SelectionCounts {
            count: things.len() + archived,
            session_count: cards.iter().filter(|c| !is_terminal(c)).count(),
            active_count: cards.iter().filter(|c| !c.node.archived && !is_terminal(c)).count(),
            one_group: self.exactly_one_group(&things).is_some(),
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
