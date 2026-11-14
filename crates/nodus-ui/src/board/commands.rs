//! What the board does (BoardView.tsx's callbacks): opening, expanding,
//! creating, archiving, stopping, restoring and removing cards; sections;
//! notes; views; selection; and what follows runs and notifications
//! (BOARD-032…BOARD-154). Each reads the board as it is now and goes
//! through the store and the core, with 0.9.4's placement and strings.

use std::time::Duration;

use gpui::Context;
use nodus_board::geometry::{CARD_SIZE, NOTE_H, NOTE_W, Point};
use nodus_domain::{
    ActionRunResult, BoardNote, BoardSection, CreateSessionOptions, FolderColor, NodePatch, SessionCard, SessionKind,
    SessionStatus, ViewPatch, js,
};

use super::canvas::{BoardCanvas, NAME_LANDING_MS, TITLE_CLICK_MS};
use super::layout::{
    self, NEW_CARD_SIZE, NOTE_SIZE, VIEW_SIZE, card_obstacles, card_size, card_title, contains_point, find_free_spot,
    find_spot_beside, is_expanded, is_terminal, layout_frame, new_section_corner, note_obstacles, pad_sheet_spot,
    section_at, section_views, spot_for_new_session, spot_in_section, spot_right_of_archive, stack_top, view_obstacles,
};
use crate::terminal::actions::PreviewFind;

/// A view can start a session at most this often.
const VIEW_START_GAP_MS: f64 = 3000.0;
/// How far from its view a session the view started goes.
const VIEW_SESSION_GAP: f64 = 3.0 * nodus_board::geometry::GAP;

/// `err.message` as 0.9.4 showed it: Electron's IPC prefix included.
fn message(err: &nodus_core::CommandError) -> String {
    err.ipc_message()
}

fn round(p: Point) -> Point {
    Point {
        x: js::round(p.x),
        y: js::round(p.y),
    }
}

impl BoardCanvas {
    pub(crate) fn cards(&self) -> &[SessionCard] {
        self.board.as_ref().map_or(&[], |b| b.cards.as_slice())
    }

    fn card(&self, id: &str) -> Option<&SessionCard> {
        self.cards().iter().find(|c| c.node.session_id == id)
    }

    fn nodus(&self, cx: &gpui::App) -> nodus_core::Nodus {
        self.state.read(cx).nodus.clone()
    }

    /// Notes sit on top of cards, and views among them, so new cards (and
    /// notes) keep clear of both (`notesInTheWay`).
    pub(crate) fn notes_in_the_way(&self) -> Vec<nodus_domain::Rect> {
        let Some(board) = &self.board else { return Vec::new() };
        let mut out = note_obstacles(&board.notes);
        out.extend(view_obstacles(&board.views));
        out
    }

    /// The frames, with their boxes.
    pub(crate) fn groups(&self) -> Vec<layout::SectionView> {
        self.board
            .as_ref()
            .map(|b| section_views(&b.sections, &b.cards))
            .unwrap_or_default()
    }

    // ---- selection --------------------------------------------------------------

    /// BOARD-153.
    pub fn clear_selection(&mut self, cx: &mut Context<'_, Self>) {
        for n in &mut self.nodes {
            n.selected = false;
        }
        self.selection_rect = false;
        self.sync_selection(cx);
        cx.notify();
    }

    /// BOARD-152.
    pub fn select_all_active(&mut self, cx: &mut Context<'_, Self>) {
        let archived: Vec<bool> = self
            .nodes
            .iter()
            .map(|n| n.card_index().is_some_and(|i| self.cards()[i].node.archived))
            .collect();
        for (n, archived) in self.nodes.iter_mut().zip(archived) {
            if n.is_card() {
                n.selected = !archived;
            }
        }
        self.sync_selection(cx);
        cx.notify();
    }

    /// BOARD-154, and a search jump: exactly these cards.
    pub fn select_cards(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        for n in &mut self.nodes {
            if n.is_card() {
                n.selected = ids.contains(&n.id);
            }
        }
        self.sync_selection(cx);
        cx.notify();
    }

    /// FRAME-026: exactly these things, of every kind, selected.
    pub fn select_things(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        for n in &mut self.nodes {
            n.selected = n.selectable && !n.hidden && ids.contains(&n.id);
        }
        self.selection_rect = false;
        self.sync_selection(cx);
        cx.notify();
    }

    // ---- opening and expanding ------------------------------------------------------

    /// BOARD-044.
    pub fn open_card(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(session_id) else { return };
        let (archived, expanded) = (card.node.archived, is_expanded(card));
        let id = session_id.to_owned();
        if archived {
            self.state.update(cx, |s, cx| s.open_preview(&id, cx));
        } else if expanded {
            // An expanded card shows its session right there.
            let renderers = super::nodes::renderers(cx);
            renderers.wake(&id, cx);
        } else {
            self.state.update(cx, |s, cx| s.open_terminal(&id, cx));
        }
    }

