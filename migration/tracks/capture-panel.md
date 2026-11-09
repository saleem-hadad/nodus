# Track: capture-panel (P10)

- **Branch:** `rust/capture-panel`
- **Spec:** `migration/features/07-capture.md` (the panel UI CAP behaviors), spec 08 UI-131…UI-135 (Settings' capture
  rows), spec 01 PLAT-048 (File › Capture…)
- **Crates:** `nodus-ui` (`src/capture/`: the panel window and page), `nodus-core` (`src/capture.rs`: `nodus.capture()`)
- **Sources:** `src/renderer/src/capture/CaptureApp.tsx`, `capture.css`, `src/main/capture.ts`, the capture handlers in
  `src/main/ipc.ts`; backend on main in `nodus-capture` (see `migration/tracks/capture.md`)

## Status

| Piece | State |
| --- | --- |
| Track file | ✅ |
| Core: `nodus.capture()` (`CaptureHandlers`, `CaptureSessions`) | ☐ |
| Panel window (`PanelWindow`, macOS NSPanel, Windows tool window) | ☐ |
| Panel page: header, field, tiles, footer, menu, banners, recents, receipt | ☐ |
| Settings' capture rows, File › Capture… | ☐ |
| Scenes vs goldens | ☐ |

## How to resume

Read this file, then `git log --oneline main..rust/capture-panel`. The first unchecked row is what's left.

## Done (implemented and verified)

(filled in as pieces land)

## Deferred

(filled in as pieces land)

## Edits in other tracks' files

(additive lines only; listed here as they land)
