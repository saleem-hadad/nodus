//! The board's cards, deck, archive, notes and pad, drawn where the parity
//! goldens have them (storefront at x 60, y 96, zoom 0.8; infra at x 30,
//! y 80, zoom 0.72), each scene saved as a PNG in Display P3 like the
//! goldens, to compare side by side with `crates/nodus-ui/parity/compare.mjs`.
//!
//! ```sh
//! cargo run -p nodus-ui --example board_cards -- <out dir> [scene…] [--theme light|dark]
//! ```
//!
//! The window is a non-activating panel that never takes the focus, and
//! the pointer is simulated. Only the nodes are drawn (no title bar, no
//! canvas HUD): crops of the cards are what to compare.
#![allow(clippy::print_stdout, reason = "an example reports on stdout")]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;
use nodus_domain::{AgentInfo, BoardNote, Project, SessionCard, ThemePreference};
use nodus_ui::board::cards::present::{self, StackLook};
use nodus_ui::board::cards::{
    BoardEnv, CardActions, Resized, StackView, Z, archive_zone, note, note_pad, session_card::SessionNode,
};
use nodus_ui::components::icon::Assets;
use nodus_ui::parity::Parity;
use nodus_ui::platform::Platform;
use nodus_ui::theme;

const W: f32 = 1440.;
const H: f32 = 900.;
/// 2026-10-20 15:00 UTC, the harness's frozen clock.
const NOW: f64 = 1_792_508_400_000.0;
const MIN: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MIN;
const DAY: f64 = 24.0 * HOUR;

struct NoActions;

impl CardActions for NoActions {
    fn rename(&self, _: &str, _: &str, _: &mut App) {}
    fn set_renaming(&self, _: Option<&str>, _: &mut App) {}
    fn expand(&self, _: &[String], _: bool, _: &mut App) {}
    fn archive(&self, _: &[String], _: &mut App) {}
    fn stop(&self, _: &[String], _: &mut App) {}
    fn edit_note(&self, _: &str, _: &BoardNote, _: &str, _: &mut App) {}
    fn start_note(&self, _: &str, _: &mut App) {}
    fn open_session(&self, _: &str, _: &mut App) {}
    fn show_card(&self, _: &str, _: Option<&str>, _: &mut App) {}
    fn open_agents(&self, _: &str, _: Option<&str>, _: &mut App) {}
    fn open_terminal(&self, _: &str, _: &mut App) {}
    fn remove_notes(&self, _: &[String], _: &mut App) {}
    fn resize(&self, _: &Resized, _: (f32, f32, f32, f32), _: bool, _: &mut App) {}
    fn press_title(&self) {}
    fn raise(&self, _: &str, _: bool, _: &mut App) {}
    fn set_stack_query(&self, _: &str, _: &mut App) {}
    fn open_stack(&self, _: &mut App) {}
    fn close_stack(&self, _: &mut App) {}
    fn submit_stack(&self, _: &mut Window, _: &mut App) {}
}

