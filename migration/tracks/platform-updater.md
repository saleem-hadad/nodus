# Track: platform-updater (P3)

- **Branch:** `rust/platform-updater`
- **Scope, in order:** `nodus_platform::process` (merged early), secrets (D6), notifications and the Dock badge, open / reveal / permissions / System Settings, then `crates/nodus-updater` (spec 06's UPD behaviors that aren't UI). Packaging is P11.
- **Specs:** 01 (PLAT-002, PLAT-076, PLAT-115…PLAT-121), 02 (notifications, Dock badge: the OS side of HOST-208…HOST-215), 05 (VOICE-009, VOICE-010), 06 (UPD-001…UPD-075), 07 (CAP-032, CAP-040, CAP-041, CAP-055, CAP-059: the OS side).

## Status

| Module | Commit | State |
| --- | --- | --- |
| `nodus_platform::process` | *Start every child process through nodus-platform* | ✅ ready to merge to main on its own |
| `nodus_platform::secrets` | — | ☐ next |
| `nodus_platform::notify` | — | ☐ |
| `nodus_platform::{open, permissions}` | — | ☐ |
| `nodus-updater` | — | ☐ |

## Done (implemented and tested)

- PLAT-097 / PLAT-105: the login-shell capture and `claude --version` now run through `process::Command` (10 s timeout, 8 MiB cap / Node's 1 MiB default, the shell killed outright at the timeout as the plan decided).

## Deferred

- (none yet)

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
