//! Others on a server board (SRV-180…182): who's there, where their
//! pointers are, and following one of them.
//!
//! - [`Peers`] holds what the server last said of the board on show, each
//!   pointer gliding from where it was drawn to where it is now, the
//!   avatars of the people there, and whom this window follows.
//! - The canvas tells the server where this person is: at most every 50 ms,
//!   only when something changed, and the same again every 4 s so someone
//!   reading quietly stays on the board ([`BoardCanvas::share_presence`]).
//! - A pointer moving draws the canvas again each frame until it arrives,
//!   but only the canvas: what's notified is [`Peers`], which no node draws
//!   from, so the nodes keep their drawings (`node_view::partial_frame`).
//! - Following someone flies this viewport to theirs at each update, until
//!   this person pans, zooms or presses Esc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PathBuilder, StatefulInteractiveElement, Styled, Task, Window, canvas, div, point, px,
};
use nodus_core::Nodus;
use nodus_core::PRESENCE_EVERY;
use nodus_domain::{BoardPoint, Presence, Viewport};

use super::canvas::BoardCanvas;
use super::hud::rgba_mix;
use super::viewport;
use crate::components::frosted::frosted;
use crate::components::tooltip::native_tooltip;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// How long a pointer takes to glide to where the server last said it is:
/// a little over the 50 ms between updates, so a moving pointer never stops.
pub const GLIDE: Duration = Duration::from_millis(90);
/// How often an idle viewer says it's still there; the server drops a peer
/// after 10 s without a frame.
const KEEPALIVE: Duration = Duration::from_secs(4);
/// How long the members are trusted for avatars before an unknown login
/// asks for them again.
const ASK_AGAIN: Duration = Duration::from_secs(60);
/// How long following takes to catch up with each move.
const FOLLOW_MS: f64 = 120.0;
/// Avatars the title bar shows before `+N`.
const AVATARS_SHOWN: usize = 5;

/// A pointer on its way from where it was drawn to where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Glide {
    from: BoardPoint,
    to: BoardPoint,
    since: Instant,
}

impl Glide {
    /// Where it's drawn at `now`, and whether it's still on its way.
    fn at(&self, now: Instant) -> (BoardPoint, bool) {
        let t = now.saturating_duration_since(self.since).as_secs_f64() / GLIDE.as_secs_f64();
        if t >= 1.0 {
            return (self.to, false);
        }
        // Ease out: quick to start, settling at the end.
        let e = 1.0 - (1.0 - t).powi(3);
        let mix = |a: f64, b: f64| a + (b - a) * e;
        (
            BoardPoint {
                x: mix(self.from.x, self.to.x),
                y: mix(self.from.y, self.to.y),
            },
            true,
        )
    }
}

/// Someone else on the board.
#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub login: String,
    pub device: String,
    pub color: Hsla,
    /// What they look at: the flow point at the centre of their canvas, and
    /// their zoom.
    pub view: Option<Viewport>,
    cursor: Option<Glide>,
}

impl Peer {
    /// Their pointer at `now`, and whether it's still gliding there.
    fn cursor_at(&self, now: Instant) -> Option<(BoardPoint, bool)> {
        self.cursor.map(|glide| glide.at(now))
    }
}

/// Whom this window follows (SRV-182).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Following {
    pub project: String,
    pub login: String,
    pub device: String,
}

/// Who's on the board on show, as the server last said.
pub struct Peers {
    nodus: Nodus,
    boards: HashMap<String, Vec<Peer>>,
    /// Avatars by login: none for someone without one, or not found yet.
    avatars: HashMap<String, Option<PathBuf>>,
    /// When each server's members were last asked for, for their avatars.
    asked: HashMap<String, Instant>,
    following: Option<Following>,
}

impl Peers {
    pub fn new(nodus: Nodus) -> Self {
        Self {
            nodus,
            boards: HashMap::new(),
            avatars: HashMap::new(),
            asked: HashMap::new(),
            following: None,
        }
    }

    /// Everyone else on a board, each device once, in the order they came.
    pub fn on_board(&self, project: &str) -> &[Peer] {
        self.boards.get(project).map_or(&[], Vec::as_slice)
    }

