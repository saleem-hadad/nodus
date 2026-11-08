//! Pseudo-terminals, behaving as node-pty 1.1.0 does in the 0.9.4 host.
//!
//! portable-pty doesn't match it where the host relies on it, so this layer
//! is the host's own:
//!
//! - **macOS** (`unix`): a `cwd` that can't be entered or a `file` that can't be
//!   run still spawns, prints nothing and exits 1, as node-pty's spawn helper
//!   does (HOST-053); a process ended by a signal reports exit code 0
//!   (HOST-058); signals go to the process, not its group (HOST-062); every
//!   byte of output arrives before the exit, which waits up to 200 ms for the
//!   terminal to close (HOST-057); `PWD` and `TERM` are set (HOST-049).
//! - **Windows** (`windows`): the process gets one raw command line (HOST-051),
//!   a missing program is `File not found: <path>` (HOST-052), and killing it
//!   ends every process attached to its console, then closes the console
//!   (HOST-063).
//!
//! Output is decoded as UTF-8 across reads, invalid bytes as U+FFFD (HOST-055).

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::Pty;
#[cfg(windows)]
pub use windows::Pty;

/// What to run, as the host's `spawn` gives it.
#[derive(Debug, Clone, Default)]
pub struct PtyOptions {
    /// The program; empty means node-pty's default (`sh`, or `cmd.exe` on Windows).
    pub file: String,
    /// Its arguments: the argv after `file` on a Unix, joined into one command
    /// line on Windows.
    pub args: Vec<String>,
    /// The whole environment, in order.
    pub env: Vec<(String, String)>,
    /// Where it starts; empty means the host's own folder.
    pub cwd: String,
    /// The size, as JavaScript numbers: fractions are cut off as node-pty's
    /// native side does, and 0 or NaN mean 80 × 24.
    pub cols: f64,
    pub rows: f64,
}

/// What a pseudo-terminal reports, in order: output, then once, the exit.
#[derive(Debug, Clone, PartialEq)]
pub enum PtyEvent {
    Data(String),
    /// The exit code: 0 when a signal ended it on a Unix.
    Exit(i64),
}

/// node-pty's `DEFAULT_COLS` and `DEFAULT_ROWS`.
const DEFAULT_COLS: f64 = 80.0;
const DEFAULT_ROWS: f64 = 24.0;

/// `opt.cols || DEFAULT_COLS`: 0 and NaN fall back to the default.
fn or_default(value: f64, default: f64) -> f64 {
    if value == 0.0 || value.is_nan() { default } else { value }
}

/// V8's `Number::Int32Value` (ECMAScript ToInt32): NaN and the infinities
/// are 0, fractions are cut off, and the rest wraps modulo 2³².
pub(crate) fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let truncated = value.trunc();
    let modulo = truncated.rem_euclid(4_294_967_296.0);
    let unsigned = modulo as u32;
    unsigned as i32
}

/// node-pty's check before resizing.
fn check_size(cols: f64, rows: f64) -> Result<(), String> {
    if cols <= 0.0 || rows <= 0.0 || cols.is_nan() || rows.is_nan() || cols.is_infinite() || rows.is_infinite() {
        return Err("resizing must be done using positive cols and rows".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_sizes_as_v8_does() {
        assert_eq!(to_int32(80.9), 80);
        assert_eq!(to_int32(-1.5), -1);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(4_294_967_296.0 + 7.0), 7);
        assert_eq!(to_int32(2_147_483_648.0), i32::MIN);
        assert_eq!(or_default(0.0, DEFAULT_COLS), 80.0);
        assert_eq!(or_default(f64::NAN, DEFAULT_ROWS), 24.0);
        assert_eq!(or_default(132.0, DEFAULT_COLS), 132.0);
    }

    #[test]
    fn refuses_sizes_node_pty_refuses() {
        assert!(check_size(80.0, 24.0).is_ok());
        for (cols, rows) in [
            (0.0, 24.0),
            (80.0, -1.0),
            (f64::NAN, 24.0),
            (f64::INFINITY, 24.0),
            (80.0, f64::NEG_INFINITY),
        ] {
            assert_eq!(
                check_size(cols, rows).unwrap_err(),
                "resizing must be done using positive cols and rows"
            );
        }
    }
}
