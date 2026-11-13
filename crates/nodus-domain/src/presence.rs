//! Who else is on a server board, where their pointer is and what they're
//! looking at (SRV-180…182). A server relays it as `ev:presence`; it's
//! never saved.

use serde::{Deserialize, Serialize};

use crate::board::Viewport;

/// A point on the board, in flow units (the board's own coordinates, not
/// the screen's).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoardPoint {
    pub x: f64,
    pub y: f64,
}

/// Someone else on the board: one of a member's devices (SRV-180).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresencePeer {
    /// Their GitHub login, from their device's token, never their word.
    pub login: String,
    /// The device, so one person's two devices are two peers.
    pub device: String,
    /// `#rrggbb`, worked out from the login, so it's the same everywhere.
    pub color: String,
    /// Their pointer on the board; none while it's off the board.
    pub cursor: Option<BoardPoint>,
    /// What they're looking at; follow mode tracks it (SRV-182).
    pub viewport: Option<Viewport>,
}

/// `ev:presence`: everyone else on one board, at most 20 times a second.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    /// The project whose board it is.
    pub project: String,
    /// Everyone on it but the connection told, in the order they came.
    pub peers: Vec<PresencePeer>,
}

/// The colours presence picks from: bright enough on both themes, and far
/// enough apart to tell two people apart.
pub const PRESENCE_COLORS: [&str; 10] = [
    "#e5484d", "#f76b15", "#e2a336", "#46a758", "#12a594", "#0090ff", "#3e63dd", "#8e4ec6", "#d6409f", "#978365",
];

/// A login's colour (SRV-181): the same for the same login, whatever its
/// case, on every server and device. FNV-1a over the lower-cased login.
pub fn presence_color(login: &str) -> &'static str {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in login.to_ascii_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    PRESENCE_COLORS[hash as usize % PRESENCE_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_keeps_its_colour() {
        assert_eq!(presence_color("octo"), presence_color("Octo"));
        assert_eq!(presence_color("octo"), presence_color("octo"));
        // Not everyone gets the same one.
        let colors: std::collections::HashSet<_> = ["octo", "alice", "bob", "carol", "dave", "erin"]
            .iter()
            .map(|login| presence_color(login))
            .collect();
        assert!(colors.len() >= 4, "{colors:?}");
    }

    #[test]
    fn goes_over_the_wire_with_null_for_nothing() {
        let presence = Presence {
            project: "p1".into(),
            peers: vec![PresencePeer {
                login: "octo".into(),
                device: "d1".into(),
                color: "#0090ff".into(),
                cursor: Some(BoardPoint { x: 10.5, y: -2.0 }),
                viewport: None,
            }],
        };
        assert_eq!(
            crate::json::to_string(&presence).unwrap(),
            r##"{"project":"p1","peers":[{"login":"octo","device":"d1","color":"#0090ff","cursor":{"x":10.5,"y":-2},"viewport":null}]}"##
        );
    }
}
