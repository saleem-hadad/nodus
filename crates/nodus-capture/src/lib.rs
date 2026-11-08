//! Floating capture: double-tap Shift in any app and a panel floats up over
//! it. This owns hearing the double tap, what comes along from the screen (the
//! app in front, its browser tab, a screenshot, the clipboard), Claude's guess
//! at what a capture is and where it goes, and making it on the board.
//!
//! Ports `src/main/capture.ts`, `src/main/doubleTap.ts`,
//! `src/main/screenContext.ts`, `src/main/captureGuess.ts` and the text
//! helpers in `src/shared/capture.ts`.

pub mod double_tap;
pub mod guess;
pub mod hook;
pub mod page;
pub mod screen;
pub mod service;
pub mod submit;
pub mod targets;
pub mod text;

pub use double_tap::{DoubleTap, GAP_MS, SHIFT_KEYS, TAP_MS};
pub use guess::{ClaudeEnv, GuessInput, Guesser};
pub use hook::{HookEvent, KeyHook, RunningHook, SystemHook};
pub use screen::{Clipboard, FrontApp, Screenshot, SystemClipboard};
pub use service::{Capture, CaptureDeps, PanelWindow, Permissions, Screen, SystemPermissions, SystemScreen};
pub use submit::{CaptureHandlers, SubmitError};
pub use targets::CaptureSessions;
pub use text::{TITLE_MAX, capture_kind, describe_attachments, fence, title_from, with_screen};
