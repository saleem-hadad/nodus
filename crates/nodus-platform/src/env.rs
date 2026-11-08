//! The environment sessions, terminals and commands start in, and where the
//! `claude` binary is.
//!
//! Apps launched from Finder or the Dock get a minimal environment, so the
//! user's login-shell environment is read once at startup ([`Env::init`]).
//! Windows apps get the user's whole environment already, so there's nothing
//! to read there. A port of `src/main/env.ts`.

mod claude;
mod login_shell;
mod path_search;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use claude::exe_behind_shim;

/// Environment variables by name.
pub type EnvMap = BTreeMap<String, String>;

/// Session markers set by a parent Claude Code process; claude refuses to start nested.
const STRIP_EXACT: [&str; 7] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "ELECTRON_RUN_AS_NODE",
];
const STRIP_PREFIXES: [&str; 6] = [
    "CLAUDE_CODE_SESSION",
    "CLAUDE_CODE_MESSAGING",
    "CLAUDE_CODE_SSE",
    "CLAUDE_AGENT_SDK",
    "NODUS_",
    "ELECTRON_",
];

/// The user's environment and the `claude` binary, as found at startup.
#[derive(Debug)]
pub struct Env {
    base: EnvMap,
    claude_path: Option<PathBuf>,
    claude_version: Option<String>,
    git_bash: OnceLock<Option<PathBuf>>,
}

/// A program to run and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    pub file: PathBuf,
    pub args: Vec<String>,
}

/// What a session that needs Claude Code says when it isn't installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Claude Code CLI not found. Install it ({install}) and restart Nodus.", install = INSTALL)]
pub struct ClaudeNotFound;

const INSTALL: &str = if cfg!(windows) {
    "irm https://claude.ai/install.ps1 | iex, in PowerShell"
} else {
    "npm i -g @anthropic-ai/claude-code"
};

impl Env {
    /// The app's own environment, as it is until [`Env::init`] has read the
    /// login shell's. No `claude` has been looked for yet.
    pub fn from_process() -> Self {
        Self::new(process_env(), None, None)
    }

    /// Reads the login shell's environment over the app's own, adds the
    /// folders Claude Code installs to the end of the PATH, finds `claude` on
    /// it and asks its version. The shell can take a few seconds.
    pub async fn init() -> Self {
        let mut base = process_env();
        if let Some(captured) = login_shell::capture().await {
            base.extend(captured);
        }
        Self::resolve(base, std::env::home_dir().as_deref()).await
    }

    async fn resolve(mut base: EnvMap, home: Option<&Path>) -> Self {
        path_search::add_fallback_dirs(&mut base, home);
        let claude_path = claude::find(base.get("PATH").map_or("", String::as_str));
        let mut env = Self::new(base, claude_path, None);
        if let Some(claude) = &env.claude_path {
            env.claude_version = claude::read_version(claude, &env.spawn_env()).await;
        }
        env
    }

    fn new(base: EnvMap, claude_path: Option<PathBuf>, claude_version: Option<String>) -> Self {
        Self {
            base,
            claude_path,
            claude_version,
            git_bash: OnceLock::new(),
        }
    }

