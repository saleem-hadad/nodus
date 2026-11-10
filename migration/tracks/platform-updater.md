# Track: platform-updater (P3)

- **Branch:** `rust/platform-updater`
- **Scope, in order:** `nodus_platform::process` (merged early), secrets (D6), notifications and the Dock badge, open / reveal / permissions / System Settings, then `crates/nodus-updater` (spec 06's UPD behaviors that aren't UI). Packaging is P11.
- **Specs:** 01 (PLAT-002, PLAT-076, PLAT-115…PLAT-121), 02 (notifications, Dock badge: the OS side of HOST-208…HOST-215), 05 (VOICE-009, VOICE-010), 06 (UPD-001…UPD-075), 07 (CAP-032, CAP-040, CAP-041, CAP-055, CAP-059: the OS side).

## Status

| Module | Commit | State |
| --- | --- | --- |
| `nodus_platform::process` | *Start every child process through nodus-platform* | ✅ on main (cherry-picked as bb351d3) |
| `nodus_platform::secrets` | *Read and write the xAI key as 0.9.4's safeStorage does* | ✅ |
| `nodus_platform::{notify, shell, permissions, app}` | *Show notifications, the Dock badge, links and permissions natively* | ✅ (real delivery left for the packaged test) |
| `nodus-updater` | *Update the app from GitHub releases as 0.9.4 does* | ✅ |

## Done (implemented and tested)

