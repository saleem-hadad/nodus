//! What the core does differently on a server (`nodus serve`), behind
//! [`Hosting`], which only a server gives ([`NodusConfig::hosting`]). With
//! none, as in the app, nothing here runs.
//!
//! - **One token per session** (SRV-143): every session gets its own
//!   `NODUS_TOKEN`, and what the host relays for it (hooks, the status line,
//!   the board tools, its credential helper) is taken only with that token.
//! - **Git through the App** (SRV-080…084): each session of a project from
//!   GitHub gets the credential helper and its starter's commit identity in
//!   its environment, never a token. The helper asks the host, the host asks
//!   here, and the server mints a token for that one repository.
//! - **A worktree per card** (SRV-121…127): a new card on a project from
//!   GitHub gets a worktree and branch of its own, cut from the default
//!   branch after a fetch, set up by the repo's `.nodus/setup`, trusted for
//!   Claude Code, with a preview port; removing the card saves what wasn't
//!   pushed first. All git work runs through the runner, so in the session
//!   host as `nodus-run` (SRV-005).
//!
//! [`NodusConfig::hosting`]: crate::NodusConfig::hosting

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use nodus_domain::{
    BoardNode, CreatedBy, Event, FolderColor, Project, ProjectGithub, SessionKind, SetupState, Worktree, WorktreeSetup,
};
use nodus_platform::env::EnvMap;
use nodus_platform::process::Output;
use nodus_platform::runner::RunRequest;
use nodus_sessions::client::{BoxFuture as ClientFuture, ServerSessions};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::nodus::{Inner, lock};
use crate::{CommandError, CommandResult, Nodus};

/// A future a [`Hosting`] gives back.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The preview ports cards are given (SRV-127).
pub const PREVIEW_PORTS: std::ops::RangeInclusive<u16> = 8100..=8999;
/// How long a clone, a fetch or a card's setup may take.
const CLONE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const GIT_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// How long saved work is kept (SRV-124).
const SAVED_DAYS: u32 = 30;
/// How often a card's setup line and its preview port are looked at.
const SETUP_POLL: Duration = Duration::from_secs(1);
const PREVIEW_POLL: Duration = Duration::from_secs(3);

/// Someone's name and address, as git records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

/// What an installation token is for (SRV-080): each gets only the
/// permissions its job needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenScope {
    /// Cloning, fetching and pushing: `contents: write`.
    Git,
    /// The `gh` shim: pull requests and issues too (SRV-083).
    Gh,
}

/// What a server gives the core, and only a server (SRV-002).
pub trait Hosting: Send + Sync {
    /// A session's own token (SRV-143): the same every time for the same
    /// session, so it holds across restarts of `serve`, and nobody without
    /// the server's secret can work it out.
    fn session_token(&self, session_id: &str) -> String;
    /// A member's commit identity, by their login (SRV-082): their name, and
    /// `<githubId>+<login>@users.noreply.github.com`.
    fn author(&self, login: &str) -> Option<GitIdentity>;
    /// The App's bot, which commits (SRV-082).
    fn committer(&self) -> BoxFuture<'_, Option<GitIdentity>>;
    /// An installation token for one repository, with only what `scope`
    /// needs, cached for 50 minutes (SRV-080).
    fn mint<'a>(&'a self, github: &'a ProjectGithub, scope: TokenScope) -> BoxFuture<'a, Result<String, String>>;
    /// GitHub's address as git reaches it: `https://github.com`.
    fn github_url(&self) -> String;
    /// git's `credential.helper` for sessions: `nodus git-credential`, as a
    /// shell command (SRV-084).
    fn credential_helper(&self) -> String;
}

