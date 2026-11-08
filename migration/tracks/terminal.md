# Track: terminal

- **Branch:** `rust/terminal` (the GPUI-free half of phase P7)
- **Spec:** `migration/features/12-terminal-ui.md`, the TERM behaviors and "Terminal emulation"
- **Crate:** `nodus-terminal` (L1, no GPUI)
- **Sources:** `src/renderer/src/terminal/registry.ts`, `useAttachedTerminal.ts`, xterm.js 6.0.0 (`Keyboard.ts`, `CoreBrowserTerminal.ts`, `InputHandler.ts`, `Clipboard.ts`, `SelectionService.ts`, the WebGL addon's colour rules), `@xterm/addon-unicode11` 0.9.0, `@xterm/addon-web-links` 0.12.0

## Status

In progress (interrupted once before anything was committed; restarted from `main` at cb199b8). Done so far: the terminal model, keys, paste, selection, snapshot, registry, and the xterm parity fixtures. Next: links, fit/wheel/mouse math, the attach phase, this file's final lists.

## How it's checked

`crates/nodus-terminal/tests/fixtures/generate.mjs` bundles xterm.js 6's own TypeScript (and the real registry.ts with a stand-in terminal and session API) with esbuild and writes the fixtures beside it; `tests/xterm_parity.rs` and `src/registry/tests.rs` replay them. Re-run after changing the TypeScript or upgrading xterm (Node 22, after `pnpm install`):

```sh
node crates/nodus-terminal/tests/fixtures/generate.mjs
```

- `keys.json`: 3,083 key presses (every named key, letters, digits, punctuation, numpad, Russian and Arabic letters, ⌥ compositions, dead keys, IME, AltGr; 16 modifier combinations; Mac and Windows; application cursor mode; with and without a selection) through registry.ts's real custom key handler and xterm's `evaluateKeyboardEvent`.
- `replies.json`: DA1/DA2/DA3, DSR/CPR (incl. a pending wrap), DECXCPR, DECRQM (ANSI and private, before and after setting), OSC 4/10/11/12 queries and changes, window ops, kitty, XTVERSION, modifyOtherKeys, OSC 52, DECRQSS, XTGETTCAP, queries split across writes.
- `screens.json`: Claude-Code-like output, wide characters at the edge, emoji old and new, combining marks, erasing, scroll regions, alternate screens, soft and full reset, OSC 8: every cell's characters and width, wrap flags, the cursor and the history length.
- `widths.json`: the Unicode 11 width of every code point.
- `registry.json`: 18 scenarios, 273 calls.

## Done (implemented and tested)

(in progress; see the commits)

## Deferred

(in progress)

## API the UI's terminal element uses

(in progress)
