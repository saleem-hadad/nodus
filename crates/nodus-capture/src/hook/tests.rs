//! The hook's translation of OS events, driven with synthetic ones, and the
//! double tap heard through it. No real hook is installed.

use std::sync::atomic::{AtomicU32, Ordering};

use super::mac_types::*;
use super::win_messages::*;
use super::*;

const SHIFT_FLAG: u64 = 0x2_0000;
const COMMAND_FLAG: u64 = 0x10_0000;
/// kVK_ANSI_A, and kVK_ANSI_Backslash, whose raw code is 42 like VC_SHIFT_L.
const KVK_A: i64 = 0x00;
const KVK_BACKSLASH: i64 = 0x2A;

fn flags(keycode: i64, flags: u64) -> Option<HookEvent> {
    mac_event(FLAGS_CHANGED, keycode, flags, None)
}

#[test]
fn reads_shift_from_flags_changed() {
    assert_eq!(flags(0x38, SHIFT_FLAG), Some(HookEvent::KeyDown(42)));
    assert_eq!(flags(0x38, 0), Some(HookEvent::KeyUp(42)));
    assert_eq!(flags(0x3C, SHIFT_FLAG | 0x100), Some(HookEvent::KeyDown(54)));
    assert_eq!(flags(0x3C, 0x100), Some(HookEvent::KeyUp(54)));
    // Letting go of the right Shift while the left is held still reads as a
    // press, as libuiohook reads the device-independent flag.
    assert_eq!(flags(0x3C, SHIFT_FLAG | 0x2), Some(HookEvent::KeyDown(54)));
}

#[test]
fn reads_other_modifiers_as_other_keys() {
    for (keycode, flag) in [
        (0x37, COMMAND_FLAG),
        (0x36, COMMAND_FLAG),
        (0x3B, 0x4_0000),
        (0x3E, 0x4_0000),
        (0x3A, 0x8_0000),
        (0x3D, 0x8_0000),
    ] {
        assert_eq!(flags(keycode, flag), Some(HookEvent::KeyDown(VC_OTHER)), "{keycode:#x}");
        assert_eq!(flags(keycode, 0), Some(HookEvent::KeyUp(VC_OTHER)), "{keycode:#x}");
    }
    // Caps Lock and Fn change flags, but aren't keys here.
    assert_eq!(flags(0x39, 0x1_0000), None);
    assert_eq!(flags(0x3F, 0x80_0000), None);
}

#[test]
fn reads_keys_and_clicks() {
    assert_eq!(
        mac_event(KEY_DOWN, KVK_A, SHIFT_FLAG, None),
        Some(HookEvent::KeyDown(VC_OTHER))
    );
    assert_eq!(mac_event(KEY_UP, KVK_A, 0, None), Some(HookEvent::KeyUp(VC_OTHER)));
    // Raw codes aren't libuiohook's: backslash isn't the left Shift.
    assert_eq!(
        mac_event(KEY_DOWN, KVK_BACKSLASH, 0, None),
        Some(HookEvent::KeyDown(VC_OTHER))
    );
    for kind in [LEFT_MOUSE_DOWN, RIGHT_MOUSE_DOWN, OTHER_MOUSE_DOWN, OTHER_MOUSE_UP] {
        assert_eq!(mac_event(kind, 0, 0, None), Some(HookEvent::MouseDown));
    }
    // Left and right buttons coming up, moves and the wheel aren't heard.
    for kind in [2, 4, 5, 6, 7, 22, 27] {
        assert_eq!(mac_event(kind, 0, 0, None), None);
    }
    assert_eq!(MASK & (1 << 2), 0);
    assert_ne!(MASK & (1 << FLAGS_CHANGED), 0);
}

#[test]
fn reads_caps_lock_and_media_keys() {
    let data1 = |code: u32, state: u32| (code << 16) | (state << 8);
    // Caps Lock down (state 0xA) and up (0xB).
    assert_eq!(
        mac_event(SYSTEM_DEFINED, 0, 0, Some((8, data1(4, 0xA)))),
        Some(HookEvent::KeyDown(VC_OTHER))
    );
    assert_eq!(
        mac_event(SYSTEM_DEFINED, 0, 0, Some((8, data1(4, 0xB)))),
        Some(HookEvent::KeyUp(VC_OTHER))
    );
    for code in [0, 1, 7, 14, 16, 19, 20] {
        assert!(mac_event(SYSTEM_DEFINED, 0, 0, Some((8, data1(code, 0xA)))).is_some());
    }
    // Brightness isn't one libuiohook reports; nor is another subtype.
    assert_eq!(mac_event(SYSTEM_DEFINED, 0, 0, Some((8, data1(2, 0xA)))), None);
    assert_eq!(mac_event(SYSTEM_DEFINED, 0, 0, Some((7, data1(4, 0xA)))), None);
    assert_eq!(mac_event(SYSTEM_DEFINED, 0, 0, None), None);
}

#[test]
fn reads_a_system_events_fields_from_its_data() {
    let mut data = vec![0u8; 132];
    data[120..124].copy_from_slice(&8u32.to_be_bytes());
    data[128..132].copy_from_slice(&0x0004_0A00u32.to_be_bytes());
    assert_eq!(system_fields(&data), Some((8, 0x0004_0A00)));
    assert_eq!(system_fields(&data[..131]), None);
}