/// What the core keeps for a server.
pub(crate) struct Hosted {
    pub(crate) hosting: Arc<dyn Hosting>,
    /// Tokens given to the core's own git work (a clone, a fetch, saving a
    /// card's work), by token, for the repository each may reach. Gone once
    /// the work is done.
    jobs: std::sync::Mutex<HashMap<String, ProjectGithub>>,
    /// Cards whose worktree is still being set up, by session id: their
    /// process waits for it (SRV-123).
    preparing: std::sync::Mutex<HashMap<String, watch::Receiver<bool>>>,
}

impl Hosted {
    pub(crate) fn new(hosting: Arc<dyn Hosting>) -> Self {
        Self {
            hosting,
            jobs: std::sync::Mutex::default(),
            preparing: std::sync::Mutex::default(),
        }
    }
}

/// A token for one piece of the core's own git work, given up when dropped.
struct JobToken<'a> {
    hosted: &'a Hosted,
    token: String,
}

impl Drop for JobToken<'_> {
    fn drop(&mut self) {
        lock(&self.hosted.jobs).remove(&self.token);
    }
}

/// The credential helper's settings, as `GIT_CONFIG_COUNT` and its keys
/// and values (SRV-081, SRV-084): every helper git was given before is
/// dropped, then ours, asked with the repository's path so it can answer
/// for that one only.
pub(crate) fn helper_env(helper: &str) -> Vec<(String, String)> {
    let entries = [
        ("credential.helper", ""),
        ("credential.helper", helper),
        ("credential.useHttpPath", "true"),
    ];
    let mut env = vec![("GIT_CONFIG_COUNT".to_owned(), entries.len().to_string())];
    for (i, (key, value)) in entries.iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{i}"), (*key).to_owned()));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), (*value).to_owned()));
    }
    env
}

/// A random token, as long as a session's.
fn random_token() -> String {
    uuid::Uuid::new_v4().simple().to_string() + &uuid::Uuid::new_v4().simple().to_string()
}

/// Compared in constant time.
fn same(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// A worktree's branch name (SRV-122): `nodus/<title slug>-<first 4 of
/// the card's id>`, the slug lower case, ASCII and short.
pub fn branch_name(title: Option<&str>, kind: Option<SessionKind>, session_id: &str) -> String {
    let fallback = if kind == Some(SessionKind::Terminal) {
        "terminal"
    } else {
        "session"
    };
    let mut slug = String::new();
    for c in title.unwrap_or("").chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { fallback } else { slug };
    format!("nodus/{slug}-{}", short_id(session_id, 4))
}

/// The first `n` letters and digits of a card's id.
fn short_id(session_id: &str, n: usize) -> String {
    session_id.chars().filter(char::is_ascii_alphanumeric).take(n).collect()
}

/// A project folder's `projects/<slug>` (its clone is `main` in it).
fn project_home(project: &Project) -> PathBuf {
    let main = Path::new(&project.repo_path);
    main.parent().map_or_else(|| main.to_path_buf(), Path::to_path_buf)
}

/// What a run printed last, for the card (SRV-123).
fn last_line(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    nodus_domain::js::slice_utf16(&strip_ansi(line), 0, 200)
}

/// A line without its terminal colours.
fn strip_ansi(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\r' {
            out.clear();
            continue;
        }
        out.push(c);
    }
    out
}

impl Inner {
    /// The server's part, when this is a server.
    pub(crate) fn hosted(&self) -> Option<&Hosted> {
        self.hosted.as_ref()
    }

