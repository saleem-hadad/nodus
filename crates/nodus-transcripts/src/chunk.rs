//! What a transcript gained since a read of it, as raw JSON lines: what the
//! chat view reads (CHAT-001…CHAT-003), on this computer and, for a session
//! on a server, through `session:chat` (SRV-166). The lines are parsed by
//! [`crate::ChatLog`], wherever they were read.

use std::fs::{self, File};
use std::path::Path;

use nodus_domain::ChatChunk;

use crate::lines::read_range;

/// Only this much of the end of a long transcript is read when the chat
/// opens (CHAT-002).
pub const CHAT_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// At most this much of a transcript goes in one `session:chat` answer
/// (SRV-166). JSON-escaped, even a worst case of it stays well under a
/// frame's 4 MB.
pub const CHAT_CHUNK_BYTES: u64 = 512 * 1024;

/// How much is looked at a time for the first whole line of a long
/// transcript's last 16 MiB.
const SCAN: u64 = 64 * 1024;

/// What the transcript at `path` gained since a read that got to `offset`
/// in `file` (the last chunk's [`ChatChunk::file`]; `""` and 0 at first),
/// at most `limit` bytes of it.
///
/// - CHAT-001: no file yet is no text.
/// - CHAT-002: a first read of a long transcript starts at its last 16 MiB,
///   from the first whole line there; only whole lines are given, so a line
///   still being written waits for its newline. A line longer than `limit`
///   goes in pieces, cut between characters.
/// - CHAT-003: another file under the name, or one that shrank, is read
///   again from the start, and the chunk says `restart`.
pub fn read_chunk(path: &Path, file: &str, offset: u64, limit: u64) -> ChatChunk {
    read_chunk_within(path, file, offset, limit, CHAT_MAX_BYTES)
}

/// [`read_chunk`], opening a long transcript at its last `tail` bytes.
fn read_chunk_within(path: &Path, file: &str, offset: u64, limit: u64, tail: u64) -> ChatChunk {
    let unchanged = ChatChunk {
        file: file.to_owned(),
        offset,
        ..ChatChunk::default()
    };
    let Ok(stat) = fs::metadata(path) else {
        return unchanged;
    };
    let id = file_id(&stat);
    let size = stat.len();
    let replaced = !file.is_empty() && !id.is_empty() && file != id;
    let restart = size < offset || replaced;
    let from = if restart { 0 } else { offset };
    let mut chunk = ChatChunk {
        file: id,
        restart,
        offset: from,
        ..ChatChunk::default()
    };
    if size == from {
        return chunk;
    }
    let Ok(fd) = File::open(path) else {
        return unchanged;
    };
    let start = if from == 0 && size > tail {
        match first_line_from(&fd, size - tail, size) {
            Some(start) => start,
            // No whole line in it yet.
            None => return chunk,
        }
    } else {
        from
    };
    let end = size.min(start.saturating_add(limit));
    let buf = read_range(&fd, start, end);
    let take = match buf.iter().rposition(|&b| b == b'\n') {
        Some(nl) => nl + 1,
        // A line longer than one read: its start now, the rest after.
        None if end < size => match char_boundary(&buf) {
            0 => buf.len(),
            cut => cut,
        },
        // CHAT-002: a line still being written waits for its newline.
        None => 0,
    };
    chunk.text = String::from_utf8_lossy(&buf[..take]).into_owned();
    chunk.offset = start + take as u64;
    chunk.more = take > 0 && end < size;
    chunk
}

/// Where the first whole line at or after `from` starts: just past the
/// first newline from `from - 1`, so a line starting exactly at `from`
/// counts. None when there's no newline before `end`.
fn first_line_from(fd: &File, from: u64, end: u64) -> Option<u64> {
    let mut at = from.saturating_sub(1);
    while at < end {
        let buf = read_range(fd, at, end.min(at + SCAN));
        if buf.is_empty() {
            return None;
        }
        if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            return Some(at + nl as u64 + 1);
        }
        at += buf.len() as u64;
    }
    None
}

/// The longest start of `buf` that doesn't end partway through a UTF-8
/// character.
fn char_boundary(buf: &[u8]) -> usize {
    let len = buf.len();
    // The last character's first byte, among the last four.
    let Some(lead) = (len.saturating_sub(4)..len).rev().find(|&i| buf[i] & 0xC0 != 0x80) else {
        return len;
    };
    let width = match buf[lead] {
        b if b < 0x80 => 1,
        b if b >= 0xF0 => 4,
        b if b >= 0xE0 => 3,
        b if b >= 0xC0 => 2,
        _ => 1,
    };
    if lead + width > len { lead } else { len }
}

