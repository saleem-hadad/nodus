//! Reading 0.9.4's `localStorage` from Chromium's LevelDB
//! (`<userData>/Local Storage/leveldb`), once, so a draft, its attachments
//! and what was sent survive the upgrade.
//!
//! Chromium keeps each value under `_<origin>\0<key>`, the key and the value
//! each a byte saying how its string is stored (`0` UTF-16LE, `1` Latin-1)
//! followed by the string. LevelDB keeps recent writes in `*.log` files
//! (32 KiB blocks of records, each a write batch) and the rest in `*.ldb`
//! tables (blocks of prefix-compressed entries, mostly Snappy-compressed,
//! found through an index block). Every file present is read, and for each
//! key the write with the highest sequence number wins (a deletion included).

use std::collections::HashMap;
use std::path::Path;

/// One write: the user key, its sequence number, and the value (none for a deletion).
type Write = (Vec<u8>, u64, Option<Vec<u8>>);

/// The values `origin`'s pages stored, by key. Anything unreadable is skipped.
pub fn read(dir: &Path, origin: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut writes: Vec<Write> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        match ext {
            "log" => writes.extend(log_writes(&bytes)),
            "ldb" | "sst" => writes.extend(table_writes(&bytes).unwrap_or_default()),
            _ => {}
        }
    }
    let mut latest: HashMap<Vec<u8>, (u64, Option<Vec<u8>>)> = HashMap::new();
    for (key, seq, value) in writes {
        match latest.get(&key) {
            Some((at, _)) if *at > seq => {}
            _ => {
                latest.insert(key, (seq, value));
            }
        }
    }
    let mut prefix = vec![b'_'];
    prefix.extend_from_slice(origin.as_bytes());
    prefix.push(0);
    let mut values: Vec<(String, String)> = latest
        .into_iter()
        .filter_map(|(key, (_, value))| {
            let key = decode(key.strip_prefix(prefix.as_slice())?)?;
            Some((key, decode(&value?)?))
        })
        .collect();
    values.sort();
    values
}

/// A Chromium storage string: `0` + UTF-16LE, or `1` + Latin-1.
fn decode(bytes: &[u8]) -> Option<String> {
    let (&format, rest) = bytes.split_first()?;
    match format {
        0 => {
            if rest.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
            String::from_utf16(&units).ok()
        }
        1 => Some(rest.iter().map(|&b| char::from(b)).collect()),
        _ => None,
    }
}

/// A varint32/64 at `pos`, and where the next field starts.
fn varint(bytes: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(pos)?;
        pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, pos));
        }
    }
    None
}

/// A length-prefixed slice at `pos`, and where the next field starts.
fn sized(bytes: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    let (len, pos) = varint(bytes, pos)?;
    let end = pos.checked_add(usize::try_from(len).ok()?)?;
    Some((bytes.get(pos..end)?, end))
}

fn fixed32(bytes: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?))
}

fn fixed64(bytes: &[u8], pos: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(pos..pos + 8)?.try_into().ok()?))
}

// ---- *.log -------------------------------------------------------------------------

const BLOCK: usize = 32 * 1024;
const HEADER: usize = 7;

/// The write batches in a log file, whole (records put back together).
fn log_batches(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut batches = Vec::new();
    let mut pending: Option<Vec<u8>> = None;
    let mut pos = 0;
    while pos < bytes.len() {
        let left = BLOCK - pos % BLOCK;
        if left < HEADER {
            pos += left;
            continue;
        }
        let Some(header) = bytes.get(pos..pos + HEADER) else {
            break;
        };
        let len = usize::from(u16::from_le_bytes([header[4], header[5]]));
        let kind = header[6];
        let start = pos + HEADER;
        let end = start + len;
        if len > left - HEADER || end > bytes.len() {
            break;
        }
        if kind == 0 {
            // Preallocated zeroes: nothing more in this block.
            pos += left;
            pending = None;
            continue;
        }
        let data = &bytes[start..end];
        match kind {
            1 => batches.push(data.to_vec()),
            2 => pending = Some(data.to_vec()),
            3 => {
                if let Some(p) = pending.as_mut() {
                    p.extend_from_slice(data);
                }
            }
            4 => {
                if let Some(mut p) = pending.take() {
                    p.extend_from_slice(data);
                    batches.push(p);
                }
            }
            _ => pending = None,
        }
        pos = end;
    }
    batches
}

