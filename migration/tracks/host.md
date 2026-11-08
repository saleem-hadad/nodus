# Track: host (P2a)

- **Branch:** `rust/host`
- **Spec:** `migration/features/02-session-host-status.md`, HOST-001 to HOST-128 (host protocol, host process, the app's client).
- **Crates:** `nodus-host-proto`, `nodus-host`, `nodus-sessions::client`, the `host` subcommand in `nodus`.

## Progress

| Module | State |
| --- | --- |
| `nodus-host-proto` (messages, reader, paths, Windows command line) | done |
| `nodus-host` PTY layer (macOS, Windows) | — |
| `nodus-host` host process (socket, sessions, hooks, MCP relay, lifecycle) | — |
| `nodus` `host` subcommand | — |
| `nodus-sessions::client` | — |
| Wire tests against the 0.9.4 Node host | — |

## Done (implemented and tested)

HOST-001–021 (nodus-host-proto; hostProtocol.test.ts ported in full).

## Deferred

None yet.

## Notes for whoever resumes

- `PROTOCOL` stays 1 (decision U1 is settled at packaging).
- Spec §13 says a Windows spawn with no arguments uses the quoted program; node-pty 1.1.0's
  `argsToCommandLine(file, '')` returns `file` exactly as given. The port follows node-pty.
