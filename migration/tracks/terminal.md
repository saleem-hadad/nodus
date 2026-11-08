# Track: terminal

- **Branch:** `rust/terminal` (the GPUI-free half of phase P7)
- **Spec:** `migration/features/12-terminal-ui.md`, the TERM behaviors and "Terminal emulation"
- **Crate:** `nodus-terminal` (L1, no GPUI; depends on `alacritty_terminal =0.26.0`, `nodus-domain`, `regex`, `tokio`, `unicode-width`, `url`)
- **Sources:** `src/renderer/src/terminal/registry.ts`, `useAttachedTerminal.ts`, `ExpandedCard.tsx` (wheel gestures), xterm.js 6.0.0 (`Keyboard.ts`, `CoreBrowserTerminal.ts`, `InputHandler.ts`, `Clipboard.ts`, `SelectionService.ts`, `SelectionModel.ts`, `CoreMouseService.ts`, `MoveToCell.ts`, `Mouse.ts`, `Viewport.ts`, `OscLinkProvider.ts`, the WebGL addon's colour rules), `@xterm/addon-unicode11` 0.9.0, `@xterm/addon-web-links` 0.12.0, `@xterm/addon-fit` 0.11.0

## Status

Done for the GPUI-free half: 83 TERM behaviors implemented and tested, 26 deferred to the UI track (all drawing, focus, timing or event routing; the model side of each is in place and named below). The first worktree was lost to an interruption before anything was committed; the work restarted from `main` at cb199b8.

## How it's checked

`crates/nodus-terminal/tests/fixtures/generate.mjs` bundles xterm.js 6's own TypeScript (and the real registry.ts with a stand-in terminal and session API) with esbuild and writes the fixtures beside it; `tests/xterm_parity.rs` and `src/registry/tests.rs` replay them. Re-run after changing the TypeScript or upgrading xterm (Node 22, after `pnpm install`):

```sh
node crates/nodus-terminal/tests/fixtures/generate.mjs
```

- `keys.json`: 3,083 key presses (every named key, letters, digits, punctuation, numpad, Russian and Arabic letters, ⌥ compositions, dead keys, IME, AltGr; all 16 modifier combinations; Mac and Windows; application cursor mode; with and without a selection) through registry.ts's real custom key handler and xterm's `evaluateKeyboardEvent`, following `CoreBrowserTerminal._keyDown`. The key code each press gets is also checked against Chromium's.
- `replies.json`: DA1/DA2/DA3, DSR/CPR (incl. a pending wrap), DECXCPR, DECRQM (ANSI and private, before and after setting), OSC 4/10/11/12 queries and changes, window ops, kitty, XTVERSION, modifyOtherKeys, OSC 52, DECRQSS, XTGETTCAP, queries split across writes.
- `screens.json`: 23 outputs through xterm's parser and buffer with Unicode 11 active (Claude-Code-like UI with synchronized output, wide characters at the edge, emoji old and new, combining marks, erasing, scroll regions, alternate screens, soft and full reset, OSC 8, DECALN, REP): every cell's characters and width, wrap flags, the cursor and the history length.
- `widths.json`: the Unicode 11 width of every code point.
- `links.json`: the web-links addon and xterm's OSC 8 provider on 22 lines, every row.
- `registry.json`: 18 scenarios, 273 calls, in order.

Everything else has unit tests named after its TERM ID.

## Unicode widths (TERM-081)

alacritty_terminal measures characters with `unicode-width` (Unicode 16); 0.9.4 used xterm's Unicode 11 table. `unicode11.rs` holds the addon's tables (generated from `UnicodeV11.ts`, checked over all 1.1 M code points against the addon). `XtermHandler::input` writes each character at xterm's width: when that differs from alacritty's, it writes a stand-in of the right width (U+E000 or U+4E00) through `Term::input`, so wrapping, insert mode and wide-character spacers all follow, then puts the real character in the cell; a character xterm counts as zero-width is joined to the previous cell as alacritty does its own. Measured differences: 171 ranges, of which 44 newer emoji are 1 cell in xterm and 2 in alacritty (🥲 U+1F972, 🫠…), 112 newer combining marks are 1 cell in xterm and 0 in alacritty, and a few go the other way (🤻 U+1F93B is 2 in xterm, U+0600–0604 0). Claude Code's own symbols (⏺ ✻ ⎿ ╭─╮) are one cell in both.

## Done (implemented and tested)

- TERM-001 to TERM-022 registry and lifecycle (`registry::Registry`): one terminal per session, one container at a time, detach keeps everything and only from its own container, blur on detach, parked terminals never fitted and still parsing, output dropped while opening and after exit, exit while opening written after the replay, exit lines per kind, open at the fitted size, disposed-during-open, open failure resumable, shared open, reset on reattach, ready reattach resizes. Replayed against registry.ts.
- TERM-024 to TERM-033, TERM-035 resize only when ready, input dropped only once exited (type-ahead while opening goes through), `needs_open`/`has_output`, dispose, `type_in` (trim + space, queue, `starting`, bracketed paste or 30 s on `tokio::time`, same terminal only), dropped paths quoted (`paste::shell_quote`, `paste::dropped_paths`), paste (`paste::prepare`, empty brackets kept).
- TERM-036 to TERM-048, TERM-050 to TERM-053 keys (`keys`, `Terminal::key_down`): ⇧Enter per kind, ⌘ keys on a Mac, ⌘K clear, ⌘C/⌘A, Windows Ctrl+C/V/Shift+A/W/T and other Ctrl+Shift letters, non-Latin layouts by key code, AltGr, Esc, the whole key table, ⌥ composition on a Mac, Alt as ESC prefix elsewhere, ⇧PgUp/⇧PgDn. Checked against the 3,083 fixtures.
- TERM-054, TERM-057, TERM-059, TERM-060, TERM-061 input returns to the bottom and clears the selection; selection kept through erases, cleared by a row resize; copied text as xterm builds it (a written space is kept, an erased cell isn't: written cells carry bit 15 of alacritty's cell flags); selecting never copies; Mac right-click selects a word unless inside the selection (`Terminal::right_click`).
- TERM-058, TERM-079, TERM-084 colours (`palette`): the fixed theme in both app themes, the 256 palette, OSC 4/10/11/12 changes and restores, bold-as-bright, dim, inverse, the WebGL renderer's selection blending; resolved per cell in `snapshot`.
- TERM-062 ⌥/Alt-click arrow sequences (`Terminal::move_to_cell_sequence`, a port of `MoveToCell.ts`; nothing while scrolled up). The UI applies the 500 ms and one-character conditions.
- TERM-063, TERM-069 to TERM-072 mouse reports and wheel (`pointer`): protocols 9/1000/1002/1003, SGR and SGR-pixel encodings, move debouncing, Shift forces selection off a Mac only; the default encoding is never sent because 0.9.4's registry ignores xterm's `onBinary`. Wheel: 50 px a line in the normal screen (1.25 × pixel deltas, ×5 with Alt, rounded to rows), one arrow per turn in the alternate screen with xterm's line accumulation, reports when asked, prevented wheels ignored.
- TERM-065, TERM-068 links (`links`): web-links regex with JavaScript's `\s`, `new URL` check via the WHATWG `url` crate, wrapped lines, xterm's range quirks; OSC 8 runs (http/https), ahead of web links; the confirm text.
- TERM-073, TERM-074 the view follows output at the bottom and holds still when scrolled up; 10,000 lines of history.
- TERM-077, TERM-078, TERM-082 focus reports; every query answered as xterm answers it (DECXCPR, DECRQSS and DECSTR, which alacritty's parser drops, come from a side scanner in `queries.rs` that splits the input where they end); synchronized output parsed at once with drawing held (`Snapshot::hold`).
- TERM-081 Unicode 11 widths (above).
- TERM-085, TERM-091, TERM-092 focus does nothing while parked; reattach resumes; detach only from its own container.
- TERM-086 to TERM-089 attach phase (`host::AttachPhase`).
- TERM-104 to TERM-107 wheel gestures (`host::WheelGestures`): a gesture belongs to whoever its first event lands on, 250 ms gap, Ctrl+wheel to the board, non-hosting bodies pan.

## Deferred

All to the UI track (nodus-ui), with what the element needs from this crate:

- TERM-023 refit at most once per frame on container resize: call `Registry::refit(id, container, fit::fit(w, h, &metrics))` from the element's layout.
- TERM-034 drops without files: the element ignores them; files go to `Registry::drop_paths`.
- TERM-049 consumed keys don't reach board shortcuts: stop propagation unless `key_down` returns `Propagate` or `Unhandled`.
- TERM-055 IME composition drawn at the cursor (`palette::theme::COMPOSITION_*`), candidate window at the cursor cell; commits go to `Registry::input_text`, `Composing` means the IME has the key.
- TERM-056 drag selection, edge auto-scroll (50 ms, 15 lines), double/triple-click detection: call `select_range` / `select_word_at` / `select_line_at` / `extend_selection` with `pointer::pointer_cell(…, selection: true)`.
- TERM-064 pointer shapes: I-beam, arrow while `mouse_events_active()`, hand over `link_at`.
- TERM-066, TERM-067 hover underline and hand, open on release over the link pressed: `link_at`, then `links::browser_url` → `nodus_platform` open; OSC 8 shows `links::osc8_confirm_text` and then opens nothing (0.9.4 per code reading; confirm in the running app, see plan "Quirks").
- TERM-075 scrollbar (14 px overlay, fades, `theme::SCROLLBAR_*`; `display_offset`, `history_size`).
- TERM-076 cursor drawing and the 600 ms blink timer (`Snapshot::cursor` gives shape, focus, blink, colours).
- TERM-080 custom box-drawing, block and powerline glyphs.
- TERM-083 mapping a pointer on a zoomed board back to unscaled pixels before `pointer_cell` / `report_position`.
- TERM-090 focusing on attach and on resolve.
- TERM-093 to TERM-103 the expanded card's hosting, overlays, wake and focus rules.
- TERM-108, TERM-109 the window/card handover and ⌘K on the board.
- MODAL-*, TRANS-*, AGV-* are the UI track's views, not this crate.

## API the UI's terminal element uses

- `Registry::new(Arc<dyn Sessions>, Platform::current(), tokio_handle)`. nodus-core implements `Sessions` (`open` → replay or error message, `write`, `resize`) and feeds `pty_data(id, data)` / `pty_exit(id)` on the same ordered stream as the open's reply.
- Hosting: `attach(id, project, ContainerId, kind, Fit)` (async; `Err(message)` for the error overlay), `detach(id, Some(container))`, `refit`, `dispose`, `focus`/`blur`, `needs_open`, `has_output`, `on_output`/`off_output`, `type_in`, with `host::AttachPhase` for the loading/error overlays.
- Input: `key_down(id, &KeyEvent) -> KeyAction` (writes `Send`/`WriteDirect` itself; the UI acts on `Copy`, `Paste`, `Menu`, `Propagate`, `Unhandled`, `Composing`), `input_text`, `paste`, `drop_paths`, `input` (mouse reports and wheel arrows from `Terminal::mouse_report` / `Terminal::wheel` inside `with_terminal`).
- `KeyEvent::from_parts(dom_key, dom_code, Modifiers)`: the UI maps GPUI keystrokes to DOM `key`/`code` names (`"Enter"`, `"ArrowLeft"`, `"a"`, `"Dead"`, `"Process"` for IME), key codes are derived as Chromium's.
- Drawing: `with_terminal(id, |t| t.snapshot())` → `Snapshot` (cells with text, width, fg/bg, dim, bold, italic, underline style and colour, strikethrough, invisible, selected, OSC 8 URI; cursor; background; scroll position; `hold` for mode 2026, whose 1000 ms limit the UI enforces with `end_synchronized_output`); `generation()` changes whenever a redraw is due.
- Geometry: `fit::cell_metrics(advance, ascent + descent, dpr)`, `fit::fit(width, height, &metrics)`, `pointer::pointer_cell`, `pointer::report_position`.

## Known differences from xterm.js (all rare; none seen in Claude Code's output)

- Resizing columns only clears the selection (alacritty); xterm keeps it. Reflow on resize is alacritty's.
- A selection inside a scroll region moves with the region's scrolling; xterm's stays on absolute lines.
- 47/1047 overwrite the main screen's saved cursor (alacritty's `swap_alt`).
- SGR 53 (overline) and DECSCA are dropped by alacritty's parser: no overline is drawn, DECRQSS `"q` always says 0.
- C1 controls sent as UTF-8 are dropped; xterm executes them.
- Parsing is synchronous, so a query answered in the same chunk as the process's exit is still written (xterm parses later and drops it).
- Windows wheel speed: Chromium's legacy `wheelDeltaY` may make 0.9.4 scroll 0.5 × the pixels on Windows rather than 1.25 ×; check against 0.9.4 on Windows.
