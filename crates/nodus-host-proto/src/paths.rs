//! Where the host's files live, and keeping them the user's own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

/// The platforms whose host files differ: a Unix socket in the host folder,
/// or a named pipe and a key on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS (and any other Unix).
    Posix,
    Windows,
}

impl Platform {
    /// The platform this was built for.
    pub const fn current() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Posix }
    }

    fn separator(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }
}

/// Every file the host and the app share, in `<userData>/host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    /// Private to the user, like the socket inside it: whoever can connect can run commands.
    pub dir: PathBuf,
    /// The socket's path, or the pipe's name on Windows.
    pub socket: PathBuf,
    pub log: PathBuf,
    /// The curl config that gives hook commands the token ([`write_hook_config`]).
    pub hook_config: PathBuf,
    /// On Windows, the host's key, which its welcome has to carry ([`trusts_host`]).
    pub key: PathBuf,
}

/// The host's files for a userData folder (`hostPaths(userData, platform)`).
///
/// The socket's name hashes the host folder's path exactly as Node's
/// `path.join` writes it, so the Rust app and a 0.9.4 app find each other's
/// host. (HOST-001–006)
pub fn host_paths(user_data: &Path, platform: Platform) -> HostPaths {
    host_paths_in(user_data, platform, &node_tmpdir())
}

fn host_paths_in(user_data: &Path, platform: Platform, tmpdir: &str) -> HostPaths {
    let dir = node_join(platform, &user_data.to_string_lossy(), "host");
    let hash = || {
        let digest = Sha1::digest(dir.as_bytes());
        digest
            .iter()
            .take(6)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let in_dir = |name: &str| format!("{dir}{}{name}", platform.separator());
    let mut socket = in_dir("host.sock");
    match platform {
        // Windows has no sockets in folders. A pipe's default security lets
        // only its owner (and admins) write to it, so other users can't say
        // hello. But anyone can make a pipe of this name first, so the host
        // proves itself with its key.
        Platform::Windows => socket = format!(r"\\.\pipe\nodus-{}", hash()),
        // macOS caps socket paths at 104 bytes; the per-user temp folder is private too.
        Platform::Posix if socket.len() > 100 => {
            socket = node_join(platform, tmpdir, &format!("nodus-{}.sock", hash()))
        }
        Platform::Posix => {}
    }
    HostPaths {
        socket: socket.into(),
        log: in_dir("host.log").into(),
        hook_config: in_dir("hook.curlrc").into(),
        key: in_dir("host.key").into(),
        dir: dir.into(),
    }
}

/// Node's `os.tmpdir()` on a Unix: `$TMPDIR` (or `$TMP`, `$TEMP`, `/tmp`)
/// without a trailing slash. On Windows the host never uses it.
fn node_tmpdir() -> String {
    let dir = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "/tmp".into());
    match dir.strip_suffix('/') {
        Some(trimmed) if dir.len() > 1 => trimmed.to_owned(),
        _ => dir,
    }
}

/// Node's `path.join(base, name)` for the platform: the two joined by a
/// separator, then normalised (`.` and `..` resolved, repeated separators
/// collapsed, `/` turned into `\` on Windows). `name` is a plain file name.
fn node_join(platform: Platform, base: &str, name: &str) -> String {
    let joined = if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    };
    match platform {
        Platform::Posix => {
            let absolute = joined.starts_with('/');
            let body = normalize_segments(joined.split('/'), absolute);
            let body = body.join("/");
            match (absolute, body.is_empty()) {
                (true, _) => format!("/{body}"),
                (false, true) => ".".into(),
                (false, false) => body,
            }
        }
        Platform::Windows => {
            let joined = joined.replace('/', "\\");
            let (root, rest) = windows_root(&joined);
            let absolute = rest.starts_with('\\');
            let body = normalize_segments(rest.split('\\'), absolute || root.starts_with("\\\\")).join("\\");
            let sep = if absolute { "\\" } else { "" };
            if body.is_empty() && root.is_empty() && !absolute {
                ".".into()
            } else {
                format!("{root}{sep}{body}")
            }
        }
    }
}

