//! A server's files over plain HTTP (SRV-022, SRV-163): uploads (files
//! dropped on a terminal, pictures added to a board) go up with
//! `PUT /api/uploads`, and come back, with board pictures and views' pages,
//! from `GET /api/files/<id>`. Both carry the token as `Authorization:
//! Bearer`, as the WebSocket does.
//!
//! The server picks every id: a UUID, with the extension of the name it was
//! given. It serves only ids of that form, so a request can never name a
//! path.

use serde::{Deserialize, Serialize};

/// `GET`, followed by the id: a file the server issued, a board picture,
/// or a view's page.
pub const FILES_PATH: &str = "/api/files/";

/// `PUT`, with the file as the body and its name as `?name=<percent-encoded>`
/// (a label only): answers [`Upload`].
pub const UPLOADS_PATH: &str = "/api/uploads";

/// The biggest upload, in bytes: anything bigger is refused (SRV-163).
pub const MAX_UPLOAD: u64 = 50 * 1024 * 1024;

/// What an upload became on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    /// The name the server chose: a UUID, with the extension of `name`.
    pub id: String,
    /// Where the server keeps it: what's pasted into a server terminal.
    pub path: String,
    /// The name it was uploaded with, kept only as a label.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
}

/// The extension an upload's id takes from its name: lower case, letters
/// and digits only, at most five; none otherwise.
pub fn upload_ext(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let ok = !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| ext.to_ascii_lowercase())
}

/// Whether `id` has the form of an id the server issues: a lower-case UUID,
/// optionally followed by `.` and a short extension.
pub fn file_id_ok(id: &str) -> bool {
    let (uuid, ext) = match id.split_once('.') {
        Some((uuid, ext)) => (uuid, Some(ext)),
        None => (id, None),
    };
    let ext_ok = ext.is_none_or(|ext| {
        !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    ext_ok && is_uuid(uuid)
}

/// `8-4-4-4-12` lower-case hex digits.
pub fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, len)| {
            group.len() == len && group.chars().all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
        })
}

/// A name for a query string: everything but unreserved characters as
/// `%XX` of its UTF-8 bytes.
pub fn encode_name(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// `name`'s value in a query string, decoded (`+` is a space); none when
/// it isn't there.
pub fn name_from_query(query: &str) -> Option<String> {
    let value = query.split('&').find_map(|pair| pair.strip_prefix("name="))?;
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        decoded.push(byte);
                        i += 3;
                        continue;
                    }
                    None => decoded.push(b'%'),
                }
            }
            b'+' => decoded.push(b' '),
            byte => decoded.push(byte),
        }
        i += 1;
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_short_plain_extension_from_the_name() {
        assert_eq!(upload_ext("Screen Shot.PNG").as_deref(), Some("png"));
        assert_eq!(upload_ext("notes.tar.gz").as_deref(), Some("gz"));
        for none in ["Makefile", ".env", "a.", "a.toolong", "a.p g", "a.ü"] {
            assert_eq!(upload_ext(none), None, "{none}");
        }
    }

    #[test]
    fn knows_only_ids_of_its_own_form() {
        let id = "0b9c41aa-1d2e-4f3a-8b4c-5d6e7f8a9b0c";
        assert!(file_id_ok(id));
        assert!(file_id_ok(&format!("{id}.png")));
        for wrong in [
            "",
            "../state.json",
            "%2e%2e",
            "state.json",
            &format!("{id}.PNG"),
            &format!("{id}.png.exe"),
            &format!("{id}/x"),
            &format!("{id}."),
            "0B9C41AA-1D2E-4F3A-8B4C-5D6E7F8A9B0C",
        ] {
            assert!(!file_id_ok(wrong), "{wrong}");
        }
    }

    #[test]
    fn carries_a_name_through_a_query_string() {
        for name in [
            "Screen Shot 2026-11-13 at 10.00.png",
            "notes & ideas?.md",
            "صورة.png",
            "100%",
        ] {
            let query = format!("x=1&name={}", encode_name(name));
            assert_eq!(name_from_query(&query).as_deref(), Some(name), "{query}");
        }
        assert_eq!(name_from_query("name=a+b").as_deref(), Some("a b"));
        assert_eq!(name_from_query("other=1"), None);
    }
}
