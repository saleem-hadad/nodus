//! A session's terminal size follows whoever typed last (SRV-144).
//!
//! Each connection says the size its terminal wants when it opens a
//! session and whenever it resizes. The session's terminal takes that size
//! only from the connection it follows: whoever typed into it last, or the
//! first to open it. Typing from a connection whose size differs resizes
//! the terminal to that size first. Every connection with the session open
//! is told the size it has (`ev:pty-size`), so their terminals reflow to it.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Columns and rows.
pub(crate) type Size = (f64, f64);

/// A live session's terminal: its size, and the connection it follows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pty {
    size: Size,
    follows: u64,
}

#[derive(Default)]
struct State {
    /// What each connection's terminal wants, per session it has open.
    wanted: HashMap<(u64, String), Size>,
    ptys: HashMap<String, Pty>,
}

#[derive(Default)]
pub(crate) struct Terminals {
    state: Mutex<State>,
}

/// What a connection opening a session gets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Opened {
    /// The size to open it at: the connection's own, unless someone else's
    /// typing sized it.
    pub size: Size,
    /// The terminal's size changed for everyone else watching.
    pub resized: bool,
}

impl Terminals {
    /// `session:open` from `connection`, whose terminal is `size`.
    pub(crate) fn opened(&self, connection: u64, session_id: &str, size: Size) -> Opened {
        let mut state = self.lock();
        state.wanted.insert((connection, session_id.to_owned()), size);
        match state.ptys.get(session_id) {
            Some(pty) if pty.follows != connection => Opened {
                size: pty.size,
                resized: false,
            },
            current => {
                let resized = current.is_none_or(|pty| pty.size != size);
                state.ptys.insert(
                    session_id.to_owned(),
                    Pty {
                        size,
                        follows: connection,
                    },
                );
                Opened { size, resized }
            }
        }
    }

    /// `session:resize` from `connection`: the size to give the terminal,
    /// when it follows that connection and the size is new.
    pub(crate) fn resized(&self, connection: u64, session_id: &str, size: Size) -> Option<Size> {
        let mut state = self.lock();
        state.wanted.insert((connection, session_id.to_owned()), size);
        match state.ptys.entry(session_id.to_owned()) {
            Entry::Occupied(mut followed) => {
                let pty = followed.get_mut();
                if pty.follows != connection || pty.size == size {
                    return None;
                }
                pty.size = size;
                Some(size)
            }
            // One nobody has sized yet takes this size.
            Entry::Vacant(new) => {
                new.insert(Pty {
                    size,
                    follows: connection,
                });
                Some(size)
            }
        }
    }

    /// `connection` typed into the session: from now on the terminal
    /// follows it, and takes its size first when that differs. A
    /// connection without the session open has no size, and leaves it.
    pub(crate) fn typed(&self, connection: u64, session_id: &str) -> Option<Size> {
        let mut state = self.lock();
        let size = *state.wanted.get(&(connection, session_id.to_owned()))?;
        let pty = state.ptys.entry(session_id.to_owned()).or_insert(Pty {
            size,
            follows: connection,
        });
        pty.follows = connection;
        if pty.size == size {
            return None;
        }
        pty.size = size;
        Some(size)
    }

    /// `session:close` from `connection`: its terminal is gone.
    pub(crate) fn closed(&self, connection: u64, session_id: &str) {
        self.lock().wanted.remove(&(connection, session_id.to_owned()));
    }

    /// The connection has gone.
    pub(crate) fn disconnected(&self, connection: u64) {
        self.lock().wanted.retain(|(of, _), _| *of != connection);
    }

    /// The session's process ended: the next one starts at its opener's size.
    pub(crate) fn exited(&self, session_id: &str) {
        self.lock().ptys.remove(session_id);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Size = (160.0, 48.0);
    const NARROW: Size = (80.0, 24.0);

    #[test]
    fn follows_whoever_typed_last() {
        let terminals = Terminals::default();
        // The first to open sizes it.
        assert_eq!(
            terminals.opened(1, "s1", WIDE),
            Opened {
                size: WIDE,
                resized: true
            }
        );
        // A second viewer gets the size it has.
        assert_eq!(
            terminals.opened(2, "s1", NARROW),
            Opened {
                size: WIDE,
                resized: false
            }
        );
        // Its own resizes don't move it while it follows the first.
        assert_eq!(terminals.resized(2, "s1", (90.0, 30.0)), None);
        assert_eq!(terminals.resized(1, "s1", (150.0, 48.0)), Some((150.0, 48.0)));
        // The second types: its size, once.
        assert_eq!(terminals.typed(2, "s1"), Some((90.0, 30.0)));
        assert_eq!(terminals.typed(2, "s1"), None);
        assert_eq!(terminals.resized(1, "s1", WIDE), None);
        assert_eq!(terminals.resized(2, "s1", NARROW), Some(NARROW));
        // The first types again.
        assert_eq!(terminals.typed(1, "s1"), Some(WIDE));
        // Someone without the terminal open (the prompt bar) leaves it.
        assert_eq!(terminals.typed(3, "s1"), None);

        // A new process starts at its opener's size.
        terminals.exited("s1");
        assert_eq!(terminals.opened(2, "s1", NARROW).size, NARROW);
        // Gone, a connection has no size to give.
        terminals.disconnected(1);
        assert_eq!(terminals.typed(1, "s1"), None);
        terminals.closed(2, "s1");
        assert_eq!(terminals.typed(2, "s1"), None);
    }
}