/// The device of a Windows path (`C:` or `\\server\share`), and the rest.
fn windows_root(path: &str) -> (String, &str) {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return (path[..2].to_owned(), &path[2..]);
    }
    if let Some(unc) = path.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        if let (Some(server), Some(share)) = (parts.next(), parts.next())
            && !server.is_empty()
            && !share.is_empty()
        {
            let rest = parts.next().map_or("", |rest| &path[path.len() - rest.len() - 1..]);
            return (format!("\\\\{server}\\{share}"), rest);
        }
    }
    (String::new(), path)
}

fn normalize_segments<'a>(segments: impl Iterator<Item = &'a str>, rooted: bool) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for segment in segments {
        match segment {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|last| *last != "..") {
                    out.pop();
                } else if !rooted {
                    out.push("..");
                }
            }
            _ => out.push(segment),
        }
    }
    out
}

/// Makes a folder only the user can open, and its missing parents, even when
/// it was there already with another mode (`ensurePrivateDir`). (HOST-007)
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        // Windows has no mode bits; the folder's ACL is the user's.
        fs::create_dir_all(dir)
    }
}

/// Writes a file in a private folder that only the user can read, beside it
/// and renamed into place (`writePrivateFile`). (HOST-008)
pub fn write_private_file(file: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = file.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        ensure_private_dir(parent)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use io::Write;
        let mut out = options.open(&tmp)?;
        out.write_all(text.as_bytes())?;
    }
    fs::rename(&tmp, file)
}

/// Why the hook config wasn't written.
#[derive(Debug, thiserror::Error)]
pub enum HookConfigError {
    #[error("The session host’s token can’t go in a curl config")]
    UnsafeToken,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Writes the curl config hook commands read the host's token from. Only the
/// user can read it. The token goes inside quotes in a config file, where a
/// quote or a new line would add options of its own, so only plain ones do
/// (`writeHookConfig`). (HOST-009, HOST-010)
pub fn write_hook_config(paths: &HostPaths, token: &str) -> Result<(), HookConfigError> {
    // /^[\w-]+$/, with JavaScript's ASCII-only \w.
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(HookConfigError::UnsafeToken);
    }
    write_private_file(
        &paths.hook_config,
        &format!("header = \"Authorization: Bearer {token}\"\n"),
    )?;
    Ok(())
}

