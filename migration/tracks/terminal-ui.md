# Track: terminal-ui (P7, spec 12's UI)

- **Branch:** `rust/terminal-ui`
- **Scope:** `crates/nodus-ui/src/terminal/`: the GPUI element that draws `nodus-terminal`'s model, the host that shows a
  session's terminal in the session window or an expanded card, the session window (`TerminalModal.tsx`), the
  transcript preview (`TranscriptPreview.tsx`) and the agents viewer (`AgentViewer.tsx`).
- **Spec:** `migration/features/12-terminal-ui.md`: the TERM drawing, focus and timing behaviors the terminal track
  deferred, and every MODAL, TRANS and AGV behavior.

## Status

🟡 in progress (resumed after two stalls). Landed: icons, cell metrics, custom glyphs, the element, the registry's
core adapter and ordered event pump (`store.rs`), the host (keys, IME, pointer, selection, links, wheel, scrollbar,
drops), the session window, the transcript preview, the agents viewer, the test driver's `terminal` command and the
parity scenes. Next: the expanded card's element API (with the scaled bitmap on a zoomed board), checking every scene
against the goldens in both themes, then fmt/clippy/tests/prepush.

## How to resume

Read this file, then `git log --oneline main..rust/terminal-ui`. The status line above says what's next.

Parity scenes (stand-in claude and shell from the harness's world, never the real claude):

```sh
python3 crates/nodus-ui/parity/terminal_scenes.py [scene-prefix] [--theme light|dark] --golden <dir with the goldens>
NODE_PATH=<parity-harness>/migration/parity/node_modules node crates/nodus-ui/parity/compare.mjs <dir> <scene.theme> [x y w h]
```

## Done (implemented; IDs in the code)

- **TERM (the 26 the model track deferred):** TERM-023 refit in the element's layout, at most once a frame;
  TERM-034 only file drops reach the terminal; TERM-049 consumed keys stop propagation (the board's own listener also
  skips keys while `typing`, board-canvas); TERM-055 IME composition at the cursor (white on black) and the candidate
  window at the cursor cell; TERM-056 drag, double/triple click, ⇧-click, ⌥-drag column, edge auto-scroll every 50 ms;
  TERM-064 I-beam, arrow while reporting and over the scrollbar, hand over a link; TERM-066/067 hover underline and
  open on release over the link pressed (http/https through `nodus_platform::shell::open_external`); TERM-075
  14 px overlay scrollbar with its colours and fades; TERM-076 block/outline/bar/underline cursor, 600 ms blink
  restarted on output and keys; TERM-080 box drawing, blocks, shades and powerline as shapes; TERM-090 focus on attach
  and on resolve.
- **Expanded card side:** TERM-094 not-started overlay (`overlays::not_started_overlay`), TERM-101 the same loading
  pill and error overlay as the window, TERM-100 a press in the body focuses (host), TERM-104…106 the host claims a
  gesture only through `nodus_terminal::host::WheelGestures`.
- **MODAL:** 001–012, 013 (label colours), 014 (title via `card_title`), 015 cwd with `~`, 016 branch, 017, 018 usage
  stats, 019 (mic slot; recording comes with the voice UI), 020, 021 (`nodus_core::stoppable`), 022, 023, 024 (the
  window stays; Resume replaces Stop once not live), 025–027, 032.
- **TRANS:** 001–024 (messages, collapse, calls, chips, highlights, times, ranges, free slot, Restore & Open, ⌘K back to
  search).
- **AGV:** 001–020 (nested list, default agent, 1500 ms poll, elapsed ticking, transcript refresh without flashing,
  stick to bottom within 48 px, live line).
- **Core adapter:** `store::Terminals` implements the registry's `Sessions` over `nodus.sessions()` (`open`, `write`,
  `resize`) and feeds `pty_data`/`pty_exit` from one ordered pump that holds a session's events until its open's reply
  is applied (TERM-008…011 ordering).

## In progress

- TERM-083 and the expanded card's element: zoom is mapped back in `Geometry::unscale`, but nothing sets it yet and the
  terminal isn't drawn as a scaled 1× bitmap; `CardTerminals` (the board-cards `TerminalHost` trait's shape) is next.
- Side-by-side checks of `terminal-claude`, `terminal-shell`, `terminal-stop-confirm`, `transcript-preview`,
  `agent-viewer` in both themes.

## Deferred

- TERM-093, 095–099, 102, 103: the expanded card's start/wake rules, hosting condition, mic and click handling are
  board-cards' (XCARD-019…028); this track gives it the element and the host's calls.
- TERM-108 (closing the window leaves the card expanded) and TERM-109 (board ⌘K yields to a focused terminal):
  board-canvas (BOARD-103, BOARD-157).
- MODAL-028 (archived from outside closes the window), MODAL-029 (finished run), MODAL-030 (removed view): the board's
  store (`loadBoard`, `finishRun`, `removeView` in board-canvas); MODAL-031 board shortcuts off while the window is up
  (BOARD-158) and the menu's Search Sessions… closing it (spec 11's search).

## Integration notes

- `actions.rs` keeps small copies of `openTerminal`/`openPreview`/`patchCards`/`removeCard` for the windows; board-canvas
  adds the same on `AppState` (`board/store.rs`). Once both are on main, route these to the board's, keeping
  `PreviewFindState` (the search a preview came from, TRANS-002/008/009/012), which the board's `open_preview` lacks.
