//! Scripted verification. Drives *synthetic* input through `Window::dispatch_event` /
//! `dispatch_keystroke` (never the user's real mouse/keyboard), asserts state, and screenshots the
//! app's own windows with `screencapture -l <windowNumber>`.
use std::time::{Duration, Instant};

use gpui::*;

use crate::board::{Board, TERM_CARD, VIEW_CARD};
use crate::panel::Panel;
use crate::shell::Shell;
use crate::terminal::TerminalView;

fn shots_dir() -> std::path::PathBuf {
    std::env::var("S1_SHOTS").map(Into::into).unwrap_or_else(|_| std::path::PathBuf::from("shots"))
}

async fn sleep(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

async fn shot(cx: &mut AsyncApp, h: AnyWindowHandle, name: &str) {
    #[cfg(target_os = "macos")]
    {
        let Ok(wid) = cx.update_window(h, |_, window, _| crate::mac::window_number(window)) else { return };
        let path = shots_dir().join(format!("{name}.png"));
        let _ = std::fs::create_dir_all(shots_dir());
        let p = path.clone();
        let ok = cx
            .background_executor()
            .spawn(async move {
                // ScreenCaptureKit single-window capture (helper next to the binary); falls back to screencapture -l.
                match std::env::var("S1_WINCAP") {
                    Ok(tool) => std::process::Command::new(tool).arg(wid.to_string()).arg(&p).status().map(|s| s.success()).unwrap_or(false),
                    Err(_) => std::process::Command::new("screencapture").arg("-x").arg("-o").arg(format!("-l{wid}")).arg(&p).status().map(|s| s.success()).unwrap_or(false),
                }
            })
            .await;
        println!("S1 SHOT {name} ok={ok}");
    }
}

fn dispatch(cx: &mut AsyncApp, h: AnyWindowHandle, ev: PlatformInput) {
    let _ = cx.update_window(h, |_, window, cx| {
        window.dispatch_event(ev, cx);
    });
}

fn keystroke_for(c: char) -> Keystroke {
    let (key, shift) = match c {
        ' ' => ("space".to_string(), false),
        '\r' | '\n' => ("enter".to_string(), false),
        c if c.is_ascii_uppercase() => (c.to_ascii_lowercase().to_string(), true),
        c => (c.to_string(), false),
    };
    Keystroke {
        modifiers: Modifiers { shift, ..Default::default() },
        key_char: if c == '\r' || c == '\n' { None } else { Some(c.to_string()) },
        key,
    }
}

fn type_text(cx: &mut AsyncApp, h: AnyWindowHandle, text: &str) {
    let _ = cx.update_window(h, |_, window, cx| {
        for c in text.chars() {
            window.dispatch_keystroke(keystroke_for(c), cx);
        }
    });
}

fn key(cx: &mut AsyncApp, h: AnyWindowHandle, spec: &'static str) {
    let _ = cx.update_window(h, |_, window, cx| {
        window.dispatch_keystroke(Keystroke::parse(spec).ok().expect("keystroke"), cx);
    });
}

fn mouse_down(pos: Point<Pixels>, mods: Modifiers) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: pos, modifiers: mods, click_count: 1, first_mouse: false })
}
fn mouse_move(pos: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent { position: pos, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() })
}
fn mouse_up(pos: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: pos, modifiers: Modifiers::default(), click_count: 1 })
}

fn screen(cx: &mut AsyncApp, t: &Entity<TerminalView>) -> String {
    t.read_with(cx, |t, _| t.session.screen_text())
}

async fn wait_screen(cx: &mut AsyncApp, t: &Entity<TerminalView>, needle: &str, ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        if screen(cx, t).contains(needle) {
            return true;
        }
        sleep(cx, 50).await;
    }
    false
}

async fn run_anim(cx: &mut AsyncApp, board: &Entity<Board>, frames: u32, f: impl FnMut(&mut Board, u32) -> bool + 'static) -> String {
    let mut f = f;
    board.update(cx, |b, cx| {
        b.frames = Default::default();
        b.frames.recording = true;
        b.animate = Some(Box::new(move |b, tick| {
            f(b, tick);
            tick < frames
        }));
        cx.notify();
    });
    let start = Instant::now();
    loop {
        sleep(cx, 50).await;
        let done = board.read_with(cx, |b, _| b.animate.is_none());
        if done || start.elapsed() > Duration::from_secs(20) {
            break;
        }
    }
    sleep(cx, 50).await;
    board.update(cx, |b, _| {
        b.frames.recording = false;
        b.frames.summary()
    })
}

