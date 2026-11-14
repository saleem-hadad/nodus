//! Messages between sessions (BOARD-184): what a session sent other Claude
//! Code sessions with SendMessage and what it took in from them, read from
//! its transcript, and who is at the other end.
//!
//! Claude Code gives each message a `msg_id`. The sender's transcript has it
//! in SendMessage's result, and the receiver's has it in the `origin` of the
//! line the message came in on (a `user` line, or a `queued_command`
//! attachment while it was busy). When both transcripts are on the board the
//! two meet by that id. When only one is, its other end is looked up in
//! Claude Code's registry of running sessions
//! (`~/.claude/sessions/<pid>.json`), which has each one's session id,
//! messaging socket and name.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use nodus_domain::BoardTalk;
use serde_json::Value;

use crate::lines::Json;
use crate::parse::time_of;
use crate::value::{as_str, field};

/// Claude Code's tool for messaging another session.
const SEND_TOOL: &str = "SendMessage";

/// A message this session sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub msg_id: String,
    /// SendMessage's `to` without its ` [ref]`: a name, or `uds:<socket>`.
    pub to: String,
    pub at: f64,
}

/// A message this session took in from another.
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub msg_id: String,
    /// The sender's address, `uds:<socket>`.
    pub from: String,
    /// The sender's pid, as Claude Code checked it.
    pub pid: Option<u64>,
    pub at: f64,
}

/// What a transcript says about messages to and from other sessions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Peers {
    pub sent: Vec<Sent>,
    pub received: Vec<Received>,
    /// SendMessage calls waiting for their result: tool use id → `to`.
    pending: HashMap<String, String>,
}

impl Peers {
    /// Takes in one transcript line.
    pub fn apply(&mut self, line: &Json) {
        match line.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for b in blocks(line) {
                    if as_str(field(Some(b), "type")) == Some("tool_use")
                        && as_str(field(Some(b), "name")) == Some(SEND_TOOL)
                        && let Some(id) = as_str(field(Some(b), "id"))
                        && let Some(to) = as_str(field(field(Some(b), "input"), "to"))
                    {
                        self.pending.insert(id.to_owned(), bare(to).to_owned());
                    }
                }
            }
            Some("user") => {
                self.receive(line, line.get("origin"));
                if self.pending.is_empty() {
                    return;
                }
                let results: Vec<&Value> = blocks(line)
                    .iter()
                    .filter(|b| as_str(field(Some(b), "type")) == Some("tool_result"))
                    .collect();
                for b in &results {
                    let Some(to) = as_str(field(Some(b), "tool_use_id")).and_then(|id| self.pending.remove(id)) else {
                        continue;
                    };
                    // `toolUseResult` is the line's one result; else what the model read.
                    let read = if results.len() == 1 {
                        line.get("toolUseResult").cloned()
                    } else {
                        None
                    };
                    let read = read.or_else(|| result_json(b));
                    let sent = read
                        .as_ref()
                        .filter(|r| field(Some(r), "success") == Some(&Value::Bool(true)));
                    if let Some(msg_id) = as_str(field(sent, "msg_id"))
                        && let Some(at) = time_of(line)
                    {
                        self.sent.push(Sent {
                            msg_id: msg_id.to_owned(),
                            to,
                            at,
                        });
                    }
                }
            }
            Some("attachment") => self.receive(line, field(line.get("attachment"), "origin")),
            _ => {}
        }
    }

    /// A message from another session, if `origin` says the line is one.
    fn receive(&mut self, line: &Json, origin: Option<&Value>) {
        if as_str(field(origin, "kind")) != Some("peer") {
            return;
        }
        let Some(from) = as_str(field(origin, "from")) else {
            return;
        };
        let Some(msg_id) = as_str(field(origin, "msg_id")).or_else(|| as_str(line.get("uuid"))) else {
            return;
        };
        let Some(at) = time_of(line) else {
            return;
        };
        // Queued while the session was busy, then taken in: one message.
        if self.received.iter().rev().any(|r| r.msg_id == msg_id) {
            return;
        }
        self.received.push(Received {
            msg_id: msg_id.to_owned(),
            from: from.to_owned(),
            pid: field(origin, "verifiedPeerPid").and_then(Value::as_u64),
            at,
        });
    }
}

