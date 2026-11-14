# 17 · Frames and groups

Added after 1.0.14. Before, the board had **sections**: a tinted box drawn round the session cards that named it, growing and shrinking with them, made with ⌘G and named by Claude (09, BOARD-080…BOARD-098). Sections become **frames**, as on a Miro board, and a separate, lighter **group** glues any things together.

- A **frame** is a named, tinted box with a size of its own. Whatever lies in it, of any kind, moves with it. It keeps its size as things move in and out. Claude names a frame made round a selection.
- A **group** glues things together so they're selected and moved as one. It isn't drawn, and no Claude is involved.

Sources:
- `crates/nodus-domain/src/board.rs` (`BoardSection` with `w`/`h`, `BoardGroup`)
- `crates/nodus-board/src/frames.rs` (frame geometry, checking and saving groups), `handlers.rs` (`save_section`, `name_section`, `save_groups`, `remove_groups`), `cards.rs` (a card's frame for the board tools)
- `crates/nodus-transcripts/src/naming.rs` (the naming prompt)
- `crates/nodus-tools/src/drawing.rs` (`list_board`)
- `crates/nodus-ui/src/board/frames.rs` (the canvas's side), `layout.rs` (`section_views`, `section_at`, `layout_frame`, `spot_in_section`, `new_section_corner`), `section.rs` (drawing a frame), `input.rs`, `commands.rs`, `menus.rs`, `overlays/selection.rs`

## Data

- **FRAME-001** A frame is stored as a section (`sections` in `state.json`, `board:save-section`), so older copies still read the board. It's a scene node at z −2, under everything, as sections were (BOARD-020).
- **FRAME-002** `BoardSection` gains `w` and `h`, the frame's size, left out when absent. The core keeps a sent size within 200×120…40000 units, rounded; a save without a size keeps the size it had.
- **FRAME-003** A section saved before frames had a size is drawn round the active cards that name it in `sectionId`, as BOARD-081 drew it. The first time the board shows it, it's saved with that box as its size, once per board opened, so it looks the same and keeps that size from then on. Cards' `sectionId` is no longer written or read for anything else.
- **GROUP-030** `BoardGroup { id, members, createdAt }`, kept in the board's `groups` (a lenient record map, as shapes are, SHAPE-030; left out while empty). Members are cards' session ids and notes', views' and shapes' ids. `board:save-groups` keeps each group with each member once and two or more members. A thing is in one group at most: a group saved with it takes it from any other, and a group left with fewer than two goes. A board keeps at most 2000 groups.
- **GROUP-031** `board:remove-groups` takes groups off the board; what was in them stays where it is.
- **GROUP-032** A member that left the board goes from its group when the board loads, and a group left with fewer than two goes. An archived card stays in its group, and is back in it when it's restored.

## Frames

- **FRAME-004** A frame draws as a section did (spec 09 § UI details › Sections): its header with its name (`Untitled frame` when it has none), how many things are in it, and the status dot of the sessions among them. An empty frame shows `Drag things here` in a dashed box across its body. The rename field's placeholder is `Frame name`.
- **FRAME-005** `New frame here` (the pane menu's `New section here` before) makes an empty frame 768×818, which a new session's card (720×750) fits in, with its header at the click. If that spot isn't clear of the things on the board, the dock and other frames, it goes to the nearest free spot (`findFreeSpot`), and the view pans to show it. (0.9.4 sent a section it couldn't place 40 card rows down, BOARD-092.) It starts renaming.
- **FRAME-010** What's in a frame is what has its centre in its box, edges counting: active cards, free notes, views and shapes. Where frames overlap, a thing is in the smallest. Nothing records it: dropping a thing in a frame puts it in, and dragging it out takes it out.
- **FRAME-011** The header's count is everything in the frame. Its status is the most urgent of the sessions in it (BOARD-083).
- **FRAME-012** Dragged by its header, a frame takes along what's in it, the frames wholly inside it, and whatever is glued to any of those (GROUP-010).
- **FRAME-013** A frame resizes from its edges and corners: 10 px grab zones straddling its sides (the top one only above its edge, leaving the header to its drag) and 16 px ones at its corners, with the resize cursors. The far sides stay put, it never goes under 200×120, and it follows the pointer as it moves (no glide). Released, it's saved rounded. Nothing in it moves, and what its new box holds is in it from then on. A read-only board has no grab zones.
- **FRAME-014** On release, the frame and everything it took along are saved where they landed: cards in one patch, then notes, views and frames. Shapes are saved as one change ⌘Z takes back (SHAPE-040). The frame keeps its size. While it's dragged, nothing it carries glides.
- **FRAME-015** While a card, note, view or shape is dragged, the frame its centre would land in shows its hot style (BOARD-098), for any kind of thing.
- **FRAME-020** **Put in a frame** (⌥⌘G, Ctrl+Alt+G off macOS; the selection bar's Frame; every thing's menu) puts the selected things, and whatever is glued to them, in a new frame. The frame wraps them: 24 units at the sides and bottom, the 44-unit header on top. When that box would cross something else, or come within 32 units of the dock or another frame, they're tidied first: packed in rows of up to four, in reading order, each row as tall as its tallest, at the nearest free spot (BOARD-088's tidy, for things of any size).
- **FRAME-021** The frame takes the next unused folder colour (BOARD-085). Tidied things are saved where they went (FRAME-014). The selection clears and the view shows the frame's corner (BOARD-091).
- **FRAME-022** Claude names the new frame when something in it says what it's about (FRAME-030): its header shows `Naming…` with Claude's glow and the name lands as BOARD-089 and BOARD-090 describe. Otherwise it starts renaming.
- **FRAME-024** `New session in frame` and `New terminal in frame` put the card at the first free slot of a grid inside the frame, slots the size of a new card (720×750), free as `findFreeSpot` has it. When the frame is full, the slots go on below its bottom, and the frame grows to hold the card.
- **FRAME-025** `Delete frame` (the frame's menu) takes it away at once; what was in it stays where it is.
- **FRAME-026** `Select what’s in it` selects everything in the frame (FRAME-012), of every kind.
- **FRAME-027** A card in a frame has `Move out of frame` in its menu: it goes to a free spot right of the frame (BOARD-094).
- The frame's menu otherwise keeps the section's: Rename, its colours, the new session and terminal, `Archive N sessions`.

## Naming a frame

- **FRAME-030** Things that say what a frame is about: a Claude session (not a terminal) with a title or a transcript; a note with words; a view with a title; a shape with words in it.
- **FRAME-031** `board:name-section(projectId, ids)` takes those things' ids (the wire's field is still `sessionIds`). Each source is a session (its title, and its opening prompt as SESS-110 has it), or the words of a note, view or shape, cut to 280 UTF-16 units. No sources → `Nothing in this frame says what to name it yet`. The prompt is `Name this frame. It holds:\n\n` and then one entry per source, joined by `\n`: `` `${i + 1}. ${Kind}: ${title}` ``, Kind being `Session`, `Note`, `View` or `Words`, with a session's `\n   Opening prompt: …` as before. The system prompt: `You name frames on a board where someone works with Claude Code. A frame holds sessions, sticky notes, views and words drawn on the board. Reply with the frame's name only: 1 to 4 words in Title Case that say what the things in it have in common. No quotes, no trailing punctuation, no preamble.` A server from before frames names a frame after the sessions in it only.

## Groups

- **GROUP-001** **Group** (⌘G, Ctrl+G off macOS; the selection bar; every thing's menu) glues the selected things together, with whatever was already glued to them, as one new group: the groups they were in are taken in. Fewer than two things, or things that are already exactly one group: nothing happens. No Claude is asked. Before, ⌘G made a section that Claude named (BOARD-087); that's now ⌥⌘G (FRAME-020).
- **GROUP-002** What can be glued: active cards, free notes, views and shapes. Archived cards, notes stuck on a card, frames, the archive and the pad can't.
- **GROUP-003** **Ungroup** (⇧⌘G, Ctrl+Shift+G off macOS; the selection bar's button when the selection is one group; every thing's menu while it's in a group) takes apart every group a selected thing is in. The things stay where they are, selected.
- **GROUP-010** Selecting a thing selects what's glued to it: a click, a marquee that touches it, a drag that starts on it. ⇧ or ⌘ on a selected thing takes it and its group out of the selection. So a click on a grouped card still opens it (BOARD-043) and also selects its group, and dragging any member moves them all.
- **GROUP-011** Each group something selected is in shows a dashed accent outline, 1.5 px, radius 10, 8 units round its things as they're drawn, over the nodes.
- **GROUP-020** Every thing's menu (card, terminal, note, view, shape) acts on the selection it's in, or the thing with whatever is glued to it: `Group` (⌘G) when that's two or more things that aren't already one group, `Ungroup` (⇧⌘G) when any of them is in a group, and `Put in a frame` (⌥⌘G).
- **GROUP-021** The selection bar (SEL-001) shows while a card, or two or more things, are selected. `N selected` counts the things (archived cards selected in the open stack included). After `New session from context`: `Group` (disabled for fewer than two) or, when the selection is exactly one group, `Ungroup`; then `Frame` (tooltip `Put in a frame Claude names (⌥⌘G)`); then `Archive`.

## Claude's board tools

- **FRAME-040** `list_board` (TOOL-102) lists `frames`, each `{ id, name, x, y, w, h }`, when there are any. `list_sessions` gives a card's `section` as the name of the frame it's in (FRAME-010), `Untitled frame` when unnamed.
- **GROUP-040** `list_board` lists `groups`, each as the ids in it, when there are any. The tool's description says so. Sessions can't make frames or groups yet.

## Not covered

- ⌘Z doesn't take back making, moving, resizing or deleting a frame, or grouping (only shapes' moves among what a frame drag saves).
- ⌘C doesn't copy groups or frames.
