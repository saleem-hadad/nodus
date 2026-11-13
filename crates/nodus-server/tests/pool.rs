//! The pool of Claude accounts (SRV-100…105), on the real binary over its
//! WebSocket, as tests/serve.rs runs it: loopback, a dev token, a scratch
//! data folder and a stand-in `claude` on the env file's PATH.
//!
//! The stand-in answers `claude -p` (the token check and background jobs)
//! and, as a session, does what Claude Code does with Nodus's hooks: it
//! reports its account's `rate_limits` through the status line and a
//! failed turn through `StopFailure`, posting to the session host with its
//! own token from `$NODUS_HOOK_CONFIG`. Typed lines tell it what to do:
//! `limit` fails a turn with `rate_limit`, `authfail` with
//! `authentication_failed`, `usage <n>` reports n% of the five hours, and
//! `fake <session id>` reports without its own token, or for another
//! session. It never prints its token. The real claude is never run.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodus_domain::{BoardNode, BoardSnapshot, Event, OpenSessionResult, Project, SessionCard};
use nodus_wire::client::{Client, Events, Received};
use nodus_wire::{AccountState, ClaudeAccount, Notice, Request};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(25);
const TOKEN: &str = "a-dev-token";
const ALPHA: &str = "tok-alpha";
const BRAVO: &str = "tok-bravo";
/// How long after it starts an Alpha session says its five hours reset.
const ALPHA_RESET_SECS: u64 = 8;

// ---------------------------------------------------------------------------
// The world a test runs in.

struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        // Short, so the host's socket path fits macOS's 104 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nodus-pool-")
            .tempdir_in("/tmp")
            .unwrap();
        let world = Self { dir };
        let root = world.root();
        for folder in ["home", "bin", "log", "data"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        write_stand_in(&root.join("bin/claude"), &root.join("log"));
        std::fs::write(
            world.data().join("env"),
            format!(
                "PATH={}/bin:/usr/bin:/bin\nHOME=/nowhere\nSHELL=/bin/sh\nLANG=en_US.UTF-8\n",
                root.display()
            ),
        )
        .unwrap();
        world
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn data(&self) -> PathBuf {
        self.root().join("data")
    }

    fn log(&self) -> PathBuf {
        self.root().join("log")
    }

    fn serve(&self) -> Server {
        let home = self.root().join("home");
        let mut child = Command::new(env!("CARGO_BIN_EXE_nodus-server"))
            .args(["serve", "--bind", "127.0.0.1:0", "--dev-token", TOKEN, "--run-home"])
            .arg(&home)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", &home)
            .env("NODUS_USER_DATA", self.data())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let (listening, url) = std::sync::mpsc::channel();
        for stream in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let kept = Arc::clone(&output);
            let listening = listening.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(at) = line.find("listening http://") {
                        let _ = listening.send(line[at + "listening ".len()..].to_owned());
                    }
                    let mut kept = kept.lock().unwrap();
                    kept.push_str(&line);
                    kept.push('\n');
                }
            });
        }
        let url = url
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the server didn't listen: {}", output.lock().unwrap()));
        Server { child, url, output }
    }

    /// A project on the server: a clone of a one-commit repository.
    async fn project(&self, server: &Server) -> Project {
        let work = self.root().join("shop-work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("README.md"), "# Shop\n").unwrap();
        git(&work, &["init", "-q"]);
        git(&work, &["add", "README.md"]);
        git(
            &work,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "First",
            ],
        );
        let folder = self.root().join("projects/shop/main");
        git(
            &self.root(),
            &["clone", "-q", &work.to_string_lossy(), &folder.to_string_lossy()],
        );
        let (client, _events) = Client::connect(&server.url, TOKEN).await.unwrap();
        let added = client
            .call_as(&Request::ProjectsAdd {
                repo_path: folder.to_string_lossy().into_owned(),
                name: Some("shop".into()),
            })
            .await
            .unwrap();
        client.close();
        added
    }

    /// Every start of a session's process, oldest first: its arguments and
    /// environment as the stand-in wrote them down.
    fn starts(&self, session_id: &str) -> Vec<(Vec<String>, BTreeMap<String, String>)> {
        let mut starts = Vec::new();
        for n in 1.. {
            let args = self.log().join(format!("args.{session_id}.{n}"));
            let Ok(args) = std::fs::read_to_string(&args) else {
                break;
            };
            let env = std::fs::read_to_string(self.log().join(format!("env.{session_id}.{n}"))).unwrap_or_default();
            starts.push((args.lines().map(str::to_owned).collect(), parse_env(&env)));
        }
        starts
    }

    /// Every background `claude -p` run's environment.
    fn background_envs(&self) -> Vec<BTreeMap<String, String>> {
        let mut envs = Vec::new();
        for entry in std::fs::read_dir(self.log()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.starts_with("p-env.") {
                envs.push(parse_env(&std::fs::read_to_string(self.log().join(name)).unwrap()));
            }
        }
        envs
    }
}