fn rss_kb() -> u64 {
    let pid = std::process::id().to_string();
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub fn run_main(h: AnyWindowHandle, shell: Entity<Shell>, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(e) = main_steps(h, shell, cx).await {
            println!("S1 AUTOPILOT ERROR {e:#}");
        }
        println!("S1 RSS_KB {}", rss_kb());
        println!("S1 AUTOPILOT MAIN DONE");
        if crate::env_flag("S1_QUIT") {
            sleep(cx, 300).await;
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

async fn main_steps(h: AnyWindowHandle, shell: Entity<Shell>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    sleep(cx, 1500).await;
    println!("S1 RSS_KB_START {}", rss_kb());
    cx.update_window(h, |_, window, _| {
        println!("S1 STEP1 appearance={:?} scale={} bounds={:?}", window.appearance(), window.scale_factor(), window.bounds());
        #[cfg(target_os = "macos")]
        unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            let w = crate::mac::ns_window(window);
            for (i, name) in ["close", "miniaturize", "zoom"].iter().enumerate() {
                let b: *mut AnyObject = msg_send![&*w, standardWindowButton: i as usize];
                if !b.is_null() {
                    let f: objc2_foundation::NSRect = msg_send![b, frame];
                    let sv: *mut AnyObject = msg_send![b, superview];
                    let sf: objc2_foundation::NSRect = msg_send![sv, frame];
                    println!("S1 STEP1 traffic_light {name} x={} y_from_top={}", f.origin.x, sf.size.height - f.origin.y - f.size.height);
                }
            }
            let fr = w.frame();
            let on_space: bool = msg_send![&*w, isOnActiveSpace];
            let occl: usize = msg_send![&*w, occlusionState];
            println!(
                "S1 STEP1 nswindow frame=({},{} {}x{}) visible={} onActiveSpace={on_space} occlusion={occl:#x}",
                fr.origin.x, fr.origin.y, fr.size.width, fr.size.height, w.isVisible()
            );
            let style = w.styleMask();
            println!("S1 STEP1 styleMask={:#x} titlebarAppearsTransparent={}", style.0, w.titlebarAppearsTransparent());
        }
    })?;
    shot(cx, h, "01-initial").await;
    // Flip this window's appearance (what a system light/dark switch does) and check the theme follows.
    #[cfg(target_os = "macos")]
    {
        let was_dark = cx.update_window(h, |_, window, _| matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark))?;
        cx.update_window(h, |_, window, _| crate::mac::force_appearance(window, !was_dark))?;
        sleep(cx, 400).await;
        let (now_dark, tokens_dark) = cx.update_window(h, |_, window, cx| {
            (matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark), crate::theme::tokens(cx).dark)
        })?;
        println!("S1 STEP1 appearance_flip was_dark={was_dark} now_dark={now_dark} tokens_dark={tokens_dark} pass={}", now_dark != was_dark && tokens_dark == now_dark);
        shot(cx, h, "01b-flipped").await;
        cx.update_window(h, |_, window, _| crate::mac::force_appearance(window, was_dark))?;
        sleep(cx, 300).await;
    }

    // ---- Step 1: gpui-component Input takes synthetic keystrokes ----
    let input = shell.read_with(cx, |s, _| s.input.clone());
    cx.update_window(h, |_, window, cx| input.update(cx, |s, cx| s.focus(window, cx)))?;
    type_text(cx, h, "hello gpui");
    sleep(cx, 100).await;
    let v = input.read_with(cx, |s, cx| s.value().to_string());
    println!("S1 STEP1 input_value={v:?} pass={}", v == "hello gpui");

    let Some(board) = shell.read_with(cx, |s, _| s.board.clone()) else { return Ok(()) };
    let side = shell.read_with(cx, |s, _| s.side_term.clone()).unwrap();
    let card_term = board.read_with(cx, |b, _| b.term.clone());

    // ---- Step 2: board input handlers ----
    cx.update_window(h, |_, window, cx| {
        let f = board.read(cx).focus.clone();
        window.focus(&f, cx);
    })?;
    let vp = board.read_with(cx, |b, _| b.viewport);
    let p = vp.origin + point(px(300.), px(250.));
    let (pan0, _z0) = board.read_with(cx, |b, _| (b.pan, b.zoom));
    dispatch(cx, h, PlatformInput::Scroll(ScrollEvent { position: p, delta: ScrollDelta::Pixels(point(px(30.), px(20.))), modifiers: Modifiers::default(), touch_phase: TouchPhase::Moved, source: Default::default() }));
    let pan1 = board.read_with(cx, |b, _| b.pan);
    println!("S1 STEP2 pan_by_scroll {:?} -> {:?} pass={}", pan0, pan1, (pan1.x - pan0.x - 30.).abs() < 0.01 && (pan1.y - pan0.y - 20.).abs() < 0.01);

    let w_before = board.read_with(cx, |b, _| b.to_world(p));
    // hover state comes from the last mouse move; pinch listeners require the hitbox be hovered
    dispatch(cx, h, PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: None, modifiers: Modifiers::default() }));
    sleep(cx, 50).await;
    dispatch(cx, h, PlatformInput::Pinch(PinchEvent { position: p, delta: 0.25, modifiers: Modifiers::default(), phase: TouchPhase::Moved }));
    let (w_after, z1) = board.read_with(cx, |b, _| (b.to_world(p), b.zoom));
    println!("S1 STEP2 pinch zoom={z1:.3} world_under_pointer {:?} -> {:?} pass={}", w_before, w_after, (z1 - 1.25).abs() < 0.001 && (w_before.x - w_after.x).abs() < 0.01 && (w_before.y - w_after.y).abs() < 0.01);
    dispatch(cx, h, PlatformInput::Scroll(ScrollEvent { position: p, delta: ScrollDelta::Pixels(point(px(0.), px(30.))), modifiers: Modifiers { control: true, ..Default::default() }, touch_phase: TouchPhase::Moved, source: Default::default() }));
    let (w_after2, z2) = board.read_with(cx, |b, _| (b.to_world(p), b.zoom));
    println!("S1 STEP2 ctrl_wheel zoom={z2:.3} world_under_pointer {:?} pass={}", w_after2, z2 < z1 && (w_before.x - w_after2.x).abs() < 0.01);

    // drag card 0 by its header
    sleep(cx, 50).await;
    let (hdr, pos0, zoom) = board.read_with(cx, |b, _| {
        let c = b.cards.iter().find(|c| c.id == 0).unwrap();
        let r = b.card_screen_bounds(c);
        (r.origin + point(px(60.), px(10.)), c.pos, b.zoom)
    });
    dispatch(cx, h, mouse_down(hdr, Modifiers::default()));
    for i in 1..=5 {
        sleep(cx, 20).await;
        dispatch(cx, h, mouse_move(hdr + point(px(20. * i as f32), px(10. * i as f32))));
    }
    dispatch(cx, h, mouse_up(hdr + point(px(100.), px(50.))));
    let pos1 = board.read_with(cx, |b, _| b.cards.iter().find(|c| c.id == 0).unwrap().pos);
    println!(
        "S1 STEP2 drag card0 {:?} -> {:?} pass={}",
        pos0,
        pos1,
        ((pos1.x - pos0.x) - 100. / zoom).abs() < 0.5 && ((pos1.y - pos0.y) - 50. / zoom).abs() < 0.5
    );

    // rubber band from empty space (left of everything) across cards 0..2
    board.update(cx, |b, cx| {
        b.pan = point(200., 120.);
        b.zoom = 0.6;
        cx.notify();
    });
    sleep(cx, 100).await;
    let vp = board.read_with(cx, |b, _| b.viewport);
    let start = vp.origin + point(px(20.), px(20.));
    let end = vp.origin + point(px(200. + 400. * 0.6), px(120. + 330. * 0.6));
    dispatch(cx, h, mouse_down(start, Modifiers::default()));
    for i in 1..=6 {
        sleep(cx, 16).await;
        let t = i as f32 / 6.;
        dispatch(cx, h, mouse_move(start + (end - start) * t));
    }
    sleep(cx, 50).await;
    shot(cx, h, "02-rubber-band").await;
    dispatch(cx, h, mouse_up(end));
    let sel = board.read_with(cx, |b, _| b.selected.clone());
    println!("S1 STEP2 rubber_band selected={sel:?}");

    // smoothness: continuous pan, then continuous zoom (fresh text sizes every frame)
    board.update(cx, |b, cx| {
        b.pan = point(0., 0.);
        b.zoom = 1.0;
        b.selected.clear();
        cx.notify();
    });
    sleep(cx, 200).await;
    let s = run_anim(cx, &board, 180, |b, t| {
        b.pan.x = -200. * ((t as f32) / 30.).sin();
        b.pan.y = -60. * ((t as f32) / 45.).cos();
        true
    })
    .await;
    println!("S1 STEP2 anim_pan {s}");
    let s = run_anim(cx, &board, 180, move |b, t| {
        let target = 0.6 + 0.6 * (1. + ((t as f32) / 25.).sin()) / 2.;
        let center = b.viewport.origin + point(px(400.), px(250.));
        b.zoom_at(center, target / b.zoom);
        true
    })
    .await;
    println!("S1 STEP2 anim_zoom {s}");
    let r = board.read_with(cx, |b, cx| b.term.read(cx).session.resizes.load(std::sync::atomic::Ordering::Relaxed));
    println!("S1 STEP2 pty_resizes_after_zoom_anim {r} (1 = only the initial fit)");
    board.update(cx, |b, cx| {
        b.pan = point(0., 0.);
        b.zoom = 1.0;
        cx.notify();
    });
    sleep(cx, 300).await;
    shot(cx, h, "03-board").await;
    // Step 7: drag the sticker; it tilts from 3° to 7° with the ease-stack spring, then settles back.
    let st = board.read_with(cx, |b, _| {
        let c = b.cards.iter().find(|c| c.id == crate::board::STICKER_CARD).unwrap();
        b.card_screen_bounds(c).center()
    });
    dispatch(cx, h, mouse_down(st, Modifiers::default()));
    for i in 1..=10 {
        sleep(cx, 30).await;
        dispatch(cx, h, mouse_move(st + point(px(6. * i as f32), px(-3. * i as f32))));
    }
    sleep(cx, 500).await;
    shot(cx, h, "03b-sticker-dragging").await;
    dispatch(cx, h, mouse_up(st + point(px(60.), px(-30.))));
    sleep(cx, 700).await;
    shot(cx, h, "03c-sticker-rest").await;

    // ---- Step 3: terminals ----
    let ok = wait_screen(cx, &card_term, "%", 4000).await || wait_screen(cx, &card_term, "$", 1000).await;
    println!("S1 STEP3 card_prompt={ok}");
    cx.update_window(h, |_, window, cx| {
        let f = card_term.read(cx).focus.clone();
        window.focus(&f, cx);
    })?;
    type_text(cx, h, "ls --color=auto -la\r");
    let ok = wait_screen(cx, &card_term, "README.md", 3000).await;
    println!("S1 STEP3 ls_color pass={ok}");
    type_text(cx, h, "printf '\\033[1;31mBOLD RED\\033[0m \\033[42;30m GREEN BG \\033[0m \\033[4munder\\033[0m \\033[38;5;208midx208\\033[0m \\033[38;2;255;100;200mtruecolor\\033[0m wide:\\u4e2d\\u6587|\\n'\r");
    let ok = wait_screen(cx, &card_term, "truecolor", 3000).await;
    println!("S1 STEP3 printf_colors pass={ok}");
    // ctrl-c interrupts
    type_text(cx, h, "sleep 30\r");
    sleep(cx, 400).await;
    key(cx, h, "ctrl-c");
    type_text(cx, h, "echo after-ctrl-c\r");
    let ok = wait_screen(cx, &card_term, "\nafter-ctrl-c", 3000).await;
    println!("S1 STEP3 ctrl_c pass={ok}");
    // Throughput: flood the card terminal while the board animates; the UI must keep its frame rate.
    board.update(cx, |b, cx| {
        b.frames = Default::default();
        b.frames.recording = true;
        cx.notify();
    });
    let bytes0 = card_term.read_with(cx, |t, _| t.session.bytes_read.load(std::sync::atomic::Ordering::Relaxed));
    let t0 = Instant::now();
    type_text(cx, h, "seq 1 400000; echo FLOOD-DONE\r");
    let s_anim = run_anim(cx, &board, 150, |b, t| {
        b.pan.x = -40. * ((t as f32) / 15.).sin();
        true
    })
    .await;
    let done = wait_screen(cx, &card_term, "FLOOD-DONE\n", 20000).await;
    let bytes = card_term.read_with(cx, |t, _| t.session.bytes_read.load(std::sync::atomic::Ordering::Relaxed)) - bytes0;
    println!(
        "S1 STEP3 flood done={done} bytes={bytes} secs={:.2} MB/s={:.1} frames_during: {s_anim}",
        t0.elapsed().as_secs_f64(),
        bytes as f64 / 1e6 / t0.elapsed().as_secs_f64()
    );
    board.update(cx, |b, cx| {
        let c = b.cards.iter().find(|c| c.id == TERM_CARD).unwrap().clone();
        b.pan = point(-c.pos.x + 30., -c.pos.y + 30.);
        cx.notify();
    });
    sleep(cx, 300).await;
    shot(cx, h, "04-terminal-card").await;
    let resizes0 = card_term.read_with(cx, |t, _| t.session.resizes.load(std::sync::atomic::Ordering::Relaxed));
    let size0 = card_term.read_with(cx, |t, _| *t.session.size.lock().unwrap());
    board.update(cx, |b, cx| {
        let c = b.cards.iter().find(|c| c.id == TERM_CARD).unwrap().clone();
        b.zoom = 1.35;
        b.pan = point(-c.pos.x * 1.35 + 20., -c.pos.y * 1.35 + 20.);
        cx.notify();
    });
    sleep(cx, 300).await;
    shot(cx, h, "05-terminal-card-zoomed").await;
    let resizes1 = card_term.read_with(cx, |t, _| t.session.resizes.load(std::sync::atomic::Ordering::Relaxed));
    let size1 = card_term.read_with(cx, |t, _| *t.session.size.lock().unwrap());
    println!("S1 STEP3 zoom_keeps_grid size {size0:?} -> {size1:?} resizes {resizes0}->{resizes1} pass={}", size0 == size1);
    board.update(cx, |b, cx| {
        b.zoom = 1.0;
        b.pan = point(0., 0.);
        cx.notify();
    });

    // side panel terminal: top, then vim
    cx.update_window(h, |_, window, cx| {
        let f = side.read(cx).focus.clone();
        window.focus(&f, cx);
    })?;
    let _ = wait_screen(cx, &side, "%", 3000).await;
    type_text(cx, h, "top -o cpu -s 1\r");
    let ok = wait_screen(cx, &side, "Processes:", 4000).await;
    sleep(cx, 1200).await;
    println!("S1 STEP3 top pass={ok}");
    shot(cx, h, "06-top").await;
    type_text(cx, h, "q");
    sleep(cx, 400).await;
    type_text(cx, h, "vim -u NONE -N -c 'syntax on' -c 'set number' src/main.rs\r");
    let ok = wait_screen(cx, &side, "fn main", 4000).await;
    type_text(cx, h, "Go// typed in vim");
    key(cx, h, "escape");
    sleep(cx, 400).await;
    let ok2 = screen(cx, &side).contains("typed in vim");
    println!("S1 STEP3 vim open={ok} insert={ok2}");
    shot(cx, h, "07-vim").await;
    type_text(cx, h, ":q!\r");
    sleep(cx, 400).await;

    // resize on element resize: shrink the window, side terminal rows/cols must follow
    let before = side.read_with(cx, |t, _| *t.session.size.lock().unwrap());
    cx.update_window(h, |_, window, _| window.resize(size(px(1100.), px(700.))))?;
    sleep(cx, 500).await;
    let after = side.read_with(cx, |t, _| *t.session.size.lock().unwrap());
    type_text(cx, h, "stty size\r");
    sleep(cx, 400).await;
    let scr = screen(cx, &side);
    let stty = scr.lines().rev().find(|l| l.split_whitespace().count() == 2 && l.split_whitespace().all(|w| w.parse::<u32>().is_ok())).unwrap_or("").to_string();
    println!("S1 STEP3 resize {before:?} -> {after:?} stty_size={stty:?} pass={}", before != after && stty == format!("{} {}", after.1, after.0));

    // ---- Step 4: webview ----
    let start = Instant::now();
    loop {
        let done = board.read_with(cx, |b, _| b.ipc_log.iter().any(|m| m.contains("\"done\"")));
        if done || start.elapsed() > Duration::from_secs(8) {
            println!("S1 STEP4 ipc_roundtrips_done={done} msgs={}", board.read_with(cx, |b, _| b.ipc_log.len()));
            break;
        }
        sleep(cx, 100).await;
    }
    sleep(cx, 300).await;
    board.update(cx, |b, cx| {
        if let Some(c) = b.cards.iter_mut().find(|c| c.id == 0) {
            c.pos = point(40., 40.);
        }
        // the sticker drag test may have left it over the view card
        if let Some(c) = b.cards.iter_mut().find(|c| c.id == crate::board::STICKER_CARD) {
            c.pos = point(40., 520.);
        }
        cx.notify();
    });
    sleep(cx, 300).await;
    // (a) z-order: the overlapping note is drawn after the view card -> snapshot mode.
    let view_pt = |cx: &mut AsyncApp| board.read_with(cx, |b, _| {
        let c = b.cards.iter().find(|c| c.id == VIEW_CARD).unwrap();
        b.card_screen_bounds(c).origin + point(px(60.), px(120.))
    });
    let overlap_pt = board.read_with(cx, |b, _| {
        let c = b.cards.iter().find(|c| c.id == 5).unwrap();
        b.card_screen_bounds(c).origin + point(px(20.), px(30.))
    });
    let (snap, obstructed) = board.read_with(cx, |b, _| (b.web.as_ref().map(|w| w.showing_snapshot.get()), b.view_obstructed()));
    let hit = cx.update_window(h, |_, window, _| crate::mac::hit_test_class(window, overlap_pt))?;
    let order = board.read_with(cx, |b, _| b.cards.iter().map(|c| (c.id, c.pos.x as i32, c.pos.y as i32)).collect::<Vec<_>>());
    println!("S1 STEP4 order {order:?}");
    println!("S1 STEP4 overlapped obstructed={obstructed} showing_snapshot={snap:?} hit_at_overlap={hit}");
    shot(cx, h, "08a-view-obstructed-snapshot").await;
    board.update(cx, |b, cx| {
        if let Some(c) = b.cards.iter_mut().find(|c| c.id == 5) {
            c.pos = point(400., 620.);
        }
        cx.notify();
    });
    sleep(cx, 300).await;
    let vp_pt = view_pt(cx);
    let snap = board.read_with(cx, |b, _| b.web.as_ref().map(|w| w.showing_snapshot.get()));
    let hit = cx.update_window(h, |_, window, _| crate::mac::hit_test_class(window, vp_pt))?;
    println!("S1 STEP4 unobstructed showing_snapshot={snap:?} hit_on_view={hit}");
    shot(cx, h, "08b-view-live").await;
    // (b) pointer pass-through while the board drags something
    let hdr = board.read_with(cx, |b, _| {
        let c = b.cards.iter().find(|c| c.id == 1).unwrap();
        b.card_screen_bounds(c).origin + point(px(60.), px(10.))
    });
    dispatch(cx, h, mouse_down(hdr, Modifiers::default()));
    dispatch(cx, h, mouse_move(hdr + point(px(5.), px(5.))));
    sleep(cx, 100).await;
    let hit_drag = cx.update_window(h, |_, window, _| crate::mac::hit_test_class(window, vp_pt))?;
    dispatch(cx, h, mouse_up(hdr + point(px(5.), px(5.))));
    sleep(cx, 100).await;
    let hit_after = cx.update_window(h, |_, window, _| crate::mac::hit_test_class(window, vp_pt))?;
    println!("S1 STEP4 passthrough hit_during_drag={hit_drag} hit_after={hit_after}");
    // (d) transient user activation inside the sandboxed iframe, from a real (synthetic NSEvent) click
    let btn = board.read_with(cx, |b, _| b.web.as_ref().map(|w| w.last_doc.get().origin + point(px(40.), px(20.))));
    if let Some(btn) = btn {
        let before = board.read_with(cx, |b, _| b.ipc_log.len());
        let nsw = cx.update_window(h, |_, window, _| crate::mac::ns_window(window))?;
        crate::mac::synthetic_click(&nsw, btn);
        sleep(cx, 500).await;
        let msgs = board.read_with(cx, |b, _| b.ipc_log[before..].to_vec());
        println!("S1 STEP4 click_msgs {msgs:?}");
    }
    // ink tool out -> snapshot
    board.update(cx, |b, cx| {
        b.ink_mode = true;
        cx.notify();
    });
    sleep(cx, 400).await;
    let snap = board.read_with(cx, |b, _| b.web.as_ref().map(|w| w.showing_snapshot.get()));
    println!("S1 STEP4 ink_mode showing_snapshot={snap:?}");
    board.update(cx, |b, cx| {
        b.ink_mode = false;
        cx.notify();
    });
    sleep(cx, 200).await;
    shot(cx, h, "08-webview").await;
    let probe = |cx: &mut AsyncApp, label: &'static str| {
        let _ = board.update(cx, |b, _| {
            if let Some(web) = &b.web {
                let _ = web.webview.evaluate_script_with_callback("innerWidth + 'x' + innerHeight + ' dpr=' + devicePixelRatio", move |r| {
                    println!("S1 STEP4 page_viewport[{label}] {r}");
                });
            }
        });
    };
    probe(cx, "zoom1.0");
    // pan so the view card is cut by the board's left/top edge (clip test) and zoom (page zoom test)
    board.update(cx, |b, cx| {
        let c = b.cards.iter().find(|c| c.id == VIEW_CARD).unwrap().clone();
        b.zoom = 0.7;
        b.pan = point(-c.pos.x * 0.7 - 90., -c.pos.y * 0.7 - 60.);
        cx.notify();
    });
    sleep(cx, 400).await;
    probe(cx, "zoom0.7");
    shot(cx, h, "09-webview-clipped-zoomed").await;
    board.update(cx, |b, cx| {
        b.zoom = 1.0;
        b.pan = point(0., 0.);
        cx.notify();
    });
    sleep(cx, 300).await;
    // pan the webview card: does the native view follow within the same frame? (pan smoothness w/ webview)
    let s = run_anim(cx, &board, 120, |b, t| {
        b.pan.x = -120. * ((t as f32) / 20.).sin();
        true
    })
    .await;
    println!("S1 STEP4 anim_pan_with_webview {s}");
    sleep(cx, 300).await;
    println!("S1 RSS_KB_END {}", rss_kb());
    Ok(())
}