    /// BOARD-150.
    pub fn expand_cards(&mut self, ids: &[String], expanded: bool, cx: &mut Context<'_, Self>) {
        let targets: Vec<SessionCard> = self
            .cards()
            .iter()
            .filter(|c| ids.contains(&c.node.session_id) && !c.node.archived && is_expanded(c) != expanded)
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        // Expanding shows the session there, as opening it would in the modal.
        if expanded {
            let renderers = super::nodes::renderers(cx);
            for c in &targets {
                renderers.wake(&c.node.session_id, cx);
            }
        }
        self.patch_cards(
            targets
                .iter()
                .map(|c| NodePatch {
                    session_id: c.node.session_id.clone(),
                    expanded: Some(expanded),
                    ..NodePatch::default()
                })
                .collect(),
            cx,
        );
        if expanded && targets.len() == 1 {
            let mut card = targets[0].node.clone();
            card.expanded = Some(true);
            let size = nodus_board::geometry::card_size(Some(&card));
            self.ensure_visible(Point { x: card.x, y: card.y }, size, cx);
        }
    }

    /// BOARD-103: closing a session's window leaves its card expanded.
    pub(crate) fn expand_after_modal(&mut self, closed: &str, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(closed).cloned() else { return };
        if card.node.archived || is_expanded(&card) {
            return;
        }
        let id = closed.to_owned();
        let state = self.state.clone();
        cx.defer(move |cx| {
            state.update(cx, |s, cx| {
                s.patch_cards(
                    vec![NodePatch {
                        session_id: id,
                        expanded: Some(true),
                        ..NodePatch::default()
                    }],
                    cx,
                );
            });
        });
        let mut node = card.node.clone();
        node.expanded = Some(true);
        let size = nodus_board::geometry::card_size(Some(&node));
        self.ensure_visible(Point { x: node.x, y: node.y }, size, cx);
    }

