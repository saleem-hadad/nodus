# Track: cutover (P12, last step)

- **Branch:** `rust/cutover`, from `main`. **Not merged until the user approves.**
- **Scope:** turn the repo into a Rust-only project: decision U1's protocol bump, CI and the release workflow on cargo, README and `docs/how-it-works.md` for the Rust workspace, the Electron sources and toolchain removed.
- **Not in scope:** cutting the release itself (version number, secrets, the workflow run), and `migration/plan.md` (the orchestrator updates it).

## Steps

| # | Step | State |
| --- | --- | --- |
| 1 | `PROTOCOL` → 2 (U1, default (a)) | ✅ |
| 2 | CI and release workflows on cargo only | ✅ |
| 3 | README and how-it-works for the Rust app | ☐ |
| 4 | Electron sources and toolchain removed | ☐ |
| 5 | Workspace version: where the release number goes | ☐ |
| 6 | Final check: fmt, clippy, tests, prepush, arm64 bundle + smoke | ☐ |

## 1. Protocol (U1)

- `nodus_host_proto::PROTOCOL` is 2; `PROTOCOL_0_9_4` (1) names what 0.9.4 says. The messages are unchanged, so a 0.9.4 host or app is simply "another version": HOST-099…104's documented flow ("Sessions from another version of Nodus are still running", Stop Sessions / Quit) runs, and the sessions resume from their cards.
- Tests: `nodus-host-proto` (the number, and `hello`/`welcome` reading 0.9.4's 1); `nodus-sessions/tests/client.rs` `host_099_102_104_a_0_9_4_host_is_another_version_and_its_sessions_are_stopped` and `host_103_quit_leaves_a_0_9_4_host_running` (a fake host sending 0.9.4's welcome with a running session: asked once, `shutdown` sent, nothing adopted, no hook endpoint taken; Quit sends nothing); `nodus-host/tests/lifecycle.rs` `host_038_039_a_0_9_4_app_is_another_version_and_may_only_stop_the_host` (the other direction: a 0.9.4 app on a Rust host).
- The wire test still proves 0.9.4's shapes: `fixtures/wire-transcript.txt` is untouched (recorded from the Node host); the Rust run must match it line for line, its welcomes saying 2 where the fixture says 1.
- Against the real 0.9.4 Node host (built from `git archive v0.9.4` with esbuild, run by Electron 44.5.1 as Node): the wire fixture still matches, and `node_host_a_running_0_9_4_host_is_offered_to_be_stopped` passes (a 0.9.4 app's session on it; the Rust app asks; Quit leaves it running; Stop Sessions shuts it down with its session; nothing adopted). It replaces the old test that had the Rust client adopt the Node host, which U1 rules out.

## 2. CI and release

- `ci.yml` (the README's build badge): what `rust.yml` was, on every pull request and push to main: `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` and the GPUI effects example, on macOS and Windows. The pnpm jobs (typecheck/test/build) and the Windows Electron smoke test are gone; the Windows smoke test runs on the native build in `package.yml`. `rust.yml` is deleted.
- `package.yml` is `rust-package.yml` renamed (name `Packages`), paths filter kept: bundles, DMGs, installers, handoff gates and the native smoke test on both OSes.
- `release.yml` is `release-rust.yml` renamed (name `Release`), replacing 0.9.4's tag-triggered Electron release. Still started by hand, with every gate (version checks, tests, signing and notarizing, the handoff gates, draft → digests checked → published). A one-line comment above the trigger shows how to switch to `v*` tags.
- Actions stay pinned by the same SHAs.
