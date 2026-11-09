# Track: terminal-ui (P7, spec 12's UI)

- **Branch:** `rust/terminal-ui`
- **Scope:** `crates/nodus-ui/src/terminal/`: the GPUI element that draws `nodus-terminal`'s model, the host that shows a
  session's terminal in the session window or an expanded card, the session window (`TerminalModal.tsx`), the
  transcript preview (`TranscriptPreview.tsx`) and the agents viewer (`AgentViewer.tsx`).
- **Spec:** `migration/features/12-terminal-ui.md`: the TERM drawing, focus and timing behaviors the terminal track
  deferred, and every MODAL, TRANS and AGV behavior.

## Status

🟡 in progress. Pieces land in this order: icons and module skeleton, cell metrics and custom glyphs, the element, the
registry's core adapter and ordered event pump, the host (input, selection, wheel, links, IME), the session window, the
transcript preview, the agents viewer, the expanded card's slot, parity scenes.

## How to resume

Read this file, then `git log --oneline main..rust/terminal-ui`. The status line above says what's next.

## Done (implemented; IDs in the code)

(filled in as pieces are checked)

## Deferred

(filled in as pieces land)