fn merge(mut base: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

#[allow(clippy::too_many_arguments)]
fn card(
    id: &str,
    x: f64,
    y: f64,
    title: &str,
    branch: &str,
    ago: f64,
    reply: &str,
    messages: u32,
    status: &str,
    live: bool,
    extra: serde_json::Value,
) -> SessionCard {
    let base = serde_json::json!({
        "sessionId": id, "x": x, "y": y, "archived": false, "createdAt": NOW - 40.0 * DAY,
        "status": status, "live": live,
        "meta": {
            "sessionId": id, "title": title, "lastAssistantText": reply, "gitBranch": branch,
            "messageCount": messages, "createdAt": NOW - ago - HOUR, "updatedAt": NOW - ago,
            "transcriptPath": "/t"
        }
    });
    serde_json::from_value(merge(base, extra)).expect("card")
}

fn agent(id: &str, desc: &str, kind: &str, status: &str, started_ago: f64, tools: u32, tool: Option<&str>, activity: &str) -> AgentInfo {
    serde_json::from_value(serde_json::json!({
        "agentId": id, "agentType": kind, "description": desc, "status": status, "toolCount": tools,
        "startedAt": NOW - started_ago, "updatedAt": NOW - started_ago + 60_000.0,
        "tool": tool, "activity": activity
    }))
    .expect("agent")
}

fn storefront_cards() -> Vec<SessionCard> {
    let agents = vec![
        agent("a3", "Check how other lists show loading states", "Explore", "done", 110_000.0, 1, None,
            "The order history and the wishlist both use a pulsing grey block with the final layout; neither shares a component."),
        agent("a1", "Find every place the grid fetches products", "Explore", "working", 90_000.0, 2, Some("Read"), "Results.tsx"),
        agent("a2", "Write the skeleton component and its story", "general-purpose", "working", 70_000.0, 1, Some("Write"), "ProductSkeleton.tsx"),
    ];
    vec![
        card("sf-skeleton", 400., 40., "Add skeleton loading to the product grid", "feat/product-grid", 2.0 * MIN,
            "The grid renders nothing until `useProducts` resolves, so the footer jumps up and back. I'll look at how the other lists handle loading before adding a shared skeleton.",
            2, "working", true, serde_json::json!({ "agents": agents, "agentCount": 3 })),
        card("sf-tax", 752., 40., "Fix rounding in checkout tax totals", "fix/tax-rounding", 6.0 * MIN,
            "I need to run the checkout tests to confirm the totals now agree. May I run `pnpm test src/checkout`?",
            3, "waiting", true, serde_json::json!({})),
        card("sf-cart", 1072., 40., "Migrate the cart store to Zustand", "refactor/cart-store", 48.0 * MIN,
            "**Done.** The cart now lives in `src/cart/store.ts`. `useCart` and `useCartCount` read from the store with selectors, so the header badge no longer re-renders on every cart change. All 31 cart tests pass.",
            2, "done", true, serde_json::json!({})),
        card("sf-coupons", 400., 380., "Write e2e tests for coupon codes", "test/coupons", 3.5 * HOUR,
            "Added `e2e/coupons.spec.ts` with three tests. The sale-only case caught a bug: the code was accepted for full-price items too. I left the test failing and noted it in the file so you can decide how strict it should be.",
            2, "done", false, serde_json::json!({})),
        card("sf-search-plan", 752., 380., "Plan the v2 search page", "main", 26.0 * HOUR,
            "## Plan\n\n1. Reuse the skeleton tiles from the grid.\n2. Filters as a sheet on mobile, a sidebar on desktop.\n3. An empty state that suggests removing the last filter.\n\nThe cart store pattern works for the filter state too.",
            2, "done", false, serde_json::json!({ "parents": ["sf-cart", "sf-coupons"] })),
        serde_json::from_value(serde_json::json!({
            "sessionId": "sf-terminal", "kind": "terminal", "x": 1072, "y": 380, "archived": false,
            "createdAt": NOW - 3.0 * HOUR, "status": "done", "live": true
        }))
        .expect("terminal"),
    ]
}

fn archived(id: &str, title: &str, ago: f64, branch: &str, reply: &str) -> SessionCard {
    card(id, 0., 0., title, branch, ago, reply, 2, "done", false, serde_json::json!({ "archived": true, "archivedAt": NOW - ago }))
}

fn storefront_archive() -> Vec<SessionCard> {
    vec![
        archived("sf-arch-01", "Add Apple Pay to checkout", 2.0 * DAY, "feat/apple-pay", "Apple Pay now shows on Safari when the device supports it. The button sits above the card form and falls back to the card form on other browsers."),
        archived("sf-arch-02", "Fix flaky cart drawer test", 2.0 * DAY + 5.0 * HOUR, "main", "The drawer test raced the slide-in animation. It now waits for the drawer to be visible before clicking, and passed 50 runs in a row."),
        archived("sf-arch-03", "Lazy-load product images below the fold", 3.0 * DAY, "perf/images", "Images below the first two rows load lazily now. Largest Contentful Paint on the category page went from 2.9s to 1.8s on a throttled connection."),
        archived("sf-arch-04", "Translate checkout copy to German", 3.0 * DAY + 7.0 * HOUR, "i18n/de", "Added `de.json` for checkout with 64 strings. Three strings need a native speaker to check the tone; I marked them with TODO."),
    ]
}

fn note_of(json: serde_json::Value) -> BoardNote {
    serde_json::from_value(merge(serde_json::json!({ "createdAt": NOW - 5.0 * HOUR }), json)).expect("note")
}

/// A scene: what's drawn, and where the pointer is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scene {
    Populated,
    CardHover,
    Selected,
    DeckFanned,
    ArchiveHover,
    ArchiveOpen,
    NoteSelected,
    NoteEditing,
    PadHover,
    Expanded,
    ExpandedStopped,
}

