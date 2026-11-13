//! The three kinds of frame on a server's WebSocket (SRV-023), read and
//! written as JavaScript reads and writes JSON (`nodus_domain::json`).

use nodus_domain::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{NOTICE_EVENT, Notice};

/// One text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// `{ "id": 7, "call": "board:load", "args": {…} }`, from a client.
    Call(CallFrame),
    /// `{ "id": 7, "ok": … }` or `{ "id": 7, "err": { "channel", "message" } }`,
    /// from the server.
    Answer(Answer),
    /// `{ "ev": "pty-data", "p": {…} }`, from the server.
    Event(Event),
    /// `{ "ev": "server-notice", "p": { "kind", "message" } }`, from the
    /// server: the server's own news for every member (SRV-042), which the
    /// core's events don't carry.
    Notice(Notice),
    /// `{ "presence": {…} }`, from a client: its cursor and viewport on a
    /// board (SRV-180), as it came; [`crate::PresenceFrame::read`] reads it.
    Presence(Value),
}

/// A call as it arrives, before its name is checked: any name, any arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct CallFrame {
    /// Chosen by the client; its answer carries it back.
    pub id: u64,
    pub call: String,
    /// `null` when the frame had none.
    pub args: Value,
}

/// A call's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub id: u64,
    pub result: Result<Value, CallError>,
}

/// A call that failed: the channel `CommandError` carries and its message,
/// verbatim, so the app shows what it shows for a local project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct CallError {
    pub channel: String,
    pub message: String,
}

impl CallError {
    pub fn new(channel: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            message: message.into(),
        }
    }
}

/// A text frame that isn't one of the three.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("not JSON: {0}")]
    Json(serde_json::Error),
    #[error("not a frame")]
    Unknown,
    #[error("not an event: {0}")]
    Event(serde_json::Error),
}

impl Frame {
    /// Reads a text frame.
    pub fn parse(text: &str) -> Result<Self, FrameError> {
        let value: Value = nodus_domain::json::from_str(text).map_err(FrameError::Json)?;
        let Value::Object(mut frame) = value else {
            return Err(FrameError::Unknown);
        };
        if frame.get("ev").and_then(Value::as_str) == Some(NOTICE_EVENT) {
            let notice = frame.shift_remove("p").unwrap_or(Value::Null);
            return serde_json::from_value(notice)
                .map(Frame::Notice)
                .map_err(FrameError::Event);
        }
        if frame.contains_key("ev") {
            return serde_json::from_value(Value::Object(frame))
                .map(Frame::Event)
                .map_err(FrameError::Event);
        }
        if let Some(presence) = frame.shift_remove("presence") {
            return Ok(Frame::Presence(presence));
        }
        let id = frame.get("id").and_then(Value::as_u64).ok_or(FrameError::Unknown)?;
        if let Some(call) = frame.get("call") {
            let call = call.as_str().ok_or(FrameError::Unknown)?.to_owned();
            let args = frame.shift_remove("args").unwrap_or(Value::Null);
            return Ok(Frame::Call(CallFrame { id, call, args }));
        }
        let result = match (frame.shift_remove("ok"), frame.shift_remove("err")) {
            (_, Some(err)) => Err(serde_json::from_value(err).map_err(|_| FrameError::Unknown)?),
            (Some(ok), None) => Ok(ok),
            (None, None) => return Err(FrameError::Unknown),
        };
        Ok(Frame::Answer(Answer { id, result }))
    }

