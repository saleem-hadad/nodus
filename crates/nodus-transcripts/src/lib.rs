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
pub mod lines;
pub mod meta;
pub mod naming;
mod nodeerr;
pub mod parse;
pub mod paths;
pub mod preview;
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
    search: SearchIndex,
}

impl Transcripts {
    /// The transcripts under `home`/.claude/projects.
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            meta: Arc::new(MetaCache::new()),
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