    /// Everyone else on a board, each person once (their first device).
    pub fn people(&self, project: &str) -> Vec<&Peer> {
        let mut people: Vec<&Peer> = Vec::new();
        for peer in self.on_board(project) {
            if !people.iter().any(|p| p.login == peer.login) {
                people.push(peer);
            }
        }
        people
    }

    pub fn avatar(&self, login: &str) -> Option<PathBuf> {
        self.avatars.get(login).cloned().flatten()
    }

    /// Whom this window follows on `project`.
    pub fn following(&self, project: &str) -> Option<&Following> {
        self.following.as_ref().filter(|f| f.project == project)
    }

    /// Follows someone on `project`, or stops when it already did.
    pub fn toggle_follow(&mut self, project: &str, peer: &Peer, cx: &mut Context<'_, Self>) {
        let same = self
            .following(project)
            .is_some_and(|f| f.login == peer.login && f.device == peer.device);
        self.following = (!same).then(|| Following {
            project: project.to_owned(),
            login: peer.login.clone(),
            device: peer.device.clone(),
        });
        cx.notify();
    }

    /// Stops following; whether it was.
    pub fn unfollow(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let was = self.following.take().is_some();
        if was {
            cx.notify();
        }
        was
    }

    /// The server's `ev:presence` for a board on show, from `server`.
    pub fn heard(&mut self, presence: Presence, server: Option<&str>, cx: &mut Context<'_, Self>) {
        let now = Instant::now();
        let before = self.boards.remove(&presence.project).unwrap_or_default();
        let peers: Vec<Peer> = presence
            .peers
            .into_iter()
            .map(|peer| {
                let was = before.iter().find(|p| p.login == peer.login && p.device == peer.device);
                // From where it's drawn now, unless it has just come onto the board.
                let cursor = peer.cursor.map(|to| match was.and_then(|p| p.cursor) {
                    Some(glide) if glide.to == to => glide,
                    Some(glide) => Glide {
                        from: glide.at(now).0,
                        to,
                        since: now,
                    },
                    None => Glide {
                        from: to,
                        to,
                        since: now,
                    },
                });
                Peer {
                    color: color_of(&peer.color),
                    login: peer.login,
                    device: peer.device,
                    view: peer.viewport,
                    cursor,
                }
            })
            .collect();
        // The one followed left.
        if self
            .following(&presence.project)
            .is_some_and(|f| !peers.iter().any(|p| p.login == f.login && p.device == f.device))
        {
            self.following = None;
        }
        if let Some(server) = server {
            self.want_avatars(server, peers.iter().map(|p| p.login.as_str()), cx);
        }
        self.boards.insert(presence.project, peers);
        cx.notify();
    }

