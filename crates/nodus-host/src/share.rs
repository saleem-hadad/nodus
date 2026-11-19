//! Letting `nodus serve` read the sessions' transcripts on a server
//! (SRV-005). It reads them as another user, `nodus`, through this host
//! user's group, but Claude Code writes its transcripts 0600 and their
//! folders 0700 whatever the umask. So a server's host adds the group's
//! read bits itself: over every transcript once as it starts, and over a
//! session's own on each of its hooks.
//!
//! Only what this user owns is changed, never through a symlink, and no bit
//! is ever taken away.

use std::path::PathBuf;
#[cfg(unix)]
use std::path::{Component, Path};

/// Whether a host shares its user's transcripts with its group: a server's
/// (`host --socket`, SRV-004), whose group is how `nodus serve` reads them.
/// Linux only, where servers run: on a Mac a user's group (`staff`) is
/// every user's, and there the standalone host only runs in tests.
pub(crate) fn shares(standalone: bool) -> bool {
    standalone && cfg!(target_os = "linux")
}

/// Where this user's Claude Code keeps its transcripts: `projects` in
/// `$CLAUDE_CONFIG_DIR` when that's set, else in `~/.claude`.
pub(crate) fn projects_dir() -> Option<PathBuf> {
    let claude = match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir()?.join(".claude"),
    };
    Some(claude.join("projects"))
}

/// How deep below `projects` the walk goes:
/// `<folder>/<session id>/subagents/agent-<id>.jsonl`.
#[cfg(unix)]
const DEPTH: usize = 4;

/// Group read, and for a folder group search too.
#[cfg(unix)]
const FILE_BITS: u32 = 0o040;
#[cfg(unix)]
const FOLDER_BITS: u32 = 0o050;

/// This process's user, who owns what it may change.
#[cfg(unix)]
pub(crate) fn user() -> u32 {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() }
}

/// Every transcript under `projects`, as the host starts. How many files
/// and folders it changed.
#[cfg(unix)]
pub(crate) fn share_all(projects: &Path, uid: u32) -> usize {
    let mut changed = share_way(projects, uid);
    share_tree(projects, uid, DEPTH, &mut changed);
    changed
}

/// One session's transcript, from a hook: its folder, the file, and its
/// subagents' beside it (`<session id>/`). Nothing but a transcript right
/// in a folder of `projects` is touched, whatever the hook says.
#[cfg(unix)]
pub(crate) fn share_session(projects: &Path, transcript: &Path, uid: u32) -> usize {
    let Ok(rest) = transcript.strip_prefix(projects) else {
        return 0;
    };
    let parts: Vec<Component<'_>> = rest.components().collect();
    let [Component::Normal(folder), Component::Normal(name)] = parts[..] else {
        return 0;
    };
    let Some(session) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")) else {
        return 0;
    };
    let folder = projects.join(folder);
    let mut changed = share_way(projects, uid);
    if !add(&folder, FOLDER_BITS, uid, &mut changed) {
        return changed;
    }
    add(transcript, FILE_BITS, uid, &mut changed);
    share_tree(&folder.join(session), uid, DEPTH - 2, &mut changed);
    changed
}

/// The way down to `projects`: the group may pass through Claude Code's own
/// folder, which it needn't list, and list `projects`.
#[cfg(unix)]
fn share_way(projects: &Path, uid: u32) -> usize {
    let mut changed = 0;
    if let Some(claude) = projects.parent() {
        add(claude, 0o010, uid, &mut changed);
    }
    add(projects, FOLDER_BITS, uid, &mut changed);
    changed
}

/// A folder of ours, and below it, `depth` folders down, its folders and
/// transcripts.
#[cfg(unix)]
fn share_tree(dir: &Path, uid: u32, depth: usize, changed: &mut usize) {
    if !add(dir, FOLDER_BITS, uid, changed) || depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // The entry itself: a symlink is neither.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            share_tree(&path, uid, depth - 1, changed);
        } else if kind.is_file() && is_transcript(&path) {
            add(&path, FILE_BITS, uid, changed);
        }
    }
}

/// A session's or subagent's transcript, or a subagent's description.
#[cfg(unix)]
fn is_transcript(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    name.ends_with(".jsonl") || name.ends_with(".meta.json")
}

