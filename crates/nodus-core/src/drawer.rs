//! What session cards' drawers show (spec 19): the servers sessions started,
//! looked for every few seconds (DRAWER-021), and their pull requests as
//! GitHub has them, asked every two minutes (DRAWER-014).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use nodus_domain::{CardPr, CardServer, Event, PrChecks, PrRef, PrState, Project, SessionKind};
use nodus_platform::Env;
use nodus_platform::listeners::{Listener, Scanner};
use nodus_platform::runner::RunRequest;
use serde_json::Value;

use crate::nodus::{Inner, lock};

/// How often the servers sessions started are looked for (DRAWER-021).
const SERVER_POLL: Duration = Duration::from_secs(3);
/// How often the pull request watch wakes to see what's due.
const PR_TICK: Duration = Duration::from_secs(15);
/// How often a board's pull requests are asked about (DRAWER-014).
const PR_EVERY: Duration = Duration::from_secs(120);
/// How long to wait after GitHub couldn't be asked (no `gh`, signed out).
const PR_BACKOFF: Duration = Duration::from_secs(600);
/// Branches that are a repository's default often enough not to ask about.
const DEFAULT_BRANCHES: &[&str] = &["main", "master", "trunk", "develop", "HEAD"];

/// The servers each session has, as last looked.
#[derive(Debug, Default)]
pub(crate) struct Drawers {
    servers: Mutex<HashMap<String, Vec<CardServer>>>,
}

impl Drawers {
    pub(crate) fn servers(&self, session_id: &str) -> Vec<CardServer> {
        lock(&self.servers).get(session_id).cloned().unwrap_or_default()
    }

    /// Takes in a new look, and gives the sessions whose servers changed.
    fn replace(&self, found: HashMap<String, Vec<CardServer>>) -> Vec<String> {
        let mut servers = lock(&self.servers);
        let mut changed: Vec<String> = found
            .iter()
            .filter(|(id, now)| servers.get(*id) != Some(now))
            .map(|(id, _)| id.clone())
            .collect();
        changed.extend(servers.keys().filter(|id| !found.contains_key(*id)).cloned());
        *servers = found;
        changed
    }
}

/// What was found, by session, each session's by port.
fn by_session(found: Vec<Listener>) -> HashMap<String, Vec<CardServer>> {
    let mut out: HashMap<String, Vec<CardServer>> = HashMap::new();
    for l in found {
        out.entry(l.session).or_default().push(CardServer {
            port: l.port,
            pid: Some(l.pid),
            label: l.label,
            started_at: Some(l.started_at),
        });
    }
    for servers in out.values_mut() {
        servers.sort_by_key(|s| s.port);
    }
    out
}

impl Inner {
    /// DRAWER-021: the servers sessions started, looked for every few
    /// seconds while the app runs; boards whose cards' servers changed are
    /// told so.
    pub(crate) async fn watch_servers(core: Weak<Self>) {
        let scanner = Arc::new(Mutex::new(Scanner::new()));
        loop {
            tokio::time::sleep(SERVER_POLL).await;
            let Some(core) = core.upgrade() else { return };
            let scan = Arc::clone(&scanner);
            let Ok(found) = tokio::task::spawn_blocking(move || lock(&scan).scan()).await else {
                continue;
            };
            let changed: HashSet<String> = core.drawers.replace(by_session(found)).into_iter().collect();
            if changed.is_empty() {
                continue;
            }
            let projects: Vec<String> = core.store.read(|state| {
                state
                    .boards
                    .iter()
                    .filter(|(_, board)| board.nodes.keys().any(|id| changed.contains(id)))
                    .map(|(project_id, _)| project_id.clone())
                    .collect()
            });
            for project_id in projects {
                let _ = core.events.send(Event::BoardChanged { project_id });
            }
        }
    }

