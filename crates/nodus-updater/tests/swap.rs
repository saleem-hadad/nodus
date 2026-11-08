//! The swap script, run for real by /bin/sh in a scratch folder, with `open`
//! played by a script that writes down how it was called (UPD-059…UPD-063).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use nodus_updater::SWAP_SCRIPT;
use tempfile::TempDir;

struct Scene {
    dir: TempDir,
}

impl Scene {
    /// The running app (`old`), the staged one (`new`) and a fake `open`.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let scene = Self { dir };
        std::fs::create_dir_all(scene.app()).unwrap();
        std::fs::write(scene.app().join("version"), "old").unwrap();
        std::fs::create_dir_all(scene.staged()).unwrap();
        std::fs::write(scene.staged().join("version"), "new").unwrap();
        std::fs::create_dir_all(scene.bin()).unwrap();
        let open = scene.bin().join("open");
        let log = scene.path("open.log");
        std::fs::write(
            &open,
            format!("#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        scene
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn app(&self) -> PathBuf {
        self.path("Applications/Nodus.app")
    }
    fn staged(&self) -> PathBuf {
        self.path("updates/0.9.5/Nodus.app")
    }
    fn previous(&self) -> PathBuf {
        self.path("updates/previous.app")
    }
    fn failed(&self) -> PathBuf {
        self.path("updates/failed")
    }
    fn bin(&self) -> PathBuf {
        self.path("bin")
    }

    fn version(&self, app: &Path) -> Option<String> {
        std::fs::read_to_string(app.join("version")).ok()
    }

    fn opened(&self) -> Option<String> {
        std::fs::read_to_string(self.path("open.log")).ok()
    }

    fn spawn(&self, pid: u32, relaunch: &str, user_data: &str) -> Child {
        let path = format!("{}:/usr/bin:/bin", self.bin().display());
        Command::new("/bin/sh")
            .args(["-c", SWAP_SCRIPT, "nodus-update"])
            .arg(pid.to_string())
            .args([self.app(), self.staged(), self.previous(), self.failed()])
            .args(["0.9.5", relaunch, user_data])
            .env("PATH", path)
            .spawn()
            .unwrap()
    }

    /// Runs the script for an app process that has already exited.
    fn run(&self, relaunch: &str, user_data: &str) -> ExitStatus {
        let mut gone = Command::new("/usr/bin/true").spawn().unwrap();
        gone.wait().unwrap();
        self.spawn(gone.id(), relaunch, user_data).wait().unwrap()
    }
}

#[test]
fn upd_060_swaps_the_new_app_in_and_the_old_one_out() {
    let scene = Scene::new();
    assert!(scene.run("", "").success());
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("new"));
    assert!(!scene.staged().exists());
    assert!(!scene.previous().exists());
    assert!(!scene.failed().exists());
    assert_eq!(scene.opened(), None, "a plain quit doesn't reopen");
}

#[test]
fn upd_060_clears_a_previous_app_left_behind() {
    let scene = Scene::new();
    std::fs::create_dir_all(scene.previous()).unwrap();
    std::fs::write(scene.previous().join("version"), "older").unwrap();
    assert!(scene.run("", "").success());
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("new"));
    assert!(!scene.previous().exists());
}

#[test]
fn upd_063_reopens_after_restart_to_update() {
    let scene = Scene::new();
    assert!(scene.run("1", "").success());
    assert_eq!(scene.opened(), Some(format!("-n|{}|", scene.app().display())));
    let scene = Scene::new();
    assert!(scene.run("1", "/Users/example/test-data").success());
    assert_eq!(
        scene.opened(),
        Some(format!(
            "-n|--env|NODUS_USER_DATA=/Users/example/test-data|{}|",
            scene.app().display()
        ))
    );
}

#[test]
fn upd_061_puts_the_old_app_back_when_the_new_one_cant_go_in() {
    let scene = Scene::new();
    std::fs::remove_dir_all(scene.staged()).unwrap();
    scene.run("1", "");
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("old"));
    assert!(!scene.previous().exists());
    assert_eq!(std::fs::read_to_string(scene.failed()).unwrap(), "0.9.5\n");
    // It reopens the old version all the same.
    assert_eq!(scene.opened(), Some(format!("-n|{}|", scene.app().display())));
}

#[test]
fn upd_062_writes_down_the_failure_when_the_old_app_cant_move() {
    let scene = Scene::new();
    std::fs::remove_dir_all(scene.app()).unwrap();
    scene.run("", "");
    assert_eq!(std::fs::read_to_string(scene.failed()).unwrap(), "0.9.5\n");
    assert_eq!(scene.version(&scene.staged()).as_deref(), Some("new"), "left alone");
    assert!(!scene.app().exists());
}

#[test]
fn upd_059_waits_for_the_app_to_exit_first() {
    let scene = Scene::new();
    let mut app = Command::new("/bin/sleep").arg("1").spawn().unwrap();
    let started = Instant::now();
    let mut swap = scene.spawn(app.id(), "", "");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("old"), "not while it runs");
    app.wait().unwrap();
    assert!(swap.wait().unwrap().success());
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("new"));
}

#[test]
fn upd_059_ignores_hangup_interrupt_and_terminate() {
    let scene = Scene::new();
    let mut app = Command::new("/bin/sleep").arg("1").spawn().unwrap();
    let mut swap = scene.spawn(app.id(), "", "");
    std::thread::sleep(Duration::from_millis(200));
    for signal in ["HUP", "INT", "TERM"] {
        Command::new("/bin/kill")
            .args([&format!("-{signal}"), &swap.id().to_string()])
            .status()
            .unwrap();
    }
    app.wait().unwrap();
    assert!(swap.wait().unwrap().success());
    assert_eq!(scene.version(&scene.app()).as_deref(), Some("new"));
}

#[test]
fn the_script_is_0_9_4s_byte_for_byte() {
    assert!(SWAP_SCRIPT.starts_with("\ntrap '' HUP INT TERM\n"));
    assert!(SWAP_SCRIPT.ends_with(
        "if [ -n \"$8\" ]; then open -n --env \"NODUS_USER_DATA=$8\" \"$app\"; else open -n \"$app\"; fi\n"
    ));
    assert_eq!(SWAP_SCRIPT.lines().count(), 18);
    // Against the TypeScript itself, while it's in the repo.
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/main/updater.ts");
    if let Ok(source) = std::fs::read_to_string(source) {
        let start = source.find("const SWAP_SCRIPT = `").unwrap() + "const SWAP_SCRIPT = `".len();
        let end = start + source[start..].find('`').unwrap();
        assert_eq!(&source[start..end], SWAP_SCRIPT);
    }
}
