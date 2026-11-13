//! Cards as the window and the board tools see them (board.ts's `toCard`,
//! `sessionList`).

use nodus_domain::{BoardNode, BoardSection, Rect, SessionCard, SessionKind, SessionStatus};
use nodus_transcripts::DiscoveredSession;
use serde::Serialize;

use crate::BoardSessions;

/// A node as a card: what's known about its session, its status, whether
/// it's running and what it has cost (SESS-080). Its subagents are added
/// after, with [`BoardSessions::attach_agents`].
pub fn to_card(node: &BoardNode, found: Option<&DiscoveredSession>, sessions: &dyn BoardSessions) -> SessionCard {
    let id = &node.session_id;
    SessionCard {
        node: node.clone(),
        meta: found.filter(|f| f.has_content).map(|f| f.meta.clone()),
        status: if node.archived {
            SessionStatus::Done
        } else {
            sessions.status(id)
        },
        live: sessions.is_live(id),
        agents: None,
        agent_count: None,
        usage: sessions.usage(id),
    }
}

/// A session as the board tools and views list it (SESS-159). Fields are in
/// the order the TypeScript writes them, since Claude reads them as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListItem {
    pub id: String,
    pub title: String,
    pub kind: SessionKind,
    /// `archived`, `waiting for the user`, `working` or `done`.
    pub status: String,
    pub running: bool,
    /// The frame it's in (FRAME-040), by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reply: Option<String>,
    /// ISO 8601, to the millisecond.
    pub last_active: String,
}

/// The cards other than `self_id` and views' brains, archived ones only when
/// asked for, as list items (SESS-159).
pub fn session_list_items(
    cards: &[SessionCard],
    sections: &[BoardSection],
    self_id: Option<&str>,
    include_archived: bool,
) -> Vec<SessionListItem> {
    let rect = |c: &SessionCard| {
        let size = crate::geometry::card_size(Some(&c.node));
        Rect {
            x: c.node.x,
            y: c.node.y,
            w: size.w,
            h: size.h,
        }
    };
    let frames: Vec<(&BoardSection, Rect)> = sections
        .iter()
        .map(|s| {
            let members: Vec<Rect> = cards
                .iter()
                .filter(|c| !c.node.archived && c.node.section_id.as_deref() == Some(s.id.as_str()))
                .map(rect)
                .collect();
            (s, crate::frames::frame_rect(s, &members))
        })
        .collect();
    cards
        .iter()
        .filter(|c| Some(c.node.session_id.as_str()) != self_id)
        .filter(|c| !set(c.node.view_of.as_deref()) && (include_archived || !c.node.archived))
        .map(|c| {
            let node = &c.node;
            let meta = c.meta.as_ref();
            SessionListItem {
                id: node.session_id.clone(),
                title: card_title(c),
                kind: node.kind.unwrap_or(SessionKind::Claude),
                status: if node.archived {
                    "archived".into()
                } else if c.status == SessionStatus::Waiting {
                    "waiting for the user".into()
                } else {
                    c.status.as_str().into()
                },
                running: c.live,
                section: (!node.archived)
                    .then(|| crate::frames::frame_holding(&frames, rect(c)))
                    .flatten()
                    .map(|s| {
                        if s.name.is_empty() {
                            "Untitled frame".to_owned()
                        } else {
                            s.name.clone()
                        }
                    }),
                action_run: set(node.run_of.as_deref()).then_some(true),
                branch: meta.and_then(|m| m.git_branch.clone()),
                last_reply: meta.and_then(|m| m.last_assistant_text.clone()),
                last_active: nodus_transcripts::date::to_iso_string(meta.map_or(node.created_at, |m| m.updated_at)),
            }
        })
        .collect()
}

/// `card.title || card.meta?.title || (terminal ? 'Terminal' : 'New session')`.
fn card_title(c: &SessionCard) -> String {
    [c.node.title.as_deref(), c.meta.as_ref().map(|m| m.title.as_str())]
        .into_iter()
        .flatten()
        .find(|t| !t.is_empty())
        .map_or_else(
            || {
                if c.node.kind == Some(SessionKind::Terminal) {
                    "Terminal"
                } else {
                    "New session"
                }
                .to_owned()
            },
            str::to_owned,
        )
}

/// A string that's set and not empty: JavaScript's truthiness for one.
pub(crate) fn set(s: Option<&str>) -> bool {
    s.is_some_and(|s| !s.is_empty())
}
