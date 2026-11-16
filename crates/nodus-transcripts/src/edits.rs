//! The files a session changed with Claude's file tools (`Edit`,
//! `MultiEdit`, `Write`, `NotebookEdit`), read from its transcript as it
//! grows, for the card's `N files` (CARD-060).
//!
//! Unlike the metadata, every line counts here, so a large transcript is read
//! whole; only lines that can name a file or a failed call are parsed.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::Value;

use crate::lines::{Json, read_lines};
use crate::paths::mtime_ms;
use crate::value::{as_str, field};

/// The tools that change a file, with the input naming it.
fn changed_path<'a>(name: &str, input: Option<&'a Value>) -> Option<&'a str> {
    match name {
        "Edit" | "MultiEdit" | "Write" => as_str(field(input, "file_path")),
        "NotebookEdit" => as_str(field(input, "notebook_path")),
        _ => None,
    }
}

/// Lines worth parsing: those that may name a changed file, or say a call failed.
fn may_matter(raw: &str) -> bool {
    raw.contains("\"file_path\"") || raw.contains("\"notebook_path\"") || raw.contains("\"is_error\"")
}

/// What has been read of one transcript.
#[derive(Debug, Clone, Default)]
struct FileEdits {
    size: u64,
    mtime_ms: f64,
    offset: u64,
    /// Each call that changes a file: its id (none when the line has none) and the file.
    calls: Vec<(Option<String>, String)>,
    /// Calls whose result was an error, which changed nothing.
    failed: HashSet<String>,
}