    /// What a session's process gets on a server on top of its command's
    /// environment (SRV-141, SRV-143, SRV-082, SRV-084, SRV-127), and the
    /// folder it starts in: its card's worktree when it has one (SRV-122).
    pub(crate) async fn session_extras(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Option<(Vec<(String, String)>, String, Option<String>)> {
        let hosted = self.hosted()?;
        let hosting = &hosted.hosting;
        let token = hosting.session_token(session_id);
        let (project, node) = self.store.read(|state| {
            let project = state.project(project_id).ok().cloned();
            let node = state.board(project_id).and_then(|b| b.nodes.get(session_id).cloned());
            (project, node)
        });
        let hook = self.endpoint.env(session_id);
        let mut env: Vec<(String, String)> = vec![
            ("NODUS_SESSION".into(), session_id.to_owned()),
            ("NODUS_TOKEN".into(), token.clone()),
        ];
        if let Some(port) = hook.get("NODUS_PORT") {
            env.push(("NODUS_PORT".into(), port.clone()));
        }
        if project.as_ref().and_then(Project::github).is_some() {
            env.extend(helper_env(&hosting.credential_helper()));
            if let Some(author) = node
                .as_ref()
                .and_then(|n| n.created_by())
                .and_then(|by| hosting.author(by))
            {
                env.push(("GIT_AUTHOR_NAME".into(), author.name));
                env.push(("GIT_AUTHOR_EMAIL".into(), author.email));
            }
            if let Some(bot) = hosting.committer().await {
                env.push(("GIT_COMMITTER_NAME".into(), bot.name));
                env.push(("GIT_COMMITTER_EMAIL".into(), bot.email));
            }
        }
        if let Some(port) = node.as_ref().and_then(BoardNode::preview_port) {
            env.push(("PORT".into(), port.to_string()));
            env.push(("NODUS_PREVIEW_PORT".into(), port.to_string()));
        }
        let cwd = node.as_ref().and_then(BoardNode::worktree).map(|w| w.path);
        Some((env, cwd.unwrap_or_default(), Some(token)))
    }

    /// Whether a card's worktree is being set up (SRV-123).
    pub(crate) fn is_setting_up(&self, session_id: &str) -> bool {
        self.hosted()
            .is_some_and(|hosted| lock(&hosted.preparing).contains_key(session_id))
    }

    /// Waits while a card's worktree is set up (SRV-123): its process starts
    /// after, however it's asked to.
    pub(crate) async fn wait_for_setup(&self, session_id: &str) {
        let Some(hosted) = self.hosted() else { return };
        let waiting = lock(&hosted.preparing).get(session_id).cloned();
        if let Some(mut done) = waiting {
            let _ = done.wait_for(|done| *done).await;
        }
    }

    /// git with a token for one repository, for the core's own work: the
    /// environment, and the token, given up once dropped (SRV-084).
    async fn job_env<'a>(&self, hosted: &'a Hosted, github: &ProjectGithub) -> (EnvMap, JobToken<'a>) {
        // The helper asks the host's hook port, known once it's connected.
        let _ = self.host.connect_host(true).await;
        let token = random_token();
        lock(&hosted.jobs).insert(token.clone(), github.clone());
        let mut env = self.env().await.spawn_env();
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.extend(helper_env(&hosted.hosting.credential_helper()));
        if let Some(port) = self.endpoint.env("").get("NODUS_PORT") {
            env.insert("NODUS_PORT".into(), port.clone());
        }
        env.insert("NODUS_TOKEN".into(), token.clone());
        env.remove("NODUS_SESSION");
        (env, JobToken { hosted, token })
    }

    /// Runs git through the runner: in the session host on a server
    /// (SRV-005).
    async fn git_run(&self, env: &EnvMap, cwd: &Path, args: &[&str], timeout: Duration) -> Result<Output, String> {
        let request = RunRequest::new("git")
            .args(args.iter().copied())
            .cwd(cwd)
            .env(env.clone())
            .timeout(timeout);
        self.runner.run(request).await.map_err(|err| err.to_string())
    }

    /// Like [`Inner::git_run`], failing with git's own words when it does.
    async fn git_ok(&self, env: &EnvMap, cwd: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
        let out = self.git_run(env, cwd, args, timeout).await?;
        if out.succeeded() {
            Ok(out.stdout_text())
        } else {
            let why = last_line(&out.stderr_text());
            Err(if why.is_empty() {
                format!("git {} failed", args.first().unwrap_or(&""))
            } else {
                why
            })
        }
    }

