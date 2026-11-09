# Track: board-cards (P6, spec 10)

- **Branch:** `rust/board-cards`
- **Scope:** `crates/nodus-ui/src/board/cards/`: the session card, the expanded card's chrome, the agent deck and fan, the archive frame and stack looks, sticky notes, stickers and the note pad (CARD, XCARD, DECK, ARCH, NOTEUI).
- **Spec:** `migration/features/10-board-cards-notes.md` (203 behaviors).

## Status

Every component is drawn natively and plugs into board-canvas's node interface (`board::nodes::NodeRenderers`, installed by `cards::install` from `board::init`). The pure logic of the spec (previews, times, the deck's state machine, the fan's pick and side, wakes, wheel gestures, archive geometry, notes) lives in `present.rs` with its tests. Side-by-side checks against the goldens are under way; until a behavior's look has been checked it's listed as in progress.

## Files

| File | What |
| --- | --- |
| `mod.rs` | `BoardEnv` (what the board knows), `CardActions` (what the cards ask of it), `TerminalHost` (the terminal track's slot), shadows as CSS layers (`Shadow`, `Shade`) |
| `install.rs` | `CardRenderers` (the canvas's `NodeRenderers`), `CanvasActions` (`CardActions` on the canvas's `BoardActions`) |
| `present.rs` | pure presentation logic with tests |
| `session_card.rs` | the node: deck, card, stickers, fan; the compact card |
| `expanded_card.rs` | the expanded card: title bar, usage, tools, body, overlays, wakes, wheel arbitration |
| `agent_deck.rs` | deck layers, `DeckHandles`, the fan |
| `archive_zone.rs` | the frame, header, placeholders, search, divider, scrollbar |
| `note.rs`, `sticker.rs`, `note_pad.rs`, `paper.rs` | notes and their editor, stickers, the pad |
| `status_dot.rs`, `icons.rs`, `para.rs`, `anim.rs`, `resize.rs` | dots, lucide icons, Chromium-like paragraphs, keyframe helpers, resize zones |
| `examples/board_cards.rs` | gallery: the parity scenes' nodes where the goldens have them |

## Done

(none checked yet)

## In progress (implemented, IDs in the code; look not yet checked against the goldens)

- CARD-001…CARD-048 (CARD-043 see Deferred), CARD-049 (the title's press → `press_title`; the 240ms wait and double-click are the canvas's)
- XCARD-001…XCARD-032 (XCARD-010 see Deferred)
- DECK-001…DECK-028
- ARCH-001…ARCH-005, ARCH-007…ARCH-011, ARCH-013, ARCH-015…ARCH-017, ARCH-019…ARCH-028, ARCH-032…ARCH-036
- NOTEUI-001…NOTEUI-026, NOTEUI-031…NOTEUI-044

## In the canvas's code (board-canvas, spec 09's IDs)

CARD-050…CARD-053 (edges, glides), ARCH-006, ARCH-012, ARCH-014's placement, ARCH-018, ARCH-029…ARCH-031, ARCH-037…ARCH-040, NOTEUI-027…NOTEUI-030, NOTEUI-045…NOTEUI-049.

## Deferred

- CARD-043, DECK-029 and the other accessible names: GPUI has no accessibility tree yet; the strings are kept (`present::aria_label`, tested).
- XCARD-010: the mic is the actions-git-voice-ui track's; the card has a `TerminalHost::mic` slot and draws the idle button until then.
- XCARD-018…XCARD-028's terminal: the body has a `TerminalHost::element` slot for the terminal-ui track's element; `install.rs` passes no host yet, so the card shows the not-started overlay.