impl FileEdits {
    fn apply(&mut self, line: &Json) {
        let content = field(line.get("message"), "content").and_then(Value::as_array);
        let Some(blocks) = content else { return };
        match line.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for block in blocks {
                    if as_str(field(Some(block), "type")) != Some("tool_use") {
                        continue;
                    }
                    let name = as_str(field(Some(block), "name")).unwrap_or_default();
                    if let Some(path) = changed_path(name, field(Some(block), "input")).filter(|p| !p.is_empty()) {
                        let id = as_str(field(Some(block), "id")).map(str::to_owned);
                        self.calls.push((id, path.to_owned()));
                    }
                }
            }
            Some("user") => {
                for block in blocks {
                    if as_str(field(Some(block), "type")) == Some("tool_result")
                        && field(Some(block), "is_error").and_then(Value::as_bool) == Some(true)
                        && let Some(id) = as_str(field(Some(block), "tool_use_id"))
                    {
                        self.failed.insert(id.to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    /// Each file a call that didn't fail changed, first changed first.
    fn files(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.calls
            .iter()
            .filter(|(id, _)| id.as_ref().is_none_or(|id| !self.failed.contains(id)))
            .filter(|(_, path)| seen.insert(path.as_str()))
            .map(|(_, path)| path.clone())
            .collect()
    }
}

/// What every transcript asked about so far changed, by file, for the life of
/// the app.
#[derive(Debug, Default)]
pub struct EditCache {
    files: Mutex<HashMap<PathBuf, FileEdits>>,
}

impl EditCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The files the transcript's calls changed, as Claude named them (absolute
    /// paths), first changed first. Read from where it was left off; none when
    /// the file isn't there.
    pub fn edited(&self, file: &Path) -> Vec<String> {
        let mut files = self.lock();
        let Ok(stat) = fs::metadata(file) else {
            files.remove(file);
            return Vec::new();
        };
        let (size, mtime) = (stat.len(), mtime_ms(&stat));
        let cached = files.get_mut(file);
        if let Some(cached) = &cached
            && cached.size == size
            && cached.mtime_ms == mtime
        {
            return cached.files();
        }
        let Ok(fd) = File::open(file) else {
            return cached.map(|c| c.files()).unwrap_or_default();
        };
        let keep: &dyn Fn(&str) -> bool = &may_matter;
        // Transcripts only grow; anything else is read again from the start.
        if let Some(cached) = cached
            && size > cached.size
        {
            cached.offset = read_lines(&fd, cached.offset, size, false, |l| cached.apply(l), Some(keep));
            cached.size = size;
            cached.mtime_ms = mtime;
            return cached.files();
        }
        let mut state = FileEdits {
            size,
            mtime_ms: mtime,
            ..FileEdits::default()
        };
        state.offset = read_lines(&fd, 0, size, false, |l| state.apply(l), Some(keep));
        let out = state.files();
        files.insert(file.to_owned(), state);
        out
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, FileEdits>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Those of `paths` inside the git repository `folder` is in, relative to its
/// root with `/` between names as git writes them, each once; none when
/// `folder` isn't in a repository.
pub fn in_repo(paths: &[String], folder: &Path) -> Vec<String> {
    let Some(root) = folder.ancestors().find(|dir| dir.join(".git").exists()) else {
        return Vec::new();
    };
    // Claude names files under the folder it really runs in (`/private/tmp`
    // for `/tmp`).
    let real = fs::canonicalize(root).ok();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in paths {
        let path = Path::new(path);
        let Some(rel) = path
            .strip_prefix(root)
            .ok()
            .or_else(|| real.as_deref().and_then(|r| path.strip_prefix(r).ok()))
        else {
            continue;
        };
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let rel = rel.join("/");
        if !rel.is_empty() && seen.insert(rel.clone()) {
            out.push(rel);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde_json::json;

    use super::*;

    fn edit(id: &str, name: &str, input: &Value) -> Value {
        json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": "Changing it"},
            {"type": "tool_use", "id": id, "name": name, "input": input},
        ]}})
    }

    fn result(id: &str, error: bool) -> Value {
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": id, "content": "done", "is_error": error},
        ]}})
    }

    fn append(file: &Path, lines: &[Value]) {
        let mut f = fs::OpenOptions::new().create(true).append(true).open(file).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    #[test]
    fn card_060_lists_the_files_a_session_changed() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("s.jsonl");
        append(
            &file,
            &[
                edit(
                    "a",
                    "Edit",
                    &json!({"file_path": "/r/src/a.rs", "old_string": "x", "new_string": "y"}),
                ),
                result("a", false),
                edit("b", "Read", &json!({"file_path": "/r/src/read.rs"})),
                edit("c", "Write", &json!({"file_path": "/r/new.md", "content": "hi"})),
                // A failed call changed nothing...
                edit("d", "Edit", &json!({"file_path": "/r/src/b.rs"})),
                result("d", true),
                edit(
                    "e",
                    "NotebookEdit",
                    &json!({"notebook_path": "/r/n.ipynb", "new_source": ""}),
                ),
                edit("f", "MultiEdit", &json!({"file_path": "/r/src/a.rs", "edits": []})),
            ],
        );
        let cache = EditCache::new();
        assert_eq!(cache.edited(&file), ["/r/src/a.rs", "/r/new.md", "/r/n.ipynb"]);
        // ...until one on the same file works; what's appended is read on its own.
        append(
            &file,
            &[
                edit("g", "Edit", &json!({"file_path": "/r/src/b.rs"})),
                result("g", false),
            ],
        );
        assert_eq!(
            cache.edited(&file),
            ["/r/src/a.rs", "/r/new.md", "/r/n.ipynb", "/r/src/b.rs"]
        );
        assert_eq!(cache.files.lock().unwrap()[&file].calls.len(), 6);
        // Gone, so forgotten.
        fs::remove_file(&file).unwrap();
        assert!(cache.edited(&file).is_empty());
        assert!(cache.files.lock().unwrap().is_empty());
    }

    #[test]
    fn card_060_keeps_the_files_in_the_repository() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("app/src")).unwrap();
        let abs = |rel: &str| root.join(rel).to_string_lossy().into_owned();
        let paths = [
            abs("app/src/a.rs"),
            abs("README.md"),
            abs("app/src/a.rs"),
            "/elsewhere/scratch/app/src/a.rs".to_owned(),
        ];
        // A project in a subfolder still names files from the repository's root.
        assert_eq!(in_repo(&paths, &root.join("app")), ["app/src/a.rs", "README.md"]);
        // Through the folder's real path too.
        let real = fs::canonicalize(&root).unwrap();
        let named = [real.join("x.rs").to_string_lossy().into_owned()];
        assert_eq!(in_repo(&named, &root), ["x.rs"]);
        // Not a repository: nothing to review.
        assert!(in_repo(&paths, dir.path()).is_empty());
    }
}
