//! What an app says of itself on a server board (SRV-180): which board,
//! where its pointer is and what it's looking at. The server answers with
//! `ev:presence` (`nodus_domain::Presence`), naming the sender from its
//! token, never from the frame.

use std::time::Duration;

use nodus_domain::{BoardPoint, Viewport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Frame;

/// An app sends presence at most this often, and a server relays it to a
/// board at most this often: 20 times a second.
pub const PRESENCE_EVERY: Duration = Duration::from_millis(50);

/// A peer drops out this long after its last frame.
pub const PRESENCE_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest project id a frame may name.
const MAX_PROJECT: usize = 128;
/// Coordinates past this are nobody's.
const MAX_COORD: f64 = 1e7;

/// `{ "presence": { project, cursor, viewport } }`. Fields it doesn't know,
/// a `login` among them, are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceFrame {
    /// The board the app has open; none once it has left every board, which
    /// takes it off at once.
    pub project: Option<String>,
    /// The pointer on the board, in flow units; none while it's off it.
    pub cursor: Option<BoardPoint>,
    pub viewport: Option<Viewport>,
}

impl PresenceFrame {
    /// The frame's text.
    pub fn to_text(&self) -> String {
        Frame::Presence(serde_json::to_value(self).expect("a presence frame is always JSON")).to_text()
    }

    /// A frame as it came, if it's one: a project that's a short string or
    /// null, and finite numbers. A cursor or viewport that isn't one reads
    /// as none.
    pub fn read(value: &Value) -> Option<Self> {
        let Value::Object(fields) = value else {
            return None;
        };
        let project = match fields.get("project") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if !id.is_empty() && id.len() <= MAX_PROJECT => Some(id.clone()),
            Some(_) => return None,
        };
        let number = |value: &Value, key: &str| {
            value
                .get(key)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.abs() <= MAX_COORD)
        };
        let cursor = fields.get("cursor").and_then(|cursor| {
            Some(BoardPoint {
                x: number(cursor, "x")?,
                y: number(cursor, "y")?,
            })
        });
        let viewport = fields.get("viewport").and_then(|viewport| {
            Some(Viewport {
                x: number(viewport, "x")?,
                y: number(viewport, "y")?,
                zoom: number(viewport, "zoom").filter(|zoom| *zoom > 0.0)?,
            })
        });
        // Nothing is anywhere on no board.
        let on = project.is_some();
        Some(Self {
            project,
            cursor: cursor.filter(|_| on),
            viewport: viewport.filter(|_| on),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_and_reads_a_frame() {
        let frame = PresenceFrame {
            project: Some("p1".into()),
            cursor: Some(BoardPoint { x: 12.5, y: -3.0 }),
            viewport: Some(Viewport {
                x: 100.0,
                y: 0.0,
                zoom: 1.25,
            }),
        };
        let text = frame.to_text();
        assert_eq!(
            text,
            r#"{"presence":{"project":"p1","cursor":{"x":12.5,"y":-3},"viewport":{"x":100,"y":0,"zoom":1.25}}}"#
        );
        let Ok(Frame::Presence(value)) = Frame::parse(&text) else {
            panic!("a presence frame");
        };
        assert_eq!(PresenceFrame::read(&value), Some(frame));
    }

    #[test]
    fn ignores_who_it_says_it_is_and_what_doesnt_fit() {
        let read = PresenceFrame::read(&json!({
            "project": "p1", "login": "mallory", "device": "x",
            "cursor": { "x": "1", "y": 2 }, "viewport": { "x": 0, "y": 0, "zoom": 0 }
        }))
        .unwrap();
        assert_eq!(
            read,
            PresenceFrame {
                project: Some("p1".into()),
                cursor: None,
                viewport: None,
            }
        );
        // Leaving every board.
        let left = PresenceFrame::read(&json!({ "project": null, "cursor": { "x": 1, "y": 2 } })).unwrap();
        assert_eq!((left.project, left.cursor), (None, None));
        for wrong in [
            json!(null),
            json!([1]),
            json!({ "project": 7 }),
            json!({ "project": "" }),
        ] {
            assert_eq!(PresenceFrame::read(&wrong), None, "{wrong}");
        }
        let long = "p".repeat(MAX_PROJECT + 1);
        assert_eq!(PresenceFrame::read(&json!({ "project": long })), None);
    }
}
