//! The board's context menus (spec 09 § Context menus; BOARD-172…BOARD-177,
//! BOARD-128): pane, card, terminal card, section, note, note pad and view,
//! with 0.9.4's items, hints and enable rules.

use std::rc::Rc;

use gpui::{Context, IntoElement, Pixels, Point as GPoint, WeakEntity};
use nodus_domain::{FolderColor, SessionCard, SessionKind};

use super::canvas::BoardCanvas;
use super::layout::{is_expanded, is_terminal, section_views, split_notes};
use super::scene::NodeKind;
use crate::components::color_swatches::color_swatches;
use crate::components::context_menu::MenuItem;
use crate::components::icon::Icon;
use crate::platform::keys;

/// An item that runs a board command.
fn item(
    canvas: &WeakEntity<BoardCanvas>,
    label: impl Into<gpui::SharedString>,
    icon: Icon,
    run: impl Fn(&mut BoardCanvas, &mut Context<'_, BoardCanvas>) + 'static,
) -> MenuItem {
    let canvas = canvas.clone();
    MenuItem::item(label, icon, move |_, cx| {
        if let Some(canvas) = canvas.upgrade() {
            canvas.update(cx, |c, cx| run(c, cx));
        }
    })
}

fn disabled(mut item: MenuItem, off: bool) -> MenuItem {
    if let MenuItem::Item { disabled, .. } = &mut item {
        *disabled = off;
    }
    item
}

/// The colour swatches row (`ColorSwatches`): picking closes the menu, then recolours.
fn swatches(
    canvas: &WeakEntity<BoardCanvas>,
    value: FolderColor,
    pick: impl Fn(&mut BoardCanvas, FolderColor, &mut Context<'_, BoardCanvas>) + 'static,
) -> MenuItem {
    let canvas = canvas.clone();
    let pick = Rc::new(pick);
    MenuItem::Custom(Rc::new(move |window, cx| {
        let canvas = canvas.clone();
        let pick = pick.clone();
        let on_pick: crate::components::ColorHandler = Rc::new(move |color, _, cx| {
            if let Some(canvas) = canvas.upgrade() {
                canvas.update(cx, |c, cx| {
                    c.close_menu(cx);
                    pick(c, color, cx);
                });
            }
        });
        color_swatches(value, &on_pick, window, cx).into_any_element()
    }))
}

impl BoardCanvas {
    /// BOARD-173: right-click on the pane (or inside a section frame).
    pub(crate) fn pane_menu(&mut self, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let p = self.to_flow(at);
        let in_archive = self.in_dock(p);
        let in_section = self.in_section(p);
        let me = cx.entity().downgrade();
        let items = vec![
            disabled(
                item(&me, "New session here", Icon::Plus, move |c, cx| {
                    c.create_session_centred(p, SessionKind::Claude, cx);
                })
                .hint("N"),
                in_archive,
            ),
            disabled(
                item(&me, "New terminal here", Icon::SquareTerminal, move |c, cx| {
                    c.create_session_centred(p, SessionKind::Terminal, cx);
                })
                .hint("T"),
                in_archive,
            ),
            disabled(
                item(&me, "New section here", Icon::SquareDashed, move |c, cx| {
                    c.create_section_at(p, cx)
                }),
                in_archive || in_section,
            ),
            disabled(
                item(&me, "New note here", Icon::StickyNote, move |c, cx| {
                    c.create_note_at(p, cx)
                }),
                in_archive,
            ),
            disabled(
                // The New view dialog is spec 04's.
                item(&me, "New view here…", Icon::LayoutDashboard, move |_, _| {}),
                in_archive,
            ),
            MenuItem::Separator,
            item(
                &me,
                "Select all active",
                Icon::SquareDashed,
                BoardCanvas::select_all_active,
            )
            .hint(keys("⌘A")),
            item(&me, "Zoom to fit", Icon::Maximize, |c, cx| c.fit_active(320.0, cx)),
            // BOARD-177: every drawn node, padding 0.12, up to zoom 2.
            item(&me, "Show everything", Icon::Maximize, |c, cx| {
                c.fit_nodes(None, 0.12, 2.0, 320.0, cx)
            }),
        ];
        self.open_menu(at, items, cx);
    }

    /// BOARD-174: right-click on a node.
    pub(crate) fn node_menu(&mut self, id: &str, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let Some(node) = self.node(id).cloned() else { return };
        let Some(board) = self.board.clone() else { return };
        let me = cx.entity().downgrade();
        let items = match &node.kind {
            NodeKind::Pad | NodeKind::Note { index: None } => {
                vec![item(&me, "New note", Icon::StickyNote, BoardCanvas::note_beside_pad)]
            }
            NodeKind::Note { index: Some(i) } => self.note_menu(&board.notes[*i], &me),
            NodeKind::View { index } => self.view_menu_items(&board.views[*index], &me),
            NodeKind::Section { index, .. } => self.section_menu(*index, &me),
            NodeKind::Card { index, stack, .. } => {
                let card = &board.cards[*index];
                if is_terminal(card) {
                    self.terminal_menu(card, &me)
                } else {
                    let stickers = split_notes(&board.notes, &board.cards)
                        .1
                        .into_iter()
                        .find(|(sid, _)| sid == id && !card.node.archived)
                        .map(|(_, notes)| notes.iter().map(|n| n.id.clone()).collect())
                        .unwrap_or_default();
                    let _ = stack;
                    self.card_menu(card, stickers, &me)
                }
            }
            NodeKind::Archive { .. } => return,
        };
        self.open_menu(at, items, cx);
    }

    /// `cardOpenItems`.
    fn card_open_items(&self, card: &SessionCard, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let id = card.node.session_id.clone();
        let other = id.clone();
        if is_expanded(card) {
            vec![
                item(me, "Open in a window", Icon::AppWindowMac, move |c, cx| {
                    let id = id.clone();
                    c.state.update(cx, |s, cx| s.open_terminal(&id, cx));
                }),
                item(me, "Collapse", Icon::Minimize2, move |c, cx| {
                    c.expand_cards(std::slice::from_ref(&other), false, cx)
                })
                .hint("E"),
            ]
        } else {
            vec![
                item(me, "Open", Icon::SquareTerminal, move |c, cx| {
                    let id = id.clone();
                    c.state.update(cx, |s, cx| s.open_terminal(&id, cx));
                })
                .hint(keys("↩")),
                item(me, "Expand on board", Icon::Maximize2, move |c, cx| {
                    c.expand_cards(std::slice::from_ref(&other), true, cx);
                })
                .hint("E"),
            ]
        }
    }

    /// `cardSectionItems`: group it (or the selection it's in), or take it out.
    fn card_section_items(&self, card: &SessionCard, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        if card.node.archived {
            return Vec::new();
        }
        let sel = self.selected_cards();
        let ids = if sel.contains(&card.node.session_id) && sel.len() > 1 {
            sel
        } else {
            vec![card.node.session_id.clone()]
        };
        let count = self.board.as_ref().map_or(0, |b| {
            b.cards
                .iter()
                .filter(|c| ids.contains(&c.node.session_id) && !c.node.archived)
                .count()
        });
        let label = if count > 1 {
            format!("Group {count} into section")
        } else {
            "Group into section".to_owned()
        };
        let mut items = vec![item(me, label, Icon::Group, move |c, cx| c.group_cards(&ids, cx)).hint(keys("⌘G"))];
        if card.node.section_id.is_some() {
            let id = card.node.session_id.clone();
            items.push(item(me, "Remove from section", Icon::Ungroup, move |c, cx| {
                c.leave_section(&id, cx)
            }));
        }
        items
    }

    /// The terminal card's menu.
    fn terminal_menu(&self, card: &SessionCard, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let id = card.node.session_id.clone();
        let rename = id.clone();
        let mut items = self.card_open_items(card, me);
        items.push(item(me, "Rename", Icon::PencilLine, move |c, cx| {
            c.set_renaming(Some(rename.clone()), cx)
        }));
        items.extend(self.card_section_items(card, me));
        items.push(MenuItem::Separator);
        items.push(
            item(me, "Remove terminal", Icon::Trash2, move |c, cx| {
                c.remove_terminal(&id, cx)
            })
            .danger(),
        );
        items
    }

    /// The card menu (BOARD-175: the whole selection minus terminals when
    /// the card is part of a multi-selection).
    fn card_menu(&self, card: &SessionCard, stickers: Vec<String>, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let Some(board) = &self.board else { return Vec::new() };
        let id = card.node.session_id.clone();
        let sel = self.selected_cards();
        let group: Vec<String> = if sel.contains(&id) && sel.len() > 1 {
            sel.into_iter()
                .filter(|s| {
                    board
                        .cards
                        .iter()
                        .find(|c| &c.node.session_id == s)
                        .is_none_or(|c| !is_terminal(c))
                })
                .collect()
        } else {
            vec![id.clone()]
        };
        let picked: Vec<SessionCard> = group
            .iter()
            .filter_map(|g| board.cards.iter().find(|c| &c.node.session_id == g).cloned())
            .collect();
        let running: Vec<String> = nodus_board::stoppable(&picked)
            .into_iter()
            .map(|c| c.node.session_id.clone())
            .collect();

        let mut items = Vec::new();
        if card.node.archived {
            let preview = id.clone();
            items.push(
                item(me, "Preview", Icon::Eye, move |c, cx| {
                    let id = preview.clone();
                    c.state.update(cx, |s, cx| s.open_preview(&id, cx));
                })
                .hint(keys("↩")),
            );
        } else {
            items.extend(self.card_open_items(card, me));
        }
        if let Some(count) = card.agent_count.filter(|n| *n > 0) {
            let agents = id.clone();
            items.push(item(me, format!("Agents ({count})"), Icon::Bot, move |c, cx| {
                let id = agents.clone();
                c.state.update(cx, |s, cx| s.open_agents(&id, None, cx));
            }));
        }
        let rename = id.clone();
        items.push(item(me, "Rename", Icon::PencilLine, move |c, cx| {
            c.set_renaming(Some(rename.clone()), cx)
        }));
        if card.node.archived {
            let restore = id.clone();
            items.push(item(me, "Restore to board", Icon::ArchiveRestore, move |c, cx| {
                c.restore_card(&restore, cx);
            }));
        } else {
            let label = if group.len() > 1 {
                format!("Archive {} sessions", group.len())
            } else {
                "Archive".to_owned()
            };
            let ids = group.clone();
            items.push(item(me, label, Icon::Archive, move |c, cx| c.archive_cards(&ids, cx)).hint(keys("⌫")));
        }
        if !running.is_empty() {
            let label = if running.len() > 1 {
                format!("Stop {} sessions", running.len())
            } else {
                "Stop session".to_owned()
            };
            items.push(item(me, label, Icon::Square, move |c, cx| c.stop_cards(&running, cx)));
        }
        let merge_label = if group.len() > 1 {
            format!("New session from {} contexts…", group.len())
        } else {
            "New session from context…".to_owned()
        };
        let merge = group.clone();
        items.push(item(me, merge_label, Icon::Sparkles, move |c, cx| {
            c.start_merge(&merge, cx)
        }));
        items.extend(self.card_section_items(card, me));
        if !stickers.is_empty() {
            let label = if stickers.len() > 1 {
                "Delete its notes"
            } else {
                "Delete its note"
            };
            items.push(item(me, label, Icon::StickyNote, move |c, cx| {
                c.remove_notes(stickers.clone(), cx)
            }));
        }
        items.push(MenuItem::Separator);
        items.push(
            item(me, "Remove from board", Icon::Trash2, move |c, cx| {
                c.remove_from_board(&id, cx)
            })
            .danger(),
        );
        items
    }

    /// The section menu.
    fn section_menu(&self, index: usize, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let Some(board) = &self.board else { return Vec::new() };
        let Some(group) = section_views(&board.sections, &board.cards).into_iter().nth(index) else {
            return Vec::new();
        };
        let id = group.section.id.clone();
        let members: Vec<String> = group
            .members
            .iter()
            .map(|&i| board.cards[i].node.session_id.clone())
            .collect();
        let sessions: Vec<String> = group
            .members
            .iter()
            .filter(|&&i| !is_terminal(&board.cards[i]))
            .map(|&i| board.cards[i].node.session_id.clone())
            .collect();
        let (rename, recolor, new_session, new_terminal, remove) =
            (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        let archive_label = if sessions.len() > 1 {
            format!("Archive {} sessions", sessions.len())
        } else {
            "Archive session".to_owned()
        };
        let has_members = !members.is_empty();
        let has_sessions = !sessions.is_empty();
        vec![
            item(me, "Rename", Icon::PencilLine, move |c, cx| {
                c.set_renaming(Some(rename.clone()), cx)
            }),
            swatches(me, group.section.color, move |c, color, cx| {
                c.recolor_section(&recolor, color, cx)
            }),
            MenuItem::Separator,
            item(me, "New session in section", Icon::Plus, move |c, cx| {
                c.create_in_section(&new_session, SessionKind::Claude, cx);
            }),
            item(me, "New terminal in section", Icon::SquareTerminal, move |c, cx| {
                c.create_in_section(&new_terminal, SessionKind::Terminal, cx);
            }),
            disabled(
                item(me, "Select cards", Icon::SquareDashed, move |c, cx| {
                    c.select_cards(&members, cx)
                }),
                !has_members,
            ),
            disabled(
                item(me, archive_label, Icon::Archive, move |c, cx| {
                    c.archive_cards(&sessions, cx)
                }),
                !has_sessions,
            ),
            MenuItem::Separator,
            item(
                me,
                if has_members { "Ungroup" } else { "Delete section" },
                Icon::Ungroup,
                move |c, cx| c.remove_section(&remove, cx),
            ),
        ]
    }

    /// The note menu: acts on the selected notes when this one is among them.
    fn note_menu(&self, note: &nodus_domain::BoardNote, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let sel = self.selected_notes();
        let ids = if sel.contains(&note.id) && sel.len() > 1 {
            sel
        } else {
            vec![note.id.clone()]
        };
        let (edit, start) = (note.id.clone(), note.id.clone());
        let recolor = ids.clone();
        let blank = nodus_domain::js::trim(&note.text).is_empty();
        let delete_label = if ids.len() > 1 {
            format!("Delete {} notes", ids.len())
        } else {
            "Delete note".to_owned()
        };
        vec![
            item(me, "Edit", Icon::PencilLine, move |c, cx| {
                c.set_renaming(Some(edit.clone()), cx)
            })
            .hint(keys("↩")),
            disabled(
                item(me, "Start session", Icon::Play, move |c, cx| {
                    c.start_note(start.clone(), cx)
                }),
                blank,
            ),
            swatches(me, note.color, move |c, color, cx| c.recolor_notes(&recolor, color, cx)),
            MenuItem::Separator,
            item(me, delete_label, Icon::Trash2, move |c, cx| c.delete_notes(&ids, cx))
                .hint(keys("⌫"))
                .danger(),
        ]
    }

    /// The view menu (BOARD-128).
    fn view_menu_items(&self, view: &nodus_domain::BoardView, me: &WeakEntity<BoardCanvas>) -> Vec<MenuItem> {
        let can_refresh = view.refresh.is_some()
            || view
                .commands
                .as_ref()
                .is_some_and(|cmds| cmds.iter().any(|c| c.approved.as_deref() == Some(c.run.as_str())));
        let (refresh, open, rename, stop, delete) = (
            view.id.clone(),
            view.id.clone(),
            view.id.clone(),
            view.id.clone(),
            view.id.clone(),
        );
        let mut items = vec![
            disabled(
                item(me, "Refresh", Icon::RefreshCw, move |c, cx| {
                    c.refresh_view(&refresh, cx)
                }),
                !can_refresh,
            ),
            item(me, "Open its session", Icon::SquareTerminal, move |c, cx| {
                c.open_view_session(open.clone(), cx)
            }),
            item(me, "Rename", Icon::PencilLine, move |c, cx| {
                c.set_renaming(Some(rename.clone()), cx)
            }),
        ];
        if view.starts_sessions == Some(true) {
            items.push(item(me, "Stop it starting sessions", Icon::Ban, move |c, cx| {
                c.stop_view_starting(&stop, cx)
            }));
        }
        items.push(MenuItem::Separator);
        items.push(
            item(me, "Delete view", Icon::Trash2, move |c, cx| {
                c.delete_views(std::slice::from_ref(&delete), cx)
            })
            .hint(keys("⌫"))
            .danger(),
        );
        items
    }

    /// A view's own menu button (BOARD-128).
    pub fn view_menu(&mut self, view_id: &str, at: GPoint<Pixels>, cx: &mut Context<'_, Self>) {
        let Some(view) = self
            .board
            .as_ref()
            .and_then(|b| b.views.iter().find(|v| v.id == view_id).cloned())
        else {
            return;
        };
        let me = cx.entity().downgrade();
        let items = self.view_menu_items(&view, &me);
        self.open_menu(at, items, cx);
    }
}
