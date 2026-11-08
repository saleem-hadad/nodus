//! Finding Claude Code's program, and asking it its version.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::EnvMap;
use super::path_search::find_on_path;

/// How long `claude --version` gets to answer.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Claude Code's own program, preferring the native installer's claude.exe
/// over an npm shim.
pub(super) fn find(path_var: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return find_on_path(&["claude"], path_var);
    }
    let found = find_on_path(&["claude.exe", "claude.cmd"], path_var)?;
    let is_shim = found.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"));
    if !is_shim {
        return Some(found);
    }
    let text = fs::read(&found).ok()?;
    let exe = PathBuf::from(exe_behind_shim(
        &found.to_string_lossy(),
        &String::from_utf8_lossy(&text),
    )?);
    exe.exists().then_some(exe)
}

/// What `claude --version` prints first; None when it fails or takes too long.
pub(super) async fn read_version(claude: &Path, env: &EnvMap) -> Option<String> {
    let mut command = Command::new(claude);
    command
        .arg("--version")
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    hide_window(&mut command);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.trim().split('\n').next()?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    (!line.is_empty()).then(|| line.to_owned())
}

/// Starts it without a console window flashing up.
#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

/// The claude.exe an npm shim (claude.cmd) starts. A batch file can't take the
/// JSON options sessions are started with, as cmd.exe reads its quotes and
/// ampersands, so sessions run the program it points at instead.
pub fn exe_behind_shim(shim: &str, text: &str) -> Option<String> {
    let target = shim_target(text)?;
    // Not node.exe: a shim that runs a script has no program of its own to start.
    let lower = target.to_ascii_lowercase();
    if lower == "node.exe" || lower.ends_with("\\node.exe") {
        return None;
    }
    Some(win32::join(win32::dirname(shim), target))
}

/// The program in a `"%dp0%\…\name.exe"` (or `"%~dp0\…"`), found as the
/// TypeScript's `/"%~?dp0%?\\([^"]+?\.exe)"/i` finds it.
fn shim_target(text: &str) -> Option<&str> {
    text.match_indices('"').find_map(|(quote, _)| {
        let rest = text[quote + 1..].strip_prefix('%')?;
        let rest = rest.strip_prefix('~').unwrap_or(rest);
        let rest = rest
            .get(..3)
            .filter(|dp0| dp0.eq_ignore_ascii_case("dp0"))
            .map(|_| &rest[3..])?;
        let rest = rest.strip_prefix('%').unwrap_or(rest);
        let rest = rest.strip_prefix('\\')?;
        let target = &rest[..rest.find('"')?];
        let is_exe = target.len() > 4
            && target
                .get(target.len() - 4..)
                .is_some_and(|ext| ext.eq_ignore_ascii_case(".exe"));
        is_exe.then_some(target)
    })
}

/// Enough of Node's path.win32 to put a shim's program beside it, on any OS.
mod win32 {
    fn is_sep(c: char) -> bool {
        c == '\\' || c == '/'
    }