pub fn run_panel(h: AnyWindowHandle, panel: Entity<Panel>, cx: &mut App) {
    cx.spawn(async move |cx| {
        sleep(cx, 700).await;
        let _ = cx.update_window(h, |_, window, _| {
            #[cfg(target_os = "macos")]
            println!("S1 STEP5 desc_at_open {}", crate::mac::describe_window(window));
            println!("S1 STEP5 gpui_active={}", window.is_window_active());
        });
        type_text(cx, h, "buy milk");
        sleep(cx, 100).await;
        let v = panel.read_with(cx, |p, cx| p.input.read(cx).value().to_string());
        println!("S1 STEP5 panel_input={v:?}");
        shot(cx, h, "10-panel").await;
        // Blur: make another of our windows key (if any). The panel must hide itself.
        let others: Vec<AnyWindowHandle> = cx.update(|cx| cx.windows().into_iter().filter(|w| *w != h).collect());
        if others.is_empty() {
            // Panel alone: give the keyboard back the way a click in another app does.
            let nsw = cx.update_window(h, |_, window, _| crate::mac::ns_window(window)).ok();
            if let Some(w) = nsw {
                w.resignKeyWindow();
            }
            sleep(cx, 300).await;
            let hidden = panel.read_with(cx, |p, _| p.hidden);
            let _ = cx.update_window(h, |_, window, _| {
                #[cfg(target_os = "macos")]
                println!("S1 STEP5 after_resign {}", crate::mac::describe_window(window));
            });
            println!("S1 STEP5 hidden_on_blur pass={hidden}");
        }
        if let Some(main) = others.first().copied() {
            let _ = cx.update_window(main, |_, window, _| window.activate_window());
            sleep(cx, 600).await;
            let _ = cx.update_window(h, |_, window, _| {
                #[cfg(target_os = "macos")]
                println!("S1 STEP5 after_blur {}", crate::mac::describe_window(window));
            });
            let hidden = panel.read_with(cx, |p, _| p.hidden);
            println!("S1 STEP5 hidden_on_blur pass={hidden}");
            // Show again (what the double-⇧ hotkey would do).
            panel.update(cx, |p, _| p.hidden = false);
            if let Ok(w) = cx.update_window(h, |_, window, _| crate::mac::ns_window(window)) {
                crate::mac::panel_show_ns(&w);
            }
            sleep(cx, 500).await;
            let _ = cx.update_window(h, |_, window, _| {
                #[cfg(target_os = "macos")]
                println!("S1 STEP5 reshown {}", crate::mac::describe_window(window));
            });
        }
        println!("S1 AUTOPILOT PANEL DONE");
    })
    .detach();
}