    /// SRV-122, SRV-126, SRV-127: a new card on a project from GitHub gets
    /// its worktree and branch, cut from `origin/<default>` after a fetch,
    /// trusted for Claude Code, and a preview port, all recorded on the
    /// card. Then its `.nodus/setup`, when the repo has one, which the
    /// card's process waits for (SRV-123). Nothing for any other card.
    pub(crate) async fn prepare_card(self: &Arc<Self>, project: &Project, node: &BoardNode) -> Result<(), String> {
        let (Some(hosted), Some(github)) = (self.hosted(), project.github()) else {
            return Ok(());
        };
        let main = PathBuf::from(&project.repo_path);
        let (env, job) = self.job_env(hosted, &github).await;
        // SRV-121: the main checkout is only fetched. Without GitHub, the
        // branch is cut from what was fetched last.
        if let Err(err) = self
            .git_ok(&env, &main, &["fetch", "--prune", "origin"], FETCH_TIMEOUT)
            .await
        {
            tracing::warn!("[worktree] fetch failed {err}");
        }
        let path = project_home(project)
            .join("worktrees")
            .join(short_id(&node.session_id, 8));
        let path_text = path.to_string_lossy().into_owned();
        let base = format!("origin/{}", github.default_branch);
        let mut branch = branch_name(node.title.as_deref(), node.kind, &node.session_id);
        let mut added = self
            .git_ok(
                &env,
                &main,
                &["worktree", "add", "-b", &branch, &path_text, &base],
                GIT_TIMEOUT,
            )
            .await;
        if added.is_err() {
            // A branch of that name already: the longer id tells them apart.
            branch = format!("{branch}{}", &short_id(&node.session_id, 8)[4..]);
            added = self
                .git_ok(
                    &env,
                    &main,
                    &["worktree", "add", "-b", &branch, &path_text, &base],
                    GIT_TIMEOUT,
                )
                .await;
        }
        drop(job);
        added.map_err(|err| format!("Can't make the card's worktree: {err}"))?;
        let worktree = Worktree {
            path: path_text.clone(),
            branch,
        };
        let port = self.free_preview_port();
        let setup = path.join(".nodus").join("setup");
        let has_setup = is_executable(&setup);
        let session_id = node.session_id.clone();
        self.store.update(|state| {
            if let Some(node) = state.board_mut(&project.id).nodes.get_mut(&session_id) {
                node.set_worktree(Some(&worktree));
                node.set_preview_port(port);
                if has_setup {
                    node.set_setup(Some(&WorktreeSetup {
                        state: SetupState::Running,
                        line: String::new(),
                    }));
                }
            }
        });
        // SRV-126: as accepting Claude Code's trust dialog there would.
        for folder in [&path_text, &project.repo_path] {
            if let Err(err) = self.host.trust(folder).await {
                tracing::warn!("[worktree] trust failed {err}");
            }
        }
        if has_setup {
            let (done, waiting) = watch::channel(false);
            lock(&hosted.preparing).insert(session_id.clone(), waiting);
            let core = Arc::clone(self);
            let project = project.clone();
            self.runtime.spawn(async move {
                core.run_setup(&project, &session_id, &path, &setup).await;
                if let Some(hosted) = core.hosted() {
                    lock(&hosted.preparing).remove(&session_id);
                }
                let _ = done.send(true);
            });
        }
        Ok(())
    }

