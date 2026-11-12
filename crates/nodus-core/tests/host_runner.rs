//! The core as `nodus serve` wires it: its session host runs on its own, in
//! another process (as `nodus-run`, under systemd, on a server), and every
//! process the core starts runs there, so the core starts none (SRV-004,
//! SRV-005).
//!
//! The host is this test binary started again as `host_process`, which runs
//! `host --socket`. The core's process forbids itself to start anything, and
//! counts what it tried: git, Claude's quick calls, sessions and views all
//! work, and the count stays at zero. A stand-in `claude` stands in for the
//! real one, which is never run.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use nodus_core::domain::{CreateSessionOptions, CreateViewOptions, GitFileChange};
use nodus_core::{EnvSource, HostMode, Nodus, NodusConfig, RunnerSource};
use nodus_platform::process;

/// Set in the environment of this binary when it runs as the host.
const HOST_SOCKET: &str = "NODUS_TEST_HOST_SOCKET";
const WAIT: Duration = Duration::from_secs(20);

/// Not a test of its own: the session host, when the test below starts this
/// binary as one. Run normally, it does nothing.
#[test]
fn host_process() {
    if let Some(socket) = std::env::var_os(HOST_SOCKET) {
        let args = ["--socket".to_owned(), socket.to_string_lossy().into_owned()];
        std::process::exit(nodus_host::main(&args));
    }
}

/// The host's process, stopped if a test fails before it has gone.
struct HostProcess(Child);

impl Drop for HostProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs git for the test's own setup, in this process but not through
/// `nodus_platform::process`, so it isn't counted.
fn git(repo: &Path, env: &BTreeMap<String, String>, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .env_clear()
        .envs(env)
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A `claude` that's a session in a terminal, or answers `-p` as naming and
/// drafting expect, noting which process started it.
fn write_stand_in(file: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "-p" ]; then
  echo "$PPID" >> "{log}/quick.parents"
  case "$*" in
    *--json-schema*) echo '{{"type":"result","is_error":false,"structured_output":{{"name":"Run Tests","icon":"test-tube","prompt":"Run them."}}}}' ;;
    *) echo '{{"type":"result","is_error":false,"result":"Name: Auth Work."}}' ;;
  esac
  exit 0
fi
echo "stand-in ready"
while IFS= read -r line; do echo "got: $line"; done
"#,
        log = log.display()
    );
    std::fs::write(file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_core_starts_no_process_of_its_own() {
    // Short, so the socket's path fits.
    let dir = tempfile::Builder::new()
        .prefix("nodus-run-")
        .tempdir_in("/tmp")
        .unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (home, bin, log, config) = (
        root.join("home"),
        root.join("bin"),
        root.join("log"),
        root.join("config"),
    );
    for folder in [&home, &bin, &log, &config] {
        std::fs::create_dir_all(folder).unwrap();
    }
    write_stand_in(&bin.join("claude"), &log);
    let env: BTreeMap<String, String> = [
        ("PATH", format!("{}:/usr/bin:/bin", bin.display())),
        ("HOME", home.display().to_string()),
        ("SHELL", "/bin/sh".to_owned()),
        ("LANG", "en_US.UTF-8".to_owned()),
        // The machine's git config (signing, hooks) must not change what's seen.
        ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
        ("GIT_CONFIG_NOSYSTEM", "1".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();

    let repo = root.join("shop");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &env, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "first\n").unwrap();
    git(&repo, &env, &["add", "README.md"]);
    git(
        &repo,
        &env,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "First",
        ],
    );
    std::fs::write(repo.join("README.md"), "first\nchanged\n").unwrap();

    // The host, in a process of its own, with a home of its own.
    let socket = root.join("run").join("host.sock");
    let host = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["host_process", "--exact", "--nocapture"])
        .env(HOST_SOCKET, &socket)
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &config)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(root.join("host.log")).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut host = HostProcess(host);

    // From here on this process starts nothing; trying counts.
    let env = nodus_platform::Env::given(env, Some(&home));
    process::forbid();
    let started = process::started();
    let nodus = Nodus::start(NodusConfig {
        env: EnvSource::Given(env),
        home: home.clone(),
        host: HostMode::Connect(socket.clone()),
        runner: RunnerSource::Host,
        ..NodusConfig::new(root.join("userData"), tokio::runtime::Handle::current())
    })
    .unwrap();
    nodus.launch().await.unwrap();
    let project = nodus
        .projects()
        .add(Some(repo.to_string_lossy().into_owned()))
        .await
        .unwrap()
        .unwrap();

    // Git, in the host, waiting for it to be up (GIT-001).
    let status = tokio::time::timeout(WAIT, nodus.git().status(&project.id))
        .await
        .unwrap()
        .unwrap()
        .expect("a repository");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(
        status.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["README.md"]
    );
    let commits = nodus.git().log(&project.id, None).await.unwrap();
    assert_eq!(
        commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
        ["First"]
    );
    let file: &GitFileChange = &status.files[0];
    let diff = nodus.git().diff(&project.id, file).await.unwrap();
    assert!(diff.patch.contains("+changed"), "{diff:?}");

    // Sessions run in the host too, and Claude's quick calls find claude there.
    let mut ids = Vec::new();
    for name in ["Fix login", "Login tests"] {
        let card = nodus
            .sessions()
            .create(
                &project.id,
                &CreateSessionOptions {
                    name: Some(name.into()),
                    ..CreateSessionOptions::default()
                },
            )
            .await
            .unwrap();
        assert!(card.live);
        ids.push(card.node.session_id);
    }
    let name = nodus.board().name_section(&project.id, &ids).await.unwrap();
    assert_eq!(name, "Auth Work");
    let draft = nodus
        .actions()
        .draft(&project.id, "run the tests", false)
        .await
        .unwrap();
    assert_eq!((draft.name.as_str(), draft.prompt.as_str()), ("Run Tests", "Run them."));
    let parents = std::fs::read_to_string(log.join("quick.parents")).unwrap();
    let host_pid = host.0.id().to_string();
    assert_eq!(
        parents.lines().collect::<Vec<_>>(),
        [host_pid.as_str(), host_pid.as_str()]
    );

    // A view's page asking for git status, with its brain in the host.
    let view = nodus
        .views()
        .create(
            &project.id,
            &CreateViewOptions {
                description: "Show the changes".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .await
        .unwrap();
    let git = nodus
        .views()
        .call(&project.id, &view.id, "git", &serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(git["branch"], "main");

    // SRV-126: the folder, trusted for the host's user.
    nodus.trust_folder(&repo.to_string_lossy()).await.unwrap();
    let claude_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        claude_json["projects"][repo.to_string_lossy().as_ref()]["hasTrustDialogAccepted"],
        true
    );

    assert_eq!(process::started(), started, "the core started a process itself");

    // Quit and Stop Sessions stops the host, which leaves.
    nodus.menu().quit_and_stop_sessions().await;
    nodus.shutdown();
    let deadline = std::time::Instant::now() + WAIT;
    let code = loop {
        if let Some(status) = host.0.try_wait().unwrap() {
            break status.code();
        }
        assert!(std::time::Instant::now() < deadline, "the host didn't exit");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(code, Some(0));
    assert_eq!(process::started(), started);
}
