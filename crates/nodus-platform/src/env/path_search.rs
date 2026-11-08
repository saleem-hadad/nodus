//! Looking programs up on the PATH, and the folders Claude Code's installers
//! use, for a PATH that doesn't have them yet.

use std::fs;
use std::path::{Path, PathBuf};

use super::EnvMap;

/// What separates the PATH's folders.
pub(super) const DELIMITER: char = if cfg!(windows) { ';' } else { ':' };

/// The first of `bins` found on the PATH; each name is looked for in every
/// folder before the next.
pub(super) fn find_on_path(bins: &[&str], path_var: &str) -> Option<PathBuf> {
    bins.iter().find_map(|bin| {
        path_var
            .split(DELIMITER)
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir).join(bin))
            .find(|candidate| is_executable_file(candidate))
    })
}

/// Adds the installers' folders to the end of the PATH, those it lacks.
pub(super) fn add_fallback_dirs(env: &mut EnvMap, home: Option<&Path>) {
    let mut parts: Vec<String> = env
        .get("PATH")
        .map(|path| {
            path.split(DELIMITER)
                .filter(|dir| !dir.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    for dir in fallback_dirs(env, home) {
        let dir = dir.to_string_lossy().into_owned();
        if !parts.contains(&dir) {
            parts.push(dir);
        }
    }
    env.insert("PATH".into(), parts.join(&DELIMITER.to_string()));
}

/// Where Claude Code's installers put it.
fn fallback_dirs(env: &EnvMap, home: Option<&Path>) -> Vec<PathBuf> {
    let set = |name: &str| env.get(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        // The native installer, WinGet and npm.
        let mut dirs: Vec<PathBuf> = home.map(|home| home.join(".local").join("bin")).into_iter().collect();
        dirs.extend(set("LOCALAPPDATA").map(|local| local.join("Microsoft").join("WinGet").join("Links")));
        dirs.extend(set("APPDATA").map(|roaming| roaming.join("npm")));
        return dirs;
    }
    let mut dirs: Vec<PathBuf> = home
        .map(|home| vec![home.join(".local/bin"), home.join(".claude/local")])
        .unwrap_or_default();
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from),
    );
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file()) && may_execute(path)
}

#[cfg(unix)]
fn may_execute(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}

/// Windows has no execute permission to ask about: Node's X_OK there only asks
/// that the file exists.
#[cfg(not(unix))]
fn may_execute(_: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn path_of(dirs: &[&Path]) -> String {
        dirs.iter()
            .map(|dir| dir.to_string_lossy())
            .collect::<Vec<_>>()
            .join(&DELIMITER.to_string())
    }

    fn program(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn looks_for_each_name_in_every_folder_before_the_next() {
        let (first, second) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        program(first.path(), "claude.cmd");
        let exe = program(second.path(), "claude.exe");
        let path = path_of(&[first.path(), second.path()]);
        assert_eq!(find_on_path(&["claude.exe", "claude.cmd"], &path), Some(exe));
    }

    #[test]
    fn skips_empty_folders_and_folders_without_it() {
        let (empty, with) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let claude = program(with.path(), "claude");
        let path = format!(
            "{DELIMITER}{}{DELIMITER}{DELIMITER}{}",
            empty.path().display(),
            with.path().display()
        );
        assert_eq!(find_on_path(&["claude"], &path), Some(claude));
        assert_eq!(find_on_path(&["missing"], &path), None);
        assert_eq!(find_on_path(&["claude"], ""), None);
    }

    #[test]
    fn takes_only_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("claude")).unwrap();
        assert_eq!(find_on_path(&["claude"], &path_of(&[dir.path()])), None);
    }

    #[cfg(unix)]
    #[test]
    fn takes_only_files_it_may_run() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("claude"), "not a program").unwrap();
        assert_eq!(find_on_path(&["claude"], &path_of(&[dir.path()])), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn adds_the_installers_folders_it_lacks() {
        let home = Path::new("/Users/example");
        let mut env = EnvMap::from([("PATH".to_owned(), "/custom/bin::/usr/bin:".to_owned())]);
        add_fallback_dirs(&mut env, Some(home));
        assert_eq!(
            env["PATH"],
            "/custom/bin:/usr/bin:/Users/example/.local/bin:/Users/example/.claude/local:/opt/homebrew/bin:\
             /usr/local/bin:/bin:/usr/sbin:/sbin"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn makes_a_path_when_there_is_none() {
        let mut env = EnvMap::new();
        add_fallback_dirs(&mut env, None);
        assert_eq!(
            env["PATH"],
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
    }

    #[cfg(windows)]
    #[test]
    fn adds_the_installers_folders_it_lacks() {
        let home = Path::new(r"C:\Users\example");
        let mut env = EnvMap::from([
            (
                "PATH".to_owned(),
                r"C:\Windows;;C:\Users\example\AppData\Roaming\npm".to_owned(),
            ),
            ("LOCALAPPDATA".to_owned(), r"C:\Users\example\AppData\Local".to_owned()),
            ("APPDATA".to_owned(), r"C:\Users\example\AppData\Roaming".to_owned()),
        ]);
        add_fallback_dirs(&mut env, Some(home));
        assert_eq!(
            env["PATH"],
            r"C:\Windows;C:\Users\example\AppData\Roaming\npm;C:\Users\example\.local\bin;C:\Users\example\AppData\Local\Microsoft\WinGet\Links"
        );
    }
}