    /// DRAWER-014: every board's pull requests, as GitHub has them, asked
    /// every two minutes, and soon after a card shows one GitHub hasn't been
    /// asked about yet.
    pub(crate) async fn watch_prs(core: Weak<Self>) {
        // By project: when to ask next, and whether asking last worked.
        let mut next: HashMap<String, (Instant, bool)> = HashMap::new();
        // The links GitHub has been asked about, so one it can't find isn't
        // asked about again at once.
        let mut asked: HashSet<String> = HashSet::new();
        // Which repository each folder's `origin` is.
        let mut origins: HashMap<String, Option<Repo>> = HashMap::new();
        loop {
            tokio::time::sleep(PR_TICK).await;
            let Some(core) = core.upgrade() else { return };
            let projects: Vec<(Project, bool)> = core.store.read(|state| {
                state
                    .projects
                    .iter()
                    .filter(|p| p.server.is_none() && p.archived != Some(true))
                    .filter_map(|p| {
                        let board = state.boards.get(&p.id)?;
                        let mut active = board.nodes.values().filter(|n| is_claude_card(n)).peekable();
                        active.peek()?;
                        let fresh =
                            active.any(|n| n.prs().iter().any(|pr| pr.state.is_none() && !asked.contains(&pr.url)));
                        Some((p.clone(), fresh))
                    })
                    .collect()
            });
            let now = Instant::now();
            for (project, fresh) in projects {
                let due = match next.get(&project.id) {
                    None => true,
                    Some(&(at, ok)) => now >= at || (fresh && ok),
                };
                if !due {
                    continue;
                }
                let (ok, links) = core.refresh_prs(&project, &mut origins).await;
                asked.extend(links);
                let wait = if ok { PR_EVERY } else { PR_BACKOFF };
                next.insert(project.id.clone(), (Instant::now() + wait, ok));
            }
        }
    }

