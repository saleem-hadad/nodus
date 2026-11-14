//! The chat view of a Claude session (spec 16, CHAT-…): the conversation read
//! from the session's transcript, the charts, forms and pages Claude shows in
//! it, a composer that sends, queues or interrupts, and the session's
//! terminal docked under the chat while Claude asks something only the
//! terminal can show. Claude cards show this in the session window and when
//! expanded; a shell card keeps its terminal.

mod composer;
mod entries;
pub mod form;
mod look;
mod shows;
mod slash;
mod syntax;
mod view;
pub mod visual;

pub use view::{ChatView, ChatViews};