impl Drop for World {
    /// The session host outlives its server, as it should; here it goes too.
    fn drop(&mut self) {
        let _ = Command::new("pkill")
            .args(["-f", &self.root().to_string_lossy()])
            .status();
        if std::env::var_os("POOL_KEEP").is_some() {
            eprintln!("kept {}", self.root().display());
            let dir = std::mem::replace(&mut self.dir, tempfile::tempdir().unwrap());
            let _ = dir.keep();
        }
    }
}

fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The stand-in `claude` (see the top of this file).
fn write_stand_in(file: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = r#"#!/bin/sh
LOG="@LOG@"
if [ "$1" = "--version" ]; then echo "9.9.9 (Claude Code)"; exit 0; fi
if [ "$1" = "-p" ]; then
  env > "$LOG/p-env.$$"
  printf '%s\n' "$@" > "$LOG/p-args.$$"
  if [ "$2" = "Reply OK" ]; then
    case "$CLAUDE_CODE_OAUTH_TOKEN" in
      bad*) echo '{"type":"result","is_error":true,"result":"Invalid bearer token"}'; exit 1;;
    esac
    echo '{"type":"result","is_error":false,"result":"OK"}'
    exit 0
  fi
  echo '{"type":"result","is_error":false,"result":"","structured_output":{"name":"Run Tests","icon":"test-tube","prompt":"Run them."}}'
  exit 0
fi

n=1
while [ -e "$LOG/args.$NODUS_SESSION.$n" ]; do n=$((n+1)); done
env > "$LOG/env.$NODUS_SESSION.$n"
printf '%s\n' "$@" > "$LOG/args.$NODUS_SESSION.$n"

# A transcript, as Claude Code keeps one, so the next start resumes it.
dir="$HOME/.claude/projects/$(pwd -P | sed 's/[^A-Za-z0-9]/-/g')"
mkdir -p "$dir"
echo "{\"type\":\"user\",\"sessionId\":\"$NODUS_SESSION\",\"cwd\":\"$(pwd -P)\",\"timestamp\":\"2026-11-13T10:00:00.000Z\",\"uuid\":\"u$n\",\"message\":{\"role\":\"user\",\"content\":\"look\"}}" >> "$dir/$NODUS_SESSION.jsonl"

post() { # event, body, session, then curl's own options
  event=$1; body=$2; sid=$3; shift 3
  printf '%s' "$body" | curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- "$@" "http://127.0.0.1:$NODUS_PORT/hook?evt=$event&sid=$sid" >/dev/null 2>&1
}
now=$(date +%s)
case "$CLAUDE_CODE_OAUTH_TOKEN" in
  tok-alpha) reset=$((now + @ALPHA_RESET@)) ;;
  *) reset=$((now + 60)) ;;
esac
limits() {
  echo "{\"model\":{\"display_name\":\"Stand-in\"},\"rate_limits\":{\"five_hour\":{\"used_percentage\":$1,\"resets_at\":$reset},\"seven_day\":{\"used_percentage\":40,\"resets_at\":$((now + 86400))}}}"
}

printf '\033[?2004h'
prompt=""
seen=""
for arg in "$@"; do
  if [ "$seen" = "--" ]; then prompt="$arg"; fi
  [ "$arg" = "--" ] && seen="--"
