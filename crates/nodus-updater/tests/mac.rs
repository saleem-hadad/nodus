//! The macOS updater, with hdiutil, ditto, codesign and plutil played by a
//! fake that does to the file system what they would. Not ports: 0.9.4 has
//! no macOS test; these cover UPD-037…UPD-047, UPD-055…UPD-058 and the
//! macOS side of clean-up (UPD-071…UPD-075).

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::*;
use nodus_domain::UpdateState;
use nodus_updater::{
    BAD_SIGNATURE, DAMAGED, NO_APP, NOT_EXPECTED, NOT_PREPARED, OTHER_DEVELOPER, Platform, RestartOutcome, RunOutput,
    SWAP_SCRIPT,
};

const DMG_URL: &str = "https://github.com/saleem-hadad/nodus/releases/download/v0.9.5/Nodus-x64.dmg";
const DMG: &[u8] = b"a synthetic disk image";

/// What the fake tools find and answer.
struct Tools {
    /// What the mounted DMG holds, in the order readdir lists it.
    in_dmg: Vec<&'static str>,
    signature_ok: bool,
    new_id: &'static str,
    own_id: &'static str,
    new_version: &'static str,
    own_team: Option<&'static str>,
    new_team: Option<&'static str>,
    plutil_fails: bool,
    detach_fails: bool,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            in_dmg: vec!["Nodus.app", "Applications"],
            signature_ok: true,
            new_id: "dev.saleem.nodus",
            own_id: "dev.saleem.nodus",
            new_version: "0.9.5",
            own_team: Some("ABCDE12345"),
            new_team: Some("ABCDE12345"),
            plutil_fails: false,
            detach_fails: false,
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn remove(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn mac_world(tools: Tools) -> (World, Arc<Mutex<Tools>>) {
    let tools = Arc::new(Mutex::new(tools));
    let shared = tools.clone();
    let runner = Box::new(
        move |system: &FakeSystem, program: &str, args: &[String]| -> Result<RunOutput, String> {
            let tools = shared.lock().unwrap();
            let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
            let ok = |stdout: String, stderr: String| Ok(RunOutput { stdout, stderr });
            match (program, arg(0)) {
                ("/bin/rm", "-rf") => {
                    remove(Path::new(arg(1)));
                    ok(String::new(), String::new())
                }
                ("/usr/bin/hdiutil", "attach") => {
                    assert!(Path::new(arg(1)).is_file(), "the DMG is there to attach");
                    let mount = Path::new(arg(6));
                    for name in &tools.in_dmg {
                        if name.ends_with(".app") {
                            std::fs::create_dir_all(mount.join(name).join("Contents")).unwrap();
                            std::fs::write(mount.join(name).join("Contents").join("Info.plist"), "plist").unwrap();
                        } else {
                            std::fs::write(mount.join(name), "").unwrap();
                        }
                    }
                    ok(String::new(), String::new())
                }
                ("/usr/bin/hdiutil", "detach") => {
                    if tools.detach_fails {
                        return Err("hdiutil: detach failed - Resource busy".into());
                    }
                    for entry in std::fs::read_dir(arg(1)).into_iter().flatten() {
                        remove(&entry.unwrap().path());
                    }
                    ok(String::new(), String::new())
                }
                ("/usr/bin/ditto", "--noqtn") => {
                    copy_dir(Path::new(arg(1)), Path::new(arg(2))).map_err(|err| err.to_string())?;
                    ok(String::new(), String::new())
                }
                ("/usr/bin/codesign", "--verify") => {
                    if tools.signature_ok {
                        ok(String::new(), String::new())
                    } else {
                        Err("a sealed resource is missing or invalid".into())
                    }
                }
                ("/usr/bin/codesign", "-dv") => {
                    let own = Path::new(arg(1)) == system.bundle;
                    let team = if own { tools.own_team } else { tools.new_team };
                    let stderr = format!(
                        "Executable={}/Contents/MacOS/Nodus\nIdentifier=dev.saleem.nodus\nTeamIdentifier={}\n",
                        arg(1),
                        team.unwrap_or("not set")
                    );
                    ok(String::new(), stderr)
                }
                ("/usr/bin/plutil", "-extract") => {
                    if tools.plutil_fails {
                        return Err("Could not extract value".into());
                    }
                    let own = Path::new(arg(5)).starts_with(&system.bundle);
                    let value = match (arg(1), own) {
                        ("CFBundleIdentifier", true) => tools.own_id,
                        ("CFBundleIdentifier", false) => tools.new_id,
                        ("CFBundleShortVersionString", false) => tools.new_version,
                        _ => return Err("no such key".into()),
                    };
                    ok(format!("{value}\n"), String::new())
                }
                _ => panic!("unexpected {program} {args:?}"),
            }
        },
    );
    let world = World::new(Platform::Mac, "0.9.4", runner);
    serve(&world, DMG, &digest_of(DMG));
    (world, tools)
}

/// GitHub's latest: 0.9.5, with this DMG behind `Nodus-x64.dmg`.
fn serve(world: &World, dmg: &[u8], digest: &str) {
    let asset = asset_json("Nodus-x64.dmg", DMG.len(), digest, DMG_URL);
    world
        .system
        .route(latest(), Route::json(&release_json("v0.9.5", &[asset])));
    world.system.route(DMG_URL, Route::bytes(dmg));
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

async fn prepare(world: &World) -> (Arc<nodus_updater::Updater>, UpdateState) {
    let updater = world.launch();
    updater.check(true).await;
    let state = prepared(&updater).await;
    (updater, state)
}

/// The mount folder the attach used.
fn mount_of(world: &World) -> PathBuf {
    let runs = world.system.runs();
    let attach = runs.iter().find(|run| run[1] == "attach").expect("attached");
    PathBuf::from(&attach[7])
}

#[tokio::test]
async fn upd_037_to_046_copies_the_app_out_of_the_dmg_checks_it_and_stages_it() {
    let (world, _) = mac_world(Tools::default());
    let (updater, state) = prepare(&world).await;
    assert_eq!(state, ready("0.9.5"));
    let updates = world.updates();
    let dir = updates.join("0.9.5");
    let target = dir.join("Nodus.app");
    let dmg = updates.join("Nodus-0.9.5.dmg");
    let mount = mount_of(&world);
    assert_eq!(mount.parent(), Some(updates.as_path()));
    let name = mount.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("mount-") && name.len() == 12, "{name}");
    let bundle = world.system.bundle.clone();
    let plist = |app: &Path| s(&app.join("Contents").join("Info.plist"));
    assert_eq!(
        world.system.runs(),
        [
            argv(&["/bin/rm", "-rf", &s(&dir)]),
            argv(&[
                "/usr/bin/hdiutil",
                "attach",
                &s(&dmg),
                "-nobrowse",
                "-noautoopen",
                "-readonly",
                "-mountpoint",
                &s(&mount)
            ]),
            argv(&["/usr/bin/ditto", "--noqtn", &s(&mount.join("Nodus.app")), &s(&target)]),
            argv(&["/usr/bin/hdiutil", "detach", &s(&mount), "-force"]),
            argv(&["/usr/bin/codesign", "--verify", "--deep", "--strict", &s(&target)]),
            argv(&[
                "/usr/bin/plutil",
                "-extract",
                "CFBundleIdentifier",
                "raw",
                "-o",
                "-",
                &plist(&target)
            ]),
            argv(&[
                "/usr/bin/plutil",
                "-extract",
                "CFBundleIdentifier",
                "raw",
                "-o",
                "-",
                &plist(&bundle)
            ]),
            argv(&[
                "/usr/bin/plutil",
                "-extract",
                "CFBundleShortVersionString",
                "raw",
                "-o",
                "-",
                &plist(&target)
            ]),
            argv(&["/usr/bin/codesign", "-dv", &s(&bundle)]),
            argv(&["/usr/bin/codesign", "-dv", &s(&target)]),
        ]
    );
    assert!(target.join("Contents").join("Info.plist").is_file());
    assert!(!dmg.exists(), "the DMG is deleted");
    assert!(!mount.exists(), "the mount folder is removed");
    assert_eq!(updater.staged().unwrap().app, target);
    assert_eq!(
        world.states(),
        [
            UpdateState::Checking,
            UpdateState::Downloading {
                version: "0.9.5".into(),
                percent: 0
            },
            UpdateState::Downloading {
                version: "0.9.5".into(),
                percent: 100
            },
            ready("0.9.5"),
        ]
    );
    assert_eq!(world.app.user_agent_requests(&world.system), 2);
}

trait UserAgents {
    fn user_agent_requests(&self, system: &FakeSystem) -> usize;
}

impl UserAgents for FakeApp {
    /// Requests that said they're this version of Nodus.
    fn user_agent_requests(&self, system: &FakeSystem) -> usize {
        let agent = format!("Nodus/{}", self.version);
        system
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, headers)| headers.contains(&("User-Agent", agent.clone())))
            .count()
    }
}

