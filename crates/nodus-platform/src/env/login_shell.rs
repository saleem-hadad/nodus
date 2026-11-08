//! The environment the user's interactive login shell sets up.
//!
//! Apps launched from Finder or the Dock get a minimal launchd environment, no
//! PATH from ~/.zshrc and the like, so `claude` wouldn't be found. It's read
//! once at startup from the shell itself, between two markers so whatever the
//! shell's startup files print around it doesn't matter.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::EnvMap;

const MARK_START: &str = "__NODUS_ENV_START__";
const MARK_END: &str = "__NODUS_ENV_END__";

/// How long the shell gets to start up and print its environment.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How much of what it prints is read.
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// The login shell's environment; None when it couldn't say. Windows apps
/// get the user's whole environment already, so there's nothing to read.
pub(super) async fn capture() -> Option<EnvMap> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    run(Path::new(&shell), TIMEOUT).await
}

/// Runs `shell -ilc` to print the environment between the markers, and reads
/// what it printed in `timeout`.
pub(super) async fn run(shell: &Path, timeout: Duration) -> Option<EnvMap> {
    let script = format!("printf '%s' '{MARK_START}'; env -0; printf '%s' '{MARK_END}'");
    let mut child = Command::new(shell)
        .arg("-ilc")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut output = Vec::new();
    let read = async {
        let mut chunk = vec![0; 64 * 1024];
        while output.len() < MAX_OUTPUT {
            match stdout.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => output.extend_from_slice(&chunk[..read]),
            }
        }
    };
    // Past the timeout, what it printed so far is all there is.
    let _ = tokio::time::timeout(timeout, read).await;
    output.truncate(MAX_OUTPUT);
    drop(stdout);
    // Interactive shells ignore SIGTERM, so one still going is killed outright.
    let _ = child.start_kill();
    let _ = child.wait().await;
    parse(&String::from_utf8_lossy(&output))
}

/// The variables between the markers, `env -0`'s NUL-separated `NAME=value`
/// entries. Interactive shells may exit non-zero or print noise; the markers
/// are what matter.
fn parse(stdout: &str) -> Option<EnvMap> {
    let start = stdout.find(MARK_START)?;
    let end = stdout.rfind(MARK_END)?;
    if end <= start {
        return None;
    }
    let body = stdout.get(start + MARK_START.len()..end)?;
    let env: EnvMap = body
        .split('\0')
        .filter_map(|entry| match entry.find('=') {
            Some(eq) if eq > 0 => Some((entry[..eq].to_owned(), entry[eq + 1..].to_owned())),
            _ => None,
        })
        .collect();
    (!env.is_empty()).then_some(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_variables_between_the_markers() {
        let stdout = format!(
            "Welcome back!\n{MARK_START}HOME=/Users/example\0PATH=/usr/bin:/bin\0EQUATION=a=b\0\
             MULTI=line one\nline two\0=no name\0no equals sign\0\0{MARK_END}\nlast login: today"
        );
        let env = parse(&stdout).unwrap();
        assert_eq!(
            env,
            EnvMap::from([
                ("HOME".into(), "/Users/example".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("EQUATION".into(), "a=b".into()),
                ("MULTI".into(), "line one\nline two".into()),
            ])
        );
    }

    #[test]
    fn takes_the_first_start_and_the_last_end() {
        let stdout = format!("{MARK_START}A=1\0{MARK_START}B=2\0{MARK_END}C=3\0{MARK_END}");
        let env = parse(&stdout).unwrap();
        let names: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(names, ["A", "__NODUS_ENV_END__C", "__NODUS_ENV_START__B"]);
    }

    #[test]
    fn a_later_variable_wins() {
        let env = parse(&format!("{MARK_START}A=1\0A=2\0{MARK_END}")).unwrap();
        assert_eq!(env["A"], "2");
    }

    #[test]
    fn says_nothing_without_both_markers_in_order() {
        assert_eq!(parse("A=1\0B=2\0"), None);
        assert_eq!(parse(&format!("{MARK_START}A=1\0")), None);
        assert_eq!(parse(&format!("A=1\0{MARK_END}")), None);
        assert_eq!(parse(&format!("{MARK_END}A=1\0{MARK_START}")), None);
    }

    #[test]
    fn says_nothing_without_variables() {
        assert_eq!(parse(&format!("{MARK_START}{MARK_END}")), None);
        assert_eq!(parse(&format!("{MARK_START}\0junk\0{MARK_END}")), None);
    }

    #[cfg(unix)]
    mod shells {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;
        use std::time::Instant;
        use tempfile::TempDir;

        /// A stand-in shell that ignores its arguments and runs `body`.
        fn shell(dir: &TempDir, body: &str) -> PathBuf {
            let path = dir.path().join("shell");
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        #[tokio::test]
        async fn reads_a_real_shell() {
            let env = run(Path::new("/bin/sh"), TIMEOUT).await.unwrap();
            assert!(env.contains_key("PATH"));
        }

        #[tokio::test]
        async fn takes_what_a_shell_printed_before_it_hung() {
            let dir = TempDir::new().unwrap();
            let shell = shell(
                &dir,
                &format!("printf '%s' '{MARK_START}SLOW=yes'; printf '\\000{MARK_END}'; exec sleep 30"),
            );
            let started = Instant::now();
            let env = run(&shell, Duration::from_millis(500)).await.unwrap();
            assert_eq!(env["SLOW"], "yes");
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        #[tokio::test]
        async fn says_nothing_for_a_shell_that_fails() {
            let dir = TempDir::new().unwrap();
            assert_eq!(run(&shell(&dir, "echo oops; exit 3"), TIMEOUT).await, None);
            assert_eq!(run(&dir.path().join("missing"), TIMEOUT).await, None);
        }
    }
}