    /// The click on a card's title waits for a double-click (BOARD-043).
    pub(crate) fn open_after_title_click(&mut self, id: String, cx: &mut Context<'_, Self>) {
        self.title_click = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(TITLE_CLICK_MS))
                .await;
            let _ = this.update(cx, |c, cx| c.open_card(&id, cx));
        }));
    }

    // ---- creating sessions ------------------------------------------------------------

    /// `createSession` (BOARD-099…BOARD-102).
    pub fn create_session(&mut self, at: Option<Point>, kind: SessionKind, cx: &mut Context<'_, Self>) {
        if self.board.is_none() {
            return;
        }
        let dock = self.dock();
        let others = self.notes_in_the_way();
        let spot = match at {
            Some(at) => {
                let mut obstacles = card_obstacles(self.cards(), &[]);
                obstacles.extend(others);
                find_free_spot(at, &obstacles, Some(dock), NEW_CARD_SIZE)
            }
            None => spot_for_new_session(dock, self.cards(), NEW_CARD_SIZE, &others),
        };
        let pos = round(spot);
        // BOARD-101.
        let opts = CreateSessionOptions {
            x: pos.x,
            y: pos.y,
            kind: Some(kind),
            auto_mode: Some(true),
            ..CreateSessionOptions::default()
        };
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created = crate::board::on_core(&rt, {
                let (nodus, project_id, opts) = (nodus.clone(), project_id.clone(), opts.clone());
                async move { nodus.sessions().create(&project_id, &opts).await }
            })
            .await;
            let _ = this.update(cx, |c, cx| match created {
                Ok(card) => {
                    let id = card.node.session_id.clone();
                    c.state.update(cx, |s, cx| {
                        s.upsert_card(card, cx);
                    });
                    c.ensure_visible(pos, NEW_CARD_SIZE, cx);
                    c.state.update(cx, |s, cx| s.open_terminal(&id, cx));
                }
                // BOARD-102.
                Err(err) => {
                    let what = if kind == SessionKind::Terminal {
                        "a terminal"
                    } else {
                        "a session"
                    };
                    c.show_toast(format!("Couldn't start {what}: {}", message(&err)), cx);
                }
            });
        })
        .detach();
    }

    /// A session centred on a flow point (BOARD-048, BOARD-176).
    pub fn create_session_centred(&mut self, p: Point, kind: SessionKind, cx: &mut Context<'_, Self>) {
        let at = Point {
            x: p.x - CARD_SIZE.w / 2.0,
            y: p.y - CARD_SIZE.h / 2.0,
        };
        self.create_session(Some(at), kind, cx);
    }

    // ---- archiving, stopping, restoring ------------------------------------------------

    /// `confirmStopRunning` (archive-running confirm).
    fn confirm_archive_running(
        &mut self,
        running: &[SessionCard],
        cx: &mut Context<'_, Self>,
    ) -> tokio::sync::oneshot::Receiver<bool> {
        let (title, message) = if running.len() == 1 {
            let card = &running[0];
            let doing = if card.status == SessionStatus::Waiting {
                "waiting for you"
            } else {
                "working"
            };
            (
                "Archive a running session?".to_owned(),
                format!(
                    "“{}” is still {doing}. The Claude process will be stopped.",
                    card_title(card)
                ),
            )
        } else {
            (
                format!("Archive {} running sessions?", running.len()),
                "Some of these sessions are still working. Their Claude processes will be stopped.".to_owned(),
            )
        };
        self.confirm(title, message, "Stop & Archive", true, cx)
    }

    /// BOARD-110.
    pub fn archive_cards(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let targets: Vec<SessionCard> = self
            .cards()
            .iter()
            .filter(|c| ids.contains(&c.node.session_id) && !c.node.archived && !is_terminal(c))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let running: Vec<SessionCard> = targets
            .iter()
            .filter(|c| c.live && c.status != SessionStatus::Done)
            .cloned()
            .collect();
        let ask = (!running.is_empty()).then(|| self.confirm_archive_running(&running, cx));
        cx.spawn(async move |this, cx| {
            if let Some(ask) = ask
                && !ask.await.unwrap_or(false)
            {
                return;
            }
            let _ = this.update(cx, |c, cx| {
                let Some(board) = &c.board else { return };
                let top = stack_top(board.archive);
                let moving: Vec<String> = targets.iter().map(|t| t.node.session_id.clone()).collect();
                c.patch_cards(
                    moving
                        .iter()
                        .map(|id| NodePatch {
                            session_id: id.clone(),
                            x: Some(top.x),
                            y: Some(top.y),
                            archived: Some(true),
                            ..NodePatch::default()
                        })
                        .collect(),
                    cx,
                );
                for n in &mut c.nodes {
                    if moving.contains(&n.id) {
                        n.selected = false;
                    }
                }
                c.sync_selection(cx);
            });
        })
        .detach();
    }

    /// BOARD-112.
    pub fn stop_cards(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let picked: Vec<SessionCard> = self
            .cards()
            .iter()
            .filter(|c| ids.contains(&c.node.session_id))
            .cloned()
            .collect();
        let targets: Vec<SessionCard> = nodus_board::stoppable(&picked).into_iter().cloned().collect();
        if targets.is_empty() {
            return;
        }
        let ask = nodus_board::stop_confirmation(&targets, card_title)
            .map(|q| self.confirm(q.title, q.message, &q.confirm_label, q.danger, cx));
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        cx.spawn(async move |_, _| {
            if let Some(ask) = ask
                && !ask.await.unwrap_or(false)
            {
                return;
            }
            let ids: Vec<String> = targets.iter().map(|c| c.node.session_id.clone()).collect();
            crate::board::on_core(&rt, {
                let (nodus, ids) = (nodus.clone(), ids.clone());
                async move { nodus.sessions().stop(&ids).await }
            })
            .await;
        })
        .detach();
    }

    /// BOARD-113.
    pub fn restore_card(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let spot = spot_right_of_archive(
            self.dock(),
            self.cards(),
            &[session_id],
            CARD_SIZE,
            &self.notes_in_the_way(),
        );
        self.patch_cards(
            vec![NodePatch {
                session_id: session_id.to_owned(),
                x: Some(js::round(spot.x)),
                y: Some(js::round(spot.y)),
                archived: Some(false),
                ..NodePatch::default()
            }],
            cx,
        );
        self.ensure_visible(spot, CARD_SIZE, cx);
    }

    /// BOARD-114: a live shell asks first.
    pub fn remove_terminal(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(session_id).cloned() else {
            return;
        };
        let ask = card.live.then(|| {
            self.confirm(
                "Remove terminal?".into(),
                format!(
                    "“{}” will be removed from this board. Its shell and anything running in it will be stopped.",
                    card_title(&card)
                ),
                "Stop & Remove",
                true,
                cx,
            )
        });
        let id = session_id.to_owned();
        cx.spawn(async move |this, cx| {
            if let Some(ask) = ask
                && !ask.await.unwrap_or(false)
            {
                return;
            }
            let _ = this.update(cx, |c, cx| {
                c.state.update(cx, |s, cx| s.remove_card(&id, cx).detach());
            });
        })
        .detach();
    }

    /// BOARD-115: always asks; a live session is stopped first.
    pub fn remove_from_board(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(session_id).cloned() else {
            return;
        };
        let ask = self.confirm(
            "Remove from board?".into(),
            format!(
                "“{}” will be removed from this board{}. The transcript stays on disk.",
                card_title(&card),
                if card.live { " and its process stopped" } else { "" }
            ),
            "Remove",
            true,
            cx,
        );
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let id = session_id.to_owned();
        cx.spawn(async move |this, cx| {
            if !ask.await.unwrap_or(false) {
                return;
            }
            if card.live {
                crate::board::on_core(&rt, {
                    let (nodus, id) = (nodus.clone(), id.clone());
                    async move { nodus.sessions().kill(&id).await }
                })
                .await;
            }
            let _ = this.update(cx, |c, cx| {
                c.state.update(cx, |s, cx| s.remove_card(&id, cx).detach());
            });
        })
        .detach();
    }

    // ---- frames ----------------------------------------------------------------------

    /// `addSection` (BOARD-085, BOARD-086): a new frame at `frame`.
    fn add_section(&mut self, frame: nodus_domain::Rect, rename: bool, cx: &mut Context<'_, Self>) -> BoardSection {
        let used: Vec<FolderColor> = self
            .board
            .as_ref()
            .map(|b| b.sections.iter().map(|s| s.color).collect())
            .unwrap_or_default();
        let distinct_count = FolderColor::ALL.iter().filter(|c| used.contains(c)).count();
        let color = FolderColor::ALL
            .iter()
            .copied()
            .find(|c| !used.contains(c))
            .unwrap_or(FolderColor::ALL[distinct_count % FolderColor::ALL.len()]);
        let section = BoardSection {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            color,
            x: js::round(frame.x),
            y: js::round(frame.y),
            w: Some(js::round(frame.w)),
            h: Some(js::round(frame.h)),
            created_at: crate::parity::now_ms(cx),
            extra: serde_json::Map::new(),
        };
        let saved = section.clone();
        self.state.update(cx, |s, cx| s.save_section(saved, cx));
        if rename {
            self.set_renaming(Some(section.id.clone()), cx);
        }
        section
    }

    /// `nameSection` (BOARD-089, BOARD-090, FRAME-030): Claude names a new
    /// frame after the things in it, `ids`.
    fn name_section(&mut self, section_id: String, ids: Vec<String>, cx: &mut Context<'_, Self>) {
        self.naming.insert(section_id.clone(), None);
        cx.notify();
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let named = crate::board::on_core(&rt, {
                let (nodus, project_id, ids) = (nodus.clone(), project_id.clone(), ids.clone());
                async move { nodus.board().name_section(&project_id, &ids).await }
            })
            .await;
            let name = match named {
                Ok(name) => Some(name),
                Err(err) => {
                    tracing::warn!("[frames] naming failed: {}", err.ipc_message());
                    None
                }
            };
            let _ = this.update(cx, |c, cx| {
                // Read the store, not this board: the user may have left it.
                let section = {
                    let s = c.state.read(cx);
                    s.board
                        .as_ref()
                        .filter(|b| b.project.id == project_id)
                        .and_then(|b| b.sections.iter().find(|s| s.id == section_id).cloned())
                };
                let mut landed = String::new();
                if let (Some(name), Some(section)) = (name.filter(|n| !n.is_empty()), section)
                    && section.name.is_empty()
                    && c.renaming.as_deref() != Some(section_id.as_str())
                {
                    landed.clone_from(&name);
                    c.state
                        .update(cx, |s, cx| s.save_section(BoardSection { name, ..section }, cx));
                }
                c.naming.insert(section_id.clone(), Some(landed));
                cx.notify();
            });
            cx.background_executor()
                .timer(Duration::from_millis(NAME_LANDING_MS))
                .await;
            let _ = this.update(cx, |c, cx| {
                c.naming.remove(&section_id);
                cx.notify();
            });
        })
        .detach();
    }

    /// FRAME-020…FRAME-022: a frame round these things (and whatever is
    /// glued to them), tidied first when it would take in others; Claude
    /// names it when something in it says what it's about.
    pub fn frame_things(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.clone() else { return };
        let ids = self.with_groups(ids);
        let items: Vec<(String, nodus_domain::Rect)> = ids
            .iter()
            .filter_map(|id| self.node(id).filter(|n| super::frames::groupable(n)))
            .map(|n| (n.id.clone(), n.rect()))
            .collect();
        if items.is_empty() {
            return;
        }
        let others: Vec<nodus_domain::Rect> = self
            .nodes
            .iter()
            .filter(|n| super::frames::groupable(n) && !ids.contains(&n.id))
            .map(super::scene::FlowNode::rect)
            .collect();
        let mut zones = vec![self.dock()];
        zones.extend(self.groups().into_iter().map(|g| g.frame));
        let (frame, positions) = layout_frame(&items, &others, &zones);
        let nameable: Vec<String> = items
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| super::frames::says_something(&board, id))
            .collect();
        let section = self.add_section(frame, nameable.is_empty(), cx);
        self.move_things(&positions, cx);
        if !nameable.is_empty() {
            self.name_section(section.id, nameable, cx);
        }
        self.clear_selection(cx);
        // 0.9.4 passes the frame as the corner, with the default card size.
        self.ensure_visible(Point { x: frame.x, y: frame.y }, CARD_SIZE, cx);
    }

    /// `createSectionAt` (BOARD-092, FRAME-005): an empty frame one session
    /// fits in, by the click or at the nearest free spot, shown.
    pub fn create_section_at(&mut self, p: Point, cx: &mut Context<'_, Self>) {
        let mut obstacles: Vec<nodus_domain::Rect> = self
            .nodes
            .iter()
            .filter(|n| super::frames::groupable(n))
            .map(super::scene::FlowNode::rect)
            .collect();
        obstacles.push(self.dock());
        obstacles.extend(self.groups().into_iter().map(|g| g.frame));
        let size = super::frames::NEW_FRAME_SIZE;
        let corner = new_section_corner(p, &obstacles, size);
        self.add_section(
            nodus_domain::Rect {
                x: corner.x,
                y: corner.y,
                w: size.w,
                h: size.h,
            },
            true,
            cx,
        );
        self.ensure_visible(corner, size, cx);
    }

    /// `leaveSection` (BOARD-094): a card moved out of the frame it's in,
    /// to a free spot right of it.
    pub fn leave_section(&mut self, session_id: &str, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(session_id).cloned() else {
            return;
        };
        let groups = self.groups();
        let Some(here) = section_at(
            Point {
                x: card.node.x,
                y: card.node.y,
            },
            &groups,
            card_size(&card),
        ) else {
            return;
        };
        let Some(group) = groups.iter().find(|g| g.section.id == here) else {
            return;
        };
        let size = card_size(&card);
        let mut obstacles = card_obstacles(self.cards().iter().filter(|c| !c.node.archived), &[session_id]);
        obstacles.extend(self.notes_in_the_way());
        obstacles.push(self.dock());
        obstacles.extend(
            groups
                .iter()
                .filter(|g| g.section.id != group.section.id)
                .map(|g| g.frame),
        );
        let spot = find_free_spot(
            Point {
                x: group.frame.x + group.frame.w + nodus_board::geometry::GAP,
                y: card.node.y,
            },
            &obstacles,
            Some(group.frame),
            size,
        );
        self.patch_cards(
            vec![NodePatch {
                session_id: session_id.to_owned(),
                x: Some(js::round(spot.x)),
                y: Some(js::round(spot.y)),
                ..NodePatch::default()
            }],
            cx,
        );
        self.ensure_visible(spot, size, cx);
    }

    /// `New session in frame` / `New terminal in frame` (BOARD-097): at a
    /// free slot inside it. A frame too small for the new card grows to
    /// hold it (FRAME-024).
    pub fn create_in_section(&mut self, section_id: &str, kind: SessionKind, cx: &mut Context<'_, Self>) {
        let Some(group) = self.groups().into_iter().find(|g| g.section.id == section_id) else {
            return;
        };
        // The obstacles a new session's spot keeps clear of (BOARD-099).
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        let spot = spot_in_section(group.frame, &obstacles, Some(self.dock()), NEW_CARD_SIZE);
        let f = group.frame;
        let w = f.w.max(spot.x - f.x + NEW_CARD_SIZE.w + layout::SECTION_PAD);
        let h = f.h.max(spot.y - f.y + NEW_CARD_SIZE.h + layout::SECTION_PAD);
        if w > f.w || h > f.h {
            let section = BoardSection {
                x: f.x,
                y: f.y,
                w: Some(js::round(w)),
                h: Some(js::round(h)),
                ..group.section.clone()
            };
            self.state.update(cx, |s, cx| s.save_section(section, cx));
        }
        self.create_session(Some(spot), kind, cx);
    }

    /// BOARD-096.
    pub fn recolor_section(&mut self, section_id: &str, color: FolderColor, cx: &mut Context<'_, Self>) {
        let Some(section) = self
            .board
            .as_ref()
            .and_then(|b| b.sections.iter().find(|s| s.id == section_id).cloned())
        else {
            return;
        };
        self.state
            .update(cx, |s, cx| s.save_section(BoardSection { color, ..section }, cx));
    }

    /// BOARD-095, FRAME-025: the frame goes; what was in it stays.
    pub fn remove_section(&mut self, section_id: &str, cx: &mut Context<'_, Self>) {
        let id = section_id.to_owned();
        self.state.update(cx, |s, cx| s.remove_section(&id, cx));
    }

    // ---- notes ---------------------------------------------------------------------

    /// `addNote` (BOARD-116).
    fn add_note(&mut self, p: Point, id: String, cx: &mut Context<'_, Self>) {
        let note = BoardNote {
            id: id.clone(),
            text: String::new(),
            color: FolderColor::Yellow,
            x: js::round(p.x),
            y: js::round(p.y),
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
        self.set_renaming(Some(id), cx);
    }

    /// `peelNote` (BOARD-117).
    pub(crate) fn peel_note(&mut self, p: Point, cx: &mut Context<'_, Self>) {
        let id = std::mem::replace(&mut self.draft_id, uuid::Uuid::new_v4().to_string());
        self.add_note(p, id, cx);
        self.rebuild(cx);
    }

    pub(crate) fn free_note_spot(&self, p: Point) -> Point {
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        find_free_spot(p, &obstacles, Some(self.dock()), NOTE_SIZE)
    }

    /// BOARD-118.
    pub fn note_beside_pad(&mut self, cx: &mut Context<'_, Self>) {
        let Some(board) = &self.board else { return };
        let pad = layout::note_pad_rect(board.archive);
        let spot = self.free_note_spot(Point {
            x: pad.x + pad.w + nodus_board::geometry::GAP,
            y: pad_sheet_spot(pad).y,
        });
        self.peel_note(spot, cx);
        self.ensure_visible(spot, NOTE_SIZE, cx);
    }

    /// BOARD-119.
    pub fn create_note_at(&mut self, p: Point, cx: &mut Context<'_, Self>) {
        let spot = self.free_note_spot(Point {
            x: p.x - NOTE_W / 2.0,
            y: p.y - NOTE_H / 2.0,
        });
        self.add_note(spot, uuid::Uuid::new_v4().to_string(), cx);
        self.ensure_visible(spot, NOTE_SIZE, cx);
    }

    /// BOARD-120.
    pub fn recolor_notes(&mut self, ids: &[String], color: FolderColor, cx: &mut Context<'_, Self>) {
        let notes: Vec<BoardNote> = self
            .board
            .as_ref()
            .map(|b| b.notes.iter().filter(|n| ids.contains(&n.id)).cloned().collect())
            .unwrap_or_default();
        self.state.update(cx, |s, cx| {
            for note in notes {
                s.save_note(BoardNote { color, ..note }, cx);
            }
        });
    }

    /// BOARD-121.
    pub fn delete_notes(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let targets: Vec<String> = self
            .board
            .as_ref()
            .map(|b| {
                b.notes
                    .iter()
                    .filter(|n| ids.contains(&n.id))
                    .map(|n| n.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if !targets.is_empty() {
            self.remove_notes(targets, cx);
        }
    }

    /// BOARD-124: typing outlives a board closed mid-note.
    pub fn edit_note(&mut self, note: BoardNote, text: String, cx: &mut Context<'_, Self>) {
        let next = BoardNote { text, ..note };
        let open = self
            .state
            .read(cx)
            .board
            .as_ref()
            .is_some_and(|b| b.project.id == self.project_id);
        if open {
            self.save_note(next, cx);
        } else {
            let nodus = self.nodus(cx);
            let rt = crate::board::core_rt(cx);
            let project_id = self.project_id.clone();
            cx.spawn(async move |_, _| {
                crate::board::on_core(&rt, {
                    let (nodus, project_id, next) = (nodus.clone(), project_id.clone(), next.clone());
                    async move { nodus.board().save_note(&project_id, &next).await }
                })
                .await;
            })
            .detach();
        }
    }

    /// BOARD-122, BOARD-123.
    pub fn start_note(&mut self, note_id: String, cx: &mut Context<'_, Self>) {
        let Some(board) = self.board.clone() else { return };
        let Some(note) = board.notes.iter().find(|n| n.id == note_id).cloned() else {
            return;
        };
        if note.session_id.is_some() || js::trim(&note.text).is_empty() || self.starting_notes.contains(&note_id) {
            return;
        }
        self.starting_notes.insert(note_id.clone());
        let others: Vec<BoardNote> = board.notes.iter().filter(|n| n.id != note_id).cloned().collect();
        let mut obstacles = card_obstacles(&board.cards, &[]);
        obstacles.extend(note_obstacles(&others));
        obstacles.extend(view_obstacles(&board.views));
        let spot = find_free_spot(
            Point { x: note.x, y: note.y },
            &obstacles,
            Some(self.dock()),
            NEW_CARD_SIZE,
        );
        let pos = round(spot);
        let opts = CreateSessionOptions {
            x: pos.x,
            y: pos.y,
            initial_prompt: Some(note.text.clone()),
            auto_mode: Some(true),
            ..CreateSessionOptions::default()
        };
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created = crate::board::on_core(&rt, {
                let (nodus, project_id, opts) = (nodus.clone(), project_id.clone(), opts.clone());
                async move { nodus.sessions().create(&project_id, &opts).await }
            })
            .await;
            let _ = this.update(cx, |c, cx| {
                match created {
                    Ok(card) => {
                        let id = card.node.session_id.clone();
                        let latest = c
                            .board
                            .as_ref()
                            .and_then(|b| b.notes.iter().find(|n| n.id == note_id).cloned());
                        c.state.update(cx, |s, cx| {
                            s.upsert_card(card, cx);
                            if let Some(latest) = latest {
                                s.save_note(
                                    BoardNote {
                                        session_id: Some(id),
                                        ..latest
                                    },
                                    cx,
                                );
                            }
                        });
                        c.ensure_visible(pos, CARD_SIZE, cx);
                    }
                    Err(err) => c.show_toast(format!("Couldn't start a session: {}", message(&err)), cx),
                }
                c.starting_notes.remove(&note_id);
            });
        })
        .detach();
    }

    // ---- views ------------------------------------------------------------------------

    /// BOARD-125: errors go back to the caller.
    pub fn create_view(
        &mut self,
        description: String,
        at: Option<Point>,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Task<Result<(), nodus_core::CommandError>> {
        let preferred = match at {
            Some(at) => Point {
                x: at.x - VIEW_SIZE.w / 2.0,
                y: at.y - VIEW_SIZE.h / 2.0,
            },
            None => self.viewport_centre(VIEW_SIZE),
        };
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        let spot = find_free_spot(preferred, &obstacles, Some(self.dock()), VIEW_SIZE);
        self.place_view(description, spot, true, cx)
    }

    /// A view with its top left exactly at `at`, the board left where it is
    /// (BUBBLE-031). Errors go back to the caller.
    pub fn create_view_at(
        &mut self,
        description: String,
        at: Point,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Task<Result<(), nodus_core::CommandError>> {
        self.place_view(description, at, false, cx)
    }

    /// A view at `spot`, brought into view when `follow`.
    fn place_view(
        &mut self,
        description: String,
        spot: Point,
        follow: bool,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Task<Result<(), nodus_core::CommandError>> {
        let opts = nodus_domain::CreateViewOptions {
            description,
            x: js::round(spot.x),
            y: js::round(spot.y),
        };
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let view = crate::board::on_core(&rt, {
                let (nodus, project_id, opts) = (nodus.clone(), project_id.clone(), opts.clone());
                async move { nodus.views().create(&project_id, &opts).await }
            })
            .await?;
            let _ = this.update(cx, |c, cx| {
                c.state.update(cx, |s, cx| s.upsert_view(view, cx));
                if follow {
                    c.ensure_visible(spot, VIEW_SIZE, cx);
                }
            });
            Ok(())
        })
    }

    /// BOARD-126.
    pub fn open_view_session(&mut self, view_id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let brain = crate::board::on_core(&rt, {
                let (nodus, project_id, view_id) = (nodus.clone(), project_id.clone(), view_id.clone());
                async move { nodus.views().brain(&project_id, &view_id).await }
            })
            .await;
            let _ = this.update(cx, |c, cx| match brain {
                Ok(id) => c.state.update(cx, |s, cx| s.open_terminal(&id, cx)),
                Err(err) => c.show_toast(format!("Couldn't open its session: {}", message(&err)), cx),
            });
        })
        .detach();
    }

    /// BOARD-127.
    pub fn delete_views(&mut self, ids: &[String], cx: &mut Context<'_, Self>) {
        let targets: Vec<nodus_domain::BoardView> = self
            .board
            .as_ref()
            .map(|b| b.views.iter().filter(|v| ids.contains(&v.id)).cloned().collect())
            .unwrap_or_default();
        if targets.is_empty() {
            return;
        }
        let (title, body) = if targets.len() == 1 {
            (
                "Delete view?".to_owned(),
                format!(
                    "“{}” and its page go, and the Claude session that keeps it up to date stops. Its transcript stays on disk.",
                    targets[0].title
                ),
            )
        } else {
            (
                format!("Delete {} views?", targets.len()),
                "Their pages go, and the Claude sessions that keep them up to date stop. Their transcripts stay on disk."
                    .to_owned(),
            )
        };
        let ask = self.confirm(title, body, "Delete", true, cx);
        cx.spawn(async move |this, cx| {
            if !ask.await.unwrap_or(false) {
                return;
            }
            for view in targets {
                let Ok(task) = this.update(cx, |c, cx| c.state.update(cx, |s, cx| s.remove_view(&view.id, cx))) else {
                    return;
                };
                task.await;
            }
        })
        .detach();
    }

    /// BOARD-128: `Refresh`.
    pub fn refresh_view(&mut self, view_id: &str, cx: &mut Context<'_, Self>) {
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        let view_id = view_id.to_owned();
        cx.spawn(async move |this, cx| {
            if let Err(err) = crate::board::on_core(&rt, {
                let (nodus, project_id, view_id) = (nodus.clone(), project_id.clone(), view_id.clone());
                async move { nodus.views().refresh(&project_id, &view_id).await }
            })
            .await
            {
                let _ = this.update(cx, |c, cx| {
                    c.show_toast(format!("Couldn't refresh it: {}", message(&err)), cx);
                });
            }
        })
        .detach();
    }

    /// `Stop it starting sessions`.
    pub fn stop_view_starting(&mut self, view_id: &str, cx: &mut Context<'_, Self>) {
        self.patch_view(
            ViewPatch {
                id: view_id.to_owned(),
                x: None,
                y: None,
                w: None,
                h: None,
                title: None,
                starts_sessions: Some(false),
            },
            cx,
        );
    }

    /// The view relay's `startSession` (BOARD-129, VIEW-056…058): beside the
    /// view, expanded, in the view's permission mode (VIEW-155), with what it
    /// was asked stuck on it.
    pub fn start_view_session(
        &mut self,
        view_id: &str,
        prompt: String,
        title: Option<String>,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        let Some(view) = self
            .board
            .as_ref()
            .and_then(|b| b.views.iter().find(|v| v.id == view_id).cloned())
        else {
            return Err("This view isn’t allowed to start sessions. The user can ask its brain to let it.".into());
        };
        if view.starts_sessions != Some(true) {
            self.show_toast(
                format!(
                    "“{}” tried to start a session. Ask its brain if you want it to be able to.",
                    view.title
                ),
                cx,
            );
            return Err("This view isn’t allowed to start sessions. The user can ask its brain to let it.".into());
        }
        let now = crate::parity::now_ms(cx);
        let last = self.view_starts.get(view_id).copied().unwrap_or(0.0);
        if now - last < VIEW_START_GAP_MS {
            return Err("This view just started a session; wait a moment before starting another".into());
        }
        self.view_starts.insert(view_id.to_owned(), now);
        let size = nodus_board::geometry::view_size(&view);
        let anchor = nodus_domain::Rect {
            x: view.x,
            y: view.y,
            w: size.w,
            h: size.h,
        };
        let mut obstacles = card_obstacles(self.cards(), &[]);
        obstacles.extend(self.notes_in_the_way());
        let spot = find_spot_beside(anchor, NEW_CARD_SIZE, &obstacles, Some(self.dock()), VIEW_SESSION_GAP);
        let pos = round(spot);
        let opts = CreateSessionOptions {
            x: pos.x,
            y: pos.y,
            name: title.map(|t| js::slice_utf16(&t, 0, 80)).filter(|t| !t.is_empty()),
            initial_prompt: Some(prompt.clone()),
            expanded: Some(true),
            auto_mode: Some(view.runs_in_auto_mode()),
            from_view: Some(view.id.clone()),
            ..CreateSessionOptions::default()
        };
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let Ok(card) = crate::board::on_core(&rt, {
                let (nodus, project_id, opts) = (nodus.clone(), project_id.clone(), opts.clone());
                async move { nodus.sessions().create(&project_id, &opts).await }
            })
            .await
            else {
                return;
            };
            let _ = this.update(cx, |c, cx| {
                let id = card.node.session_id.clone();
                let note = BoardNote {
                    id: uuid::Uuid::new_v4().to_string(),
                    text: prompt,
                    color: FolderColor::Yellow,
                    x: pos.x,
                    y: pos.y,
                    w: None,
                    h: None,
                    session_id: Some(id.clone()),
                    from: None,
                    from_project: None,
                    from_title: None,
                    created_at: crate::parity::now_ms(cx),
                    extra: serde_json::Map::new(),
                };
                c.state.update(cx, |s, cx| {
                    s.upsert_card(card, cx);
                    s.save_note(note, cx);
                });
                c.ensure_visible(pos, NEW_CARD_SIZE, cx);
            });
        })
        .detach();
        Ok(())
    }

    // ---- runs and notifications ----------------------------------------------------------

    /// BOARD-104: a run of an action, at the new-session spot for a card.
    pub fn run_action(&mut self, action_id: String, action_name: String, cx: &mut Context<'_, Self>) {
        let spot = spot_for_new_session(self.dock(), self.cards(), CARD_SIZE, &self.notes_in_the_way());
        let opts = CreateSessionOptions {
            x: js::round(spot.x),
            y: js::round(spot.y),
            action_id: Some(action_id),
            ..CreateSessionOptions::default()
        };
        let nodus = self.nodus(cx);
        let rt = crate::board::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let created = crate::board::on_core(&rt, {
                let (nodus, project_id, opts) = (nodus.clone(), project_id.clone(), opts.clone());
                async move { nodus.sessions().create(&project_id, &opts).await }
            })
            .await;
            let _ = this.update(cx, |c, cx| match created {
                Ok(card) => c.state.update(cx, |s, cx| s.upsert_card(card, cx)),
                Err(err) => c.show_toast(format!("Couldn't run {action_name}: {}", message(&err)), cx),
            });
        })
        .detach();
    }

    /// BOARD-105: a run the user took over joins the board, moved clear.
    pub(crate) fn join_taken_over_runs(&mut self, cx: &mut Context<'_, Self>) {
        let Some(source) = &self.source else { return };
        let joined: Vec<SessionCard> = self
            .cards()
            .iter()
            .filter(|c| self.run_ids.contains(&c.node.session_id))
            .cloned()
            .collect();
        self.run_ids = source
            .cards
            .iter()
            .filter(|c| c.node.run_of.is_some())
            .map(|c| c.node.session_id.clone())
            .collect();
        let mut patches = Vec::new();
        for card in joined {
            let mut obstacles = card_obstacles(
                self.cards().iter().filter(|c| !c.node.archived),
                &[&card.node.session_id],
            );
            obstacles.extend(self.notes_in_the_way());
            let spot = find_free_spot(
                Point {
                    x: card.node.x,
                    y: card.node.y,
                },
                &obstacles,
                Some(self.dock()),
                card_size(&card),
            );
            let (x, y) = (js::round(spot.x), js::round(spot.y));
            if x != card.node.x || y != card.node.y {
                patches.push(NodePatch {
                    session_id: card.node.session_id.clone(),
                    x: Some(x),
                    y: Some(y),
                    ..NodePatch::default()
                });
            }
        }
        if !patches.is_empty() {
            let state = self.state.clone();
            cx.defer(move |cx| state.update(cx, |s, cx| s.patch_cards(patches, cx)));
        }
    }

    /// BOARD-106. MODAL-029: the run's terminal is disposed, and its window
    /// closes if it shows the run.
    pub fn run_finished(&mut self, run: &ActionRunResult, cx: &mut Context<'_, Self>) {
        if run.project_id != self.project_id {
            return;
        }
        let id = run.session_id.clone();
        crate::terminal::store::dispose(&id, cx);
        self.state.update(cx, |s, cx| {
            if s.terminal_session_id.as_deref() == Some(id.as_str()) {
                s.terminal_session_id = None;
                cx.notify();
            }
        });
        let text = match run.message.as_deref() {
            Some(m) if !m.is_empty() => format!("{}: {}", run.action_name, layout::truncate(m, 180)),
            _ => format!("{} finished", run.action_name),
        };
        self.show_toast(text, cx);
    }

    /// BOARD-107: a card a notification asked to open.
    pub fn take_open_request(&mut self, cx: &mut Context<'_, Self>) {
        let request = self.state.read(cx).open_request.clone();
        let Some(request) = request else { return };
        let Some(session_id) = request.session_id.clone() else {
            return;
        };
        if request.project_id != self.project_id || self.board.is_none() {
            return;
        }
        self.state.update(cx, |s, _| s.open_request = None);
        if let Some(card) = self.card(&session_id).cloned() {
            self.ensure_visible(
                Point {
                    x: card.node.x,
                    y: card.node.y,
                },
                card_size(&card),
                cx,
            );
            self.open_card(&session_id, cx);
        } else if self.source.as_ref().is_some_and(|b| {
            b.cards
                .iter()
                .any(|c| c.node.session_id == session_id && c.node.run_of.is_some())
        }) {
            self.state.update(cx, |s, cx| s.open_terminal(&session_id, cx));
        }
    }

    /// `jumpToCard`: takes a search result to its session. BOARD-036: an
    /// active card is centred, selected and flashed. BOARD-037: an archived
    /// one's transcript opens at the match (`find`), the viewport untouched.
    /// An unknown id does nothing, and search stays open.
    pub fn jump_to_card(&mut self, session_id: &str, find: Option<PreviewFind>, cx: &mut Context<'_, Self>) {
        let Some(card) = self.card(session_id).cloned() else {
            return;
        };
        self.search_open = false;
        if card.node.archived {
            let id = session_id.to_owned();
            self.state.update(cx, |s, cx| s.open_preview_at(&id, find, cx));
            return;
        }
        self.select_cards(&[session_id.to_owned()], cx);
        let size = card_size(&card);
        let zoom = self.viewport.zoom.max(0.8);
        let (w, h) = self.canvas_size();
        let to = super::viewport::centred_on(card.node.x + size.w / 2.0, card.node.y + size.h / 2.0, zoom, w, h);
        self.fly_to(to, 320.0, cx);
        // Then the card flashes, twice over 800ms (drawn by the card, spec 10).
        let id = session_id.to_owned();
        let landing = crate::theme::motion::duration(cx, Duration::from_millis(320));
        self.flash_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(landing).await;
            let _ = this.update(cx, |c, cx| {
                c.flash = Some((id, std::time::Instant::now()));
                cx.notify();
            });
            cx.background_executor().timer(Duration::from_millis(2 * 800)).await;
            let _ = this.update(cx, |c, cx| {
                c.flash = None;
                cx.notify();
            });
        }));
    }

    /// Whether a flow point is in the dock (BOARD-048, BOARD-173).
    pub(crate) fn in_dock(&self, p: Point) -> bool {
        contains_point(self.dock(), p)
    }

    /// Whether a flow point is inside any section frame.
    pub(crate) fn in_section(&self, p: Point) -> bool {
        self.groups().iter().any(|g| contains_point(g.frame, p))
    }
}
