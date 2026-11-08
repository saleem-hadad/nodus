//! Turning a socket's bytes into messages (`messageReader` in hostProtocol.ts).

use serde_json::Value;

/// Decodes UTF-8 across reads as Node's `StringDecoder` does: a character
/// whose bytes are split between two reads arrives whole, and bytes that
/// can't be UTF-8 become U+FFFD, one per maximal invalid sequence, as the
/// WHATWG decoder (and V8) replace them. (HOST-017, HOST-055)
#[derive(Debug, Default)]
pub struct Utf8Decoder {
    /// The start of a character whose other bytes haven't come yet.
    pending: Vec<u8>,
}

impl Utf8Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The text of the next chunk, holding back an incomplete last character.
    pub fn decode(&mut self, chunk: &[u8]) -> String {
        let joined;
        let mut bytes = if self.pending.is_empty() {
            chunk
        } else {
            let mut all = std::mem::take(&mut self.pending);
            all.extend_from_slice(chunk);
            joined = all;
            &joined[..]
        };
        let mut out = String::with_capacity(bytes.len());
        loop {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    out.push_str(text);
                    return out;
                }
                Err(err) => {
                    let (valid, rest) = bytes.split_at(err.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).expect("from_utf8 checked these bytes"));
                    match err.error_len() {
                        Some(len) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            bytes = &rest[len..];
                        }
                        // The bytes so far are the start of a character.
                        None => {
                            self.pending = rest.to_vec();
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// Whatever is held back, once no more bytes are coming: an incomplete
    /// character is one U+FFFD (`decoder.end()`).
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            String::new()
        } else {
            self.pending.clear();
            char::REPLACEMENT_CHARACTER.to_string()
        }
    }
}

/// Splits a socket's bytes into messages: one JSON value per line. Blank
/// lines and lines that aren't JSON are dropped, and reading carries on.
/// There's no limit on a line's length. (HOST-014–021)
#[derive(Debug, Default)]
pub struct MessageReader {
    decoder: Utf8Decoder,
    /// The start of a line whose newline hasn't come yet.
    partial: String,
}