done
echo "stand-in ready on ${CLAUDE_CODE_OAUTH_TOKEN#tok-}"
[ -n "$prompt" ] && echo "prompt: $prompt"
post StatusLine "$(limits 100)" "$NODUS_SESSION" -K "$NODUS_HOOK_CONFIG"
echo "reported"

while IFS= read -r line; do
  line=$(printf '%s' "$line" | tr -d '\r')
  case "$line" in
    limit)
      post StopFailure '{"hook_event_name":"StopFailure","error":"rate_limit"}' "$NODUS_SESSION" -K "$NODUS_HOOK_CONFIG"
      echo "hit the limit" ;;
    authfail)
      post StopFailure '{"hook_event_name":"StopFailure","error":"authentication_failed"}' "$NODUS_SESSION" -K "$NODUS_HOOK_CONFIG"
      echo "turned down" ;;
    "usage "*)
      post StatusLine "$(limits ${line#usage })" "$NODUS_SESSION" -K "$NODUS_HOOK_CONFIG"
      echo "reported ${line#usage }" ;;
    "fake "*)
      other=${line#fake }
      fake=$(limits 77)
      # No token at all.
      post StatusLine "$fake" "$NODUS_SESSION"
      post StopFailure '{"error":"rate_limit"}' "$NODUS_SESSION"
      # A token that isn't the session's own.
      post StatusLine "$fake" "$NODUS_SESSION" -H "Authorization: Bearer not-the-sessions-token"
      post StopFailure '{"error":"authentication_failed"}' "$NODUS_SESSION" -H "Authorization: Bearer not-the-sessions-token"
      # Its own token, for another session.
      post StatusLine "$fake" "$other" -K "$NODUS_HOOK_CONFIG"
      post StopFailure '{"error":"rate_limit"}' "$other" -K "$NODUS_HOOK_CONFIG"
      post StopFailure '{"error":"authentication_failed"}' "$other" -K "$NODUS_HOOK_CONFIG"
      echo "faked" ;;
    *) echo "got: $line" ;;
  esac