/// Whether a host's welcome shows it's one this app's user started. On
/// Windows it has to carry the key in the host folder, which only the user can
/// read: anyone on the PC can make a pipe with the host's name before the host
/// does, and would then be handed every session. A Mac's socket sits in a
/// folder only the user can open, so it needs no key (`trustsHost`).
/// (HOST-011, HOST-012)
pub fn trusts_host(key: Option<&serde_json::Value>, paths: &HostPaths, platform: Platform) -> bool {
    if platform != Platform::Windows {
        return true;
    }
    let Some(key) = key.and_then(serde_json::Value::as_str).filter(|key| !key.is_empty()) else {
        return false;
    };
    // readFileSync(…, 'utf8'): bytes that aren't UTF-8 read as U+FFFD.
    fs::read(&paths.key).is_ok_and(|bytes| String::from_utf8_lossy(&bytes) == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sha1_12(text: &str) -> String {
        Sha1::digest(text.as_bytes())
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("nodus-host-").tempdir().unwrap()
    }

    // hostProtocol.test.ts › host files

    #[test]
    fn keeps_the_socket_in_the_host_folder_when_the_path_is_short_enough() {
        let dir = Path::new("/Users/example/Library/Application Support/Nodus");
        let paths = host_paths_in(dir, Platform::Posix, "/tmp");
        let host = "/Users/example/Library/Application Support/Nodus/host";
        assert_eq!(
            paths,
            HostPaths {
                dir: host.into(),
                socket: format!("{host}/host.sock").into(),
                log: format!("{host}/host.log").into(),
                hook_config: format!("{host}/hook.curlrc").into(),
                key: format!("{host}/host.key").into(),
            }
        );
        // HOST-002 vector (Data & formats §2).
        assert_eq!(sha1_12(host), "dc3de46669b8");
    }

    #[test]
    fn moves_a_socket_with_a_long_path_to_the_temp_folder_under_a_name_from_the_folder() {
        let user_data = format!("/private/var/folders/xy/T/nodus-host-abc/{}", "x".repeat(120));
        let host_dir = format!("{user_data}/host");
        let paths = host_paths_in(Path::new(&user_data), Platform::Posix, "/var/folders/xy/T");
        assert_eq!(
            paths.socket,
            PathBuf::from(format!("/var/folders/xy/T/nodus-{}.sock", sha1_12(&host_dir)))
        );
        assert_eq!(paths.hook_config, PathBuf::from(format!("{host_dir}/hook.curlrc")));
    }

    #[test]
    fn keeps_a_socket_path_of_exactly_100_bytes_in_the_folder() {
        // "/" + a + "/host/host.sock" (15 bytes) = 100 bytes.
        let user_data = format!("/{}", "a".repeat(84));
        let paths = host_paths_in(Path::new(&user_data), Platform::Posix, "/tmp");
        assert_eq!(paths.socket.to_string_lossy().len(), 100);
        assert_eq!(paths.socket, PathBuf::from(format!("{user_data}/host/host.sock")));
        let longer = format!("/{}", "a".repeat(85));
        let paths = host_paths_in(Path::new(&longer), Platform::Posix, "/tmp");
        assert!(paths.socket.to_string_lossy().starts_with("/tmp/nodus-"));
    }

    #[test]
    fn names_a_pipe_on_windows_from_the_folder_too() {
        let user_data = r"C:\Users\example\AppData\Roaming\Nodus";
        let paths = host_paths_in(Path::new(user_data), Platform::Windows, "");
        assert_eq!(paths.dir, PathBuf::from(r"C:\Users\example\AppData\Roaming\Nodus\host"));
        // HOST-002 vector (Data & formats §2).
        assert_eq!(paths.socket, PathBuf::from(r"\\.\pipe\nodus-faa591a66f7f"));
        assert_eq!(
            paths.hook_config,
            PathBuf::from(r"C:\Users\example\AppData\Roaming\Nodus\host\hook.curlrc")
        );
        assert_eq!(
            paths.key,
            PathBuf::from(r"C:\Users\example\AppData\Roaming\Nodus\host\host.key")
        );
    }

    #[test]
    fn joins_the_folder_as_node_path_join_does() {
        let posix = |base: &str| node_join(Platform::Posix, base, "host");
        assert_eq!(posix("/tmp/x/"), "/tmp/x/host");
        assert_eq!(posix("/tmp//x"), "/tmp/x/host");
        assert_eq!(posix("/tmp/./y/../x"), "/tmp/x/host");
        assert_eq!(posix("rel/dir"), "rel/dir/host");
        assert_eq!(posix("../up"), "../up/host");
        let windows = |base: &str| node_join(Platform::Windows, base, "host");
        assert_eq!(windows(r"C:\Users\example\Nodus\"), r"C:\Users\example\Nodus\host");
        assert_eq!(windows("C:/Users/example/Nodus"), r"C:\Users\example\Nodus\host");
        assert_eq!(windows(r"C:\a\..\b"), r"C:\b\host");
        assert_eq!(windows(r"\\server\share\Nodus"), r"\\server\share\Nodus\host");
    }

    #[cfg(unix)]
    #[test]
    fn makes_the_host_folder_private_even_one_that_exists_already() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = scratch();
        let host_dir = scratch.path().join("host");
        fs::create_dir(&host_dir).unwrap();
        fs::set_permissions(&host_dir, fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&host_dir).unwrap();
        assert_eq!(fs::metadata(&host_dir).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn writes_the_hook_config_readable_only_by_the_user() {
        let scratch = scratch();
        let paths = host_paths(scratch.path(), Platform::current());
        write_hook_config(&paths, "first").unwrap();
        write_hook_config(&paths, "second").unwrap();
        assert_eq!(
            fs::read_to_string(&paths.hook_config).unwrap(),
            "header = \"Authorization: Bearer second\"\n"
        );
        // Windows has no mode bits; the folder's ACL is the user's.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&paths.hook_config).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(fs::metadata(&paths.dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // Written beside it and renamed into place.
        let names: Vec<_> = fs::read_dir(&paths.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec!["hook.curlrc"]);
    }

    #[test]
    fn wont_put_a_token_in_the_hook_config_that_would_add_curl_options() {
        let scratch = scratch();
        let paths = host_paths(scratch.path(), Platform::current());
        write_hook_config(&paths, "abc123").unwrap();
        let bad = write_hook_config(&paths, "x\"\noutput = \"C:\\evil.cmd").unwrap_err();
        assert_eq!(bad.to_string(), "The session host’s token can’t go in a curl config");
        assert!(matches!(
            write_hook_config(&paths, ""),
            Err(HookConfigError::UnsafeToken)
        ));
        // Only ASCII counts as a word character.
        assert!(matches!(
            write_hook_config(&paths, "é"),
            Err(HookConfigError::UnsafeToken)
        ));
        assert_eq!(
            fs::read_to_string(&paths.hook_config).unwrap(),
            "header = \"Authorization: Bearer abc123\"\n"
        );
    }

    // hostProtocol.test.ts › trustsHost

    fn windows_paths(scratch: &tempfile::TempDir) -> HostPaths {
        // The real folder layout, with a pipe name: the key file is what's checked.
        let mut paths = host_paths(scratch.path(), Platform::current());
        paths.socket = r"\\.\pipe\nodus-test".into();
        paths
    }

    #[test]
    fn trusts_a_pipe_whose_welcome_carries_the_key_in_the_host_folder() {
        let scratch = scratch();
        let paths = windows_paths(&scratch);
        write_private_file(&paths.key, &"k".repeat(64)).unwrap();
        assert!(trusts_host(Some(&json!("k".repeat(64))), &paths, Platform::Windows));
    }

    #[test]
    fn doesnt_trust_one_that_made_the_pipe_first_and_cant_read_the_folder() {
        let scratch = scratch();
        let paths = windows_paths(&scratch);
        write_private_file(&paths.key, &"k".repeat(64)).unwrap();
        assert!(!trusts_host(Some(&json!("j".repeat(64))), &paths, Platform::Windows));
        assert!(!trusts_host(None, &paths, Platform::Windows));
        assert!(!trusts_host(Some(&json!("")), &paths, Platform::Windows));
        // Not a string at all.
        assert!(!trusts_host(Some(&json!(64)), &paths, Platform::Windows));
        // Exact: neither trimmed nor case-folded.
        assert!(!trusts_host(
            Some(&json!(format!("{} ", "k".repeat(64)))),
            &paths,
            Platform::Windows
        ));
        assert!(!trusts_host(Some(&json!("K".repeat(64))), &paths, Platform::Windows));
    }

    #[test]
    fn doesnt_trust_a_pipe_while_theres_no_key_file() {
        let scratch = scratch();
        let paths = windows_paths(&scratch);
        assert!(!trusts_host(Some(&json!("k".repeat(64))), &paths, Platform::Windows));
    }

    #[test]
    fn needs_no_key_on_a_mac_where_the_sockets_folder_keeps_others_out() {
        let scratch = scratch();
        assert!(trusts_host(
            None,
            &host_paths(scratch.path(), Platform::Posix),
            Platform::Posix
        ));
    }

    #[test]
    fn reads_the_temp_folder_as_node_does() {
        // Only the trailing slash goes; the default is /tmp.
        assert!(!node_tmpdir().ends_with('/') || node_tmpdir() == "/");
    }
}