    /// Environment for spawned claude processes: the user's shell environment
    /// minus nesting markers, as a terminal that speaks UTF-8 in full colour.
    /// Callers add their own variables over it.
    pub fn spawn_env(&self) -> EnvMap {
        let mut env: EnvMap = self
            .base
            .iter()
            .filter(|(name, _)| !should_strip(name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        if env.get("LANG").is_none_or(String::is_empty) {
            env.insert("LANG".into(), "en_US.UTF-8".into());
        }
        env
    }

    /// A variable from the user's login-shell environment.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.base.get(name).map(String::as_str)
    }

    /// The user's login shell. On Windows, PowerShell: PowerShell 7 when it's installed.
    pub fn user_shell(&self) -> PathBuf {
        if cfg!(windows) {
            let system_root = self
                .set("SystemRoot")
                .or_else(|| self.set("SYSTEMROOT"))
                .unwrap_or(r"C:\Windows");
            let windows_powershell = || {
                let v1 = Path::new(system_root)
                    .join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0");
                v1.join("powershell.exe")
            };
            return path_search::find_on_path(&["pwsh.exe"], self.path_var()).unwrap_or_else(windows_powershell);
        }
        let shell = self
            .set("SHELL")
            .map(str::to_owned)
            .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()));
        match shell {
            Some(shell) if Path::new(&shell).is_absolute() => shell.into(),
            _ => "/bin/zsh".into(),
        }
    }

    /// A terminal card's shell: a login shell, as Terminal.app starts one, or PowerShell on Windows.
    pub fn terminal_shell(&self) -> ShellCommand {
        let login = if cfg!(windows) { "-NoLogo" } else { "-l" };
        ShellCommand {
            file: self.user_shell(),
            args: vec![login.into()],
        }
    }

    /// Runs one command line in the user's shell. PowerShell is told to print
    /// UTF-8, which Windows PowerShell otherwise doesn't when its output is piped.
    pub fn shell_command(&self, run: &str) -> ShellCommand {
        let args = if cfg!(windows) {
            let utf8 = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; ";
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!("{utf8}{run}"),
            ]
        } else {
            vec!["-c".into(), run.into()]
        };
        ShellCommand {
            file: self.user_shell(),
            args,
        }
    }

    /// Git for Windows' bash, which Claude Code runs hooks in when it finds
    /// one; None without it, or off Windows. Looked for the way Claude Code
    /// does: its CLAUDE_CODE_GIT_BASH_PATH, then beside the git on the PATH,
    /// then where Git installs by default. Looked for once.
    pub fn git_bash(&self) -> Option<&Path> {
        if !cfg!(windows) {
            return None;
        }
        self.git_bash.get_or_init(|| self.find_git_bash()).as_deref()
    }

    fn find_git_bash(&self) -> Option<PathBuf> {
        if let Some(set) = self.set("CLAUDE_CODE_GIT_BASH_PATH") {
            let set = PathBuf::from(set);
            return set.exists().then_some(set);
        }
        let mut candidates = Vec::new();
        // …\Git\cmd\git.exe, …\Git\bin\git.exe or …\Git\mingw64\bin\git.exe.
        if let Some(git) = path_search::find_on_path(&["git.exe"], self.path_var()) {
            let mut dir = git.as_path();
            for _ in 0..3 {
                dir = dir.parent().unwrap_or(dir);
                candidates.push(dir.join("bin").join("bash.exe"));
            }
        }
        let local = self.set("LOCALAPPDATA").map(|local| Path::new(local).join("Programs"));
        let roots = [
            self.set("ProgramFiles").map(PathBuf::from),
            self.set("ProgramFiles(x86)").map(PathBuf::from),
            local,
        ];
        for root in roots.into_iter().flatten() {
            candidates.push(root.join("Git").join("bin").join("bash.exe"));
        }
        candidates.into_iter().find(|candidate| candidate.exists())
    }

    pub fn claude_path(&self) -> Option<&Path> {
        self.claude_path.as_deref()
    }

    pub fn claude_version(&self) -> Option<&str> {
        self.claude_version.as_deref()
    }

    /// The `claude` binary, for starting a session.
    pub fn require_claude(&self) -> Result<&Path, ClaudeNotFound> {
        self.claude_path().ok_or(ClaudeNotFound)
    }

    /// A variable that's set to something.
    fn set(&self, name: &str) -> Option<&str> {
        self.var(name).filter(|value| !value.is_empty())
    }

    fn path_var(&self) -> &str {
        self.var("PATH").unwrap_or_default()
    }
}

fn should_strip(name: &str) -> bool {
    STRIP_EXACT.contains(&name) || STRIP_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn process_env() -> EnvMap {
    to_env_map(
        std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())),
        cfg!(windows),
    )
}