/// A write batch's writes: sequence, count, then each put or delete.
fn batch_writes(batch: &[u8]) -> Option<Vec<Write>> {
    let seq = fixed64(batch, 0)?;
    let count = fixed32(batch, 8)?;
    let mut pos = 12;
    let mut writes = Vec::new();
    for i in 0..u64::from(count) {
        let tag = *batch.get(pos)?;
        pos += 1;
        let (key, next) = sized(batch, pos)?;
        pos = next;
        let value = if tag == 1 {
            let (value, next) = sized(batch, pos)?;
            pos = next;
            Some(value.to_vec())
        } else {
            None
        };
        writes.push((key.to_vec(), seq + i, value));
    }
    Some(writes)
}

fn log_writes(bytes: &[u8]) -> Vec<Write> {
    log_batches(bytes)
        .iter()
        .filter_map(|batch| batch_writes(batch))
        .flatten()
        .collect()
}

// ---- *.ldb -------------------------------------------------------------------------

/// `kTableMagicNumber`, as the footer's last 8 bytes hold it.
const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const FOOTER: usize = 48;

/// A block's bytes, uncompressed, from its handle (offset, size).
fn block(bytes: &[u8], offset: u64, size: u64) -> Option<Vec<u8>> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(size).ok()?)?;
    let data = bytes.get(start..end)?;
    match *bytes.get(end)? {
        0 => Some(data.to_vec()),
        1 => snappy(data),
        _ => None,
    }
}

/// A block's entries: keys shared with the previous one, then values.
fn block_entries(block: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let restarts = usize::try_from(fixed32(block, block.len().checked_sub(4)?)?).ok()?;
    let limit = block.len().checked_sub(4 + 4 * restarts)?;
    let mut entries = Vec::new();
    let mut key: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos < limit {
        let (shared, next) = varint(block, pos)?;
        let (unshared, next) = varint(block, next)?;
        let (value_len, next) = varint(block, next)?;
        let shared = usize::try_from(shared).ok()?;
        let unshared = usize::try_from(unshared).ok()?;
        let value_len = usize::try_from(value_len).ok()?;
        key.truncate(shared);
        key.extend_from_slice(block.get(next..next + unshared)?);
        let value = block.get(next + unshared..next + unshared + value_len)?;
        entries.push((key.clone(), value.to_vec()));
        pos = next + unshared + value_len;
    }
    Some(entries)
}

fn table_writes(bytes: &[u8]) -> Option<Vec<Write>> {
    let footer = bytes.get(bytes.len().checked_sub(FOOTER)?..)?;
    if fixed64(footer, FOOTER - 8)? != MAGIC {
        return None;
    }
    let (_, pos) = varint(footer, 0)?;
    let (_, pos) = varint(footer, pos)?;
    let (index_offset, pos) = varint(footer, pos)?;
    let (index_size, _) = varint(footer, pos)?;
    let mut writes = Vec::new();
    for (_, handle) in block_entries(&block(bytes, index_offset, index_size)?)? {
        let (offset, pos) = varint(&handle, 0)?;
        let (size, _) = varint(&handle, pos)?;
        let Some(entries) = block(bytes, offset, size).and_then(|b| block_entries(&b)) else {
            continue;
        };
        for (internal, value) in entries {
            let Some(split) = internal.len().checked_sub(8) else {
                continue;
            };
            let trailer = fixed64(&internal, split)?;
            let value = (trailer & 0xff == 1).then_some(value);
            writes.push((internal[..split].to_vec(), trailer >> 8, value));
        }
    }
    Some(writes)
}