impl Scene {
    const ALL: [Self; 11] = [
        Self::Populated,
        Self::CardHover,
        Self::Selected,
        Self::DeckFanned,
        Self::ArchiveHover,
        Self::ArchiveOpen,
        Self::NoteSelected,
        Self::NoteEditing,
        Self::PadHover,
        Self::Expanded,
        Self::ExpandedStopped,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Populated => "board-populated",
            Self::CardHover => "board-card-hover",
            Self::Selected => "board-selected",
            Self::DeckFanned => "board-deck-fanned",
            Self::ArchiveHover => "board-archive-hover",
            Self::ArchiveOpen => "board-archive-open",
            Self::NoteSelected => "board-note-selected",
            Self::NoteEditing => "board-note-editing",
            Self::PadHover => "board-pad-hover",
            Self::Expanded => "board-expanded",
            Self::ExpandedStopped => "board-expanded-stopped",
        }
    }

    fn infra(self) -> bool {
        matches!(self, Self::Expanded | Self::ExpandedStopped)
    }

    /// The viewport: (x, y, zoom).
    fn viewport(self) -> (f32, f32, f32) {
        if self.infra() { (30., 80., 0.72) } else { (60., 96., 0.8) }
    }

    /// Where the pointer rests (flow units), if over something.
    fn pointer(self) -> Option<(f32, f32)> {
        match self {
            Self::CardHover => Some((1072. + 144., 40. + 108.)),
            Self::DeckFanned => Some((544., 263.)),
            Self::ArchiveHover => Some((172., 200.)),
            Self::PadHover => Some((172., 546.)),
            _ => None,
        }
    }
}

struct Gallery {
    scene: Rc<RefCell<Scene>>,
}