/// Adds `bits` to the mode of a folder (bits with search) or a file (bits
/// without) that this user owns, through a descriptor opened without
/// following a symlink, so a symlink swapped in meanwhile isn't followed
/// either. Whether it's ours and of that kind, changed or not.
#[cfg(unix)]
fn add(path: &Path, bits: u32, uid: u32, changed: &mut usize) -> bool {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        // A FIFO would wait for a writer.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    let kind = if bits & 0o010 == 0 {
        meta.is_file()
    } else {
        meta.is_dir()
    };
    if meta.uid() != uid || !kind {
        return false;
    }
    let mode = meta.mode() & 0o7777;
    if mode & bits != bits
        && file
            .set_permissions(std::fs::Permissions::from_mode(mode | bits))
            .is_ok()
    {
        *changed += 1;
    }
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    const SESSION: &str = "00000000-0000-4000-8000-000000000001";

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// `.claude/projects` as Claude Code leaves it: a project's folder 0700,
    /// its transcripts 0600, a session's subagents beside it.
    struct Claude {
        _dir: tempfile::TempDir,
        claude: PathBuf,
        projects: PathBuf,
        folder: PathBuf,
        transcript: PathBuf,
        subagents: PathBuf,
        agent: PathBuf,
        agent_meta: PathBuf,
        other: PathBuf,
    }

    impl Claude {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let claude = dir.path().join(".claude");
            let projects = claude.join("projects");
            let folder = projects.join("-srv-projects-shop-worktrees-aa8a1c4d");
            let transcript = folder.join(format!("{SESSION}.jsonl"));
            let subagents = folder.join(SESSION).join("subagents");
            let agent = subagents.join("agent-a1.jsonl");
            let agent_meta = subagents.join("agent-a1.meta.json");
            let other = folder.join("notes.txt");
            fs::create_dir_all(&subagents).unwrap();
            for file in [&transcript, &agent, &agent_meta, &other] {
                fs::write(file, "{}\n").unwrap();
                set_mode(file, 0o600);
            }
            for folder in [&claude, &projects, &folder, &folder.join(SESSION), &subagents] {
                set_mode(folder, 0o700);
            }
            Self {
                _dir: dir,
                claude,
                projects,
                folder,
                transcript,
                subagents,
                agent,
                agent_meta,
                other,
            }
        }

        fn uid(&self) -> u32 {
            fs::metadata(&self.claude).unwrap().uid()
        }
    }

    #[test]
    fn only_a_servers_host_shares_and_only_on_linux() {
        assert!(!shares(false));
        assert_eq!(shares(true), cfg!(target_os = "linux"));
    }

    #[test]
    fn shares_every_transcript_with_the_group_adding_bits_only() {
        let claude = Claude::new();
        // A transcript someone made readable to everyone keeps that.
        set_mode(&claude.agent, 0o644);
        let changed = share_all(&claude.projects, claude.uid());
        assert_eq!(mode(&claude.claude), 0o710);
        assert_eq!(mode(&claude.projects), 0o750);
        assert_eq!(mode(&claude.folder), 0o750);
        assert_eq!(mode(&claude.folder.join(SESSION)), 0o750);
        assert_eq!(mode(&claude.subagents), 0o750);
        assert_eq!(mode(&claude.transcript), 0o640);
        assert_eq!(mode(&claude.agent), 0o644);
        assert_eq!(mode(&claude.agent_meta), 0o640);
        // Only transcripts.
        assert_eq!(mode(&claude.other), 0o600);
        assert_eq!(changed, 7);
        // Once is enough.
        assert_eq!(share_all(&claude.projects, claude.uid()), 0);
    }

    #[test]
    fn a_hook_shares_its_own_session_and_nothing_outside_projects() {
        let claude = Claude::new();
        let elsewhere = claude.folder.join("2-other.jsonl");
        fs::write(&elsewhere, "").unwrap();
        set_mode(&elsewhere, 0o600);
        share_session(&claude.projects, &claude.transcript, claude.uid());
        assert_eq!(mode(&claude.claude), 0o710);
        assert_eq!(mode(&claude.projects), 0o750);
        assert_eq!(mode(&claude.folder), 0o750);
        assert_eq!(mode(&claude.transcript), 0o640);
        assert_eq!(mode(&claude.subagents), 0o750);
        assert_eq!(mode(&claude.agent), 0o640);
        assert_eq!(mode(&claude.agent_meta), 0o640);
        // Another session's is left for its own hooks.
        assert_eq!(mode(&elsewhere), 0o600);

        // A path a hook made up: outside projects, too deep, or climbing out.
        let secret = claude.claude.join("secret.jsonl");
        fs::write(&secret, "").unwrap();
        set_mode(&secret, 0o600);
        for made_up in [
            secret.clone(),
            claude.agent.clone(),
            claude.folder.join("..").join("..").join("secret.jsonl"),
            claude.other.clone(),
        ] {
            assert_eq!(
                share_session(&claude.projects, &made_up, claude.uid()),
                0,
                "{made_up:?}"
            );
        }
        assert_eq!(mode(&secret), 0o600);
        assert_eq!(mode(&claude.other), 0o600);
    }

    #[test]
    fn never_follows_a_symlink() {
        let claude = Claude::new();
        let outside = tempfile::tempdir().unwrap();
        let target_dir = outside.path().join("dir");
        let target_file = outside.path().join("file.jsonl");
        fs::create_dir(&target_dir).unwrap();
        set_mode(&target_dir, 0o700);
        fs::write(&target_file, "").unwrap();
        set_mode(&target_file, 0o600);
        std::os::unix::fs::symlink(&target_dir, claude.projects.join("-linked")).unwrap();
        std::os::unix::fs::symlink(&target_file, claude.folder.join("linked.jsonl")).unwrap();
        std::os::unix::fs::symlink(&target_file, claude.folder.join("2-linked.jsonl")).unwrap();
        share_all(&claude.projects, claude.uid());
        share_session(&claude.projects, &claude.folder.join("2-linked.jsonl"), claude.uid());
        assert_eq!(mode(&target_dir), 0o700);
        assert_eq!(mode(&target_file), 0o600);
        // The rest still is.
        assert_eq!(mode(&claude.transcript), 0o640);
    }

    #[test]
    fn leaves_what_another_user_owns() {
        let claude = Claude::new();
        // Not this host's user: as if every file were someone else's.
        let stranger = claude.uid().wrapping_add(1);
        assert_eq!(share_all(&claude.projects, stranger), 0);
        assert_eq!(share_session(&claude.projects, &claude.transcript, stranger), 0);
        assert_eq!(mode(&claude.projects), 0o700);
        assert_eq!(mode(&claude.folder), 0o700);
        assert_eq!(mode(&claude.transcript), 0o600);
    }
}
