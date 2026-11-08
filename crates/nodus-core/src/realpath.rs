//! Node's `fs.realpathSync`, which a project's identity depends on (spec 01,
//! PLAT-069, PLAT-071, risk R2).
//!
//! Node's JavaScript realpath resolves the path against the working folder,
//! then walks it a component at a time, reading each symlink it meets and
//! starting over from what it points to. Unlike `std::fs::canonicalize` it
//! keeps the case the path was given in (on a case-insensitive disk) and never
//! returns a `\\?\` path on Windows, so a folder added in 0.9.4 is found again
//! by the same string.
//!
//! It lives here until the transcripts track's `realRepoPath` (spec 03) can
//! take it over.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

/// How many links a path may go through before it counts as a loop (Linux's
/// MAXSYMLINKS is 40).
const MAX_LINKS: usize = 40;

/// The real path of `path`: absolute, with every symlink in it resolved.
/// Fails when a component is missing or can't be read.
pub fn realpath(path: &Path) -> io::Result<PathBuf> {
    let mut parts = resolve(&std::env::current_dir()?, path);
    let mut links = 0;
    let mut at = 0;
    'walk: loop {
        while at < parts.normal.len() {
            let here = parts.prefix_through(at);
            let meta = std::fs::symlink_metadata(&here)?;
            if meta.file_type().is_symlink() {
                links += 1;
                if links > MAX_LINKS {
                    return Err(io::Error::other("too many levels of symbolic links"));
                }
                let target = std::fs::read_link(&here)?;
                let parent = parts.prefix_through_exclusive(at);
                let rest: Vec<OsString> = parts.normal[at + 1..].to_vec();
                let mut next = resolve(&parent, &target);
                next.normal.extend(rest);
                parts = next;
                at = 0;
                continue 'walk;
            }
            at += 1;
        }
        // The root itself has to be there (Node checks it on Windows).
        let full = parts.to_path();
        std::fs::symlink_metadata(&full)?;
        return Ok(full);
    }
}

/// An absolute path split into its root (prefix and root folder) and its
/// named components, with `.` and `..` already taken out.
#[derive(Debug, Clone)]
struct Parts {
    root: PathBuf,
    normal: Vec<OsString>,
}

impl Parts {
    fn to_path(&self) -> PathBuf {
        let mut path = self.root.clone();
        path.extend(&self.normal);
        path
    }

    /// The path up to and including component `index`.
    fn prefix_through(&self, index: usize) -> PathBuf {
        let mut path = self.root.clone();
        path.extend(&self.normal[..=index]);
        path
    }

    /// The path up to, but not including, component `index`.
    fn prefix_through_exclusive(&self, index: usize) -> PathBuf {
        let mut path = self.root.clone();
        path.extend(&self.normal[..index]);
        path
    }
}

/// `path.resolve(from, to)`: `to` against `from`, with `.` and `..` resolved
/// by name, never through the disk.
fn resolve(from: &Path, to: &Path) -> Parts {
    let joined = if to.has_root() { to.to_path_buf() } else { from.join(to) };
    let mut root = PathBuf::new();
    let mut normal: Vec<OsString> = Vec::new();
    for component in joined.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            Component::Normal(name) => normal.push(name.to_os_string()),
        }
    }
    Parts { root, normal }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn follows_links_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let real_root = realpath(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("repos/shop/src")).unwrap();
        symlink(dir.path().join("repos"), dir.path().join("link")).unwrap();
        assert_eq!(
            realpath(&dir.path().join("link/shop/./src/..")).unwrap(),
            real_root.join("repos/shop")
        );
    }

    #[test]
    fn follows_relative_links() {
        let dir = tempfile::tempdir().unwrap();
        let real_root = realpath(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        symlink("a/b", dir.path().join("short")).unwrap();
        assert_eq!(realpath(&dir.path().join("short")).unwrap(), real_root.join("a/b"));
    }

    #[test]
    fn fails_for_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(realpath(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn stops_at_a_loop() {
        let dir = tempfile::tempdir().unwrap();
        symlink(dir.path().join("b"), dir.path().join("a")).unwrap();
        symlink(dir.path().join("a"), dir.path().join("b")).unwrap();
        assert!(realpath(&dir.path().join("a")).is_err());
    }
}