    /// Asks GitHub about a board's pull requests: the ones its cards keep,
    /// and the open one on each card's branch (DRAWER-012), then keeps what
    /// changed. Whether GitHub could be asked, and the links it was asked
    /// about.
    async fn refresh_prs(
        self: &Arc<Self>,
        project: &Project,
        origins: &mut HashMap<String, Option<Repo>>,
    ) -> (bool, Vec<String>) {
        let nodes: Vec<(String, Vec<CardPr>, Option<nodus_domain::Worktree>)> = self.store.read(|state| {
            state.boards.get(&project.id).map_or_else(Vec::new, |board| {
                board
                    .nodes
                    .values()
                    .filter(|n| is_claude_card(n))
                    .map(|n| (n.session_id.clone(), n.prs(), n.worktree()))
                    .collect()
            })
        });
        // Each card's branch, from its own transcript only (a project's
        // other sessions are never read for this).
        let transcripts = Arc::clone(self.board.transcripts());
        let repo_path = project.repo_path.clone();
        let cards: Vec<Want> = tokio::task::spawn_blocking(move || {
            nodes
                .into_iter()
                .map(|(session_id, prs, worktree)| {
                    let folder = worktree.as_ref().map_or_else(|| repo_path.clone(), |w| w.path.clone());
                    let branch = transcripts
                        .session(&folder, &session_id)
                        .and_then(|f| f.meta.git_branch)
                        .or_else(|| worktree.map(|w| w.branch))
                        .filter(|b| !b.is_empty() && !DEFAULT_BRANCHES.contains(&b.as_str()));
                    Want {
                        session_id,
                        prs,
                        branch,
                        folder,
                    }
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        let links: Vec<String> = cards.iter().flat_map(|c| c.prs.iter().map(|p| p.url.clone())).collect();
        if cards.iter().all(|c| c.prs.is_empty() && c.branch.is_none()) {
            return (true, links);
        }
        let env = self.env().await;
        for card in cards.iter().filter(|c| c.branch.is_some()) {
            if !origins.contains_key(&card.folder) {
                let origin = self.origin(&env, &card.folder).await;
                origins.insert(card.folder.clone(), origin);
            }
        }
        // One question to each repository.
        let mut asks: BTreeMap<Repo, Ask> = BTreeMap::new();
        for card in &cards {
            for pr in &card.prs {
                if let Some(at) = PrRef::parse(&pr.url) {
                    asks.entry(Repo::of(&at)).or_default().numbers.insert(at.number);
                }
            }
            if let (Some(branch), Some(Some(repo))) = (&card.branch, origins.get(&card.folder)) {
                asks.entry(repo.clone()).or_default().branches.insert(branch.clone());
            }
        }
        let mut answers: HashMap<Repo, Answer> = HashMap::new();
        let mut ok = true;
        for (repo, ask) in &asks {
            match self.ask_github(&env, repo, ask).await {
                Ok(answer) => {
                    answers.insert(repo.clone(), answer);
                }
                Err(err) => {
                    tracing::warn!("[drawer] couldn't ask GitHub about {}/{}: {err}", repo.owner, repo.name);
                    ok = false;
                }
            }
        }
        if answers.is_empty() {
            return (ok, links);
        }
        let changed = self.store.update(|state| {
            let board = state.board_mut(&project.id);
            let mut changed = false;
            for card in &cards {
                let Some(node) = board.nodes.get_mut(&card.session_id).filter(|n| !n.archived) else {
                    continue;
                };
                let origin = origins.get(&card.folder).cloned().flatten();
                let prs = updated(node.prs(), card.branch.as_deref(), origin.as_ref(), &answers);
                if prs != node.prs() {
                    node.set_prs(&prs);
                    changed = true;
                }
            }
            changed
        });
        if changed {
            let _ = self.events.send(Event::BoardChanged {
                project_id: project.id.clone(),
            });
        }
        (ok, links)
    }

    /// The GitHub repository a folder's `origin` is, if it is one.
    async fn origin(&self, env: &Env, folder: &str) -> Option<Repo> {
        let request = RunRequest::new("git")
            .args(["remote", "get-url", "origin"])
            .cwd(folder)
            .env(env.spawn_env())
            .timeout(Duration::from_secs(10));
        let output = self.runner.run(request).await.ok()?;
        Repo::from_remote(output.stdout_text().trim())
    }

    /// One GraphQL question through `gh` (DRAWER-014).
    async fn ask_github(&self, env: &Env, repo: &Repo, ask: &Ask) -> Result<Answer, String> {
        let (query, branches) = ask.query();
        let mut args = vec![
            "api".to_owned(),
            "graphql".into(),
            "--hostname".into(),
            repo.host.clone(),
            "-f".into(),
            format!("query={query}"),
            "-f".into(),
            format!("owner={}", repo.owner),
            "-f".into(),
            format!("name={}", repo.name),
        ];
        for (var, branch) in branches {
            args.push("-f".into());
            args.push(format!("{var}={branch}"));
        }
        let request = RunRequest::new("gh")
            .args(args)
            .env(env.spawn_env())
            .timeout(Duration::from_secs(30));
        let output = self.runner.run(request).await.map_err(|err| err.to_string())?;
        let text = output.stdout_text();
        // A number GitHub doesn't know fails only its own part.
        let json: Value = serde_json::from_str(&text)
            .map_err(|_| format!("gh answered {:?}", text.chars().take(200).collect::<String>()))?;
        Answer::read(&json, ask).ok_or_else(|| {
            let message = json
                .pointer("/errors/0/message")
                .and_then(Value::as_str)
                .unwrap_or("no repository in the answer");
            message.to_owned()
        })
    }
}

/// An active Claude card: one with a transcript, that the drawer shows.
fn is_claude_card(node: &nodus_domain::BoardNode) -> bool {
    !node.archived && node.kind != Some(SessionKind::Terminal) && node.view_of.is_none() && node.run_of.is_none()
}

/// A card to ask about.
#[derive(Debug, Clone)]
struct Want {
    session_id: String,
    prs: Vec<CardPr>,
    branch: Option<String>,
    folder: String,
}

/// A repository on GitHub, or on a GitHub Enterprise host.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Repo {
    host: String,
    owner: String,
    name: String,
}

impl Repo {
    fn of(at: &PrRef) -> Self {
        Self {
            host: at.host.clone(),
            owner: at.owner.clone(),
            name: at.name.clone(),
        }
    }

    /// A remote's URL: `git@github.com:o/n.git`, `https://github.com/o/n`,
    /// `ssh://git@host/o/n.git`. An SSH alias for GitHub
    /// (`github.com-work`) is GitHub.
    fn from_remote(url: &str) -> Option<Self> {
        let url = url.trim();
        let rest = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
            rest.to_owned()
        } else {
            // scp-like: [user@]host:path
            let (host, path) = url.split_once(':')?;
            format!("{host}/{path}")
        };
        let rest = rest.rsplit_once('@').map_or(rest.as_str(), |(_, r)| r);
        let mut parts = rest.split('/');
        let host = parts.next()?.split(':').next()?.to_ascii_lowercase();
        let owner = parts.next().filter(|s| !s.is_empty())?;
        let name = parts.next().filter(|s| !s.is_empty())?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        let host = if host.contains("github.com") {
            "github.com".to_owned()
        } else {
            host
        };
        Some(Self {
            host,
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// What to ask one repository: pull requests by number, and the open one on
/// each branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ask {
    numbers: BTreeSet<u64>,
    branches: BTreeSet<String>,
}

impl Ask {
    /// The GraphQL query, and its branch variables by name.
    fn query(&self) -> (String, Vec<(String, String)>) {
        let vars: Vec<(String, String)> = self
            .branches
            .iter()
            .enumerate()
            .map(|(i, b)| (format!("b{i}"), b.clone()))
            .collect();
        let mut q = String::from("query($owner: String!, $name: String!");
        for (var, _) in &vars {
            q.push_str(&format!(", ${var}: String!"));
        }
        q.push_str(") { repository(owner: $owner, name: $name) { defaultBranchRef { name } ");
        for n in &self.numbers {
            q.push_str(&format!("p{n}: pullRequest(number: {n}) {{ ...pr }} "));
        }
        for (var, _) in &vars {
            q.push_str(&format!(
                "{var}: pullRequests(headRefName: ${var}, states: [OPEN], first: 1, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ ...pr }} }} "
            ));
        }
        q.push_str(
            "} } fragment pr on PullRequest { number title url state isDraft \
             commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } }",
        );
        (q, vars)
    }
}

/// A pull request as GitHub described it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fetched {
    url: String,
    title: String,
    state: PrState,
    checks: Option<PrChecks>,
}

impl Fetched {
    fn read(pr: &Value) -> Option<Self> {
        let draft = pr.get("isDraft").and_then(Value::as_bool) == Some(true);
        let state = match pr.get("state")?.as_str()? {
            "OPEN" if draft => PrState::Draft,
            "OPEN" => PrState::Open,
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ => return None,
        };
        let checks = match pr
            .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
            .and_then(Value::as_str)
        {
            Some("SUCCESS") => Some(PrChecks::Passed),
            Some("FAILURE" | "ERROR") => Some(PrChecks::Failed),
            Some("PENDING" | "EXPECTED") => Some(PrChecks::Pending),
            _ => None,
        };
        Some(Self {
            url: PrRef::parse(pr.get("url")?.as_str()?)?.url(),
            title: pr.get("title")?.as_str()?.to_owned(),
            state,
            checks,
        })
    }

    fn into_pr(self, number: u64) -> CardPr {
        CardPr {
            url: self.url,
            number,
            title: self.title,
            state: Some(self.state),
            checks: self.checks,
        }
    }
}

/// One repository's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Answer {
    default_branch: Option<String>,
    prs: HashMap<u64, Fetched>,
    /// The open pull request on each branch that has one.
    branches: HashMap<String, (u64, Fetched)>,
}

impl Answer {
    fn read(json: &Value, ask: &Ask) -> Option<Self> {
        let repo = json.pointer("/data/repository").filter(|r| r.is_object())?;
        let mut answer = Self {
            default_branch: repo
                .pointer("/defaultBranchRef/name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ..Self::default()
        };
        for n in &ask.numbers {
            if let Some(pr) = repo.get(format!("p{n}")).and_then(Fetched::read) {
                answer.prs.insert(*n, pr);
            }
        }
        for (i, branch) in ask.branches.iter().enumerate() {
            let pr = repo.pointer(&format!("/b{i}/nodes/0"));
            if let Some(pr) = pr
                && let Some(n) = pr.get("number").and_then(Value::as_u64)
                && let Some(fetched) = Fetched::read(pr)
            {
                answer.branches.insert(branch.clone(), (n, fetched));
            }
        }
        Some(answer)
    }
}

/// A card's pull requests with GitHub's answers taken in: each kept one
/// described afresh, and the open one on its branch added (DRAWER-012).
fn updated(
    mut prs: Vec<CardPr>,
    branch: Option<&str>,
    origin: Option<&Repo>,
    answers: &HashMap<Repo, Answer>,
) -> Vec<CardPr> {
    for pr in &mut prs {
        let Some(at) = PrRef::parse(&pr.url) else { continue };
        if let Some(fetched) = answers.get(&Repo::of(&at)).and_then(|a| a.prs.get(&at.number)) {
            *pr = fetched.clone().into_pr(at.number);
        }
    }
    if let (Some(branch), Some(origin)) = (branch, origin)
        && let Some(answer) = answers.get(origin)
        && answer.default_branch.as_deref() != Some(branch)
        && let Some((n, fetched)) = answer.branches.get(branch)
        && !prs.iter().any(|p| p.url == fetched.url)
    {
        prs.push(fetched.clone().into_pr(*n));
    }
    prs
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn repo(owner: &str, name: &str) -> Repo {
        Repo {
            host: "github.com".into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn reads_remotes() {
        for url in [
            "git@github.com:saleem-hadad/nodus.git",
            "https://github.com/saleem-hadad/nodus",
            "https://github.com/saleem-hadad/nodus.git\n",
            "ssh://git@github.com/saleem-hadad/nodus.git",
            "https://token@github.com/saleem-hadad/nodus.git",
            "git@github.com-work:saleem-hadad/nodus.git",
            "ssh://git@github.com:22/saleem-hadad/nodus.git",
        ] {
            assert_eq!(Repo::from_remote(url), Some(repo("saleem-hadad", "nodus")), "{url}");
        }
        assert_eq!(
            Repo::from_remote("git@ghe.acme.io:team/app.git").unwrap().host,
            "ghe.acme.io"
        );
        assert_eq!(Repo::from_remote(""), None);
        assert_eq!(Repo::from_remote("/Users/me/bare.git"), None);
    }

    #[test]
    fn asks_by_number_and_by_branch_variable() {
        let ask = Ask {
            numbers: [53, 7].into(),
            branches: ["fix/a\"b".to_owned()].into(),
        };
        let (q, vars) = ask.query();
        assert!(q.contains("p7: pullRequest(number: 7) { ...pr }"), "{q}");
        assert!(q.contains("p53: pullRequest(number: 53)"), "{q}");
        assert!(q.contains("$b0: String!"), "{q}");
        assert!(q.contains("b0: pullRequests(headRefName: $b0, states: [OPEN]"), "{q}");
        // The branch's name goes as a variable, never into the query.
        assert!(!q.contains("fix/a"), "{q}");
        assert_eq!(vars, [("b0".to_owned(), "fix/a\"b".to_owned())]);
    }

    fn pr(n: u64, state: &str, draft: bool, rollup: Option<&str>) -> Value {
        json!({
            "number": n, "title": format!("PR {n}"), "url": format!("https://github.com/o/r/pull/{n}"),
            "state": state, "isDraft": draft,
            "commits": {"nodes": [{"commit": {"statusCheckRollup": rollup.map(|s| json!({"state": s}))}}]}
        })
    }

    #[test]
    fn reads_an_answer() {
        let ask = Ask {
            numbers: [1, 2, 3, 9].into(),
            branches: ["feat".to_owned(), "none".to_owned()].into(),
        };
        let json = json!({"data": {"repository": {
            "defaultBranchRef": {"name": "main"},
            "p1": pr(1, "OPEN", false, Some("PENDING")),
            "p2": pr(2, "MERGED", false, Some("SUCCESS")),
            "p3": pr(3, "OPEN", true, None),
            "p9": null,
            "b0": {"nodes": [pr(4, "OPEN", false, Some("FAILURE"))]},
            "b1": {"nodes": []},
        }}, "errors": [{"message": "Could not resolve to a PullRequest with the number of 9."}]});
        let a = Answer::read(&json, &ask).unwrap();
        assert_eq!(a.default_branch.as_deref(), Some("main"));
        assert_eq!(
            (a.prs[&1].state, a.prs[&1].checks),
            (PrState::Open, Some(PrChecks::Pending))
        );
        assert_eq!(
            (a.prs[&2].state, a.prs[&2].checks),
            (PrState::Merged, Some(PrChecks::Passed))
        );
        assert_eq!((a.prs[&3].state, a.prs[&3].checks), (PrState::Draft, None));
        assert!(!a.prs.contains_key(&9));
        assert_eq!(a.branches["feat"].0, 4);
        assert_eq!(a.branches["feat"].1.checks, Some(PrChecks::Failed));
        assert!(!a.branches.contains_key("none"));
        assert_eq!(
            Answer::read(&json!({"errors": [{"message": "Bad credentials"}]}), &ask),
            None
        );
    }

    #[test]
    fn takes_answers_in() {
        let origin = repo("o", "r");
        let ask = Ask {
            numbers: [1].into(),
            branches: ["feat".to_owned()].into(),
        };
        let json = json!({"data": {"repository": {
            "defaultBranchRef": {"name": "main"},
            "p1": pr(1, "MERGED", false, Some("SUCCESS")),
            "b0": {"nodes": [pr(4, "OPEN", false, None)]},
        }}});
        let answers = HashMap::from([(origin.clone(), Answer::read(&json, &ask).unwrap())]);
        let kept = vec![CardPr::found("https://github.com/o/r/pull/1").unwrap()];
        let prs = updated(kept, Some("feat"), Some(&origin), &answers);
        assert_eq!(
            prs.iter()
                .map(|p| (p.number, p.title.as_str(), p.state))
                .collect::<Vec<_>>(),
            [(1, "PR 1", Some(PrState::Merged)), (4, "PR 4", Some(PrState::Open))]
        );
        // Asked again: nothing new joins, nothing changes.
        assert_eq!(updated(prs.clone(), Some("feat"), Some(&origin), &answers), prs);
        // Another repository's pull request is left as it was.
        let other = vec![CardPr::found("https://github.com/x/y/pull/1").unwrap()];
        assert_eq!(updated(other.clone(), None, None, &answers), other);
    }

    #[test]
    fn a_new_look_says_which_sessions_changed() {
        let drawers = Drawers::default();
        let server = |port| CardServer {
            port,
            pid: Some(1),
            label: "vite".into(),
            started_at: Some(0),
        };
        let mut changed = drawers.replace(HashMap::from([("a".into(), vec![server(5173)])]));
        assert_eq!(changed, ["a"]);
        assert!(
            drawers
                .replace(HashMap::from([("a".into(), vec![server(5173)])]))
                .is_empty()
        );
        changed = drawers.replace(HashMap::from([("b".into(), vec![server(3000)])]));
        changed.sort();
        assert_eq!(changed, ["a", "b"]);
        assert_eq!(drawers.servers("b"), [server(3000)]);
        assert!(drawers.servers("a").is_empty());
    }
}