impl MessageReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every complete message in what has been read so far, in order.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Value> {
        let text = self.decoder.decode(chunk);
        let mut messages = Vec::new();
        // Only the new text is scanned, so a long message arriving in pieces stays linear.
        let mut rest = text.as_str();
        while let Some(nl) = rest.find('\n') {
            let line = if self.partial.is_empty() {
                &rest[..nl]
            } else {
                self.partial.push_str(&rest[..nl]);
                self.partial.as_str()
            };
            if !line.is_empty() {
                // JSON.parse: a lone surrogate escape reads as U+FFFD rather than losing the line.
                if let Ok(msg) = nodus_domain::json::from_str::<Value>(line) {
                    messages.push(msg);
                }
            }
            self.partial.clear();
            rest = &rest[nl + 1..];
        }
        self.partial.push_str(rest);
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, encode};
    use serde_json::json;

    fn read(chunks: &[&[u8]]) -> Vec<Value> {
        let mut reader = MessageReader::new();
        chunks.iter().flat_map(|chunk| reader.feed(chunk)).collect()
    }

    // hostProtocol.test.ts › messageReader

    #[test]
    fn reads_several_messages_from_one_chunk() {
        let bytes = encode(&ClientMessage::Kill { session_id: "a".into() }) + &encode(&ClientMessage::Shutdown);
        assert_eq!(
            read(&[bytes.as_bytes()]),
            vec![json!({"type":"kill","sessionId":"a"}), json!({"type":"shutdown"})]
        );
    }

    #[test]
    fn waits_for_the_rest_of_a_message_split_across_chunks() {
        let msg = encode(&ClientMessage::Write {
            session_id: "a".into(),
            data: "hello world".into(),
        });
        let bytes = msg.as_bytes();
        let mut reader = MessageReader::new();
        assert!(reader.feed(&bytes[..10]).is_empty());
        assert_eq!(
            reader.feed(&bytes[10..]),
            vec![json!({"type":"write","sessionId":"a","data":"hello world"})]
        );
    }

    #[test]
    fn keeps_a_character_whose_bytes_are_split_across_chunks() {
        let msg = encode(&ClientMessage::Write {
            session_id: "a".into(),
            data: "héllo 🎉".into(),
        });
        let bytes = msg.as_bytes();
        // Every split point, including inside the two- and four-byte characters.
        for i in 1..bytes.len() {
            assert_eq!(
                read(&[&bytes[..i], &bytes[i..]]),
                vec![json!({"type":"write","sessionId":"a","data":"héllo 🎉"})],
                "split at {i}"
            );
        }
    }

    #[test]
    fn drops_malformed_and_blank_lines_and_carries_on() {
        let bytes = format!("not json\n\n{}", encode(&ClientMessage::Shutdown));
        assert_eq!(read(&[bytes.as_bytes()]), vec![json!({"type":"shutdown"})]);
    }

    #[test]
    fn keeps_newlines_inside_a_message() {
        let msg = ClientMessage::Write {
            session_id: "a".into(),
            data: "line one\nline two\r\n".into(),
        };
        assert_eq!(
            read(&[encode(&msg).as_bytes()]),
            vec![json!({"type":"write","sessionId":"a","data":"line one\nline two\r\n"})]
        );
    }

    // More of what the reader promises (HOST-017, HOST-020, HOST-021).

    #[test]
    fn hands_on_any_json_value() {
        assert_eq!(
            read(&[b"5\nnull\n[1,2]\n\"x\"\n{}\n"]),
            vec![json!(5), Value::Null, json!([1, 2]), json!("x"), json!({})]
        );
    }

    #[test]
    fn reads_a_lone_surrogate_without_losing_the_line() {
        assert_eq!(
            read(&[b"{\"data\":\"cut \\ud83d\"}\n"]),
            vec![json!({"data":"cut \u{fffd}"})]
        );
    }

    #[test]
    fn has_no_line_limit() {
        let data = "x".repeat(5 * 1024 * 1024);
        let line = encode(&ClientMessage::Write {
            session_id: "a".into(),
            data: data.clone(),
        });
        let mut reader = MessageReader::new();
        let mut got = Vec::new();
        for chunk in line.as_bytes().chunks(65536) {
            got.extend(reader.feed(chunk));
        }
        assert_eq!(got, vec![json!({"type":"write","sessionId":"a","data":data})]);
    }

    #[test]
    fn a_line_ending_in_cr_still_reads() {
        assert_eq!(
            read(&[b"{\"type\":\"shutdown\"}\r\n"]),
            vec![json!({"type":"shutdown"})]
        );
    }

    #[test]
    fn decodes_invalid_bytes_as_replacement_characters() {
        let mut decoder = Utf8Decoder::new();
        assert_eq!(decoder.decode(b"a\xffb"), "a\u{fffd}b");
        // An incomplete four-byte character cut short by an ASCII byte: one U+FFFD.
        assert_eq!(decoder.decode(b"\xf0\x9f\x8eA"), "\u{fffd}A");
        // An overlong and a surrogate, one per byte as WHATWG does.
        assert_eq!(decoder.decode(b"\xc0\x80"), "\u{fffd}\u{fffd}");
        assert_eq!(decoder.decode(b"\xed\xa0\x80"), "\u{fffd}\u{fffd}\u{fffd}");
        // Held back across reads, then completed.
        assert_eq!(decoder.decode(b"\xf0\x9f"), "");
        assert_eq!(decoder.decode(b"\x8e\x89!"), "🎉!");
        // Held back, then given up on at the end.
        assert_eq!(decoder.decode(b"\xe2\x82"), "");
        assert_eq!(decoder.finish(), "\u{fffd}");
        assert_eq!(decoder.finish(), "");
    }
}
