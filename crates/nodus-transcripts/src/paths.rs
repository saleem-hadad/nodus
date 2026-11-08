//! Where Claude Code keeps a project's transcripts:
//! `<home>/.claude/projects/<encoded repo path>/<session id>.jsonl`, and each
//! session's subagents in `<session id>/subagents/agent-<agent id>.jsonl`.
//!
//! The folder name depends on the repo path exactly as Claude Code computes
//! it, so the repo path is resolved the way Node's `fs.realpathSync` does
//! (symlinks resolved one component at a time, the case of everything else
//! kept as given, no `\\?\` on Windows) rather than with `canonicalize`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use nodus_domain::json::number_to_string;

/// A session's transcript file, as listed in the project's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    pub session_id: String,
    pub file: PathBuf,
}

/// A subagent's transcript, beside its session's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    pub agent_id: String,
    pub file: PathBuf,
    /// agent-<id>.meta.json: the agent's type and description, rewritten when it ends.
    pub meta_file: PathBuf,
}

/// Agent ids are used in paths, so only ever accept this shape: `/^[\w-]+$/`
/// with JavaScript's ASCII `\w` (SESS-008).
pub fn is_agent_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(is_word_or_dash)
}

fn is_word_or_dash(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// The repo path with its symlinks resolved, as `fs.realpathSync` gives it;
/// the path as given when it can't be resolved (SESS-002).
pub fn real_repo_path(repo_path: &str) -> String {
    realpath(repo_path).unwrap_or_else(|| repo_path.to_owned())
}

/// The transcript folder's name: the real repo path with every UTF-16 unit
/// that isn't `[a-zA-Z0-9]` replaced by `-`, so a character outside the BMP
/// becomes two (SESS-003).
pub fn encode_repo_path(repo_path: &str) -> String {
    let real = real_repo_path(repo_path);
    let mut out = String::with_capacity(real.len());
    for c in real.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// `<home>/.claude/projects`, where Claude Code keeps every project's transcripts.
pub fn projects_dir(home: &Path) -> PathBuf {
    home.join(".claude").join("projects")
}

/// A project's transcript folder (SESS-001).
pub fn transcript_dir(home: &Path, repo_path: &str) -> PathBuf {
    projects_dir(home).join(encode_repo_path(repo_path))
}

/// A session's transcript (SESS-004).
pub fn transcript_path(home: &Path, repo_path: &str, session_id: &str) -> PathBuf {
    transcript_dir(home, repo_path).join(format!("{session_id}.jsonl"))
}

/// The project's session transcripts, in the order the folder lists them; none
/// when it can't be read (SESS-005).
pub fn list_session_files(home: &Path, repo_path: &str) -> Vec<SessionFile> {
    let dir = transcript_dir(home, repo_path);
    let Ok(names) = read_dir_names(&dir) else {
        return Vec::new();
    };
    names
        .into_iter()
        .filter(|name| is_session_file_name(name))
        .map(|name| SessionFile {
            session_id: name[..name.len() - ".jsonl".len()].to_owned(),
            file: dir.join(&name),
        })
        .collect()
}

/// `/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.jsonl$/i`.
fn is_session_file_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() != 36 + 6 || !name[36..].eq_ignore_ascii_case(".jsonl") {
        return false;
    }
    bytes[..36].iter().enumerate().all(|(i, &b)| match i {
        8 | 13 | 18 | 23 => b == b'-',
        _ => b.is_ascii_hexdigit(),
    })
}

/// Where Claude Code keeps the transcripts of a session's subagents (SESS-006).
pub fn subagents_dir(home: &Path, repo_path: &str, session_id: &str) -> PathBuf {
    transcript_dir(home, repo_path).join(session_id).join("subagents")
}

/// A session's subagent transcripts, in the order the folder lists them
/// (SESS-007).
pub fn list_agent_files(home: &Path, repo_path: &str, session_id: &str) -> Vec<AgentFile> {
    list_agent_files_in(&subagents_dir(home, repo_path, session_id))
}

/// The subagent transcripts in a session's `subagents` folder: every
/// `agent-<id>.jsonl` with an id of `[\w-]+`, in the order the folder lists
/// them; none when it can't be read (SESS-007, HOST-183).
pub fn list_agent_files_in(dir: &Path) -> Vec<AgentFile> {
    let Ok(names) = read_dir_names(dir) else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|name| {
            // /^agent-([\w-]+)\.jsonl$/
            let agent_id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
            is_agent_id(agent_id).then(|| AgentFile {
                agent_id: agent_id.to_owned(),
                file: dir.join(name),
                meta_file: dir.join(format!("agent-{agent_id}.meta.json")),
            })
        })
        .collect()
}