    /// SRV-123: runs the worktree's `.nodus/setup` there, with the card's
    /// environment, its output in `setup-logs/` beside the worktrees, whose
    /// last line the card shows meanwhile. The card says if it failed;
    /// either way the session starts after.
    async fn run_setup(&self, project: &Project, session_id: &str, worktree: &Path, script: &Path) {
        let log = project_home(project)
            .join("setup-logs")
            .join(format!("{}.log", short_id(session_id, 8)));
        let mut env = self.env().await.spawn_env();
        if let Some((extras, _, _)) = self.session_extras(&project.id, session_id).await {
            env.extend(extras);
        }
        let request = RunRequest::new("/bin/sh")
            .args([
                "-c".to_owned(),
                r#"mkdir -p "$(dirname "$1")" && exec "$0" > "$1" 2>&1"#.to_owned(),
                script.to_string_lossy().into_owned(),
                log.to_string_lossy().into_owned(),
            ])
            .cwd(worktree)
            .env(env)
            .timeout(SETUP_TIMEOUT);
        let running = self.runner.run(request);
        tokio::pin!(running);
        let mut shown = String::new();
        let ended = loop {
            tokio::select! {
                ended = &mut running => break ended,
                () = tokio::time::sleep(SETUP_POLL) => {
                    let line = tokio::fs::read_to_string(&log).await.map(|t| last_line(&t)).unwrap_or_default();
                    if line != shown {
                        shown.clone_from(&line);
                        self.set_setup(&project.id, session_id, Some((SetupState::Running, line)));
                    }
                }
            }
        };
        let line = tokio::fs::read_to_string(&log)
            .await
            .map(|t| last_line(&t))
            .unwrap_or_default();
        match ended {
            Ok(out) if out.succeeded() => self.set_setup(&project.id, session_id, None),
            Ok(out) => {
                let why = if out.timed_out {
                    "Setup took too long".to_owned()
                } else if line.is_empty() {
                    format!(
                        "Setup failed ({})",
                        out.code().map_or("stopped".to_owned(), |c| format!("exit {c}"))
                    )
                } else {
                    line
                };
                self.set_setup(&project.id, session_id, Some((SetupState::Failed, why)));
            }
            Err(err) => self.set_setup(&project.id, session_id, Some((SetupState::Failed, err.to_string()))),
        }
    }

    fn set_setup(&self, project_id: &str, session_id: &str, setup: Option<(SetupState, String)>) {
        let setup = setup.map(|(state, line)| WorktreeSetup { state, line });
        self.store.update(|state| {
            if let Some(node) = state.board_mut(project_id).nodes.get_mut(session_id) {
                node.set_setup(setup.as_ref());
            }
        });
        let _ = self.events.send(Event::BoardChanged {
            project_id: project_id.to_owned(),
        });
    }

    /// The lowest preview port no card has and nothing listens on (SRV-127).
    fn free_preview_port(&self) -> Option<u16> {
        let taken: std::collections::HashSet<u16> = self.store.read(|state| {
            state
                .boards
                .values()
                .flat_map(|board| board.nodes.values())
                .filter_map(BoardNode::preview_port)
                .collect()
        });
        PREVIEW_PORTS
            .filter(|port| !taken.contains(port))
            .find(|&port| std::net::TcpListener::bind(("0.0.0.0", port)).is_ok())
    }

    /// SRV-124: a card that left the board takes its worktree with it.
    /// What wasn't committed, or was committed and not pushed, is saved
    /// first in `saved/<cardId>-<time>/`: a `git diff --binary` patch of
    /// everything not committed, and a bundle of the branch's commits that
    /// no remote has (its last commit when none). Saved work older than 30
    /// days goes. Archived cards keep theirs.
    pub(crate) async fn retire_worktree(&self, project: &Project, node: &BoardNode) {
        let (Some(hosted), Some(github), Some(worktree)) = (self.hosted(), project.github(), node.worktree()) else {
            return;
        };
        let (env, _job) = self.job_env(hosted, &github).await;
        let path = PathBuf::from(&worktree.path);
        let main = PathBuf::from(&project.repo_path);
        let home = project_home(project);
        if path.exists() {
            if let Err(err) = self.save_work(&env, &home, &path, &worktree, &node.session_id).await {
                tracing::error!("[worktree] couldn't save {}: {err}", worktree.path);
                // Kept, rather than lost.
                return;
            }
            let removed = self
                .git_ok(
                    &env,
                    &main,
                    &["worktree", "remove", "--force", &worktree.path],
                    GIT_TIMEOUT,
                )
                .await;
            if let Err(err) = removed {
                tracing::error!("[worktree] couldn't remove {}: {err}", worktree.path);
            }
        }
        let _ = self.git_run(&env, &main, &["worktree", "prune"], GIT_TIMEOUT).await;
        let _ = self
            .git_run(&env, &main, &["branch", "-D", &worktree.branch], GIT_TIMEOUT)
            .await;
        let saved = home.join("saved");
        let prune = RunRequest::new("find")
            .args([
                saved.to_string_lossy().into_owned(),
                "-mindepth".into(),
                "1".into(),
                "-maxdepth".into(),
                "1".into(),
                "-mtime".into(),
                format!("+{SAVED_DAYS}"),
                "-exec".into(),
                "rm".into(),
                "-rf".into(),
                "{}".into(),
                "+".into(),
            ])
            .env(env.clone())
            .timeout(GIT_TIMEOUT);
        let _ = self.runner.run(prune).await;
    }

