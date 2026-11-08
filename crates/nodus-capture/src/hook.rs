//! The global key hook that hears the double tap: what 0.9.4 got from
//! uiohook-napi (libuiohook), rebuilt on the OS directly.
//!
//! - macOS: a CGEventTap at the session level, inserted at the head, as an
//!   **active** tap (`kCGEventTapOptionDefault`), as libuiohook makes it: an
//!   active tap is gated on Accessibility, the permission 0.9.4 asks for
//!   (a listen-only one falls under Input Monitoring). It runs on a thread
//!   of its own with a CFRunLoop, passes every event on unchanged, and is
//!   switched back on when macOS switches it off for being slow or for
//!   secure input.
//! - Windows: low-level keyboard and mouse hooks on a thread with a message
//!   loop, always calling the next hook.
//!
//! Each OS event is translated into libuiohook's terms (key codes 42 and 54
//! for the Shift keys, key down/up, mouse down) by pure functions, which the
//! tests drive with synthetic events; a real hook is never installed in a
//! test (it needs Accessibility).

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::double_tap::{DoubleTap, VC_SHIFT_L, VC_SHIFT_R};

/// libuiohook's `VC_UNDEFINED`: any key that isn't a Shift key, as far as
/// the double tap cares.
pub const VC_OTHER: u16 = 0;

/// What the hook hears, in libuiohook's terms (`keydown`, `keyup`, `mousedown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    KeyDown(u16),
    KeyUp(u16),
    /// Any mouse button pressed.
    MouseDown,
}

/// macOS event types (`CGEventType`) the tap listens to.
pub mod mac_types {
    pub const LEFT_MOUSE_DOWN: u32 = 1;
    pub const RIGHT_MOUSE_DOWN: u32 = 3;
    pub const KEY_DOWN: u32 = 10;
    pub const KEY_UP: u32 = 11;
    pub const FLAGS_CHANGED: u32 = 12;
    /// `NX_SYSDEFINED`: Caps Lock and the media keys.
    pub const SYSTEM_DEFINED: u32 = 14;
    pub const OTHER_MOUSE_DOWN: u32 = 25;
    pub const OTHER_MOUSE_UP: u32 = 26;
    pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    pub const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

    /// The events the tap asks for: keys, modifiers, system keys and the
    /// mouse buttons that count.
    pub const MASK: u64 = 1 << LEFT_MOUSE_DOWN
        | 1 << RIGHT_MOUSE_DOWN
        | 1 << KEY_DOWN
        | 1 << KEY_UP
        | 1 << FLAGS_CHANGED
        | 1 << SYSTEM_DEFINED
        | 1 << OTHER_MOUSE_DOWN
        | 1 << OTHER_MOUSE_UP;
}

/// macOS virtual key codes of the modifiers (`kVK_*`).
mod kvk {
    pub const COMMAND: i64 = 0x37;
    pub const SHIFT: i64 = 0x38;
    pub const OPTION: i64 = 0x3A;
    pub const CONTROL: i64 = 0x3B;
    pub const RIGHT_COMMAND: i64 = 0x36;
    pub const RIGHT_SHIFT: i64 = 0x3C;
    pub const RIGHT_OPTION: i64 = 0x3D;
    pub const RIGHT_CONTROL: i64 = 0x3E;
}

/// `CGEventFlags` masks: device-independent, as libuiohook reads them.
mod mask {
    pub const SHIFT: u64 = 0x0002_0000;
    pub const CONTROL: u64 = 0x0004_0000;
    pub const ALTERNATE: u64 = 0x0008_0000;
    pub const COMMAND: u64 = 0x0010_0000;
}

/// The system keys libuiohook turns into key events (`NX_KEYTYPE_*`):
/// sound up, sound down, Caps Lock, mute, eject, play, fast, rewind.
const SYSTEM_KEYS: [u32; 8] = [0, 1, 4, 7, 14, 16, 19, 20];

/// A macOS key code in libuiohook's terms: only the Shift keys matter.
fn mac_code(keycode: i64) -> u16 {
    match keycode {
        kvk::SHIFT => VC_SHIFT_L,
        kvk::RIGHT_SHIFT => VC_SHIFT_R,
        _ => VC_OTHER,
    }
}

/// A macOS event as libuiohook reports it (its `hook_event_proc`).
///
/// - Key down and up are keys (auto-repeat included).
/// - Modifiers arrive as `flagsChanged`: Shift, ⌃, ⌥ and ⌘ (left and right)
///   are down while their device-independent flag is set and up when it's
///   cleared, so letting go of one Shift while the other is held reads as
///   another press of it, as libuiohook has it. Caps Lock and Fn here are
///   nothing; Caps Lock comes as a system key.
/// - A system-defined event of subtype 8 is a key for Caps Lock and the
///   media keys: down when bit 0 of its state is clear.
/// - Left, right and other mouse buttons going down are mouse downs, and so
///   is another button coming up (libuiohook calls its pressed handler there).
pub fn mac_event(event_type: u32, keycode: i64, flags: u64, system: Option<(u32, u32)>) -> Option<HookEvent> {
    use mac_types::*;
    let key = |down: bool, code: u16| {
        Some(if down {
            HookEvent::KeyDown(code)
        } else {
            HookEvent::KeyUp(code)
        })
    };
    match event_type {
        KEY_DOWN => key(true, mac_code(keycode)),
        KEY_UP => key(false, mac_code(keycode)),
        FLAGS_CHANGED => {
            let held = |m: u64| flags & m != 0;
            match keycode {
                kvk::SHIFT | kvk::RIGHT_SHIFT => key(held(mask::SHIFT), mac_code(keycode)),
                kvk::CONTROL | kvk::RIGHT_CONTROL => key(held(mask::CONTROL), VC_OTHER),
                kvk::OPTION | kvk::RIGHT_OPTION => key(held(mask::ALTERNATE), VC_OTHER),
                kvk::COMMAND | kvk::RIGHT_COMMAND => key(held(mask::COMMAND), VC_OTHER),
                _ => None,
            }
        }
        SYSTEM_DEFINED => {
            let (subtype, data1) = system?;
            let code = (data1 & 0xFFFF_0000) >> 16;
            if subtype != 8 || !SYSTEM_KEYS.contains(&code) {
                return None;
            }
            let state = (data1 & 0xFF00) >> 8;
            key(state & 0x1 == 0, VC_OTHER)
        }
        LEFT_MOUSE_DOWN | RIGHT_MOUSE_DOWN | OTHER_MOUSE_DOWN | OTHER_MOUSE_UP => Some(HookEvent::MouseDown),
        _ => None,
    }
}