    /// The boards left, or whose server went away: nobody there any more.
    pub fn forget(&mut self, projects: &[String], cx: &mut Context<'_, Self>) {
        let mut changed = false;
        for project in projects {
            changed |= self.boards.remove(project).is_some();
        }
        if self.following.as_ref().is_some_and(|f| projects.contains(&f.project)) {
            self.following = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Fetches the avatars of logins it hasn't seen, through the server's
    /// members and the core's avatar cache (SRV-045), as Settings → Servers
    /// shows them: for the people on a board, and the senders in a
    /// transcript (SRV-142).
    pub fn want_avatars<'a>(
        &mut self,
        server: &str,
        logins: impl IntoIterator<Item = &'a str>,
        cx: &mut Context<'_, Self>,
    ) {
        let unknown = logins.into_iter().any(|login| !self.avatars.contains_key(login));
        let recent = self.asked.get(server).is_some_and(|at| at.elapsed() < ASK_AGAIN);
        if !unknown || recent {
            return;
        }
        self.asked.insert(server.to_owned(), Instant::now());
        let (nodus, server) = (self.nodus.clone(), server.to_owned());
        cx.spawn(async move |this, cx| {
            let Ok(members) = nodus.servers().members(&server).await else {
                return;
            };
            for member in members {
                let file = match &member.avatar_url {
                    Some(url) => nodus.servers().avatar(url).await,
                    None => None,
                };
                let ok = this.update(cx, |peers, cx| {
                    peers.avatars.insert(member.login, file);
                    cx.notify();
                });
                if ok.is_err() {
                    return;
                }
            }
        })
        .detach();
    }
}

/// `#rrggbb` as a colour; grey when it isn't one.
fn color_of(hex: &str) -> Hsla {
    hex.strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .map_or_else(|| gpui::rgb(0x8b_8d_98).into(), |rgb| gpui::rgb(rgb).into())
}

/// What this window last told the server, and what keeps it told.
#[derive(Default)]
pub(crate) struct PresenceOut {
    last: Option<(Option<BoardPoint>, Option<Viewport>)>,
    sent_at: Option<Instant>,
    pending: Option<Task<()>>,
    keepalive: Option<Task<()>>,
    left: Option<Leave>,
    /// The view of the one followed this window last flew to.
    followed: Option<Viewport>,
    /// The viewport is moving because it follows someone, not because
    /// this person moved it.
    following_moves: bool,
}

/// Takes this window off the board when the board goes.
struct Leave {
    nodus: Nodus,
    project_id: String,
}

impl Drop for Leave {
    fn drop(&mut self) {
        self.nodus.board().leave(&self.project_id);
    }
}

impl BoardCanvas {
    /// The flow point at the centre of the canvas, and the zoom: what this
    /// person looks at, as followers see it.
    fn view_centre(&self) -> Option<Viewport> {
        let (w, h) = self.canvas_size();
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let (x, y) = viewport::to_flow(self.viewport, w / 2.0, h / 2.0);
        Some(Viewport {
            x,
            y,
            zoom: self.viewport.zoom,
        })
    }

