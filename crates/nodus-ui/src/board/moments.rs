//! What just happened on the board that motion marks (spec 15): something
//! new arriving (MOTION-010), Claude drawing (MOTION-020…MOTION-025), a
//! session stopping to ask (MOTION-013), a long run finishing (MOTION-031)
//! and the news running up a lineage edge (MOTION-035). The canvas shows it
//! each board it rebuilds from; the nodes, links, edges and the pen ask it
//! where each thing is in its moment.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nodus_board::geometry::Point;
use nodus_domain::{BoardSnapshot, Rect, SessionStatus, ShapeKind};

use super::cards::present;

/// A pop: scale and fade in (MOTION-010).
pub const POP_MS: f64 = 300.0;
/// The raise of a session that stopped to ask (MOTION-013).
pub const LIFT_MS: f64 = 560.0;
/// How high it raises, in board units.
pub const LIFT_UP: f32 = 6.0;
/// The ring round a card whose long run finished (MOTION-031).
pub const RING_MS: f64 = 1700.0;
/// The pulse up a lineage edge (MOTION-035): along it, then a ring at the parent.
pub const PULSE_TRAVEL_MS: f64 = 900.0;
pub const PULSE_MS: f64 = 1250.0;
/// A run this long gets the ring when it finishes (MOTION-031).
const LONG_RUN: Duration = Duration::from_secs(30);
/// What Claude made this recently is drawn; older (⌘Z bringing it back, a
/// board opened later) pops like anything else (MOTION-025).
const FRESH_MS: f64 = 15_000.0;
/// More than this many things from Claude at once arrive as the user's do
/// (MOTION-025), and more than this many arriving at once just appear
/// (MOTION-010): a paste or ⌘Z of a crowd shouldn't fade in dozens of layers.
const MAX_DRAWN: usize = 40;
const MAX_POPS: usize = 40;
/// A batch drawn in about this long, at most (MOTION-020).
const BUDGET_MS: f64 = 4000.0;
/// The quickest a batch is drawn, as a share of its own pace.
const FASTEST: f64 = 0.18;
/// The pen's way from the session's card to the first thing, and between things.
const FIRST_TRAVEL_MS: f64 = 420.0;
const TRAVEL_MS: f64 = 300.0;
/// The pen stays after the last thing, then fades.
const LINGER_MS: f64 = 350.0;
const PEN_FADE_MS: f64 = 300.0;

/// What Claude draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum What {
    Shape(ShapeKind),
    Link,
    Note,
    View,
}

/// Where something is in its moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// Claude will draw it after what comes before it: not shown yet.
    Waiting,
    /// Claude is drawing it, 0…1 (linear) through it.
    Drawing(f32),
    /// Popping in, 0…1 (linear) through it.
    Pop(f32),
}

/// One thing in a drawing: when the pen travels to it and when it's drawn,
/// in ms from the drawing's start.
#[derive(Debug, Clone)]
pub struct Step {
    pub id: String,
    pub what: What,
    pub travel: (f64, f64),
    pub draw: (f64, f64),
}

/// What one session is drawing, and its pen.
#[derive(Debug, Clone)]
pub struct Drawing {
    pub session: String,
    /// What the pen's label says: the session's title.
    pub title: String,
    /// Where the pen sets off: the session's card, if it's on the board.
    pub origin: Option<Point>,
    pub start: Instant,
    pub steps: Vec<Step>,
}

/// Where a drawing's pen is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PenAt {
    /// On its way to step `to`, from the step before it (or the origin),
    /// 0…1 (eased) of the way.
    Travel { to: usize, p: f32 },
    /// Drawing step `at`, 0…1 through it.
    Draw { at: usize, p: f32 },
    /// Done: resting at the last step's end.
    Rest,
}

impl Drawing {
    fn end_ms(&self) -> f64 {
        self.steps.last().map_or(0.0, |s| s.draw.1) + LINGER_MS + PEN_FADE_MS
    }