    /// The frame's text.
    pub fn to_text(&self) -> String {
        let value = match self {
            Self::Call(call) => object([
                ("id", Value::from(call.id)),
                ("call", Value::String(call.call.clone())),
                ("args", call.args.clone()),
            ]),
            Self::Answer(Answer { id, result: Ok(ok) }) => object([("id", Value::from(*id)), ("ok", ok.clone())]),
            Self::Answer(Answer { id, result: Err(err) }) => object([
                ("id", Value::from(*id)),
                (
                    "err",
                    object([
                        ("channel", err.channel.as_str().into()),
                        ("message", err.message.as_str().into()),
                    ]),
                ),
            ]),
            Self::Event(event) => return nodus_domain::json::to_string(event).expect("an event is always JSON"),
            Self::Notice(notice) => object([
                ("ev", NOTICE_EVENT.into()),
                ("p", serde_json::to_value(notice).expect("a notice is always JSON")),
            ]),
            Self::Presence(presence) => object([("presence", presence.clone())]),
        };
        nodus_domain::json::to_string(&value).expect("a frame is always JSON")
    }
}

/// An object with these keys, in this order.
fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Map<_, _>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Request;
    use serde_json::json;

    fn round_trip(frame: &Frame, text: &str) {
        assert_eq!(frame.to_text(), text);
        assert_eq!(&Frame::parse(text).unwrap(), frame);
    }

    #[test]
    fn writes_and_reads_each_kind() {
        round_trip(
            &Frame::Call(CallFrame {
                id: 7,
                call: "board:load".into(),
                args: json!({ "projectId": "p1" }),
            }),
            r#"{"id":7,"call":"board:load","args":{"projectId":"p1"}}"#,
        );
        round_trip(
            &Frame::Answer(Answer {
                id: 7,
                result: Ok(json!({ "x": 1, "y": 2.5 })),
            }),
            r#"{"id":7,"ok":{"x":1,"y":2.5}}"#,
        );
        // Numbers as JavaScript writes them.
        let whole = Frame::Answer(Answer {
            id: 7,
            result: Ok(json!({ "at": 1_700_000_000_000.0 })),
        });
        assert_eq!(whole.to_text(), r#"{"id":7,"ok":{"at":1700000000000}}"#);
        // A command with nothing to give answers null, which is still an answer.
        round_trip(
            &Frame::Answer(Answer {
                id: 8,
                result: Ok(Value::Null),
            }),
            r#"{"id":8,"ok":null}"#,
        );
        round_trip(
            &Frame::Answer(Answer {
                id: 9,
                result: Err(CallError::new("board:load", "Unknown project nope")),
            }),
            r#"{"id":9,"err":{"channel":"board:load","message":"Unknown project nope"}}"#,
        );
        round_trip(
            &Frame::Event(Event::BoardChanged {
                project_id: "p1".into(),
            }),
            r#"{"ev":"board-changed","p":{"projectId":"p1"}}"#,
        );
        round_trip(
            &Frame::Notice(Notice {
                kind: "device-signed-in".into(),
                message: "Saleem signed in on Studio".into(),
            }),
            r#"{"ev":"server-notice","p":{"kind":"device-signed-in","message":"Saleem signed in on Studio"}}"#,
        );
        round_trip(
            &Frame::Presence(json!({ "project": "p1", "cursor": null })),
            r#"{"presence":{"project":"p1","cursor":null}}"#,
        );
    }

    #[test]
    fn reads_a_call_without_arguments() {
        let Frame::Call(call) = Frame::parse(r#"{"call":"projects:list","id":1}"#).unwrap() else {
            panic!("a call");
        };
        assert_eq!(call.args, Value::Null);
        assert_eq!(
            Request::from_call(&call.call, call.args).unwrap(),
            Request::ProjectsList {}
        );
    }

    #[test]
    fn turns_down_what_isnt_a_frame() {
        for text in [
            "",
            "[1]",
            "{}",
            r#"{"id":"7","call":"x"}"#,
            r#"{"id":-1,"call":"x"}"#,
            r#"{"id":7}"#,
            r#"{"call":"board:load"}"#,
            r#"{"ev":"no-such-event","p":{}}"#,
        ] {
            assert!(Frame::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn reads_a_lone_surrogate_as_javascript_does() {
        let Frame::Call(call) = Frame::parse(r#"{"id":1,"call":"session:write","args":{"data":"\ud83d"}}"#).unwrap()
        else {
            panic!("a call");
        };
        assert_eq!(call.args["data"], "\u{fffd}");
    }
}
