# Track: actions-voice-git (P3)

- **Branch:** `rust/actions-voice-git`
- **Spec:** `migration/features/05-actions-voice-git.md` (183 behaviors: ACT, VOICE, GIT)
- **Crates:** `nodus-git`, `nodus-actions`, `nodus-voice`
- **Shared files touched (additive):** root `Cargo.toml` `[workspace.dependencies]` (`icu_collator`,
  `objc2-audio-toolbox`, `objc2-core-audio-types`, `opus1`; the `windows` comment).

## Status

| Module | State |
| --- | --- |
| `nodus-git` (runner, status, log, diff, diff parser, labels) | ✅ |
| `nodus-actions` (normalize, lists, draft, run args, rail and dialog logic) | ✅ |
| `nodus-voice` (key, xAI request, hint words) | ✅ |
| `nodus-voice` recorder (capture, Opus, WebM, meter) and controller (`useVoice`) | ✅ |

Checked with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (macOS and
`--target x86_64-pc-windows-msvc`), `cargo test --workspace` and `sh migration/prepush.sh main..HEAD`. Since main
gained nodus-ui, GPUI's build script can't run for the Windows target on a Mac (it needs a resource compiler), so
the Windows check runs with `--exclude nodus-ui`; everything else is clean.

**147 of 183 done** (37 GIT, 71 ACT, 39 VOICE; some in other crates, listed), **36 deferred** (UI or core wiring).

## Done (implemented and tested)

**nodus-git:** GIT-001, GIT-002 (stand-in git: argv, `GIT_TERMINAL_PROMPT=0`, non-zero exit as an answer, a signal and
32 MiB as failures, `spawn git ENOENT`), GIT-004…GIT-009, GIT-010 (the same status feeds the labels and voice's hint
words), GIT-011, GIT-012, GIT-013 (`clamp_limit`), GIT-014…GIT-020, GIT-048…GIT-055. git.test.ts (`tests/git.rs`, real
git in temp repos with `GIT_CONFIG_GLOBAL=/dev/null`) and diff.test.ts (`parse.rs`) are ported.

Pure parts for nodus-ui (`nodus_git::labels`), tested; the drawing is UI: GIT-025 (tooltip, `999+` badge), GIT-026
(label, `↑n`/`↓n`), GIT-029 (head and tracking line), GIT-034 (`<n> changed file(s)` / `No changes`), GIT-036 (labels,
tooltip, folder/name split), GIT-037 (icon names), GIT-038 (selection by path then index), GIT-039 (↑/↓/Home/End),
GIT-041 (cache key, the "same diff" test incl. the empty-patch-turned-too-large quirk), GIT-047 (gutter width, signs).

**nodus-actions:** ACT-001…ACT-012 (`normalize_action` over the JSON the window sends; actions.test.ts ported),
ACT-002, ACT-013…ACT-015 (`Actions` over the `Store`: scopes, moving between them, empty lists left out), ACT-016
(`draft_action`; the env wait is core's), ACT-019…ACT-027 (`draft_with`: argv, prompt, schema and system prompt
byte-checked against actions.ts, cwd, stdin closed, SIGKILL at 2 min on a paused clock, spawn failure text, JSON and
fallback reasons; against a stand-in `claude`), ACT-028 (`run_args`; runArgs test ported), ACT-083 (`remote_error`).

Logic for nodus-ui (`nodus_actions::rail`, `nodus_actions::dialog`), tested; the drawing is UI: ACT-031 and ACT-039
(toast texts), ACT-047…ACT-052 (`Rail`: reveal 100 ms, hide 300 ms / 0 ms, keep open, shown with the menu),
ACT-054…ACT-057, ACT-059, ACT-060, ACT-062…ACT-066 (`fan_top`, fan step and status; the duration is 08's
`formatDuration`), ACT-067…ACT-078 (`ActionDialog`: steps, chips, scope, request counter, stale drafts, save
payload, footer buttons, ⌘/Ctrl+Enter), ACT-080.