done
"#
    .replace("@LOG@", &log.to_string_lossy())
    .replace("@ALPHA_RESET@", &ALPHA_RESET_SECS.to_string());
    std::fs::write(file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Server {
    child: Child,
    url: String,
    output: Arc<Mutex<String>>,
}

impl Server {
    async fn connect(&self) -> App {
        let (client, events) = Client::connect(&self.url, TOKEN).await.unwrap();
        App {
            client,
            events,
            output: BTreeMap::new(),
            seen: Vec::new(),
            notices: Vec::new(),
            answers: Vec::new(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// An app connected to the server, keeping everything it was sent.

struct App {
    client: Client,
    events: Events,
    /// Output so far, by session.
    output: BTreeMap<String, String>,
    seen: Vec<Event>,
    notices: Vec<Notice>,
    /// Every answer, as the wire carried it, failures too.
    answers: Vec<String>,
}

impl App {
    async fn call(&mut self, request: Request) -> Result<Value, String> {
        let answer = tokio::time::timeout(Duration::from_secs(120), self.client.call(&request))
            .await
            .expect("an answer in time");
        let answer = answer.map_err(|err| err.message);
        self.answers.push(match &answer {
            Ok(value) => nodus_domain::json::to_string(value).unwrap(),
            Err(message) => message.clone(),
        });
        answer
    }

    async fn ok<T: serde::de::DeserializeOwned>(&mut self, request: Request) -> T {
        let name = request.name();
        let value = self.call(request).await.unwrap_or_else(|err| panic!("{name}: {err}"));
        serde_json::from_value(value).unwrap()
    }

    async fn accounts(&mut self) -> Vec<ClaudeAccount> {
        self.ok(Request::AccountsList {}).await
    }

    async fn account(&mut self, label: &str) -> ClaudeAccount {
        self.accounts()
            .await
            .into_iter()
            .find(|a| a.label == label)
            .unwrap_or_else(|| panic!("no account {label}"))
    }

    async fn add(&mut self, label: &str, token: &str) -> ClaudeAccount {
        self.ok(Request::AccountsAdd {
            label: label.into(),
            token: token.into(),
        })
        .await
    }

    async fn card(&mut self, project_id: &str, session_id: &str) -> BoardNode {
        let board: BoardSnapshot = self
            .ok(Request::BoardLoad {
                project_id: project_id.into(),
            })
            .await;
        board
            .cards
            .into_iter()
            .find(|card| card.node.session_id == session_id)
            .map(|card| card.node)
            .unwrap_or_else(|| panic!("no card {session_id}"))
    }

    /// A Claude card started and opened here.
    async fn start(&mut self, project_id: &str) -> Result<String, String> {
        let card = self
            .call(Request::SessionCreate {
                project_id: project_id.into(),
                opts: serde_json::from_value(json!({ "x": 0, "y": 0, "name": "Look", "initialPrompt": "look" }))
                    .unwrap(),
            })
            .await?;
        let card: SessionCard = serde_json::from_value(card).unwrap();
        let id = card.node.session_id.clone();
        let opened: OpenSessionResult = self
            .ok(Request::SessionOpen {
                project_id: project_id.into(),
                session_id: id.clone(),
                cols: 100.0,
                rows: 30.0,
            })
            .await;
        self.output.insert(id.clone(), opened.replay);
        Ok(id)
    }

    async fn type_line(&mut self, session_id: &str, line: &str) {
        self.ok::<Value>(Request::SessionWrite {
            session_id: session_id.into(),
            data: format!("{line}\r"),
        })
        .await;
    }

    fn keep(&mut self, received: Received) {
        match received {
            Received::Event(event) => {
                if let Event::PtyData { session_id, data } = &event {
                    self.output.entry(session_id.clone()).or_default().push_str(data);
                }
                self.seen.push(event);
            }
            Received::Notice(notice) => self.notices.push(notice),
        }
    }

    /// Takes in what has arrived, for up to `wait`.
    async fn pump(&mut self, wait: Duration) {
        if let Ok(Some(received)) = tokio::time::timeout(wait, self.events.next_received()).await {
            self.keep(received);
        }
        while let Ok(Some(received)) = tokio::time::timeout(Duration::ZERO, self.events.next_received()).await {
            self.keep(received);
        }
    }

    /// Waits until a session has printed `text` `times` times.
    async fn printed(&mut self, session_id: &str, text: &str, times: usize) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while self.output.get(session_id).map_or(0, |out| out.matches(text).count()) < times {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{session_id} never printed {text:?} {times} times: {:?}",
                self.output.get(session_id)
            );
            self.pump(Duration::from_millis(200)).await;
        }
    }

    /// Asks again, taking in events meanwhile, until `check` says yes.
    async fn until<T>(&mut self, what: &str, mut check: impl AsyncFnMut(&mut Self) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if let Some(found) = check(self).await {
                return found;
            }
            assert!(tokio::time::Instant::now() < deadline, "never: {what}");
            self.pump(Duration::from_millis(150)).await;
        }
    }

    /// SRV-100: nothing the server sent this app holds a token.
    fn sent_no_token(&self, tokens: &[&str]) {
        let mut sent: Vec<String> = self.answers.clone();
        sent.extend(self.seen.iter().map(|e| nodus_domain::json::to_string(e).unwrap()));
        sent.extend(self.notices.iter().map(|n| nodus_domain::json::to_string(n).unwrap()));
        assert!(sent.len() > 10, "{sent:?}");
        for text in &sent {
            for token in tokens {
                assert!(!text.contains(token), "{token} went out in {text}");
            }
        }
    }
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_moves_off_an_account_at_its_limit_waits_and_comes_back() {
    let world = World::new();
    let server = world.serve();
    let project = world.project(&server).await;
    let mut app = server.connect().await;
    app.ok::<Value>(Request::Watch {
        projects: vec![project.id.clone()],
    })
    .await;

    // SRV-101: with no account, nothing starts.
    let err = app.start(&project.id).await.unwrap_err();
    assert_eq!(err, "No Claude accounts yet. Add one in Settings → Servers.");

    // SRV-100: each token is tried, as nodus-run would, and kept 0600.
    let alpha = app.add("Alpha", ALPHA).await;
    let bravo = app.add("Bravo", BRAVO).await;
    assert_eq!((alpha.state, bravo.state), (AccountState::Ok, AccountState::Ok));
    let checks: Vec<_> = world
        .background_envs()
        .into_iter()
        .filter_map(|env| env.get("CLAUDE_CODE_OAUTH_TOKEN").cloned())
        .collect();
    assert_eq!(checks.len(), 2, "{checks:?}");
    {
        use std::os::unix::fs::PermissionsExt;
        let file = world.data().join("claude-accounts.json");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_to_string(&file).unwrap().contains(ALPHA));
    }
    app.until("members hear the pool changed", async |app| {
        app.seen
            .iter()
            .any(|e| matches!(e, Event::AccountsChanged { .. }))
            .then_some(())
    })
    .await;

    // Neither used yet, and no usage known: the first added.
    let id = app.start(&project.id).await.unwrap();
    app.printed(&id, "stand-in ready on alpha", 1).await;
    let card = app.card(&project.id, &id).await;
    assert_eq!(card.account().as_deref(), Some(alpha.id.as_str()));
    assert_eq!(card.account_label().as_deref(), Some("Alpha"));
    let starts = world.starts(&id);
    assert_eq!(
        starts[0].1.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str),
        Some(ALPHA)
    );
    assert!(starts[0].0.contains(&"--session-id".to_owned()), "{:?}", starts[0].0);
    // The StopFailure hook is in its settings, after the others (SRV-103).
    let settings = starts[0].0.iter().find(|a| a.starts_with("{\"hooks\"")).unwrap();
    let settings: Value = serde_json::from_str(settings).unwrap();
    let hooks: Vec<&String> = settings["hooks"].as_object().unwrap().keys().collect();
    assert_eq!(hooks.last().map(|h| h.as_str()), Some("StopFailure"));

    // SRV-102: its status line reports Alpha's usage.
    let alpha_now = app
        .until("Alpha's usage arrives", async |app| {
            let a = app.account("Alpha").await;
            a.five_hour.is_some().then_some(a)
        })
        .await;
    let five = alpha_now.five_hour.unwrap();
    assert_eq!(five.used_percentage, 100.0);
    let alpha_reset = five.resets_at.unwrap();
    assert_eq!(alpha_now.seven_day.unwrap().used_percentage, 40.0);
    assert_eq!(alpha_now.sessions, 1);
    assert!(alpha_now.last_used_at.is_some());

    // SRV-103, SRV-104: the turn hits Alpha's limit; the session stops and
    // resumes on Bravo, told to carry on.
    app.type_line(&id, "limit").await;
    app.printed(&id, "stand-in ready on bravo", 1).await;
    app.printed(&id, "prompt: Continue.", 1).await;
    let moved = app
        .until("the card says where it went", async |app| {
            let card = app.card(&project.id, &id).await;
            (card.account_note().as_deref() == Some("Moved to Bravo")).then_some(card)
        })
        .await;
    assert_eq!(moved.account().as_deref(), Some(bravo.id.as_str()));
    let starts = world.starts(&id);
    assert_eq!(starts.len(), 2);
    let (args, env) = &starts[1];
    assert_eq!(env.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str), Some(BRAVO));
    let resume = args.iter().position(|a| a == "--resume").expect("resumed");
    assert_eq!(args[resume + 1], id);
    assert_eq!(args.last().map(String::as_str), Some("Continue."));
    let a = app.account("Alpha").await;
    assert_eq!(a.state, AccountState::Limited);
    assert_eq!(a.resets_at, Some(alpha_reset));
    assert_eq!(app.account("Bravo").await.state, AccountState::Ok);

    // Bravo's limit too: nothing is free, so the card waits, and nothing
    // else starts until Alpha's window resets.
    app.printed(&id, "reported", 2).await;
    app.type_line(&id, "limit").await;
    app.printed(&id, "hit the limit", 2).await;
    let note = app
        .until("the card waits", async |app| {
            let note = app.card(&project.id, &id).await.account_note().unwrap_or_default();
            note.starts_with("Waiting").then_some(note)
        })
        .await;
    assert!(
        note.starts_with("Waiting for a Claude account until ") && note.ends_with(" UTC"),
        "{note}"
    );
    let err = app.start(&project.id).await.unwrap_err();
    assert!(
        err.starts_with("Every Claude account is at its limit until ") && err.ends_with(" UTC."),
        "{err}"
    );

    // SRV-103: at its reset Alpha is ok again by itself, and the waiting
    // session resumes on it.
    app.printed(&id, "stand-in ready on alpha", 2).await;
    app.printed(&id, "prompt: Continue.", 2).await;
    assert!(crate_now() >= alpha_reset, "freed early");
    assert_eq!(app.account("Alpha").await.state, AccountState::Ok);
    app.until("the card says it moved back", async |app| {
        let card = app.card(&project.id, &id).await;
        (card.account_note().as_deref() == Some("Moved to Alpha")).then_some(())
    })
    .await;
    assert_eq!(app.account("Bravo").await.state, AccountState::Limited);

    app.sent_no_token(&[ALPHA, BRAVO]);
    let log = server.output.lock().unwrap().clone();
    assert!(!log.contains(ALPHA) && !log.contains(BRAVO), "{log}");
}

