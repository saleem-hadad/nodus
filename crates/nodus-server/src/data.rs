//! Where a server keeps its data (SRV-003), and the environment its
//! sessions and commands start in (SRV-141).

use std::path::{Path, PathBuf};

use nodus_platform::env::EnvMap;

/// The PATH when `<data>/env` gives none.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// SRV-003: `NODUS_USER_DATA`, else `$XDG_CONFIG_HOME/Nodus`, else
/// `~/.config/Nodus`. On Linux that's the store's own default; on a Mac,
/// where the server runs for development, it keeps clear of the app's
/// folder, so the two never share a state.json.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(dir) = var("NODUS_USER_DATA") {
        return Some(PathBuf::from(dir));
    }
    let config = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
    Some(config.join("Nodus"))
}

/// The environment sessions, terminals and commands start in, and the home
/// folder whose `.claude` holds their transcripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerEnv {
    pub vars: EnvMap,
    pub home: PathBuf,
}

/// SRV-141: `<data>/env`, and nothing else of the service's environment,
/// with a `PATH` and `HOME`. Claude Code's own folders go on the PATH as
/// they do on a Mac. No file is an empty one; a line that isn't `KEY=VALUE`
/// is an error, so a typo there doesn't quietly start sessions without what
/// it meant to set.
///
/// `run_home` is where sessions live and claude keeps its transcripts:
/// `/home/nodus-run` on a server whose sessions run as `nodus-run` (SRV-003,
/// SRV-005), whatever the file says. Without it, the file's `HOME`, else
/// this user's, as with one user in development and tests.
pub(crate) fn server_env(file: &Path, run_home: Option<&Path>) -> Result<ServerEnv, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("Can't read {}: {err}", file.display())),
    };
    let mut vars = parse_env(&text).map_err(|err| format!("{}: {err}", file.display()))?;
    let home = match (run_home, vars.get("HOME").filter(|home| !home.is_empty())) {
        (Some(home), _) => home.to_path_buf(),
        (None, Some(home)) => PathBuf::from(home),
        (None, None) => std::env::home_dir().ok_or("No home folder: give --run-home")?,
    };
    vars.insert("HOME".into(), home.to_string_lossy().into_owned());
    if vars.get("PATH").is_none_or(String::is_empty) {
        vars.insert("PATH".into(), DEFAULT_PATH.into());
    }
    Ok(ServerEnv { vars, home })
}

/// Holds `<data>/serve.lock` for as long as it's kept, so a second `serve`
/// can't share the data folder: two cores writing one state.json would
/// each undo what the other saved.
pub(crate) fn lock(data: &Path) -> Result<std::fs::File, String> {
    let path = data.join("serve.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| format!("Can't open {}: {err}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(format!("Another nodus-server serve is using {}", data.display()))
        }
        Err(std::fs::TryLockError::Error(err)) => Err(format!("Can't lock {}: {err}", path.display())),
    }
}

/// `KEY=VALUE` lines, as systemd's `EnvironmentFile=` reads them: blank
/// lines and `#` comments skipped, an `export ` before the key allowed, and
/// a value in matching quotes unquoted.
fn parse_env(text: &str) -> Result<EnvMap, String> {
    let mut vars = EnvMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: not KEY=VALUE", number + 1));
        };
        let key = key.trim();
        let named = key
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
        if key.is_empty() || !named {
            return Err(format!("line {}: {key:?} isn't a variable's name", number + 1));
        }
        let value = value.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|&q| value.strip_prefix(q).and_then(|v| v.strip_suffix(q)))
            .filter(|_| value.len() >= 2);
        vars.insert(key.to_owned(), unquoted.unwrap_or(value).to_owned());
    }
    Ok(vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_value_lines() {
        let vars = parse_env(
            "# for the sessions\n\nPATH=/home/nodus-run/.local/bin:/usr/bin\nexport LANG=C.UTF-8\n  DOCKER_HOST = \"unix:///run/user/1001/docker.sock\"\nEMPTY=\nQUOTE='a=b'\n",
        )
        .unwrap();
        assert_eq!(vars["PATH"], "/home/nodus-run/.local/bin:/usr/bin");
        assert_eq!(vars["LANG"], "C.UTF-8");
        assert_eq!(vars["DOCKER_HOST"], "unix:///run/user/1001/docker.sock");
        assert_eq!(vars["EMPTY"], "");
        assert_eq!(vars["QUOTE"], "a=b");
        assert_eq!(vars.len(), 5);
    }

    #[test]
    fn turns_down_lines_that_arent_variables() {
        assert_eq!(parse_env("A=1\nnonsense\n").unwrap_err(), "line 2: not KEY=VALUE");
        assert!(parse_env("1A=x").is_err());
        assert!(parse_env("=x").is_err());
        assert!(parse_env("A B=x").is_err());
    }

    #[test]
    fn gives_sessions_a_home_and_a_path_and_nothing_else_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        let none = server_env(&file, None).unwrap();
        assert_eq!(none.vars["PATH"], DEFAULT_PATH);
        assert_eq!(none.home, std::env::home_dir().unwrap());
        assert_eq!(none.vars.len(), 2);

        std::fs::write(&file, "HOME=/home/dev\nPATH=/opt/nodus/bin/shims:/usr/bin\n").unwrap();
        let given = server_env(&file, None).unwrap();
        assert_eq!(given.home, Path::new("/home/dev"));
        assert_eq!(given.vars["PATH"], "/opt/nodus/bin/shims:/usr/bin");

        // The sessions' own user's home, over anything the file says.
        let run = server_env(&file, Some(Path::new("/home/nodus-run"))).unwrap();
        assert_eq!(run.home, Path::new("/home/nodus-run"));
        assert_eq!(run.vars["HOME"], "/home/nodus-run");
    }

    #[test]
    fn lets_one_server_use_a_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock(dir.path()).unwrap();
        let err = lock(dir.path()).unwrap_err();
        assert!(err.starts_with("Another nodus-server serve is using"), "{err}");
        drop(held);
        lock(dir.path()).unwrap();
    }
}
