# Track: board-cards (P6, spec 10)

- **Branch:** `rust/board-cards`
- **Scope:** `crates/nodus-ui/src/board/cards/`: the session card, the expanded card's chrome, the agent deck and fan, the archive frame and stack looks, sticky notes, stickers and the note pad (CARD, XCARD, DECK, ARCH, NOTEUI).
- **Spec:** `migration/features/10-board-cards-notes.md` (203 behaviors).

## Status

Every component is drawn natively and plugged into the board. All 203 behaviors are done (180 here, 23 by the canvas under spec 09's IDs). Checked side by side with the goldens in both themes, through the real app (`scenes.py`) and the cards gallery (`examples/board_cards.rs`).

## How it plugs into the canvas

- `cards::install` (from `board::init`) registers `CardRenderers` as the canvas's `board::nodes::NodeRenderers`: `card` (compact or expanded, with its deck, fan and stickers), `note` (free notes and the pad's top sheet), `archive`, `pad`, and the hooks `wake` (`wakeCardTerminal`, XCARD-025) and `focus_terminal` (XCARD-017, BOARD-042).
- Each node gets a `BoardEnv` built from `BoardCx` (renaming, archive hot, the stack, zoom, the search jump's `flash`) and the app's state, read through the main window's shell (`shell::window::main_shell`): never through the canvas, which is being updated while it draws its nodes.
- `CanvasActions` maps the cards' `CardActions` onto the canvas's `BoardActions` and the app state; notes saved after the board changed go to their own project on the core's runtime (NOTEUI-015).
- The expanded card's body hosts terminal-ui's `terminal::card::CardTerminals` through the `TerminalHost` trait (`install.rs`); the card's start, wake and focus rules decide when it attaches (TERM-093, 095–099, 103, 108). One wheel arbiter decides gestures (`present::WheelArbiter`, given to the terminals as their wheel gate): the window marks where a gesture begins, as 0.9.4's capture listener did.
- Content is drawn at the board's zoom (`Z`), text laid out at its CSS size and drawn scaled (`para.rs`) so lines break where Chromium's do.

## Files

| File | What |
| --- | --- |
| `mod.rs` | `BoardEnv`, `CardActions`, `TerminalHost`; shadows as CSS layers (`Shadow`: transitions, `soft` for GPUI, `rings` drawn exactly), `outer_shadow` (outside the box only) |
| `install.rs` | `CardRenderers`, `CanvasActions`, the `TerminalHost` impl for `CardTerminals` |
| `present.rs` | pure presentation logic with tests: previews, times, titles, the deck's state machine, the fan's pick and side, wakes, the wheel arbiter, archive geometry, notes, signatures, resizing |
| `session_card.rs` | the node (deck, card, stickers, fan), the compact card, the rename field, the flash |
| `expanded_card.rs` | title bar, usage, tools, body with the terminal, overlays, wakes, resize zones, grip |
| `agent_deck.rs` | deck layers, `DeckHandles`, the fan and its deal-in |
| `archive_zone.rs` | the frame, header, placeholders, search, divider, scrollbar |
| `border.rs` | rounded borders and shadow rings as Chromium draws them (exact width; Blink's dashes) |
| `note.rs`, `sticker.rs`, `note_pad.rs`, `paper.rs` | notes and their editor, stickers, the pad |
| `status_dot.rs`, `icons.rs`, `para.rs`, `anim.rs`, `resize.rs` | dots, lucide icons, Chromium-like paragraphs, keyframes with delays and fill modes, resize zones |
| `examples/board_cards.rs` | the gallery: the parity scenes' nodes where the goldens have them, without the app |

## Done (implemented; IDs in the code)

Checked against the goldens where a golden shows the behavior (looks, layout, copy, both themes); the rest (interactions, timings, states no golden captures) against the TypeScript and `present.rs`'s tests. Timings: CSS transitions and keyframes keep their durations, delays, fill modes and easings, `ease-stack`'s 29 stops and `ease-spring` overshooting included (the fan's deal, the sticker's landing, the stack, the search field, the pad's peel).

- **CARD** (53): CARD-001…CARD-048; CARD-049 (the title's press is reported; the 240ms wait and double-click rename are the canvas's, BOARD-043/047); CARD-050…CARD-053 by the canvas (`edges.rs`, `render.rs`: facing-side edges, the dashes' march, the 420ms glides).
- **XCARD** (32): XCARD-001…XCARD-032 (the terminal through `CardTerminals`; the mic is the voice UI's `voice::card_mic`, typing in and waking the card; XCARD-017's click is the canvas's, calling `focus_terminal`).
- **DECK** (29): DECK-001…DECK-029 (DECK-014's z 1400 through `BoardActions::raise`).
- **ARCH** (40): ARCH-001…ARCH-005, ARCH-007…ARCH-011, ARCH-013…ARCH-017, ARCH-019…ARCH-028, ARCH-032…ARCH-036 here; ARCH-006, ARCH-012, ARCH-018, ARCH-029…ARCH-031, ARCH-037…ARCH-040 by the canvas (`scene.rs`: z rules, pile order, results, clipping, glides, closing, travel, drops, count).
- **NOTEUI** (49): NOTEUI-001…NOTEUI-026, NOTEUI-031…NOTEUI-044 here; NOTEUI-027…NOTEUI-030 and NOTEUI-045…NOTEUI-049 by the canvas (note z, glides, bounces, delete, the pad's drop, click and menu, travel with the archive).

Also done for spec 12 here: TERM-093, TERM-095…TERM-099, TERM-102, TERM-103, TERM-108 (the expanded card's start, wake, hosting, mic and click rules); and for spec 09 the card's side of BOARD-036 (the flash), BOARD-042 and BOARD-044 (`focus_terminal`, `wake`).

## Deferred

(none)

## Checked side by side

Real app, `python3 crates/nodus-ui/parity/scenes.py <scene> --golden <dir> --world <scratch>` (put Node 22+ first on `PATH`: the stand-in `claude` needs it), compared with `compare.mjs`:

| Scene | Light | Dark |
| --- | --- | --- |
| board-populated | 0.72 % | 0.77 % |
| board-card-hover | 0.80 % | 0.85 % |
| board-selected (the selection bar is spec 11's) | 1.98 % | 1.30 % |
| board-deck-fanned | 0.92 % | 0.95 % |
| board-archive-hover | 0.82 % | 0.88 % |
| board-archive-open (30px blur, saturate 1.8) | 0.75 % | 0.80 % |
| board-note-selected | 0.95 % | 0.84 % |
| board-note-editing | 0.98 % | 0.87 % |
| board-pad-hover (the peel; GPUI draws the tooltip) | 1.11 % | 1.14 % |
| board-zoomed-in / board-zoomed-out | 1.09 % / 0.67 % | 1.25 % / 0.72 % |
| board-expanded (live terminals, usage, the mic) | 0.67 % | 0.71 % |
| board-expanded-stopped | 0.35 % | 0.39 % |

What's left in these is text rasterisation, the parts of the board other tracks draw (prompt bar, ink, selection bar, minimap), and the terminal card's folder, which reads `<scratch>/…` rather than `~/nodus-parity/…` because the world wasn't under `/Users`.

Gallery, `cargo run -p nodus-ui --example board_cards -- <dir>` (nodes only: no title bar, prompt bar, sections, edges or ink, so a whole frame differs by 1–2.5 % from those alone): every card, deck, fan, archive (folded, hovered, open), note and pad scene, cropped region by region. The open panel's blurred backdrop and the dashes' period (7.19 device px at 80 %) now match to the pixel value.

## Known differences

- Text rasterisation and placement (gpui-text's): SF Pro's tracking, and baselines that round half a device pixel the other way; a few labels sit a device pixel off (the title bar's usage line, the archive and pad header labels, a fan row's tool line).
- Dashed borders and shadow rings are paths with 4× MSAA where Chromium covers edges analytically: edge pixels can differ by an eighth of coverage.
- GPUI snaps layout lengths to device pixels where Chromium keeps fractions, so some boxes sit up to a device pixel off (e.g. children of a 1.5px border at 80 %).
- Tooltips are GPUI-drawn (`components/tooltip.rs`), so they show in shots where the goldens' native ones don't. ~~The pad's top sheet's wraps one word a line~~: fixed by gpui-polish (PATCHES 15).