    fn ms(&self, now: Instant) -> f64 {
        since(self.start, now)
    }

    /// The pen this frame, and how much of it shows.
    pub fn pen(&self, now: Instant) -> (PenAt, f32) {
        let ms = self.ms(now);
        let last = self.steps.last().map_or(0.0, |s| s.draw.1);
        let opacity = if ms < 200.0 {
            (ms / 200.0) as f32
        } else if ms > last + LINGER_MS {
            (1.0 - (ms - last - LINGER_MS) / PEN_FADE_MS).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        for (i, s) in self.steps.iter().enumerate() {
            if ms < s.travel.1 {
                let span = (s.travel.1 - s.travel.0).max(1.0);
                let p = ((ms - s.travel.0) / span).clamp(0.0, 1.0) as f32;
                return (
                    PenAt::Travel {
                        to: i,
                        p: crate::theme::motion::ease_in_out(p),
                    },
                    opacity,
                );
            }
            if ms < s.draw.1 {
                let span = (s.draw.1 - s.draw.0).max(1.0);
                let p = ((ms - s.draw.0) / span).clamp(0.0, 1.0) as f32;
                return (PenAt::Draw { at: i, p }, opacity);
            }
        }
        (PenAt::Rest, opacity)
    }
}

/// How long Claude takes to draw one thing, at its own pace.
fn draw_ms(what: What, text: &str) -> f64 {
    match what {
        What::Shape(ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Diamond) => 680.0,
        What::Shape(ShapeKind::Text) => (40.0 * text.chars().count() as f64).clamp(260.0, 720.0),
        What::Shape(ShapeKind::Sticker) => 420.0,
        What::Shape(ShapeKind::Image) => 480.0,
        What::Link | What::Note | What::View => 520.0,
    }
}

/// A session's run: its status, and since when it's been running.
#[derive(Debug, Clone, Copy)]
struct Run {
    status: SessionStatus,
    since: Option<Instant>,
}

/// Something new from Claude, waiting to be put in its session's drawing.
struct New {
    id: String,
    what: What,
    created_at: f64,
    text: String,
}

/// The board's moments.
#[derive(Default)]
pub struct Moments {
    /// Seen a board yet: what's on the first one was already there.
    seeded: bool,
    known: HashSet<String>,
    pops: HashMap<String, Instant>,
    /// Made by hand: no pop when it comes in (it may come in later, from the store).
    hushed: HashSet<String>,
    runs: HashMap<String, Run>,
    lifts: HashMap<String, Instant>,
    rings: HashMap<String, Instant>,
    pulses: HashMap<String, Instant>,
    pub drawings: Vec<Drawing>,
}

impl Moments {
    /// Forgets the board: the next one's things were already there.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes in the board as rebuilt. `now_ms` is the wall clock things are
    /// stamped with; `still` (parity shots, reduced motion) notes what's
    /// there without marking anything; `rect_of` finds a node's box.
    pub fn observe(
        &mut self,
        board: &BoardSnapshot,
        now: Instant,
        now_ms: f64,
        still: bool,
        rect_of: &dyn Fn(&str) -> Option<Rect>,
    ) {
        let mut ids: HashSet<String> = HashSet::new();
        ids.extend(board.cards.iter().map(|c| c.node.session_id.clone()));
        ids.extend(board.shapes.iter().map(|s| s.id.clone()));
        ids.extend(board.links.iter().map(|l| l.id.clone()));
        ids.extend(board.notes.iter().map(|n| n.id.clone()));
        ids.extend(board.views.iter().map(|v| v.id.clone()));
        let first = !self.seeded;
        self.seeded = true;
        if !first && !still {
            self.arrivals(board, &ids, now, now_ms, rect_of);
        }
        self.known = ids;
        self.statuses(board, now, first || still);
    }

