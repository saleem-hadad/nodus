//! git.test.ts, against real git in synthetic repositories, plus the runner's
//! edges (GIT-001, GIT-002, GIT-018, GIT-019) with a stand-in git.

use std::fs;
use std::path::{Path, PathBuf};

use nodus_domain::{GitCommit, GitFileChange, GitFileStatus, GitStatus};
use nodus_git::{DiffError, Git};
use nodus_platform::env::EnvMap;
use tempfile::TempDir;

/// The user's git config (signing, default branch, status options) must not
/// change what these tests see.
fn test_env() -> EnvMap {
    let mut env: EnvMap = std::env::vars().collect();
    env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env
}

fn runner() -> Git {
    Git::new(test_env())
}

/// `fs.realpathSync`, which keeps Windows paths as they are.
fn real(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.to_owned()
    } else {
        fs::canonicalize(dir).unwrap()
    }
}

struct Repo {
    _dir: TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let dir = TempDir::with_prefix("nodus-git-").unwrap();
        let root = real(dir.path());
        git(&root, &["init", "-q", "-b", "main"]);
        Self { _dir: dir, root }
    }

    fn write(&self, file: &str, text: &[u8]) {
        let path = self.root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
        git(&self.root, args)
    }

    fn commit_files(&self, files: &[(&str, &str)]) {
        for (file, text) in files {
            self.write(file, text.as_bytes());
        }
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", "files"]);
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(cwd)
        .envs(test_env())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn change(path: &str, status: GitFileStatus) -> GitFileChange {
    GitFileChange {
        path: path.into(),
        old_path: None,
        status,
    }
}

// gitStatus

#[tokio::test]
async fn is_null_outside_a_repository() {
    let plain = TempDir::with_prefix("nodus-plain-").unwrap();
    assert_eq!(runner().status(plain.path()).await.unwrap(), None);
}

#[tokio::test]
async fn reads_a_repository_with_no_commits_yet() {
    let repo = Repo::new();
    repo.write("first.txt", b"hi\n");
    assert_eq!(
        runner().status(&repo.root).await.unwrap(),
        Some(GitStatus {
            branch: Some("main".into()),
            oid: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            files: vec![change("first.txt", GitFileStatus::Untracked)],
        })
    );
}

#[tokio::test]
async fn lists_every_kind_of_change_sorted_by_path() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n"), ("gone.txt", "bye\n"), ("old name.txt", "same\n")]);
    repo.write("a.txt", b"a changed\n");
    repo.write("c.txt", b"new\n");
    repo.git(&["add", "c.txt"]);
    fs::remove_file(repo.root.join("gone.txt")).unwrap();
    repo.git(&["mv", "old name.txt", "new name.txt"]);
    repo.write("dir/un tracked.txt", b"loose\n");

    let status = runner().status(&repo.root).await.unwrap().unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    let oid = status.oid.unwrap();
    assert!(oid.len() == 40 && oid.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    let mut renamed = change("new name.txt", GitFileStatus::Renamed);
    renamed.old_path = Some("old name.txt".into());
    assert_eq!(
        status.files,
        [
            change("a.txt", GitFileStatus::Modified),
            change("c.txt", GitFileStatus::Added),
            change("dir/un tracked.txt", GitFileStatus::Untracked),
            change("gone.txt", GitFileStatus::Deleted),
            renamed,
        ]
    );
}

#[tokio::test]
async fn counts_commits_ahead_of_the_upstream() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n")]);
    let clone_dir = TempDir::with_prefix("nodus-clone-").unwrap();
    let clone = real(clone_dir.path());
    git(&clone, &["clone", "-q", repo.root.to_str().unwrap(), "."]);
    fs::write(clone.join("b.txt"), "b\n").unwrap();
    git(&clone, &["add", "b.txt"]);
    git(&clone, &["commit", "-q", "-m", "b"]);
    let status = runner().status(&clone).await.unwrap().unwrap();
    assert_eq!(
        (
            status.branch.as_deref(),
            status.upstream.as_deref(),
            status.ahead,
            status.behind
        ),
        (Some("main"), Some("origin/main"), 1, 0)
    );
}

#[tokio::test]
async fn reads_no_branch_on_a_detached_head() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n")]);
    repo.git(&["checkout", "-q", "--detach"]);
    assert_eq!(runner().status(&repo.root).await.unwrap().unwrap().branch, None);
}

// gitDiff

#[tokio::test]
async fn diffs_a_changed_file_against_head() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "one\ntwo\n")]);
    repo.write("a.txt", b"one\nthree\n");
    let diff = runner()
        .diff(&repo.root, &change("a.txt", GitFileStatus::Modified))
        .await
        .unwrap();
    assert!(diff.patch.contains("-two\n+three\n"), "{}", diff.patch);
}