**Done in other crates (on main, tested there):** ACT-029, ACT-030 (nodus-board `new_session` / `create_session`,
nodus-sessions `launch::for_create`), ACT-034…ACT-038 (nodus-board `on_done`, the load sweep, `finish_run` emitting
`Event::RunFinished`), ACT-040/041 (nodus-sessions `launch::is_takeover`; the spec's quirk table is also run from
`nodus-actions/tests/actions.rs`), ACT-043 (`Board::claim_run`), ACT-044 (`send_to_session`). ACT-033:
`nodus_actions::is_run` is the predicate; the session tools are 04's (nodus-tools). VOICE-008 (nodus-store keeps
only a non-empty `voiceKey`), VOICE-009, VOICE-010 (`nodus_platform::permissions`).

**nodus-voice:** VOICE-001, VOICE-003…VOICE-007 (`VoiceKeys` with nodus-platform's `SafeStorage`: saved key wins,
an unopenable one still reads `settings` and falls back to the shell's, Keychain-unavailable text, `voiceKey` is
base64 `v10…`), VOICE-011…VOICE-018 (`Voice::transcribe` against a local stand-in for xAI: endpoint path, bearer
key, multipart parts in order, `prompt.webm` / `audio/webm`, hint words, 90 s on headers and body, every message),
VOICE-019 (`mime_arg`; the env wait and project are core's), VOICE-020…VOICE-023 (`Recording`), VOICE-024…VOICE-037
(`controller::VoiceController` on a paused clock: phases, one mic per window slot, no key → Settings, mic refused,
open failure fading after 4.2 s, 600 ms floor, 5 min cap, Retry/Dismiss, stale answers dropped, Esc targets, unmount,
stuck `opening` when asking fails, `clock`). Logic for the UI: VOICE-040 (`MICROPHONE_SETTINGS_LABEL`), VOICE-043
(`session_mic_button`), VOICE-044 (`halo_spread`), the bars' `bar_scale`.

### The recorder (VOICE-020…VOICE-023)

- **Capture.** macOS: Core Audio's voice-processing I/O unit (`kAudioUnitSubType_VoiceProcessingIO`: Apple's echo
  cancellation, noise suppression and AGC), 48 kHz mono float from the unit, silence on its speaker side, other
  apps' audio ducked as little as macOS 14+ allows. AVAudioEngine's voice processing was tried first: its input tap
  never fired, and with an output chain the engine wouldn't start (-10875). Windows: WASAPI shared mode on the
  default capture endpoint in `AudioCategory_Communications`, 48 kHz mono float via `AUTOCONVERTPCM`. Each device
  runs on its own thread. Open failures read as Chromium's (`Requested device not found`, `Permission denied`,
  `Could not start audio source`).
- **Encoding.** As Chromium's MediaRecorder (`AudioTrackOpusEncoder` + `WebmMuxer`/mkvmuxer live mode): 48 kHz mono,
  60 ms Opus frames, `OPUS_APPLICATION_AUDIO`, automatic bitrate, VBR, ≤ 4000-byte packets, the tail under one frame
  dropped; WebM with unknown-size Segment and Clusters, no Cues, `Chrome` as muxing/writing app, Chromium's OpusHead
  (pre-skip 0), BitDepth 32, a new Cluster only past 32 767 ms. Opus is `opus1`, a pure-Rust port of libopus 1.6
  (bit-exact on arm64 per its authors), so the Windows check needs no C toolchain. Other rates go through a
  windowed-sinc resampler to 48 kHz.
- **Meter.** Chromium's `RealtimeAnalyser` maths (Blackman, |X|/N, 0.7 smoothing per analysis, −100…−30 dB to bytes)
  and 0.9.4's five bands, at 48 kHz (the rate recorded), where Chromium used the output device's rate.