fn env(zoom: f32, stack: StackView, renaming: Option<&str>, archive_hot: bool) -> Rc<BoardEnv> {
    Rc::new(BoardEnv {
        scale: Z(zoom),
        zoom,
        project_id: "storefront".into(),
        repo_path: Some("/Users/example/nodus-parity/repos/storefront".into()),
        renaming: renaming.map(str::to_owned),
        archive_hot,
        stack,
        board_bounds: Bounds::new(point(px(0.), px(52.)), size(px(W), px(H - 52.))),
        now_ms: NOW,
        text_focus: renaming.is_some(),
        terminal_modal: None,
        platform: Platform::CURRENT,
        actions: Rc::new(NoActions),
        terminals: None,
    })
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let scene = *self.scene.borrow();
        let t = theme::tokens(cx);
        let (vx, vy, zoom) = scene.viewport();
        let at = move |x: f32, y: f32| (vx + x * zoom, 52. + vy + y * zoom);
        let place = move |x: f32, y: f32, el: AnyElement| {
            let (sx, sy) = at(x, y);
            div().absolute().left(px(sx)).top(px(sy)).child(el)
        };
        let mut root = div().relative().size_full().bg(t.bg_canvas).overflow_hidden();
        // The board's dots (spec 09, Background).
        let dot = t.canvas_dot;
        root = root.child(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let gap = 22.0 * zoom;
                    let r = 0.8 * zoom;
                    let x0 = (vx - 10.2 * zoom).rem_euclid(gap);
                    let y0 = (52. + vy - 10.2 * zoom).rem_euclid(gap);
                    let mut y = y0 - gap;
                    while y < H {
                        let mut x = x0 - gap;
                        while x < W {
                            window.paint_quad(fill(
                                Bounds::new(point(px(x - r), px(y - r)), size(px(2. * r), px(2. * r))),
                                dot,
                            ).corner_radii(px(r)));
                            x += gap;
                        }
                        y += gap;
                    }
                },
            )
            .absolute()
            .size_full(),
        );

        if scene.infra() {
            let e = env(zoom, StackView::default(), None, false);
            let live = scene == Scene::Expanded;
            let mut certs = card("in-certs", 400., 0., "Rotate staging TLS certificates", "main", 35.0 * MIN,
                "Renewed all four staging certificates. Every ingress serves the new chain; the oldest expires on 18 January.",
                4, "done", live, serde_json::json!({ "expanded": true }));
            if live {
                certs.usage = Some(serde_json::from_value(serde_json::json!({
                    "model": "Opus 5.5", "contextPercent": 42.0, "contextTokens": 84000, "contextSize": 200000,
                    "costUsd": 1.84, "durationMs": 1_394_000, "at": NOW
                })).expect("usage"));
            }
            let shell: SessionCard = serde_json::from_value(serde_json::json!({
                "sessionId": "in-shell", "kind": "terminal", "x": 1152, "y": 0, "archived": false, "expanded": true,
                "createdAt": NOW - 5.0 * HOUR, "status": "done", "live": live
            })).expect("shell");
            let alerts = card("in-alerts", 400., 520., "Tidy up noisy disk alerts", "feat/alerts", 4.0 * HOUR,
                "Added `for: 30m` to the three disk alerts and a separate warning at 90% that never pages.", 2, "done", false,
                serde_json::json!({}));
            let budget = card("in-budget", 28., 52., "Add a cost budget alarm", "main", 2.0 * DAY,
                "Budget and alarm added in `budgets.tf`. The alarm emails the infra list.", 2, "done", false,
                serde_json::json!({ "archived": true }));
            root = root
                .child(place(0., 0., archive_zone::archive(&e, 1, (344., 362.), false, window, cx)))
                .child(place(0., 386., note_pad::pad(&e, (344., 296.), window, cx)))
                .child(place(28., 52., node(&e, budget, false, Some(look(0, false)), (288., 216.))))
                .child(place(400., 0., node(&e, certs, false, None, (720., 480.))))
                .child(place(1152., 0., node(&e, shell, false, None, (720., 480.))))
                .child(place(400., 520., node(&e, alerts, false, None, (288., 216.))))
                .child(place(64., 438., note::note(&e, &draft(), true, false, false, (216., 216.), None, window, cx)));
            return root;
        }

        let open = scene == Scene::ArchiveOpen;
        let archive = storefront_archive();
        let stack = StackView {
            open,
            hover: scene == Scene::ArchiveHover,
            query: String::new(),
            result_count: if open { 16 } else { 0 },
            scroll: 0.,
            scroll_max: if open { present::stack_scroll_max(16) } else { 0. },
        };
        let renaming = (scene == Scene::NoteEditing).then_some("sf-empty-states");
        let e = env(zoom, stack, renaming, false);
        let lift = scene == Scene::ArchiveHover;
        let size = if open { present::archive_size(Some(16)) } else { present::archive_size(None) };
        let frame = place(0., 0., archive_zone::archive(&e, 16, size, false, window, cx));
        let mut raised = Vec::new();
        if open {
            raised.push(frame);
        } else {
            root = root.child(frame);
        }
        root = root.child(place(0., 386., note_pad::pad(&e, (344., 296.), window, cx)));
        // The stack: depths 2, 1, then the top card (results when open).
        if open {
            for (slot, c) in archive.iter().enumerate() {
                let (x, y) = (356. + (slot % 2) as f32 * 320., 52. + (slot / 2) as f32 * 248.);
                raised.push(place(x, y, node(&e, c.clone(), false, Some(StackLook { depth: 0, result: true, lifted: false, rise: None }), (288., 216.))));
            }
        } else {
            for depth in (0..3).rev() {
                root = root.child(place(28., 52., node(&e, archive[depth].clone(), false, Some(look(depth as u8, lift)), (288., 216.))));
            }
        }
        let selected = |id: &str| scene == Scene::Selected && (id == "sf-cart" || id == "sf-coupons");
        let sticker = note_of(serde_json::json!({
            "id": "sf-sticker", "text": "Ship before the sale", "color": "yellow", "x": 400, "y": 40,
            "sessionId": "sf-skeleton", "createdAt": NOW - 3.0 * HOUR
        }));
        let mut cards = storefront_cards();
        if scene == Scene::DeckFanned {
            cards.rotate_left(1);
        }
        for c in cards {
            let (x, y) = (c.node.x as f32, c.node.y as f32);
            let stickers = if c.node.session_id == "sf-skeleton" { vec![sticker.clone()] } else { vec![] };
            let sel = selected(&c.node.session_id);
            root = root.child(place(x, y, SessionNode {
                env: e.clone(),
                card: c,
                matched: vec![],
                stickers,
                selected: sel,
                dragging: false,
                stack: None,
                size: (288., 216.),
            }.into_any_element()));
        }
        let yellow = note_of(serde_json::json!({
            "id": "sf-empty-states", "text": "Ask design about empty states for the grid and search",
            "color": "yellow", "x": 1400, "y": 40
        }));
        let blue = note_of(serde_json::json!({
            "id": "sf-signed",
            "text": "The search client still calls /v2/legacy/search, which v3 removes. Switch it to /v3/search before the docs go live.",
            "color": "blue", "x": 1400, "y": 300, "w": 240, "h": 216, "from": "ds-api-ref", "fromProject": "docs-site",
            "fromTitle": "Update API reference for v3", "createdAt": NOW - 80.0 * MIN
        }));
        let docs: Project = serde_json::from_value(serde_json::json!({
            "id": "docs-site", "name": "docs-site", "repoPath": "/r", "color": "purple", "createdAt": 0
        })).expect("project");
        let signature = present::signature(&blue, &[], &[docs]);
        root = root
            .child(place(1400., 40., note::note(&e, &yellow, false, false, false, (216., 216.), None, window, cx)))
            .child(place(1400., 300., note::note(&e, &blue, false, scene == Scene::NoteSelected, false, (240., 216.), signature, window, cx)))
            .child(place(64., 438., note::note(&e, &draft(), true, false, false, (216., 216.), None, window, cx)));
        root.children(raised)
    }
}

