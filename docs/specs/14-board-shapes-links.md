# 14 · Shapes, pictures and links

Added after 1.0.8; 0.9.4 had none of this. It makes the board work like Claude Whiteboards: things on it move freely, and arrows tie them together. The board tools that let a session draw are spec 04's TOOL-101…TOOL-110; after 1.0.17 a session also sees the board as a picture, puts up pictures from files and draws ink (TOOL-111…TOOL-114).

Sources:
- `crates/nodus-domain/src/board.rs` (`ShapeKind`, `BoardShape`, `LinkEnd`, `BoardLink`)
- `crates/nodus-board/src/routes.rs` (where a link runs)
- `crates/nodus-store/src/records.rs` (how `state.json` keeps them), `state.rs`
- `crates/nodus-board/src/shapes.rs` (checks, saving, pruning), `drawing.rs` (the undo history)
- `crates/nodus-core/src/board.rs` (`save_shapes`, `remove_shapes`, `save_links`, `remove_links`, `save_image`, `import_image`)
- `crates/nodus-ui/src/board/shapes.rs` (the canvas's side), `images.rs`, `cards/shape.rs` (drawing a shape), `overlays/ink/links.rs` (drawing links), `overlays/ink/{input,dock,ops,draw}.rs` (the tools), `theme/tokens.rs` (the shapes' palette)

## Summary

A **shape** is a thing on the board, like a note: a rectangle, ellipse or diamond with words in it, words on their own, or a picture. It is a node in the board's scene (09), so it is selected, marquee'd, dragged, multi-dragged and resized with the cards, notes and views. A **link** is an arrow from one thing to another: an active card, a free note, a view or a shape, or a loose point. Links run between the boxes as drawn, so they follow anything that moves. Ink (11) stays as it was: marks over the board, not things on it.

## Data

- **SHAPE-030** A board's `shapes` and `links` sit in `state.json` beside `ink`, as objects keyed by id, oldest first. They're left out while empty, so a board without any is written byte for byte as before. A record this version can't read, such as a newer version's kind of shape or an emoji sticker from before they were taken out (SHAPE-005), is kept as it was and written back. It isn't drawn, and it doesn't make `state.json` unreadable (`Records`). A server board's snapshot leaves such a shape out in the same way, so a server on another version can't make its board unreadable.
- `BoardShape { id, kind: rect|ellipse|diamond|text|image, x, y, w, h, color: <ink colour>, text?, image?, by?, createdAt }`.
  - `text` holds the words.
  - `image` is a picture's file name in `<userData>/board-images/`: a plain name, never a path.
  - `by` is the session that drew it with its board tools.
  - Saved shapes are checked. Sizes are rounded into 16–4000 board units and positions rounded. Words are kept up to 2000 UTF-16 units. A picture needs its file. A board keeps at most 2000 shapes.
- `BoardLink { id, from: LinkEnd, to: LinkEnd, color, label?, by?, createdAt }`, `LinkEnd { node?, x, y }`.
  - An end holds the id of a card (its session id), a note, a view or a shape. Without one it's loose at `x, y`.
  - `to` has the arrowhead.
  - Labels are kept up to 200 units. A link with both ends on the same thing isn't kept. A board keeps at most 2000 links.
- **SHAPE-031** A link goes when what holds one of its ends goes: a shape or note deleted, a card removed, and anything gone when the board next loads (a deleted view). A link to an archived card is kept but not drawn until the card comes back.

## Shapes

