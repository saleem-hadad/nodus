//! Trusting a folder for Claude Code, as accepting its trust dialog there
//! does, so a session started in it doesn't stop at the dialog (SRV-126).
//!
//! Claude Code has no command or setting for this. Its documentation says to
//! trust a folder by hand by setting `projects["<path>"].hasTrustDialogAccepted`
//! to `true` in `~/.claude.json` (`$CLAUDE_CONFIG_DIR/.claude.json` when that's
//! set), where the path is the repository's root, or the main checkout's root
//! for a worktree, or the folder itself outside a repository. The host does
//! that for its own user, whose file it is: read, change that one key, and
//! write the file back whole, everything else kept as it was, renamed into
//! place so Claude Code never reads half of it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

/// One trust at a time, so two don't each write the file without the other's.
static WRITING: Mutex<()> = Mutex::new(());

/// The host user's `.claude.json`: in `$CLAUDE_CONFIG_DIR` when that's set,
/// else in their home folder. None without either.
pub fn claude_config() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir).join(".claude.json"));
    }
    std::env::home_dir().map(|home| home.join(".claude.json"))
}

/// Marks `folder` trusted in `config` (SRV-126). A missing file is made;
/// one that isn't a JSON object, or whose entry for the folder isn't one, is
/// left alone and the trust fails. Nothing is written when it's trusted
/// already.
pub fn trust_folder(config: &Path, folder: &str) -> io::Result<()> {
    let _writing = WRITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("{} {what}", config.display()));
    let (mut root, mode) = match fs::read_to_string(config) {
        // Strictly: anything JSON.parse would read differently is left alone.
        Ok(text) if !text.trim().is_empty() => match serde_json::from_str::<Value>(&text) {
            Ok(root) => (root, file_mode(config)),
            Err(_) => return Err(bad("isn't JSON")),
        },
        Ok(_) => (Value::Object(Map::new()), file_mode(config)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => (Value::Object(Map::new()), None),
        Err(err) => return Err(err),
    };
    let Some(root_fields) = root.as_object_mut() else {
        return Err(bad("isn't a JSON object"));
    };
    let projects = root_fields
        .entry("projects")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(projects) = projects.as_object_mut() else {
        return Err(bad("has projects that aren't an object"));
    };
    let project = projects.entry(folder).or_insert_with(|| Value::Object(Map::new()));
    let Some(project) = project.as_object_mut() else {
        return Err(bad("has a project entry that isn't an object"));
    };
    if project.get("hasTrustDialogAccepted") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    project.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
    // As Claude Code writes it: JSON.stringify(config, null, 2).
    let text = nodus_domain::json::to_string_pretty(&root).map_err(io::Error::other)?;
    write_replacing(config, &text, mode)
}

/// The file's permission bits, to keep on the new one.
#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|meta| meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

/// Writes `text` beside `file` and renames it over it, with `mode` (0600 for
/// a new file, which holds the user's account details).
fn write_replacing(file: &Path, text: &str, mode: Option<u32>) -> io::Result<()> {
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".nodus-{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode.unwrap_or(0o600));
    }
    #[cfg(not(unix))]
    let _ = mode;
    let written = (|| {
        use io::Write;
        let mut out = options.open(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            // The umask can take bits off a mode set at open.
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        }
        fs::rename(&tmp, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn trusts_a_folder_keeping_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".claude.json");
        let before = r#"{
  "numStartups": 12,
  "cost": 0.1,
  "big": 1e+21,
  "projects": {
    "/srv/other": {
      "allowedTools": [],
      "hasTrustDialogAccepted": false
    },
    "/srv/repo": {
      "lastSessionId": "s",
      "history": []
    }
  },
  "z": "last"
}"#;
        fs::write(&config, before).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config, fs::Permissions::from_mode(0o640)).unwrap();
        }
        trust_folder(&config, "/srv/repo").unwrap();
        let after = fs::read_to_string(&config).unwrap();
        assert_eq!(
            after,
            before.replace(
                "      \"history\": []\n",
                "      \"history\": [],\n      \"hasTrustDialogAccepted\": true\n"
            )
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o640);
        }

        // A folder it doesn't know yet, and one it had said no to.
        trust_folder(&config, "/srv/new").unwrap();
        trust_folder(&config, "/srv/other").unwrap();
        let root = read(&config);
        assert_eq!(
            root["projects"]["/srv/new"],
            serde_json::json!({"hasTrustDialogAccepted": true})
        );
        assert_eq!(root["projects"]["/srv/other"]["hasTrustDialogAccepted"], true);
        assert_eq!(root["projects"]["/srv/other"]["allowedTools"], serde_json::json!([]));
        // Nothing but the file is left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn makes_the_file_when_there_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".claude.json");
        trust_folder(&config, "/srv/repo").unwrap();
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "{\n  \"projects\": {\n    \"/srv/repo\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  }\n}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn leaves_a_file_it_cannot_read_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".claude.json");
        for text in [
            "{ not json",
            "[1, 2]",
            r#"{"projects": 5}"#,
            r#"{"projects": {"/srv/repo": true}}"#,
        ] {
            fs::write(&config, text).unwrap();
            assert!(trust_folder(&config, "/srv/repo").is_err(), "{text}");
            assert_eq!(fs::read_to_string(&config).unwrap(), text);
        }
    }
}
