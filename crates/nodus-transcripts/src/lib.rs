//! Claude Code's transcripts in ~/.claude/projects: finding a project's
//! sessions, reading their metadata and messages as the files grow, searching
//! what was said and done in them, and the quick headless claude calls that
//! name a section or summarize sessions for a handoff.
//!
//! Ports `src/main/discovery.ts`, `src/main/search.ts`, `src/shared/search.ts`,
//! `src/main/naming.ts` and `src/main/summarize.ts`.
//!
//! [`Transcripts`] holds what's been read (the metadata cache and the search
//! index) for the life of the app; the free functions in each module are the
//! pieces other domains reuse (the status machine's agents, capture).

pub mod chat;
mod claude;
pub mod date;
pub mod edits;
pub mod fork;
pub mod lines;
pub mod meta;
pub mod naming;
mod nodeerr;
pub mod parse;
pub mod paths;
pub mod peers;
pub mod preview;
pub mod prs;
pub mod search;
pub mod slash;
pub mod summarize;
mod value;
pub mod watch;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use nodus_domain::{SessionSearchHit, TranscriptMessage};

pub use chat::ChatLog;
pub use claude::ClaudeCall;
pub use edits::EditCache;
pub use fork::{ForkError, fork_transcript};
pub use lines::{Json, read_lines};
pub use meta::{DiscoveredSession, MetaCache, meta_signature};
pub use naming::{NamingError, NamingKind, NamingSource, name_section};
pub use parse::{
    AssistantParts, HANDBACK_TOOL, UserText, assistant_parts, describe_tool_use, is_task_notification, message_text,
    prompt_full, time_of, tool_name, truncate, user_text,
};
pub use paths::{
    AgentFile, SessionFile, agents_signature, encode_repo_path, is_agent_id, list_agent_files, list_agent_files_in,
    list_session_files, real_repo_path, subagents_dir, transcript_dir, transcript_path,
};
pub use preview::{ReadError, read_transcript};
pub use search::{SearchIndex, SearchTarget, match_ranges, query_terms, terms_pattern};
pub use summarize::{SummarizeTarget, summarize_sessions};
pub use watch::{OnChange, Watcher};

/// What the app has read of the transcripts under one home folder.
pub struct Transcripts {
    home: PathBuf,
    meta: Arc<MetaCache>,
    edits: EditCache,
    search: SearchIndex,
}

