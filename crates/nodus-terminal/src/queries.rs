//! The few sequences xterm.js answers or acts on that alacritty's parser never
//! hands to a handler: DECXCPR (`CSI ? 6 n`), DECRQSS (`DCS $ q Pt ST`) and
//! DECSTR (`CSI ! p`). A small scanner follows the same bytes alongside the
//! parser and says where each one ends, so the terminal can act on it at that
//! point in the stream, after everything before it and before everything after.

/// A sequence the scanner found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SideSequence {
    /// `CSI ? 6 n`: the cursor position as `CSI ? y ; x R`.
    CursorPositionPrivate,
    /// `DCS $ q Pt ST`, with `Pt`.
    RequestStatusString(String),
    /// `CSI ! p`, the soft reset.
    SoftReset,
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi {
        params: String,
        intermediates: String,
    },
    /// OSC, SOS, PM and APC strings, skipped to their end.
    SkipString,
    DcsHeader {
        params: String,
        intermediates: String,
    },
    DcsData {
        request: bool,
        data: String,
    },
}

/// The longest parameter or data string worth keeping.
const LIMIT: usize = 64;

#[derive(Debug, Clone, Default)]
pub(crate) struct Scanner {
    state: State,
}

impl Scanner {
    /// Scans `bytes`, returning each found sequence with the offset just past
    /// the byte that completed it.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<(usize, SideSequence)> {
        let mut found = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            if let Some(seq) = self.step(b) {
                found.push((i + 1, seq));
            }
        }
        found
    }

    fn step(&mut self, b: u8) -> Option<SideSequence> {
        // CAN and SUB cancel any sequence.
        if b == 0x18 || b == 0x1a {
            self.state = State::Ground;
            return None;
        }
        match &mut self.state {
            State::Ground => {
                if b == 0x1b {
                    self.state = State::Escape;
                }
                None
            }
            State::Escape => {
                self.state = match b {
                    b'[' => State::Csi {
                        params: String::new(),
                        intermediates: String::new(),
                    },
                    b'P' => State::DcsHeader {
                        params: String::new(),
                        intermediates: String::new(),
                    },
                    b']' | b'X' | b'^' | b'_' => State::SkipString,
                    0x1b => State::Escape,
                    // C0 controls inside an escape are executed and the escape goes on.
                    0x00..=0x1f => State::Escape,
                    // Intermediates (ESC ( B …) then a final byte.
                    0x20..=0x2f => State::Escape,
                    _ => State::Ground,
                };
                None
            }
            State::Csi { params, intermediates } => match b {
                0x1b => {
                    self.state = State::Escape;
                    None
                }
                0x00..=0x1f => None,
                0x20..=0x2f => {
                    if intermediates.len() < LIMIT {
                        intermediates.push(b as char);
                    }
                    None
                }
                0x30..=0x3f => {
                    if params.len() < LIMIT {
                        params.push(b as char);
                    }
                    None
                }
                0x40..=0x7e => {
                    let seq = match (b, params.as_str(), intermediates.as_str()) {
                        (b'n', p, "") if p.starts_with('?') && first_param(&p[1..]) == Some(6) => {
                            Some(SideSequence::CursorPositionPrivate)
                        }
                        (b'p', p, "!") if !p.starts_with(['?', '>', '=', '<']) => Some(SideSequence::SoftReset),
                        _ => None,
                    };
                    self.state = State::Ground;
                    seq
                }
                _ => None,
            },
            State::SkipString => {
                match b {
                    0x07 => self.state = State::Ground,
                    0x1b => self.state = State::Escape,
                    _ => {}
                }
                None
            }
            State::DcsHeader { params, intermediates } => match b {
                0x1b => {
                    self.state = State::Escape;
                    None
                }
                0x00..=0x1f => None,
                0x20..=0x2f => {
                    if intermediates.len() < LIMIT {
                        intermediates.push(b as char);
                    }
                    None
                }
                0x30..=0x3f => {
                    if params.len() < LIMIT {
                        params.push(b as char);
                    }
                    None
                }
                _ => {
                    // The final byte: xterm only answers `DCS $ q`, whatever the params.
                    let request =
                        b == b'q' && intermediates.as_str() == "$" && !params.starts_with(['?', '>', '=', '<']);
                    self.state = State::DcsData {
                        request,
                        data: String::new(),
                    };
                    None
                }
            },
            State::DcsData { request, data } => {
                if b == 0x1b {
                    // ESC ends the string (it is the start of ST).
                    let seq = request.then(|| SideSequence::RequestStatusString(std::mem::take(data)));
                    self.state = State::Escape;
                    return seq;
                }
                if data.len() < LIMIT {
                    data.push(b as char);
                }
                None
            }
        }
    }
}

fn first_param(params: &str) -> Option<u32> {
    let first = params.split([';', ':']).next()?;
    if first.is_empty() {
        return Some(0);
    }
    first.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sequences_where_they_end() {
        let mut s = Scanner::default();
        let input = b"ab\x1b[?6nc\x1bP$qm\x1b\\d\x1b[!p";
        assert_eq!(
            s.scan(input),
            vec![
                (7, SideSequence::CursorPositionPrivate),
                (14, SideSequence::RequestStatusString("m".into())),
                (20, SideSequence::SoftReset),
            ]
        );
    }

    #[test]
    fn follows_sequences_split_across_chunks() {
        let mut s = Scanner::default();
        assert!(s.scan(b"\x1bP$").is_empty());
        assert!(s.scan(b"q r").is_empty());
        assert_eq!(
            s.scan(b"\x1b\\"),
            vec![(1, SideSequence::RequestStatusString(" r".into()))]
        );
    }

    #[test]
    fn ignores_what_xterm_ignores() {
        let mut s = Scanner::default();
        assert!(
            s.scan(b"\x1b[6n\x1b[?5n\x1b]0;?6n\x07\x1bPq#0\x1b\\\x1b[?!p")
                .is_empty()
        );
    }
}
