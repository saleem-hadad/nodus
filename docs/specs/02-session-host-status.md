# 02 · Session host & status

Sources: `src/main/host.ts`, `src/main/hostProtocol.ts`, `src/main/hostProtocol.test.ts`, `src/main/pty.ts`, `src/main/hooks.ts`, `src/main/hooks.test.ts`, `src/main/agents.ts`, `src/main/notify.ts`, `src/main/notify.test.ts`, `src/renderer/src/components/runningSessions.ts`, `src/renderer/src/components/runningSessions.test.ts`; wiring in `src/main/ipc.ts` (186–287, 349–353, 886–975, 1037–1044) and `src/main/index.ts` (151–158); running counts in `src/main/board.ts` (115–124, 193–227); callers `src/main/menu.ts` (92–96), `src/main/updater.ts` (350–373); types `src/shared/types.ts`, `src/shared/api.ts`; host probe `scripts/smoke.mjs` (140–161, 278–290); `docs/how-it-works.md` ("Sessions keep running after you quit", "Status", "Agents", "Windows", "Your data", "Known limitations"); `node_modules/node-pty` 1.1.0 (behavior the 0.9.4 host inherits from it). `src/shared/sessions.ts` (`isEmptySession`) is not used by this domain's running counts; it belongs to 03.

## Summary

Nodus never runs sessions in the app process. A **session host** — a detached background process started from the app's own binary — owns every session's pseudo-terminal, so sessions survive the app quitting or crashing. The app talks to it over a Unix socket (macOS) or a named pipe (Windows) with newline-delimited JSON (`PROTOCOL = 1`). The host also runs a token-protected HTTP server on `127.0.0.1` that receives Claude Code's hook POSTs and status-line reports, and relays the board tools' MCP requests to whichever app is connected. It keeps each session's last ~2 M UTF-16 units of output and a history of hooks and status-driving keystrokes, so a relaunched app adopts every live session with its output and an up-to-date status.

On the app side, `pty.ts` connects to (or starts, or asks to replace) the host, tracks live sessions, buffers and coalesces their output, and builds the `claude` command line; `hooks.ts` is the **status machine** (`working` / `waiting` / `done`, per session and per subagent) driven by hooks plus keystroke heuristics (Esc, Ctrl+C, Enter, 1–9), and turns status-line reports into usage; `agents.ts` reads subagent transcripts and meta files; `notify.ts` shows background notifications and the Dock badge; `runningSessions.ts` computes the title bar's running count.

Rule 4 of the plan applies throughout: the Rust host and client must be **wire-compatible with 0.9.4**: a Rust app must adopt a running 0.9.4 Node host, and a 0.9.4 app must adopt a Rust host. Everything in "Data & formats" is normative.

## Behaviors

### Host folder and private files (hostProtocol.ts)

- HOST-001 **Host folder.** `hostPaths(userData)` puts every host file in `<userData>/host` (joined with the platform separator, no trailing separator).
- HOST-002 **Name hash.** `<hash>` is the first 12 lowercase hex characters of the SHA-1 of the UTF-8 bytes of the host folder path string exactly as joined (vectors in Data & formats §2).
- HOST-003 **Socket in folder.** On non-Windows platforms the socket is `<userData>/host/host.sock`.
- HOST-004 **Long-path fallback.** On non-Windows platforms, when the UTF-8 byte length of `<userData>/host/host.sock` is greater than 100, the socket is `<os temp dir>/nodus-<hash>.sock` instead (Node `os.tmpdir()`: `$TMPDIR` without a trailing separator); exactly 100 bytes stays in the folder.
- HOST-005 **Windows pipe.** On Windows the socket is the named pipe `\\.\pipe\nodus-<hash>`, with no length check.
- HOST-006 **Other host files.** `log` = `<userData>/host/host.log`, `hookConfig` = `<userData>/host/hook.curlrc`, `key` = `<userData>/host/host.key` on every platform, even when the socket moved to the temp folder.
- HOST-007 **Private folder.** `ensurePrivateDir(dir)` creates the folder (and missing parents) with mode 0700, then chmods the folder to 0700 even when it already existed with another mode.
- HOST-008 **Private file write.** `writePrivateFile(file, text)` makes the parent folder private, writes `text` as UTF-8 to `<file>.<pid>.tmp` (pid of the writing process) with mode 0600, then renames it over `file`; no temp file remains.
- HOST-009 **Hook config token check.** `writeHookConfig(paths, token)` throws `The session host’s token can’t go in a curl config` when the token doesn't match `^[\w-]+$` (ASCII `A–Z a–z 0–9 _ -`, non-empty), leaving any existing file unchanged.
- HOST-010 **Hook config content.** The file holds exactly `header = "Authorization: Bearer <token>"` followed by one `\n`; writing again replaces the token.
- HOST-011 **Trust on macOS.** `trustsHost(key)` is true on every non-Windows platform, whatever the key.
- HOST-012 **Trust on Windows.** On Windows `trustsHost(key)` is true only when `key` is a non-empty string equal (exact, case-sensitive, untrimmed) to the whole UTF-8 contents of `host.key`; a missing or unreadable key file, or a missing, non-string or empty key, gives false.

### Wire framing (hostProtocol.ts)

- HOST-013 **Protocol number.** `PROTOCOL` is `1`; `hello`, `welcome.protocol` and `shutdown` keep their shape in every version so any app can recognise and stop any host.
- HOST-014 **Line framing.** Both directions send one JSON value per message, serialized without raw newlines, followed by one `\n` (0x0A), UTF-8 encoded.
- HOST-015 **Several per read.** A reader delivers every complete line of a chunk, in order.
- HOST-016 **Split messages.** A message split across reads is delivered once its terminating `\n` arrives, and not before.
- HOST-017 **Split characters.** UTF-8 is decoded statefully across reads, so a multi-byte character split at any byte boundary arrives intact; invalid byte sequences decode to U+FFFD.
- HOST-018 **Blank and bad lines.** Empty lines are skipped and lines that aren't valid JSON are dropped silently; reading carries on with the next line.
- HOST-019 **Newlines inside strings.** String values containing `\n` or `\r\n` round-trip intact (escaped inside the JSON).
- HOST-020 **No line limit.** There is no maximum line length (a welcome with several 2 M-unit buffers is one line).
- HOST-021 **Any JSON value.** The reader hands on whatever value a line parses to (object, number, `null`, array…); shape checks are the consumer's.

### Host process: startup (host.ts)

- HOST-022 **Arguments.** The host takes two positional arguments, the socket path and the key path; the app always passes both, on every platform (0.9.4: `<Electron binary> <app>/out/main/host.js <socket> <key>` with `ELECTRON_RUN_AS_NODE=1`; Rust: `Nodus host <socket> <key>`). Rust only, not on Windows: `host --socket <path>` runs a host nobody launches (hosted Nodus, SRV-004): it makes the socket's folder when missing (0750), chmods the socket 0660 instead of 0600, fails rather than fall back to the temp folder when the path is too long for a socket, skips HOST-032 and the no-app half of HOST-047, and, after the welcome, stops every session and exits (HOST-064) when a hello has another protocol.
- HOST-023 **Usage error.** Without a socket path, or on Windows without a key path, the host logs `failed to start` with the error `usage: host <socket path> <key path>` and exits with code 1.
- HOST-024 **One host per socket.** At startup the host connects to its socket path; if the connection succeeds it closes it without sending anything, logs `another host is running`, and exits with code 0.
- HOST-025 **Stale socket.** Otherwise it deletes the file at the socket path, ignoring any error.
- HOST-026 **Hook server first.** It then starts the hook HTTP server on `127.0.0.1` port 0 (OS-chosen); the bound port is `hookPort`; a listen failure logs `failed to start` and exits with code 1.
- HOST-027 **Socket listen.** Then it listens on the socket path or pipe name; a failure (e.g. on Windows another host already owns the pipe) logs `failed to start` and exits with code 1.
- HOST-028 **Socket mode.** On non-Windows platforms the socket file is chmod 0600 straight after listening.
- HOST-029 **Windows key.** On Windows the host makes a key from 32 random bytes as 64 lowercase hex characters, and writes it (no newline) to the key path with `writePrivateFile` after the pipe is listening and before any welcome can go out; elsewhere it writes no key file.
- HOST-030 **Hook token.** Each host process makes its hook token at start from 16 random bytes as 32 lowercase hex characters.
- HOST-031 **Ready log.** Once listening it logs `listening <socket path> hooks on <hookPort>`.
- HOST-032 **Unused host.** 30 000 ms after startup, if no app client (HOST-038) is connected, the host applies the idle rule (HOST-047).
- HOST-033 **Crash-proof.** An uncaught exception is logged as `uncaught <error>` and an unhandled rejection as `unhandled <error>`; the host keeps running.
- HOST-034 **Log lines.** Each log line is the current time as ISO-8601 UTC with milliseconds (`YYYY-MM-DDTHH:mm:ss.sssZ`), a space, then the arguments joined by single spaces (strings verbatim, numbers in decimal, errors as their stack), written to stdout, which the app points at `host.log`.
- HOST-035 **Background process.** The host shows no Dock icon, window or menu, and keeps running when the app quits or crashes.

### Host: connections (host.ts)