impl Transcripts {
    /// The transcripts under `home`/.claude/projects.
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            meta: Arc::new(MetaCache::new()),
            edits: EditCache::new(),
            search: SearchIndex::new(),
        }
    }

    /// The transcripts under the user's home folder (`os.homedir()`).
    pub fn for_user() -> Option<Self> {
        std::env::home_dir().map(Self::new)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The metadata cache, shared with the [`Watcher`].
    pub fn meta(&self) -> &Arc<MetaCache> {
        &self.meta
    }

    /// A watcher of these transcripts' folders, sharing the metadata cache.
    pub fn watcher(&self, runtime: tokio::runtime::Handle) -> Watcher {
        Watcher::new(self.home.clone(), Arc::clone(&self.meta), runtime)
    }

    pub fn transcript_dir(&self, repo_path: &str) -> PathBuf {
        transcript_dir(&self.home, repo_path)
    }

    pub fn transcript_path(&self, repo_path: &str, session_id: &str) -> PathBuf {
        transcript_path(&self.home, repo_path, session_id)
    }

    pub fn subagents_dir(&self, repo_path: &str, session_id: &str) -> PathBuf {
        subagents_dir(&self.home, repo_path, session_id)
    }

    pub fn list_agent_files(&self, repo_path: &str, session_id: &str) -> Vec<AgentFile> {
        list_agent_files(&self.home, repo_path, session_id)
    }

    /// A transcript's metadata, read from where it was left off.
    pub fn read_session(&self, session_id: &str, file: &Path) -> Option<DiscoveredSession> {
        self.meta.read_session(session_id, file)
    }

    /// `readSession(id, transcriptPath(repoPath, id))`.
    pub fn session(&self, repo_path: &str, session_id: &str) -> Option<DiscoveredSession> {
        self.read_session(session_id, &self.transcript_path(repo_path, session_id))
    }

    /// [`Self::session`], where None means there's no transcript: one that
    /// may be there but couldn't be read (its folder or file not readable
    /// to this user) is an error instead, so it's never taken for a session
    /// nothing was said in (SESS-091).
    pub fn try_session(&self, repo_path: &str, session_id: &str) -> std::io::Result<Option<DiscoveredSession>> {
        let file = self.transcript_path(repo_path, session_id);
        if let Some(found) = self.read_session(session_id, &file) {
            return Ok(Some(found));
        }
        match std::fs::metadata(&file) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
            Ok(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} can't be read", file.display()),
            )),
        }
    }

    /// The files a session and its agents changed with Claude's file tools,
    /// relative to the root of the repository `repo_path` is in, first
    /// changed first (CARD-060).
    pub fn edited_files(&self, repo_path: &str, session_id: &str) -> Vec<String> {
        let mut paths = self.edits.edited(&self.transcript_path(repo_path, session_id));
        for agent in self.list_agent_files(repo_path, session_id) {
            paths.extend(self.edits.edited(&agent.file));
        }
        edits::in_repo(&paths, Path::new(repo_path))
    }

    /// Every session on disk for a repo, by id, in the folder's order.
    pub fn scan_repo(&self, repo_path: &str) -> IndexMap<String, DiscoveredSession> {
        self.meta.scan_repo(&self.home, repo_path)
    }

    /// Whether the session has a transcript yet (SESS-161).
    pub fn has_transcript(&self, repo_path: &str, session_id: &str) -> bool {
        self.transcript_path(repo_path, session_id).exists()
    }

    /// A session's conversation, for the preview (SESS-162).
    pub fn transcript(&self, repo_path: &str, session_id: &str) -> Result<Vec<TranscriptMessage>, ReadError> {
        read_transcript(&self.transcript_path(repo_path, session_id), false)
    }

    /// The board's sessions that hold every word of the query, best first.
    pub async fn search(&self, repo_path: &str, targets: &[SearchTarget], query: &str) -> Vec<SessionSearchHit> {
        self.search.search(&self.home, repo_path, targets, query).await
    }

    /// Lets go of what search has read.
    pub fn clear_search_index(&self) {
        self.search.clear();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_transcript_that_cant_be_read_is_an_error_not_none() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let repo = repo.to_string_lossy().into_owned();
        let transcripts = Transcripts::new(home.path().to_owned());
        // None there: nothing was said.
        assert!(matches!(transcripts.try_session(&repo, ID), Ok(None)));

        let file = transcripts.transcript_path(&repo, ID);
        let folder = file.parent().unwrap().to_owned();
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            &file,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
        )
        .unwrap();
        let found = transcripts.try_session(&repo, ID).unwrap().unwrap();
        assert_eq!(found.meta.first_prompt.as_deref(), Some("hello"));

        // As a server's `nodus` found Claude Code's folders (0700) before
        // 1.0.28: not even there, as far as it could tell.
        set_mode(&folder, 0o000);
        let root = fs::metadata(&file).is_ok();
        if !root {
            assert!(transcripts.session(&repo, ID).is_none());
            assert!(transcripts.try_session(&repo, ID).is_err());
        }
        // And its files (0600): there, but not to be opened.
        set_mode(&folder, 0o700);
        set_mode(&file, 0o000);
        let fresh = Transcripts::new(home.path().to_owned());
        if !root {
            assert!(fresh.session(&repo, ID).is_none());
            assert!(fresh.try_session(&repo, ID).is_err());
        }
        set_mode(&file, 0o600);
    }
}