#[tokio::test]
async fn diffs_an_untracked_file_as_all_new() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n")]);
    repo.write("new.txt", b"fresh\n");
    let diff = runner()
        .diff(&repo.root, &change("new.txt", GitFileStatus::Untracked))
        .await
        .unwrap();
    assert!(diff.patch.contains("+fresh\n"), "{}", diff.patch);
}

#[tokio::test]
async fn diffs_a_staged_file_in_a_repository_with_no_commits() {
    let repo = Repo::new();
    repo.write("first.txt", b"hi\n");
    repo.git(&["add", "first.txt"]);
    let diff = runner()
        .diff(&repo.root, &change("first.txt", GitFileStatus::Added))
        .await
        .unwrap();
    assert!(diff.patch.contains("+hi\n"), "{}", diff.patch);
}

#[tokio::test]
async fn takes_paths_from_the_repository_root_when_the_project_is_a_folder_inside_it() {
    let repo = Repo::new();
    repo.commit_files(&[("app/a.txt", "one\n")]);
    repo.write("app/a.txt", b"two\n");
    let diff = runner()
        .diff(&repo.root.join("app"), &change("app/a.txt", GitFileStatus::Modified))
        .await
        .unwrap();
    assert!(diff.patch.contains("+two\n"), "{}", diff.patch);
}

#[tokio::test]
async fn says_when_a_file_is_binary() {
    let repo = Repo::new();
    repo.commit_files(&[("img.bin", "a\0b")]);
    repo.write("img.bin", b"c\0d");
    let diff = runner()
        .diff(&repo.root, &change("img.bin", GitFileStatus::Modified))
        .await
        .unwrap();
    assert_eq!(
        nodus_domain::json::to_string(&diff).unwrap(),
        r#"{"patch":"","binary":true}"#
    );
}

#[tokio::test]
async fn refuses_paths_outside_the_repository() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n")]);
    let mut renamed = change("a.txt", GitFileStatus::Renamed);
    renamed.old_path = Some("../../etc/passwd".into());
    for file in [
        change("../outside.txt", GitFileStatus::Untracked),
        change("/etc/hosts", GitFileStatus::Untracked),
        renamed,
        change(".", GitFileStatus::Modified),
    ] {
        let err = runner().diff(&repo.root, &file).await.unwrap_err();
        assert!(err.to_string().starts_with("Not in the repository"), "{err}");
    }
}

#[tokio::test]
async fn fails_outside_a_repository() {
    let plain = TempDir::with_prefix("nodus-plain-").unwrap();
    let err = runner()
        .diff(plain.path(), &change("a.txt", GitFileStatus::Modified))
        .await
        .unwrap_err();
    assert!(matches!(err, DiffError::NotARepository));
    assert_eq!(err.to_string(), "Not a git repository");
}

#[tokio::test]
async fn says_when_a_diff_is_too_large() {
    // GIT-018: past 2 MiB.
    let repo = Repo::new();
    repo.commit_files(&[("big.txt", "a\n")]);
    repo.write("big.txt", "line of text\n".repeat(200_000).as_bytes());
    let diff = runner()
        .diff(&repo.root, &change("big.txt", GitFileStatus::Modified))
        .await
        .unwrap();
    assert_eq!(
        nodus_domain::json::to_string(&diff).unwrap(),
        r#"{"patch":"","tooLarge":true}"#
    );
}

// gitLog

#[tokio::test]
async fn is_empty_before_the_first_commit_and_outside_a_repository() {
    let repo = Repo::new();
    assert_eq!(runner().log(&repo.root, 5).await.unwrap(), []);
    let plain = TempDir::with_prefix("nodus-plain-").unwrap();
    assert_eq!(runner().log(plain.path(), 5).await.unwrap(), []);
}

#[tokio::test]
async fn lists_the_newest_commits_on_the_branch_first_up_to_the_limit() {
    let repo = Repo::new();
    for i in 1..=7 {
        repo.write("a.txt", format!("{i}\n").as_bytes());
        repo.git(&["add", "-A"]);
        let subject = format!("Change {i}: tabs\tand | pipes");
        let date = format!("{} +0000", 1_700_000_000 + i * 3600);
        repo.git(&[
            "commit",
            "-q",
            "-m",
            &subject,
            "-m",
            "A body that is left out",
            "--date",
            &date,
        ]);
    }
    let log = runner().log(&repo.root, 5).await.unwrap();
    let subjects: Vec<_> = log.iter().map(|c| c.subject.clone()).collect();
    let expected: Vec<_> = [7, 6, 5, 4, 3]
        .iter()
        .map(|i| format!("Change {i}: tabs\tand | pipes"))
        .collect();
    assert_eq!(subjects, expected);
    assert_eq!(
        log[0],
        GitCommit {
            oid: repo.git(&["rev-parse", "HEAD"]).trim().to_owned(),
            subject: "Change 7: tabs\tand | pipes".into(),
            author: "Test".into(),
            time: f64::from(1_700_000_000 + 7 * 3600) * 1000.0,
        }
    );
    assert_eq!(runner().log(&repo.root, 2).await.unwrap().len(), 2);
}

