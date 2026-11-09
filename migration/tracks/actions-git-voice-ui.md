# Track: actions-git-voice-ui (P9, spec 05's UI)

- **Branch:** `rust/actions-git-voice-ui`
- **Spec:** `migration/features/05-actions-voice-git.md`: the UI behaviors the actions-voice-git track left to the
  window (GIT-021…GIT-047, ACT-017/018/031/032/039/045…082, VOICE-033, VOICE-038…045); spec 12's MODAL-019 (the
  session window's mic).
- **Sources:** `components/actions/*`, `components/git/*`, `components/voice/*`, `TitleBar.tsx`, the store's git and
  actions parts, `BoardView.tsx`'s `runAction` and `runFinished`.
- **Code:** `crates/nodus-ui/src/git/` (branch chip, Changes button, Changes window), `crates/nodus-ui/src/actions/`
  (the rail, its fan, menu and confirm; the dialog in `dialog.rs` and `dialog_view.rs`; spec 05's icons),
  `crates/nodus-ui/src/voice/` (the mics, the recording look and notices).

## Status

✅ ready to merge. Done: GIT 27, ACT 43, VOICE 9, plus MODAL-019. Deferred: none of this track's; two spec-05 IDs
still unticked belong to the core (ACT-016's and VOICE-019's env wait and project lookup, see below). Every scene is
within 0.03–3.3 % of its golden in its own region (text rasterisation and metrics, gpui-text's), see the table.

| Piece | State |
| --- | --- |
| Icons (lucide 1.49.0, `actions::icons`, plus Chromium's textarea grip) | ✅ |
| Git: `GitModel` (refresh, polling, reset), branch chip + log popover, Changes button | ✅ compared |
| Git: the Changes window | ✅ compared |
| Voice: `Mic`, `voice_status`, `voice_notice`, `session_mic`, `card_mic`; the session window's mic | ✅ compared (idle) |
| Actions: rail, hint, fan, sliver, menu, delete confirm, on the board canvas's run hook | ✅ compared |
| Actions: dialog (describe, drafting, review, save, errors) | ✅ compared |

## How to resume

Read this file, then `git log --oneline main..rust/actions-git-voice-ui`.

## Scenes

```sh
cargo build -p nodus
python3 crates/nodus-ui/parity/scenes05.py [prefix…] [--theme light|dark] --out <dir> --world <scratch>
python3 crates/nodus-ui/parity/terminal_scenes.py terminal-claude --golden <dir> --world <scratch>
NODE_PATH=migration/parity/node_modules node crates/nodus-ui/parity/compare.mjs <dir> <scene>.<theme> x y w h
```

`scenes05.py` renders `git-log`, `git-diff` and `actions-*` from the harness's seed with its stand-in `claude` (never
the real one; drafts come from `responses.json`) and never records from the microphone. It seeds with Node 22+ (the
stand-ins are ES modules; under an older Node every session, a run's too, exits at once). `<dir>` holds copies of
the goldens; shots land beside them as `<scene>.<theme>.rust.png`. Under heavy load the rail's 100 ms reveal can
miss the scripted pointer path in `actions-run`; run it again alone.

### Checked against the goldens (each scene's own region, CSS px)

Light is measured on main with gpui-text's text fixes (after the rebase); dark before them.

| Scene | Region | Light | Dark | What differs |
| --- | --- | --- | --- | --- |
| git-log | chip + popover (250, 0, 520, 330) | 2.76 % | 2.82 % | text rasterisation only |
| git-diff | Changes window (130, 56, 1180, 790) | 0.33 % | 1.05 % | text rasterisation only |
| git-diff | title bar (0, 0, 1440, 52) | 0.24 % | 0.20 % | text |
| actions-rail | rail (1376, 350, 64, 255) | 0.06 % | 0.03 % | — |
| actions-rail | hint (1176, 404, 196, 38) | 3.3 % | 4.2 % | text rasterisation only |
| actions-run | fan card (1112, 356, 256, 78) | 1.16 % | 1.38 % | the card sits 0.5 px lower; text |
| actions-run | rail with the running tile and dot (1376, 350, 64, 255) | 0.14 % | 0.12 % | — |
| actions-new | dialog (410, 180, 620, 420) | 1.94 % | 1.60 % | chip labels ~0.2 px wider per glyph (text metrics) |
| actions-drafting | dialog (410, 180, 620, 420) | 1.76 % | 1.51 % | same |
| actions-review | dialog (410, 140, 620, 620) | 0.26 % | 2.31 % | line boxes rounded to device pixels (+0.5 px below the header); text |
| terminal-claude | the mic (1118, 62, 40, 32) | 2.11 % | 2.09 % | the header's actions sit 0.5 px right (terminal-ui's layout, Stop too) |

Whole-shot numbers are higher because the board's cards and notes behind are still placeholders (board-cards).

## Done (implemented; IDs in the code; checked against the goldens where a scene shows them)

- **GIT (27):** GIT-021…GIT-047. Chip, ahead/behind, popover timing and contents, relative times, Changes button and
  count, the window's header, file list, status icons and colours, selection and focus look, ↑/↓/Home/End, the
  diff's gutter, signs, colours and changed spans (scenes); shared reads, reset on navigation, 4 s polling while
  focused, the cache by `<status>\0<oldPath>\0<path>`, binary / too large / error panes and scroll keeping (code, on
  `nodus_git::labels`' tested logic).
- **ACT (43):** ACT-017, ACT-018 (lists after save/remove and with every projects refresh), ACT-031 (the board
  canvas's `run_action`: spot, store, `Couldn't run …` toast), ACT-032 (runs left off the canvas: board canvas),
  ACT-039 (the canvas's `run_finished` toast and closing the run's terminal), ACT-045 (the canvas's
  `join_taken_over_runs`), ACT-046…ACT-082.
- **VOICE (9):** VOICE-033 (Esc taken by a keystroke interceptor while busy, unless another field or a terminal has
  the keyboard), VOICE-038…VOICE-045. The idle mic is in `terminal-claude`; the recording look, spinner and notice
  have no golden and follow `voice.css`'s values (sizes, colours, the 2 + 6 × level halo, 80 ms / 70 ms transitions).
- **MODAL-019** (spec 12): the session window's mic types what was said at the prompt, resumes a stopped session and
  focuses the terminal (`SessionWindow::type_in`).

## Not this track's (still unticked in spec 05)

- ACT-016, VOICE-019: the commands' wait for the login-shell env and project lookup are nodus-core's
  (`actions:draft`, `voice:transcribe`); the UI calls them as they are.

## What other tracks mount

- **The actions rail (board):** `actions::action_rail(&state, &menus, RailHooks { run }, window, cx)`, an absolute
  layer over the board area; `shell/root.rs` mounts it beside the board canvas with `run` = the canvas's
  `run_action(id, name)`. The canvas's `overlay_open()` counts `actions::overlay_open(cx)` (the rail's dialog, menu
  or delete confirmation; ACT-053). A board view that wants to mount it itself makes the same call.
- **Session mic (window):** `voice::session_mic(id, SessionMicProps { project_id, variant: MicVariant::Window, scale:
  1., on_text }, window, cx)`; mounted in `terminal/session_window.rs`.
- **Session mic (expanded card, board-cards):** `voice::card_mic(session_id, project_id, zoom, type_in, window, cx)`
  returns the `.card-tool` mic drawn at the board's zoom; `type_in` is `Rc<dyn Fn(String, &mut App)>`, which should
  call `CardTerminals::type_in(id, &text, !live, cx)` and wake the card (XCARD-025). That's board-cards'
  `TerminalHost::mic`.
- **The prompt bar's mic (overlays, spec 11):** `cx.new(|cx| voice::Mic::new(&nodus, project_id, on_text, window, cx))`,
  `Mic::set_own_field(Some(field_focus))` so Esc in the bar cancels, then `voice::voice_status(id, &mic, false, Z(1.),
  window, cx)` and `voice::voice_notice(id, &mic, NoticeAnchor::Bar, Z(1.), window, cx)`; `voice::spinner` is the
  `.spin` loader. All mics in a window share one listening slot (VOICE-025).
- **The capture panel's mic (07, merged):** it holds its own `VoiceController` (its window's slot) and now draws its
  recording look with `voice::voice_status_of(id, phase, elapsed, levels, …)`, the same status as the other mics
  (its own copy lacked the clock's tabular figures and the bars' 70 ms easing). Its notice stays its own: 0.9.4
  gives `.voice-notice` no pill there, unlike the session mic's and the bar's (`NoticeAnchor`).

## Known differences

- Text: SF Pro and Menlo glyph advances and rasterisation (gpui-text), and line boxes rounded to device pixels (a
  two-line subtitle is 0.4 px taller).
- The run fan sits 0.5 px lower than 0.9.4's (Chromium's `getBoundingClientRect` of the anchor button).
- Text that 0.9.4 let you select (the dialog's error, the Changes window's paths and lines) isn't selectable: GPUI
  text has no selection outside text fields.
- `maxLength={40}` on the name: a paste over 40 UTF-16 units is refused whole, where Chromium keeps its first 40.
- Mono text is Menlo, as Chromium resolved `--font-mono` (see `terminal::MONO`).

## Edits in shared files (additive)

- `nodus-ui/Cargo.toml`: nodus-actions, nodus-git, nodus-voice.
- `components/icon.rs`: one line serving `actions::icons` under `icons/spec05/`.
- `lib.rs`: `actions`, `git`, `voice`.
- `shell/title_bar.rs`: the git controls where the comment said they go; the back button's 2px margin (it was a
  separate child, which added the crumbs' 4px gap twice: every crumb sat 4px right of 0.9.4's).
- `shell/root.rs`: the rail mounted over the board area, with the canvas's `run_action`.
- `board/canvas.rs`: `overlay_open()` also counts the rail's overlays (one line, as BOARD-142 asked).
- `terminal/session_window.rs`: the mic placeholder is the session mic (MODAL-019).
- `capture/render.rs`: `render_voice_status` draws the shared `voice::voice_status_of` (no golden shows a busy mic).