    async fn save_work(
        &self,
        env: &EnvMap,
        home: &Path,
        path: &Path,
        worktree: &Worktree,
        session_id: &str,
    ) -> Result<(), String> {
        let status = self
            .git_ok(
                env,
                path,
                &["status", "--porcelain", "--untracked-files=all"],
                GIT_TIMEOUT,
            )
            .await?;
        let dirty = !status.trim().is_empty();
        let unpushed = self
            .git_ok(
                env,
                path,
                &["rev-list", "--count", &worktree.branch, "--not", "--remotes"],
                GIT_TIMEOUT,
            )
            .await
            .map(|n| n.trim().parse::<u64>().unwrap_or(0))
            .unwrap_or(0);
        if !dirty && unpushed == 0 {
            return Ok(());
        }
        let stamp = time_stamp();
        let dir = home.join("saved").join(format!("{session_id}-{stamp}"));
        let dir_text = dir.to_string_lossy().into_owned();
        let mkdir = RunRequest::new("mkdir")
            .args(["-p", dir_text.as_str()])
            .env(env.clone())
            .timeout(GIT_TIMEOUT);
        let made = self.runner.run(mkdir).await.map_err(|err| err.to_string())?;
        if !made.succeeded() {
            return Err(format!("Can't make {dir_text}"));
        }
        // Everything not committed, new files too: staged here, since the
        // worktree goes next.
        self.git_ok(env, path, &["add", "-A"], GIT_TIMEOUT).await?;
        let patch = format!("--output={}", dir.join("changes.patch").display());
        self.git_ok(
            env,
            path,
            &["diff", "--binary", "--cached", "HEAD", &patch],
            GIT_TIMEOUT,
        )
        .await?;
        let bundle = dir.join("branch.bundle").to_string_lossy().into_owned();
        let bundled = if unpushed > 0 {
            self.git_ok(
                env,
                path,
                &["bundle", "create", &bundle, &worktree.branch, "--not", "--remotes"],
                GIT_TIMEOUT,
            )
            .await
        } else {
            self.git_ok(
                env,
                path,
                &["bundle", "create", &bundle, "-1", &worktree.branch],
                GIT_TIMEOUT,
            )
            .await
        };
        bundled.map(|_| ())
    }
}

/// `yyyymmdd-HHMMSS`, in UTC.
fn time_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Whether a file is there and may be run.
fn is_executable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// SRV-143, SRV-084: the server's checks on what its host relays.
pub(crate) struct CoreServerSessions(pub(crate) Weak<Inner>);

impl ServerSessions for CoreServerSessions {
    fn accepts(&self, session_id: &str, token: Option<&str>) -> bool {
        let Some(core) = self.0.upgrade() else {
            return false;
        };
        let Some(hosted) = core.hosted() else {
            return true;
        };
        token.is_some_and(|token| same(token, &hosted.hosting.session_token(session_id)))
    }