/// The line's message content blocks.
fn blocks(line: &Json) -> &[Value] {
    match field(line.get("message"), "content") {
        Some(Value::Array(blocks)) => blocks,
        _ => &[],
    }
}

/// A tool result's text, read as the JSON SendMessage answers with.
fn result_json(block: &Value) -> Option<Value> {
    let text = match field(Some(block), "content")? {
        Value::String(s) => s.as_str(),
        Value::Array(parts) => parts.iter().find_map(|p| as_str(field(Some(p), "text")))?,
        _ => return None,
    };
    serde_json::from_str(text).ok()
}

/// An address without the ` [ref]` that picks one of two sessions of a name.
fn bare(to: &str) -> &str {
    let to = to.trim();
    match to.rfind(" [") {
        Some(i) if to.ends_with(']') => to[..i].trim_end(),
        _ => to,
    }
}

/// `uds:<socket>`'s socket, its `%XX` escapes undone.
fn socket(address: &str) -> Option<String> {
    let raw = address.strip_prefix("uds:")?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = raw
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (raw[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// One running session in Claude Code's registry.
#[derive(Debug, Clone, PartialEq)]
struct Running {
    session_id: String,
    /// When its process started, and when it took its name (ms): a message
    /// from before then was someone else's.
    started_at: f64,
    name_since: f64,
}

/// Claude Code's running sessions on this machine, by pid, socket and name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    by_pid: HashMap<u64, Running>,
    by_socket: HashMap<String, Running>,
    by_name: HashMap<String, Running>,
}

impl Registry {
    /// `<home>/.claude/sessions/*.json`; empty when there's none.
    pub fn read(home: &Path) -> Self {
        let mut registry = Self::default();
        let Ok(entries) = fs::read_dir(home.join(".claude").join("sessions")) else {
            return registry;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(text) = fs::read_to_string(&path)
                && let Ok(value) = serde_json::from_str::<Value>(&text)
            {
                registry.add(&value);
            }
        }
        registry
    }

    /// Adds one registry file's entry.
    pub fn add(&mut self, entry: &Value) {
        let Some(session_id) = as_str(field(Some(entry), "sessionId")) else {
            return;
        };
        let started_at = field(Some(entry), "startedAt").and_then(Value::as_f64).unwrap_or(0.0);
        let running = Running {
            session_id: session_id.to_owned(),
            started_at,
            name_since: field(Some(entry), "nameSince")
                .and_then(Value::as_f64)
                .unwrap_or(started_at),
        };
        if let Some(pid) = field(Some(entry), "pid").and_then(Value::as_u64) {
            self.by_pid.insert(pid, running.clone());
        }
        if let Some(sock) = as_str(field(Some(entry), "messagingSocketPath")) {
            self.by_socket.insert(sock.to_owned(), running.clone());
        }
        if let Some(name) = as_str(field(Some(entry), "name")) {
            self.by_name.insert(name.to_owned(), running);
        }
    }

    /// Who sent a message this session took in.
    fn sender(&self, r: &Received) -> Option<&str> {
        let by_socket = socket(&r.from).and_then(|s| self.by_socket.get(&s));
        let running = by_socket.or_else(|| r.pid.and_then(|p| self.by_pid.get(&p)))?;
        (r.at >= running.started_at).then_some(running.session_id.as_str())
    }

    /// Who a message this session sent went to.
    fn recipient(&self, s: &Sent) -> Option<&str> {
        match socket(&s.to) {
            Some(sock) => self
                .by_socket
                .get(&sock)
                .filter(|r| s.at >= r.started_at)
                .map(|r| r.session_id.as_str()),
            None => self
                .by_name
                .get(&s.to)
                .filter(|r| s.at >= r.name_since.max(r.started_at))
                .map(|r| r.session_id.as_str()),
        }
    }
}

/// The messages between `sessions` (each id with what its transcript says),
/// oldest first. A message whose other end isn't one of them is left out,
/// as is one a session sent itself. `registry` is read only when some
/// message's other end isn't on the board's transcripts.
pub fn talks(sessions: &[(&str, &Peers)], registry: impl FnOnce() -> Registry) -> Vec<BoardTalk> {
    let ids: HashSet<&str> = sessions.iter().map(|(id, _)| *id).collect();
    let mut sent: HashMap<&str, (&str, &Sent)> = HashMap::new();
    for &(id, peers) in sessions {
        for s in &peers.sent {
            sent.insert(s.msg_id.as_str(), (id, s));
        }
    }
    let mut out: HashMap<String, BoardTalk> = HashMap::new();
    let mut talk = |id: &str, from: &str, to: &str, at: f64| {
        if from != to && ids.contains(from) && ids.contains(to) {
            out.entry(id.to_owned()).or_insert_with(|| BoardTalk {
                id: id.to_owned(),
                from: from.to_owned(),
                to: to.to_owned(),
                at,
            });
        }
    };
    let mut lone_receipts: Vec<(&str, &Received)> = Vec::new();
    let mut joined: HashSet<&str> = HashSet::new();
    for &(id, peers) in sessions {
        for r in &peers.received {
            match sent.get(r.msg_id.as_str()) {
                Some((from, s)) => {
                    talk(&r.msg_id, from, id, s.at);
                    joined.insert(r.msg_id.as_str());
                }
                None => lone_receipts.push((id, r)),
            }
        }
    }
    let lone_sends: Vec<(&str, &Sent)> = sent
        .values()
        .filter(|(_, s)| !joined.contains(s.msg_id.as_str()))
        .copied()
        .collect();
    if !lone_receipts.is_empty() || !lone_sends.is_empty() {
        let registry = registry();
        for (to, r) in lone_receipts {
            if let Some(from) = registry.sender(r) {
                talk(&r.msg_id, from, to, r.at);
            }
        }
        for (from, s) in lone_sends {
            if let Some(to) = registry.recipient(s) {
                talk(&s.msg_id, from, to, s.at);
            }
        }
    }
    let mut out: Vec<BoardTalk> = out.into_values().collect();
    out.sort_by(|a, b| a.at.total_cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(peers: &mut Peers, lines: &[Value]) {
        for l in lines {
            peers.apply(l.as_object().unwrap());
        }
    }

    fn send(tool_id: &str, to: &str, t: &str) -> Value {
        json!({ "type": "assistant", "timestamp": t, "message": { "content": [
            { "type": "tool_use", "id": tool_id, "name": "SendMessage", "input": { "to": to, "message": "hi" } }
        ] } })
    }

    fn result(tool_id: &str, msg_id: Option<&str>, t: &str) -> Value {
        let body = match msg_id {
            Some(id) => json!({ "success": true, "message": "sent", "msg_id": id }),
            None => json!({ "success": false, "message": "Failed to send" }),
        };
        json!({ "type": "user", "timestamp": t, "toolUseResult": body, "message": { "content": [
            { "type": "tool_result", "tool_use_id": tool_id, "content": [{ "type": "text", "text": body.to_string() }] }
        ] } })
    }

    fn peer_line(msg_id: &str, pid: u64, t: &str) -> Value {
        json!({ "type": "user", "isMeta": true, "timestamp": t, "uuid": "u1",
            "origin": { "kind": "peer", "from": format!("uds:/tmp/cc-socks/{pid}.sock"), "verifiedPeerPid": pid,
                "msg_id": msg_id, "name": "nodus-ae" },
            "message": { "content": "Another Claude session sent a message:\n<cross-session-message>" } })
    }

    fn queued(msg_id: &str, pid: u64, t: &str) -> Value {
        json!({ "type": "attachment", "timestamp": t, "attachment": { "type": "queued_command", "prompt": "…",
            "origin": { "kind": "peer", "from": format!("uds:/tmp/cc-socks/{pid}.sock"), "verifiedPeerPid": pid,
                "msg_id": msg_id } } })
    }

    #[test]
    fn reads_what_was_sent_and_taken_in() {
        let mut p = Peers::default();
        lines(
            &mut p,
            &[
                send("t1", "nodus-ae [7cb0b9]", "2026-11-14T07:39:44.000Z"),
                result("t1", Some("m1"), "2026-11-14T07:39:44.800Z"),
                send("t2", "uds:/tmp/cc-socks/9.sock", "2026-11-14T07:40:00.000Z"),
                result("t2", None, "2026-11-14T07:40:00.100Z"),
                queued("m2", 35016, "2026-11-14T07:41:00.000Z"),
                peer_line("m2", 35016, "2026-11-14T07:41:05.000Z"),
                json!({ "type": "user", "timestamp": "2026-11-14T07:42:00.000Z", "message": { "content": "a prompt" } }),
            ],
        );
        assert_eq!(p.sent.len(), 1, "a failed send isn't a message");
        assert_eq!((p.sent[0].msg_id.as_str(), p.sent[0].to.as_str()), ("m1", "nodus-ae"));
        assert_eq!(p.received.len(), 1, "queued, then taken in: one message");
        assert_eq!(p.received[0].pid, Some(35016));
        assert_eq!(p.received[0].from, "uds:/tmp/cc-socks/35016.sock");
        assert!(p.pending.is_empty());
    }

    fn registry() -> Registry {
        let mut r = Registry::default();
        r.add(
            &json!({ "pid": 35016, "sessionId": "b", "messagingSocketPath": "/tmp/cc-socks/35016.sock",
            "name": "nodus-ae", "startedAt": 1000.0, "nameSince": 1000.0 }),
        );
        r.add(
            &json!({ "pid": 44532, "sessionId": "a", "messagingSocketPath": "/tmp/cc-socks/44532.sock",
            "name": "nodus-96", "startedAt": 1000.0 }),
        );
        r
    }

    fn sent(msg_id: &str, to: &str, at: f64) -> Sent {
        Sent {
            msg_id: msg_id.into(),
            to: to.into(),
            at,
        }
    }

    fn received(msg_id: &str, pid: u64, at: f64) -> Received {
        Received {
            msg_id: msg_id.into(),
            from: format!("uds:/tmp/cc-socks/{pid}.sock"),
            pid: Some(pid),
            at,
        }
    }

    #[test]
    fn joins_both_ends_by_msg_id_without_the_registry() {
        let a = Peers {
            sent: vec![sent("m1", "nodus-ae", 2000.0)],
            ..Peers::default()
        };
        let b = Peers {
            received: vec![received("m1", 1, 2500.0)],
            ..Peers::default()
        };
        let out = talks(&[("a", &a), ("b", &b)], || panic!("not needed"));
        assert_eq!(
            out,
            vec![BoardTalk {
                id: "m1".into(),
                from: "a".into(),
                to: "b".into(),
                at: 2000.0
            }]
        );
    }

    #[test]
    fn finds_the_other_end_of_a_lone_message_among_running_sessions() {
        let a = Peers {
            sent: vec![
                sent("m1", "nodus-ae", 2000.0),
                sent("m2", "uds:/tmp/cc-socks/35016.sock", 3000.0),
                // Before this nodus-ae started: another session of that name.
                sent("old", "nodus-ae", 500.0),
                // A subagent, or a session that isn't on the board.
                sent("m3", "a0f8a282d3de517b4", 3500.0),
                sent("m4", "nodus-96", 3600.0),
            ],
            received: vec![received("m5", 35016, 4000.0), received("m6", 777, 4100.0)],
            ..Peers::default()
        };
        let b = Peers::default();
        let out = talks(&[("a", &a), ("b", &b)], registry);
        let got: Vec<(&str, &str, &str)> = out
            .iter()
            .map(|t| (t.id.as_str(), t.from.as_str(), t.to.as_str()))
            .collect();
        assert_eq!(got, vec![("m1", "a", "b"), ("m2", "a", "b"), ("m5", "b", "a")]);
    }

    #[test]
    fn undoes_an_address() {
        assert_eq!(bare("nodus-ae [7cb0b9]"), "nodus-ae");
        assert_eq!(bare("uds:/tmp/x.sock"), "uds:/tmp/x.sock");
        assert_eq!(socket("uds:/tmp/a%20b.sock").as_deref(), Some("/tmp/a b.sock"));
        assert_eq!(socket("nodus-ae"), None);
    }
}
