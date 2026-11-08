//! A session's recent output, for replaying into a terminal. The host and
//! the app keep it the same way, with the same limit.

use std::collections::VecDeque;

use nodus_domain::js::utf16_len;

/// The output kept per session, in UTF-16 code units: 2 × 1024 × 1024
/// (`BUFFER_LIMIT` in host.ts and pty.ts).
pub const BUFFER_LIMIT: usize = 2 * 1024 * 1024;

/// Output chunks as they arrived, oldest dropped whole once the total passes
/// [`BUFFER_LIMIT`], but never the newest, however long. (HOST-056, HOST-117)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayBuffer {
    chunks: VecDeque<String>,
    /// The chunks' total length in UTF-16 code units.
    size: usize,
}

impl ReplayBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text` as one chunk, or nothing when it's empty, as
    /// the app adopts a welcome's buffer.
    pub fn from_text(text: String) -> Self {
        let mut buffer = Self::new();
        if !text.is_empty() {
            buffer.size = utf16_len(&text);
            buffer.chunks.push_back(text);
        }
        buffer
    }

    pub fn push(&mut self, chunk: String) {
        self.size += utf16_len(&chunk);
        self.chunks.push_back(chunk);
        while self.size > BUFFER_LIMIT && self.chunks.len() > 1 {
            let dropped = self.chunks.pop_front().expect("more than one chunk");
            self.size -= utf16_len(&dropped);
        }
    }

    /// Everything kept, joined.
    pub fn joined(&self) -> String {
        self.chunks.iter().map(String::as_str).collect()
    }

    /// The total length in UTF-16 code units.
    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_whole_oldest_chunks_past_the_limit_in_utf16_units() {
        let mut buffer = ReplayBuffer::new();
        let half = "a".repeat(BUFFER_LIMIT / 2);
        buffer.push(half.clone());
        buffer.push(half.clone());
        assert_eq!((buffer.len(), buffer.chunk_count()), (BUFFER_LIMIT, 2));
        // One unit over: the oldest whole chunk goes.
        buffer.push("b".into());
        assert_eq!((buffer.len(), buffer.chunk_count()), (BUFFER_LIMIT / 2 + 1, 2));
        assert_eq!(buffer.joined(), format!("{half}b"));
    }

    #[test]
    fn counts_utf16_units_not_bytes() {
        let mut buffer = ReplayBuffer::new();
        // 2 units each, 4 bytes each.
        buffer.push("🎉".repeat(BUFFER_LIMIT / 2));
        buffer.push("x".into());
        assert_eq!(buffer.chunk_count(), 1);
        assert_eq!(buffer.joined(), "x");
        let mut buffer = ReplayBuffer::new();
        buffer.push("é".repeat(BUFFER_LIMIT / 2));
        buffer.push("é".repeat(BUFFER_LIMIT / 2));
        assert_eq!(buffer.chunk_count(), 2);
    }

    #[test]
    fn keeps_the_newest_chunk_however_long() {
        let mut buffer = ReplayBuffer::new();
        buffer.push("a".into());
        buffer.push("z".repeat(BUFFER_LIMIT + 10));
        assert_eq!((buffer.chunk_count(), buffer.len()), (1, BUFFER_LIMIT + 10));
    }

    #[test]
    fn adopts_a_welcome_buffer_as_one_chunk() {
        assert!(ReplayBuffer::from_text(String::new()).is_empty());
        let buffer = ReplayBuffer::from_text("old output".into());
        assert_eq!(
            (buffer.chunk_count(), buffer.len(), buffer.joined()),
            (1, 10, "old output".to_owned())
        );
    }
}