/// The environment as a map. Windows calls PATH Path, and its names ignore
/// case: it gets one PATH, the first, so lookups and children agree.
fn to_env_map(vars: impl IntoIterator<Item = (String, String)>, windows: bool) -> EnvMap {
    let mut env = EnvMap::new();
    for (name, value) in vars {
        if windows && name.eq_ignore_ascii_case("PATH") {
            env.entry("PATH".into()).or_insert(value);
        } else {
            env.insert(name, value);
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> Env {
        Env::new(
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            None,
            None,
        )
    }

    #[test]
    fn spawns_without_a_parent_sessions_markers() {
        let env = env(&[
            ("HOME", "/Users/example"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_CODE_EXECPATH", "/opt/claude"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_PID", "123"),
            ("CLAUDE_EFFORT", "high"),
            ("ELECTRON_RUN_AS_NODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "s"),
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/m"),
            ("CLAUDE_CODE_SSE_PORT", "1234"),
            ("CLAUDE_AGENT_SDK_VERSION", "1"),
            ("NODUS_SESSION_ID", "s"),
            ("NODUS_USER_DATA", "/tmp/nodus"),
            ("ELECTRON_ENABLE_LOGGING", "1"),
            ("CLAUDE_CODE_GIT_BASH_PATH", "/opt/bash"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CONFIG_DIR", "/Users/example/.claude"),
        ]);
        let spawned = env.spawn_env();
        let names: Vec<&str> = spawned.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "CLAUDE_CODE_GIT_BASH_PATH",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CONFIG_DIR",
                "COLORTERM",
                "HOME",
                "LANG",
                "TERM"
            ]
        );
        assert_eq!(spawned["TERM"], "xterm-256color");
        assert_eq!(spawned["COLORTERM"], "truecolor");
        assert_eq!(spawned["LANG"], "en_US.UTF-8");
    }

    #[test]
    fn keeps_the_users_language_and_sets_the_terminal() {
        let spawned = env(&[("LANG", "de_DE.UTF-8"), ("TERM", "dumb"), ("COLORTERM", "")]).spawn_env();
        assert_eq!(spawned["LANG"], "de_DE.UTF-8");
        assert_eq!(spawned["TERM"], "xterm-256color");
        assert_eq!(spawned["COLORTERM"], "truecolor");
        assert_eq!(env(&[("LANG", "")]).spawn_env()["LANG"], "en_US.UTF-8");
    }

    #[test]
    fn reads_variables() {
        let env = env(&[("XAI_API_KEY", "example"), ("EMPTY", "")]);
        assert_eq!(env.var("XAI_API_KEY"), Some("example"));
        assert_eq!(env.var("EMPTY"), Some(""));
        assert_eq!(env.var("MISSING"), None);
    }

    #[test]
    fn folds_windows_path_names_into_one() {
        let vars = [
            ("Path", "C:\\first"),
            ("PATH", "C:\\second"),
            ("Home", "C:\\Users\\example"),
        ];
        let vars = || vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned()));
        let windows = to_env_map(vars(), true);
        assert_eq!(windows.get("PATH").map(String::as_str), Some("C:\\first"));
        assert!(!windows.contains_key("Path"));
        assert!(windows.contains_key("Home"));
        let elsewhere = to_env_map(vars(), false);
        assert_eq!(elsewhere.len(), 3);
    }

    #[test]
    fn says_how_to_install_claude() {
        let env = env(&[]);
        let err = env.require_claude().unwrap_err();
        if cfg!(windows) {
            assert_eq!(
                err.to_string(),
                "Claude Code CLI not found. Install it (irm https://claude.ai/install.ps1 | iex, in PowerShell) and restart Nodus."
            );
        } else {
            assert_eq!(
                err.to_string(),
                "Claude Code CLI not found. Install it (npm i -g @anthropic-ai/claude-code) and restart Nodus."
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn runs_the_users_shell() {
        let zsh = env(&[("SHELL", "/bin/zsh")]);
        assert_eq!(zsh.user_shell(), PathBuf::from("/bin/zsh"));
        assert_eq!(
            env(&[("SHELL", "/opt/homebrew/bin/fish")]).terminal_shell(),
            ShellCommand {
                file: "/opt/homebrew/bin/fish".into(),
                args: vec!["-l".into()]
            }
        );
        assert_eq!(
            zsh.shell_command("git status && ls"),
            ShellCommand {
                file: "/bin/zsh".into(),
                args: vec!["-c".into(), "git status && ls".into()]
            }
        );
        // Only an absolute path will do.
        assert_eq!(env(&[("SHELL", "fish")]).user_shell(), PathBuf::from("/bin/zsh"));
        assert_eq!(zsh.git_bash(), None);
    }

    #[cfg(windows)]
    #[test]
    fn runs_powershell() {
        let env = env(&[("SystemRoot", r"C:\Windows"), ("PATH", "")]);
        let powershell = PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        assert_eq!(env.user_shell(), powershell);
        assert_eq!(
            env.terminal_shell(),
            ShellCommand {
                file: powershell.clone(),
                args: vec!["-NoLogo".into()]
            }
        );
        assert_eq!(
            env.shell_command("git status"),
            ShellCommand {
                file: powershell,
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "[Console]::OutputEncoding = [Text.Encoding]::UTF8; git status".into()
                ]
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn finds_git_bash_where_its_set() {
        let dir = tempfile::TempDir::new().unwrap();
        let bash = dir.path().join("bash.exe");
        std::fs::write(&bash, "").unwrap();
        let set = env(&[("CLAUDE_CODE_GIT_BASH_PATH", bash.to_str().unwrap()), ("PATH", "")]);
        assert_eq!(set.git_bash(), Some(bash.as_path()));
        let missing = env(&[("CLAUDE_CODE_GIT_BASH_PATH", r"C:\nowhere\bash.exe"), ("PATH", "")]);
        assert_eq!(missing.git_bash(), None);
    }

    #[cfg(windows)]
    #[test]
    fn finds_git_bash_beside_git() {
        let dir = tempfile::TempDir::new().unwrap();
        let git = dir.path().join("Git");
        std::fs::create_dir_all(git.join("cmd")).unwrap();
        std::fs::create_dir_all(git.join("bin")).unwrap();
        std::fs::write(git.join("cmd").join("git.exe"), "").unwrap();
        std::fs::write(git.join("bin").join("bash.exe"), "").unwrap();
        let env = env(&[("PATH", git.join("cmd").to_str().unwrap())]);
        assert_eq!(env.git_bash(), Some(git.join("bin").join("bash.exe").as_path()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finds_claude_and_its_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let claude = dir.path().join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho '2.1.0 (Claude Code)'\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let base = EnvMap::from([("PATH".to_owned(), dir.path().display().to_string())]);
        let env = Env::resolve(base, Some(Path::new("/Users/example"))).await;
        assert_eq!(env.claude_path(), Some(claude.as_path()));
        assert_eq!(env.claude_version(), Some("2.1.0 (Claude Code)"));
        assert_eq!(env.require_claude(), Ok(claude.as_path()));
        assert!(env.var("PATH").unwrap().ends_with(":/usr/sbin:/sbin"));
    }
}