#[test]
fn reads_windows_hooks() {
    assert_eq!(win_key(WM_KEYDOWN, 0xA0), Some(HookEvent::KeyDown(42)));
    assert_eq!(win_key(WM_KEYUP, 0xA1), Some(HookEvent::KeyUp(54)));
    assert_eq!(win_key(WM_SYSKEYDOWN, 0x10), Some(HookEvent::KeyDown(42)));
    assert_eq!(win_key(WM_SYSKEYUP, 0x41), Some(HookEvent::KeyUp(VC_OTHER)));
    // VK 0x2A (Print) and 0x36 ('6') aren't Shift keys.
    assert_eq!(win_key(WM_KEYDOWN, 0x2A), Some(HookEvent::KeyDown(VC_OTHER)));
    assert_eq!(win_key(WM_KEYDOWN, 0x36), Some(HookEvent::KeyDown(VC_OTHER)));
    assert_eq!(win_key(0x0102, 0xA0), None);
    for message in [
        WM_LBUTTONDOWN,
        WM_RBUTTONDOWN,
        WM_MBUTTONDOWN,
        WM_XBUTTONDOWN,
        WM_NCXBUTTONDOWN,
    ] {
        assert_eq!(win_mouse(message), Some(HookEvent::MouseDown));
    }
    // Ups, moves and the wheel.
    for message in [0x0202, 0x0205, 0x0208, 0x020C, 0x0200, 0x020A, 0x020E] {
        assert_eq!(win_mouse(message), None);
    }
}

/// A listener and how many double taps it heard.
fn listener() -> (Listener, Arc<AtomicU32>) {
    let fired = Arc::new(AtomicU32::new(0));
    let count = fired.clone();
    (
        Listener::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }),
        fired,
    )
}

/// Replays macOS events: (type, keycode, flags, at ms).
fn replay(listener: &Listener, events: &[(u32, i64, u64, f64)]) {
    for &(kind, keycode, flags, at) in events {
        if let Some(heard) = mac_event(kind, keycode, flags, None) {
            listener.hear_at(heard, at);
        }
    }
}

/// CAP-021…CAP-030, end to end on macOS events.
#[test]
fn hears_a_double_tap_through_the_macos_events() {
    let (l, fired) = listener();
    replay(
        &l,
        &[
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 0.0),
            (FLAGS_CHANGED, 0x38, 0, 60.0),
            (FLAGS_CHANGED, 0x3C, SHIFT_FLAG, 180.0),
            (FLAGS_CHANGED, 0x3C, 0, 240.0),
        ],
    );
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    // ⇧A twice, ⌘⇧ twice and a click between taps never fire.
    let (l, fired) = listener();
    replay(
        &l,
        &[
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 0.0),
            (KEY_DOWN, KVK_A, SHIFT_FLAG, 10.0),
            (KEY_UP, KVK_A, SHIFT_FLAG, 20.0),
            (FLAGS_CHANGED, 0x38, 0, 60.0),
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 150.0),
            (FLAGS_CHANGED, 0x37, SHIFT_FLAG | COMMAND_FLAG, 160.0),
            (FLAGS_CHANGED, 0x37, SHIFT_FLAG, 170.0),
            (FLAGS_CHANGED, 0x38, 0, 210.0),
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 300.0),
            (FLAGS_CHANGED, 0x38, 0, 340.0),
            (LEFT_MOUSE_DOWN, 0, 0, 360.0),
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 400.0),
            (FLAGS_CHANGED, 0x38, 0, 440.0),
        ],
    );
    assert_eq!(fired.load(Ordering::SeqCst), 0);

    // Both Shifts pressed together count as one tap, as under libuiohook.
    let (l, fired) = listener();
    replay(
        &l,
        &[
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 0.0),
            (FLAGS_CHANGED, 0x3C, SHIFT_FLAG, 10.0),
            (FLAGS_CHANGED, 0x3C, SHIFT_FLAG, 40.0),
            (FLAGS_CHANGED, 0x38, 0, 60.0),
            (FLAGS_CHANGED, 0x38, SHIFT_FLAG, 200.0),
            (FLAGS_CHANGED, 0x38, 0, 250.0),
        ],
    );
    assert_eq!(fired.load(Ordering::SeqCst), 1);
}

/// The same on Windows' messages, auto-repeat included.
#[test]
fn hears_a_double_tap_through_the_windows_messages() {
    let (l, fired) = listener();
    for (message, vk, at) in [
        (WM_KEYDOWN, 0xA0, 0.0),
        (WM_KEYDOWN, 0xA0, 30.0),
        (WM_KEYUP, 0xA0, 60.0),
        (WM_KEYDOWN, 0xA1, 180.0),
        (WM_KEYUP, 0xA1, 240.0),
    ] {
        l.hear_at(win_key(message, vk).unwrap(), at);
    }
    assert_eq!(fired.load(Ordering::SeqCst), 1);
}

/// CAP-030: times come from the clock the listener reads, not the OS's.
#[test]
fn times_events_when_they_are_heard() {
    let (l, fired) = listener();
    l.hear(HookEvent::KeyDown(42));
    l.hear(HookEvent::KeyUp(42));
    l.hear(HookEvent::KeyDown(54));
    l.hear(HookEvent::KeyUp(54));
    assert_eq!(fired.load(Ordering::SeqCst), 1);
}