/// A cheap fingerprint of a session's subagent files for the watcher:
/// `count:bytes:latest mtime` over every entry of its subagents folder, or ''
/// when the folder can't be read (SESS-009).
pub fn agents_signature(home: &Path, repo_path: &str, session_id: &str) -> String {
    let dir = subagents_dir(home, repo_path, session_id);
    let Ok(names) = read_dir_names(&dir) else {
        return String::new();
    };
    let mut bytes = 0.0;
    let mut latest: f64 = 0.0;
    for name in &names {
        // One removed while we looked still counts, with nothing to add.
        if let Ok(meta) = fs::metadata(dir.join(name)) {
            bytes += meta.len() as f64;
            latest = latest.max(mtime_ms(&meta));
        }
    }
    format!(
        "{}:{}:{}",
        names.len(),
        number_to_string(bytes),
        number_to_string(latest)
    )
}

/// A folder's entry names as Node's `fs.readdirSync` lists them. libuv sorts
/// them byte by byte (`strcmp`) on macOS and Linux; Windows lists them in the
/// file system's order.
pub fn read_dir_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        names.push(entry?.file_name());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    }
    Ok(names.into_iter().map(|n| n.to_string_lossy().into_owned()).collect())
}

/// A file's `stat.mtimeMs`: seconds × 1000 plus nanoseconds ÷ 1e6, as Node
/// works it out, so it can have a fraction.
pub fn mtime_ms(meta: &fs::Metadata) -> f64 {
    let Ok(modified) = meta.modified() else {
        return 0.0;
    };
    let (secs, nanos) = match modified.duration_since(UNIX_EPOCH) {
        Ok(after) => (after.as_secs() as f64, f64::from(after.subsec_nanos())),
        Err(before) => {
            // A timespec before 1970: negative seconds, and nanoseconds forward from them.
            let before = before.duration();
            let carry = u32::from(before.subsec_nanos() > 0);
            let secs = -(before.as_secs() as f64) - f64::from(carry);
            let nanos = if carry == 1 {
                1e9 - f64::from(before.subsec_nanos())
            } else {
                0.0
            };
            (secs, nanos)
        }
    };
    secs * 1000.0 + nanos / 1e6
}

/// Node's `path.basename`: the last part of a path, trailing separators
/// ignored. Windows splits on `\` and `/` and skips a drive letter; elsewhere
/// only `/` separates.
pub fn basename(path: &str) -> &str {
    if cfg!(windows) {
        basename_win32(path)
    } else {
        basename_posix(path)
    }
}

pub(crate) fn basename_posix(path: &str) -> &str {
    basename_with(path, 0, |b| b == b'/')
}

pub(crate) fn basename_win32(path: &str) -> &str {
    let bytes = path.as_bytes();
    // A drive letter, so the separator after it isn't taken for a trailing one.
    let start = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        2
    } else {
        0
    };
    basename_with(path, start, |b| b == b'/' || b == b'\\')
}

fn basename_with(path: &str, mut start: usize, is_sep: impl Fn(u8) -> bool) -> &str {
    let bytes = path.as_bytes();
    let mut end = None;
    let mut i = bytes.len();
    while i > start {
        i -= 1;
        if is_sep(bytes[i]) {
            if end.is_some() {
                start = i + 1;
                break;
            }
        } else if end.is_none() {
            end = Some(i + 1);
        }
    }
    match end {
        Some(end) => &path[start..end],
        None => "",
    }
}

