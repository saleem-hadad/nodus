# Track: cutover (P12, last step)

- **Branch:** `rust/cutover`, from `main`. **Not merged until the user approves.**
- **Scope:** turn the repo into a Rust-only project: decision U1's protocol bump, CI and the release workflow on cargo, README and `docs/how-it-works.md` for the Rust workspace, the Electron sources and toolchain removed.
- **Not in scope:** cutting the release itself (version number, secrets, the workflow run), and `migration/plan.md` (the orchestrator updates it).

## Steps

| # | Step | State |
| --- | --- | --- |
| 1 | `PROTOCOL` → 2 (U1, default (a)) | ✅ |
| 2 | CI and release workflows on cargo only | ✅ |
| 3 | README and how-it-works for the Rust app | ✅ |
| 4 | Electron sources and toolchain removed | ✅ |
| 5 | Workspace version 1.0.0 (the user's pick, 2026-11-10) | ✅ |
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

## 3. Docs

- README: only "Build from source" changed (cargo run / test, the bundle and DMG scripts, the Windows installer script); same layout and voice.
- `docs/how-it-works.md`: the diagram (nodus-ui ⇄ nodus-core ⇄ `Nodus host`), the host running as `Nodus host` (was "as plain Node"), the xAI key "never reaches the window" (was "the main process"), the key hook (event tap / low-level hooks, was uiohook-napi), and Project layout, Built with, Development, Releasing and Troubleshooting installs rewritten for the workspace. Every user-facing section is otherwise unchanged; `NODUS_USER_DATA` is still documented.

## 4. Electron removed

- **Deleted:** `src/` (155 files), `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `electron-builder.yml`, `electron.vite.config.ts`, `tsconfig.json`, `tsconfig.node.json`, `tsconfig.web.json`, `vitest.config.mts`, `.nvmrc`, `scripts/postinstall.cjs`, `scripts/smoke.mjs` (now `packaging/smoke.py`), `build/entitlements.mac.plist` (the bundle signs with `packaging/macos/entitlements.plist`). `.gitignore` drops `out` (electron-vite's); `node_modules` stays for `migration/parity`'s `npm ci`, `dist` for the packages. `build/` keeps the icons (`icon.icns` goes in the bundle and DMG, `icon-win.png` is `icon.ico`'s source, `icon.png`/`icon.svg` the artwork).
- **What depended on them, and now reads tag v0.9.4 instead:**
  - `nodus-updater/tests/swap.rs` compared `SWAP_SCRIPT` with `src/main/updater.ts` "while it's in the repo"; it now compares with `tests/fixtures/swap-script-0.9.4.sh`, the template copied byte for byte out of v0.9.4's `updater.ts`.
  - The four fixture generators (`nodus-sessions`, `nodus-terminal`, `nodus-tools`, `nodus-views`: `tests/fixtures/generate.mjs`) read `src/` (and `node_modules` for xterm.js) through `NODUS_V094`, a checkout of the tag, and say the committed fixtures came from v0.9.4's sources. Checked: run against `git archive v0.9.4` (Electron 44.5.1 as Node 24), all four rewrite their fixtures byte for byte unchanged.
  - `migration/parity/capture-electron.mjs` defaults `--app` to `../nodus-0.9.4` (a `git worktree add ../nodus-0.9.4 v0.9.4`) and says so if it's missing; the README leads with that.
  - The Node-host tests (`nodus-host/tests/wire.rs`, `nodus-sessions/tests/client.rs`) document building `host.js` from the tag.
- `packaging/` never read `node_modules` at build time: dmgbuild, makensis and the NSIS plugins are electron-builder-binaries' pinned archives, downloaded and checked by SHA-256, and the NSIS templates are vendored.
- **What still names Electron, on purpose:** code and docs about 0.9.4 itself, which every install upgrades from: reading its safeStorage ciphertext and Chromium Local Storage, its updater's checks (handoff gates), its installer's registry id and NSIS templates (electron-builder's, MIT), its parity goldens, and doc comments saying which `src/…` file a module ports. `migration/` is kept as the record.

## 5. Version 1.0.0

The user approved the cutover and picked 1.0.0 for the first Rust release (2026-11-10). `[workspace.package] version` is `1.0.0`, and `Cargo.lock`'s 19 workspace crates with it (nothing else moved, so `--locked` builds pass). Where it flows, with nothing else to edit:

- `env!("CARGO_PKG_VERSION")` → `nodus_core::VERSION`: the updater's `current` (Settings, the "is it newer" check against `releases/latest`), the board tools' MCP `serverInfo.version`, Help › Report a Bug's body.
- `packaging/macos/bundle.sh` (`cargo pkgid -p nodus`): `CFBundleShortVersionString` and `CFBundleVersion` in Info.plist, which 0.9.4's updater compares with the release tag; `dmg.sh`: the volume name `Nodus 1.0.0-<arch>`.
- `packaging/windows/installer.sh`: the NSIS `VERSION` define (`VIProductVersion`, `BrandingText`, `DisplayVersion`, `DisplayName`) and the `nodus-1.0.0-<arch>.nsis.7z` package; `crates/nodus/build.rs`: `Nodus.exe`'s FileVersion `1.0.0` and ProductVersion `1.0.0.0`.
- The workflows: `package.yml`'s gates and `release.yml`'s version check (it must equal the `version` input, be `X.Y.Z`, newer than 0.9.4 and the latest release, and not released yet).

Left at 0.9.4 on purpose: everything naming the Electron release being replaced (the handoff gates' downloads and checks, the updater's tests of an installed 0.9.4, `PROTOCOL_0_9_4`, the parity scenes' Settings rows, which match 0.9.4's goldens). No test pinned the app's own version.
