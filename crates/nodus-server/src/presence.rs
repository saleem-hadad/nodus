//! Who is on which board (SRV-180): each connection's last presence frame,
//! relayed to the other connections on the same board as `ev:presence`.
//!
//! - A connection is named by its token (its member's login and its
//!   device), never by what its frames say.
//! - A board's relay goes at most every [`PRESENCE_EVERY`]: frames that
//!   come faster are folded into the next one, which carries the latest of
//!   each.
//! - A connection's frames wait in a slot that holds only the newest, so a
//!   slow app never queues stale cursors.
//! - A peer drops out [`PRESENCE_TIMEOUT`] after its last frame, and at
//!   once when it leaves the board or disconnects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use indexmap::IndexMap;
use nodus_domain::{BoardPoint, Event, Presence, PresencePeer, Viewport, presence_color};
use nodus_wire::{Frame, PRESENCE_EVERY, PRESENCE_TIMEOUT, PresenceFrame};
use tokio::sync::watch;
use tokio::time::Instant;

/// How often peers past their timeout are looked for.
const SWEEP_EVERY: Duration = Duration::from_millis(250);

/// Who a connection is, from its token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub login: String,
    pub device: String,
}

/// One connection on a board.
struct Peer {
    project: String,
    who: Identity,
    cursor: Option<BoardPoint>,
    viewport: Option<Viewport>,
    last: Instant,
}

#[derive(Default)]
struct Relay {
    /// When the board's last relay went.
    last: Option<Instant>,
    /// One is on its way.
    scheduled: bool,
}

#[derive(Default)]
struct State {
    /// By connection, in the order they came onto their boards.
    peers: IndexMap<u64, Peer>,
    /// Each connection's slot for its `ev:presence` frames.
    slots: HashMap<u64, watch::Sender<Option<String>>>,
    relays: HashMap<String, Relay>,
}

pub(crate) struct Presences {
    me: Weak<Presences>,
    state: Mutex<State>,
}

