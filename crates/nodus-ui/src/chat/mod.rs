//! The chat view of a Claude session (spec 16, CHAT-…): the conversation read
//! from the session's transcript, the charts, forms and pages Claude shows in
//! it, a composer that sends, queues or interrupts, and the session's
//! terminal docked under the chat while Claude asks something only the
//! terminal can show. Claude cards show this in the session window and when
//! expanded, on a local board and a server's alike; a shell card keeps its
//! terminal ([`shows_chat`]).

mod composer;
mod entries;
pub mod form;
mod look;
mod shows;
mod slash;
mod syntax;
mod view;
pub mod visual;

/// The theme's purple, for merged pull requests in cards' drawers too (DRAWER-033).
pub(crate) use look::purple;
pub use view::{ChatView, ChatViews};

/// CHAT-029: whether a card of `kind` on `project_id`'s board shows its
/// session as a chat, in the session window and the expanded card: a Claude
/// card does, on a local board and on a server's (SRV-166) unless that
/// server is too old to read transcripts out; a shell card keeps its
/// terminal. The one check for every place a card's body is chosen.
pub fn shows_chat(nodus: &nodus_core::Nodus, project_id: &str, kind: nodus_domain::SessionKind) -> bool {
    kind == nodus_domain::SessionKind::Claude && nodus.sessions().has_chat(project_id)
}