- **SHAPE-001** Shapes are nodes, oldest first, after the views and before the free notes. They draw at z 0. Every one is draggable and selectable.
- **SHAPE-002** The dock's **Shape** tool (key `R`, in the place of 1.0.7's ink rectangle) draws the kind picked, from corner to corner. Shift keeps it square, or a circle. A press that moves less than 8 screen px puts one down at its usual size, centred on the press: 180×96 for a rectangle or ellipse, 160×112 for a diamond, scaled by `sizeAt(zoom)` so it looks the same size at any zoom. The new shape has the shapes' colour (SHAPE-017). It's selected with its words open for writing, and the tool goes back to the Pointer.
- **SHAPE-003** Pressing the Shape button takes up the tool with its picker out, to its right: Rectangle, Ellipse, Diamond, the current one shown on. Pressing the button again puts the picker away, as does a press anywhere else or Esc. The button's icon is the current kind's. `R` takes up the tool without the picker.
- **SHAPE-004** **Text** (no key) puts down words on their own, 240×40 (scaled), the line centred on the press, open for writing. The tool goes back to the Pointer. Words left empty go.
- **SHAPE-005** Emoji stickers were taken out after 1.0.33 (feedback #17). From 1.0.9 the dock had a **Sticker** tool (`S`) that stamped an emoji from a picker of 16, and the board tools had a `sticker` kind. Now the dock has no Sticker button, `S` does nothing on the board, and `draw_on_board` refuses `sticker` as it refuses any kind it doesn't know (TOOL-103). Stickers already on a board stay in `state.json` but aren't drawn (SHAPE-030), and a link to one is kept and isn't drawn either (LINK-003).
- **SHAPE-006** **Add a picture…** (the dock button under Text) opens the system's file picker for several files. PNG, JPEG, GIF and WebP files go up in the middle of the view, each 32 units (scaled) down and right of the one before.
- **SHAPE-007** ⌘V (Ctrl+V) on the board with a picture on the clipboard and no copied notes puts it up at the pointer, or the view's middle.
- **SHAPE-008** Picture files dropped on the board, over cards and notes too, go up where they're dropped.
- **SHAPE-041** On a server board (hosted Nodus, added after 1.0.12), a picture added by SHAPE-006…SHAPE-008 is uploaded to the server (`PUT /api/uploads`), which picks its name, and the shape holds that name; the bytes are kept here too, in `server-files/<server>/<name>`. A picture the board has but this app hasn't is fetched once with `GET /api/files/<name>` when it's first drawn, and the board redraws when it's in; until then, and while the server is away, it draws as a picture not yet loaded. Adding a picture while the server is away fails, as every change to a server board does (SRV-163, SRV-028).
  - Other files are left out.
  - A picture is copied into `board-images` under a new name, then shown at its own proportions (read from its header), its longer side at most 480 units (scaled) and never larger than it is.
  - Files aren't removed when their shape goes, so ⌘Z can bring it back.
- **SHAPE-010** Dragging moves shapes with the rest of the selection. A shape dropped on the archive or the note pad goes back to where it was. Moves are saved rounded and glide as notes do (⌘Z, a session's moves).
- **SHAPE-011** Selected, a shape resizes from its edges and corners as a note does (NOTEUI-025): 9px handles, down to 16 units.
- **SHAPE-012** Double-click, Return (the one shape selected) or the menu's **Edit text** writes in a rectangle, ellipse, diamond or text.
  - Esc, ⌘Return, or leaving the field puts the words in, and the board takes the keyboard back. Keys, presses and the wheel stay in the field meanwhile.
  - A box's words are centred, the UI font at 15/21, semibold, in the theme's text colour. A diamond's keep to its middle half.
  - Text on its own is 18/24 semibold in its colour's words tone (SHAPE-013).
- **SHAPE-013** Shapes have a palette of their own, apart from the pen's, picked for each theme (added after 1.0.32; until then a shape was its ink colour, outlined in it over a 14% tint). Each colour has a fill, an edge and a words tone:

  | Colour | Light: fill / edge / words | Dark: fill / edge / words |
  |---|---|---|
  | Graphite | `#ffffff` / `#c4c4ca` / text | `#2c2c30` / `#5c5c63` / text |
  | Red | `#fde4e1` / `#ef7f75` / `#d0342a` | `#432826` / `#c4625a` / `#ff7f75` |
  | Orange | `#feecd6` / `#f2a94a` / `#b86a0a` | `#43341f` / `#c99447` / `#ffb347` |
  | Green | `#e1f4e4` / `#62bd77` / `#25893b` | `#223a28` / `#58a86a` / `#5fd677` |
  | Blue | `#e1edfe` / `#5f99ef` / `#0a66d6` | `#213450` / `#5389d6` / `#5aa5ff` |
  | Purple | `#efe4fb` / `#b383e4` / `#9447c4` | `#382a48` / `#9c72c6` / `#cc84f2` |

  A rectangle (6-unit corners), ellipse or diamond is filled with its fill and edged 2 units in its edge, inside its box. A picture is shown whole (`contain`) with 8-unit corners and a small shadow. Selected or being written in, a shape has a 1.5px accent ring 3 units outside its box, a rectangle's ring rounded 9 units to follow its corners.
- **SHAPE-015** Picking a colour in the palette also recolours the selected shapes and the selected link, as one change.
- **SHAPE-016** The minimap draws shapes in their edge colour.
- **SHAPE-017** New shapes, words and arrows take the shapes' colour, which is kept apart from the pen's (INKUI-004): Graphite at first, then the colour last picked for them, app-wide and across restarts (key `nodus.shapeColor`). The palette works on it while the Shape, Arrow or Text tool is out, or the Pointer has shapes or a link selected and no ink stroke; its button then reads **Shape colour** and its dot and swatches show the shapes' edge colours. Otherwise it works on the pen's colour, as before.
- **SHAPE-020** ⌫ / Delete deletes the selected shapes with the cards, notes and views selected beside them, and every link they held, as one change.
- **SHAPE-021** The eraser takes shapes it passes over (within 6 screen px of the box) and links it passes over (within 6 screen px of the curve), with the ink, as one change. A shape's links go with it.
- **SHAPE-022** A shape's right-click menu: **Edit text** (↩, for the kinds with words), then **Delete** (⌫), or **Delete N shapes** for a selection.
- **SHAPE-040** ⌘Z / ⇧⌘Z take back and redo shape and link changes with the ink's, in one history of 100 steps (INKUI-021). A shape or link goes back one by one, as it was before the change, so things a session drew in the meantime stay. An empty text shape that goes as it's put down leaves no step.

## Links

- **LINK-001** A link runs between the boxes of what holds its ends, as drawn now, in straight runs at right angles with bends rounded 10 units (less where a run is short). Until 1.0.32 it was the lineage edge's Bézier, which lineage edges keep (BOARD-029, BOARD-030).
  - Two boxes at least 24 units apart one way: from the side of one to the facing side of the other. Straight across where their facing sides share 16 units or more (at the middle of the two boxes' middles, kept 8 units inside what they share); else a step halfway between them.
  - At least 24 units apart both ways: one bend, out of the side facing along the longer way and into the other's top or bottom (or out of the top or bottom and into a side).
  - Closer than that: as for the way they're further apart; overlapping: straight between their edges.
  - A box and a loose point: out of the side facing the point (the box's proportions decide which, as BOARD-029's do), straight when the point is level with that side, else one bend. Two points: a straight line.
- **LINK-002** Drawn above the cards and under the ink's strokes, 1.75 units wide (the accent when selected), with a filled 10-unit arrowhead at `to`. A Graphite link is a quiet grey (`#a3a3aa` light, `#74747c` dark); another colour is its shape edge (SHAPE-013).
- **LINK-003** It's drawn while both ends are. That leaves out an archived card, a note stuck on a card, and something hidden.
- **LINK-004** A label sits halfway along the link: a chip with round ends in `--bg-elevated`, with a 1-unit accent border while its link is selected or it's being written. Its words are the UI font 13/22 medium in `--text-secondary`, with 10 units each side.
- **LINK-005** The selected link shows both ends as handles: 10px circles in its colour around a `--bg-elevated` centre.
- **LINK-010** The **Arrow** tool (`A`) draws a link from what's pressed, or from the spot.
  - While dragging, what's under the pointer that it can hold has a dashed accent box 6px outside it, and a loose end is dashed.
  - On release it holds what's under the pointer, or ends loose there. It has the shapes' colour (SHAPE-017), and the tool stays out. 1.0.7's ink arrows stay as ink.
- **LINK-011** A link that reached nothing and moved less than 8 screen px is a slip and isn't kept.
- **LINK-012** With the Pointer, a press within 6 screen px of a link, or on its label, selects it. This drops the selection of cards, notes, views and ink, and the board doesn't see the press. A press elsewhere drops it. Hovering one shows the pointing hand.
- **LINK-013** ⌫ / Delete deletes the selected link. Esc lets go of it.
- **LINK-014** Dragging an end handle of the selected link takes that end elsewhere: onto something (boxed while over it) or loose.
- **LINK-020** A double-click on a link or its label, or **Edit label** in its menu, writes its label in place. Return or leaving the field keeps it, Esc leaves it as it was, and an emptied label goes.
- **LINK-021** The palette recolours the selected link (SHAPE-015).
- **LINK-022** A right-click on a link, over the board or a node: **Edit label**, then **Delete link** (⌫).

## Keys

`R` Shape, `A` Arrow (with the board's usual guards, INKUI-002). ↩ writes in the one selected shape. ⌫ deletes selected shapes and the selected link. ⌘Z / ⇧⌘Z (SHAPE-040).

## Known limits

- A view's live page is a native webview (decision D3), so a shape or link crossing over a view is hidden under its page while the page is live.
- Shapes aren't copied by ⌘C.