#[tokio::test]
async fn upd_041_a_dmg_without_an_app_fails_and_is_still_detached() {
    let (world, _) = mac_world(Tools {
        in_dmg: vec!["README.txt"],
        ..Tools::default()
    });
    let (updater, state) = prepare(&world).await;
    assert_eq!(state, error(NO_APP));
    let runs = world.system.runs();
    assert!(runs.iter().any(|run| run[..2] == ["/usr/bin/hdiutil", "detach"]));
    assert!(!runs.iter().any(|run| run[0] == "/usr/bin/ditto"));
    // UPD-047: the version's folder and the DMG are gone.
    assert_eq!(world.update_files(), Vec::<String>::new());
    assert_eq!(updater.staged(), None);
}

#[tokio::test]
async fn upd_044_a_bad_signature_is_refused() {
    let (world, _) = mac_world(Tools {
        signature_ok: false,
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(BAD_SIGNATURE));
    assert_eq!(world.update_files(), Vec::<String>::new());
}

#[tokio::test]
async fn upd_045_another_app_or_another_version_is_refused() {
    let (world, _) = mac_world(Tools {
        new_id: "com.example.other",
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(NOT_EXPECTED));
    let (world, _) = mac_world(Tools {
        new_version: "0.9.6",
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(NOT_EXPECTED));
}

#[tokio::test]
async fn upd_045_a_plist_that_cant_be_read_is_a_generic_failure() {
    let (world, _) = mac_world(Tools {
        plutil_fails: true,
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(NOT_PREPARED));
}

#[tokio::test]
async fn upd_046_only_the_same_team_may_replace_a_signed_copy() {
    let (world, _) = mac_world(Tools {
        new_team: Some("ZZZZZ99999"),
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(OTHER_DEVELOPER));
    let (world, _) = mac_world(Tools {
        new_team: None,
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, error(OTHER_DEVELOPER));
}

#[tokio::test]
async fn upd_046_an_ad_hoc_copy_takes_any_signed_update() {
    let (world, _) = mac_world(Tools {
        own_team: None,
        new_team: Some("ZZZZZ99999"),
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, ready("0.9.5"));
    let dv = world.system.runs().iter().filter(|run| run[1] == "-dv").count();
    assert_eq!(dv, 1, "only the running copy's team is read");
}

#[tokio::test]
async fn upd_039_without_a_digest_only_the_size_is_checked() {
    let (world, _) = mac_world(Tools::default());
    serve(&world, DMG, "null");
    assert_eq!(prepare(&world).await.1, ready("0.9.5"));
    let (world, _) = mac_world(Tools::default());
    serve(&world, b"a synthetic disk imagE", "null");
    assert_eq!(prepare(&world).await.1, ready("0.9.5"), "same size, no digest: taken");
    let (world, _) = mac_world(Tools::default());
    serve(&world, b"short", "null");
    assert_eq!(prepare(&world).await.1, error(DAMAGED));
}

#[tokio::test]
async fn upd_043_a_failed_detach_is_only_logged() {
    let (world, _) = mac_world(Tools {
        detach_fails: true,
        ..Tools::default()
    });
    assert_eq!(prepare(&world).await.1, ready("0.9.5"));
}

fn swap_args(world: &World, staged: &Path, relaunch: &str, user_data: &str) -> (PathBuf, Vec<String>) {
    let updates = world.updates();
    (
        PathBuf::from("/bin/sh"),
        vec![
            "-c".into(),
            SWAP_SCRIPT.into(),
            "nodus-update".into(),
            "4242".into(),
            s(&world.system.bundle),
            s(staged),
            s(&updates.join("previous.app")),
            s(&updates.join("failed")),
            "0.9.5".into(),
            relaunch.into(),
            user_data.into(),
        ],
    )
}

#[tokio::test]
async fn upd_055_057_restart_to_update_quits_and_the_swap_reopens_the_app() {
    let (world, _) = mac_world(Tools::default());
    *world.app.user_data_override.lock().unwrap() = Some("/Users/example/test-data".into());
    let (updater, _) = prepare(&world).await;
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Quitting);
    assert_eq!(*world.app.log.lock().unwrap(), ["quit"]);
    updater.on_will_quit();
    let staged = updater.staged().unwrap().app;
    assert_eq!(
        world.system.spawns(),
        [swap_args(&world, &staged, "1", "/Users/example/test-data")]
    );
    // Once per run.
    updater.on_will_quit();
    assert_eq!(world.system.spawns().len(), 1);
}

#[tokio::test]
async fn upd_056_058_any_quit_installs_but_only_restart_reopens() {
    let (world, _) = mac_world(Tools::default());
    *world.app.user_data_override.lock().unwrap() = Some("/Users/example/test-data".into());
    let (updater, _) = prepare(&world).await;
    updater.on_will_quit();
    let staged = updater.staged().unwrap().app;
    assert_eq!(world.system.spawns(), [swap_args(&world, &staged, "", "")]);
}

#[tokio::test]
async fn nothing_staged_nothing_swaps() {
    let (world, _) = mac_world(Tools::default());
    let updater = world.launch();
    updater.on_will_quit();
    assert_eq!(updater.restart_to_update().await, RestartOutcome::Nothing);
    assert!(world.system.spawns().is_empty());
    assert!(world.app.log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upd_071_to_074_clean_up_keeps_only_a_failure_that_still_matters() {
    let (world, _) = mac_world(Tools::default());
    let updates = world.updates();
    std::fs::create_dir_all(updates.join("mount-abc123")).unwrap();
    std::fs::create_dir_all(updates.join("0.9.4").join("Nodus.app")).unwrap();
    std::fs::create_dir_all(updates.join("previous.app")).unwrap();
    std::fs::write(updates.join("Nodus-0.9.5.dmg"), "half").unwrap();
    std::fs::write(updates.join("failed"), "0.9.5\n").unwrap();
    let updater = world.launch();
    // UPD-027: an automatic check doesn't download a version that failed.
    updater.check(false).await;
    assert_eq!(
        state(&updater),
        error("Nodus 0.9.5 downloaded but couldn’t install itself.")
    );
    assert_eq!(world.system.hits(DMG_URL), 0);
    assert_eq!(world.update_files(), ["failed"]);
    let runs = world.system.runs();
    let mount = s(&updates.join("mount-abc123"));
    let detach = runs
        .iter()
        .position(|run| *run == argv(&["/usr/bin/hdiutil", "detach", &mount, "-force"]))
        .expect("the leftover mount is detached");
    let removed = runs
        .iter()
        .position(|run| *run == argv(&["/bin/rm", "-rf", &mount]))
        .expect("and removed");
    assert!(detach < removed);
    // UPD-028: an asked check tries it again.
    updater.check(true).await;
    assert_eq!(prepared(&updater).await, ready("0.9.5"));
}

#[tokio::test]
async fn upd_071_a_failure_this_copy_has_caught_up_with_is_cleared() {
    let (world, _) = mac_world(Tools::default());
    std::fs::create_dir_all(world.updates()).unwrap();
    std::fs::write(world.updates().join("failed"), "0.9.4\n").unwrap();
    let updater = world.launch();
    updater.check(false).await;
    assert_eq!(prepared(&updater).await, ready("0.9.5"));
    assert!(!world.updates().join("failed").exists());
}
