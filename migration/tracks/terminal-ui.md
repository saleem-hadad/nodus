# Track: terminal-ui (P7, spec 12's UI)

- **Branch:** `rust/terminal-ui`
- **Scope:** `crates/nodus-ui/src/terminal/`: the GPUI element that draws `nodus-terminal`'s model, the host that shows a
  session's terminal in the session window or an expanded card, the session window (`TerminalModal.tsx`), the
  transcript preview (`TranscriptPreview.tsx`) and the agents viewer (`AgentViewer.tsx`).
- **Spec:** `migration/features/12-terminal-ui.md`: the TERM drawing, focus and timing behaviors the terminal track
  deferred, and every MODAL, TRANS and AGV behavior.

## Status

✅ ready to merge (after two stalls). Done: TERM 17 of the 26 the model track left (plus TERM-104…107's UI side),
MODAL 28 of 32, TRANS 24 of 24, AGV 20 of 20. Deferred: TERM-093, 095–099, 102, 103, 108 (the expanded card's own rules,
board-cards and board-canvas) and MODAL-028…031 (the board's store and search). Every terminal scene is within
0.04–0.6 % of its golden in both themes (text rasterisation only); see "Checked against the goldens".

## How to resume

Read this file, then `git log --oneline main..rust/terminal-ui`.

Parity scenes (the harness's world with its stand-in claude and shell, never the real claude; the app and the session
host it starts are stopped after each shot):

```sh
cargo build -p nodus
python3 crates/nodus-ui/parity/terminal_scenes.py [scene-prefix …] [--theme light|dark] \
  --golden <a folder holding the goldens> --world <scratch folder>
```

Shots land beside the goldens as `<scene>.<theme>.rust.png`. Compare the scene's own region (the board behind the
windows is board-canvas's): `NODE_PATH=<parity-harness>/migration/parity/node_modules node
crates/nodus-ui/parity/compare.mjs <folder> <scene.theme> x y w h`.

## Done (implemented; IDs in the code)

- **TERM (the model track's 26):** TERM-023 refit in the element's layout, at most once a frame; TERM-034 only file
  drops reach the terminal; TERM-049 consumed keys stop propagation, and Tab, ⇧Tab and the copy key, which the window's
  root binds, are bound to nothing in the `Terminal` context so they reach it (`store::terminal_key_bindings`); TERM-055
  IME composition at the cursor (white on black) and the candidate window at the cursor cell; TERM-056 drag,
  double/triple click, ⇧-click, ⌥-drag column, edge auto-scroll every 50 ms; TERM-064 I-beam, arrow while reporting and
  over the scrollbar, hand over a link; TERM-066/067 hover underline and open on release over the link pressed
  (http/https through `nodus_platform::shell::open_external`); TERM-075 14 px overlay scrollbar, its colours and fades;
  TERM-076 block/outline/bar/underline cursor, 600 ms blink restarted on output and keys; TERM-080 box drawing, blocks,
  shades and powerline as shapes; TERM-083 a card's terminal is laid out and fitted at 1× and drawn into a layer scaled
  by the board's zoom (soft when zoomed in, as 0.9.4's canvas was), with the pointer mapped back to unscaled cells;
  TERM-090 focus on attach and on resolve; TERM-094 not-started overlay; TERM-100 a press in the body focuses;
  TERM-101 the window's loading pill and error overlay; TERM-109 the terminal consumes ⌘K / Ctrl+K before the board's
  key listener sees it.
- **MODAL:** 001–027 except 019's recording (the mic button is drawn and `SessionWindow::type_in` does the rest: typed in,
  resumed, focused; the recorder is the voice UI's), 032. 013 label colours, 014 `card_title`, 015 cwd with `~`, 016
  branch, 018 usage stats, 021 `nodus_core::stoppable`, 024 the window stays and Resume replaces Stop.
- **TRANS:** 001–024.
- **AGV:** 001–020.
- **Core adapter:** `store::Terminals` implements the registry's `Sessions` over `nodus.sessions()` (`open`, `write`,
  `resize`) and feeds `pty_data`/`pty_exit` from one ordered pump that holds a session's events until its open's reply
  is applied (TERM-008…011's ordering).

## Deferred

- TERM-093, 095–099, 102, 103: the expanded card's start/wake rules, hosting condition, mic and click handling are
  board-cards' (XCARD-019…028); `CardTerminals` gives it the element and the calls those rules make.
- TERM-108: the card shows the terminal again once the window lets go of it (the host's lifecycle does that); keeping
  the card expanded is board-canvas's (BOARD-103).
- MODAL-028 (archived from outside closes the window), MODAL-029 (a finished run), MODAL-030 (a removed view): the
  board's store (`fetch_board`, `finishRun`, `removeView` on board-canvas). MODAL-031: the board's key listener
  already skips keys while the window is up (BOARD-158); the menu's Search Sessions… closing it is spec 11's search.

## The element for expanded cards

`terminal::card::CardTerminals` has the shape of board-cards' `TerminalHost` trait (`board/cards/mod.rs`); the module
doc has the forwarding impl to paste once both are on main. Build it per frame in `BoardEnv`:

```rust
CardTerminals::new(project_id, zoom, cx)                     // cheap; the board's zoom
    .with_wheel_gate(Rc::new(|id, ctrl, cx| cx.default_global::<Wheel>().0.wheel(Instant::now(), Some(id), ctrl)))
```

- `needs_open(id)`, `set_enabled(id, shell, enabled, cx)`, `phase(id, cx) -> Option<AttachPhase>` (`None` before the
  terminal rendered: `Phase::Idle`), `reattach(id, cx)`, `focus(id, window, cx)` (now, or as soon as it's attached),
  `type_in(id, text, starting, cx)`, `element(id, window, cx) -> Option<AnyElement>`.
- `element` fills the card body's content box (the body keeps its own padding and background); its host lives while the
  card renders it each frame and parks the terminal the frame it doesn't.
- The wheel gate lets the card's own arbiter decide who owns a gesture; without one the host uses
  `nodus_terminal::host::WheelGestures` and the board calls `host::board_wheel` for its own wheel events.

## Checked against the goldens

`terminal_scenes.py`, cropped to each window (the board behind them isn't drawn on this branch), pixelmatch at 0.1:

| Scene | Light | Dark |
| --- | --- | --- |
| terminal-claude (window) | 0.13 % | 0.14 % |
| terminal-shell (window) | 0.18 % | 0.21 % |
| terminal-stop-confirm (window / dialog) | 0.04 % / 0.60 % | 0.04 % / 0.60 % |
| transcript-preview (panel) | 0.15 % | 0.18 % |
| agent-viewer (panel) | 0.34 % | 0.40 % |
| board-expanded (stand-in card bodies at 72 %: Claude / shell) | 1.80 % / 1.28 % | 1.80 % / 1.28 % |

What's left is glyph rasterisation, and in `board-expanded` the scaled picture's resampling filter (same rows, columns
and positions). Two visible differences remain, neither this track's to fix: the agents list wraps "Check how other
lists show loading states" a word earlier (GPUI counts the space after a word when wrapping; gpui-text), and the
shell's folder reads `<scratch>/…` instead of `~/nodus-parity/…` only because the world wasn't under `/Users`.
`board-expanded` uses the driver's stand-in card bodies (`{"cmd":"terminal","action":"card",…}`) until board-cards hosts
terminals; `board-expanded-stopped` needs the cards' chrome and wasn't compared.

## Integration notes

- `actions.rs` keeps small copies of `openTerminal`/`openPreview`/`patchCards`/`removeCard` for the windows; board-canvas
  adds the same on `AppState` (`board/store.rs`). Once both are on main, route these to the board's, keeping
  `PreviewFindState` (the search a preview came from, TRANS-002/008/009/012), which the board's `open_preview` lacks.
- Merging main: `app.rs` conflicts on one line (this branch sets `terminal::store::CoreRuntime` beside
  `window::Core`; main added `Flags` and `core_shell::serve` there): keep all three. board-canvas adds its own
  `board::CoreRuntime` global for the same handle; keep one.
- board-cards' `present::WheelArbiter` duplicates `nodus_terminal::host::WheelGestures`; with the wheel gate above only
  one of them decides.
- GPUI matches key bindings before key listeners: a binding the board adds for a plain key would reach a focused
  terminal first; the board listens for keys instead (board-canvas does), or binds `NoAction` for it in `Terminal`.