#[tokio::test]
async fn follows_the_branch_that_is_checked_out() {
    let repo = Repo::new();
    repo.commit_files(&[("a.txt", "a\n")]);
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write("b.txt", b"b\n");
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "On feature"]);
    let subjects = |log: Vec<GitCommit>| log.into_iter().map(|c| c.subject).collect::<Vec<_>>();
    assert_eq!(
        subjects(runner().log(&repo.root, 5).await.unwrap()),
        ["On feature", "files"]
    );
    repo.git(&["switch", "-q", "main"]);
    assert_eq!(subjects(runner().log(&repo.root, 5).await.unwrap()), ["files"]);
}

// The runner, with a stand-in git.

#[tokio::test]
async fn fails_when_git_is_missing() {
    let empty = TempDir::new().unwrap();
    let mut env = test_env();
    env.insert("PATH".into(), empty.path().display().to_string());
    let err = Git::new(env).status(empty.path()).await.unwrap_err();
    assert_eq!(err.to_string(), "spawn git ENOENT");
}

#[cfg(unix)]
mod stand_in {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A `git` that logs its arguments and environment, and answers as told.
    fn fake_git(dir: &Path, script: &str) -> Git {
        let git = dir.join("git");
        fs::write(&git, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = test_env();
        env.insert("PATH".into(), format!("{}:/usr/bin:/bin", dir.display()));
        env.insert("LOG".into(), dir.join("log").display().to_string());
        Git::new(env)
    }

    #[tokio::test]
    async fn runs_git_with_its_prefix_and_no_prompts() {
        let dir = TempDir::new().unwrap();
        let git = fake_git(
            dir.path(),
            r#"echo "$GIT_TERMINAL_PROMPT $*" >> "$LOG"; printf '# branch.head main\0'"#,
        );
        let status = git.status(dir.path()).await.unwrap().unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(
            fs::read_to_string(dir.path().join("log")).unwrap(),
            "0 --no-optional-locks -c core.quotepath=false status --porcelain=v2 --branch --untracked-files=all -z\n"
        );
    }

    #[tokio::test]
    async fn treats_a_non_zero_exit_as_an_answer_and_a_signal_as_a_failure() {
        let dir = TempDir::new().unwrap();
        let git = fake_git(dir.path(), "exit 128");
        assert_eq!(git.status(dir.path()).await.unwrap(), None);
        assert_eq!(git.log(dir.path(), 5).await.unwrap(), []);
        let git = fake_git(dir.path(), "kill -9 $$");
        let err = git.status(dir.path()).await.unwrap_err();
        assert!(
            err.to_string().starts_with("Command failed: git --no-optional-locks"),
            "{err}"
        );
        // GIT-001: past 32 MiB, a status read fails.
        let git = fake_git(dir.path(), "head -c 34000000 /dev/zero");
        let err = git.status(dir.path()).await.unwrap_err();
        assert_eq!(err.to_string(), "stdout maxBuffer length exceeded");
    }

    #[tokio::test]
    async fn fails_a_diff_git_refuses_but_not_an_untracked_one_that_differs() {
        let dir = TempDir::new().unwrap();
        let root = real(dir.path());
        let script = format!(
            r#"case "$4" in
  rev-parse) [ "$5" = --show-toplevel ] && echo "{root}"; exit 0;;
  diff) echo "$*" >> "$LOG"; printf 'diff --git a/x b/x\n+x\n'; exit $CODE;;
esac"#,
            root = root.display()
        );
        let mut env = test_env();
        env.insert("PATH".into(), format!("{}:/usr/bin:/bin", dir.path().display()));
        env.insert("LOG".into(), dir.path().join("log").display().to_string());
        fs::write(dir.path().join("git"), format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(dir.path().join("git"), fs::Permissions::from_mode(0o755)).unwrap();

        env.insert("CODE".into(), "1".into());
        let git = Git::new(env.clone());
        let untracked = change("x", GitFileStatus::Untracked);
        assert_eq!(
            git.diff(&root, &untracked).await.unwrap().patch,
            "diff --git a/x b/x\n+x\n"
        );
        let err = git
            .diff(&root, &change("x", GitFileStatus::Modified))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "git diff failed for x");
        env.insert("CODE".into(), "2".into());
        let err = Git::new(env).diff(&root, &untracked).await.unwrap_err();
        assert_eq!(err.to_string(), "git diff failed for x");
        let log = fs::read_to_string(dir.path().join("log")).unwrap();
        let first = log.lines().next().unwrap();
        assert_eq!(
            first,
            "--no-optional-locks -c core.quotepath=false diff --no-color --no-ext-diff --no-index -- /dev/null x"
        );
        assert!(
            log.lines()
                .nth(1)
                .unwrap()
                .ends_with("diff --no-color --no-ext-diff -M HEAD -- x")
        );
    }
}