    fn arrivals(
        &mut self,
        board: &BoardSnapshot,
        ids: &HashSet<String>,
        now: Instant,
        now_ms: f64,
        rect_of: &dyn Fn(&str) -> Option<Rect>,
    ) {
        let fresh = |created_at: f64| (now_ms - created_at).abs() < FRESH_MS;
        let mut batches: Vec<(String, Vec<New>)> = Vec::new();
        let mut add = |session: &str, new: New| match batches.iter_mut().find(|(s, _)| s == session) {
            Some((_, items)) => items.push(new),
            None => batches.push((session.to_owned(), vec![new])),
        };
        let mut pops: Vec<String> = Vec::new();
        let is_new = |id: &str| ids.contains(id) && !self.known.contains(id);
        for s in board.shapes.iter().filter(|s| is_new(&s.id)) {
            match &s.by {
                Some(by) if fresh(s.created_at) => add(
                    by,
                    New {
                        id: s.id.clone(),
                        what: What::Shape(s.kind),
                        created_at: s.created_at,
                        text: s.text.clone(),
                    },
                ),
                _ => pops.push(s.id.clone()),
            }
        }
        // A link the user drew is drawn by their hand; one Claude drew is drawn.
        for l in board.links.iter().filter(|l| is_new(&l.id)) {
            if let Some(by) = &l.by
                && fresh(l.created_at)
            {
                add(
                    by,
                    New {
                        id: l.id.clone(),
                        what: What::Link,
                        created_at: l.created_at,
                        text: String::new(),
                    },
                );
            }
        }
        for n in board.notes.iter().filter(|n| is_new(&n.id)) {
            match &n.from {
                Some(from) if fresh(n.created_at) => add(
                    from,
                    New {
                        id: n.id.clone(),
                        what: What::Note,
                        created_at: n.created_at,
                        text: String::new(),
                    },
                ),
                _ => pops.push(n.id.clone()),
            }
        }
        for v in board.views.iter().filter(|v| is_new(&v.id)) {
            match &v.from {
                Some(from) if fresh(v.created_at) => add(
                    from,
                    New {
                        id: v.id.clone(),
                        what: What::View,
                        created_at: v.created_at,
                        text: String::new(),
                    },
                ),
                // A live page can't scale with its card (D3): one pops only before it has a page.
                _ if v.version == 0 => pops.push(v.id.clone()),
                _ => {}
            }
        }
        for c in board
            .cards
            .iter()
            .filter(|c| !c.node.archived && is_new(&c.node.session_id))
        {
            pops.push(c.node.session_id.clone());
        }
        for (session, mut items) in batches {
            if items.len() > MAX_DRAWN {
                pops.extend(items.into_iter().filter(|n| n.what != What::Link).map(|n| n.id));
                continue;
            }
            // Oldest first; things before the links that join them.
            items.sort_by(|a, b| {
                a.created_at
                    .partial_cmp(&b.created_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then((a.what == What::Link).cmp(&(b.what == What::Link)))
            });
            let card = board.cards.iter().find(|c| c.node.session_id == session);
            let title = card.map_or_else(
                || {
                    board
                        .notes
                        .iter()
                        .find(|n| n.from.as_deref() == Some(session.as_str()))
                        .and_then(|n| n.from_title.clone())
                        .unwrap_or_else(|| "Claude".to_owned())
                },
                present::title,
            );
            let origin = card
                .filter(|c| !c.node.archived)
                .and_then(|c| rect_of(&c.node.session_id))
                .map(|r| Point {
                    x: r.x + 24.0,
                    y: r.y + 20.0,
                });
            self.draw(&session, title, origin, items, now);
        }
        let crowd = pops.len() > MAX_POPS;
        for id in pops {
            if !self.hushed.remove(&id) && !crowd {
                self.pops.insert(id, now);
            }
        }
    }

    /// Puts things in `session`'s drawing: after what it's drawing now, or
    /// in a new one from its card.
    fn draw(&mut self, session: &str, title: String, origin: Option<Point>, items: Vec<New>, now: Instant) {
        let pace: f64 = items
            .iter()
            .enumerate()
            .map(|(i, n)| if i == 0 { FIRST_TRAVEL_MS } else { TRAVEL_MS } + draw_ms(n.what, &n.text))
            .sum();
        let k = (BUDGET_MS / pace.max(1.0)).clamp(FASTEST, 1.0);
        let running = self
            .drawings
            .iter()
            .position(|d| d.session == session && d.ms(now) < d.end_ms());
        let (index, mut at, mut first) = match running {
            Some(i) => {
                let d = &self.drawings[i];
                let last = d.steps.last().map_or(0.0, |s| s.draw.1);
                (i, last.max(d.ms(now)), false)
            }
            None => {
                self.drawings.retain(|d| d.session != session);
                self.drawings.push(Drawing {
                    session: session.to_owned(),
                    title,
                    origin,
                    start: now,
                    steps: Vec::new(),
                });
                (self.drawings.len() - 1, 0.0, true)
            }
        };
        let drawing = &mut self.drawings[index];
        for n in items {
            let travel = k * if first { FIRST_TRAVEL_MS } else { TRAVEL_MS };
            first = false;
            let draw = k * draw_ms(n.what, &n.text);
            drawing.steps.push(Step {
                id: n.id,
                what: n.what,
                travel: (at, at + travel),
                draw: (at + travel, at + travel + draw),
            });
            at += travel + draw;
        }
    }

    fn statuses(&mut self, board: &BoardSnapshot, now: Instant, still: bool) {
        let mut seen = HashSet::new();
        for card in board.cards.iter().filter(|c| !c.node.archived) {
            let id = &card.node.session_id;
            seen.insert(id.clone());
            let status = card.status;
            let Some(run) = self.runs.get(id).copied() else {
                let since = (status != SessionStatus::Done).then_some(now);
                self.runs.insert(id.clone(), Run { status, since });
                continue;
            };
            if run.status == status {
                continue;
            }
            let ran = run.since.is_some_and(|s| now.duration_since(s) >= LONG_RUN);
            if !still {
                match (run.status, status) {
                    (SessionStatus::Working, SessionStatus::Waiting) => {
                        self.lifts.insert(id.clone(), now);
                    }
                    (_, SessionStatus::Done) => {
                        if ran && !present::is_expanded(card) {
                            self.rings.insert(id.clone(), now);
                        }
                        self.pulses.insert(id.clone(), now);
                    }
                    _ => {}
                }
            }
            let since = match status {
                SessionStatus::Done => None,
                _ => run.since.or(Some(now)),
            };
            self.runs.insert(id.clone(), Run { status, since });
        }
        self.runs.retain(|id, _| seen.contains(id));
    }

    /// Something the user is making by hand, which shows as they make it:
    /// no pop (a shape drawn corner to corner, a sheet dragged off the pad).
    pub fn hush(&mut self, id: &str) {
        if self.pops.remove(id).is_none() && !self.known.contains(id) {
            self.hushed.insert(id.to_owned());
        }
    }

    /// Drops what's over.
    pub fn tick(&mut self, now: Instant) {
        let within = |at: &Instant, ms: f64| since(*at, now) < ms;
        self.pops.retain(|_, at| within(at, POP_MS));
        self.lifts.retain(|_, at| within(at, LIFT_MS));
        self.rings.retain(|_, at| within(at, RING_MS));
        self.pulses.retain(|_, at| within(at, PULSE_MS));
        self.drawings.retain(|d| d.ms(now) < d.end_ms());
    }

    /// Something is moving: the board wants the next frame.
    pub fn active(&self) -> bool {
        !(self.pops.is_empty()
            && self.lifts.is_empty()
            && self.rings.is_empty()
            && self.pulses.is_empty()
            && self.drawings.is_empty())
    }

    /// Where `id` is in its arrival; None once it's simply there.
    pub fn arrival(&self, id: &str, now: Instant) -> Option<Arrival> {
        for d in &self.drawings {
            if let Some(s) = d.steps.iter().find(|s| s.id == id) {
                let ms = d.ms(now);
                if ms < s.draw.0 {
                    return Some(Arrival::Waiting);
                }
                if ms < s.draw.1 {
                    let p = (ms - s.draw.0) / (s.draw.1 - s.draw.0).max(1.0);
                    return Some(Arrival::Drawing(p as f32));
                }
                return None;
            }
        }
        let at = self.pops.get(id)?;
        let p = since(*at, now) / POP_MS;
        (p < 1.0).then_some(Arrival::Pop(p as f32))
    }

    /// How far a card is raised (board units, up negative): a quick raise,
    /// then down on the spring (MOTION-013).
    pub fn lift(&self, id: &str, now: Instant) -> f32 {
        let Some(at) = self.lifts.get(id) else { return 0.0 };
        let t = (since(*at, now) / LIFT_MS) as f32;
        lift_at(t)
    }

    /// The ring round a card whose long run just finished: 0…1 through it.
    pub fn ring(&self, id: &str, now: Instant) -> Option<f32> {
        let at = self.rings.get(id)?;
        let t = since(*at, now) / RING_MS;
        (t < 1.0).then_some(t as f32)
    }

    /// The pulse from a session that just finished up its lineage edges, in
    /// ms since it set off.
    pub fn pulse(&self, child: &str, now: Instant) -> Option<f64> {
        let at = self.pulses.get(child)?;
        let ms = since(*at, now);
        (ms < PULSE_MS).then_some(ms)
    }
}

/// Milliseconds from `at` to `now` on the moments' clock.
pub fn since(at: Instant, now: Instant) -> f64 {
    now.saturating_duration_since(at).as_secs_f64() * 1000.0
}

/// The lift at `t` (0…1): up in the first fifth on `--ease-out`, then back
/// down on `--ease-spring`, which overshoots below rest and settles.
pub fn lift_at(t: f32) -> f32 {
    use crate::theme::motion::{ease_out, ease_spring};
    if t >= 1.0 {
        0.0
    } else if t < 0.2 {
        -LIFT_UP * ease_out(t / 0.2)
    } else {
        -LIFT_UP * (1.0 - ease_spring((t - 0.2) / 0.8))
    }
}

/// The ring's turn (degrees) and opacity at `t` (0…1): once round, in over
/// the first 15%, out over the last 35%.
pub fn ring_at(t: f32) -> (f64, f32) {
    use crate::theme::motion::{ease_in_out, ease_out};
    let angle = 360.0 * f64::from(ease_in_out(t));
    let opacity = if t < 0.15 {
        ease_out(t / 0.15)
    } else if t > 0.65 {
        1.0 - ease_in_out((t - 0.65) / 0.35)
    } else {
        1.0
    };
    (angle, opacity)
}

/// A pop at `t` (0…1): scale (on the spring) and opacity.
pub fn pop_at(t: f32) -> (f32, f32) {
    use crate::theme::motion::{ease_out, ease_spring};
    (0.9 + 0.1 * ease_spring(t), ease_out((t / 0.6).min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardShape, InkColor};

    fn shape(id: &str, by: Option<&str>, created_at: f64) -> BoardShape {
        BoardShape {
            id: id.into(),
            kind: ShapeKind::Rect,
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 60.0,
            color: InkColor::Red,
            text: String::new(),
            image: None,
            by: by.map(str::to_owned),
            created_at,
            extra: serde_json::Map::new(),
        }
    }

    fn board(shapes: Vec<BoardShape>) -> BoardSnapshot {
        BoardSnapshot {
            project: nodus_domain::Project {
                id: "p".into(),
                name: "p".into(),
                repo_path: "/r".into(),
                color: nodus_domain::FolderColor::Blue,
                created_at: 0.0,
                opened_at: None,
                group_id: None,
                actions: None,
                focused: None,
                archived: None,
                server: None,
                extra: serde_json::Map::new(),
            },
            cards: vec![],
            archive: Rect::default(),
            sections: vec![],
            notes: vec![],
            views: vec![],
            ink: vec![],
            shapes,
            links: vec![],
            groups: vec![],
            viewport: None,
            offline: false,
        }
    }

    #[test]
    fn what_was_there_first_doesnt_arrive() {
        let mut m = Moments::default();
        let now = Instant::now();
        m.observe(&board(vec![shape("a", Some("s"), 1000.0)]), now, 1000.0, false, &|_| {
            None
        });
        assert_eq!(m.arrival("a", now), None);
        assert!(!m.active());
    }

    #[test]
    fn claude_draws_in_order_and_the_user_pops() {
        let mut m = Moments::default();
        let now = Instant::now();
        m.observe(&board(vec![]), now, 1000.0, false, &|_| None);
        let next = board(vec![
            shape("one", Some("s"), 1000.0),
            shape("two", Some("s"), 1001.0),
            shape("mine", None, 1001.0),
            shape("old", Some("s"), -20_000.0),
        ]);
        m.observe(&next, now, 1002.0, false, &|_| None);
        assert_eq!(m.drawings.len(), 1);
        assert_eq!(m.arrival("one", now), Some(Arrival::Waiting));
        assert_eq!(m.arrival("two", now), Some(Arrival::Waiting));
        assert_eq!(m.arrival("mine", now), Some(Arrival::Pop(0.0)));
        // ⌘Z bringing back something Claude drew long ago pops.
        assert_eq!(m.arrival("old", now), Some(Arrival::Pop(0.0)));
        let d = &m.drawings[0];
        assert!(d.steps[0].draw.1 <= d.steps[1].travel.0);
        let mid = now + Duration::from_millis((d.steps[0].draw.0 + 10.0) as u64);
        assert!(matches!(m.arrival("one", mid), Some(Arrival::Drawing(_))));
        assert_eq!(m.arrival("two", mid), Some(Arrival::Waiting));
        let done = now + Duration::from_secs(10);
        m.tick(done);
        assert!(!m.active());
        assert_eq!(m.arrival("one", done), None);
    }

    #[test]
    fn a_big_drawing_keeps_to_its_budget() {
        let mut m = Moments::default();
        let now = Instant::now();
        m.observe(&board(vec![]), now, 0.0, false, &|_| None);
        let many: Vec<BoardShape> = (0..30)
            .map(|i| shape(&format!("s{i}"), Some("s"), f64::from(i)))
            .collect();
        m.observe(&board(many), now, 30.0, false, &|_| None);
        // 30 × (300 + 680) + 120 at its own pace; drawn at the fastest.
        let end = m.drawings[0].steps.last().map(|s| s.draw.1).unwrap_or_default();
        assert!((end - 29_520.0 * FASTEST).abs() < 1.0, "{end}");
        let too_many: Vec<BoardShape> = (0..50).map(|i| shape(&format!("t{i}"), Some("u"), 40.0)).collect();
        m.observe(&board(too_many), now, 40.0, false, &|_| None);
        assert!(m.drawings.iter().all(|d| d.session != "u"));
        // Fifty at once is a crowd: they just appear.
        assert_eq!(m.arrival("t0", now), None);
    }

    #[test]
    fn still_boards_mark_nothing() {
        let mut m = Moments::default();
        let now = Instant::now();
        m.observe(&board(vec![]), now, 0.0, true, &|_| None);
        m.observe(&board(vec![shape("a", Some("s"), 0.0)]), now, 0.0, true, &|_| None);
        assert!(!m.active());
    }

    #[test]
    fn lifts_and_settles() {
        assert_eq!(lift_at(0.0), 0.0);
        assert!((lift_at(0.2) + LIFT_UP).abs() < 1e-4);
        assert!(
            (0..100).any(|i| lift_at(0.2 + i as f32 * 0.008) > 0.0),
            "the spring overshoots"
        );
        assert_eq!(lift_at(1.0), 0.0);
        let (a, o) = ring_at(1.0);
        assert!((a - 360.0).abs() < 1e-6 && o.abs() < 1e-5);
        assert_eq!(pop_at(1.0), (1.0, 1.0));
    }
}
