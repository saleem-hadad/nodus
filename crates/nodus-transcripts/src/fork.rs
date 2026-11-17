//! A session's conversation up to one of its messages, copied under a new
//! session id for a card forked from it (SESS-222). It's what Claude Code's
//! own `/branch` writes, at any point rather than only the latest: the
//! messages that lead to that one, each marked `forkedFrom`, so `claude
//! --resume <new id>` carries on from there and the original is never
//! touched.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::nodeerr::fs_message;

/// Why a fork couldn't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForkError {
    /// The session has no transcript yet.
    #[error("That session has no conversation to fork yet")]
    NoTranscript,
    /// The message isn't in the transcript (any more).
    #[error("That message isn’t in the session’s conversation any more")]
    NoMessage,
    #[error("Couldn’t write the fork: {0}")]
    Write(String),
}

/// Writes `to`: the lines of the transcript at `from` up to the one whose
/// `uuid` is `at`, keeping only the messages `at` follows from (its
/// `parentUuid` chain, across a compaction by `logicalParentUuid`), so a
/// rewound branch or a subagent's line isn't carried over. Lines without a
/// `uuid` (the session's mode, title, costs) come along as they are. Every
/// `sessionId` of `from_session` becomes `new_session`, and each message
/// records where it came from in `forkedFrom`, as `/branch` does. The file
/// appears whole: it's written beside `to` first, then moved there.
pub fn fork_transcript(
    from: &Path,
    from_session: &str,
    at: &str,
    to: &Path,
    new_session: &str,
) -> Result<(), ForkError> {
    let text = match fs::read_to_string(from) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(ForkError::NoTranscript),
        Err(err) => return Err(ForkError::Write(fs_message(&err, "open", from))),
    };
    // A line still being written isn't whole yet, and is left out.
    let lines: Vec<Map<String, Value>> = text
        .lines()
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        })
        .collect();
    let Some(cut) = lines.iter().position(|l| uuid(l) == Some(at)) else {
        return Err(ForkError::NoMessage);
    };
    let parents: HashMap<&str, Option<&str>> = lines[..=cut]
        .iter()
        .filter_map(|l| {
            let parent = str_field(l, "parentUuid").or_else(|| str_field(l, "logicalParentUuid"));
            uuid(l).map(|u| (u, parent))
        })
        .collect();
    let mut chain = HashSet::new();
    let mut next = Some(at);
    while let Some(id) = next.filter(|id| chain.insert(*id)) {
        next = parents.get(id).copied().flatten();
    }

    let mut out = String::new();
    for line in &lines[..=cut] {
        let mut line = line.clone();
        if let Some(id) = uuid(&line).map(str::to_owned) {
            if !chain.contains(id.as_str()) {
                continue;
            }
            line.insert(
                "forkedFrom".into(),
                json!({ "sessionId": from_session, "messageUuid": id }),
            );
        }
        for key in ["sessionId", "session_id"] {
            if str_field(&line, key) == Some(from_session) {
                line.insert(key.into(), Value::from(new_session));
            }
        }
        out.push_str(&Value::Object(line).to_string());
        out.push('\n');
    }

    let write = |path: &Path, err: io::Error, syscall: &str| ForkError::Write(fs_message(&err, syscall, path));
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|err| write(dir, err, "mkdir"))?;
    }
    let part = to.with_extension("jsonl.part");
    fs::write(&part, out).map_err(|err| write(&part, err, "open"))?;
    fs::rename(&part, to).map_err(|err| {
        let _ = fs::remove_file(&part);
        write(to, err, "rename")
    })
}

fn uuid(line: &Map<String, Value>) -> Option<&str> {
    str_field(line, "uuid")
}