/// The path separator Node's `path.sep` is.
pub const SEP: char = if cfg!(windows) { '\\' } else { '/' };

// ---------------------------------------------------------------------------
// fs.realpathSync, as Node's JavaScript implements it

fn realpath(given: &str) -> Option<String> {
    if given.contains('\0') {
        return None;
    }
    let windows = cfg!(windows);
    let mut p = resolve(&[given]);
    let mut seen_links: HashMap<(u64, u64), String> = HashMap::new();
    let mut known_hard: HashSet<String> = HashSet::new();

    let mut current = split_root(&p).to_owned();
    let mut base = current.clone();
    let mut pos = current.len();
    if windows {
        fs::symlink_metadata(&base).ok()?;
        known_hard.insert(base.clone());
    }

    while pos < p.len() {
        let previous = current.clone();
        match next_part(&p, pos) {
            None => {
                let last = &p[pos..];
                current.push_str(last);
                base = format!("{previous}{last}");
                pos = p.len();
            }
            Some(sep) => {
                current.push_str(&p[pos..=sep]);
                base = format!("{previous}{}", &p[pos..sep]);
                pos = sep + 1;
            }
        }
        if known_hard.contains(&base) {
            continue;
        }
        let stats = fs::symlink_metadata(&base).ok()?;
        if !stats.file_type().is_symlink() {
            known_hard.insert(base.clone());
            continue;
        }
        let id = file_id(&stats);
        let link_target = match id.and_then(|id| seen_links.get(&id)) {
            Some(target) => target.clone(),
            None => {
                // Fails for a dangling link, or a loop.
                fs::metadata(&base).ok()?;
                read_link(&base)?
            }
        };
        let resolved_link = resolve(&[&previous, &link_target]);
        if let Some(id) = id {
            seen_links.insert(id, link_target);
        }
        p = resolve(&[&resolved_link, &p[pos..]]);
        current = split_root(&p).to_owned();
        base = current.clone();
        pos = current.len();
        if windows && !known_hard.contains(&base) {
            fs::symlink_metadata(&base).ok()?;
            known_hard.insert(base.clone());
        }
    }
    Some(p)
}

#[cfg(unix)]
fn file_id(stats: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((stats.dev(), stats.ino()))
}