fn draft() -> BoardNote {
    note_of(serde_json::json!({ "id": "draft", "text": "", "color": "yellow", "x": 64, "y": 438, "createdAt": 0 }))
}

fn look(depth: u8, lifted: bool) -> StackLook {
    StackLook { depth, result: false, lifted, rise: None }
}

fn node(env: &Rc<BoardEnv>, card: SessionCard, selected: bool, stack: Option<StackLook>, size: (f32, f32)) -> AnyElement {
    SessionNode {
        env: env.clone(),
        card,
        matched: vec![],
        stickers: vec![],
        selected,
        dragging: false,
        stack,
        size,
    }
    .into_any_element()
}

/// Runs `f` on the gallery's window outside any update of its view (a
/// forced frame renders the view).
fn in_window<R>(
    cx: &mut AsyncApp,
    handle: WindowHandle<Gallery>,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R> {
    cx.update(|cx| cx.update_window(handle.into(), |_, window, cx| f(window, cx)))
}

/// The theme, with the text caret hidden as the parity shots have it.
fn set_theme(preference: ThemePreference, cx: &mut App) {
    theme::apply_preference(preference, cx);
    component::theme::Theme::global_mut(cx).caret = transparent_black();
}

/// An sRGB colour in Display P3, as the goldens hold Chromium's output.
fn to_p3(image: &mut image::RgbaImage) {
    for pixel in image.pixels_mut() {
        let [r, g, b] = nodus_ui::automation::srgb_to_display_p3([pixel[0], pixel[1], pixel[2]]);
        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().cloned().unwrap_or_else(|| std::env::temp_dir().display().to_string()));
    let mut themes = vec![ThemePreference::Light, ThemePreference::Dark];
    let mut scenes: Vec<Scene> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--theme" {
            themes = vec![if args.get(i + 1).map(String::as_str) == Some("dark") {
                ThemePreference::Dark
            } else {
                ThemePreference::Light
            }];
            i += 2;
            continue;
        }
        scenes.extend(Scene::ALL.into_iter().filter(|s| s.name().contains(args[i].as_str())));
        i += 1;
    }
    if scenes.is_empty() {
        scenes = Scene::ALL.to_vec();
    }
    std::fs::create_dir_all(&out).expect("out dir");

    application().with_assets(Assets).run(move |cx: &mut App| {
        init(cx);
        theme::fonts::register(cx);
        cx.set_global(Parity { frozen: true, now_ms: Some(NOW) });
        set_theme(themes[0], cx);
        let scene = Rc::new(RefCell::new(scenes[0]));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(40.), px(40.)), size(px(W), px(H))))),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            is_movable: false,
            ..WindowOptions::default()
        };
        let handle = cx
            .open_window(options, {
                let scene = scene.clone();
                move |window, cx| {
                    #[cfg(target_os = "macos")]
                    {
                        nodus_ui::mac::tag_layer_srgb(window);
                        // Only the simulated pointer counts.
                        nodus_ui::mac::ignore_real_mouse(window);
                    }
                    cx.new(|_| Gallery { scene })
                }
            })
            .expect("window");
        cx.spawn(async move |cx| {
            for theme_pref in themes {
                let name = if theme_pref == ThemePreference::Dark { "dark" } else { "light" };
                let _ = cx.update(|cx| set_theme(theme_pref, cx));
                for s in &scenes {
                    *scene.borrow_mut() = *s;
                    let (vx, vy, zoom) = s.viewport();
                    // Away first, so hover states start from rest. Every step
                    // draws a frame at once: the pointer is hit-tested against
                    // the last frame, and a covered window isn't asked for any.
                    let _ = in_window(cx, handle, |window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                            position: point(px(720.), px(20.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }), cx);
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                    cx.background_executor().timer(Duration::from_millis(900)).await;
                    if let Some((x, y)) = s.pointer() {
                        let position = point(px(vx + x * zoom), px(52. + vy + y * zoom));
                        let _ = in_window(cx, handle, |window, cx| {
                            window.refresh();
                            window.draw(cx).clear(cx);
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                                position,
                                pressed_button: None,
                                modifiers: Modifiers::default(),
                            }), cx);
                            window.refresh();
                            window.draw(cx).clear(cx);
                        });
                    }
                    cx.background_executor().timer(Duration::from_millis(1400)).await;
                    // A frame drawn now, even while the window is covered (macOS
                    // stops asking for frames then), so the shot is this scene.
                    let image = in_window(cx, handle, |window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        window.render_to_image()
                    });
                    match image {
                        Ok(Ok(mut image)) => {
                            to_p3(&mut image);
                            let path = out.join(format!("{}.{name}.rust.png", s.name()));
                            image.save(&path).expect("save");
                            println!("{}", path.display());
                        }
                        other => println!("capture failed for {}: {:?}", s.name(), other.err()),
                    }
                }
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