- PLAT-097 / PLAT-105: the login-shell capture and `claude --version` now run through `process::Command` (10 s timeout, 8 MiB cap / Node's 1 MiB default, the shell killed outright at the timeout as the plan decided).
- PLAT-115 (base64 as Node writes and leniently reads it), PLAT-116 (macOS v10 AES-128-CBC/PBKDF2), PLAT-117 (Keychain `Nodus Safe Storage` / `Nodus Key`, legacy account `Nodus` moved, 24-character random password created), PLAT-118 (Windows v10 AES-256-GCM, `Local State` DPAPI key: read, created, replaced when DPAPI can't open it, audit flag), PLAT-119 (`CANNOT_SAVE_KEY` texts), PLAT-120 (Electron's decrypt checks: empty → "", no v10/v11 → refused, failures as errors), PLAT-121 (same format written). Known-answer vectors from Python's hashlib + the openssl tool (macOS) and OpenSSL's EVP AES-256-GCM plus McGrew–Viega cases 13/14 (Windows); synthetic passwords and keys only. The real Keychain and DPAPI are behind traits and never touched by tests.

- PLAT-002 (`app::set_app_user_model_id`, Windows), PLAT-076 (`shell::show_item_in_folder`; Finder `activateFileViewerSelectingURLs`, Explorer `SHOpenFolderAndSelectItems` with an open-the-folder fallback), PLAT-024/025's opener (`shell::open_external`; the `^https?:` check stays with the caller).
- HOST-208/209/210/211, OS side (`notify::Notifier`): macOS UNUserNotificationCenter as Electron 44 does it (delegate set and alert|sound|badge authorization on first use; title, subtitle, body, default sound, thread id = group; banner+list+sound while in front; click = default action → `on_click(id)`; close = remove delivered); Windows toast XML as Electron writes it (tested), group (default `Notifications`), tag = id, `Activated` → click, close = `History.RemoveGroupedTagWithId`. HOST-210: unsupported outside an app bundle (tested). HOST-215, OS side: `notify::set_dock_badge(badge_label(count))` (macOS, main queue).
- VOICE-009 (`permissions::microphone`, AVCaptureDevice status/request, Electron's status names), VOICE-010 (`open_microphone_settings`, verbatim URLs).
- CAP-032/CAP-040/CAP-041 OS side (`accessibility_trusted(prompt)` with `kAXTrustedCheckOptionPrompt`; `request_accessibility()`: `tccutil reset Accessibility dev.saleem.nodus` 5 s for packaged builds only, prompt, open the pane), CAP-055/CAP-059 OS side (`screen_capture_granted` = `CGPreflightScreenCaptureAccess`, `open_screen_capture_settings`).
- **nodus-updater** (tests: 8 Vitest cases ported in `tests/windows.rs`, named after them; plus `tests/mac.rs`, `lookup.rs`, `start.rs`, `swap.rs`, unit tests): UPD-001, UPD-002 (status replaced whole with `current`, sent as `Event::UpdateStatus`), UPD-003…UPD-009 (off reasons verbatim, platform before development, Windows never off, bundle and folder writable), UPD-010 (clean-up before every check), UPD-011 (swap on quit only on macOS), UPD-012 (30 s, then every 4 h from start, setting read per tick; paused-clock test), UPD-015…UPD-029 (busy/staged/off no-ops, `Accept` + `Nodus/<v>` UA, 20 s timeout incl. body, 403/429, other statuses, asset choice incl. arm64, tag and digest rules, `isNewer` with `parseInt` semantics, failed version on automatic vs asked checks, check resolves after the lookup), UPD-030…UPD-036 (UA-only download, 60 s stall re-armed per chunk, percents as JS computes them only on change, status errors incl. no body, damaged, interrupted, generic), UPD-037…UPD-047 (exact `rm`/`hdiutil`/`ditto`/`codesign`/`plutil` argv in order, `mkdtemp` mount, first `*.app`, detach always and only logged, rmdir, bundle id/version/team checks, ad-hoc skip, DMG always deleted, version folder removed on failure), UPD-048…UPD-053 (installer path, no-checksum refusal, kept installer reused by size + SHA-256, deleted on failure), UPD-054…UPD-063 (restart; swap script verbatim, checked byte for byte against updater.ts, and run for real by /bin/sh: swap, rollback, failed marker, signals ignored, waits for the PID, `open -n` / `--env NODUS_USER_DATA=…`), UPD-064…UPD-070 (one install at a time, missing installer stops nothing, the question, Not Now, stop before spawn, failure path, never on quit), UPD-071…UPD-075 (failed kept only while newer, newest newer installer kept on Windows, `mount-*` force-detached before `rm -rf`, errors logged, missing folder fine).
- UPD-005/UPD-021 helpers: `app::is_packaged()` (compile-time `NODUS_PACKAGED=1`, the spec's proposed answer), `app::arm64_machine()` / `running_under_arm64_translation()` (`sysctl.proc_translated`, `IsWow64Process2`), `app::is_in_applications_folder()` / `in_applications_folder()` (Electron's ElectronBundleMover rule, tested), `app::bundle_of(exe)`.

## Deferred

- Real notification delivery, the click callback and the Dock badge on screen: an unsigned dev binary can't deliver (UNErrorDomain 1) and tests have no bundle or run loop. Verify in the packaged test (P10/P11).
- Native question dialogs (UPD-066's `Stop … to update?`, 02's replace-host dialog): they need the UI's window and main thread, so `Updater::restart_to_update` returns `RestartOutcome::Ask(StopSessionsQuestion)` (message, detail, buttons, default 0, cancel 1, verbatim) and the UI calls `answer_stop_sessions(bool)`. A second click while the question is out gets `RestartOutcome::Nothing` (asks once, UPD-064).
- UPD-013 (`settings:update` with truthy `autoUpdate` → `check(false)`), UPD-014 (store default), UPD-076…UPD-079 (IPC and menu wiring): nodus-core / nodus-store / menu. UPD-080…UPD-085: UI. REL-*: P11.
- Choice to confirm (spec 06 "unclear"): when the 20 s lookup timeout fires while the body is read, 0.9.4 shows V8's `The operation was aborted due to timeout`; the port shows `Couldn’t reach GitHub to check for updates.`. A body that isn't JSON shows serde's message (`expected value at line 1 column 1`) where 0.9.4 showed V8's `SyntaxError` text.
- Windows TLS: reqwest uses rustls (webpki roots, as Node bundles its own) everywhere but Windows, where it uses SChannel (native-tls): ring's C code can't be cross-checked for `x86_64-pc-windows-msvc` from a Mac. The proxy is off (`no_proxy`), as Node's fetch ignores the system proxy.
- Windows-only code (job objects, DPAPI, toasts, ShellExecute/SHOpenFolderAndSelectItems, IsWow64Process2, AUMID) passes clippy for `x86_64-pc-windows-msvc` but has only run in CI's Windows job, not here.

## Verified OSCrypt parameters

From Electron 44.5.1's `patches/chromium/revert_oscrypt_remove_sync_backend.patch` (Chromium 152 dropped `os_crypt/sync`; Electron restores it), `feat_ensure_mas_builds_of_the_same_application_can_use_safestorage.patch`, `shell/browser/electron_browser_main_parts.cc`, `shell/browser/api/electron_api_safe_storage.cc`, and Chromium 152.0.7977.130's `components/os_crypt/common/keychain_password_mac.mm`, `crypto/apple/keychain_v2.mm`, `crypto/aes_cbc.h`:

- macOS: prefix `v10`; IV 16 × `' '`; salt `saltysalt`; 1003 iterations; PBKDF2-HMAC-SHA1 (`crypto::kdf::Pbkdf2HmacSha1`); key 16 bytes (`kDerivedKeySize`); AES-CBC with PKCS#7 (`crypto::aes_cbc`); empty plaintext → empty ciphertext; ciphertext without `v10` → failure (not plaintext, as older Chromium did). Keychain: SecItem generic password, service `<app> Safe Storage`, account `<app>` + ` Key`; if missing, the legacy account `<app>` is copied to `<app> Key` and deleted; else `base64(16 random bytes)` is added (`kSecAttrAccessibleWhenUnlocked`, the default). The Keychain is tried once per process; a failure is final until relaunch.
- Windows: prefix `v10` + 12-byte nonce + AES-256-GCM (`crypto::Aead`, empty AAD) ciphertext‖tag; key 32 bytes from `Local State` › `os_crypt.encrypted_key` = base64(`DPAPI` + `CryptProtectData(key)`), protected with `CRYPTPROTECT_AUDIT`, unprotected with flags 0; `os_crypt.audit_enabled` set to true (an older key is re-protected once). No key or a key DPAPI can't open → a new random key replaces it; a value without the `DPAPI` header → no encryption. Data without `v10` → `CryptUnprotectData` directly (legacy).
- Electron's `decryptString`: empty → `""`; neither `v10` nor `v11` → `Ciphertext does not appear to be encrypted.`; `encryptString`/`decryptString` throw when encryption isn't available.

## APIs other tracks use

### `nodus_platform::process` (crates/nodus-platform/src/process.rs)

The one way to start a child process. Covers every `execFile`/`spawn` in `src/main/*.ts`.

```rust
use nodus_platform::process::{Capture, Command, Exit, Signal, NODE_MAX_BUFFER};

// execFile(git, args, { cwd, env: spawnEnv({ GIT_TERMINAL_PROMPT: '0' }), maxBuffer: 32 MiB })
let output = Command::new("git")
    .args(["--no-optional-locks", "status"])
    .cwd(repo)
    .env(env.spawn_env())                 // replaces the environment; omit to inherit the app's
    .env_var("GIT_TERMINAL_PROMPT", "0")  // set over whichever environment
    .max_buffer(32 * 1024 * 1024)         // Node's maxBuffer, per stream: stops it, overflow = Some(stream)
    .output().await?;                     // Err(SpawnError) only when it couldn't start: "spawn git ENOENT"
output.code();       // Option<i32>: None when a signal stopped it (Node's `code`)
output.signal();     // Option<Signal>; Signal::name() == "SIGKILL" (Node's `signal`)
output.stdout_text() // lossy UTF-8; output.stdout is the bytes
output.overflow;     // Some(Stream::Stdout) past maxBuffer
output.timed_out;

// execFile's error semantics (non-zero exit, signal, timeout or maxBuffer all fail):
Command::new("/usr/bin/plutil").args([...]).exec().await?;   // ExecError: "Command failed: …\n<stderr>",
                                                              // "stdout maxBuffer length exceeded", or the SpawnError

// spawn('claude', args, { stdio: ['ignore','pipe','pipe'] }) with a SIGKILL timer:
Command::new(claude).args(args).cwd(dir).env(env.spawn_env())
    .timeout(Duration::from_secs(300)).kill_signal(Signal::KILL)
    .output().await?;

// cancel from elsewhere (capture's guess):
let child = Command::new(claude).args(args).spawn()?;
let killer = child.killer();          // Clone + Send; killer.kill(Signal::TERM) stops the tree
let output = child.wait().await;

// a view's command: own session, keep the first N bytes and let it run on
Command::new(shell.file).args(shell.args).new_session()
    .stdout(Capture::First(n)).stderr(Capture::First(n))
    .timeout(Duration::from_secs(120)).kill_signal(Signal::KILL);

// stdin: closed unless given
Command::new("powershell.exe").args([...]).stdin(json_bytes);

// detached (the session host, the swap script, the Windows installer): returns the pid
Command::new(exe).args(["host", socket, key]).cwd(dir).env(env).spawn_detached(Some(log_file))?;
Command::new(installer).args(["--updated", "/S", "--force-run"]).spawn_detached(None)?;

// Windows only: a raw command line
#[cfg(windows)] Command::new(file).raw_arg(command_line);
```

- Timeouts, `maxBuffer` and a `Killer` stop the **whole tree**: Unix process group (or session with `new_session()`), Windows job object (`taskkill /pid N /t /f` if the job couldn't be set up). The default kill signal is SIGTERM, as Node's `killSignal`.
- Dropping a `Child` or an unfinished `wait()`/`output()` future stops the tree (SIGKILL).
- Windows: every non-detached child gets `CREATE_NO_WINDOW`; detached ones get `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, as libuv. Exit reports `Exit::Signal(sent)` when Nodus stopped it, as libuv does.
- After a stop, pipes held open by a process that left the tree are given up on after 1 s; otherwise `wait()` lasts until the output closes, like Node's `close` event.
- `Capture::First(n)` for a view's output in UTF-16 units: keep `3 * MAX_CHARS` bytes, then `slice_utf16` to `MAX_CHARS`.

### `nodus-updater` (crates/nodus-updater)

```rust
use nodus_updater::{App, RealSystem, RestartOutcome, Updater};

let updater = Updater::new(app /* Arc<dyn App>: version, user_data, auto_update, live_session_count, stop_all, quit */,
                           Arc::new(RealSystem::new(std::env::current_exe()?)), sink /* Arc<dyn EventSink> */);
updater.start();                                  // at launch (PLAT-010 step 7)
updater.status();                                 // updates:status
updater.check(true).await;                        // updates:check, menu Check for Updates…
updater.check(false).await;                       // settings:update with truthy autoUpdate (UPD-013)
match updater.restart_to_update().await {         // updates:install
    RestartOutcome::Ask(q) => { /* show q.message, q.detail, q.buttons; then */ updater.answer_stop_sessions(stop).await; }
    _ => {}
}
updater.on_will_quit();                           // from the normal quit path, after windows close (macOS swap)
```
Events: every status change is `Event::UpdateStatus(UpdateStatus)`. Constants: `RELEASES_URL`, the messages (`UNREACHABLE`, …).

### `nodus_platform::notify`, `shell`, `permissions`, `app`

```rust
use nodus_platform::notify::{Notifier, Notification, badge_label, set_dock_badge};
let notifier = Notifier::new(move |id| on_click(id));   // id of the clicked notification; any thread
if notifier.is_supported() { /* HOST-210 */ }
let id = notifier.show(&Notification { title, subtitle: project_name, body, group: project_id });
notifier.close(&id);                                     // withdraw (HOST-204/209)
set_dock_badge(&badge_label(waiting_count));             // macOS; "" clears

nodus_platform::shell::open_external(url) -> bool;      // https, x-apple.systempreferences:, ms-settings:
nodus_platform::shell::show_item_in_folder(&path);

use nodus_platform::permissions::*;
microphone().await -> bool;            // VOICE-009
open_microphone_settings();            // VOICE-010
accessibility_trusted(false) -> bool;  // CAP-032 (true on Windows)
request_accessibility().await -> bool; // CAP-040/041 OS steps; capture then syncs and returns its state
screen_capture_granted() -> bool;      // CAP-055
open_screen_capture_settings();        // CAP-059

nodus_platform::app::{APP_ID, is_packaged(), set_app_user_model_id(), arm64_machine(), is_in_applications_folder(), bundle_of(exe)};
```

### `nodus_platform::secrets` (crates/nodus-platform/src/secrets.rs)

```rust
use nodus_platform::secrets::{SafeStorage, decode_base64, encode_base64, CANNOT_SAVE_KEY};

let storage = SafeStorage::system(&user_data);       // Keychain (macOS) / Local State + DPAPI (Windows); lazy
storage.is_encryption_available();                    // macOS: first call reads or creates the Keychain item
let sealed = storage.encrypt_string(key.trim())?;     // Err(SafeStorageError) when unavailable
state.voice_key = Some(encode_base64(&sealed));       // Buffer.toString('base64')
let key = storage.decrypt_string(&decode_base64(&state_voice_key)); // Buffer.from(…, 'base64'), lenient
// Err(e) → log "[voice] could not decrypt the saved key {e}" and act as if no key; Ok("") → no key.
// Tests: SafeStorage::with_mac_password(b"synthetic"), SafeStorage::with_windows_key([7; 32]), SafeStorage::unavailable().
```
