//! Error messages as Node words them, for the errors the user can see: a
//! transcript that can't be opened, a `claude` that can't be started.

use std::io;
use std::path::Path;

/// libuv's code and description for an error, where it has one.
fn uv_error(err: &io::Error) -> Option<(&'static str, &'static str)> {
    #[cfg(windows)]
    if let Some(code) = err.raw_os_error() {
        // libuv's mapping of the Windows error codes.
        match code {
            2 | 3 => return Some(("ENOENT", "no such file or directory")),
            5 => return Some(("EPERM", "operation not permitted")),
            32 | 33 => return Some(("EBUSY", "resource busy or locked")),
            _ => {}
        }
    }
    #[cfg(unix)]
    if err.raw_os_error() == Some(1) {
        return Some(("EPERM", "operation not permitted"));
    }
    Some(match err.kind() {
        io::ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
        io::ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
        io::ErrorKind::IsADirectory => ("EISDIR", "illegal operation on a directory"),
        io::ErrorKind::NotADirectory => ("ENOTDIR", "not a directory"),
        io::ErrorKind::InvalidFilename => ("ENAMETOOLONG", "name too long"),
        _ => return None,
    })
}

/// `EACCES: permission denied, open '<path>'`.
pub(crate) fn fs_message(err: &io::Error, syscall: &str, path: &Path) -> String {
    match uv_error(err) {
        Some((code, description)) => format!("{code}: {description}, {syscall} '{}'", path.display()),
        None => err.to_string(),
    }
}

/// `spawn <file> ENOENT`.
pub(crate) fn spawn_message(err: &io::Error, file: &Path) -> String {
    match uv_error(err) {
        Some((code, _)) => format!("spawn {} {code}", file.display()),
        None => format!("spawn {} {err}", file.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_errors_as_node_does() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(
            fs_message(&missing, "open", Path::new("/x/t.jsonl")),
            "ENOENT: no such file or directory, open '/x/t.jsonl'"
        );
        assert_eq!(
            spawn_message(&missing, Path::new("/bin/claude")),
            "spawn /bin/claude ENOENT"
        );
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs_message(&denied, "open", Path::new("/x")),
            "EACCES: permission denied, open '/x'"
        );
    }
}