/// A system-defined event's subtype and `data1`, read from the bytes
/// `CGEventCreateData` makes of it (big-endian, at 120 and 128), as
/// libuiohook reads them.
pub fn system_fields(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 132 {
        return None;
    }
    let at = |i: usize| u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    Some((at(120), at(128)))
}

/// Windows messages the hooks hear.
pub mod win_messages {
    pub const WM_KEYDOWN: u32 = 0x0100;
    pub const WM_KEYUP: u32 = 0x0101;
    pub const WM_SYSKEYDOWN: u32 = 0x0104;
    pub const WM_SYSKEYUP: u32 = 0x0105;
    pub const WM_LBUTTONDOWN: u32 = 0x0201;
    pub const WM_RBUTTONDOWN: u32 = 0x0204;
    pub const WM_MBUTTONDOWN: u32 = 0x0207;
    pub const WM_XBUTTONDOWN: u32 = 0x020B;
    pub const WM_NCXBUTTONDOWN: u32 = 0x00AB;
}

/// A Windows virtual key in libuiohook's terms: `VK_SHIFT` and `VK_LSHIFT`
/// are the left Shift, `VK_RSHIFT` the right.
fn win_code(vk: u32) -> u16 {
    match vk {
        0x10 | 0xA0 => VC_SHIFT_L,
        0xA1 => VC_SHIFT_R,
        _ => VC_OTHER,
    }
}

/// A low-level keyboard message as libuiohook reports it.
pub fn win_key(message: u32, vk: u32) -> Option<HookEvent> {
    use win_messages::*;
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(HookEvent::KeyDown(win_code(vk))),
        WM_KEYUP | WM_SYSKEYUP => Some(HookEvent::KeyUp(win_code(vk))),
        _ => None,
    }
}

/// A low-level mouse message: any button going down.
pub fn win_mouse(message: u32) -> Option<HookEvent> {
    use win_messages::*;
    matches!(
        message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCXBUTTONDOWN
    )
    .then_some(HookEvent::MouseDown)
}

/// Where the hook's events go, from the hook's own thread. It must be quick.
pub type HookSink = Arc<dyn Fn(HookEvent) + Send + Sync>;

/// A global key hook that can be started.
pub trait KeyHook: Send + Sync {
    /// Starts hearing keys and clicks in every app (CAP-034). Fails when the
    /// OS won't let it.
    fn start(&self, sink: HookSink) -> Result<Box<dyn RunningHook>, String>;
}

/// A started hook.
pub trait RunningHook: Send {
    /// Stops it (CAP-035).
    fn stop(self: Box<Self>) -> Result<(), String>;
}

/// The OS's hook: a CGEventTap on macOS, low-level hooks on Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemHook;

impl KeyHook for SystemHook {
    fn start(&self, sink: HookSink) -> Result<Box<dyn RunningHook>, String> {
        #[cfg(target_os = "macos")]
        return mac::start(sink).map(|hook| Box::new(hook) as Box<dyn RunningHook>);
        #[cfg(windows)]
        return windows::start(sink).map(|hook| Box::new(hook) as Box<dyn RunningHook>);
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = sink;
            Err("no key hook on this OS".into())
        }
    }
}

/// Feeds what the hook hears to a [`DoubleTap`], timed when it's heard on a
/// monotonic clock, never by the OS's own event times (CAP-030), and calls
/// `on_double_tap` when Shift is tapped twice.
pub struct Listener {
    tap: Mutex<DoubleTap>,
    started: Instant,
    on_double_tap: Box<dyn Fn() + Send + Sync>,
}

impl Listener {
    pub fn new(on_double_tap: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            tap: Mutex::new(DoubleTap::new()),
            started: Instant::now(),
            on_double_tap: Box::new(on_double_tap),
        }
    }

    /// An event heard now.
    pub fn hear(&self, event: HookEvent) {
        let at = self.started.elapsed().as_secs_f64() * 1000.0;
        self.hear_at(event, at);
    }

    /// An event heard at `at` ms (tests).
    pub fn hear_at(&self, event: HookEvent, at: f64) {
        let fired = {
            let mut tap = self.tap.lock().unwrap_or_else(PoisonError::into_inner);
            match event {
                HookEvent::KeyDown(code) => {
                    tap.key_down(code, at);
                    false
                }
                HookEvent::KeyUp(code) => tap.key_up(code, at),
                // CAP-029
                HookEvent::MouseDown => {
                    tap.other();
                    false
                }
            }
        };
        if fired {
            (self.on_double_tap)();
        }
    }
}

#[cfg(test)]
mod tests;