/// Windows reports no device and inode, so links are read every time.
#[cfg(not(unix))]
fn file_id(_: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

fn read_link(path: &str) -> Option<String> {
    let target = fs::read_link(path).ok()?.to_string_lossy().into_owned();
    // libuv gives a junction's target as C:\…, where Rust keeps \\?\C:\….
    if cfg!(windows)
        && let Some(rest) = target.strip_prefix(r"\\?\")
        && !rest.starts_with("UNC\\")
    {
        return Some(rest.to_owned());
    }
    Some(target)
}

fn is_sep(b: u8) -> bool {
    b == b'/' || (cfg!(windows) && b == b'\\')
}

/// The root at the start of a path: leading slashes, or on Windows a drive or
/// UNC share and the separators after it.
fn split_root(p: &str) -> &str {
    let bytes = p.as_bytes();
    if !cfg!(windows) {
        let end = bytes.iter().position(|&b| b != b'/').unwrap_or(bytes.len());
        return &p[..end];
    }
    // /^(?:[a-zA-Z]:|[\\/]{2}[^\\/]+[\\/][^\\/]+)?[\\/]*/
    let mut i = 0;
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        i = 2;
    } else if bytes.len() >= 2 && is_sep(bytes[0]) && is_sep(bytes[1]) {
        let server = 2 + bytes[2..].iter().take_while(|&&b| !is_sep(b)).count();
        if server > 2 && server < bytes.len() {
            let share = server + 1 + bytes[server + 1..].iter().take_while(|&&b| !is_sep(b)).count();
            if share > server + 1 {
                i = share;
            }
        }
    }
    while i < bytes.len() && is_sep(bytes[i]) {
        i += 1;
    }
    &p[..i]
}

fn next_part(p: &str, from: usize) -> Option<usize> {
    p.as_bytes()[from..].iter().position(|&b| is_sep(b)).map(|i| from + i)
}

/// Node's `path.resolve` for this platform.
pub(crate) fn resolve(paths: &[&str]) -> String {
    if cfg!(windows) {
        resolve_win32(paths)
    } else {
        resolve_posix(paths)
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(crate) fn resolve_posix(paths: &[&str]) -> String {
    let mut resolved = String::new();
    let mut absolute = false;
    let cwd = cwd();
    for path in paths.iter().rev().copied().chain(std::iter::once(cwd.as_str())) {
        if absolute {
            break;
        }
        if path.is_empty() {
            continue;
        }
        resolved = format!("{path}/{resolved}");
        absolute = path.starts_with('/');
    }
    let normalized = normalize_string(&resolved, !absolute, '/', |b| b == b'/');
    if absolute {
        format!("/{normalized}")
    } else if normalized.is_empty() {
        ".".into()
    } else {
        normalized
    }
}

pub(crate) fn resolve_win32(paths: &[&str]) -> String {
    let is_sep = |b: u8| b == b'/' || b == b'\\';
    let mut resolved_device = String::new();
    let mut resolved_tail = String::new();
    let mut resolved_absolute = false;
    let mut i = paths.len() as isize - 1;
    while i >= -1 {
        let path: String = if i >= 0 {
            let path = paths[i as usize];
            if path.is_empty() {
                i -= 1;
                continue;
            }
            path.to_owned()
        } else if resolved_device.is_empty() {
            cwd()
        } else {
            // The drive's own current folder, or its root.
            let drive_cwd = std::env::var(format!("={resolved_device}")).unwrap_or_else(|_| cwd());
            let b = drive_cwd.as_bytes();
            if drive_cwd.get(..2).map(str::to_lowercase) != Some(resolved_device.to_lowercase())
                && b.get(2) == Some(&b'\\')
            {
                format!("{resolved_device}\\")
            } else {
                drive_cwd
            }
        };
        let bytes = path.as_bytes();
        let len = bytes.len();
        let mut root_end = 0;
        let mut device = String::new();
        let mut is_absolute = false;
        if len == 1 {
            if is_sep(bytes[0]) {
                root_end = 1;
                is_absolute = true;
            }
        } else if is_sep(bytes[0]) {
            is_absolute = true;
            if is_sep(bytes[1]) {
                let mut j = 2;
                let mut last = j;
                while j < len && !is_sep(bytes[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    let first_part = &path[last..j];
                    last = j;
                    while j < len && is_sep(bytes[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !is_sep(bytes[j]) {
                            j += 1;
                        }
                        if j == len || j != last {
                            device = format!("\\\\{first_part}\\{}", &path[last..j]);
                            root_end = j;
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            device = path[..2].to_owned();
            root_end = 2;
            if len > 2 && is_sep(bytes[2]) {
                is_absolute = true;
                root_end = 3;
            }
        }

        if !device.is_empty() {
            if resolved_device.is_empty() {
                resolved_device = device;
            } else if device.to_lowercase() != resolved_device.to_lowercase() {
                // Another drive's path doesn't apply.
                i -= 1;
                continue;
            }
        }
        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            resolved_tail = format!("{}\\{resolved_tail}", &path[root_end..]);
            resolved_absolute = is_absolute;
            if is_absolute && !resolved_device.is_empty() {
                break;
            }
        }
        i -= 1;
    }
    let tail = normalize_string(&resolved_tail, !resolved_absolute, '\\', is_sep);
    if resolved_absolute {
        format!("{resolved_device}\\{tail}")
    } else {
        let joined = format!("{resolved_device}{tail}");
        if joined.is_empty() { ".".into() } else { joined }
    }
}

/// Node's `normalizeString`: drops `.` parts and empty ones, and takes `..`
/// back a part (or keeps it, above a relative path's start).
fn normalize_string(path: &str, allow_above_root: bool, separator: char, is_sep: impl Fn(u8) -> bool) -> String {
    let bytes = path.as_bytes();
    let mut res = String::new();
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code = 0u8;
    let mut i = 0usize;
    while i <= bytes.len() {
        if i < bytes.len() {
            code = bytes[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/';
        }
        let at_sep = if i < bytes.len() { is_sep(code) } else { true };
        if at_sep {
            if last_slash == i as isize - 1 || dots == 1 {
                // Nothing to add.
            } else if dots == 2 {
                let ends_in_dots =
                    res.len() >= 2 && last_segment_length == 2 && res.as_bytes()[res.len() - 2..] == *b"..";
                if !ends_in_dots {
                    if res.len() > 2 {
                        match res.rfind(separator) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(at) => {
                                res.truncate(at);
                                last_segment_length = match res.rfind(separator) {
                                    Some(sep) => res.len() - 1 - sep,
                                    None => res.len(),
                                };
                            }
                        }
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if res.is_empty() {
                        res.push_str("..");
                    } else {
                        res.push(separator);
                        res.push_str("..");
                    }
                    last_segment_length = 2;
                }
            } else {
                let segment = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(separator);
                }
                res.push_str(segment);
                last_segment_length = (i as isize - last_slash - 1) as usize;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == b'.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_utf16_unit_that_isnt_a_letter_or_digit() {
        // A path that doesn't exist isn't resolved.
        assert_eq!(encode_repo_path("/work/my.repo"), "-work-my-repo");
        assert_eq!(encode_repo_path("/work/café"), "-work-caf-");
        assert_eq!(encode_repo_path("/work/a😀b"), "-work-a--b");
        assert_eq!(encode_repo_path("/work/a_b c"), "-work-a-b-c");
    }

    #[test]
    fn checks_session_and_agent_names() {
        assert!(is_session_file_name("00000000-0000-4000-8000-000000000001.jsonl"));
        assert!(is_session_file_name("ABCDEF00-0000-4000-8000-000000000001.JSONL"));
        assert!(!is_session_file_name("00000000-0000-4000-8000-00000000000g.jsonl"));
        assert!(!is_session_file_name("00000000-0000-4000-8000-000000000001.json"));
        assert!(!is_session_file_name("00000000x0000-4000-8000-000000000001.jsonl"));
        assert!(is_agent_id("a1-b_C"));
        assert!(!is_agent_id(""));
        assert!(!is_agent_id("../x"));
        assert!(!is_agent_id("é"));
    }

    #[test]
    fn lists_sessions_and_agents_in_name_order() {
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let repo = repo.to_str().unwrap();
        let dir = transcript_dir(home.path(), repo);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "00000000-0000-4000-8000-000000000002.jsonl",
            "00000000-0000-4000-8000-000000000001.jsonl",
            "notes.txt",
            "00000000-0000-4000-8000-000000000003.json",
        ] {
            fs::write(dir.join(name), "").unwrap();
        }
        let ids: Vec<String> = list_session_files(home.path(), repo)
            .into_iter()
            .map(|f| f.session_id)
            .collect();
        if cfg!(unix) {
            assert_eq!(
                ids,
                [
                    "00000000-0000-4000-8000-000000000001",
                    "00000000-0000-4000-8000-000000000002"
                ]
            );
        } else {
            assert_eq!(ids.len(), 2);
        }

        let id = "00000000-0000-4000-8000-000000000001";
        let agents = subagents_dir(home.path(), repo, id);
        assert_eq!(agents_signature(home.path(), repo, id), "");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("agent-b2.jsonl"), "12345").unwrap();
        fs::write(agents.join("agent-a1.jsonl"), "12").unwrap();
        fs::write(agents.join("agent-a1.meta.json"), "{}").unwrap();
        fs::write(agents.join("agent-x.y.jsonl"), "").unwrap();
        let found = list_agent_files(home.path(), repo, id);
        let names: Vec<&str> = found.iter().map(|a| a.agent_id.as_str()).collect();
        if cfg!(unix) {
            assert_eq!(names, ["a1", "b2"]);
        }
        let a1 = found.iter().find(|a| a.agent_id == "a1").unwrap();
        assert_eq!(a1.meta_file, agents.join("agent-a1.meta.json"));
        let signature = agents_signature(home.path(), repo, id);
        assert!(signature.starts_with("4:9:"), "{signature}");
    }

    #[test]
    fn missing_folders_list_nothing() {
        let home = tempfile::TempDir::new().unwrap();
        assert!(list_session_files(home.path(), "/nowhere/repo").is_empty());
        assert!(list_agent_files(home.path(), "/nowhere/repo", "x").is_empty());
    }

    #[test]
    fn takes_basenames_as_node_does() {
        assert_eq!(basename_posix("/repo/src/auth.ts"), "auth.ts");
        assert_eq!(basename_posix("/repo/src/"), "src");
        assert_eq!(basename_posix("auth.ts"), "auth.ts");
        assert_eq!(basename_posix("/"), "");
        assert_eq!(basename_posix(r"C:\repo\auth.ts"), r"C:\repo\auth.ts");
        assert_eq!(basename_win32(r"C:\repo\auth.ts"), "auth.ts");
        assert_eq!(basename_win32("C:/repo/src/"), "src");
        assert_eq!(basename_win32("C:foo"), "foo");
        assert_eq!(basename_win32(r"C:\"), "");
    }

    #[test]
    fn resolves_paths_as_node_does() {
        assert_eq!(resolve_posix(&["/a/b/../c/./d/"]), "/a/c/d");
        assert_eq!(resolve_posix(&["/a/b", "../x"]), "/a/x");
        assert_eq!(resolve_posix(&["/a/b", "/x/y"]), "/x/y");
        assert_eq!(resolve_posix(&["//a//b"]), "/a/b");
        assert_eq!(resolve_posix(&["/.."]), "/");
        assert_eq!(resolve_win32(&[r"C:\a\b\..\c"]), r"C:\a\c");
        assert_eq!(resolve_win32(&["C:/a/b/"]), r"C:\a\b");
        assert_eq!(resolve_win32(&[r"C:\a", r"..\x"]), r"C:\x");
        assert_eq!(resolve_win32(&[r"\\server\share\a\..\b"]), r"\\server\share\b");
        assert_eq!(resolve_win32(&[r"c:\Work"]), r"c:\Work");
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlinks_and_keeps_the_given_case() {
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("Repo");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let real_str = real.to_str().unwrap();
        let canonical = fs::canonicalize(&real).unwrap();
        // The temp folder itself may sit behind a symlink (/var → /private/var).
        assert_eq!(real_repo_path(real_str), canonical.to_str().unwrap());
        assert_eq!(real_repo_path(link.to_str().unwrap()), canonical.to_str().unwrap());
        // On a case-insensitive disk the given case is kept, where canonicalize would fix it.
        let shouted = dir.path().join("REPO");
        assert!(real_repo_path(shouted.to_str().unwrap()).ends_with("/REPO"));
        // Through a relative link.
        let relative = dir.path().join("rel");
        std::os::unix::fs::symlink("Repo", &relative).unwrap();
        assert_eq!(real_repo_path(relative.to_str().unwrap()), canonical.to_str().unwrap());
        // A dangling link, or a path that isn't there, comes back as given.
        let dangling = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("gone"), &dangling).unwrap();
        assert_eq!(real_repo_path(dangling.to_str().unwrap()), dangling.to_str().unwrap());
        assert_eq!(real_repo_path("/nowhere/at/all"), "/nowhere/at/all");
        assert_eq!(
            encode_repo_path(link.to_str().unwrap()),
            encode_repo_path(canonical.to_str().unwrap())
        );
    }

    #[test]
    fn reads_mtimes_in_milliseconds() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("f");
        fs::write(&file, "x").unwrap();
        let ms = mtime_ms(&fs::metadata(&file).unwrap());
        assert!(ms > 1.7e12, "{ms}");
    }
}