    /// How long the root is: `C:\`, `C:`, `\`, or `\\server\share\`.
    fn root_len(path: &str) -> usize {
        let bytes = path.as_bytes();
        let sep = |i: usize| bytes.get(i).is_some_and(|b| *b == b'\\' || *b == b'/');
        let name_end = |from: usize| (from..bytes.len()).find(|&i| sep(i)).unwrap_or(bytes.len());
        if sep(0) && sep(1) && !sep(2) && bytes.len() > 2 {
            let server_end = name_end(2);
            if sep(server_end) && server_end + 1 < bytes.len() && !sep(server_end + 1) {
                let share_end = name_end(server_end + 1);
                return if sep(share_end) { share_end + 1 } else { share_end };
            }
        }
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return if sep(2) { 3 } else { 2 };
        }
        usize::from(sep(0))
    }

    /// path.win32.dirname.
    pub(super) fn dirname(path: &str) -> &str {
        let root = root_len(path);
        let rest = path[root..].trim_end_matches(is_sep);
        let Some(last) = rest.rfind(is_sep) else {
            return if root > 0 { &path[..root] } else { "." };
        };
        let dir = rest[..last].trim_end_matches(is_sep);
        if dir.is_empty() && root > 0 {
            &path[..root]
        } else {
            &path[..root + dir.len()]
        }
    }

    /// path.win32.join of a folder and a relative path, normalized.
    pub(super) fn join(dir: &str, relative: &str) -> String {
        normalize(&format!("{dir}\\{relative}"))
    }

    /// path.win32.normalize: backslashes, no `.` or empty parts, and `..` taken out where it can be.
    fn normalize(path: &str) -> String {
        let root_len = root_len(path);
        let root = path[..root_len].replace('/', "\\");
        let absolute = root.ends_with('\\');
        let mut parts: Vec<&str> = Vec::new();
        for part in path[root_len..].split(is_sep) {
            match part {
                "" | "." => {}
                ".." if parts.last().is_some_and(|last| *last != "..") => {
                    parts.pop();
                }
                ".." if absolute => {}
                _ => parts.push(part),
            }
        }
        let mut normalized = root + &parts.join("\\");
        if normalized.is_empty() {
            return ".".into();
        }
        if path.ends_with(is_sep) && !parts.is_empty() {
            normalized.push('\\');
        }
        normalized
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dirname_as_node_does() {
            assert_eq!(dirname(r"C:\Users\me\npm\claude.cmd"), r"C:\Users\me\npm");
            assert_eq!(dirname(r"C:\claude.cmd"), r"C:\");
            assert_eq!(dirname("C:/Users/example/npm/claude.cmd"), "C:/Users/example/npm");
            assert_eq!(dirname(r"\\server\share\npm\claude.cmd"), r"\\server\share\npm");
            assert_eq!(dirname(r"\\server\share\claude.cmd"), r"\\server\share\");
            assert_eq!(dirname(r"npm\claude.cmd"), "npm");
            assert_eq!(dirname("claude.cmd"), ".");
        }

        #[test]
        fn joins_and_normalizes_as_node_does() {
            assert_eq!(join(r"C:\npm", r"bin\claude.exe"), r"C:\npm\bin\claude.exe");
            assert_eq!(
                join("C:/Users/example/npm", r".\bin\..\claude.exe"),
                r"C:\Users\example\npm\claude.exe"
            );
            assert_eq!(join(r"C:\npm", r"..\..\..\claude.exe"), r"C:\claude.exe");
            assert_eq!(
                join(r"\\server\share\npm", r"..\claude.exe"),
                r"\\server\share\claude.exe"
            );
            assert_eq!(join(".", r"..\claude.exe"), r"..\claude.exe");
            assert_eq!(join("C:", "claude.exe"), r"C:\claude.exe");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPM: &str = r"C:\Users\me\AppData\Roaming\npm";

    #[test]
    fn finds_the_program_an_npm_shim_starts() {
        let shim = [
            "@ECHO off",
            "GOTO start",
            ":find_dp0",
            "SET dp0=%~dp0",
            "EXIT /b",
            ":start",
            "SETLOCAL",
            "CALL :find_dp0",
            r#""%dp0%\node_modules\@anthropic-ai\claude-code\bin\claude.exe"   %*"#,
            "",
        ]
        .join("\r\n");
        assert_eq!(
            exe_behind_shim(&format!(r"{NPM}\claude.cmd"), &shim),
            Some(format!(r"{NPM}\node_modules\@anthropic-ai\claude-code\bin\claude.exe"))
        );
    }

    #[test]
    fn finds_nothing_in_a_shim_that_runs_a_script_with_node() {
        let shim = [
            "@ECHO off",
            "SETLOCAL",
            "CALL :find_dp0",
            r#"IF EXIST "%dp0%\node.exe" ("#,
            r#"  SET "_prog=%dp0%\node.exe""#,
            ") ELSE (",
            r#"  SET "_prog=node""#,
            ")",
            r#"endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\node_modules\@anthropic-ai\claude-code\cli.js" %*"#,
        ]
        .join("\r\n");
        assert_eq!(exe_behind_shim(&format!(r"{NPM}\claude.cmd"), &shim), None);
    }

    #[test]
    fn reads_the_other_ways_a_shim_can_say_it() {
        let shim = format!(r"{NPM}\claude.cmd");
        let behind = |text: &str| exe_behind_shim(&shim, text);
        assert_eq!(behind(r#""%~dp0\claude.exe" %*"#), Some(format!(r"{NPM}\claude.exe")));
        assert_eq!(
            behind(r#""%DP0%\Bin\Claude.EXE" %*"#),
            Some(format!(r"{NPM}\Bin\Claude.EXE"))
        );
        assert_eq!(behind(r#""%dp0\claude.exe""#), Some(format!(r"{NPM}\claude.exe")));
        assert_eq!(
            behind(r#""%dp0%\..\tools\claude.exe""#),
            Some(r"C:\Users\me\AppData\Roaming\tools\claude.exe".to_owned())
        );
        // The first quoted program, past ones that aren't programs.
        assert_eq!(
            behind(r#"type "%dp0%\README.txt" & "%dp0%\claude.exe""#),
            Some(format!(r"{NPM}\claude.exe"))
        );
        assert_eq!(behind(r#""%dp0%\bin\NODE.EXE""#), None);
        assert_eq!(behind(r#""%dp0%\mynode.exe""#), Some(format!(r"{NPM}\mynode.exe")));
        assert_eq!(behind(r#""%dp0%\.exe""#), None);
        assert_eq!(behind(r#""%dp0%claude.exe""#), None);
        assert_eq!(behind(r#""%dp1%\claude.exe""#), None);
        assert_eq!(behind(r#"%dp0%\claude.exe"#), None);
        assert_eq!(behind(""), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn asks_claude_its_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let claude = dir.path().join("claude");
        fs::write(&claude, "#!/bin/sh\nprintf '  2.1.0 (Claude Code)\\r\\nmore\\n'\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        let env = EnvMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]);
        assert_eq!(
            read_version(&claude, &env).await.as_deref(),
            Some("2.1.0 (Claude Code)")
        );

        fs::write(&claude, "#!/bin/sh\necho 2.1.0\nexit 1\n").unwrap();
        assert_eq!(read_version(&claude, &env).await, None);
        assert_eq!(read_version(&dir.path().join("missing"), &env).await, None);
    }
}