/// Snappy's raw format: the length, then literals and back-references.
fn snappy(input: &[u8]) -> Option<Vec<u8>> {
    let (len, mut pos) = varint(input, 0)?;
    let len = usize::try_from(len).ok()?;
    let mut out: Vec<u8> = Vec::with_capacity(len);
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        let (n, offset) = match tag & 3 {
            0 => {
                let mut n = usize::from(tag >> 2);
                if n >= 60 {
                    let extra = n - 59;
                    n = 0;
                    for i in 0..extra {
                        n |= usize::from(*input.get(pos + i)?) << (8 * i);
                    }
                    pos += extra;
                }
                let n = n + 1;
                out.extend_from_slice(input.get(pos..pos.checked_add(n)?)?);
                pos += n;
                continue;
            }
            1 => {
                let n = usize::from((tag >> 2) & 7) + 4;
                let offset = (usize::from(tag >> 5) << 8) | usize::from(*input.get(pos)?);
                pos += 1;
                (n, offset)
            }
            2 => {
                let n = usize::from(tag >> 2) + 1;
                let offset = usize::from(u16::from_le_bytes(input.get(pos..pos + 2)?.try_into().ok()?));
                pos += 2;
                (n, offset)
            }
            _ => {
                let n = usize::from(tag >> 2) + 1;
                let offset = usize::try_from(fixed32(input, pos)?).ok()?;
                pos += 4;
                (n, offset)
            }
        };
        if offset == 0 || offset > out.len() {
            return None;
        }
        let start = out.len() - offset;
        for i in 0..n {
            let byte = out[start + i];
            out.push(byte);
        }
    }
    (out.len() == len).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn put_sized(out: &mut Vec<u8>, bytes: &[u8]) {
        put_varint(out, bytes.len() as u64);
        out.extend_from_slice(bytes);
    }

    fn latin1(s: &str) -> Vec<u8> {
        let mut out = vec![1];
        out.extend(s.chars().map(|c| c as u8));
        out
    }

    fn utf16(s: &str) -> Vec<u8> {
        let mut out = vec![0];
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    fn key(origin: &str, name: &str) -> Vec<u8> {
        let mut out = vec![b'_'];
        out.extend_from_slice(origin.as_bytes());
        out.push(0);
        out.extend(latin1(name));
        out
    }

    /// A write batch: puts (with a value) and deletes, from `seq`.
    fn batch(seq: u64, writes: &[(Vec<u8>, Option<Vec<u8>>)]) -> Vec<u8> {
        let mut out = seq.to_le_bytes().to_vec();
        out.extend_from_slice(&(writes.len() as u32).to_le_bytes());
        for (k, v) in writes {
            out.push(u8::from(v.is_some()));
            put_sized(&mut out, k);
            if let Some(v) = v {
                put_sized(&mut out, v);
            }
        }
        out
    }

    /// A log file holding `batches`, the long ones split across blocks.
    fn log(batches: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for batch in batches {
            let mut rest: &[u8] = batch;
            let mut first = true;
            loop {
                let left = BLOCK - out.len() % BLOCK;
                if left < HEADER {
                    out.extend(std::iter::repeat_n(0, left));
                    continue;
                }
                let room = left - HEADER;
                let n = rest.len().min(room);
                let last = n == rest.len();
                let kind = match (first, last) {
                    (true, true) => 1,
                    (true, false) => 2,
                    (false, false) => 3,
                    (false, true) => 4,
                };
                out.extend_from_slice(&[0, 0, 0, 0]);
                out.extend_from_slice(&(n as u16).to_le_bytes());
                out.push(kind);
                out.extend_from_slice(&rest[..n]);
                rest = &rest[n..];
                first = false;
                if last {
                    break;
                }
            }
        }
        out
    }

    /// A block of entries, no shared prefixes, one restart.
    fn raw_block(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in entries {
            put_varint(&mut out, 0);
            put_varint(&mut out, k.len() as u64);
            put_varint(&mut out, v.len() as u64);
            out.extend_from_slice(k);
            out.extend_from_slice(v);
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out
    }

    /// Snappy with one literal for the first `lit` bytes and back-references
    /// copying the rest when it repeats them.
    fn snappy_of(data: &[u8], lit: usize) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, data.len() as u64);
        let n = lit - 1;
        if n < 60 {
            out.push((n as u8) << 2);
        } else {
            out.push(60 << 2);
            out.push(n as u8);
        }
        out.extend_from_slice(&data[..lit]);
        let mut done = lit;
        while done < data.len() {
            let n = (data.len() - done).min(64);
            // A two-byte-offset copy of what came `lit` bytes before.
            out.push((((n - 1) as u8) << 2) | 2);
            out.extend_from_slice(&(lit as u16).to_le_bytes());
            done += n;
        }
        out
    }

    /// A table with one data block (compressed with snappy or not).
    fn table(entries: &[(Vec<u8>, Vec<u8>)], compressed: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let raw = raw_block(entries);
        let data = if compressed { snappy_of(&raw, raw.len()) } else { raw };
        out.extend_from_slice(&data);
        out.push(u8::from(compressed));
        out.extend_from_slice(&[0; 4]);
        let mut handle = Vec::new();
        put_varint(&mut handle, 0);
        put_varint(&mut handle, data.len() as u64);
        let index_at = out.len();
        let index = raw_block(&[(entries.last().unwrap().0.clone(), handle)]);
        out.extend_from_slice(&index);
        out.push(0);
        out.extend_from_slice(&[0; 4]);
        let mut footer = Vec::new();
        put_varint(&mut footer, 0);
        put_varint(&mut footer, 0);
        put_varint(&mut footer, index_at as u64);
        put_varint(&mut footer, index.len() as u64);
        footer.resize(FOOTER - 8, 0);
        footer.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&footer);
        out
    }

    fn internal(user: Vec<u8>, seq: u64, put: bool) -> Vec<u8> {
        let mut out = user;
        out.extend_from_slice(&((seq << 8) | u64::from(put)).to_le_bytes());
        out
    }

    #[test]
    fn decompresses_snappy_literals_and_copies() {
        let data = b"abcdabcdabcdabcdab".to_vec();
        assert_eq!(snappy(&snappy_of(&data, 4)).unwrap(), data);
        let long: Vec<u8> = (0..200u8).collect();
        assert_eq!(snappy(&snappy_of(&long, 200)).unwrap(), long);
        // A copy reaching before the start is refused.
        assert_eq!(snappy(&[4, 0x0a, 0x05, 0x00]), None);
    }

    #[test]
    fn reads_the_newest_value_of_each_key_for_the_origin() {
        let dir = tempfile::tempdir().unwrap();
        let draft = key("file://", "capture.draft");
        let history = key("file://", "capture.history");
        let done = key("file://", "capture.clipboardDone");
        let elsewhere = key("http://localhost:5173", "capture.draft");
        // An older table: a draft since replaced, and history.
        std::fs::write(
            dir.path().join("000005.ldb"),
            table(
                &[
                    (internal(draft.clone(), 3, true), latin1("\"old\"")),
                    (
                        internal(history.clone(), 4, true),
                        utf16("[\"Fix the coupon field – now\"]"),
                    ),
                ],
                true,
            ),
        )
        .unwrap();
        // The log: the newer draft, a clipboard key put then deleted, and
        // another origin's draft; one batch long enough to span blocks.
        let long = format!("\"{}\"", "x".repeat(40_000));
        std::fs::write(
            dir.path().join("000007.log"),
            log(&[
                batch(10, &[(draft.clone(), Some(latin1("\"Fix the coupon field\"")))]),
                batch(11, &[(done.clone(), Some(latin1("\"t:x\""))), (done.clone(), None)]),
                batch(13, &[(elsewhere, Some(latin1(&long)))]),
            ]),
        )
        .unwrap();
        std::fs::write(dir.path().join("CURRENT"), "MANIFEST-000001\n").unwrap();
        assert_eq!(
            read(dir.path(), "file://"),
            vec![
                ("capture.draft".to_owned(), "\"Fix the coupon field\"".to_owned()),
                (
                    "capture.history".to_owned(),
                    "[\"Fix the coupon field – now\"]".to_owned()
                ),
            ]
        );
        assert_eq!(read(&dir.path().join("missing"), "file://"), Vec::new());
    }

    /// A log Electron 44 wrote (0.9.4's Chromium): a hidden `file://` page
    /// set three values, and set and removed a fourth.
    #[test]
    fn reads_what_electron_wrote() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("000003.log"),
            include_bytes!("testdata/electron-44.log"),
        )
        .unwrap();
        let history = format!("[\"one\",\"two\",\"{}\"]", "x".repeat(5000));
        assert_eq!(
            read(dir.path(), "file://"),
            vec![
                ("capture.clipboardDone".to_owned(), "\"t:abc\"".to_owned()),
                (
                    "capture.draft".to_owned(),
                    "\"Fix the coupon field – now ✓\"".to_owned()
                ),
                ("capture.history".to_owned(), history),
            ]
        );
    }

    #[test]
    fn reads_an_uncompressed_table_and_a_long_batch() {
        let dir = tempfile::tempdir().unwrap();
        let draft = key("file://", "capture.draft");
        std::fs::write(
            dir.path().join("000003.ldb"),
            table(&[(internal(draft.clone(), 1, true), latin1("\"a\""))], false),
        )
        .unwrap();
        let long = format!("\"{}\"", "é".repeat(20_000));
        std::fs::write(
            dir.path().join("000004.log"),
            log(&[batch(2, &[(draft, Some(latin1(&long)))])]),
        )
        .unwrap();
        assert_eq!(read(dir.path(), "file://"), vec![("capture.draft".to_owned(), long)]);
    }
}