impl Presences {
    /// The registry, sweeping out peers past their timeout for as long as
    /// it lives.
    pub(crate) fn start() -> Arc<Self> {
        let presences = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            state: Mutex::new(State::default()),
        });
        let me = Arc::downgrade(&presences);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(SWEEP_EVERY);
            loop {
                every.tick().await;
                let Some(presences) = me.upgrade() else { return };
                presences.sweep();
            }
        });
        presences
    }

    /// A connection that may be on boards: where its `ev:presence` frames
    /// go, the newest only.
    pub(crate) fn connect(&self, connection: u64) -> watch::Receiver<Option<String>> {
        let (slot, frames) = watch::channel(None);
        self.lock().slots.insert(connection, slot);
        frames
    }

    /// A connection's frame: on a board (or another one), moved, or off
    /// every board.
    pub(crate) fn frame(&self, connection: u64, who: Identity, frame: PresenceFrame) {
        let mut state = self.lock();
        if !state.slots.contains_key(&connection) {
            return;
        }
        let now = Instant::now();
        let Some(project) = frame.project else {
            if let Some(gone) = state.peers.shift_remove(&connection) {
                self.relay(&mut state, &gone.project);
            }
            return;
        };
        let moved_from = match state.peers.get_mut(&connection) {
            Some(peer) => {
                let changed = peer.project != project || peer.cursor != frame.cursor || peer.viewport != frame.viewport;
                let left = (peer.project != project).then(|| std::mem::replace(&mut peer.project, project.clone()));
                peer.cursor = frame.cursor;
                peer.viewport = frame.viewport;
                peer.last = now;
                if !changed {
                    // The same again keeps it on the board, and says nothing new.
                    return;
                }
                left
            }
            None => {
                state.peers.insert(
                    connection,
                    Peer {
                        project: project.clone(),
                        who,
                        cursor: frame.cursor,
                        viewport: frame.viewport,
                        last: now,
                    },
                );
                None
            }
        };
        if let Some(left) = moved_from {
            self.relay(&mut state, &left);
        }
        self.relay(&mut state, &project);
    }

    /// The connection has gone: off its board at once.
    pub(crate) fn disconnect(&self, connection: u64) {
        let mut state = self.lock();
        state.slots.remove(&connection);
        if let Some(gone) = state.peers.shift_remove(&connection) {
            self.relay(&mut state, &gone.project);
        }
    }

    /// Takes off every peer whose last frame is older than the timeout.
    fn sweep(&self) {
        let mut state = self.lock();
        let now = Instant::now();
        let mut boards = Vec::new();
        state.peers.retain(|_, peer| {
            let fresh = now.duration_since(peer.last) < PRESENCE_TIMEOUT;
            if !fresh && !boards.contains(&peer.project) {
                boards.push(peer.project.clone());
            }
            fresh
        });
        for project in boards {
            self.relay(&mut state, &project);
        }
    }

    /// Tells everyone on `project` who else is there: now, or once
    /// [`PRESENCE_EVERY`] has passed since the board's last relay.
    fn relay(&self, state: &mut State, project: &str) {
        let now = Instant::now();
        let relay = state.relays.entry(project.to_owned()).or_default();
        if relay.scheduled {
            return;
        }
        let due = relay.last.map_or(now, |last| last + PRESENCE_EVERY);
        if due <= now {
            relay.last = Some(now);
            send(state, project);
            return;
        }
        relay.scheduled = true;
        let (me, project) = (self.me.clone(), project.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep_until(due).await;
            let Some(presences) = me.upgrade() else { return };
            let mut state = presences.lock();
            let relay = state.relays.entry(project.clone()).or_default();
            relay.scheduled = false;
            relay.last = Some(Instant::now());
            send(&mut state, &project);
        });
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Puts each connection on `project` its list of everyone else there. A
/// board nobody is on any more is forgotten.
fn send(state: &mut State, project: &str) {
    let on: Vec<u64> = state
        .peers
        .iter()
        .filter(|(_, peer)| peer.project == project)
        .map(|(connection, _)| *connection)
        .collect();
    if on.is_empty() {
        state.relays.remove(project);
        return;
    }
    for &to in &on {
        let peers = on
            .iter()
            .filter(|&&from| from != to)
            .map(|from| {
                let peer = &state.peers[from];
                PresencePeer {
                    login: peer.who.login.clone(),
                    device: peer.who.device.clone(),
                    color: presence_color(&peer.who.login).to_owned(),
                    cursor: peer.cursor,
                    viewport: peer.viewport,
                }
            })
            .collect();
        let text = Frame::Event(Event::Presence(Presence {
            project: project.to_owned(),
            peers,
        }))
        .to_text();
        if let Some(slot) = state.slots.get(&to) {
            slot.send_replace(Some(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(login: &str) -> Identity {
        Identity {
            login: login.into(),
            device: format!("{login}-laptop"),
        }
    }

    fn on(project: &str, x: f64) -> PresenceFrame {
        PresenceFrame {
            project: Some(project.into()),
            cursor: Some(BoardPoint { x, y: 0.0 }),
            viewport: None,
        }
    }

    /// Past the next relay.
    async fn settle() {
        tokio::time::sleep(PRESENCE_EVERY + Duration::from_millis(10)).await;
    }

    /// What the slot holds, as the presence it says.
    fn told(frames: &mut watch::Receiver<Option<String>>) -> Option<Presence> {
        let text = frames.borrow_and_update().clone()?;
        match Frame::parse(&text).unwrap() {
            Frame::Event(Event::Presence(presence)) => Some(presence),
            other => panic!("not presence: {other:?}"),
        }
    }

    fn logins(presence: &Presence) -> Vec<&str> {
        presence.peers.iter().map(|p| p.login.as_str()).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn each_hears_the_others_on_its_board_only() {
        let presences = Presences::start();
        let mut alice = presences.connect(1);
        let mut octo = presences.connect(2);
        let mut elsewhere = presences.connect(3);
        presences.frame(1, who("alice"), on("p1", 1.0));
        settle().await;
        presences.frame(2, who("octo"), on("p1", 2.0));
        presences.frame(3, who("bob"), on("p2", 3.0));
        settle().await;
        assert_eq!(logins(&told(&mut alice).unwrap()), ["octo"]);
        let heard = told(&mut octo).unwrap();
        assert_eq!(logins(&heard), ["alice"]);
        assert_eq!(heard.peers[0].cursor, Some(BoardPoint { x: 1.0, y: 0.0 }));
        assert_eq!(heard.peers[0].color, presence_color("alice"));
        assert!(told(&mut elsewhere).unwrap().peers.is_empty());

        // Off the board: the others hear at once.
        presences.frame(
            1,
            who("alice"),
            PresenceFrame {
                project: None,
                cursor: None,
                viewport: None,
            },
        );
        settle().await;
        assert!(told(&mut octo).unwrap().peers.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn relays_a_board_at_most_every_50ms() {
        let presences = Presences::start();
        let mut octo = presences.connect(2);
        presences.frame(2, who("octo"), on("p1", 0.0));
        presences.connect(1);
        let mut relays = 0;
        for i in 0..100 {
            presences.frame(1, who("alice"), on("p1", f64::from(i)));
            tokio::time::sleep(Duration::from_millis(5)).await;
            if octo.has_changed().unwrap() {
                relays += 1;
                octo.borrow_and_update();
            }
        }
        // 500 ms of frames every 5 ms: about ten relays, not a hundred.
        assert!((9..=12).contains(&relays), "{relays} relays");
        settle().await;
        let last = told(&mut octo).unwrap();
        assert_eq!(last.peers[0].cursor, Some(BoardPoint { x: 99.0, y: 0.0 }));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_peer_drops_out_after_10_seconds() {
        let presences = Presences::start();
        let mut alice = presences.connect(1);
        presences.connect(2);
        presences.frame(1, who("alice"), on("p1", 0.0));
        presences.frame(2, who("octo"), on("p1", 0.0));
        tokio::time::sleep(Duration::from_secs(5)).await;
        // The same frame again keeps Octo there, and says nothing new.
        presences.frame(2, who("octo"), on("p1", 0.0));
        tokio::time::sleep(Duration::from_secs(4)).await;
        presences.frame(1, who("alice"), on("p1", 1.0));
        tokio::time::sleep(Duration::from_millis(5500)).await;
        // 14.5 s in, Octo's last frame was 9.5 s ago.
        assert_eq!(logins(&told(&mut alice).unwrap()), ["octo"]);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(told(&mut alice).unwrap().peers.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_disconnect_takes_it_off_at_once() {
        let presences = Presences::start();
        let mut octo = presences.connect(2);
        presences.connect(1);
        presences.frame(2, who("octo"), on("p1", 0.0));
        presences.frame(1, who("alice"), on("p1", 0.0));
        settle().await;
        assert_eq!(logins(&told(&mut octo).unwrap()), ["alice"]);
        presences.disconnect(1);
        settle().await;
        assert!(told(&mut octo).unwrap().peers.is_empty());
        // Frames from a connection that has gone are nobody's.
        presences.frame(1, who("alice"), on("p1", 5.0));
        settle().await;
        assert!(!octo.has_changed().unwrap());
    }
}