fn str_field<'a>(line: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    line.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: &str = "00000000-0000-4000-8000-00000000000a";
    const NEW: &str = "00000000-0000-4000-8000-00000000000f";

    fn msg(kind: &str, uuid: &str, parent: Option<&str>) -> Value {
        json!({ "type": kind, "uuid": uuid, "parentUuid": parent, "sessionId": FROM, "message": { "content": uuid } })
    }

    fn write(dir: &Path, lines: &[Value]) -> std::path::PathBuf {
        let file = dir.join(format!("{FROM}.jsonl"));
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(&file, text).unwrap();
        file
    }

    fn read(file: &Path) -> Vec<Value> {
        fs::read_to_string(file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn uuids(lines: &[Value]) -> Vec<&str> {
        lines.iter().filter_map(|l| l["uuid"].as_str()).collect()
    }

    #[test]
    fn copies_the_chain_up_to_the_message_under_the_new_id() {
        let dir = tempfile::tempdir().unwrap();
        let from = write(
            dir.path(),
            &[
                json!({ "type": "permission-mode", "permissionMode": "acceptEdits", "sessionId": FROM }),
                msg("user", "u1", None),
                msg("assistant", "a1", Some("u1")),
                // A rewound reply the conversation went past.
                msg("assistant", "gone", Some("u1")),
                msg("assistant", "a2", Some("a1")),
                json!({ "type": "ai-title", "aiTitle": "Fix it", "sessionId": FROM }),
                msg("user", "u2", Some("a2")),
                msg("assistant", "a3", Some("u2")),
            ],
        );
        let to = dir.path().join(format!("{NEW}.jsonl"));
        fork_transcript(&from, FROM, "a2", &to, NEW).unwrap();
        let lines = read(&to);
        assert_eq!(uuids(&lines), ["u1", "a1", "a2"]);
        assert_eq!(lines.len(), 4, "the mode comes along, the later title doesn't");
        for line in &lines {
            assert_eq!(line["sessionId"], NEW);
        }
        assert_eq!(
            lines[3]["forkedFrom"],
            json!({ "sessionId": FROM, "messageUuid": "a2" })
        );
        assert!(lines[0].get("forkedFrom").is_none());
        // The original is untouched, and nothing's left beside the fork.
        assert_eq!(read(&from).len(), 8);
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [format!("{FROM}.jsonl"), format!("{NEW}.jsonl")]);
    }

    #[test]
    fn follows_a_compaction_back_to_what_came_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut boundary = msg("system", "c1", None);
        boundary["logicalParentUuid"] = json!("a1");
        let from = write(
            dir.path(),
            &[
                msg("user", "u1", None),
                msg("assistant", "a1", Some("u1")),
                boundary,
                msg("user", "u2", Some("c1")),
                msg("assistant", "a2", Some("u2")),
            ],
        );
        let to = dir.path().join(format!("{NEW}.jsonl"));
        fork_transcript(&from, FROM, "a2", &to, NEW).unwrap();
        assert_eq!(uuids(&read(&to)), ["u1", "a1", "c1", "u2", "a2"]);
    }

    #[test]
    fn leaves_out_a_subagents_lines_and_a_line_still_being_written() {
        let dir = tempfile::tempdir().unwrap();
        let from = write(
            dir.path(),
            &[
                msg("user", "u1", None),
                json!({ "type": "assistant", "uuid": "s1", "parentUuid": null, "isSidechain": true, "sessionId": FROM }),
                msg("assistant", "a1", Some("u1")),
            ],
        );
        let mut text = fs::read_to_string(&from).unwrap();
        text.push_str(r#"{"type":"assistant","uuid":"a"#);
        fs::write(&from, text).unwrap();
        let to = dir.path().join(format!("{NEW}.jsonl"));
        fork_transcript(&from, FROM, "a1", &to, NEW).unwrap();
        assert_eq!(uuids(&read(&to)), ["u1", "a1"]);
    }

    #[test]
    fn says_why_it_cant() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join(format!("{NEW}.jsonl"));
        let missing = dir.path().join("missing.jsonl");
        assert_eq!(
            fork_transcript(&missing, FROM, "a1", &to, NEW),
            Err(ForkError::NoTranscript)
        );
        let from = write(dir.path(), &[msg("user", "u1", None)]);
        let err = fork_transcript(&from, FROM, "nope", &to, NEW).unwrap_err();
        assert_eq!(err, ForkError::NoMessage);
        assert_eq!(
            err.to_string(),
            "That message isn’t in the session’s conversation any more"
        );
        assert!(!to.exists());
    }
}