- HOST-036 **Hello first.** If the first message on a connection isn't `{"type":"hello",…}` the host closes the connection without replying.
- HOST-037 **Welcome.** On `hello` the host replies `welcome` whatever the hello's `protocol`: `protocol` 1, `hookPort`, `hookToken`, every live session in spawn order (with buffer, history and status line), `mcp: true`, `hookAuthHeader: true`, and `key` on Windows only (absent elsewhere).
- HOST-038 **App clients.** Only a hello with `protocol` equal to 1 makes the connection an app client; the host logs `app connected ` (with a trailing space) for it and `app connected (protocol <n>)` for any other protocol.
- HOST-039 **Other protocols may only stop it.** On a greeted connection that isn't an app client the host acts on `shutdown` only and ignores every other message.
- HOST-040 **Ignored messages.** A second `hello`, an unknown `type`, or anything else the host can't act on is ignored; an exception while handling a message is logged as `message failed <error>`.
- HOST-041 **Spawned reply.** `spawned` is written only to the connection that sent the `spawn`.
- HOST-042 **Broadcasts.** `data`, `exit` and `hook` are written to every app client, never dropped (buffered without limit).
- HOST-043 **Socket errors.** Errors on a connection are ignored.
- HOST-044 **Disconnect.** When an app client's connection closes, the host logs `app disconnected`, and every MCP relay waiting on that client settles unanswered.
- HOST-045 **Idle check on close.** When a connection that said hello closes, the host applies the idle rule; a connection that closes without a hello (another host's probe) doesn't trigger it.
- HOST-046 **Newest app.** With several app clients, MCP requests go to the one that connected most recently.
- HOST-047 **Idle rule.** The host exits with code 0, logging `idle, exiting`, when it has no sessions and either no app client is connected or it is stopping; a host with live sessions and no app keeps running indefinitely.

### Host: sessions and pseudo-terminals (host.ts)

- HOST-048 **Duplicate spawn.** A `spawn` for a session id the host already runs starts nothing and replies `spawned` without `error`.
- HOST-049 **PTY spawn.** Otherwise the host starts `file` in a new pseudo-terminal of `cols` × `rows` named `xterm-256color`, in `cwd`, with exactly the environment `env` (on POSIX plus `PWD=<cwd>` and `TERM=xterm-256color`, overriding `env`).
- HOST-050 **POSIX argv.** On POSIX the process argv is `[file, ...args]`, each element passed unchanged.
- HOST-051 **Windows command line.** On Windows the command line is the node-pty-quoted `file`, a space, and `windowsCommandLine(args)`; with no args it is just the quoted `file` (Data & formats §13).
- HOST-052 **Spawn error.** When starting the process fails synchronously, the reply is `spawned` with `error` set to the error's message (e.g. `posix_spawnp failed.`, `File not found: <path>`), and no session is kept.
- HOST-053 **Late spawn failure (macOS).** On macOS a `cwd` that can't be entered, or a `file` that can't be executed, is not a spawn error: the session spawns, prints nothing, and exits with code 1.
- HOST-054 **Spawn log.** Each started session logs `spawned <sessionId> <file>`.
- HOST-055 **Output.** Output is decoded as UTF-8 (statefully, invalid bytes as U+FFFD) into strings; each chunk is appended to the session's buffer and broadcast as `data`.
- HOST-056 **Output buffer.** After each append, while the buffer's total length in UTF-16 code units exceeds 2 097 152 (2 × 1024 × 1024) and it holds more than one chunk, the oldest chunk is dropped (whole chunks only; the newest chunk is always kept, however long).
- HOST-057 **Exit.** When a session's process exits, after all its output has been broadcast, the host cancels its SIGKILL timer, forgets the session, logs `exited <sessionId> <exitCode>`, broadcasts `exit`, and applies the idle rule.
- HOST-058 **Exit code.** `exitCode` is the process's exit status; on POSIX a process ended by a signal reports 0.
- HOST-059 **Write.** `write` for an unknown session, or whose `data` isn't a string, is ignored; otherwise `data` is written to the pseudo-terminal as UTF-8.
- HOST-060 **Status keys in history.** After writing, if `data` matches `^(?:\r|\x1b|\x03|[1-9])$` exactly, the host appends `{input: data, at: <now, ms>}` to the session's history.
- HOST-061 **Resize.** `resize` for an unknown session is ignored; errors from resizing (exited process, non-positive or non-finite size) are swallowed.
- HOST-062 **Kill on POSIX.** `kill` sends SIGTERM to the session's child process id (not its process group), ignoring errors, and, unless a SIGKILL timer is already pending for that session, sends SIGKILL 3000 ms later; an unknown session id is ignored.
- HOST-063 **Kill on Windows.** On Windows `kill` sends no signal: it terminates every process attached to the session's console and closes the pseudo-console, with no SIGKILL timer.
- HOST-064 **Shutdown.** `shutdown` logs `stopping every session`, marks the host stopping, sends every session SIGTERM then SIGHUP (POSIX; Windows as HOST-063) with the 3000 ms SIGKILL timer, applies the idle rule at once, and exits with code 0 after 4000 ms whatever is left.
- HOST-065 **Stopping host exits.** Once stopping, the host exits when its last session exits, even with app clients connected.
- HOST-066 **History cap.** Each session's history holds at most 1000 entries; the oldest are dropped first.
- HOST-067 **Slim payloads.** A hook entry in history keeps only the payload's top-level number and boolean fields and its string fields of at most 1024 UTF-16 code units, in their original order; objects, arrays, `null` and longer strings are left out.

### Host: hook endpoint (host.ts)

- HOST-068 **MCP path first.** A request whose path is exactly `/mcp` (any query, any method) goes to the MCP relay before any hook check.
- HOST-069 **Token sources.** The hook token is taken from an `Authorization` header matching `^Bearer (\S+)$`, otherwise from the `token` query parameter (processes started by an older app).
- HOST-070 **Constant-time compare.** A token matches only when it has the same byte length as the host's token and compares equal in constant time.
- HOST-071 **Rejections.** Any request other than `POST` to path `/hook` with a matching token is answered `404` with an empty body, and its body is drained.
- HOST-072 **Accept first.** An accepted hook request's body is read to its end and answered `204` with no body before it is processed.
- HOST-073 **Unknown target.** When the `evt` query parameter is missing or empty, or `sid` names no live session, nothing more happens.
- HOST-074 **Payload.** A non-empty body is parsed as JSON (whole body decoded as UTF-8); an empty body or one that doesn't parse gives the payload `{}`.
- HOST-075 **Status line.** For `evt` = `StatusLine` the session's status line becomes `{payload, at: <now>}`, replacing the previous one; it isn't added to history.
- HOST-076 **History.** Any other `evt`, known hook name or not, is appended to history as `{hook: evt, payload: <slimmed>, at: <now>}`.
- HOST-077 **Broadcast.** The host then broadcasts `hook` with the session id, `event` = evt, and the full, unslimmed payload.
- HOST-078 **Null payload.** A body of `null` with any event other than `StatusLine` makes the host log `uncaught <TypeError>` and neither record nor broadcast it (the `204` has already gone out).

### Host: MCP relay (host.ts)

- HOST-079 **MCP auth.** `/mcp` requires a matching token in the `Authorization: Bearer` header (the URL token isn't accepted) and an `X-Nodus-Session` header naming a live session; otherwise it answers `401` with an empty body and drains the request.
- HOST-080 **Non-POST.** With valid auth, `DELETE` is answered `204` and any other non-POST method `405` with the header `Allow: POST`, both with empty bodies.
- HOST-081 **Size cap.** As the POST body arrives it is decoded as UTF-8; as soon as its length exceeds 1 048 576 UTF-16 code units the answer is `413` with an empty body.
- HOST-082 **Parse error.** A body that isn't JSON is answered `200`, `Content-Type: application/json`, body `{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}`.
- HOST-083 **Requests.** The requests are the items (the elements of a batch array, or the single value) that are objects with an `id` key (any value, `null` included) and a string `method`; other items (notifications) are dropped.
- HOST-084 **Notifications only.** With no requests the answer is `202` with an empty body.
- HOST-085 **Relay.** Each request is relayed, concurrently, to the newest app client as `mcp` with a fresh relay id (1, 2, 3… per host process), the session id and the request.
- HOST-086 **Relay settles.** A relay settles with the `reply` of the first `mcp-reply` carrying its id (from any app client), or with nothing after 20 000 ms, or with nothing when its client disconnects.
- HOST-087 **No app.** With no app client connected every request is unanswered at once.
- HOST-088 **Unanswered.** A missing or `null` reply is replaced: for `tools/call` by `{"jsonrpc":"2.0","id":<id>,"result":{"content":[{"type":"text","text":"Nodus isn’t open, so the board can’t be reached right now."}],"isError":true}}`, for any other method by `{"jsonrpc":"2.0","id":<id>,"error":{"code":-32603,"message":"Nodus isn’t open, so the board can’t be reached right now."}}` (also after a timeout, while the app is open).
- HOST-089 **Answer.** The answer is `200` with `Content-Type: application/json`: the array of answers in request order for a batch, otherwise the single answer.
- HOST-090 **Relay failure.** An exception in the MCP handler logs `mcp request failed <error>` and ends the response, with status `500` if none was sent yet.

### App: starting and connecting to the host (pty.ts)

- HOST-091 **Starting a host.** To start a host the app makes the host folder private, opens `host.log` for writing (truncating it), and starts the host detached (own session/process group; on Windows detached with no window) with stdin closed, stdout and stderr on `host.log`, the host folder as working directory and the app's environment (0.9.4 adds `ELECTRON_RUN_AS_NODE=1`), keeping no handle to it.
- HOST-092 **Waiting for it.** After starting it the app tries to connect every 100 ms, at most 100 times (10 000 ms), then fails with `The session host didn't start. See <host.log path>`.
- HOST-093 **One attempt at a time.** `connectHost(start)` returns at once when connected; while an attempt is in flight every caller gets that attempt, whatever `start` it passed.
- HOST-094 **Connect order.** An attempt first connects to a running host and greets it; a greeting that resolves false runs the replace dialog (HOST-102); then, when no host is connected and `start` is true, it starts a host and greets it, failing with `The session host speaks another protocol` if that greeting resolves false.
- HOST-095 **Launch adopts, never starts.** At launch the app calls `connectHost(false)`: a running host is adopted, a missing one just means no sessions, and an error is logged as `[host] connect failed <error>` without stopping launch; only a session start starts a host.
- HOST-096 **Hello.** On connecting the app sends `{"type":"hello","protocol":1}`.
- HOST-097 **Welcome timeout.** If no message arrives within 5000 ms the app closes the connection and the greeting fails with `The session host didn’t answer`; it fails the same way if the connection closes before any message.
- HOST-098 **Trust first.** The first message is checked with `trustsHost(<its key if it is a welcome>)` before anything else; an untrusted host is disconnected and the greeting fails with `Another program answered as the session host, so Nodus didn’t trust it with your sessions.`
- HOST-099 **Other protocol.** A trusted first message that isn't a `welcome`, or whose `protocol` isn't 1, resolves the greeting false and leaves the connection open.
- HOST-100 **Adopted in order.** On a good welcome the connection becomes the host connection and the welcome is adopted before any later message is handled, including messages that arrived in the same read.
- HOST-101 **Later messages.** Every message after the welcome is handled as in HOST-114 to HOST-126.
- HOST-102 **Replace dialog.** For a host on another protocol the app shows a warning message box with message `Sessions from another version of Nodus are still running`, detail `This version can’t connect to them. Stop them to carry on, or quit and let them finish.`, and buttons `Stop Sessions` (index 0, default) and `Quit` (index 1, cancel); this can happen at launch.
- HOST-103 **Quit.** `Quit` closes the connection and exits the app at once with code 0, without its quit handlers.
- HOST-104 **Stop Sessions.** `Stop Sessions` sends `{"type":"shutdown"}` and waits, with no timeout, for the host to close the connection, then the attempt carries on (HOST-094).
- HOST-105 **Adopt: hook config.** When the welcome has `hookAuthHeader: true` the app writes `hook.curlrc` with the welcome's token; on failure it logs `[host] could not write the hook config <error>` and goes on without a config path.
- HOST-106 **Adopt: endpoint.** The app sets the hook endpoint to the welcome's `hookPort`, `hookToken` and the config path (or none).
- HOST-107 **Adopt: board tools.** The app records whether the host relays the board tools: `welcome.mcp === true`.
- HOST-108 **Adopt: sessions.** For each welcome session, in order, the app tracks it as live with `buffer` as its replay buffer (a single chunk, or none when empty), starts its status at `initial`, replays its `history` oldest first (hook entries through the status machine and input entries through the keystroke rules, each with its recorded `at`), then replays `statusLine`, if present, as a `StatusLine` event with its `at`.
- HOST-109 **Host lost.** When the host connection closes, the app forgets it, fails every pending spawn with `The session host stopped`, and, unless it is quitting, reports every tracked session exited with code -1.
- HOST-110 **No reconnect.** The app never reconnects by itself; the next session start connects to or starts a host.
- HOST-111 **Quit leaves sessions running.** On quit the app marks itself quitting and closes its end of the connection; the host and its sessions keep running and nothing is reported exited.
- HOST-112 **Stop all.** `stopAll()` (Quit and Stop Sessions, Windows update), when connected, marks quitting, sends `shutdown`, waits until the connection closes or 5000 ms pass, clears quitting, and, if the connection was lost, reports every session still tracked exited with code -1 (after a timeout with the host still connected, sessions stay tracked).

### App: sessions (pty.ts)

- HOST-113 **Spawn once.** `spawnSession` does nothing for a live session, and concurrent calls for the same session share one start.
- HOST-114 **Spawn request.** A start first connects (starting a host if needed), then builds the command (HOST-129), then sends `spawn` with a request id counting from 1 per app run, `initial` = `working` when there is an initial prompt else `done`, `cols` = max(20, cols ?? 120) and `rows` = max(5, rows ?? 36), not rounded.
- HOST-115 **Spawned.** A `spawned` with `error` fails the start with that message; otherwise the session is tracked as live, before any of its output is handled, and the start succeeds; a `spawned` with an unknown id is ignored.
- HOST-116 **Output of unknown sessions.** `data` for a session that isn't tracked is dropped.
- HOST-117 **App buffer.** The app keeps each live session's output with the host's limit and trimming: 2 097 152 UTF-16 code units, whole oldest chunks dropped while more than one remains.
- HOST-118 **Coalescing.** Output reaches the UI as `ev:pty-data` at most once per 16 ms per session: the first pending chunk starts a 16 ms timer (later chunks don't restart it), which emits everything pending as one string.
- HOST-119 **Snapshot.** `snapshot(sessionId)` first emits anything pending as `ev:pty-data`, then returns the whole buffer joined; `''` for a session that isn't live; everything emitted before it returns is part of it.
- HOST-120 **Exit.** On `exit` the app emits any pending output, stops tracking the session, ends its process in the status machine (which may emit `ev:status` and `ev:usage`), emits `ev:pty-exit(sessionId, exitCode)`, then resolves every `kill` waiting on it (waiters resolve even for a session it wasn't tracking).
- HOST-121 **Write.** `write` does nothing for a session that isn't live; otherwise it passes the data to the keystroke rules (HOST-160) first, then sends `write`.
- HOST-122 **Send prompt.** `sendPrompt(sessionId, text)` writes `\x1b[200~` + text + `\x1b[201~`, then writes `\r` 200 ms later.
- HOST-123 **Resize.** `resize` does nothing for a session that isn't live or a non-finite size; otherwise it sends `cols` = max(20, floor(cols)) and `rows` = max(5, floor(rows)).
- HOST-124 **Kill.** `kill` resolves at once for a session that isn't live; otherwise it sends `kill` and resolves when the session exits or after 4000 ms, whichever comes first.
- HOST-125 **Kill a project.** `killProject(projectId)` kills every live session of that project without waiting.
- HOST-126 **Board tools requests.** For each `mcp` message the app asks the board tools (04, `answerMcp`) and sends `mcp-reply` with the same id and the awaited answer; when answering throws it logs `[board-tools] <error>` and sends `mcp-reply` without `reply`, as it does for an undefined answer.
- HOST-127 **Sends while disconnected.** A message sent while no host is connected is dropped.
- HOST-128 **Live list.** `isLive(id)` and `liveSessionIds()` (in tracking order) reflect the tracked sessions.
- HOST-129 **Claude command (shared with 03).** A Claude card runs the resolved `claude` (01 `requireClaude`, whose error propagates) with: `--resume <id>` when resuming, else `--session-id <id>`; `--name <name>` when named; `--permission-mode <mode>` (default `acceptEdits`); when the host relays board tools `--mcp-config <BOARD_MCP_CONFIG> --allowedTools <rule>…`; the extra args; `--settings <hook settings JSON>`; and, with an initial prompt, `-- <prompt>`, in that order.
- HOST-130 **Claude environment.** A Claude card's environment is the spawn environment (01 `spawnEnv`) plus `CLAUDE_CODE_GIT_BASH_PATH=<Git Bash path>` when Git Bash was found and the user's environment doesn't set that variable, plus the hook variables (HOST-176), the added variables winning.
- HOST-131 **Terminal command.** A terminal card runs the user's shell (01 `terminalShell`) with the plain spawn environment: no hook variables and no settings.

### Status machine (hooks.ts)

- HOST-132 **Three statuses.** Each session has a main-thread status — `working`, `waiting` or `done` — and a status for each agent its current process has reported through hooks.
- HOST-133 **Combined status.** A live session reads `waiting` when its main thread or any agent is waiting, else `working` when any is working, else `done`; a session that isn't live reads `done`.
- HOST-134 **Only changes.** `ev:status(sessionId, status, live)` is emitted only when the (status, live) pair differs from the last one emitted for that session.
- HOST-135 **Process start.** When a session's process starts or is adopted, its interrupt timer is cancelled, the main thread takes the initial status, its agents are forgotten, its status is emitted with live = true, and its usage resumes (HOST-172).
- HOST-136 **Process end.** When a session's process exits, its interrupt timer is cancelled, the main thread becomes `done`, its agents are forgotten, its status is emitted with live = false, and its usage pauses (HOST-172).
- HOST-137 **Forget.** `forgetStatus(sessionId)` (card removed, run finished, brain stopped) drops the session's main status, last emitted pair, last hook time, agents, usage and usage key; a pending interrupt timer is left to fire harmlessly.
- HOST-138 **Known events.** Only `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `SessionEnd`, `SubagentStart` and `SubagentStop` drive status; `StatusLine` drives usage; any other event is ignored.
- HOST-139 **Agent hooks.** A hook whose payload has a non-empty string `agent_id` drives that agent only, and doesn't count as a main-thread hook.
- HOST-140 **Last hook time.** Every other known hook records its time as the session's last main-thread hook time.
- HOST-141 **Working.** `UserPromptSubmit` and `PostToolUse` set the main thread `working`.
- HOST-142 **Tool use.** `PreToolUse` sets it `waiting` when `tool_name` is `AskUserQuestion`, else `working`.
- HOST-143 **Permission.** `PermissionRequest` sets it `waiting`.
- HOST-144 **Notification.** `Notification` sets it `waiting` only when it is about permission (HOST-145) and no agent is waiting; otherwise it changes nothing.
- HOST-145 **About permission.** A notification is about permission when `String(notification_type ?? '')` is `permission_prompt` or `elicitation_dialog`, or `String(message ?? '')` contains `permission` in any case.
- HOST-146 **Stop.** `Stop` sets the main thread `done`.
- HOST-147 **SessionStart.** `SessionStart` sets it `done` unless it is `working`.
- HOST-148 **SessionEnd.** `SessionEnd` with `reason` `clear` sets it `done`; with any other reason it sets it `done` and emits live = false, though the process may still be running.
- HOST-149 **Subagent events without an agent.** `SubagentStart` and `SubagentStop` without `agent_id` change no status (they only record the hook time).
- HOST-150 **Agent stop.** An agent's `SubagentStop` makes it `working` when `agent_transcript_path` is a string whose transcript has background work pending (HOST-189), else `done`.
- HOST-151 **Agent waiting.** An agent's `PermissionRequest`, its `PreToolUse` for `AskUserQuestion`, and its `Notification` about permission make it `waiting`; its other `Notification`s change nothing.
- HOST-152 **Agent working.** An agent's `SubagentStart`, `PostToolUse`, `UserPromptSubmit` and other `PreToolUse` make it `working`; its `Stop`, `SessionStart` and `SessionEnd` are ignored.
- HOST-153 **Agent record.** Each agent keeps its status, the time of its last hook (`at`), and since when it has had its status (`since`, kept while the status repeats).
- HOST-154 **Agent events.** `ev:agent-status(sessionId, agentId, status)` is emitted when an agent's status changes, including the first time it is seen, and the session's combined status is emitted after it if it changed.
- HOST-155 **Longest waiting.** The agent that has waited longest is the waiting agent with the smallest `since`; on a tie, the one first seen.
- HOST-156 **Settle.** `settleAgent` sets a known agent `done`, at the current time, unless it already is.
- HOST-157 **Hold.** `holdAgent` sets a known `done` agent `working`, at the current time.
- HOST-158 **Hook times.** A live hook is stamped with the app's clock when it arrives; replayed hooks and keystrokes keep the time the host recorded.
- HOST-159 **Untracked sessions.** Hooks for a session the app isn't tracking still update its stored status, but it reads, and is emitted as, `done` with live = false.

### Keystroke rules (hooks.ts `noteUserInput`)

- HOST-160 **When keys count.** A write is considered only while the main thread is `working` or `waiting`, or an agent is waiting.
- HOST-161 **What a key is.** Only a write that is exactly `\r`, one of `1`–`9`, `\x1b` or `\x03` counts; pastes, multi-character writes and `0` don't.
- HOST-162 **Answering an agent.** When the main thread isn't `waiting` and an agent is, a key sets the longest-waiting agent `working`, stamped with the keystroke's time, and nothing else happens.
- HOST-163 **Interrupt.** Otherwise Esc (`\x1b`) or Ctrl+C (`\x03`) (re)starts a 1200 ms timer for the session; when it fires, if no main-thread hook arrived strictly after the keystroke's time and the main thread is `working` or `waiting`, the main thread becomes `done`.
- HOST-164 **Answering the main thread.** Otherwise, while the main thread is `waiting`, Enter (`\r`) or `1`–`9` sets it `working`.
- HOST-165 **Replayed interrupt.** An Esc or Ctrl+C replayed from history after a relaunch starts a fresh 1200 ms timer then, judged against its recorded time and the replayed hooks.

### Usage from the status line (hooks.ts)

- HOST-166 **Usage report.** A `StatusLine` event becomes the session's usage, stamped with the event's time (`at`).
- HOST-167 **Model.** `model` is `model.display_name`, else `model.id`, as a string, and absent when both are empty or missing (a missing or `null` `model` counts as `{}`).
- HOST-168 **Context tokens.** `contextTokens` is the sum of `input_tokens`, `cache_creation_input_tokens` and `cache_read_input_tokens` of `context_window.current_usage` (a missing or non-finite part counts 0) when `current_usage` is an object, and absent when it is `null` or missing.
- HOST-169 **Context size and share.** `contextSize` is `context_window.context_window_size` when a finite number; `contextPercent` is `context_window.used_percentage` when a finite number, else `contextTokens / contextSize × 100` when both are known and the size isn't 0, else absent.
- HOST-170 **Cost and time.** `costUsd` is `cost.total_cost_usd` and `durationMs` is `cost.total_duration_ms`, each only when a finite number.
- HOST-171 **Usage dedupe.** Usage is always stored, but `ev:usage(sessionId, usage)` is emitted only when (model, contextPercent, contextTokens, contextSize, cost rounded to whole cents, halves rounding up as JS `Math.round`) differs from what was last emitted for the session; a report that only moved the clock isn't sent.
- HOST-172 **Pause and resume.** When a session with usage starts a process, its usage is emitted again with `at` = now and duration unchanged, and its dedupe key is cleared; when its process exits, `durationMs` (if known) grows by now − `at`, `at` becomes now, and it is emitted.
- HOST-173 **Usage kept.** Usage outlives the process and is dropped only by `forgetStatus`.

### Hook endpoint and settings (hooks.ts)

- HOST-174 **Endpoint.** The hook endpoint (port, token, optional curl config path) is the adopted host's; before any host it is port 0 and an empty token.
- HOST-175 **Settings per process.** Every Claude process gets the same `--settings` JSON on a given host; the user's own settings still apply, but flag settings outrank them, so Nodus's status line replaces theirs.
- HOST-176 **Hook environment.** `hookEnv(sessionId)` is `NODUS_PORT` (the port in decimal), `NODUS_TOKEN` (the token), `NODUS_SESSION` (the session id), and `NODUS_HOOK_CONFIG` (the curl config path) only when a config path is set.
- HOST-177 **POSIX hooks with a config.** With a config path, each command is the curl command that reads the token from `$NODUS_HOOK_CONFIG` (Data & formats §10), so the token never appears in the settings or the command line.
- HOST-178 **POSIX hooks for an older host.** Without a config path, the curl command puts `token=$NODUS_TOKEN` in the URL.
- HOST-179 **PowerShell hooks.** On Windows without Git Bash each command is the PowerShell command (token from `$env:NODUS_TOKEN` in an `Authorization` header) with `"shell":"powershell"`, whether or not a config path is set.
- HOST-180 **Settings JSON.** `hookSettings()` registers each event of HOST-138, in that order, as `[{"hooks":[{"type":"command","command":…,"timeout":5}]}]` (plus `"shell":"powershell"` after `command` for PowerShell), and the status line as `{"type":"command","command":<StatusLine command>,"padding":0}`, byte-identical to Data & formats §10.
- HOST-181 **Settings cache.** The JSON is built once per (config path set, PowerShell) pair and reused.
- HOST-182 **Silent hooks.** Every hook and status-line command prints nothing and exits 0 whatever happens (UserPromptSubmit output would reach Claude's context; an empty status line leaves the terminal its row).

### Agents (agents.ts)

- HOST-183 **Agent files.** A session's agents are the files `agent-<id>.jsonl` (`<id>` matching `[\w-]+`) in `<transcript dir>/<session id>/subagents/`, each with `agent-<id>.meta.json` beside it (listing by 03 `listAgentFiles`).
- HOST-184 **Meta file.** The meta file is read as JSON (`agentType`, `description`, `name`, `parentAgentId`, `finished`, `stoppedByUser`) and cached by mtime; a parsed value that isn't an object counts as `{}`, a parse failure (mid-rewrite) keeps the previous parse or none, and a missing file drops the cache.
- HOST-185 **Incremental parse.** A transcript is parsed incrementally and cached by (size, mtime): unchanged, the cache is used; grown, parsing continues after the last complete line; shrunk, or the same size with a new mtime, it starts over; it reads 4 MiB (4 × 1024 × 1024) at a time, doubling the read while a single line is longer, and stops at an incomplete last line; a file that can't be opened keeps the previous parse or none, and a missing one is skipped.
- HOST-186 **Times.** An agent's `startedAt` and `updatedAt` are the earliest and latest parseable `timestamp` of any line.
- HOST-187 **Background tasks.** A user line's non-empty string `toolUseResult.backgroundTaskId` adds a pending task; a task-notification line (03 `isTaskNotification`) removes each task named in a `<task-id>…</task-id>` in its text (trimmed) and clears the agent's end.
- HOST-188 **New prompt.** A user line whose content is a string, or an array without a `tool_result` block, and which isn't `isMeta`, clears the agent's end.
- HOST-189 **Pending work.** `hasPendingWork(path)` is true only for a path that, resolved to an absolute path, is under `~/.claude/projects/`, ends in `.jsonl`, and whose parse has pending background tasks.
- HOST-190 **Steps.** An assistant line adds its tool uses (the handback tool `SubagentHandback` excluded) to `toolCount`; with tool uses the latest step becomes the last tool's name and detail (03 `describeToolUse`) and the agent's end clears; without, any text becomes the latest step's reply line (HOST-191); a handback, or `stop_reason` `end_turn` with no tool use, sets the end to the line's time (now if it has none).
- HOST-191 **Reply line.** A reply line drops blank lines and Markdown heading lines (`^\s{0,3}#{1,6}\s`), joins the rest with spaces (the whole text if nothing is left), removes `**`, `__` and `` ` ``, then collapses whitespace and truncates to 200 characters ending in `…` (03 `truncate`).
- HOST-192 **End time.** An agent's end time is the later of its transcript's end and, when the meta file says `finished: true` or `stoppedByUser: true`, the meta file's mtime (ms, fractional); absent when neither is known.
- HOST-193 **Status: not live.** An agent of a session that isn't live is `done`.
- HOST-194 **Status: unheard.** An agent no hook has reported in the session's current process is `done`.
- HOST-195 **Status: woken later.** An agent whose transcript ended its turn with background tasks pending is `working`; if its hook status was `done` it is held `working` (HOST-157, emitting `ev:agent-status`).
- HOST-196 **Status: ended without a stop.** Otherwise an agent whose hook status is `done` is `done`; one whose end time is at or after its last hook is settled `done` (HOST-156, emitting `ev:agent-status`); any other has its hook status.
- HOST-197 **Agent info.** Each agent reads as `agentId`; `agentType` = meta `agentType` or `agent`; `description` = meta `description`, else `name`, else `Agent`; `status`; `parentAgentId` when non-empty; `stopped: true` only when `stoppedByUser`; `tool` and `activity` from the latest step; `background` = the number of pending tasks when `working` with its turn ended (absent when 0); `toolCount`; `startedAt` (first timestamp, else the transcript's mtime); `updatedAt` (last timestamp, else `startedAt`).
- HOST-198 **Agents list.** `session:agents` returns every agent of the session, oldest `startedAt` first (stable).
- HOST-199 **Agents on cards.** A non-terminal card with agent files gets `agentCount` = the number of files and, unless archived, `agents` = those not `done` or updated at or after the session's last prompt time (0 if unknown), only when there are any.
- HOST-200 **Agent transcript ids.** `session:agent-transcript` fails with `Not an agent of this session` unless both the agent id and the session id match `^[\w-]+$`, and reads `<subagents dir>/agent-<agentId>.jsonl` as an agent transcript (03 `readTranscript`).
- HOST-201 **Agents refresh.** Nothing here watches agent files: the project watcher (03) emits `ev:board-changed`, and the board reload reads agents again.

### Notifications and Dock badge (notify.ts)

- HOST-202 **Notice rules.** A change to `waiting` is a "waiting" notice unless the session's previous status was `waiting`; a change from `working` to `done` while live is a "finished" notice; nothing else is a notice.
- HOST-203 **Previous status.** The notifier remembers each session's last emitted status while it is live, and forgets it when an emission says it isn't.
- HOST-204 **Withdraw.** When a session leaves `waiting`, the notification shown for it is closed if it was a "waiting" one; a "finished" one stays.
- HOST-205 **Gates.** A notice is shown only once notifications are armed, the `notify` setting is on, and the app is in the background (no window, a destroyed one, or one without focus).
- HOST-206 **Armed after catch-up.** Notifications are armed once the launch-time adoption has finished (or failed), so replayed history never notifies; arming updates the badge.
- HOST-207 **Which sessions.** Only a session whose card is on a project's board, and isn't a view's brain or a terminal, notifies.
- HOST-208 **Content.** Title = the card's title, else the transcript's title, else `New session`; subtitle = the project name (macOS); body = `Needs you` for "waiting", else the last assistant reply with whitespace collapsed and trimmed, else `Finished`; group id = the project id.
- HOST-209 **One per session.** Showing a notification closes the previous one for the same session.
- HOST-210 **Unsupported.** Nothing is shown where the system doesn't support notifications.
- HOST-211 **Click.** Clicking a notification opens its project and, if the session is still on that project's board, its card (a finished action run has left the board, so only the board opens).
- HOST-212 **Open a card.** Opening a card of a project that no longer exists only brings the window up; otherwise the window comes up (re-created if closed, restored if minimised), and the request goes to the UI as `ev:open-card`, or, when the window was closed or still loading, is kept for `app:take-open-request`.
- HOST-213 **Take the open request.** `app:take-open-request` returns the kept request once, then `null`.
- HOST-214 **Waiting count.** The waiting count is the number of nodes, on every board, that aren't a view's brain, aren't archived, are live and read `waiting` (action runs included).
- HOST-215 **Dock badge.** Once armed, on macOS only, the Dock badge shows the waiting count while the `dockBadge` setting is on, and nothing when the count is 0 or the setting is off; it is recomputed on arming, on every status emission and on every settings change, and set only when the number changes.

### Running counts (board.ts `projectStats`, runningSessions.ts)

- HOST-216 **Running counts.** Each node on a project's board with a live process counts once: a view's brain or an action run as `background`, a terminal as `terminals`, any other as `sessions`.
- HOST-217 **Working and waiting counts.** A project's `working` and `waiting` count its live, non-archived nodes that aren't views' brains, by combined status.
- HOST-218 **Running total.** The title bar's total is the sum over projects of `sessions + background`; shells never count.
- HOST-219 **By project.** The per-project list holds, in project order, each project whose `sessions + background` is above 0, with that count.

### Wiring (ipc.ts, index.ts)

- HOST-220 **Status side effects.** Each status emission goes, in order: `ev:status` to the UI, the notifier, then—when `done`—finishing an action run (05) and archiving a session that asked to be (04), then the view brain hook (04).
- HOST-221 **Launch order.** Status, agent and usage listeners are set up before the launch adoption, which runs before any window exists; its events reach no window, and the UI reads statuses and usage from board loads.
- HOST-222 **Open.** `session:open` for a live session resizes it and returns `{replay: <snapshot>, live: true}`; otherwise it starts the session (03) and returns `{replay: '', live: true}`.
- HOST-223 **Write path.** `session:write` ignores non-string data and, before writing, runs the action-run takeover check (05) and the view brain input hook (04).
- HOST-224 **Resize and kill.** `session:resize` and `session:kill` call resize and kill.
- HOST-225 **Quitting.** Quitting the app disconnects from the host (HOST-111); Quit and Stop Sessions runs `stopAll` and then quits.

## Data & formats

### 1. Constants

| Name | Value | Where |
| --- | --- | --- |
| `PROTOCOL` | `1` | hostProtocol.ts:15 |
| `STATUS_LINE_EVENT` | `StatusLine` | hostProtocol.ts:21 |
| `MCP_PATH` | `/mcp` | hostProtocol.ts:103 |
| `MCP_SESSION_HEADER` | `X-Nodus-Session` (Node reads it lowercased) | hostProtocol.ts:104 |
| `STATUS_KEY` | `^(?:\r\|\x1b\|\x03\|[1-9])$` | hostProtocol.ts:107 |
| Socket path byte limit | 100 (UTF-8 bytes, `>` moves it) | hostProtocol.ts:130 |
| Name hash | SHA-1 hex, first 12 chars | hostProtocol.ts:122 |
| Host `BUFFER_LIMIT` | 2 097 152 UTF-16 code units | host.ts:53 |
| `HISTORY_LIMIT` | 1000 entries | host.ts:54 |
| `KILL_GRACE_MS` | 3000 | host.ts:55 |
| `UNUSED_MS` | 30 000 | host.ts:57 |
| Shutdown hard exit | `KILL_GRACE_MS + 1000` = 4000 ms | host.ts:188 |
| Slim string limit | 1024 UTF-16 code units | host.ts:112 |
| Hook token | 16 random bytes → 32 lowercase hex | host.ts:77 |
| Windows host key | 32 random bytes → 64 lowercase hex | host.ts:83 |
| `RELAY_TIMEOUT_MS` | 20 000 | host.ts:333 |
| `MCP_MAX_BYTES` | 1 048 576 (compared with the decoded body's UTF-16 length) | host.ts:334 |
| App `BUFFER_LIMIT` | 2 097 152 UTF-16 code units | pty.ts:32 |
| `FLUSH_MS` | 16 | pty.ts:33 |
| `KILL_WAIT_MS` | 4000 | pty.ts:35 |
| `stopAll` wait | `KILL_WAIT_MS + 1000` = 5000 ms | pty.ts:364 |
| `HOST_START_MS` | 10 000, polled every 100 ms | pty.ts:36, 294–295 |
| `WELCOME_MS` | 5000 | pty.ts:38 |
| Spawn defaults | cols 120 (min 20), rows 36 (min 5) | pty.ts:414–415 |
| Resize minimums | cols 20, rows 5, floored | pty.ts:458 |
| `sendPrompt` Return delay | 200 ms | pty.ts:453 |
| Interrupt timer | 1200 ms | hooks.ts:265 |
| Hook `timeout` | 5 (seconds, Claude Code's) | hooks.ts:420 |
| curl max time | `-m 2` | hooks.ts:380 |
| PowerShell timeout | `-TimeoutSec 2` | hooks.ts:395 |
| Status line `padding` | 0 | hooks.ts:423 |
| Agent read chunk | 4 194 304 bytes | agents.ts:32 |
| Reply line length | 200 | agents.ts:90 |

### 2. Files and paths

| File | Path | Mode | Written by | Content |
| --- | --- | --- | --- | --- |
| Host folder | `<userData>/host` | 0700 | app (`ensurePrivateDir`, before each host start and in `writePrivateFile`) | — |
| Socket (macOS) | `<userData>/host/host.sock`, or `$TMPDIR/nodus-<hash>.sock` when the former is over 100 bytes | 0600 | host | Unix stream socket |
| Pipe (Windows) | `\\.\pipe\nodus-<hash>` | default pipe security (owner and admins may write) | host | named pipe, byte stream |
| Log | `<userData>/host/host.log` | default | app opens it truncated at each host start; host's stdout and stderr | log lines (HOST-034) |
| Curl config | `<userData>/host/hook.curlrc` | 0600 | app on each adoption of a host with `hookAuthHeader` | `header = "Authorization: Bearer <token>"\n` |
| Host key | `<userData>/host/host.key` | 0600 (Windows: folder ACL) | host, Windows only | 64 hex chars, no newline |

`<userData>` is `~/Library/Application Support/Nodus` on macOS, `%APPDATA%\Nodus` on Windows, or `NODUS_USER_DATA` (01 owns its resolution). The hash input is the folder string exactly as Node's `path.join(userData, 'host')` produces it (normalised separators, no trailing separator). Synthetic test vectors (SHA-1, first 12 hex chars):

| Host folder string | `<hash>` |
| --- | --- |
| `/Users/example/Library/Application Support/Nodus/host` | `dc3de46669b8` |
| `C:\Users\example\AppData\Roaming\Nodus\host` | `faa591a66f7f` |

`/Users/example/Library/Application Support/Nodus/host/host.sock` is 63 bytes, so it stays in the folder.

### 3. Host process launch

| | 0.9.4 | Rust |
| --- | --- | --- |
| Program | `process.execPath` (the Electron binary) | the `Nodus` binary |
| Args | `<dir of main bundle>/host.js`, `<socket>`, `<key>` | `host`, `<socket>`, `<key>` |
| Env | app's `process.env` + `ELECTRON_RUN_AS_NODE=1` | app's environment |
| cwd | `<userData>/host` | same |
| stdio | stdin ignored; stdout and stderr → `host.log` (opened `w`, truncated) | same |
| Detach | `detached: true` (POSIX `setsid`; Windows `DETACHED_PROCESS` + new process group), `windowsHide: true`, `unref()` | same semantics |

### 4. Framing

- Transport: Unix domain stream socket (macOS) or named pipe (Windows), full duplex, UTF-8.
- One message = `JSON.stringify(msg) + "\n"`. Serialization as `JSON.stringify`: keys in the order given below, no whitespace, fields whose value is `undefined` omitted, strings escaped as `\"`, `\\`, `\b`, `\f`, `\n`, `\r`, `\t`, other U+0000–U+001F as `\u00xx` (lowercase hex), everything else raw UTF-8 (`/`, U+2028, U+2029 not escaped). serde_json produces the same bytes for valid strings.
- Readers: stateful UTF-8 decoding; split on `\n`; skip empty lines; drop lines that fail to parse; no line-length limit; messages may carry unknown extra fields, which are ignored.
- Numbers: integers (`id`, `at`, `exitCode`, `hookPort`, `cols`, `rows`, `protocol`) are JSON integers. Forwarded payloads (`hook.payload`, `mcp.message`, `mcp-reply.reply`, `statusLine.payload`) are opaque JSON; readers on both sides must accept JS number spellings such as `1e+21`.

### 5. Messages, app → host (`ClientMessage`)

| `type` | Fields, in order | Host action |
| --- | --- | --- |
| `hello` | `protocol: number` | HOST-036–038 |
| `spawn` | `id: number` (request id), `sessionId: string`, `projectId: string`, `initial: "working"\|"waiting"\|"done"`, `file: string`, `args: string[]`, `env: {[name]: string}`, `cwd: string`, `cols: number`, `rows: number` | HOST-048–054; reply `spawned` |
| `write` | `sessionId: string`, `data: string` | HOST-059–060 |
| `resize` | `sessionId: string`, `cols: number`, `rows: number` | HOST-061 |
| `kill` | `sessionId: string` | HOST-062–063 |
| `shutdown` | — | HOST-064 (also accepted from a greeted non-client) |
| `mcp-reply` | `id: number` (relay id), `reply?: any` (omitted when the app has no answer) | HOST-086 |
| `exec` | `id: number`, `argv: string[]`, `cwd?: string`, `env?: {[name]: string}`, `stdin?: string`, `timeoutMs?: number`, `claude?: true` (`argv[0]` is Claude Code's program, found on `env`'s PATH or the installers' folders under the host user's home), `stdout?`/`stderr?: "all"\|"ignore"\|{"first": n}\|{"maxBuffer": n}` (each kept to 32 MiB whatever is asked), `killSignal?: number`, `newSession?: true` | Rust only (SRV-005): runs a process outside any terminal, its tree stopped at the timeout; reply `exec-result` |
| `exec-cancel` | `id: number` | Rust only: SIGKILLs a running `exec`'s tree; a closing connection does the same to all of its own |
| `trust` | `id: number`, `path: string` | Rust only (SRV-126): sets `projects[path].hasTrustDialogAccepted` in the host user's `.claude.json` (`$CLAUDE_CONFIG_DIR/.claude.json` when set), keeping the rest; reply `trusted` |

`exec`, `exec-cancel` and `trust` came without a protocol change: a host that doesn't know them ignores them (HOST-040), and only `nodus serve`, whose host is its own binary's, sends them. Like every other message but `shutdown`, they're only acted on from an app client (HOST-039).

Examples (placeholders in `<…>`):

```json
{"type":"hello","protocol":1}
{"type":"spawn","id":1,"sessionId":"<uuid>","projectId":"<uuid>","initial":"done","file":"<path to claude>","args":["--session-id","<uuid>","--permission-mode","acceptEdits","--settings","<hook settings JSON>"],"env":{"<NAME>":"<value>"},"cwd":"<repo path>","cols":120,"rows":36}
{"type":"write","sessionId":"<uuid>","data":"\u001b"}
{"type":"resize","sessionId":"<uuid>","cols":132,"rows":40}
{"type":"kill","sessionId":"<uuid>"}
{"type":"shutdown"}
{"type":"mcp-reply","id":3,"reply":{"jsonrpc":"2.0","id":7,"result":{}}}
{"type":"mcp-reply","id":4}
```

### 6. Messages, host → app (`HostMessage`)

| `type` | Fields, in order | Sent |
| --- | --- | --- |
| `welcome` | `protocol: 1`, `hookPort: number`, `hookToken: string` (32 hex), `sessions: HostSession[]`, `mcp: true`, `hookAuthHeader: true`, `key?: string` (Windows only, 64 hex) | to the greeting connection, once |
| `spawned` | `id: number` (the request id), `error?: string` | to the requester |
| `data` | `sessionId: string`, `data: string` | to every app client |
| `exit` | `sessionId: string`, `exitCode: number` | to every app client |
| `hook` | `sessionId: string`, `event: string`, `payload: any` (full JSON body; `{}` if empty or unparseable) | to every app client |
| `mcp` | `id: number` (relay id), `sessionId: string`, `message: object` (one JSON-RPC request) | to the newest app client |
| `exec-result` | `id: number`, `code?: number`, `signal?: number`, `stdout: string`, `stderr: string` (UTF-8, bad bytes replaced), `timedOut?: true`, `overflow?: "stdout"\|"stderr"`, `error?: string` (it never started, as `spawn git ENOENT`), `errno?: number` | to the requester (Rust only) |
| `trusted` | `id: number`, `error?: string` | to the requester (Rust only) |

Older hosts may omit `mcp` (no board tools: the app adds no `--mcp-config`), `hookAuthHeader` (the app writes no curl config and hooks send the token in the URL), `key`, and `statusLine`; readers treat each as absent/false.

```json
{"type":"welcome","protocol":1,"hookPort":<port>,"hookToken":"<32 hex>","sessions":[{"sessionId":"<uuid>","projectId":"<uuid>","initial":"working","buffer":"<recent output>","history":[{"hook":"UserPromptSubmit","payload":{"session_id":"<uuid>","hook_event_name":"UserPromptSubmit"},"at":1762600000000},{"input":"\u001b","at":1762600001000}],"statusLine":{"payload":{"model":{"display_name":"<model>"}},"at":1762600002000}}],"mcp":true,"hookAuthHeader":true,"key":"<64 hex, Windows only>"}
{"type":"spawned","id":1}
{"type":"spawned","id":2,"error":"posix_spawnp failed."}
{"type":"data","sessionId":"<uuid>","data":"<output>"}
{"type":"exit","sessionId":"<uuid>","exitCode":0}
{"type":"hook","sessionId":"<uuid>","event":"PreToolUse","payload":{"tool_name":"Bash","tool_input":{"command":"<cmd>"}}}
{"type":"mcp","id":1,"sessionId":"<uuid>","message":{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"<tool>","arguments":{}}}}
```

### 7. Shared structures

- `HostSession` (fields in order): `sessionId: string`, `projectId: string`, `initial: SessionStatus` (status when the process started), `buffer: string` (the host's chunks joined), `history: HistoryEntry[]` (oldest first, ≤ 1000), `statusLine?: {payload: object, at: number}`.
- `HistoryEntry`: either `{"hook": string, "payload": object (slimmed), "at": number}` or `{"input": string, "at": number}` (keys in that order). Distinguished by the presence of the `hook` key.
- `at`: milliseconds since the Unix epoch (`Date.now()`, integer).

### 8. Hook HTTP endpoint (`127.0.0.1:<hookPort>`)

```
POST /hook?evt=<event>&sid=<sessionId>            Authorization: Bearer <token>
POST /hook?evt=<event>&sid=<sessionId>&token=<token>      (older apps' processes)
Content-Type: application/json   (sent, not checked)
Body: the hook's stdin JSON from Claude Code, or empty
```

| Case | Status | Body |
| --- | --- | --- |
| path `/mcp` | see §9 | |
| not `POST`, or path not `/hook`, or token missing or wrong | `404` | empty (request drained) |
| accepted (even with unknown `sid`, missing `evt`, bad JSON) | `204` | empty, sent before processing |

Query parsing follows WHATWG `URLSearchParams` (percent-decoding, `+` as space, first value of a repeated key). The token check is constant-time on bytes. Header lookup is case-insensitive; the Bearer regex is case-sensitive: `^Bearer (\S+)$`.

### 9. MCP relay endpoint (`127.0.0.1:<hookPort>/mcp`)

```
POST /mcp
Authorization: Bearer <token>
X-Nodus-Session: <sessionId>
Body: one JSON-RPC 2.0 message, or a batch array
```

| Case | Status | Headers | Body |
| --- | --- | --- | --- |
| token missing/wrong, or session header missing / not a live session | `401` | — | empty |
| authorized, `DELETE` | `204` | — | empty |
| authorized, other non-POST (`GET`…) | `405` | `Allow: POST` | empty |
| body over 1 048 576 UTF-16 units | `413` | — | empty |
| body not JSON | `200` | `Content-Type: application/json` | `{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}` |
| no requests (notifications only) | `202` | — | empty |
| requests | `200` | `Content-Type: application/json` | answer object, or array of answers in request order for a batch |
| handler exception | `500` (if unsent) | — | empty |

No server-sent streams; every request is answered on its own POST. Unanswered replies (HOST-088) use `APP_CLOSED` = `Nodus isn’t open, so the board can’t be reached right now.` (curly apostrophes U+2019).

### 10. Hook commands and the `--settings` JSON

Command templates (`<event>` is the hook name, or `StatusLine`):

POSIX, token from the curl config (host with `hookAuthHeader`):

```
curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K "$NODUS_HOOK_CONFIG" "http://127.0.0.1:$NODUS_PORT/hook?evt=<event>&sid=$NODUS_SESSION" >/dev/null 2>&1; exit 0
```

POSIX, token in the URL (older host):

```
curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- "http://127.0.0.1:$NODUS_PORT/hook?evt=<event>&sid=$NODUS_SESSION&token=$NODUS_TOKEN" >/dev/null 2>&1; exit 0
```

PowerShell (Windows without Git Bash), with `"shell":"powershell"`:

```
$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = "Bearer $env:NODUS_TOKEN" } -Body $body.ToArray() -Uri "http://127.0.0.1:$env:NODUS_PORT/hook?evt=<event>&sid=$env:NODUS_SESSION" } catch {}; exit 0
```

Event order in `hooks`: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `SessionEnd`, `SubagentStart`, `SubagentStop`. Hook object key order: `type`, `command`, [`shell`], `timeout`. Status line key order: `type`, `command`, [`shell`], `padding`. The three complete strings, verbatim (each is one line; a golden test should compare bytes):

Variant A — POSIX with curl config (`hookSettings()` when a config path is set and PowerShell isn't used):

```json
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=SessionStart&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"UserPromptSubmit":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=UserPromptSubmit&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=PreToolUse&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PostToolUse":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=PostToolUse&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PermissionRequest":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=PermissionRequest&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"Notification":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=Notification&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"Stop":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=Stop&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=SessionEnd&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SubagentStart":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=SubagentStart&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SubagentStop":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=SubagentStop&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","timeout":5}]}]},"statusLine":{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- -K \"$NODUS_HOOK_CONFIG\" \"http://127.0.0.1:$NODUS_PORT/hook?evt=StatusLine&sid=$NODUS_SESSION\" >/dev/null 2>&1; exit 0","padding":0}}
```

Variant B — POSIX, token in the URL (no config path):

```json
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=SessionStart&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"UserPromptSubmit":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=UserPromptSubmit&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=PreToolUse&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PostToolUse":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=PostToolUse&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"PermissionRequest":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=PermissionRequest&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"Notification":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=Notification&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"Stop":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=Stop&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=SessionEnd&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SubagentStart":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=SubagentStart&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}],"SubagentStop":[{"hooks":[{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=SubagentStop&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","timeout":5}]}]},"statusLine":{"type":"command","command":"curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- \"http://127.0.0.1:$NODUS_PORT/hook?evt=StatusLine&sid=$NODUS_SESSION&token=$NODUS_TOKEN\" >/dev/null 2>&1; exit 0","padding":0}}
```

Variant C — PowerShell (Windows without Git Bash; identical with or without a config path):

```json
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=SessionStart&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"UserPromptSubmit":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=UserPromptSubmit&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=PreToolUse&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"PostToolUse":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=PostToolUse&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"PermissionRequest":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=PermissionRequest&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"Notification":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=Notification&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"Stop":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=Stop&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=SessionEnd&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"SubagentStart":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=SubagentStart&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}],"SubagentStop":[{"hooks":[{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=SubagentStop&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","timeout":5}]}]},"statusLine":{"type":"command","command":"$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($body); try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' -Headers @{ Authorization = \"Bearer $env:NODUS_TOKEN\" } -Body $body.ToArray() -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt=StatusLine&sid=$env:NODUS_SESSION\" } catch {}; exit 0","shell":"powershell","padding":0}}
```

### 11. Environment variables

| Variable | Set by | Value | Read by |
| --- | --- | --- | --- |
| `NODUS_PORT` | app, in every Claude process | hook port, decimal | hook commands, MCP config |
| `NODUS_TOKEN` | app, in every Claude process | hook token | URL/PowerShell hooks, MCP config's `Authorization` |
| `NODUS_SESSION` | app, in every Claude process | session id | hook commands, MCP config's `X-Nodus-Session` |
| `NODUS_HOOK_CONFIG` | app, when the host takes the header | absolute path of `hook.curlrc` | curl `-K` |
| `CLAUDE_CODE_GIT_BASH_PATH` | app, Windows, when Git Bash was found and the user's env lacks it | Git Bash path | Claude Code |
| `ELECTRON_RUN_AS_NODE` | 0.9.4 app, for the host process only | `1` | Electron |
| `PWD`, `TERM` | host (node-pty), POSIX | `<cwd>`, `xterm-256color` | the session's process |

### 12. Board tools MCP config (owned by 04; reproduced because it targets this domain's endpoint)

```json
{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}
```

### 13. Windows command line

`windowsCommandLine(args)`: each argument is left as is when it is non-empty and contains no whitespace and no `"`; otherwise it becomes `"` + the argument with every run of backslashes before a `"` doubled and that `"` escaped as `\"`, and a trailing run of backslashes doubled, + `"`. Arguments are joined with single spaces. "Whitespace" is JavaScript's `\s`: U+0009–U+000D, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF.

Vectors (raw, not JSON-escaped):

| Argument | Quoted |
| --- | --- |
| `--resume`, `abc`, `two words`, *(empty)* | `--resume abc "two words" ""` |
| `"fix the tests"` | `"\"fix the tests\""` |
| `C:\Program Files\Nodus\` | `"C:\Program Files\Nodus\\"` |
| `ends in two \\` | `"ends in two \\\\"` |
| `a\"b` | `"a\\\"b"` |
| `x`U+00A0`y` | `"x`U+00A0`y"` |

The program goes in front as node-pty 1.1.0 `argsToCommandLine(file, [])` quotes it: wrapped in `"` only when it is empty, or contains a space or tab, is longer than one character, and isn't already wrapped in a matching pair of `"`; inside it a `"` becomes (2n+1 backslashes)+`"` for n preceding backslashes; when wrapped, trailing backslashes are doubled. Final command line: `<quoted file> <windowsCommandLine(args)>`, or `<quoted file>` when args is empty.

### 14. IPC payload types (`src/shared/types.ts`)

- `SessionStatus` = `"working" | "waiting" | "done"`.
- `SessionUsage` = `{model?: string, contextPercent?: number, contextTokens?: number, contextSize?: number, costUsd?: number, durationMs?: number, at: number}`.
- `AgentInfo` = `{agentId, agentType, description, status, parentAgentId?, stopped?, tool?, activity?, background?, toolCount, startedAt, updatedAt}`.
- `RunningCounts` = `{sessions, background, terminals}`; `ProjectStats` = `{total, active, working, waiting, running: RunningCounts}` (`total`/`active` are 03's).
- `OpenSessionResult` = `{replay: string, live: boolean}`.
- `OpenCardRequest` = `{projectId: string, sessionId?: string}`.

### 15. Agent inputs (Claude Code's files, read only)

- `<transcript dir>/<session id>/subagents/agent-<id>.jsonl`: JSON lines; fields read: `timestamp` (ISO string), `type` (`user`/`assistant`), `isMeta`, `toolUseResult.backgroundTaskId`, `origin.kind` (`task-notification`), `message.content` (string or blocks: `text`, `tool_use` {`name`, `input`}, `tool_result`), `message.stop_reason`.
- `agent-<id>.meta.json`: `{agentType?, description?, name?, parentAgentId?, finished?, stoppedByUser?}`; rewritten by Claude Code when the agent ends.
- `<transcript dir>` = `~/.claude/projects/<repo path with every non-alphanumeric char replaced by ->` (03).

## API surface

### Commands implemented (window.api → nodus-core)

| Channel | Signature | Notes |
| --- | --- | --- |
| `session:open` | `(projectId, sessionId, cols, rows) → OpenSessionResult` | live branch here (HOST-222); start branch shared with 03 |
| `session:write` | `(sessionId, data)` fire-and-forget | HOST-223, HOST-121 |
| `session:resize` | `(sessionId, cols, rows)` fire-and-forget | HOST-123 |
| `session:kill` | `(sessionId) → void` | HOST-124 |
| `session:agents` | `(projectId, sessionId) → AgentInfo[]` | HOST-198 |
| `session:agent-transcript` | `(projectId, sessionId, agentId) → TranscriptMessage[]` | HOST-200; parsing is 03's |
| `projects:stats` | `() → Record<projectId, ProjectStats>` | running/working/waiting here (HOST-216–217) |
| `app:take-open-request` | `() → OpenCardRequest \| null` | HOST-213 |

### Events emitted (nodus-core → UI)

| Channel | Args | Source |
| --- | --- | --- |
| `ev:pty-data` | `(sessionId, data)` | HOST-118–120 |
| `ev:pty-exit` | `(sessionId, exitCode)` | HOST-120; `-1` when the host was lost |
| `ev:status` | `(sessionId, status, live)` | HOST-134 |
| `ev:agent-status` | `(sessionId, agentId, status)` | HOST-154 |
| `ev:usage` | `(sessionId, usage)` | HOST-171–172 |
| `ev:open-card` | `(OpenCardRequest)` | HOST-212 |

Ordering guarantees the UI relies on: every `ev:pty-data` emitted before `session:open` returns is contained in its replay (the terminal registry drops data while opening, 12); a process start emits `ev:status` before its first `ev:pty-data`; an exit emits pending `ev:pty-data`, then `ev:status`, then `ev:pty-exit`. The UI refreshes `projects:stats` 250 ms after the last `ev:status` or `ev:pty-exit` (App.tsx, 08).

### Internal API used by other domains

`connectHost(start)`, `disconnect()`, `stopAll()` (menu 01, updater 06), `spawnSession(SpawnOptions)` (03, 04, 05, 07), `snapshot`, `write` (04 views type into brains), `sendPrompt` (03, 07), `resize`, `kill` (03 board, 04, 05), `killProject` (01 projects:remove), `isLive`, `liveSessionIds` (06); `getStatus` (03, 04, 05, 07), `getUsage` (03 `toCard`), `forgetStatus` (03, 04, 05), `hookEnv`, `hookSettings`; `readAgents`, `attachAgents` (03 `loadBoard`), `hasPendingWork`; `configureNotify`, `armNotifications`, `openCard` (07 capture), `takeOpenRequest`, `statusChanged`, `updateBadge` (01 settings), `noticeFor`, `waitingCount`; `runningTotal`, `runningByProject` (08 `RunningCount`).

`SpawnOptions` = `{sessionId, projectId, cwd, kind?: 'claude'|'terminal', resume: boolean, name?, initialPrompt?, permissionMode?: 'acceptEdits'|'auto', extraArgs?: string[], cols?, rows?}`.

## Keyboard & mouse

| Input | Bytes written | Effect on status |
| --- | --- | --- |
| Return | `\r` | main `waiting` → `working`; answers the longest-waiting agent when the main thread isn't waiting |
| 1 … 9 | `1` … `9` | same as Return (menu choices in Claude's prompts) |
| Esc | `\x1b` | interrupt: `done` after 1200 ms unless a main-thread hook follows; answers a waiting agent when the main thread isn't waiting |
| Ctrl+C | `\x03` | same as Esc |
| 0, letters, pastes, ⇧Return, escape sequences | anything else | no status effect |
| Prompt bar / capture send into a session | `\x1b[200~<text>\x1b[201~`, then `\r` 200 ms later | the `\r` counts as Return |

Only whole writes count (one keystroke per write, as xterm sends them). The host records exactly these four kinds of write (`STATUS_KEY`) so a relaunched app can replay them. Mouse: clicking a notification opens its card (HOST-211); pointing at the title bar's running count shows the per-project list (UI in 08, data HOST-219).

## Platform differences

| Area | macOS | Windows |
| --- | --- | --- |
| Transport | Unix socket in `<userData>/host` (0600), `$TMPDIR` fallback over 100 bytes | named pipe `\\.\pipe\nodus-<hash>`, default security |
| Host identity | folder permissions only; no key | host writes `host.key`; app trusts only a welcome carrying it (HOST-012) |
| Command line | argv array | single command line, `windowsCommandLine` + node-pty file quoting |
| Spawn failures | missing cwd/file → session exits 1 (HOST-053); `posix_spawnp failed.` if the helper can't start | missing file → `File not found: <path>` |
| PTY env | `PWD` and `TERM` forced | env passed as is |
| Kill | SIGTERM (+SIGHUP on shutdown) to the pid, SIGKILL after 3000 ms | terminate the console's processes and close the pseudo-console; no timer |
| Exit code | 0 when signalled | process exit code |
| Hooks | curl in the POSIX shell | curl in Git Bash when found (and `CLAUDE_CODE_GIT_BASH_PATH` set for Claude), else PowerShell (slower to start, so status moves a moment later) |
| Dock badge | yes (`dockBadge` setting) | none |
| Notifications | title, subtitle (project), body, thread id = project id | title, body, toast group = project id; needs AppUserModelID `dev.saleem.nodus` (01) |
| Updating | host keeps running across an update and is adopted | the installer can't replace files the host uses: update stops sessions first (06) |

## Edge cases & errors

Verbatim strings:

- `usage: host <socket path> <key path>` — host argument error (logged with `failed to start`; exit 1).
- `The session host’s token can’t go in a curl config` — `writeHookConfig` with an unsafe token.
- `Another program answered as the session host, so Nodus didn’t trust it with your sessions.` — untrusted pipe owner (Windows).
- `The session host didn’t answer` — no first message within 5000 ms, or closed before it.
- `The session host stopped` — pending spawns when the host connection drops.
- `The session host didn't start. See <host.log path>` — note the straight apostrophe, unlike the others.
- `The session host speaks another protocol` — a freshly started host answered with another protocol.
- Dialog: `Sessions from another version of Nodus are still running` / `This version can’t connect to them. Stop them to carry on, or quit and let them finish.` / `Stop Sessions` / `Quit`.
- `Nodus isn’t open, so the board can’t be reached right now.` — MCP unanswered.
- `Parse error` (-32700), -32603 for unanswered non-tool requests.
- `Not an agent of this session` — bad agent or session id.
- Notification bodies `Needs you`, `Finished`; title fallback `New session`.
- Console logs (app): `[host] connect failed`, `[host] could not write the hook config`, `[board-tools]`.
- Host log lines: `failed to start`, `another host is running`, `listening … hooks on …`, `app connected `, `app connected (protocol <n>)`, `app disconnected`, `message failed`, `spawned <id> <file>`, `exited <id> <code>`, `stopping every session`, `idle, exiting`, `mcp request failed`, `uncaught`, `unhandled`.
- Pass-through spawn errors from the PTY layer (`posix_spawnp failed.`, `File not found: <path>`), and 01's `Claude Code CLI not found. Install it (…) and restart Nodus.` from `requireClaude`.

Edge behaviors:

- Duplicate `spawn` for a running session replies success without a process (HOST-048); the app re-tracks it with an empty buffer.
- A greeting connection on another protocol still receives the full welcome, sessions and token included (the smoke test relies on this with `protocol: 0`, scripts/smoke.mjs:158).
- `connectHost(true)` called while the launch's `connectHost(false)` is in flight returns that attempt; if it found no host, the following `spawn` is dropped and the start never settles (pty.ts:340). Launch awaits the attempt before any handler exists, so this doesn't happen in practice; keep the shared-attempt semantics.
- `kill` waiters that time out stay registered until the session exits (harmless).
- A hook body of `null` for a non-`StatusLine` event is logged as `uncaught` and lost (HOST-078).
- unclear: the 0.9.4 MCP handler decodes each TCP chunk separately (`body += chunk`, host.ts:390–391), so a multi-byte character split across chunks becomes U+FFFD; nothing relies on it — decide at review whether to reproduce or decode the whole body.
- `null` from the app as an MCP reply is treated as no reply (`??`, host.ts:403).
- A spawn whose `cols`/`rows` aren't numbers reaches node-pty as-is in 0.9.4 (the app never sends that); a Rust host that can't deserialize a `spawn` with a numeric `id` should still reply `spawned` with an `error` rather than ignore it, or the app waits forever. unclear: exact error text (host.ts:131–133 gives node-pty's message).
- Lone UTF-16 surrogates: `JSON.stringify` writes them as `\udxxx` escapes and `JSON.parse` accepts them; serde_json rejects them, which would drop the whole line. A Rust reader must accept them (map to U+FFFD) to stay wire-compatible.
- Windows: a CreateProcess failure inside node-pty happens after `spawned` was sent and surfaces as `uncaught` in the host log (node-pty conpty.cc:426). unclear: the session's state afterwards in 0.9.4.
- A host started by a version without board tools keeps running after an update; sessions started on it get no board tools until a new host starts (docs "Known limitations").
- `SessionEnd` (non-clear) emits live = false while the process exits; the exit itself then changes nothing, so the UI refreshes stats on `ev:pty-exit` too.
- Agent replay after relaunch: `SubagentStop` asks `hasPendingWork` against the transcript as it is now, not as it was.
- macOS bundle swap: a 0.9.4 host resolved node-pty's `spawn-helper` to `<Nodus.app>/Contents/Resources/app.asar.unpacked/node_modules/node-pty/build/Release/spawn-helper` when it started and runs it for every spawn. After the Rust app replaces the bundle, spawns on that adopted host fail with `posix_spawnp failed.` unless that file still exists. unclear: decision needed with 06 (ship a compatible helper at that path, or stop/replace the Node host before the first spawn).

## Tests to port

### hostProtocol.test.ts → nodus-host-proto

messageReader:
1. `reads several messages from one chunk` — `kill{sessionId:a}` + `shutdown` in one buffer → both, in order.
2. `waits for the rest of a message split across chunks` — first 10 bytes deliver nothing; the rest delivers `write{a,'hello world'}`.
3. `keeps a character whose bytes are split across chunks` — `write` with data `héllo 🎉` split at every byte offset 1…len−1 → decodes intact each time.
4. `drops malformed and blank lines and carries on` — `not json\n\n` + `shutdown` → only `shutdown`.
5. `keeps newlines inside a message` — data `line one\nline two\r\n` round-trips.

host files:
6. `keeps the socket in the host folder when the path is short enough` — darwin paths object equals `{dir, socket: dir/host.sock, log, hookConfig, key}` under `<tmp>/host`.
7. `moves a socket with a long path to the temp folder, under a name from the folder` — userData = `<tmp>/` + 120×`x`; socket = `<os tmp>/nodus-<sha1(hostDir)[:12]>.sock`; hookConfig stays in the folder.
8. `names a pipe on Windows, from the folder too` — `\\.\pipe\nodus-<hash>`; hookConfig in the folder.
9. `makes the host folder private, even one that exists already` (skipped on Windows) — 0755 folder becomes 0700.
10. `writes the hook config readable only by the user` — write `first` then `second`; content `header = "Authorization: Bearer second"\n`; file 0600 and folder 0700 (non-Windows); folder contains only `hook.curlrc`.
11. `won’t put a token in the hook config that would add curl options` — `abc123` ok; `x"\noutput = "C:\evil.cmd` and `''` throw; content still `abc123`.

trustsHost:
12. `trusts a pipe whose welcome carries the key in the host folder` — key file 64×`k`, welcome key 64×`k` → true.
13. `doesn’t trust one that made the pipe first and can’t read the folder` — 64×`j`, `undefined`, `''` → false.
14. `doesn’t trust a pipe while there’s no key file` → false.
15. `needs no key on a Mac, where the socket’s folder keeps others out` — darwin, undefined key → true.

windowsCommandLine:
16. `quotes only what needs it` — `['--resume','abc','two words','']` → `--resume abc "two words" ""`.
17. `comes back as the same arguments, whatever they hold` — `--settings`, a JSON settings string, `--`, `"fix the tests"`, `C:\Program Files\Nodus\`, `ends in two \\`, `a\"b`, `tab<TAB>here`, `line one<LF>line two`, `''`, `plain` → split back by an MSVC-rules splitter (port the test's `splitCommandLine` helper too) equals the input.

### hooks.test.ts → nodus-sessions (status machine with an injectable clock and timers)

Setup: each test uses a new session id that is live and `markProcess(id, true)`; listeners record `(id, status, live)`, `(id, agentId, status)`, `(id, usage)`; `hasPendingWork` mock defaults to false; on non-Windows `gitBash` is null, on Windows `bash.exe`.

session status:
1. `is working through a turn and done when it stops` — UserPromptSubmit → working; PreToolUse(Bash), PostToolUse → working; Stop → done; emitted statuses exactly `done, working, done`.
2. `waits for the user on a permission prompt or a question` — PermissionRequest → waiting; PostToolUse then PreToolUse(AskUserQuestion) → waiting; Notification(permission_prompt) → waiting.
3. `ignores notifications that are not about permission` — Notification(idle_prompt, `Claude is waiting`) → done.
4. `goes back to working when the user answers a prompt` — PermissionRequest, input `1` → working.
5. `ends the turn on an interrupt that no hook follows` — UserPromptSubmit, `\x1b` → still working; +1200 ms → done.
6. `keeps working when a hook follows the interrupt` — UserPromptSubmit, `\x03`, +100 ms PostToolUse, +1200 ms → working.
7. `reads as done once the process is gone` — UserPromptSubmit; not live; markProcess(false) → done; last emission `(id, done, false)`.
8. `reports the process ending on SessionEnd, but not on /clear` — SessionEnd(clear) → last `(id, done, true)`; SessionEnd(logout) → last `(id, done, false)`.
9. `ignores events it does not know` — `SomethingNew` → done.

agents:
10. `keep the card working after the main thread stops` — UserPromptSubmit, SubagentStart(a1), Stop → working; SubagentStop(a1) → done; agent events exactly `(a1, working), (a1, done)`.
11. `stay working after stopping with background work that will wake them` — pending work true; SubagentStart(a1), SubagentStop(a1, `/tmp/agent.jsonl`) → `hasPendingWork` called with that path; status working.
12. `make the card wait while one waits, and an answer goes to that agent` — SubagentStart(a1), PermissionRequest(a1) → waiting; input `\r` → working; last agent event `(a1, working)`.

status line:
13. `turns a report into usage` — model `Opus 5.5`, current_usage 1000/500/1500, size 200000, cost 0.1234, duration 5000, at 1000 → `{model:'Opus 5.5', contextTokens:3000, contextSize:200000, contextPercent:1.5, costUsd:0.1234, durationMs:5000, at:1000}`, one event.
14. `prefers the percentage Claude gives` — `context_window: {used_percentage: 42, context_window_size: 200000}` → percent 42, tokens absent.
15. `does not resend a report that only moved the clock` — same report at 1000 and with duration 9000 at 2000 → one event.

hook commands:
16. `registers every hook and the status line, printing nothing` — endpoint (4000, `secret-token`, `/tmp/hook.curlrc`); hooks include SessionStart, UserPromptSubmit, PreToolUse, Stop, SubagentStop; every command ends with `>/dev/null 2>&1; exit 0`.
17. `read the token from the config file when the host takes it in a header` — settings don't contain the token; every command contains `-K "$NODUS_HOOK_CONFIG"` and no `token=`; `hookEnv('s1')` = `{NODUS_PORT:'4000', NODUS_TOKEN:'secret-token', NODUS_SESSION:'s1', NODUS_HOOK_CONFIG:'/tmp/hook.curlrc'}`.
18. `put the token in the URL for an older host` — no config: settings don't contain the token; every command contains `token=$NODUS_TOKEN`; `hookEnv` has no `NODUS_HOOK_CONFIG`.

hook commands, run by a shell (real `sh` and `curl` against a local HTTP server):
19. `send the token in a header from the config file` — writeHookConfig(`abc123`), run the Stop command with stdin `{"stop":true}` → server sees url `/hook?evt=Stop&sid=s1`, `Authorization: Bearer abc123`, body `{"stop":true}`.
20. `send the token in the URL without a config file` → url `/hook?evt=Stop&sid=s1&token=abc123`, no `Authorization`.

hook commands on Windows without Git Bash (platform forced to win32, gitBash null):
21. `run in PowerShell, with the token from the environment` — 11 entries (10 hooks + status line), each `shell: powershell`, command contains `"Bearer $env:NODUS_TOKEN"` and ends with `exit 0`; settings never contain the token.
22. `post what Claude sends on stdin, byte for byte` (skipped without PowerShell; 30 s timeout) — run the Stop command with `-NoProfile -NonInteractive -Command`, stdin `{"prompt":"héllo 🎉","last_assistant_message":"done"}` → url `/hook?evt=Stop&sid=s1`, `Bearer abc123`, body identical bytes; exit code 0; stdout empty.

### notify.test.ts → nodus-sessions (notify logic) with store/live/status fakes

noticeFor:
1. `tells when a session starts needing the user` — (working|done|undefined → waiting, live) → `waiting`.
2. `tells once while it keeps waiting` — (waiting → waiting) → none.
3. `tells when a turn ends` — (working → done, live) → `finished`.
4. `stays quiet when a process goes, or nothing was running` — (working→done, not live), (waiting→done), (undefined→done), (undefined→working), (waiting→working) → none.

waitingCount:
5. `counts the waiting sessions on every board, action runs too` — board a: w1 waiting, w2 working, r1 waiting with `runOf`; board b: w3 waiting → 3.
6. `leaves out views’ brains, archived cards and sessions that aren’t running` — `viewOf` waiting, archived waiting, not-live → 0.

### runningSessions.test.ts → wherever the title bar count lives (nodus-core or nodus-ui logic module)

runningTotal:
1. `counts Claude sessions in every project, hidden ones too, but not shells` — a {sessions 2, terminals 3}, b {background 1} → 3.
2. `is zero with nothing running` — `{}` → 0; {terminals 1} → 0.

runningByProject (three projects a, b, c; names arbitrary):
3. `gives each project with Claude sessions running and its count, in project order` — stats given b, a, c order with a {sessions 2, terminals 1}, b {sessions 1}, c {} → `[a (2), b (1)]`.
4. `counts a project’s hidden sessions with its cards` — a {sessions 1, background 2} → `[a (3)]`.
5. `leaves out a project that’s only running shells` — a {terminals 2}, b {sessions 1} → `[b (1)]`.

### Additional parity tests (new, required by P2)

- Wire round-trip of every message in §5–§6 against golden bytes (key order, omitted optional fields).
- Rust app ⇄ 0.9.4 Node host and 0.9.4 Node app (or `scripts/smoke.mjs`'s probe) ⇄ Rust host: hello/welcome, spawn, data, write, resize, kill, hook POST, `/mcp` relay, shutdown; protocol-0 probe gets a welcome and doesn't keep the host alive.
- Golden bytes for the three settings JSON variants (§10).
- Agents: a fixture transcript + meta (synthetic) covering background tasks, task notifications, handback, end_turn, heading-stripped reply lines.

## Rust mapping

| Crate | Takes | Notes |
| --- | --- | --- |
| `nodus-host-proto` | hostProtocol.ts | `PROTOCOL`, `STATUS_LINE_EVENT`, `MCP_PATH`, `MCP_SESSION_HEADER`, `is_status_key`; `ClientMessage`/`HostMessage` as `#[serde(tag = "type")]` enums with renames (`mcp-reply`), struct field order = wire order, `Option` + `skip_serializing_if`; payloads as `serde_json::Value` (feature `preserve_order`) or `RawValue` to forward untouched; `HistoryEntry` untagged (hook first); `HostPaths`/`host_paths(user_data, Platform)`; `windows_command_line`; `ensure_private_dir`, `write_private_file`, `write_hook_config`, `trusts_host`; `encode`; `MessageReader` (incremental UTF-8 with replacement, lenient on lone surrogates). |
| `nodus-host` | host.ts | `run(socket, key)` for `Nodus host <socket> <key>`; tokio; `UnixListener` / `tokio::net::windows::named_pipe::ServerOptions` (first instance, default security); per-client unbounded writer queue; session table; PTY layer; hook + MCP server on `127.0.0.1:0` (hyper 1 / axum); relay map with 20 s timeouts; idle/unused/shutdown timers; stdout logging with the exact line format. Must never initialise GPUI/AppKit (no Dock icon). |
| `nodus-sessions` | pty.ts, hooks.ts, agents.ts, notify.ts logic | host client (connect/greet/adopt/replace/stopAll, buffers, 16 ms coalescing); status machine and usage with an injectable clock/scheduler so the 1200 ms and 200 ms timers are testable; `hook_env`, `hook_settings`; agents parser and status; notice/badge decisions; `claude` command building (shared with 03). |
| `nodus-platform` | Electron APIs | detached spawn of the host (POSIX `setsid`, Windows `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` with `SW_HIDE`, no `CREATE_BREAKAWAY_FROM_JOB`, as libuv does); native message box for the replace dialog; notifications (macOS `UNUserNotificationCenter`: title, subtitle, body, `threadIdentifier`, click callback, remove; Windows toast with AUMID `dev.saleem.nodus`, `Group`); Dock badge (`NSDockTile.badgeLabel`); window focus queries. |
| `nodus-core` | ipc.ts wiring | commands/events in API surface; status side-effect order (HOST-220); launch order (HOST-221). |

Suggested crates: `tokio`, `serde`, `serde_json` (`preserve_order`, `raw_value`), `hyper` + `hyper-util` or `axum`, `sha1` + `hex`, `getrandom`, `subtle` (constant-time compare), `time` (ISO timestamps), `libc`/`rustix` (kill, chmod, openpty, posix_spawn), `windows-sys` (ConPTY, `GetConsoleProcessList`, named pipes), `portable-pty` only if it can be made to match HOST-049–063, `objc2-user-notifications`, `objc2-app-kit`, `windows` (toasts), `rfd` or native for the dialog.

Risks:

1. **PTY parity.** node-pty semantics are part of the contract: macOS spawn through a helper (late cwd/exec failure → exit 1, HOST-053), exit code 0 on signal, signals to the pid only, all output delivered before `exit`, stateful UTF-8 decoding, `PWD`/`TERM` injection; on Windows a raw command line (portable-pty builds its own quoting) and kill = terminate the console process list. Expect a custom PTY layer (openpty + posix_spawn; direct ConPTY + `CreateProcessW`).
2. **String units and JSON leniency.** Limits are in UTF-16 code units (2 097 152 buffer, 1024 slim, 1 048 576 MCP), not bytes; lone-surrogate escapes must not drop lines; payload numbers must forward unchanged.
3. **Adopting across the cutover.** A 0.9.4 host adopted by the Rust app on macOS execs a `spawn-helper` from the replaced bundle (Edge cases). Needs a decision with 06 before P11.
4. **Host as a subcommand.** `Nodus host` must start without GPUI/AppKit, detach (setsid / Windows `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, hidden, like libuv), write only to `host.log`, and survive the app; the app must not hold its handle.
5. **Ordering and timers.** Welcome adoption before later messages in the same read; snapshot flush vs `session:open` result; exit ordering (pending data, status, exit); interrupt timers compared with recorded times on replay. Use one actor per connection and a fake clock in tests.
6. **Notifications.** Electron's `subtitle`/`groupId`/click/`close` mapping to UNUserNotificationCenter (needs the signed bundle and authorization) and Windows toasts (AUMID from the installer shortcut).

## Parity checklist

- [x] HOST-001 Host folder
- [x] HOST-002 Name hash
- [x] HOST-003 Socket in folder
- [x] HOST-004 Long-path fallback
- [x] HOST-005 Windows pipe
- [x] HOST-006 Other host files
- [x] HOST-007 Private folder
- [x] HOST-008 Private file write
- [x] HOST-009 Hook config token check
- [x] HOST-010 Hook config content
- [x] HOST-011 Trust on macOS
- [x] HOST-012 Trust on Windows
- [x] HOST-013 Protocol number
- [x] HOST-014 Line framing
- [x] HOST-015 Several per read
- [x] HOST-016 Split messages
- [x] HOST-017 Split characters
- [x] HOST-018 Blank and bad lines
- [x] HOST-019 Newlines inside strings
- [x] HOST-020 No line limit
- [x] HOST-021 Any JSON value
- [x] HOST-022 Arguments
- [x] HOST-023 Usage error
- [x] HOST-024 One host per socket
- [x] HOST-025 Stale socket
- [x] HOST-026 Hook server first
- [x] HOST-027 Socket listen
- [x] HOST-028 Socket mode
- [x] HOST-029 Windows key
- [x] HOST-030 Hook token
- [x] HOST-031 Ready log
- [x] HOST-032 Unused host
- [x] HOST-033 Crash-proof
- [x] HOST-034 Log lines
- [x] HOST-035 Background process
- [x] HOST-036 Hello first
- [x] HOST-037 Welcome
- [x] HOST-038 App clients
- [x] HOST-039 Other protocols may only stop it
- [x] HOST-040 Ignored messages
- [x] HOST-041 Spawned reply
- [x] HOST-042 Broadcasts
- [x] HOST-043 Socket errors
- [x] HOST-044 Disconnect
- [x] HOST-045 Idle check on close
- [x] HOST-046 Newest app
- [x] HOST-047 Idle rule
- [x] HOST-048 Duplicate spawn
- [x] HOST-049 PTY spawn
- [x] HOST-050 POSIX argv
- [x] HOST-051 Windows command line
- [x] HOST-052 Spawn error
- [x] HOST-053 Late spawn failure (macOS)
- [x] HOST-054 Spawn log
- [x] HOST-055 Output
- [x] HOST-056 Output buffer
- [x] HOST-057 Exit
- [x] HOST-058 Exit code
- [x] HOST-059 Write
- [x] HOST-060 Status keys in history
- [x] HOST-061 Resize
- [x] HOST-062 Kill on POSIX
- [x] HOST-063 Kill on Windows
- [x] HOST-064 Shutdown
- [x] HOST-065 Stopping host exits
- [x] HOST-066 History cap
- [x] HOST-067 Slim payloads
- [x] HOST-068 MCP path first
- [x] HOST-069 Token sources
- [x] HOST-070 Constant-time compare
- [x] HOST-071 Rejections
- [x] HOST-072 Accept first
- [x] HOST-073 Unknown target
- [x] HOST-074 Payload
- [x] HOST-075 Status line
- [x] HOST-076 History
- [x] HOST-077 Broadcast
- [x] HOST-078 Null payload
- [x] HOST-079 MCP auth
- [x] HOST-080 Non-POST
- [x] HOST-081 Size cap
- [x] HOST-082 Parse error
- [x] HOST-083 Requests
- [x] HOST-084 Notifications only
- [x] HOST-085 Relay
- [x] HOST-086 Relay settles
- [x] HOST-087 No app
- [x] HOST-088 Unanswered
- [x] HOST-089 Answer
- [x] HOST-090 Relay failure
- [x] HOST-091 Starting a host
- [x] HOST-092 Waiting for it
- [x] HOST-093 One attempt at a time
- [x] HOST-094 Connect order
- [x] HOST-095 Launch adopts, never starts
- [x] HOST-096 Hello
- [x] HOST-097 Welcome timeout
- [x] HOST-098 Trust first
- [x] HOST-099 Other protocol
- [x] HOST-100 Adopted in order
- [x] HOST-101 Later messages
- [x] HOST-102 Replace dialog
- [x] HOST-103 Quit
- [x] HOST-104 Stop Sessions
- [x] HOST-105 Adopt: hook config
- [x] HOST-106 Adopt: endpoint
- [x] HOST-107 Adopt: board tools
- [x] HOST-108 Adopt: sessions
- [x] HOST-109 Host lost
- [x] HOST-110 No reconnect
- [x] HOST-111 Quit leaves sessions running
- [x] HOST-112 Stop all
- [x] HOST-113 Spawn once
- [x] HOST-114 Spawn request
- [x] HOST-115 Spawned
- [x] HOST-116 Output of unknown sessions
- [x] HOST-117 App buffer
- [x] HOST-118 Coalescing
- [x] HOST-119 Snapshot
- [x] HOST-120 Exit
- [x] HOST-121 Write
- [x] HOST-122 Send prompt
- [x] HOST-123 Resize
- [x] HOST-124 Kill
- [x] HOST-125 Kill a project
- [x] HOST-126 Board tools requests
- [x] HOST-127 Sends while disconnected
- [x] HOST-128 Live list
- [x] HOST-129 Claude command (shared with 03)
- [x] HOST-130 Claude environment
- [x] HOST-131 Terminal command
- [x] HOST-132 Three statuses
- [x] HOST-133 Combined status
- [x] HOST-134 Only changes
- [x] HOST-135 Process start
- [x] HOST-136 Process end
- [x] HOST-137 Forget
- [x] HOST-138 Known events
- [x] HOST-139 Agent hooks
- [x] HOST-140 Last hook time
- [x] HOST-141 Working
- [x] HOST-142 Tool use
- [x] HOST-143 Permission
- [x] HOST-144 Notification
- [x] HOST-145 About permission
- [x] HOST-146 Stop
- [x] HOST-147 SessionStart
- [x] HOST-148 SessionEnd
- [x] HOST-149 Subagent events without an agent
- [x] HOST-150 Agent stop
- [x] HOST-151 Agent waiting
- [x] HOST-152 Agent working
- [x] HOST-153 Agent record
- [x] HOST-154 Agent events
- [x] HOST-155 Longest waiting
- [x] HOST-156 Settle
- [x] HOST-157 Hold
- [x] HOST-158 Hook times
- [x] HOST-159 Untracked sessions
- [x] HOST-160 When keys count
- [x] HOST-161 What a key is
- [x] HOST-162 Answering an agent
- [x] HOST-163 Interrupt
- [x] HOST-164 Answering the main thread
- [x] HOST-165 Replayed interrupt
- [x] HOST-166 Usage report
- [x] HOST-167 Model
- [x] HOST-168 Context tokens
- [x] HOST-169 Context size and share
- [x] HOST-170 Cost and time
- [x] HOST-171 Usage dedupe
- [x] HOST-172 Pause and resume
- [x] HOST-173 Usage kept
- [x] HOST-174 Endpoint
- [x] HOST-175 Settings per process
- [x] HOST-176 Hook environment
- [x] HOST-177 POSIX hooks with a config
- [x] HOST-178 POSIX hooks for an older host
- [x] HOST-179 PowerShell hooks
- [x] HOST-180 Settings JSON
- [x] HOST-181 Settings cache
- [x] HOST-182 Silent hooks
- [x] HOST-183 Agent files
- [x] HOST-184 Meta file
- [x] HOST-185 Incremental parse
- [x] HOST-186 Times
- [x] HOST-187 Background tasks
- [x] HOST-188 New prompt
- [x] HOST-189 Pending work
- [x] HOST-190 Steps
- [x] HOST-191 Reply line
- [x] HOST-192 End time
- [x] HOST-193 Status: not live
- [x] HOST-194 Status: unheard
- [x] HOST-195 Status: woken later
- [x] HOST-196 Status: ended without a stop
- [x] HOST-197 Agent info
- [x] HOST-198 Agents list
- [x] HOST-199 Agents on cards
- [x] HOST-200 Agent transcript ids
- [x] HOST-201 Agents refresh
- [x] HOST-202 Notice rules
- [x] HOST-203 Previous status
- [x] HOST-204 Withdraw
- [x] HOST-205 Gates
- [x] HOST-206 Armed after catch-up
- [x] HOST-207 Which sessions
- [x] HOST-208 Content
- [x] HOST-209 One per session
- [x] HOST-210 Unsupported
- [x] HOST-211 Click
- [x] HOST-212 Open a card
- [x] HOST-213 Take the open request
- [x] HOST-214 Waiting count
- [x] HOST-215 Dock badge
- [x] HOST-216 Running counts
- [x] HOST-217 Working and waiting counts
- [x] HOST-218 Running total
- [x] HOST-219 By project
- [x] HOST-220 Status side effects
- [x] HOST-221 Launch order
- [x] HOST-222 Open
- [x] HOST-223 Write path
- [x] HOST-224 Resize and kill
- [x] HOST-225 Quitting