/// Milliseconds since the epoch.
fn crate_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_session_itself_can_report_its_usage_or_failures() {
    let world = World::new();
    let server = world.serve();
    let project = world.project(&server).await;
    let mut app = server.connect().await;
    app.ok::<Value>(Request::Watch {
        projects: vec![project.id.clone()],
    })
    .await;
    let alpha = app.add("Alpha", ALPHA).await;

    let one = app.start(&project.id).await.unwrap();
    let two = app.start(&project.id).await.unwrap();
    app.printed(&one, "reported", 1).await;
    app.printed(&two, "reported", 1).await;
    app.type_line(&one, "usage 33").await;
    app.printed(&one, "reported 33", 1).await;
    app.until("its own report arrives", async |app| {
        let a = app.account("Alpha").await;
        (a.five_hour.map(|w| w.used_percentage) == Some(33.0)).then_some(())
    })
    .await;

    // SRV-143: a status line or StopFailure without the session's own token,
    // or with it for another session, is dropped.
    app.type_line(&one, &format!("fake {two}")).await;
    app.printed(&one, "faked", 1).await;
    // What comes after the fakes, from the session itself, lands; so they
    // were seen first, and changed nothing.
    app.type_line(&one, "usage 34").await;
    app.printed(&one, "reported 34", 1).await;
    let a = app
        .until("the next real report", async |app| {
            let a = app.account("Alpha").await;
            (a.five_hour.map(|w| w.used_percentage) == Some(34.0)).then_some(a)
        })
        .await;
    assert_eq!(a.state, AccountState::Ok);
    assert_eq!(a.id, alpha.id);
    assert_eq!(world.starts(&two).len(), 1, "the other session never moved");
    let card = app.card(&project.id, &two).await;
    assert_eq!(card.account_note(), None);
    assert!(app.notices.is_empty(), "{:?}", app.notices);
    let audit: Vec<Value> = app.ok(Request::AuditList {}).await;
    assert!(!audit.iter().any(|line| line["event"] == "account.failed"), "{audit:?}");

    // SRV-103: Claude turning the account down fails it, once, and every
    // member hears of it.
    let mut other = server.connect().await;
    app.type_line(&one, "authfail").await;
    app.printed(&one, "turned down", 1).await;
    let failed = app
        .until("Alpha fails", async |app| {
            let a = app.account("Alpha").await;
            (a.state == AccountState::Failed).then_some(a)
        })
        .await;
    assert_eq!(failed.failure.as_deref(), Some("authentication_failed"));
    for app in [&mut app, &mut other] {
        app.until("the notice", async |app| {
            app.notices
                .iter()
                .any(|n| n.kind == "account-failed" && n.message == "Alpha's Claude account stopped working.")
                .then_some(())
        })
        .await;
    }
    let audit: Vec<Value> = app.ok(Request::AuditList {}).await;
    let line = audit.iter().find(|line| line["event"] == "account.failed").unwrap();
    assert_eq!(line["details"]["label"], "Alpha");
    assert_eq!(line["details"]["error"], "authentication_failed");
    // Nothing else can start on it.
    let err = app.start(&project.id).await.unwrap_err();
    assert!(err.starts_with("Every Claude account has stopped working"), "{err}");
    app.sent_no_token(&[ALPHA]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokens_are_tried_kept_renamed_and_removed_and_background_claude_uses_them() {
    let world = World::new();
    let server = world.serve();
    let project = world.project(&server).await;
    let mut app = server.connect().await;
    app.ok::<Value>(Request::Watch {
        projects: vec![project.id.clone()],
    })
    .await;

    // SRV-100: a token Claude turns down isn't taken, in the server's words.
    let err = app
        .call(Request::AccountsAdd {
            label: "Broken".into(),
            token: "bad-token".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, "Claude turned that token down: Invalid bearer token");
    assert!(app.accounts().await.is_empty());
    let err = app
        .call(Request::AccountsAdd {
            label: "  ".into(),
            token: ALPHA.into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, "A label is 1 to 64 characters");

    let alpha = app.add("Alpha", ALPHA).await;
    let err = app
        .call(Request::AccountsAdd {
            label: "Again".into(),
            token: ALPHA.into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, "That token is in the pool already, as Alpha");
    let bravo = app.add("Bravo", BRAVO).await;
    let audit: Vec<Value> = app.ok(Request::AuditList {}).await;
    let added: Vec<&Value> = audit.iter().filter(|l| l["event"] == "account.added").collect();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0]["details"]["label"], "Alpha");

    // SRV-101: background `claude -p` takes a pool token too: an action's
    // draft here.
    let before = world.background_envs().len();
    let draft: Value = app
        .ok(Request::ActionsDraft {
            project_id: project.id.clone(),
            description: "Run the tests".into(),
            global: false,
        })
        .await;
    assert_eq!(draft["name"], "Run Tests");
    let envs = world.background_envs();
    assert_eq!(envs.len(), before + 1);
    let used: Vec<&str> = envs
        .iter()
        .filter_map(|env| env.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str))
        .collect();
    assert_eq!(used.iter().filter(|t| **t == ALPHA).count(), 2, "{used:?}");
    let a = app.account("Alpha").await;
    assert!(a.last_used_at.is_some());
    assert!(app.account("Bravo").await.last_used_at.is_none());

    // Then the least recently used: Bravo, for the session.
    let id = app.start(&project.id).await.unwrap();
    app.printed(&id, "stand-in ready on bravo", 1).await;

    // A new label shows on the card.
    let renamed: ClaudeAccount = app
        .ok(Request::AccountsRename {
            id: bravo.id.clone(),
            label: "Team Max".into(),
        })
        .await;
    assert_eq!(renamed.label, "Team Max");
    assert_eq!(renamed.sessions, 1);
    let card = app.card(&project.id, &id).await;
    assert_eq!(card.account_label().as_deref(), Some("Team Max"));

    // SRV-105: removing it moves its sessions to another account.
    app.ok::<Value>(Request::AccountsRemove { id: bravo.id.clone() }).await;
    app.printed(&id, "stand-in ready on alpha", 1).await;
    app.until("the card moved", async |app| {
        let card = app.card(&project.id, &id).await;
        (card.account().as_deref() == Some(alpha.id.as_str())
            && card.account_note().as_deref() == Some("Moved to Alpha"))
        .then_some(())
    })
    .await;
    let accounts = app.accounts().await;
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].sessions, 1);
    let starts = world.starts(&id);
    assert_eq!(starts.len(), 2);
    assert!(starts[1].0.contains(&"--resume".to_owned()));
    let audit: Vec<Value> = app.ok(Request::AuditList {}).await;
    assert!(
        audit
            .iter()
            .any(|l| l["event"] == "account.removed" && l["details"]["label"] == "Team Max")
    );

    app.sent_no_token(&[ALPHA, BRAVO, "bad-token"]);
}
