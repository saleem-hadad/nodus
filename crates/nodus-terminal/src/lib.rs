//! The terminal model behind every terminal in the app, with no UI: a grid
//! over alacritty_terminal, xterm-compatible key encoding, answers to the
//! queries programs send a terminal, selection, and replaying a session's
//! buffered output in order. The GPUI element that draws it is in `nodus-ui`.
//!
//! Ports the xterm.js behavior in `src/renderer/src/terminal/`.