/// What names a file apart from another one put in its place: its device
/// and inode. Nothing off Unix, where only a shrinking file reads as
/// replaced.
#[cfg(unix)]
fn file_id(stat: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt as _;
    format!("{}:{}", stat.dev(), stat.ino())
}

#[cfg(not(unix))]
fn file_id(_: &fs::Metadata) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn gives_whole_lines_and_where_to_go_on() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.jsonl");
        // CHAT-001
        assert_eq!(read_chunk(&path, "", 0, 100), ChatChunk::default());
        append(&path, "{\"a\":1}\n{\"b\":");
        let first = read_chunk(&path, "", 0, 100);
        assert_eq!((first.text.as_str(), first.offset), ("{\"a\":1}\n", 8));
        assert!(!first.restart && !first.more);
        #[cfg(unix)]
        assert!(!first.file.is_empty());
        // Nothing more till the line ends.
        let same = read_chunk(&path, &first.file, first.offset, 100);
        assert_eq!((same.text.as_str(), same.offset), ("", 8));
        append(&path, "2}\n");
        let next = read_chunk(&path, &first.file, first.offset, 100);
        assert_eq!((next.text.as_str(), next.offset), ("{\"b\":2}\n", 16));
    }

    #[test]
    fn a_long_read_comes_in_pieces() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.jsonl");
        append(&path, "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n");
        let one = read_chunk(&path, "", 0, 12);
        assert_eq!((one.text.as_str(), one.offset, one.more), ("{\"a\":1}\n", 8, true));
        let two = read_chunk(&path, &one.file, one.offset, 100);
        assert_eq!((two.text.as_str(), two.more), ("{\"b\":2}\n{\"c\":3}\n", false));
        // A line longer than a read, cut between characters.
        let path = dir.path().join("long.jsonl");
        append(&path, "{\"é\":\"ééé\"}\n");
        let mut text = String::new();
        let (mut file, mut offset) = (String::new(), 0);
        loop {
            let chunk = read_chunk(&path, &file, offset, 4);
            assert!(chunk.text.len() <= 4 && !chunk.text.contains('\u{fffd}'), "{chunk:?}");
            text.push_str(&chunk.text);
            (file, offset) = (chunk.file, chunk.offset);
            if !chunk.more {
                break;
            }
        }
        assert_eq!(text, "{\"é\":\"ééé\"}\n");
    }

    #[test]
    fn chat_002_a_long_transcript_opens_at_its_tail() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.jsonl");
        append(&path, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n");
        // The tail starts mid-line: from the next whole one.
        let chunk = read_chunk_within(&path, "", 0, 100, 12);
        assert_eq!(chunk.text, "{\"n\":3}\n");
        // It starts on a line's first byte: that line is whole.
        let chunk = read_chunk_within(&path, "", 0, 100, 16);
        assert_eq!(chunk.text, "{\"n\":2}\n{\"n\":3}\n");
        // Read on from there, the tail doesn't count.
        let on = read_chunk_within(&path, &chunk.file, chunk.offset, 100, 4);
        assert_eq!((on.text.as_str(), on.offset), ("", 24));
    }

    #[test]
    fn chat_003_a_replaced_or_shrunk_transcript_starts_over() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.jsonl");
        append(&path, "{\"a\":1}\n{\"b\":2}\n");
        let first = read_chunk(&path, "", 0, 100);
        fs::write(&path, "{\"c\":3}\n").unwrap();
        let shrunk = read_chunk(&path, &first.file, first.offset, 100);
        assert!(shrunk.restart);
        assert_eq!((shrunk.text.as_str(), shrunk.offset), ("{\"c\":3}\n", 8));
        // Another file in its place, longer than what was read.
        #[cfg(unix)]
        {
            let other = dir.path().join("other.jsonl");
            append(&other, "{\"d\":4}\n{\"e\":5}\n{\"f\":6}\n");
            fs::rename(&other, &path).unwrap();
            let replaced = read_chunk(&path, &shrunk.file, shrunk.offset, 100);
            assert!(replaced.restart && replaced.file != shrunk.file);
            assert_eq!(replaced.offset, 24);
        }
    }

    #[test]
    fn cuts_between_characters() {
        assert_eq!(char_boundary(b"abc"), 3);
        assert_eq!(char_boundary("aé".as_bytes()), 3);
        assert_eq!(char_boundary(&"aé".as_bytes()[..2]), 1);
        assert_eq!(char_boundary(&"a😀".as_bytes()[..4]), 1);
        assert_eq!(char_boundary(&[0x80, 0x80, 0x80, 0x80, 0x80]), 5);
    }
}