- **Verified:** synthetic tones at 44.1/48 kHz through the whole pipeline, decoded back with opus1's decoder;
  `ffprobe` reads the file as `matroska,webm`, Opus 48 kHz mono, `encoder=Chrome`, no duration (like Chrome's);
  `ffmpeg` decodes every packet (its "Error parsing Opus packet header" at EOF comes from flushing, with 0 decode
  errors). A real 2 s capture through the voice-processing unit was run by hand (`NODUS_VOICE_MIC=<file> cargo test
  -p nodus-voice -- --ignored records_the_real_microphone`), only because macOS already allowed the microphone; it
  checks the permission first and skips rather than prompt. Windows capture is compiled and linted only.

## Deferred

- Core wiring: GIT-003, VOICE-002 (wait for the login-shell env, resolve `repoPath`, `Unknown project <id>`), ACT-042
  (check `launch::is_takeover` → `Board::claim_run` before each `session:write`).
- UI: GIT-021…GIT-024, GIT-027, GIT-028, GIT-030…GIT-033, GIT-035, GIT-040, GIT-042…GIT-046 (store refresh and
  polling, popover timers, modal layout; GIT-031 uses 10's `timeAgo`, CARD-041); ACT-017, ACT-018 (window store),
  ACT-032, ACT-045 (canvas, 09), ACT-046, ACT-053, ACT-058, ACT-061, ACT-079, ACT-081, ACT-082; VOICE-038, VOICE-039,
  VOICE-041, VOICE-042, VOICE-045 (drawing; VOICE-033's capture-phase interception is the UI calling `on_escape`).

## What nodus-core implements

- `git:status` = `Git::from_env(&env).status(repo)`; an `Err` reads as `null` in the window (GIT-021).
- `git:diff` = `Git::diff(repo, &file)`; errors go to the window as `remote_error("git:diff", …)`.
- `git:log` = `Git::log(repo, clamp_limit(limit))`.
- `actions:global` / `actions:save` / `actions:remove` = `Actions::new(store)` `.global()` / `.save(id, &raw)` /
  `.remove(id, action_id)`; `raw` is the JSON the window sends (`null` is fine).
- `actions:draft` = after the env, `draft_action(&env, repo, description, global == Some(true))`.
- A run's launch: `StartNew.action` → `launch::ActionRun { prompt: &action.prompt, args: run_args(&action) }`.
- `nodus-tools`' `isRun` = `store.read(|s| is_run(s, session_id))`.
- `voice:key-source` / `voice:set-key` = `Voice::new(store, Arc<SafeStorage>)` `.key_source(&env)` /
  `.set_key(&env, key_arg(&arg))`; `voice:transcribe` = `.transcribe(&Git::from_env(&env), &project, bytes,
  &mime_arg(&arg), shell_key(&env).as_deref())`; `voice:microphone(-settings)` = `nodus_platform::permissions`.
- **Trait `controller::VoiceBackend`** (one per window, shared by its mics with one `VoiceSlot`): `key_source`,
  `microphone`, `start_recording` (= `controller::open_microphone()`), `transcribe` (errors as `remote_error`), and
  `open_settings` (the UI store). Each mic is a `VoiceController::new(backend, slot, on_text)`; the UI draws
  `state()`/`subscribe()`, reads `levels()` each frame, routes Esc through `on_escape`, and drops it to unmount.
- `rail::Rail` and `dialog::ActionDialog` are the UI's models; the UI makes the `actions:*` and `session:create`
  calls they hand it.

## Notes for the orchestrator

- **Collation (GIT-009).** Changed files sort with ICU4X's root collation at tertiary strength, which is what V8's
  `localeCompare` uses for `en` (checked against Node: `a.txt`, `B.txt`, `c.txt`; `_x`, `1`, `a`; `src-a/` before
  `src/`; `file10` before `file2`). A user whose system locale tailors collation (Swedish, say) gets English order.
- **Node's `path`.** `nodus_git::nodepath` ports `resolve`, `relative`, `isAbsolute` and `parse().name` for both POSIX
  and win32, checked against Node; voice's hint words use `parse_name` too (PD-8).
- The empty-tree id is cached for the process, as 0.9.4 does (a SHA-256 repository after a SHA-1 one reuses it).
- **Draft output decoding.** 0.9.4 decodes each stdout chunk of `claude -p` on its own, so a character split across
  chunks turns into U+FFFD; the port decodes the whole output once (data corruption, not a behavior).
- **Key order.** `Project` and `State` are structs, so a project's `actions` and the top-level `actions` are written
  where the struct puts them, not where JavaScript appends a key it adds later (P1's choice, invisible to readers).
- **Audio differences to know about.** Apple's and Windows' voice processing replace Chromium's WebRTC processing,
  so the audio sent to xAI isn't sample-identical; Windows applies echo cancellation only where the endpoint or
  Windows provides it for the communications category. On macOS 13 other apps' audio is ducked while recording
  (macOS 14+ lets it be kept to the minimum). The level meter runs at 48 kHz rather than the output device's rate.
- **`opus1`** is young (2026) and not yet widely used; libopus through C would need an MSVC toolchain for the
  Windows cross-check. Swapping it is contained in `encode.rs`.
