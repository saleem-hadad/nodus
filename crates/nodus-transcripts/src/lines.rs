//! Reading complete JSON lines from part of a transcript (`readLines`).
//!
//! Transcripts are append-only JSONL that Claude Code may be writing to as
//! they're read, so only lines that end in a newline are taken; the rest is
//! left for the next read.

use std::fs::File;
use std::io;

use serde_json::{Map, Value};

/// A transcript line: one JSON object.
pub type Json = Map<String, Value>;

/// Parses the complete JSON lines in bytes `[start, end)` of `file` and passes
/// each object to `on_line`. Returns the offset just past the last newline
/// read, where the next read should start (SESS-010…013).
///
/// With `skip_first_partial`, everything up to the first newline is skipped,
/// for a read that starts mid-file. `keep` drops raw lines before they're
/// parsed, for a reader that wants only some.
pub fn read_lines(
    file: &File,
    start: u64,
    end: u64,
    skip_first_partial: bool,
    mut on_line: impl FnMut(&Json),
    keep: Option<&dyn Fn(&str) -> bool>,
) -> u64 {
    // SESS-010: nothing to read.
    if end <= start {
        return start;
    }
    let buf = read_range(file, start, end);
    let mut from = 0;
    if skip_first_partial {
        // SESS-011: no newline in what was read, so no complete line.
        let Some(nl) = buf.iter().position(|&b| b == b'\n') else {
            return start;
        };
        from = nl + 1;
    }
    // SESS-012: up to the last newline only.
    let last_nl = match buf.iter().rposition(|&b| b == b'\n') {
        Some(nl) if nl >= from => nl,
        _ => return start + from as u64,
    };
    // SESS-013: UTF-8, with U+FFFD for anything that isn't.
    let text = String::from_utf8_lossy(&buf[from..last_nl]);
    for raw in text.split('\n') {
        if raw.is_empty() || keep.is_some_and(|keep| !keep(raw)) {
            continue;
        }
        // Malformed or partly written lines, and lines that aren't objects, add nothing.
        if let Ok(Value::Object(line)) = nodus_domain::json::from_str::<Value>(raw) {
            on_line(&line);
        }
    }
    start + last_nl as u64 + 1
}

/// Reads bytes `[start, end)` with positional reads until they're all read or
/// the file ends.
fn read_range(file: &File, start: u64, end: u64) -> Vec<u8> {
    let length = usize::try_from(end - start).unwrap_or(usize::MAX);
    let mut buf = vec![0; length];
    let mut read = 0;
    while read < length {
        match read_at(file, &mut buf[read..], start + read as u64) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            // Node would throw here; what was read so far is used.
            Err(_) => break,
        }
    }
    buf.truncate(read);
    buf
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn file_with(text: &str) -> (tempfile::NamedTempFile, File) {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(text.as_bytes()).unwrap();
        let file = File::open(tmp.path()).unwrap();
        (tmp, file)
    }

    fn collect(file: &File, start: u64, end: u64, skip: bool) -> (Vec<Value>, u64) {
        let mut lines = Vec::new();
        let offset = read_lines(file, start, end, skip, |l| lines.push(Value::Object(l.clone())), None);
        (lines, offset)
    }

    #[test]
    fn reads_complete_lines_and_leaves_a_partial_one() {
        let text = "{\"a\":1}\n{\"b\":2}\n{\"c\":";
        let (_tmp, file) = file_with(text);
        let (lines, offset) = collect(&file, 0, text.len() as u64, false);
        assert_eq!(lines, [serde_json::json!({"a": 1}), serde_json::json!({"b": 2})]);
        assert_eq!(offset, 16);
    }

    #[test]
    fn skips_a_partial_first_line() {
        let text = "x\":1}\n{\"b\":2}\n";
        let (_tmp, file) = file_with(text);
        let (lines, offset) = collect(&file, 0, text.len() as u64, true);
        assert_eq!(lines, [serde_json::json!({"b": 2})]);
        assert_eq!(offset, text.len() as u64);
        // No newline at all: nothing, and the start back.
        let (_tmp, file) = file_with("no newline here");
        assert_eq!(collect(&file, 3, 15, true), (vec![], 3));
    }

    #[test]
    fn returns_past_the_skipped_part_when_no_line_follows() {
        let (_tmp, file) = file_with("partial\nrest");
        assert_eq!(collect(&file, 0, 12, true), (vec![], 8));
        assert_eq!(collect(&file, 0, 7, false), (vec![], 0));
    }

    #[test]
    fn ignores_malformed_lines_and_non_objects() {
        let text = "{bad\nnull\n[1]\n\"s\"\n\n{\"ok\":true}\n";
        let (_tmp, file) = file_with(text);
        let (lines, offset) = collect(&file, 0, text.len() as u64, false);
        assert_eq!(lines, [serde_json::json!({"ok": true})]);
        assert_eq!(offset, text.len() as u64);
    }

    #[test]
    fn filters_raw_lines_before_parsing() {
        let text = "{\"type\":\"tool_result\"}\n{\"type\":\"user\"}\n";
        let (_tmp, file) = file_with(text);
        let mut lines = Vec::new();
        let keep = |raw: &str| !raw.contains("tool_result");
        read_lines(
            &file,
            0,
            text.len() as u64,
            false,
            |l| lines.push(l.clone()),
            Some(&keep),
        );
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn nothing_to_read_returns_the_start() {
        let (_tmp, file) = file_with("{}\n");
        assert_eq!(collect(&file, 3, 3, false), (vec![], 3));
        assert_eq!(collect(&file, 5, 3, false), (vec![], 5));
    }

    #[test]
    fn reads_a_lone_surrogate_and_bad_utf8() {
        let mut bytes = b"{\"t\":\"\\ud83d\"}\n{\"u\":\"".to_vec();
        bytes.extend_from_slice(&[0xff, b'"', b'}', b'\n']);
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bytes).unwrap();
        let file = File::open(tmp.path()).unwrap();
        let (lines, _) = collect(&file, 0, bytes.len() as u64, false);
        assert_eq!(lines[0]["t"], "\u{fffd}");
        assert_eq!(lines[1]["u"], "\u{fffd}");
    }
}