    /// SRV-180: tells the server where this person is on a server board,
    /// when that changed: at once, or once 50 ms have passed since the last
    /// time.
    pub(crate) fn share_presence(&mut self, cx: &mut Context<'_, Self>) {
        if self.board.is_none() || self.read_only() || !self.on_server(cx) {
            return;
        }
        let cursor = self.pointer.filter(|p| self.bounds.contains(p)).map(|p| {
            let at = self.to_flow(p);
            BoardPoint { x: at.x, y: at.y }
        });
        let now = (cursor, self.view_centre());
        if self.presence.last == Some(now) {
            return;
        }
        if let Some(sent) = self.presence.sent_at.filter(|at| at.elapsed() < PRESENCE_EVERY) {
            if self.presence.pending.is_none() {
                let wait = PRESENCE_EVERY.saturating_sub(sent.elapsed());
                self.presence.pending = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |c, cx| {
                        c.presence.pending = None;
                        c.share_presence(cx);
                    });
                }));
            }
            return;
        }
        self.send_presence(now, cx);
    }

    fn send_presence(&mut self, (cursor, view): (Option<BoardPoint>, Option<Viewport>), cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        nodus.board().presence(&self.project_id, cursor, view);
        self.presence.last = Some((cursor, view));
        self.presence.sent_at = Some(Instant::now());
        if self.presence.left.is_none() {
            self.presence.left = Some(Leave {
                nodus,
                project_id: self.project_id.clone(),
            });
        }
        if self.presence.keepalive.is_none() {
            self.presence.keepalive = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(KEEPALIVE).await;
                    let again = this.update(cx, |c, cx| {
                        if let Some(last) = c.presence.last {
                            c.send_presence(last, cx);
                        }
                    });
                    if again.is_err() {
                        return;
                    }
                }
            }));
        }
    }

    fn peers(&self, cx: &App) -> Entity<Peers> {
        self.state.read(cx).peers.clone()
    }

    /// SRV-182: flies to the view of the one followed when it moved.
    pub(crate) fn follow_step(&mut self, cx: &mut Context<'_, Self>) {
        let peers = self.peers(cx);
        let view = {
            let peers = peers.read(cx);
            let Some(following) = peers.following(&self.project_id) else {
                self.presence.followed = None;
                return;
            };
            peers
                .on_board(&self.project_id)
                .iter()
                .find(|p| p.login == following.login && p.device == following.device)
                .and_then(|p| p.view)
        };
        let Some(view) = view.filter(|v| self.presence.followed != Some(*v)) else {
            return;
        };
        self.presence.followed = Some(view);
        let (w, h) = self.canvas_size();
        let to = Viewport {
            x: w / 2.0 - view.x * view.zoom,
            y: h / 2.0 - view.y * view.zoom,
            zoom: view.zoom,
        };
        self.presence.following_moves = true;
        self.fly_to(to, FOLLOW_MS, cx);
        self.presence.following_moves = false;
    }

    /// SRV-182: this person moved the viewport themselves: following stops.
    /// Whether it was following.
    pub(crate) fn stop_following(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.presence.following_moves {
            return false;
        }
        let peers = self.peers(cx);
        if peers.read(cx).following(&self.project_id).is_none() {
            return false;
        }
        self.presence.followed = None;
        peers.update(cx, Peers::unfollow)
    }

    /// The others' pointers, each in its colour with its login, over the
    /// nodes and the ink. A gliding one asks for the next frame for
    /// [`Peers`] alone.
    pub(crate) fn render_cursors(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let peers = self.peers(cx);
        let now = Instant::now();
        let (w, h) = self.canvas_size();
        let mut gliding = false;
        let cursors: Vec<(f32, f32, Hsla, String)> = peers
            .read(cx)
            .on_board(&self.project_id)
            .iter()
            .filter_map(|peer| {
                let (at, moving) = peer.cursor_at(now)?;
                gliding |= moving;
                let (x, y) = viewport::to_screen(self.viewport, at.x, at.y);
                let shown = (-40.0..w + 40.0).contains(&x) && (-40.0..h + 40.0).contains(&y);
                shown.then(|| (x as f32, y as f32, peer.color, peer.login.clone()))
            })
            .collect();
        if gliding {
            let id = peers.entity_id();
            window.on_next_frame(move |_, cx| cx.notify(id));
        }
        if cursors.is_empty() {
            return None;
        }
        let arrows = cursors
            .iter()
            .map(|(x, y, color, _)| (*x, *y, *color))
            .collect::<Vec<_>>();
        let pointers = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for (x, y, color) in &arrows {
                    paint_pointer(window, bounds.origin + point(px(*x), px(*y)), *color);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let labels = cursors.into_iter().map(|(x, y, color, login)| {
            div()
                .absolute()
                .left(px(x + 11.))
                .top(px(y + 17.))
                .h(px(18.))
                .px(px(6.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .bg(color)
                .font_family(fonts::UI)
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .whitespace_nowrap()
                .child(login)
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(pointers)
                .children(labels)
                .into_any_element(),
        )
    }

    /// SRV-182: the board framed in the colour of the one followed, and
    /// `Following @login` at its top right; a click there stops.
    pub(crate) fn render_following(&self, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let peers = self.peers(cx);
        let (login, color) = {
            let read = peers.read(cx);
            let following = read.following(&self.project_id)?;
            let color = read
                .on_board(&self.project_id)
                .iter()
                .find(|p| p.login == following.login)
                .map(|p| p.color)?;
            (following.login.clone(), color)
        };
        let t = theme::tokens(cx);
        let me = cx.entity().downgrade();
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .border_2()
                .border_color(color)
                .child(
                    div()
                        .id("board-following")
                        .absolute()
                        .top(px(12.))
                        .right(px(14.))
                        .occlude()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .h(px(30.))
                        .pl(px(10.))
                        .pr(px(12.))
                        .rounded(px(10.))
                        .border_2()
                        .border_color(color)
                        .map(|d| frosted(d, rgba_mix(t.bg_elevated, 0.86), 20., 1.6))
                        .shadow(t.md())
                        .font_family(fonts::UI)
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text_primary)
                        .cursor_pointer()
                        .tooltip(native_tooltip("Stop following (Esc)".into()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            if let Some(c) = me.upgrade() {
                                c.update(cx, |c, cx| {
                                    c.stop_following(cx);
                                });
                            }
                        })
                        .child(div().size(px(8.)).rounded_full().bg(color))
                        .child(format!("Following @{login}")),
                )
                .into_any_element(),
        )
    }
}

/// An arrow pointer with its tip at `tip`: filled in `color`, edged white.
fn paint_pointer(window: &mut Window, tip: gpui::Point<gpui::Pixels>, color: Hsla) {
    const SHAPE: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 16.0),
        (4.4, 12.2),
        (7.2, 18.4),
        (9.8, 17.2),
        (7.1, 11.2),
        (12.6, 11.2),
    ];
    let at = |(x, y): (f32, f32)| tip + point(px(x), px(y));
    let mut fill = PathBuilder::fill();
    let mut edge = PathBuilder::stroke(px(1.5));
    fill.move_to(at(SHAPE[0]));
    edge.move_to(at(SHAPE[0]));
    for corner in &SHAPE[1..] {
        fill.line_to(at(*corner));
        edge.line_to(at(*corner));
    }
    fill.close();
    edge.close();
    if let Ok(path) = edge.build() {
        window.paint_path(path, gpui::white());
    }
    if let Ok(path) = fill.build() {
        window.paint_path(path, color);
    }
}