    fn credential(&self, session_id: Option<String>, token: String, request: Value) -> ClientFuture<Value> {
        let core = self.0.clone();
        Box::pin(async move {
            let Some(core) = core.upgrade() else {
                return refused("The server is stopping");
            };
            match core.credential(session_id.as_deref(), &token, &request).await {
                Ok(answer) => answer,
                Err(why) => refused(&why),
            }
        })
    }
}

fn refused(why: &str) -> Value {
    json!({ "error": why })
}

impl Inner {
    /// SRV-084: a credential for a session's helper, or for the core's own
    /// git work, only for `github.com` (the server's GitHub) and only for
    /// the one repository it may reach.
    async fn credential(&self, session_id: Option<&str>, token: &str, request: &Value) -> Result<Value, String> {
        let hosted = self.hosted().ok_or("This isn't a server")?;
        let hosting = &hosted.hosting;
        let github = match session_id {
            Some(session_id) => {
                if !same(token, &hosting.session_token(session_id)) {
                    return Err("That isn't this session's token".into());
                }
                let project = self.store.read(|state| {
                    let project_id = state.project_id_for_session(session_id)?;
                    state.project(project_id).ok().cloned()
                });
                project
                    .and_then(|p| p.github())
                    .ok_or("This session's project isn't from GitHub")?
            }
            None => lock(&hosted.jobs)
                .get(token)
                .cloned()
                .ok_or("That token isn't one this server gave")?,
        };
        let text = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or("");
        if text("kind") == "gh" {
            let token = hosting.mint(&github, TokenScope::Gh).await?;
            return Ok(json!({ "token": token, "repo": github.full_name }));
        }
        let url = hosting.github_url();
        let (scheme, host) = url.split_once("://").unwrap_or(("https", url.as_str()));
        let host = host.trim_end_matches('/');
        if text("protocol") != scheme || !text("host").eq_ignore_ascii_case(host) {
            return Err(format!("Only {url} is answered"));
        }
        let path = text("path").trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        if !path.eq_ignore_ascii_case(&github.full_name) {
            return Err(format!("Only {} is answered here", github.full_name));
        }
        let token = hosting.mint(&github, TokenScope::Git).await?;
        Ok(json!({ "username": "x-access-token", "password": token }))
    }

    /// SRV-127: whether something listens on each card's preview port, as
    /// the card shows it, looked at every few seconds while this runs.
    pub(crate) async fn watch_previews(core: Weak<Self>) {
        loop {
            tokio::time::sleep(PREVIEW_POLL).await;
            let Some(core) = core.upgrade() else { return };
            let ports: Vec<(String, String, u16, bool)> = core.store.read(|state| {
                state
                    .boards
                    .iter()
                    .flat_map(|(project_id, board)| {
                        board.nodes.values().filter(|n| !n.archived).filter_map(|n| {
                            let port = n.preview_port()?;
                            Some((project_id.clone(), n.session_id.clone(), port, n.preview_live()))
                        })
                    })
                    .collect()
            });
            let mut changed = Vec::new();
            for (project_id, session_id, port, was) in ports {
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                let live = tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(addr))
                    .await
                    .is_ok_and(|connected| connected.is_ok());
                if live != was {
                    changed.push((project_id, session_id, live));
                }
            }
            if changed.is_empty() {
                continue;
            }
            core.store.update(|state| {
                for (project_id, session_id, live) in &changed {
                    if let Some(node) = state.board_mut(project_id).nodes.get_mut(session_id) {
                        node.set_preview_live(*live);
                    }
                }
            });
            let mut told = std::collections::HashSet::new();
            for (project_id, _, _) in changed {
                if told.insert(project_id.clone()) {
                    let _ = core.events.send(Event::BoardChanged { project_id });
                }
            }
        }
    }
}

