# Track: platform-updater (P3)

- **Branch:** `rust/platform-updater`
- **Scope, in order:** `nodus_platform::process` (merged early), secrets (D6), notifications and the Dock badge, open / reveal / permissions / System Settings, then `crates/nodus-updater` (spec 06's UPD behaviors that aren't UI). Packaging is P11.
- **Specs:** 01 (PLAT-002, PLAT-076, PLAT-115…PLAT-121), 02 (notifications, Dock badge: the OS side of HOST-208…HOST-215), 05 (VOICE-009, VOICE-010), 06 (UPD-001…UPD-075), 07 (CAP-032, CAP-040, CAP-041, CAP-055, CAP-059: the OS side).

## Status

| Module | Commit | State |
| --- | --- | --- |
| `nodus_platform::process` | *Start every child process through nodus-platform* | ✅ ready to merge to main on its own |
| `nodus_platform::secrets` | *Read and write the xAI key as 0.9.4's safeStorage does* | ✅ |
| `nodus_platform::notify` | — | ☐ |
| `nodus_platform::{open, permissions}` | — | ☐ |
| `nodus-updater` | — | ☐ |

## Done (implemented and tested)

- PLAT-097 / PLAT-105: the login-shell capture and `claude --version` now run through `process::Command` (10 s timeout, 8 MiB cap / Node's 1 MiB default, the shell killed outright at the timeout as the plan decided).
- PLAT-115 (base64 as Node writes and leniently reads it), PLAT-116 (macOS v10 AES-128-CBC/PBKDF2), PLAT-117 (Keychain `Nodus Safe Storage` / `Nodus Key`, legacy account `Nodus` moved, 24-character random password created), PLAT-118 (Windows v10 AES-256-GCM, `Local State` DPAPI key: read, created, replaced when DPAPI can't open it, audit flag), PLAT-119 (`CANNOT_SAVE_KEY` texts), PLAT-120 (Electron's decrypt checks: empty → "", no v10/v11 → refused, failures as errors), PLAT-121 (same format written). Known-answer vectors from Python's hashlib + the openssl tool (macOS) and OpenSSL's EVP AES-256-GCM plus McGrew–Viega cases 13/14 (Windows); synthetic passwords and keys only. The real Keychain and DPAPI are behind traits and never touched by tests.

## Deferred

- (none yet)

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