/// SRV-181: the people on the server board on show, for the title bar:
/// each one's avatar (their initial without one) ringed in their colour.
/// Clicking one follows them (SRV-182), or stops.
pub fn title_bar_people(state: &Entity<AppState>, cx: &mut App) -> Option<AnyElement> {
    let s = state.read(cx);
    let project = s.route.project_id()?.to_owned();
    let peers = s.peers.clone();
    let read = peers.read(cx);
    let people: Vec<Peer> = read.people(&project).into_iter().cloned().collect();
    if people.is_empty() {
        return None;
    }
    let t = theme::tokens(cx);
    let followed = read.following(&project).map(|f| f.login.clone());
    let more = people.len().saturating_sub(AVATARS_SHOWN);
    let avatars: Vec<AnyElement> = people
        .iter()
        .take(AVATARS_SHOWN)
        .enumerate()
        .map(|(i, peer)| {
            let file = read.avatar(&peer.login);
            let following = followed.as_deref() == Some(peer.login.as_str());
            let tip = if following {
                format!("@{} · Stop following", peer.login)
            } else {
                format!("@{} · Follow", peer.login)
            };
            let face = crate::servers::avatar(&peer.login, file, 20., t);
            let (peers, project, peer) = (peers.clone(), project.clone(), peer.clone());
            div()
                .id(("titlebar-peer", i))
                .flex_none()
                .p(px(1.5))
                .rounded_full()
                .border_2()
                .border_color(peer.color)
                .when(following, |d| d.bg(peer.color))
                .when(i > 0, |d| d.ml(px(-4.)))
                .cursor_pointer()
                .tooltip(native_tooltip(tip.into()))
                .on_click(move |_, _, cx| peers.update(cx, |p, cx| p.toggle_follow(&project, &peer, cx)))
                .child(face)
                .into_any_element()
        })
        .collect();
    Some(
        div()
            .id("titlebar-people")
            .flex()
            .items_center()
            .children(avatars)
            .when(more > 0, |d| {
                d.child(
                    div()
                        .ml(px(4.))
                        .text_size(TEXT_SM)
                        .text_color(t.text_secondary)
                        .child(format!("+{more}")),
                )
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_glides_to_where_it_is_and_stops() {
        let start = Instant::now();
        let glide = Glide {
            from: BoardPoint { x: 0.0, y: 0.0 },
            to: BoardPoint { x: 100.0, y: 50.0 },
            since: start,
        };
        let (at, moving) = glide.at(start);
        assert!(moving && at == BoardPoint { x: 0.0, y: 0.0 });
        let (halfway, moving) = glide.at(start + GLIDE / 2);
        assert!(moving && halfway.x > 50.0 && halfway.x < 100.0);
        assert_eq!(glide.at(start + GLIDE), (BoardPoint { x: 100.0, y: 50.0 }, false));
    }

    #[test]
    fn reads_a_peers_colour() {
        assert_eq!(color_of("#0090ff"), gpui::rgb(0x0090ff).into());
        assert_eq!(color_of("nope"), gpui::rgb(0x8b8d98).into());
    }
}
