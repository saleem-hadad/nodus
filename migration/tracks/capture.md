# Track: capture (P3)

- **Branch:** `rust/capture`
- **Spec:** `migration/features/07-capture.md` (CAP-001…CAP-177)
- **Crate:** `nodus-capture` (no GPUI; the panel window and its page are a later UI track)
- **Sources:** `src/main/capture.ts`, `captureGuess.ts` (+ test), `doubleTap.ts` (+ test), `screenContext.ts` (+ test), `src/shared/capture.ts` (+ test), the capture handlers in `src/main/ipc.ts`; the non-drawing logic of `CaptureApp.tsx`

## Status

| Module | State |
| --- | --- |
| `text` (shared/capture.ts) | 🟡 WIP: written, being tested |
| `double_tap` | ☐ |
| `guess` | ☐ |
| screen context | ☐ |
| targets, submit, records | ☐ |
| key hook, shortcut state | ☐ |
| panel driver, timing helpers | ☐ |
| page logic | ☐ |

## Done (implemented and tested)

- (none yet)

## Deferred

- (to be filled in)

## Traits nodus-core must implement

- (to be filled in)
