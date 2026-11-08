//! Loading state.json once, keeping it in memory, and saving it back.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nodus_domain::json;
use serde_json::Value;

use crate::state::State;

/// How long a save waits for more changes. Each change starts the wait again,
/// so a burst of them is written once.
pub const SAVE_DELAY: Duration = Duration::from_millis(300);

/// The app's name before Nodus. Its data sits beside ours in Application Support.
const OLD_APP_NAME: &str = "Claude Nodes";

/// The app's state, loaded from state.json in the data folder and saved back
/// to it, shared between threads.
///
/// Changes go through [`Store::update`], which saves them a moment later
/// ([`SAVE_DELAY`]); [`Store::flush`] writes what's waiting straight away, as
/// the app does when it quits. Each save writes a temporary file and renames
/// it over state.json, so the file is never left half written.
pub struct Store {
    shared: Arc<Shared>,
    saver: Option<JoinHandle<()>>,
}

struct Shared {
    file: PathBuf,
    state: Mutex<State>,
    saving: Mutex<Saving>,
    changed: Condvar,
}

#[derive(Default)]
struct Saving {
    /// When the pending save is due; none when nothing is waiting.
    due: Option<Instant>,
    closing: bool,
}

impl Store {
    /// Loads state.json from the data folder as loadState does. A missing
    /// file, or one that isn't JSON, starts the app empty; one this version
    /// can't read is moved aside first. The first launch under this name
    /// brings the old app's state over before anything is read.
    pub fn open(user_data: &Path) -> io::Result<Self> {
        let file = user_data.join("state.json");
        migrate_old_state(user_data, &file);
        let state = load(&file);
        let shared = Arc::new(Shared {
            file,
            state: Mutex::new(state),
            saving: Mutex::new(Saving::default()),
            changed: Condvar::new(),
        });
        let saver = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("nodus-store".into())
                .spawn(move || shared.run_saver())?
        };
        Ok(Self {
            shared,
            saver: Some(saver),
        })
    }

    /// The state.json this store saves to.
    pub fn file(&self) -> &Path {
        &self.shared.file
    }

    /// Reads the state.
    pub fn read<R>(&self, read: impl FnOnce(&State) -> R) -> R {
        read(&lock(&self.shared.state))
    }

    /// Changes the state, and saves it a moment later.
    pub fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let result = change(&mut lock(&self.shared.state));
        self.save();
        result
    }

    /// Saves the state a moment later, once changes stop coming.
    pub fn save(&self) {
        lock(&self.shared.saving).due = Some(Instant::now() + SAVE_DELAY);
        self.shared.changed.notify_all();
    }

    /// Writes a pending save now, waiting for one already being written.
    /// Does nothing when nothing is waiting.
    pub fn flush(&self) {
        let mut saving = lock(&self.shared.saving);
        let due = saving.due.take();
        // The saver takes the state before it lets go of `saving`, so a save
        // it started has finished by the time this has the state.
        let state = lock(&self.shared.state);
        drop(saving);
        if due.is_some() {
            self.shared.write_logged(&state, "flush");
        }
    }
}

impl Drop for Store {
    /// Stops the saver, and writes what was still waiting to be saved.
    fn drop(&mut self) {
        lock(&self.shared.saving).closing = true;
        self.shared.changed.notify_all();
        if let Some(saver) = self.saver.take() {
            let _ = saver.join();
        }
        self.flush();
    }
}

impl Shared {
    fn run_saver(&self) {
        let mut saving = lock(&self.saving);
        loop {
            if saving.closing {
                return;
            }
            let Some(due) = saving.due else {
                saving = self.changed.wait(saving).unwrap_or_else(PoisonError::into_inner);
                continue;
            };
            let now = Instant::now();
            if now < due {
                saving = self
                    .changed
                    .wait_timeout(saving, due - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            saving.due = None;
            let state = lock(&self.state);
            drop(saving);
            self.write_logged(&state, "save");
            drop(state);
            saving = lock(&self.saving);
        }
    }

    fn write_logged(&self, state: &State, what: &str) {
        if let Err(err) = write_atomically(&self.file, state) {
            tracing::error!(file = %self.file.display(), %err, "[store] {what} failed");
        }
    }
}

/// Writes the state as `JSON.stringify(state, null, 2)` to a temporary file
/// beside state.json, then renames it over state.json.
fn write_atomically(file: &Path, state: &State) -> io::Result<()> {
    let text = json::to_string_pretty(state).map_err(io::Error::other)?;
    let mut tmp = OsString::from(file);
    tmp.push(format!(".{}.tmp", std::process::id()));
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&tmp, text)?;
    fs::rename(&tmp, file)
}

fn load(file: &Path) -> State {
    let Ok(bytes) = fs::read(file) else {
        return State::default();
    };
    let raw: Value = match json::from_str(&String::from_utf8_lossy(&bytes)) {
        Ok(raw) => raw,
        Err(err) => {
            tracing::warn!(file = %file.display(), %err, "[store] state.json isn't JSON; starting empty");
            return State::default();
        }
    };
    match State::from_json(raw) {
        Ok(state) => state,
        Err(err) => {
            // The TypeScript never gets here, as it keeps whatever it read.
            // Starting empty would write over the file on the next save, so
            // it's moved aside rather than lost.
            let aside = set_aside(file);
            tracing::error!(file = %file.display(), aside = ?aside, %err, "[store] can't read state.json; starting empty");
            State::default()
        }
    }
}

/// Renames a state.json this version can't read to state.json.unreadable-<ms>.
fn set_aside(file: &Path) -> io::Result<PathBuf> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut aside = OsString::from(file);
    aside.push(format!(".unreadable-{ms}"));
    let aside = PathBuf::from(aside);
    fs::rename(file, &aside)?;
    Ok(aside)
}

