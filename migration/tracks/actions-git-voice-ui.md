# Track: actions-git-voice-ui (P9, spec 05's UI)

- **Branch:** `rust/actions-git-voice-ui`
- **Spec:** `migration/features/05-actions-voice-git.md`: the UI behaviors the actions-voice-git track left to the
  window (GIT-021…GIT-047, ACT-017/018/031/032/039/045…082, VOICE-033, VOICE-038…045).
- **Sources:** `components/actions/*`, `components/git/*`, `components/voice/*`, `TitleBar.tsx`, the store's git and
  actions parts, `BoardView.tsx`'s `runAction` and `runFinished`.
- **Code:** `crates/nodus-ui/src/git/` (branch chip, Changes button, Changes window), `crates/nodus-ui/src/actions/`
  (the rail, its fan, menu and confirm, the dialog, the icons), `crates/nodus-ui/src/voice/` (the session mic, the
  recording look and notices).

## Status

| Piece | State |
| --- | --- |
| Icons (lucide 1.49.0, `actions::icons`) | ✅ |
| Git: `GitModel` (refresh, polling, reset), branch chip + log popover, Changes button | ✅ compared (before the rebase on the board) |
| Git: the Changes window | ✅ compared (before the rebase on the board) |
| Voice: `Mic`, `voice_status`, `voice_notice`, `session_mic` | ✅ written; ☐ in the session window |
| Actions: rail, hint, fan, menu, delete confirm | 🟡 written; ☐ on the board canvas's hooks, ☐ compared |
| Actions: dialog | ☐ (stub) |

## How to resume

Read this file, then `git log --oneline main..rust/actions-git-voice-ui`. The first ☐ above is next.

## Scenes

`python3 crates/nodus-ui/parity/scenes05.py [prefix…] [--theme light|dark] --out <dir> --world <dir>` renders this
track's scenes (`git-log`, `git-diff`, `actions-*`) beside the goldens, from the harness's seed with the stand-in
`claude`.

## In the code (by ID)

- **Git:** GIT-021…GIT-047 (`git/mod.rs`: `GitModel`, `title_bar_git`; `git/diff_modal.rs`).
- **Actions:** ACT-017, ACT-018 (`state.rs` refresh, `actions::set_actions`), ACT-031, ACT-039, ACT-046…ACT-066,
  ACT-081, ACT-082 (`actions/mod.rs`); ACT-067, ACT-073, ACT-080 only as the dialog's stub.
- **Voice:** VOICE-025…VOICE-028, VOICE-030…VOICE-033, VOICE-035 (`voice::Mic` over `nodus_voice::controller`),
  VOICE-038…VOICE-045 (`voice_status`, `voice_notice`, `session_mic`).

## Done (implemented; IDs in the code; checked against the goldens)

(filled in as pieces are compared)

## Deferred

(filled in as pieces land)

## Edits in shared files (additive)

- `nodus-ui/Cargo.toml`: nodus-actions, nodus-git, nodus-voice.
- `components/icon.rs`: one line serving `actions::icons` under `icons/spec05/`.
- `lib.rs`: `actions`, `git`, `voice`.
- `shell/title_bar.rs`: the git controls where the comment said they go; the back button's 2px margin (it was a
  separate child, which added the crumbs' 4px gap twice: every crumb sat 4px right of 0.9.4's).
- `shell/root.rs`: the rail mounted over the board area.
- `nodus-core/src/nodus.rs`: `Nodus::runtime()` (to be replaced by the board's `CoreRuntime` global).