impl Nodus {
    /// `projects:add-github` (SRV-120), on a server: clones the repository
    /// into `folder` (`<run home>/projects/<slug>/main`) through the
    /// session host with the credential helper, never as the server's own
    /// user, then adds it as a project named `name`, coloured `color`,
    /// that remembers where it came from. Returns once it's cloned. A
    /// repository already added is given back as it is.
    pub async fn add_github_project(
        &self,
        github: ProjectGithub,
        folder: &Path,
        name: &str,
        color: Option<FolderColor>,
    ) -> CommandResult<Project> {
        const CHANNEL: &str = "projects:add-github";
        let fail = |message: String| CommandError::new(CHANNEL, message);
        self.on_core(async {
            let inner = &self.inner;
            let hosted = inner.hosted().ok_or_else(|| fail("This isn't a server".into()))?;
            let known = inner.store.read(|state| {
                state
                    .projects
                    .iter()
                    .find(|p| {
                        p.github()
                            .is_some_and(|g| g.full_name.eq_ignore_ascii_case(&github.full_name))
                    })
                    .cloned()
            });
            if let Some(known) = known {
                return Ok(known);
            }
            let url = format!("{}/{}.git", hosted.hosting.github_url(), github.full_name);
            let folder_text = folder.to_string_lossy().into_owned();
            let (env, job) = inner.job_env(hosted, &github).await;
            inner
                .git_ok(
                    &env,
                    Path::new("/"),
                    &["clone", "--", &url, &folder_text],
                    CLONE_TIMEOUT,
                )
                .await
                .map_err(|err| fail(format!("Can't clone {}: {err}", github.full_name)))?;
            drop(job);
            let (project, _) = self.projects().add_project(&folder_text, Some(name))?;
            let project = inner.store.update(|state| {
                let stored = state.projects.iter_mut().find(|p| p.id == project.id)?;
                stored.set_github(Some(&github));
                if let Some(color) = color {
                    stored.color = color;
                }
                Some(stored.clone())
            });
            self.emit(Event::ProjectsChanged);
            project.ok_or_else(|| fail("The project went as it was added".into()))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_cards_branch_after_its_title_and_id() {
        let id = "0f3a9c2e-7d41-4b8a-9e2f-123456789abc";
        assert_eq!(
            branch_name(Some("Fix the login page!"), None, id),
            "nodus/fix-the-login-page-0f3a"
        );
        assert_eq!(branch_name(Some("  ✨ Ünïcode  "), None, id), "nodus/n-code-0f3a");
        assert_eq!(
            branch_name(None, Some(SessionKind::Terminal), id),
            "nodus/terminal-0f3a"
        );
        assert_eq!(branch_name(Some("---"), None, id), "nodus/session-0f3a");
        let long = branch_name(Some(&"a".repeat(100)), None, id);
        assert_eq!(long, format!("nodus/{}-0f3a", "a".repeat(40)));
    }

    #[test]
    fn gives_the_helper_to_git_and_nothing_else() {
        let env = helper_env("!'/opt/nodus/bin/nodus' git-credential");
        assert_eq!(
            env,
            [
                ("GIT_CONFIG_COUNT", "3"),
                ("GIT_CONFIG_KEY_0", "credential.helper"),
                ("GIT_CONFIG_VALUE_0", ""),
                ("GIT_CONFIG_KEY_1", "credential.helper"),
                ("GIT_CONFIG_VALUE_1", "!'/opt/nodus/bin/nodus' git-credential"),
                ("GIT_CONFIG_KEY_2", "credential.useHttpPath"),
                ("GIT_CONFIG_VALUE_2", "true"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn shows_a_runs_last_line_without_its_colours() {
        assert_eq!(last_line("one\n\x1b[32mtwo\x1b[0m\n\n"), "two");
        assert_eq!(last_line("10%\r50%\r100%\n"), "100%");
        assert_eq!(last_line(""), "");
    }

    #[test]
    fn stamps_the_time_as_a_folder_name() {
        let stamp = time_stamp();
        assert_eq!(stamp.len(), 15);
        assert_eq!(&stamp[8..9], "-");
        assert!(stamp.starts_with("20"));
    }
}