/// Brings the projects, boards and notes over from the old app on the first
/// launch under the new name. The voice key stays behind: the old app's
/// Keychain entry encrypted it, and this app can't read that entry.
fn migrate_old_state(user_data: &Path, file: &Path) {
    if file.exists() {
        return;
    }
    let Some(parent) = user_data.parent() else {
        return;
    };
    let Ok(bytes) = fs::read(parent.join(OLD_APP_NAME).join("state.json")) else {
        // Nothing to bring over.
        return;
    };
    let Ok(mut old) = json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) else {
        return;
    };
    match &mut old {
        // The TypeScript's `delete old.voiceKey` throws on null, and nothing is brought over.
        Value::Null => return,
        Value::Object(old) => {
            old.shift_remove("voiceKey");
        }
        _ => {}
    }
    let Ok(text) = json::to_string_pretty(&old) else {
        return;
    };
    let _ = fs::create_dir_all(user_data).and_then(|()| fs::write(file, text));
}

/// Locks a mutex, carrying on after a panic in another thread that held it,
/// as the TypeScript carries on after a handler throws.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::FolderColor;
    use tempfile::TempDir;

    /// An app data folder holding the old app's state.json, and where ours goes.
    fn old_app_beside(old_text: &str) -> (TempDir, PathBuf, PathBuf) {
        let root = TempDir::new().unwrap();
        let old_file = root.path().join(OLD_APP_NAME).join("state.json");
        fs::create_dir(old_file.parent().unwrap()).unwrap();
        fs::write(&old_file, old_text).unwrap();
        let user_data = root.path().join("Nodus");
        (root, old_file, user_data)
    }

    #[test]
    fn brings_the_old_apps_state_over() {
        let old_text = concat!(
            r#"{"projects":[{"id":"p","name":"shop","repoPath":"/Users/example/Code/shop","color":"red","createdAt":1}],"#,
            r#""voiceKey":"djEwAAAA","boards":{}}"#
        );
        let (_root, old_file, user_data) = old_app_beside(old_text);
        let store = Store::open(&user_data).unwrap();
        store.read(|state| {
            assert_eq!(state.projects[0].color, FolderColor::Red);
            // The old app's Keychain entry encrypted its key.
            assert_eq!(state.voice_key, None);
        });
        assert_eq!(
            fs::read_to_string(user_data.join("state.json")).unwrap(),
            concat!(
                "{\n  \"projects\": [\n    {\n      \"id\": \"p\",\n      \"name\": \"shop\",\n",
                "      \"repoPath\": \"/Users/example/Code/shop\",\n      \"color\": \"red\",\n",
                "      \"createdAt\": 1\n    }\n  ],\n  \"boards\": {}\n}"
            )
        );
        assert_eq!(fs::read_to_string(old_file).unwrap(), old_text);
    }

    #[test]
    fn leaves_the_old_app_alone_once_it_has_state() {
        let (_root, _, user_data) =
            old_app_beside(r#"{"projects":[{"id":"old","name":"old","repoPath":"/o","color":"red","createdAt":1}]}"#);
        fs::create_dir(&user_data).unwrap();
        fs::write(user_data.join("state.json"), r#"{"projects":[]}"#).unwrap();
        let store = Store::open(&user_data).unwrap();
        store.read(|state| assert!(state.projects.is_empty()));
    }

    #[test]
    fn brings_nothing_over_from_a_null_state() {
        let (_root, _, user_data) = old_app_beside("null");
        Store::open(&user_data).unwrap();
        assert!(!user_data.join("state.json").exists());
    }
}
