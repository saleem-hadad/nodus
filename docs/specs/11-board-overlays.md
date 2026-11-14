# 11 · Prompt bar, search, merge dialog, ink, bubble & selection bar

Sources:
- `src/renderer/src/components/sidekick/Sidekick.tsx` (387 lines, all read)
- `src/renderer/src/components/sidekick/compose.ts` (62, all read) + `compose.test.ts` (83, all read)
- `src/renderer/src/components/sidekick/sidekick.css` (355, all read)
- `src/renderer/src/components/board/SessionSearch.tsx` (297, all read), `search.css` (256, all read)
- `src/renderer/src/components/board/fuzzy.ts` (92, all read) + `fuzzy.test.ts` (36, all read)
- `src/renderer/src/components/Highlight.tsx` (15, all read)
- `src/renderer/src/components/board/MergeDialog.tsx` (395, all read)
- `src/renderer/src/components/board/InkLayer.tsx` (687, all read), `ink.ts` (175, all read) + `ink.test.ts` (74, all read), `ink.css` (181, all read)
- `src/renderer/src/components/board/SelectionBar.tsx` (66, all read)
- `src/renderer/src/views/BoardView.tsx` — the wiring only: state (lines 531–546), toast (788–791), `clearSelection` (834), `ensureVisible` (848–868, owned by 09), `openCard` (870–880), `viewportCentre` / `createView` / `freeNoteSpot` (1178–1283), `selectCards` / `startMerge` / `jumpToCard` / `onMergeStart` (1415–1494), the bar (1496–1620), the card menu's merge item (2282–2288), keyboard + menu commands (2365–2468), render (2470–2683); the archive stack's use of `fuzzyMatch` (571–584)
- `src/renderer/src/components/board/board.css` lines 36–59 (`.spin`, `kbd`), 1663–1720 (toast, selection bar), 1751–1987 (merge dialog), 1581–1593 (zoom pill, for the ink dock's placement)
- Supporting reads (owned elsewhere, quoted where these components depend on them): `styles/tokens.css`, `styles/global.css` (`.btn*`, `.status-dot`, `:focus-visible`, reduced motion), `components/Modal.tsx` + `Modal.css`, `components/ColorSwatches.tsx` + `.css`, `components/FolderIcon.tsx` (`FOLDER_HUES`), `components/board/notes.ts` (`NOTE_COLOR`), `components/voice/useVoice.ts`, `VoiceParts.tsx`, `voice.css`, `recorder.ts` (`BANDS`), `shared/search.ts` (`queryTerms`, `matchRanges`), `shared/types.ts` (search, summarize, ink types), `shared/geometry.ts` (sizes), `components/board/layout.ts` (`timeAgo`, `truncate`, `spotBelow`), `store.ts` (`cardTitle`, `saveInk`, `PreviewFind`), `platform.ts` (`keys`, `modKey`, `otherModKey`), `main/menu.ts` (Search Sessions… item), `@xyflow/system` 0.0.83 (setCenter easing)
- `docs/how-it-works.md` §The prompt bar, §New session from context, §Shortcuts
- Screenshots: `docs/screenshots/search.png`, `search-dark.png`, `context.png`, `context-dark.png` (viewed). `board*.png` predate the prompt bar and the ink dock and show neither; **the code wins** wherever a screenshot differs.

Cross-domain owners: canvas mechanics, viewport, selection model, z-order, `ensureVisible`, `findFreeSpot`, `spotBelow`, sections (`groupCards`), archive confirm, context menus, the board's other shortcuts → **09-board-canvas.md**. Cards, stickers, notes, the archive stack UI, card relative time → **10-board-cards-notes.md**. Voice phases, recorder, meter levels, transcription, voice notices → **05-actions-voice-git.md**. Search ranking and snippets, summarize (fork + progress events), session create/send → **03-sessions-transcripts.md**. Views create, ink persistence (`board:saveInk`) → **04-board-tools-views.md**. Modal chrome, tokens, `.btn`, `kbd`, global focus ring → **08-projects-shell-ui.md**. Transcript preview (opening at a search match), terminal modal → **12-terminal-ui.md**. App menu → **01-platform-shell.md**.

## Summary

Six overlays sit on the board:

1. **Prompt bar** ("sidekick"): a frosted pill, bottom centre, with a round mic button beside it. Type and press Return to start a Claude session in auto mode; its card lands **expanded** in a free spot in view, with the words stuck on it as a note; if it can't start, the words become a note. `/` opens a two-item menu; `/view` and `/note` turn into a token at the start of the pill so the bar makes a view or a note instead (a note token has a colour swatch). With exactly one live Claude session selected, the bar shows its name as a token and Return sends the text into that session (a session waiting on a question is opened instead). With more selected (sessions, notes, views) the new session gets them as context, in a fixed Markdown template, ahead of the prompt. The toast, the selection bar and voice notices stack above the bar.
2. **⌘K search**: a Spotlight-like modal high on the window. Empty query lists the 30 latest sessions; typing searches every session's transcript (debounced, via 03) and shows rows with status, highlighted title, match count, time and up to a few snippets with highlighted ranges. ↑/↓, Return opens (centres and flashes a live card, or opens an archived one's transcript at the match), Tab marks rows, ⌘Return starts a new session from the marked rows (or the current one) through the merge dialog, focused on the query.
3. **Merge dialog** ("New session from context"): summarizes each source session in a forked copy (03), showing per-session progress (Queued / Summarizing… / Ready / Failed) and an elapsed counter, then a review step to edit the combined handoff, write an instruction and a name; ⌘Return starts the session with `context + --- + instruction`.
4. **Ink**: a vertical tool dock above the zoom controls (pointer, pen, arrow, rectangle, text, eraser, colour, clear) and a layer that draws pen strokes, arrows, rectangles and text labels over every card, sized so they keep the on-screen thickness they had when drawn. Pointer events are taken ahead of the board in the capture phase; the pointer tool selects, moves, recolours and deletes ink; ⌘Z/⇧⌘Z undo/redo up to 100 changes per board.
5. **Selection bar**: a pill above the prompt bar while cards are selected: `N selected`, **New session from context**, **Group**, **Archive**, ×.
6. **Asking Claude on the spot** (added after 0.9.4): two dock tools under the pointer. With the **bubble** out, a click on the board opens a small frosted field pointing at the spot; with the **mic** out, a click starts listening there and the next click sends what was said. Either way it goes at once: a marker waits on the spot while Claude (Haiku) decides in the background whether it's a session (and on which model: Haiku, Sonnet or Opus), a sticky note or a view, which then lands with its top left on the spot. The session runs on that model from then on, resumes included.

The theme follows `nativeTheme.themeSource` (System/Light/Dark, 01/08); the CSS uses `prefers-color-scheme`, so "dark" below means whenever the resolved scheme is dark. Every colour below is given as **L** (light) / **D** (dark).

## Behaviors

### Prompt bar — BAR

**Placement & stacking**

- **BAR-001** The bar's column is anchored bottom-centre of the board area (the `.board-canvas` region under the title bar): horizontally centred, its bottom edge 20px above the board's bottom edge, width `max(300, min(672, boardWidth − 416))` px (clears the zoom controls on the left and the minimap on the right). It paints above the canvas, cards, ink dock, zoom controls and minimap (z 20), below modals (z 500). Only its children take pointer events; the empty parts of the column pass clicks through to the board.
- **BAR-002** The column is a vertical stack, items centred horizontally, 10px apart, bottom-anchored, in this order top → bottom: the toast (when shown), the selection bar (SEL), the voice notice (05, when shown), the row (pill + mic). Each of the first three, when it appears, rises in: opacity 0 → 1 and `translateY(6px)` → 0 over 220ms `cubic-bezier(0.2, 0.8, 0.2, 1)`.
- **BAR-003** The row holds the pill and the mic button, 10px apart, centred, bottom-aligned (the 52px mic's bottom lines up with the pill's bottom). The pill's preferred width is 520px closed and 600px open (BAR-004), shrinking (never growing) when the row is narrower than `pill + 10 + 52`; the width change animates over 220ms `cubic-bezier(0.2, 0.8, 0.2, 1)`, the shadow change over 220ms `ease`.
- **BAR-004** The pill is **open** while any of these holds: the field is focused; the field has text; a `/view` or `/note` token is set; a session token (into) shows; the voice phase is not `idle` (opening, recording or transcribing); one or more selected items count as context (`contextCount > 0`, BAR-024). Open = 600px preferred width and the large shadow.

**Field**

- **BAR-005** The field is a multi-line plain-text input, one line tall when empty, growing one 20px line at a time with its content up to 5 lines (field height `20·lines + 14`, max 114px; pill height `20·lines + 28`, 48–128px); beyond 5 lines it scrolls. The pill grows upwards (bottom stays put). Spell checking is on (Electron default; see Platform differences).
- **BAR-006** Placeholder, first match wins: target view → `Describe the view, e.g. failing tests by file`; target note → `Write a note…`; sending into a session → `Reply to this session…`; `contextCount === 1` → `Ask Claude about this…`; `contextCount > 1` → `Ask Claude about these ${contextCount}…`; else `Ask Claude to build, fix or explain…`.
- **BAR-007** A hint `[/] view or note` (a key cap holding `/`, then the words) shows at the pill's right, before the send button, vertically centred, only while: no text, no target, no session token, `contextCount === 0`, and not listening (recording/transcribing). It never takes pointer events.
- **BAR-008** A mouse-down on the pill's own background (not on a child: token, field, button, hint excluded since it passes through) while not listening keeps focus from moving away and focuses the field. The pill's background shows the text (I-beam) cursor.

**Submitting**

- **BAR-009** Return (no Shift) in the field submits; Shift+Return inserts a newline. While an IME composition is in progress, the field ignores every key for its own handling (Return, arrows, Tab, Backspace, Esc fall through to the IME).
- **BAR-010** Submit does nothing when: the trimmed text is empty; the voice phase is not `idle`; a submission is already in flight; or the slash menu is open.
- **BAR-011** What's submitted, first match wins, with the **trimmed** text: target view → `{ kind: 'view', text }`; target note → `{ kind: 'note', text, color }`; session token showing → `{ kind: 'into', sessionId, text }`; else `{ kind: 'session', text }`.
- **BAR-012** While the submission is in flight the send button shows a spinning `Loader2` (15px) and is disabled. When the board answers `false`, everything stays as typed (text, token). When it answers `true`, the text is cleared, the target is cleared (back to the default mode), and the voice notice is dismissed (05 `dismiss`: also forgets a kept recording). The note colour, the dropped-session state (BAR-020) and focus are kept.
- **BAR-013** The send button's tooltip is `${label} (Return)` and its accessible name `${label}`, with label: target view → `Create view`; target note → `Add note`; session token → `Send to “${title}”`; else `Start session`. It is disabled when the trimmed text is empty, the voice phase is not `idle`, a submission is in flight, or the slash menu is open.

**Slash menu & tokens**

- **BAR-014** The slash menu is open while: the field is focused, no target is set, the user hasn't dismissed it (BAR-016), and the whole text is a single slash word (`^/\S*$`). It lists the commands whose name starts with the lowercased text: `/` → both; `/v`, `/V`, `/vie` → `/view`; `/n` → `/note`; `/x`, `/view ` (with a space), `fix the /view`, `` → none (menu closed). Order is always `/view` then `/note`.
- **BAR-015** The menu's highlighted option starts at the first and resets to the first on every edit of the field. ↓ moves down and ↑ moves up, both wrapping. Return or Tab picks the highlighted option (never submits). Hovering an option highlights it; clicking picks it; pressing the mouse on an option doesn't move focus out of the field.
- **BAR-016** Esc while the menu is open closes it (dismissed) without blurring; any edit re-opens it (when BAR-014 still holds); blurring the field resets the dismissal.
- **BAR-017** Picking a command sets the target (`view` or `note`), **clears the typed text** (e.g. `/vi` disappears), and resets the highlight.
- **BAR-018** When no target is set and an edit leaves the text matching `^/(view|note)\s+([\s\S]*)$` (case-insensitive) — `/view` or `/note` typed out and followed by whitespace (space, newline), or pasted with more after it — the target becomes that command and the text becomes everything after the whitespace run (`/NOTE  ask finance⏎by Friday` → note token + `ask finance⏎by Friday`; `/view ` → view token + empty). `/view` alone, `/views of x` and `add a /view` are left as text. While a target is set, typed slashes are plain text.
- **BAR-019** Tokens render at the pill's start, before the field (the pill's left padding shrinks from 18px to 8px while any token shows), and animate in (opacity 0 → 1, scale 0.9 → 1, 120ms `cubic-bezier(0.2, 0.8, 0.2, 1)`):
  - view token: `LayoutDashboard` icon + `View` + ×;
  - note token: colour swatch button + `Note` + ×;
  - session token (into): `CornerDownRight` icon + the session's title (ellipsised at 240px token width) + ×; tooltip `Sent into “${title}”`.
  The view/note token's × (tooltip and name `Start a session instead`) clears the target. The session token's × (tooltip and name `Start a new session instead`) drops it (BAR-020).
- **BAR-020** The session token shows when: no target is set, the board passes an `into` session (BAR-023), and that session id isn't the dropped one. Dropping it (× or Backspace) leaves the selection as it is, so the session then counts as context for a new session (placeholder becomes `Ask Claude about this…` for one). The dropped id is forgotten whenever the `into` session id changes (including to none), so a new or re-made selection is sent into again.
- **BAR-021** Backspace in an empty field (no text at all, not just whitespace) with a token: removes the target if one is set (after which the session token reappears if BAR-020 holds), otherwise drops the session token. With no token, Backspace does its normal thing.
- **BAR-022** The note colour starts as yellow (`NOTE_COLOR`) each time the board mounts. Clicking the note token's swatch toggles a colour picker above the pill (8 swatches: blue, purple, pink, red, orange, yellow, green, graphite); picking one sets the colour, closes the picker and refocuses the field. The picker closes on a pointer-down anywhere outside the picker and the swatch (seen before anything else handles it). The swatch's tooltip is `Note colour`, its accessible name `Note colour: ${color}` (lowercase colour id) and it reports expanded while the picker is open. The `/note` menu option's icon shows the current colour.
- **BAR-023** Esc in the field (menu closed): closes the colour picker if open, otherwise blurs the field. Esc never reaches the board's own Esc from the field.

**Selection-dependent modes (board wiring)**

- **BAR-024** The board passes `into` = the selected card iff exactly one session-type card is selected, it is a Claude session (not a terminal), not archived, and no note **with non-blank text** and no view is selected (empty notes don't count). `contextCount` = selected non-terminal session cards (archived ones included, e.g. selected stack results) + selected notes with non-blank text + selected views.
- **BAR-025** A `session` submission builds context items from the current selection, in this order: selected non-terminal session cards in board-card order (title = `cardTitle` = custom title, else Claude's title, else `New session`; branch = `meta.gitBranch`; last reply = `meta.lastAssistantText`; transcript = `meta.transcriptPath`), then selected notes with non-blank text in board-note order (raw text), then selected views in board-view order (title, description). The first prompt is `withContext(items, text)` (Data & formats).
- **BAR-026** The new session's card is placed at expanded size (720×750 flow units): preferred spot = `spotBelow(sources, dock, size)` when at least one session is in the context (x = mean of the sources' x, y = lowest source bottom + 96; moved right of the archive by 80 when that would overlap it, 09), otherwise the viewport centre (the card's top-left such that it is centred in the visible board); then `findFreeSpot(preferred, cards + notes + views, dock = archive ∪ note pad, size)` (09); rounded to whole units. A spot inside a section's frame joins that section.
- **BAR-027** The session is created with: position, `initialPrompt` (BAR-025), `parents` = the context sessions' ids (omitted when none), `name` = `truncate(firstLineOfText, 64)` when there is any context item (otherwise omitted, so Claude names it), `expanded: true`, `autoMode: true`. On success: the card is added; joined to the section if any; a note `{ id: new uuid, text: <trimmed text>, color: yellow, x, y, createdAt: now, sessionId: <new id> }` is saved so it rides on the card as a sticker (10); the whole board selection is cleared **only if** there was context; the board pans to show the card (`ensureVisible`, 320ms, 09). The session's window does **not** open and the keyboard stays where it was.
- **BAR-028** If creating the session fails, the same note is saved **without** `sessionId` at the same position (a free yellow note), the board pans to show a 216×216 note there, and the toast `Couldn't start a session, so your prompt is on a note: ${message}` shows. The bar still answers `true` (clears) — nothing typed is lost.
- **BAR-029** An `into` submission: if the card is no longer on the board or is archived → toast `That session is no longer on the board.`, answer `false`. If its status is `waiting` → toast `“${title}” is waiting for your answer. Answer it, then send this.`, the card opens (archived → transcript preview; expanded → its terminal is woken in place; else the terminal modal opens) and the answer is `false`. Otherwise the text is sent to the session (03: resumes a stopped session, queued while working); a failure → toast `Couldn't send it: ${message}`, `false`; success → the board pans to show the card at its current size, `true`.
- **BAR-030** A `view` submission creates a view (04) at `findFreeSpot(viewport centre for 480×360, …)`, adds it, pans to show it, answers `true`; a failure → toast `Couldn't create the view: ${message}`, `false`.
- **BAR-031** A `note` submission saves a note `{ id: new uuid, text, color: <picked colour>, x, y (rounded), createdAt: now }` at `findFreeSpot(viewport centre for 216×216, cards + notes + views, dock, 216×216)`, pans to show it, answers `true`.
- **BAR-032** The toast shows one message at a time, replacing any previous one, and hides 4200ms after it was shown unless another message replaced it meanwhile. It sits at the top of the bar's column (BAR-002) and is read as a status (polite live region).

**Keyboard entry from the board**

- **BAR-033** `/` on the board (no ⌘/Ctrl/Alt; Shift allowed, so layouts that type `/` with Shift work) focuses the field; if the field is empty and no target is set, it first puts `/` in it with the caret at the end — so the slash menu opens. Otherwise it only focuses. Ignored while a terminal modal, transcript preview, agent viewer, diff, context menu, confirm, merge, view dialog, action-rail overlay or search is open, or while a text field has focus (09's guard).
- **BAR-034** `V` on the board (no ⌘/Ctrl/Alt, not a key repeat, same guards as BAR-033) toggles voice: idle → start recording; recording → stop and transcribe; opening or transcribing → nothing.

**Mic & recording UI** (voice internals: 05)

- **BAR-035** The mic button (52px circle beside the pill) starts recording when idle and stops when recording; it is disabled while the microphone opens and while transcribing. Its tooltip and accessible name: recording → `Stop and review (V)`; transcribing → `Transcribing…`; else `Speak a prompt (V)`. Icon: idle/opening → `Mic` 22px stroke 2; recording → filled `Square` 16px; transcribing → spinning `Loader2` 22px.
- **BAR-036** While recording, the mic button turns solid red (status-waiting) with a white icon and a soft red halo whose spread is `3.6 + level × 14.4` px, where `level` is the loudest of the five meter bands (0–1, updated every animation frame, the halo follows in 80ms linear); the pill gets a 1.5px red ring.
- **BAR-037** While recording or transcribing (not while the microphone is opening) the field is hidden and a status line takes its place: recording → blinking red dot, clock `m:ss` (05 VOICE-036), five-bar level meter; transcribing → spinning `Loader2` 13px + `Transcribing…`; then a flexible spacer and a × button (tooltip `Cancel (Esc)`, name `Cancel`) that cancels (05).
- **BAR-038** Esc while the voice phase is not idle cancels the recording/transcription (05), before anything else hears it, when focus is on the board, the bar's field or anywhere that isn't another text field; it never also blurs the field, closes the menu or clears the selection.
- **BAR-039** What was said lands in the field: if the field's trimmed text is non-empty, the result is `trimEnd(existing) + " " + said`; otherwise just `said`. Once not listening, the field is focused with the caret at the end. Speaking again appends again.
- **BAR-040** A voice notice (05: message, optional Settings / Open System Settings / Retry buttons, ×) shows in the column directly above the row while there is one.

**Lifetime**

- **BAR-041** The bar's state (text, target, note colour, dropped id, picker) belongs to the open board: switching project or leaving the board resets it. Starting a recording elsewhere (another mic) cancels this one (05).

### ⌘K session search — SRCH

- **SRCH-001** ⌘K (Ctrl+K off macOS; no Shift, no Alt, not the other of ⌘/Ctrl) on the board opens the search, unless: the key event was already handled, or focus is inside a terminal (the terminal keeps ⌘K to clear itself); or a terminal modal, agent viewer, diff or any overlay (context menu, confirm, merge dialog, view dialog, action-rail overlay, search itself) is open. If a transcript preview is open it is closed first, then the search opens ("from a transcript a search opened, back to the results"). This check runs even when focus is in a text field (e.g. the prompt bar).
- **SRCH-002** The app menu item `Search Sessions…` (accelerator shown as ⌘K, handled by the board, 01) opens the search over whatever session is open: it closes the terminal modal and the transcript preview first. It does nothing while an overlay, the diff or the agent viewer is open.
- **SRCH-003** The search closes on: Esc (anywhere), ⌘K/Ctrl+K inside it, a pointer-down on the dimmed backdrop, opening a result, or starting a merge. Closing returns nothing; focus is not restored anywhere in particular.
- **SRCH-004** The query is kept between openings for as long as the board is mounted (reset on project switch). On open, the field takes focus with its whole text selected, so typing replaces it.
- **SRCH-005** With an empty (whitespace-only) query, the list is headed `Latest sessions` and shows the board's Claude sessions (live and archived; never terminals, action runs or view brains): live before archived, then most recently updated first (`meta.updatedAt`, else `createdAt`), at most 30. Each row's snippet is the session's last prompt (`meta.lastPrompt`) on one ellipsised line, or nothing. This list keeps its order and highlight as cards update. With no sessions at all: `No sessions on this board yet.` (no header).
- **SRCH-006** With a query, 120ms after the last edit the trimmed query is sent to the search backend (03, `sessions.search(projectId, trimmedQuery)`); an answer for a query that is no longer current (edited again, closed) is dropped.
- **SRCH-007** While the current query's answer hasn't arrived the field's icon is a spinning `Loader2`; before the first answer the list is empty; afterwards, the previous answer's rows stay until the new one arrives. The answer's hits are shown in the backend's order, each mapped to its board card; hits whose session isn't among the board's searchable cards are dropped.
- **SRCH-008** When the current query has been answered and no row remains: `No session on this board mentions **${query}**.` (query in semibold primary text) with a second line `Prompts, replies, file paths, commands and edits are searched; tool output isn’t.`
- **SRCH-009** A failed search shows the error message (red) under the rows. The error stays until a later search succeeds; while it is shown, the spinner does not show for newer queries and the no-match message does not show.
- **SRCH-010** Each row: a check box (SRCH-016), then a header line — a 7px status dot for a live card (working orange breathing / waiting red pulsing / done blue; tooltip `Working`, `Needs you`, `Done`) or an `Archive` icon (12px, name `Archived`) for an archived one; the title (`cardTitle`, one line, ellipsised, semibold) with the query's terms underlined (SRCH-020); for a search hit with `matches > 0`, `1 match` / `N matches`; the relative time of the last update (10 CARD-041 format). Below it, each of the hit's snippets on its own line (SRCH-011); for a row without a hit, the plain last prompt (SRCH-005).
- **SRCH-011** A snippet line: a label column — optional `Agent` (medium weight) when a subagent said/did it, then the speaker: `You` (prompt, accent colour), `Claude` (reply), or the tool's name (tool), then for a tool with a file, the file name in monospace — and the snippet text with its matched ranges highlighted (SRCH-019); a tool snippet's text is monospace 11px. Text wraps anywhere and is clamped to 2 lines (ellipsis).
- **SRCH-012** ↓/↑ move the highlighted row by one, stopping at the ends (no wrap). The highlight resets to the first row whenever the trimmed query changes or a new answer arrives. The highlighted row scrolls into view minimally (nearest edge). Moving the mouse over a row highlights it.
- **SRCH-013** Return (no ⌘) opens the highlighted row; clicking a row (without ⌘/Ctrl) opens it. The footer's Return hint reads `read it` when the highlighted row is archived, else `go to card`.
- **SRCH-014** Opening a **live** card: the search closes; that card becomes the only selected session/terminal card (notes' and views' selection is left as is — so with nothing else selected, the prompt bar now sends into it, BAR-024); the viewport animates over 320ms to centre the card at `zoom = max(currentZoom, 0.8)` (09's animated set-centre: smooth zoom interpolation, cubic in-out); when that finishes, the card's outline flashes twice: box-shadow from `0 0 0 2px accent, 0 0 0 2px accent@55%` to `0 0 0 2px accent, 0 0 0 18px accent@0%`, 800ms per iteration, 2 iterations, `cubic-bezier(0.2, 0.8, 0.2, 1)` (accent L `#007aff` / D `#0a84ff`).
- **SRCH-015** Opening an **archived** card: the search closes and the transcript preview opens (12) with `find = { query: trimmed query, snippet: the hit's first snippet (or none) }`, scrolled to that match.
- **SRCH-016** Marking: Tab toggles the highlighted row's mark and moves the highlight down one (Shift+Tab: up one), clamped; ⌘-click (Ctrl-click) on a row toggles its mark; clicking the row's check box toggles it (without opening). Focus never leaves the field on Tab. Marks are kept across query edits until the search closes, including marks on rows no longer listed. Marked rows are highlighted in the selection colour (over the hover colour).
- **SRCH-017** The footer's start button reads `New session from this` (no marks), `New session from 1` (one), `New session from ${n}` (n > 1), followed by key caps `⌘` `↩`. It is filled accent once anything is marked. It is disabled only when there are no rows and no marks. Clicking it, or ⌘Return (Ctrl+Return) anywhere in the dialog, starts with the marked ids (in marking order) or, with none marked, the highlighted row's id (nothing if no row): the search closes and the merge dialog opens with those sessions and `focus = the raw query` (MERGE-001).
- **SRCH-018** Searchable cards are the board's cards minus terminals (the board already excludes action runs and view brains). The list is a multi-select listbox; each row's selected state for assistive tech is its mark.
- **SRCH-019** Highlighting (Highlight.tsx): given text and `[start, end)` ranges (sorted, non-overlapping; offsets in UTF-16 code units), the text is drawn as plain runs with each range drawn highlighted; no ranges → plain text. In snippet text a highlight is a 3px-radius background `mark` colour (L `rgba(255,196,0,0.32)` / D `rgba(255,214,10,0.26)`) keeping the text colour.
- **SRCH-020** Title highlights use the terms of the **answered** query (not the text in the field): `queryTerms(answeredQuery)` (lowercased, split on whitespace, de-duplicated, first 8, longest first) matched case-insensitively anywhere in the title, overlapping/touching matches merged (`matchRanges`, 03). In the title a highlight has no background; it is a 2px underline in `mark-strong` (L `rgba(255,170,0,0.75)` / D `rgba(255,214,10,0.7)`) offset 3px below the baseline. No query → no highlights.
- **SRCH-021** `fuzzyMatch(query, text)` (fuzzy.ts) scores a title against a query exactly as in Data & formats; it is used by the archive stack's search (09/10 UI), which ranks archived cards by it: keep cards where it is non-null, sort by score descending then by stack order, then keep only those with `score >= 0.5 × topScore`; each result carries the matched `indices` (UTF-16 offsets into the title) for highlighting (10). With an empty/whitespace query every archived card is a result, in stack order, with no indices.

### Merge dialog (New session from context) — MERGE

- **MERGE-001** Entry points: the selection bar's **New session from context** (all selected session/terminal card ids), the card menu's `New session from context…` / `New session from ${n} contexts…` (09), and the search's start (SRCH-017). The board keeps only ids that are on the board and not terminals, maps each to `{ sessionId, title: cardTitle }` (in the order given), and does nothing if none remain. `focus` = the given focus trimmed, or none if empty (only the search passes one).
- **MERGE-002** The dialog is modal: a pointer-down on the backdrop does **not** close it; Esc (anywhere, even while typing in its fields) and **Cancel** close it. Closing while summaries run drops their results (the backend keeps going, 03). While it is open the selection bar is hidden and the board's shortcuts are off.
- **MERGE-003** Inside the dialog, the focus is normalized: trimmed, every whitespace run collapsed to one space, empty → none.
- **MERGE-004** On open, once, every source is marked `queued` and one summarize request is made for all of them with a new request id and the focus (03, at most three at a time there). Progress events for **this dialog's request ids only** update a source's phase (`queued` / `running` / `done` / `error`) and error, except a source already `done` is never changed by an event.
- **MERGE-005** Step 1, **gathering** (title `Gathering context`; subtitle with focus `Each session sums up what it did about “${focus}” in a forked copy. The originals stay untouched.`, without `Each session summarizes itself in a forked copy. The originals stay untouched.`): one row per source with its phase icon, title and state text — `Queued`, `Summarizing…`, `Ready`, `Failed` (red) — and under them `${s}s elapsed · Usually 10–30 seconds per session` where `s` = whole seconds since the gathering step started, ticking once a second (the `${s}s elapsed · ` prefix is omitted while `s` is 0). No field has focus.
- **MERGE-006** When the request answers, for each result: an error, or a summary that is empty after trimming, makes the source `error` with the error text or `The session returned an empty summary.`; otherwise the source is `done` with the summary and its title replaced by the result's title when non-empty. If the request itself throws, every source in it becomes `error` with the thrown message.
- **MERGE-007** As soon as no source is `queued` or `running`, the dialog moves to step 2, **review**, and never goes back (a later retry doesn't return to gathering).
- **MERGE-008** Review (title `New session from context`; subtitle `Review and edit the handoff, then tell the new session what to do.`): a wrapping row of source chips, each with its phase icon (a skipped source shows the queued icon) and title; every failed chip (skipped ones too) has a red tint; a failed, not-skipped chip has **Retry** and **Skip** buttons and its error text below (selectable); a skipped chip is at 50% opacity with `Skipped`.
- **MERGE-009** **Retry** re-runs just that source with a new request id (back to `queued`, error cleared, un-skipped) and the same focus; while any source is pending the session can't start.
- **MERGE-010** **Skip** marks a failed source skipped: it no longer blocks starting and isn't in the context or the parents.
- **MERGE-011** The **Context** field holds the generated handoff (Data & formats) and follows every change of the sources (new summaries, skips) until the user edits it. Editing marks it edited and shows a `Reset to generated` link beside the label; clicking it discards the edits (back to the generated text, following again) without moving focus.
- **MERGE-012** If the context was edited when a retry answers, the retry's successful summaries are appended to the edited text instead: `trimEnd(context)` + for each, `\n\n## Session ${k} — ${title}\n${summary.trim()}` where `k` continues from the number of sources already `done` before this answer (1-based), and title = the result's title or the source's.
- **MERGE-013** **What should the new session do?** (3 rows, user-resizable vertically) takes focus when the review step appears; placeholder `e.g. Combine both approaches and implement …`. **Name** is prefilled with the default name (Data & formats) and editable.
- **MERGE-014** **Start session** is enabled only when: in review, not already starting, nothing pending, no failed-and-not-skipped source, and the context or the instruction is non-blank.
- **MERGE-015** Starting (button or ⌘Return/Ctrl+Return from anywhere in the dialog; ignored when not enabled): the button shows a spinning `Loader2` and disables; the session is created with `name = trimmed name` or, if blank, the default name; `initialPrompt` per Data & formats; `parents` = the `done`, not-skipped sources in source order.
- **MERGE-016** If creating fails, the footer shows the error in red (replacing the hint) and the dialog stays open and usable again. If it succeeds: the card is placed at expanded size (720×750) below the source cards (`spotBelow` over the source cards still on the board; with none, `spotForNewSession`, 09), then `findFreeSpot` avoiding cards, notes, views and the dock; the dialog closes; the board selection is cleared; the board pans to show it; the session's **terminal modal opens** (unlike the prompt bar).
- **MERGE-017** Footer hint, first match wins: the start error; `Retry or skip the failed sessions to continue.` while a source blocks; in review `[⌘][↩] to start`; else nothing.

### Ink — INKUI

- **INKUI-001** The ink dock is always shown on the board: a vertical frosted pill 16px from the board's left edge with its bottom 58px above the board's bottom (8px above the 34px zoom pill whose bottom is at 16px). Top to bottom: Pointer, Bubble and Mic (BUBBLE-001, BUBBLE-040; added after 0.9.4), a separator, Pen, Shape, Arrow, Text, Sticker, Add a picture…, Eraser, a separator, Colour, Clear all. Since 1.0.9 Shape (in the Rectangle's place), Arrow, Text, Sticker and Add a picture… make the shapes and links of spec 14 rather than ink; strokes drawn before stay ink. Tooltips `Pointer (Esc)`, `Ask Claude here (B)`, `Speak to Claude here (M)`, `Pen (P)`, `Arrow (A)`, `Rectangle (R)`, `Text`, `Eraser (X)`, `Ink colour`, `Clear all ink`; accessible names `Pointer`, `Pen`, `Arrow`, `Rectangle`, `Text`, `Eraser`, `Ink colour: ${Name}`, `Clear all ink`; toolbar name `Ink`. The current tool's button is shown pressed. Pressing the dock never moves keyboard focus.
- **INKUI-002** Tools are chosen from the dock or with the board keys `B` (bubble), `M` (mic), `P` (pen), `A` (arrow), `R` (rectangle), `X` (eraser) (no ⌘/Ctrl/Alt; Shift allowed; with the board's usual guards — not while typing, no modal/overlay). Text has no key. The tool is per board mount and starts as Pointer.
- **INKUI-003** Choosing any tool first finishes a label being typed (INKUI-013); choosing a tool other than Pointer also deselects the selected stroke.
- **INKUI-004** Colours: graphite, red, orange, green, blue, purple (names `Graphite`, `Red`, `Orange`, `Green`, `Blue`, `Purple`). The colour button shows a dot in the current colour and toggles a palette to its right; picking a colour sets it, closes the palette, remembers it for next time (app-wide, across boards and restarts; key `nodus.inkColor`), and — if a stroke is selected and has a different colour — recolours that stroke (one undoable change). The first-ever colour (or an unreadable/unknown stored value) is red. The palette closes on a pointer-down anywhere outside the colour button and the palette.
- **INKUI-005** **Clear all ink** is disabled when the board has no ink; otherwise it finishes any label, deselects, removes all strokes as one undoable change, and shows the toast `Cleared the ink. Press ⌘Z to bring it back.` (`Ctrl+Z` off macOS).
- **INKUI-006** Pointer input for ink is seen **before** anything on the board (capture phase on the window): a press counts only when it is the primary button of the primary pointer, Space isn't held (INKUI-020), it lands on the board's surface — the canvas pane, a card, note, view, section, edge or the ink layer itself — not on the title bar, the dock, zoom controls, minimap, action rail, prompt bar column or any modal, and not on the label being typed. Otherwise the board handles it normally.
- **INKUI-007** When ink takes a press it stops it from reaching the board (no card drag, no selection box, no click, no focus change under it), finishes any label being typed, blurs whatever has keyboard focus inside the board (a note being written, a terminal) so Esc comes to the board, and holds the pointer until it lifts (moves and the release are followed wherever the pointer goes). The click that follows a taken press is swallowed.
- **INKUI-008** Pen: the stroke starts at the press point with the current colour and size `sizeAt(zoom)` (INKUI-010); every pointer move (with all coalesced intermediate samples) adds the point if it is at least 1.5 screen px from the last kept point; the live stroke is drawn while drawing; on release the points are simplified (Douglas–Peucker, tolerance `0.5 / zoom`, i.e. half a screen pixel) and the stroke is added (one undoable change). A press without movement makes a dot.
- **INKUI-009** Arrow: from the press point to the current pointer (straight); while drawing, before the first move it shows as a dot; on release it is discarded when its on-screen length is under 8px, otherwise added.
- **INKUI-010** Ink keeps the thickness it had on screen when drawn: `size = round2(clamp(1 / zoom, 0.5, 4))` (rounded to 2 decimals) at the moment the stroke/label starts; line width `3 × size`, arrow head `14 × size`, label font `16 × size` with line `20 × size`, all in board units (so at the drawing zoom a line is ~3 screen px and a label ~16px).
- **INKUI-011** Eraser: everything within `6 / zoom` board units (6 screen px) of the pointer's path is marked (also between move events, sampled along the segment) and dims to 25% opacity while dragging; on release the marked strokes are removed as one undoable change (deselecting the selected one if removed). Nothing marked → nothing changes.
- **INKUI-012** A pointer cancel (e.g. the system takes the pointer) during a pen/arrow/eraser/move gesture discards it entirely.
- **INKUI-013** Text: a press places an empty label input whose text line is vertically centred on the press point (top = `y − 10 × size`), in the current colour and size, focused, placeholder `Label` (name `Label`). Return or losing focus finishes it: trimmed text non-empty → a text stroke is added (one undoable change); empty → nothing. Esc discards it (a new label is not added; an edited label stays as it was) and doesn't reach the board. Pressing elsewhere on the board with the Text tool finishes the current label and starts a new one.
- **INKUI-014** Double-clicking a label with the Pointer tool edits it in place (same input, prefilled, its colour and size, the original hidden meanwhile): finishing with changed text updates it, with emptied text deletes it, with unchanged text does nothing (each change undoable); Esc leaves it unchanged.
- **INKUI-015** Pointer tool: a press on a stroke (topmost; labels before lines; within 6 screen px of a line, or anywhere in a label's box) selects that stroke, deselects every selected card, note and view, and starts a move; once the pointer has travelled 3 screen px the stroke follows it; on release a moved stroke is committed at its new place (one undoable change). A press not on a stroke deselects the selected stroke (if any) and goes to the board as usual. Strokes are hit before cards, so ink over a card takes the press.
- **INKUI-016** With the Pointer tool and no button held, hovering over a stroke on the board shows the move cursor over the whole board surface (checked once per animation frame).
- **INKUI-017** Cursors over the board surface while a tool is out override the cards' own: Pen/Arrow/Rectangle → crosshair; Text → I-beam; Eraser → a 20×20 circle (radius 7, white fill at 65%, black outline at 70%, 1.5px) with its hotspot at the centre (fallback: cell); Bubble and Mic → their dock icons (BUBBLE-005). Switching away from Pointer drops the move cursor.
- **INKUI-018** While a tool other than Pointer is out, views' live pages ignore the pointer, so presses over a view go to the ink.
- **INKUI-019** Double-clicking the board while a tool other than Pointer is out does nothing (no new session). With the Pointer tool, double-clicking a non-label stroke does nothing either (no new session, no card opened).
- **INKUI-020** Holding Space (when focus isn't in a text field) makes ink ignore presses, so the board pans; releasing Space or the window losing focus ends it.
- **INKUI-021** ⌘Z (Ctrl+Z) on the board undoes the last ink change; ⇧⌘Z (Ctrl+Shift+Z) redoes. Only ink has undo on the board: when there is nothing to undo/redo the key is left alone. Undo first finishes a label being typed. Undo and redo deselect the stroke. Up to 100 changes are kept (the oldest dropped); a new change clears the redo list. History is per board: switching project clears it, the selection and any label. The board's guards apply (not while typing in a field, no modal/overlay/terminal/preview/agent viewer/diff; no Alt; ⌘ and Ctrl not both).
- **INKUI-022** Backspace or Delete on the board (no ⌘/Ctrl/Alt) deletes the selected stroke (one undoable change) instead of archiving/deleting selected cards; with no stroke selected the board's own handling runs.
- **INKUI-023** Esc on the board: with a tool other than Pointer out, returns to Pointer and does nothing else; else with a stroke selected, deselects it and does nothing else; else the board's Esc runs (close the archive panel, clear the selection, 09).
- **INKUI-024** Every change (add, move, recolour, erase, delete, clear, undo, redo, label edit) updates the board's ink at once and saves it (`board:saveInk`, 04).
- **INKUI-025** Ink draws above every card (selected and expanded included), note, view, section and edge, and below the open archive panel and its results (RAISED_Z 1500). Within the ink: lines in stored order, then the live stroke, then the selection outline, then labels (labels always above lines), then the label input.
- **INKUI-026** The selected stroke shows a dashed accent rectangle around its box (INKUI box + `6 × size` padding, corner radius `4 × size` board units), 1.5 screen px wide, dashes 5 / 4 (non-scaling); it follows the stroke while moving.
- **INKUI-027** Rectangle (added after 0.9.4): from the press point to the current pointer, the two points being opposite corners; holding Shift while dragging makes it a square as wide as the longer side, on the pointer's side of the press. While drawing, before the first move it shows as a dot; on release it is discarded when its diagonal on screen is under 8px, otherwise added (one undoable change). It is drawn as an outline only (no fill), width `3 × size`, round joins, the path starting and ending mid-way along the top edge so every corner is a join. It is hit (pointer tool, eraser) within the usual reach of any of its four sides, not in its hollow inside, so a press inside it goes to the board. Its box is the corners' box padded by half the line width, as a pen's.
- **INKUI-028** Ink on a server board (hosted Nodus, added after 1.0.12; SRV-184). INKUI-024's saves send only what changed, a stroke at a time: `board:add-ink` with the strokes added, moved, recoloured or relabelled (one with an id already there replaces it in place), and `board:remove-ink` with the ids erased, deleted or cleared, each on top of the board's ink as the server has it, so two people drawing at once keep both. What changed is found against the ink as this window showed it, so strokes another person drew since are never sent as gone. ⌘Z and ⇧⌘Z take back or redo only what that step changed, on top of the ink as it is now (a stroke put back goes on top), so strokes someone else drew meanwhile stay. Each new stroke records its drawer (`createdBy`, SRV-183). Local boards still save the whole list (`board:saveInk`) and undo by whole lists, as before.

### Asking Claude on the spot — BUBBLE

Added after 0.9.4; nothing in 0.9.4 to match. The pointer tool keeps 0.9.4's clicks (BOARD-048, BOARD-049).

- **BUBBLE-001** With the bubble tool out, a press on the board's surface (INKUI-006: over cards, notes and views too, not over the HUD, the bar or the bubble itself) is taken as an ink press is (INKUI-007) and opens the bubble there, its field focused. The tool stays out after a bubble is sent.
- **BUBBLE-002** A press while a bubble is open: a bubble with words in it stays where it is and takes the keyboard back (nothing is lost); an empty one moves to the new spot.
- **BUBBLE-003** Esc in the field closes the bubble and drops its words; the board takes the keyboard. When the field loses the keyboard (a press on the bar, the HUD, anything else) or the tool changes, an empty bubble closes and one with words stays, unfocused, until it's sent or Esc'd; a press on it gives it the keyboard again. Focus is checked each frame, since the field's blur doesn't fire for a focus given from outside.
- **BUBBLE-004** Placement, in canvas pixels (it doesn't scale with zoom; it follows the spot as the board pans or zooms): 360px wide; above and to the right of the spot, its bottom-left corner 6px right of and 6px above it. With under 76 + 12px of room above the spot it opens below (top-left corner 6px right and below); with under 360 + 18px of room to the right it opens to the left (the right corner 6px left of the spot, never past 12px from the board's left edge). The corner nearest the spot has a 4px radius, the others 16px, so it reads as a speech bubble.
- **BUBBLE-005** Cursors over the board surface while the tool is out are the tools' dock icons as 24×24 image cursors, white with a dark outline (black at 85%) so they read on either board: the bubble's `message-circle-more` (its three dots dark), hotspot at the tip of its tail (2.5, 21.5); the mic's `mic`, its stand haloed in white, hotspot at the foot of the stand (12, 22). The arrow where an image cursor can't be made.
- **BUBBLE-006** Body: frosted (`--bg-elevated` at 92%, blur 24, saturate 1.8), `shadow-lg`, padding 8px 12px, gap 6px. The field: 14px/20px text, auto-grows 1–5 lines, placeholder `Ask Claude, jot a note, or describe a view`. Under it one line (12px/16px, tertiary): Sparkles 12, `Claude decides: a session, a note or a view` (ellipsis), and a `↵` key cap once something is typed. Nothing in it picks the kind or the model.
- **BUBBLE-007** An 8px accent dot with a 3px ring of its colour at 25% marks the spot. The bubble and its dot fade in and rise 4px over 120ms ease-out.
- **BUBBLE-008** The bubble, the markers (BUBBLE-030) and the mic's pill (BUBBLE-043) draw above the cards, the ink, the dock and the bar, below search, the merge dialog, the confirm and views' dialogs; each occludes what's under it.
- **BUBBLE-013** Return (no modifiers) sends it at once (BUBBLE-030); with an empty field it does nothing. ⇧Return is a new line. While an input method composes, keys are its own. No model runs while typing.
- **BUBBLE-016** Keys in the field stay out of the board's shortcuts (a focused field is typing, BOARD-156); ⌘K still opens search.
- **BUBBLE-020** Where it goes is asked with a headless `claude -p`, as a capture's guess is (CAP-069): `--model haiku`, thinking off (`--settings {"alwaysThinkingEnabled":false}`), `--tools ""`, `--strict-mcp-config`, `--no-session-persistence`, `--json-schema` (kind `session|note|view`, model `haiku|sonnet|opus`, reason), `--output-format json`, the system prompt `nodus_capture::bubble::SYSTEM_PROMPT` (verbatim), run in the temp folder with stdin closed.
- **BUBBLE-021** Its prompt: `Project: <name>` (one line, 120 units at most), a blank line, then `The bubble:` and the text, trimmed and cut to 4000 UTF-16 units, between `"""` lines.
- **BUBBLE-022** The answer is `structured_output` of a reply that isn't an error. A kind other than note or view is a session; a model it doesn't know is Sonnet; the reason is flattened to one line of at most 120 units (not shown). Notes and views ignore the model.
- **BUBBLE-023** Each ask runs on its own, any number at once, apart from the capture panel's guesses; one that hasn't answered in 30 s is stopped (SIGKILL).
- **BUBBLE-024** With no answer (no `claude` installed, an error, garbage, the time running out) it's a session on Claude Code's own model.
- **BUBBLE-030** Sending: the bubble closes and the board takes the keyboard back (if the bubble still had it). A marker waits on the spot until Claude has decided: the dot (BUBBLE-007) and, 12px right of it and level with it, a frosted pill 28px tall (radius 14, `shadow-md`, 12px secondary text) with a spinner and the text's first line, at most 240px wide (ellipsis); it fades in sliding 4px left. Each ask holds its board until what it makes is made, so it lands even if the project changes meanwhile.
- **BUBBLE-031** What's made has its top left exactly on the spot (board units, rounded), over whatever is there, and the board doesn't move to show it. A session is started as the bar starts one (BOARD-133, BOARD-134: expanded, auto mode, the text stuck on it as a note, or left as a note if it can't start), on the model Claude picked: the card keeps it in `model` (with the node's passthrough fields, so a newer version's `model` of another shape is kept as it was) and every launch passes `--model <alias>` (SESS-121's arguments; create, open and send alike), since a resume doesn't keep the model. With no answer the card has no `model`.
- **BUBBLE-032** A note is a yellow sticky note of the text (BOARD-137's note); a view is created as the bar's `/view` is (BOARD-136), with the same toast when it can't be.
- **BUBBLE-040** With the mic tool out, a press on the board's surface (as BUBBLE-001) starts listening for that spot, through its own mic: the window's one listening slot and the voice flow of spec 05 (VOICE-025…033), with the bar's key and error handling (no key opens Settings).
- **BUBBLE-041** The next press on the board's surface while it records stops it and sends; presses while the mic opens or writes down do nothing. Esc (VOICE-033) or the pill's × cancels, and nothing is sent.
- **BUBBLE-042** What was heard, trimmed, is asked at the spot the recording started, exactly as a sent bubble (BUBBLE-030…032); nothing heard sends nothing.
- **BUBBLE-043** While it opens, records or writes down, a pill on the spot as a marker's (BUBBLE-030), the dot red (`--status-waiting`) while recording: the voice status (VOICE-038, VOICE-039: red dot, clock and meter, or the spinner and `Transcribing…`), `Opening the mic…` or `Click to send`, and a 20px × (tooltip `Cancel (Esc)`) except while writing down. A voice notice (VOICE-040) shows under it, 10px right of the spot and 20px down.
- **BUBBLE-044** Leaving the mic tool cancels a recording (or the mic opening); one being written down still goes.

### Selection bar — SEL

- **SEL-001** The selection bar shows in the prompt bar's column (above the voice notice and the row, below the toast, BAR-002) whenever at least one session-type card (Claude session or terminal, live or a selected archived result) is selected, and the merge dialog isn't open. Selecting only notes or views shows no bar.
- **SEL-002** It reads `${count} selected` (count = selected session-type cards), then a divider, then the actions.
- **SEL-003** **New session from context** (primary, `Sparkles` icon) opens the merge dialog for the selection (MERGE-001); disabled when no selected card is a Claude session (only terminals).
- **SEL-004** **Group** (`Group` icon, tooltip `Group into a section (⌘G)`, `Ctrl+G` off macOS) groups the selected cards into a section (09); disabled when every selected card is archived.
- **SEL-005** **Archive** (`Archive` icon, tooltip `Archive selected (⌫)`, `Del` off macOS) archives the selected live Claude sessions (09: asks first for running ones; terminals and archived ones are skipped); disabled when none is a live Claude session.
- **SEL-006** × (tooltip `Clear selection (Esc)`) clears the whole board selection (cards, notes, views).

## Data & formats

### Submission (prompt bar → board)

```
Submission =
  | { kind: 'session'; text }                 // text = trimmed field text
  | { kind: 'into'; sessionId; text }
  | { kind: 'view'; text }
  | { kind: 'note'; text; color: FolderColor } // blue|purple|pink|red|orange|yellow|green|graphite
```
The board answers a boolean: `true` = done (clear the bar), `false` = keep the words.

### Slash commands (compose.ts)

| target | command | description |
| --- | --- | --- |
| `view` | `/view` | `A live page Claude keeps up to date` |
| `note` | `/note` | `A sticky on the board, nothing runs` |

- `matchCommands(text)`: if `text` doesn't match `^/\S*$` (JS `\S`: not Unicode whitespace) → `[]`; else the commands (in table order) whose `command` starts with `text.toLowerCase()`.
- `takeCommand(text)`: `^/(view|note)\s+([\s\S]*)$` with the `i` flag; match → `{ target: group1.toLowerCase(), rest: group2 }` (the `\s+` is greedy, so `rest` starts at the first non-whitespace character or is empty); else `null`.

### Context template (compose.ts `withContext`) — verbatim

```
ContextItem =
  | { kind: 'session'; title; branch?; lastReply?; transcript? }
  | { kind: 'note'; text }
  | { kind: 'view'; title; description }
```
- `withContext([], prompt)` = `prompt` unchanged.
- Otherwise: `For context, I selected these on my board:` + `\n\n` + items described and joined with `\n\n` + `\n\n---\n\n` + `prompt`.
- quote(text): `text.trim()` split on `\n`; each line → `> ${line}`, an empty line → `>`; joined with `\n`.
- describe(session): lines joined with `\n`: `## Session “${title}”` (U+201C/U+201D); then `Branch: ${branch}` if branch is non-empty; then `Its last reply:\n${quote(lastReply)}` if `lastReply.trim()` is non-empty, else `It has no replies yet.`; then `Its whole conversation, if you need more: ${transcript}` if transcript is non-empty.
- describe(note): `## Note\n${text.trim()}`.
- describe(view): `## View “${title}”\n${description.trim()}`.

Example (from the tests): items = session `Fix cart rounding` (branch `fix/cart-rounding`, last reply `All 6 tests pass.\n\nDone.`, transcript `/t/a.jsonl`), session `New session` (nothing else), note ` EUR rounding: ask finance `, view `Tests` / `Test results by file`; prompt `Why does cart.test.js fail?` →

```
For context, I selected these on my board:

## Session “Fix cart rounding”
Branch: fix/cart-rounding
Its last reply:
> All 6 tests pass.
>
> Done.
Its whole conversation, if you need more: /t/a.jsonl

## Session “New session”
It has no replies yet.

## Note
EUR rounding: ask finance

## View “Tests”
Test results by file

---

Why does cart.test.js fail?
```

Bar session name (when any context item): `truncate(text.split('\n')[0], 64)`.
`truncate(s, max)` (layout.ts): `s` if `len(s) <= max`, else `s.slice(0, max − 1).trimEnd() + '…'` (lengths in UTF-16 code units).

### Merge dialog formats (MergeDialog.tsx) — verbatim

- `DEFAULT_INSTRUCTION` = `Read the context above, confirm briefly what you understand, then wait for my instructions.`
- `section(i, title, summary)` = `## Session ${i} — ${title}\n${summary.trim()}` (em dash U+2014, spaces around it).
- `compose(sources, items, focus)`: `ok` = sources in order whose item is `done`, has a summary, and isn't skipped. None → `''`. Else with `n = ok.length`:
  - head = `I'm continuing work from ${n} earlier Claude Code session${n === 1 ? '' : 's'} in this repository. Here is ` + (focus ? `what ${n === 1 ? 'it' : 'each'} did about "${focus}":` : `a handoff summary of ${n === 1 ? 'it' : 'each'}:`) — straight double quotes around the focus here (the subtitle uses curly ones).
  - result = `[head, section(1, title₁, summary₁), …, section(n, …)].join('\n\n')`, titles being the item titles (result titles once done).
- `defaultName(sources, focus)`: with focus → `truncate('From search: ' + focus, 64)`; else `titles = sources.map(s => truncate(s.title, 28))`; `joined` = `titles.join(' + ')` when ≤ 2, else `${titles[0]} + ${titles[1]} + ${titles.length − 2} more`; result `truncate('From: ' + joined, 64)`.
- `initialPrompt`: `body = instruction.trim() || DEFAULT_INSTRUCTION`; `context.trim()` non-empty → `${context.trim()}\n\n---\n\n${body}`; else `body`.
- Summarize call: `sessions.summarize(projectId, ids, requestId = random UUID, focus?)` → `SummaryResult[] { sessionId, title, summary, error? }`; progress event `ev:summarize-progress { requestId, sessionId, phase: 'queued'|'running'|'done'|'error', error? }` (03).

### Search data (SessionSearch.tsx)

- `SessionSearchHit { sessionId, matches, snippets: SearchSnippet[] }`; `SearchSnippet { kind: 'prompt'|'reply'|'tool', tool?, file?, agent?, text, ranges: [start, end)[], timestamp }` (03). Ranges and the title ranges are UTF-16 code-unit offsets.
- `DEBOUNCE_MS = 120`, `RECENT = 30`. Status labels: working `Working`, waiting `Needs you`, done `Done`.
- Opening an archived result passes `PreviewFind { query: trimmed query, snippet?: hit.snippets[0] }` to the transcript preview (12).

### Fuzzy scoring (fuzzy.ts) — exact

Constants: `MATCH = 1`, `CONSECUTIVE = 1.6`, `WORD_START = 1.2`, `TEXT_START = 0.8`, `GAP = 0.06`, `LEADING_GAP = 0.02`. All arithmetic in IEEE f64. Strings are indexed by **UTF-16 code unit**; lowercasing is JS `String.prototype.toLowerCase` (full Unicode, locale-independent).

- `isSeparator(ch)`: `ch` is JS-`\s` whitespace or one of `` - _ / . : , ; ( ) [ ] { } ' " ` ``.
- `isUpper(ch)`: `ch !== ch.toLowerCase()`.

```
fuzzyMatch(query, text):
  q = lowercase(query) with every whitespace run removed
  if q == "": return { score: 0, indices: [] }
  t = lowercase(text); n = len(q); m = len(t)
  // cheap rejection: q must be a subsequence of t
  k = 0; for j in 0..m while k < n: if t[j] == q[k]: k += 1
  if k < n: return null
  bonus[j] for j in 0..m (default 0):
    j == 0                                   -> WORD_START + TEXT_START (2.0)
    isSeparator(text[j-1])                   -> WORD_START (1.2)
    isUpper(text[j]) && !isUpper(text[j-1])  -> WORD_START * 0.8 (0.96)
  for i in 0..n:
    row[0..m] = -inf; back[0..m] = -1; prev = score[i-1]
    run = -inf; runAt = -1
    for j in i..m:
      if i > 0 && j >= 2 && prev[j-2] + GAP*(j-1) > run:     // strictly greater
        run = prev[j-2] + GAP*(j-1); runAt = j-2
      if t[j] != q[i]: continue
      if i == 0: row[j] = MATCH + bonus[j] - LEADING_GAP*j; continue
      chained = prev[j-1] + CONSECUTIVE
      jumped  = run - GAP*j                                   // a gap of g chars costs GAP*g
      if chained >= jumped && chained > -inf: row[j] = chained + MATCH + bonus[j]; back[j] = j-1
      else if jumped > -inf:                  row[j] = jumped + MATCH + bonus[j];  back[j] = runAt
    score[i] = row; from[i] = back
  end = first j with the strictly largest score[n-1][j]; if none (all -inf): return null
  indices[n-1] = end; walk i = n-1..0: indices[i] = j; j = from[i][j]
  return { score: best - m*0.005, indices }    // shorter titles win ties
```
Note `bonus` reads the **original** `text` while matching reads the lowercased `t`; `m` is the lowercased length.

Stack ranking (BoardView 571–584): `hits = archivedInStackOrder.map((c, rank) => fuzzyMatch(q, cardTitle(c)))` non-null, sorted by `score` desc then `rank` asc; `floor = hits[0].score × 0.5`; keep `score >= floor`.

### Ink (ink.ts, InkLayer.tsx) — exact

- Stored stroke (04 owns persistence): `InkStroke { id: uuid, kind: 'pen'|'arrow'|'text'|'rect', color: 'graphite'|'red'|'orange'|'green'|'blue'|'purple', points: number[] (flat x, y pairs, board units, each rounded to 0.1), size: number, text?: string }`. A pen has ≥ 1 point, an arrow exactly 2 (tail, tip), a rectangle exactly 2 (opposite corners, in the order drawn; INKUI-027), a label 1 (its top-left corner). Versions without it (0.9.4, and 1.0.x before it) drop `rect` strokes when they clean the ink, so a board saved by one of them loses its rectangles.
- Constants: `PEN_WIDTH = 3`, `ARROW_HEAD = 14`, `LABEL_FONT = 16`, `LABEL_LINE = 20` (board units at size 1); `HISTORY = 100`; `HIT_PX = 6`; `MIN_STEP_PX = 1.5`; `MIN_ARROW_PX = 8`; move threshold 3 screen px; colour key `nodus.inkColor` (renderer localStorage), default `red`.
- `r1(n) = Math.round(n × 10) / 10` (JS round: .5 rounds up, also for negatives: use `floor(n·10 + 0.5) / 10`). `flatten(pts)` = r1 of every coordinate.
- `sizeAt(zoom) = Math.round(min(4, max(0.5, 1 / zoom)) × 100) / 100`.
- `segmentDistance(p, a, b)`: `d = b − a`, `len = d·d`; `t = len ? clamp(((p − a)·d) / len, 0, 1) : 0`; `hypot(p − a − t·d)`.
- `penPath(pts)`: none → `''`; one → `M${x0} ${y0}h0.01` (a dot drawn by the round caps); two → `M${x0} ${y0}L${x1} ${y1}`; more → `M${x0} ${y0}` + for i in 1..n−1: `Q${xi} ${yi} ${r1((xi + xi+1)/2)} ${r1((yi + yi+1)/2)}` + `L${xlast} ${ylast}` (quadratic curves through the midpoints). Numbers print as JS shortest round-trip (`10`, `5`, `0.5`, never `10.0`).
- `arrowPath(a, b, head)`: `angle = atan2(by − ay, bx − ax)`; `wing(turn) = ${r1(bx − head·cos(angle + turn))} ${r1(by − head·sin(angle + turn))}`; path `M${ax} ${ay}L${bx} ${by}M${wing(−0.45)}L${bx} ${by}L${wing(0.45)}`; drawn with `head = 14 × size`.
- `simplify(pts, tol)`: fewer than 3 → unchanged. Keep first and last; stack `[(0, last)]`; pop `(from, to)`; find `i` in `from+1..to−1` with the greatest `segmentDistance(pts[i], pts[from], pts[to])` (first strictly greatest; start `far = 0`, `worst = −1`); if `worst > 0 && far > tol` keep it and push `(from, worst)` then `(worst, to)`. Return kept points in order.
- `labelWidth(text, size)`: width of `text` set in the display font (`--font-display`), weight 600, `16 × size` px; with no text measurer: `len(text) × 16·size × 0.56`.
- `strokeBox(s)`: text → `{ x, y: first point, w: labelWidth(text ?? '', size), h: 20 × size }`; else `pad = (arrow ? 14 : 1.5) × size`, box of all points grown by `pad` on every side.
- `hitStroke(s, p, tol)`: outside the box grown by `tol` → false; text → true; `reach = tol + 1.5 × size`; one point → `dist(p, pt) <= reach`; any segment with `segmentDistance <= reach` → true; arrow → `dist(p, tip) <= 14 × size`; else false.
- `strokeAt(strokes, p, tol)`: last-to-first over text strokes, then last-to-first over the others; first hit.
- `strokesAlong(strokes, a, b, tol)`: `steps = max(1, ceil(dist(a, b) / max(tol / 2, 0.5)))`; a stroke is hit if `hitStroke` at any `a + (b − a)·i/steps`, `i = 0..steps`. Returns the set of ids.
- `translate(s, dx, dy)`: every x `r1(x + dx)`, every y `r1(y + dy)`.
- Pen commit: `simplify(pts, 0.5 / zoomAtStart)` then flatten. Arrow: flatten `[start, last]`. Label: `{ kind: 'text', color, points: flatten([[x, y]]), size, text: trimmed }` with `y = pressY − 10 × size` for a new one.
- Hit tolerance: `6 / zoom` (current zoom). Pen point step: `distance(q, last) × zoomAtStart >= 1.5`. Arrow discard: `distance × zoomAtStart < 8`. Move start: `hypot(dx, dy) × zoomAtStart >= 3`.

### Other formats

- Voice clock `m:ss` (05 VOICE-036). Meter bars show bands `[3, 1, 0, 2, 4]` left → right (loudest-lowest band in the middle).
- Relative time in search rows: 10 CARD-041 (`just now`, `Nm ago`, `Nh ago`, `Nd ago`, else locale `Oct 3` / `Oct 3, 2025`).

## UI details

Token values used below (L / D):

| token | light | dark |
| --- | --- | --- |
| bg-elevated | `#ffffff` | `#2a2a2d` |
| bg-window | `#f5f5f4` | `#1e1e20` |
| bg-sunken | `rgba(0,0,0,0.035)` | `rgba(255,255,255,0.04)` |
| bg-hover | `rgba(0,0,0,0.05)` | `rgba(255,255,255,0.07)` |
| bg-pressed | `rgba(0,0,0,0.09)` | `rgba(255,255,255,0.12)` |
| bg-selected | `rgba(0,122,255,0.14)` | `rgba(10,132,255,0.24)` |
| text-primary | `rgba(0,0,0,0.88)` | `rgba(255,255,255,0.9)` |
| text-secondary | `rgba(0,0,0,0.56)` | `rgba(255,255,255,0.58)` |
| text-tertiary | `rgba(0,0,0,0.36)` | `rgba(255,255,255,0.36)` |
| text-on-accent | `#ffffff` | `#ffffff` |
| border | `rgba(0,0,0,0.1)` | `rgba(255,255,255,0.1)` |
| border-strong | `rgba(0,0,0,0.18)` | `rgba(255,255,255,0.18)` |
| separator | `rgba(0,0,0,0.07)` | `rgba(255,255,255,0.07)` |
| accent | `#007aff` | `#0a84ff` |
| accent-hover | `#0068d9` | `#3396ff` |
| focus-ring | `rgba(0,122,255,0.5)` | `rgba(10,132,255,0.55)` |
| status-working | `#ff9500` | `#ff9f0a` |
| status-done | `#007aff` | `#0a84ff` |
| status-waiting | `#ff3b30` | `#ff453a` |
| mark | `rgba(255,196,0,0.32)` | `rgba(255,214,10,0.26)` |
| mark-strong | `rgba(255,170,0,0.75)` | `rgba(255,214,10,0.7)` |
| folder-label-selected | `#0063e1` | `#0a84ff` |
| view-hue (bar only) | `#af52de` | `#bf5af2` |
| view-ink (bar only) | `#8944ab` | `#d9a3f7` |
| ink-graphite | `#1d1d1f` | `#f2f2f4` |
| ink-red | `#ff3b30` | `#ff453a` |
| ink-orange | `#ff9500` | `#ff9f0a` |
| ink-green | `#28cd41` | `#32d74b` |
| ink-blue | `#007aff` | `#0a84ff` |
| ink-purple | `#af52de` | `#bf5af2` |

Shadows (each is two layers, outer first):

| name | light | dark |
| --- | --- | --- |
| SH-SM | `0 1px 2px rgba(0,0,0,0.06)`, `0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3)`, `0 0 0 0.5px rgba(255,255,255,0.08)` |
| SH-MD | `0 4px 14px rgba(0,0,0,0.08)`, `0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35)`, `0 0 0 0.5px rgba(255,255,255,0.1)` |
| SH-LG | `0 18px 50px rgba(0,0,0,0.18)`, `0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5)`, `0 0 0 0.5px rgba(255,255,255,0.12)` |
| SH-MODAL | `0 30px 90px rgba(0,0,0,0.35)`, `0 0 0 0.5px rgba(0,0,0,0.2)` | `0 30px 90px rgba(0,0,0,0.6)`, `0 0 0 0.5px rgba(255,255,255,0.14)` |

"Frosted X%" below = background bg-elevated at X% alpha (L `rgba(255,255,255,X)` / D `rgba(42,42,45,X)`) over a backdrop blur; the blur radius and saturation are given per surface.

Easings: EASE-OUT = `cubic-bezier(0.2, 0.8, 0.2, 1)`; EASE-SPRING = `cubic-bezier(0.34, 1.4, 0.64, 1)`; `ease` = `cubic-bezier(0.25, 0.1, 0.25, 1)`. Durations: fast 120ms, med 220ms.

Fonts: UI = SF Pro Text (macOS system) / Segoe UI Variable Text, Segoe UI (Windows); Display = SF Pro Display / Segoe UI Variable Display; Mono = SF Mono, Menlo / Cascadia Mono, Consolas. Base text is UI 13px, line-height 1.4 (18.2px); text inherits line-height 1.4 of its own size unless stated. Text is not selectable unless stated. Icons are lucide, sizes in px, stroke 2 unless stated.

Spinner (`spin`): continuous rotation 360° per 0.9s, linear.

Reduced motion (system setting): every animation and transition here completes instantly (global rule sets durations to 0.01ms), spinners included.

Keyboard focus ring (global): a button focused from the keyboard draws a 2px `focus-ring` outline 2px outside its edge (ink dock buttons and swatches: 1px outside). Text fields here suppress it (they have their own styling or none).

Key cap (`kbd`, global board style): inline block, min-width 18px, padding 0 5px, margin 0 1px, radius 4px, border 0.5px border-strong, background bg-elevated, shadow `0 1px 0` border (L `rgba(0,0,0,0.1)` / D `rgba(255,255,255,0.1)`), UI font 11px, line-height 17px, centred, colour text-secondary.

Shared buttons (`.btn`, 08): inline-flex, centred vertically, gap 6px, height 28px, padding 0 12px, radius 6px, border 0.5px border-strong, background bg-elevated, SH-SM, UI 13px weight 500, no wrap, background transition 120ms `ease`; hover bg-hover; pressed bg-pressed; disabled 45% opacity and no pointer events. **Primary**: background accent, transparent border, text-on-accent; hover accent-hover (also while pressed). **Ghost**: transparent background, transparent border, no shadow; hover bg-hover; pressed bg-pressed. **Icon**: width 28px, no padding, content centred.

### Prompt bar

**Column** (`.sidekick`): see BAR-001/002. Width `max(300px, min(672px, 100% − 416px))`, bottom 20px, centred, column flex, gap 10px.

**Toast** (in the column): max-width `min(560px, columnWidth − 48px)`, padding 9px 14px, radius 10px, frosted 92% with blur 20px (no saturation boost), SH-LG, UI 12px text-primary; enters with the column rise (opacity 0 → 1, translateY 6px → 0, 220ms EASE-OUT).

**Row**: full column width, horizontal, items bottom-aligned, centred, gap 10px.

**Pill** (`.sidekick-pill`): horizontal, items bottom-aligned, gap 8px, min-height 48px, padding 7px 7px 7px 18px (left 8px while a token shows), radius 24px, frosted 90% (blur 24px, saturate 1.8), SH-MD; cursor I-beam on its own background.
- Open: preferred width 600px (closed 520px) and SH-LG; transitions width 220ms EASE-OUT, shadow 220ms `ease`.
- Recording: shadow = `0 0 0 1.5px status-waiting@55%` (L `rgba(255,59,48,0.55)` / D `rgba(255,69,58,0.55)`) + SH-LG.
- Children left → right: [view/note/session token] [status line | field] [hint] [send]. The slash menu and colour picker float above (below).

**Field**: fills the remaining width (min 0), min-height 34px, max-height 114px (5 × 20 + 14), padding 7px 0, no border, transparent, UI 14px, line-height 20px, colour text-primary, caret text-primary; placeholder text-tertiary; grows with content (and with a wrapping placeholder when empty); hidden while listening.

**Hint**: inline, no shrink, vertically centred in the pill, gap 4px, UI 12px text-tertiary, no wrap: key cap `/` (global kbd style) + ` view or note`.

**Send button**: 34×34 circle, no border; background accent, icon text-on-accent; `ArrowUp` 16px stroke 2.4; hover (enabled) accent-hover; pressed (enabled) scale 0.94; disabled background bg-pressed and icon text-tertiary; transitions background and scale 120ms `ease`; cursor arrow. Sending: `Loader2` 15px spinning.

**Status line** (listening): fills the field's place, height 34px, items centred, gap 8px, cursor arrow:
- recording: dot 8×8 circle status-waiting, blinking (opacity 1 → 0.3 at 50% → 1, 1.2s `ease-in-out`, infinite); clock min-width 28px, UI 12px weight 500 tabular figures, text-primary; meter: 5 bars, gap 2px, each 3×16 radius 2px, colour text-secondary, vertically scaled about its centre by `clamp(0.18, level_band × 1.4, 1)`, scale transition 70ms linear; the status group (dot, clock, meter) has gap 8px and colour text-secondary, UI 12px, no wrap;
- transcribing: `Loader2` 13px spinning + `Transcribing…` (UI 12px text-secondary), gap 8px;
- then a spacer, then the cancel button: 24×24 circle, no border, transparent, icon text-tertiary `X` 13px; hover background bg-hover and icon text-primary.

**Tokens**: inline-flex, no shrink, items centred, gap 6px, max-width 240px, height 28px, margin-bottom 3px (so a 28px token sits centred in a one-line pill), padding 0 3px 0 10px, radius 14px, UI 12px weight 500, no wrap, cursor arrow; label ellipsised; icon 13px. Enter animation 120ms EASE-OUT from opacity 0, scale 0.9.
- View: background view-hue@15% (L `rgba(175,82,222,0.15)` / D `rgba(191,90,242,0.15)`), text/icon view-ink (L `#8944ab` / D `#d9a3f7`); `LayoutDashboard` 13px.
- Note: padding-left 5px, background bg-hover, text text-primary; swatch button 18×18, radius 5px, no border, inner hairline `inset 0 0 0 0.5px rgba(0,0,0,0.2)`, fill = vertical gradient top → bottom of the colour's front hues: blue `#6dd0fc → #2da4f3`, purple `#c8a2f7 → #a77beb`, pink `#fba0c8 → #f074aa`, red `#ff9189 → #f76760`, orange `#ffc374 → #fba343`, yellow `#ffe27a → #f9cb45`, green `#93e19c → #5dcd6b`, graphite `#bdbdc2 → #9a9aa0` (same in both themes).
- Session (into): background bg-selected, text/icon folder-label-selected (L `#0063e1` / D `#0a84ff`); `CornerDownRight` 13px.
- Clear ×: 22×22 circle, no border, transparent, colour = token colour at 70% opacity; hover background bg-hover, 100% opacity; `X` 11px stroke 2.6.

**Popovers** (slash menu and colour picker): positioned with their left edge at the pill's left edge and their bottom 8px above the pill's top; radius 14px; frosted 96% (blur 24px, saturate 1.8); SH-LG; cursor arrow; above the pill's contents; appear with opacity 0 → 1 and translateY 6px → 0 over 120ms EASE-OUT.
- Slash menu: vertical, gap 2px, width 340px, padding 5px (two options: 92px tall). Option: 3 columns 28px / 52px / rest, column gap 8px, items centred, padding 6px 8px, radius 8px, text-primary; highlighted: background bg-selected. Icon cell 28×28, radius 7px, background bg-hover, content centred: view → `LayoutDashboard` 15px in view-ink; note → a 14×14 chip, radius 3px, the current note colour's gradient, inner hairline `rgba(0,0,0,0.2)`. Command: mono 12px weight 500. Description: UI 12px text-secondary, one line, ellipsised.
- Colour picker: padding 8px 10px around a swatch row: horizontal, gap 6px, padding 5px 8px 5px 30px (the 30px left indent is kept), 8 swatches in order blue, purple, pink, red, orange, yellow, green, graphite: each 16×16 circle, gradient as above, inner hairline `inset 0 0 0 0.5px rgba(0,0,0,0.18)`, tooltip the capitalised name (`Blue`…), name the id; hover scale 1.18 over 120ms EASE-OUT; current: rings `0 0 0 2px bg-elevated`, `0 0 0 3.5px border-strong` outside the hairline and a white `Check` 10px stroke 3.5 centred. Total ≈ 228×42px.

**Mic button** (`.sidekick-orb`): 52×52 circle, no border, frosted 90% (blur 24px, saturate 1.8), SH-MD, icon text-secondary; hover (enabled) icon text-primary; pressed (enabled) scale 0.94; transitions background, colour and scale 120ms `ease`; cursor arrow. Disabled (opening/transcribing): no hover/press effect, otherwise unchanged.
- Recording: background status-waiting, icon `#ffffff`, shadow = `0 0 0 (3.6 + level × 14.4)px status-waiting@20%` (L `rgba(255,59,48,0.2)` / D `rgba(255,69,58,0.2)`) + SH-MD; the halo's spread transitions 80ms linear.

**Voice notice** (05 content; placement here): horizontal, items centred, gap 8px, max-width 440px, min-height 36px, padding 6px 6px 6px 12px, radius 18px, frosted 86% (blur 20px, saturate 1.6), SH-MD, UI 12px; enters with the column rise (220ms EASE-OUT); `AlertCircle` 14px in status-waiting; text fills, text-primary, line-height 1.35; action buttons are `.btn` at height 24px, padding 0 8px, gap 4px, 12px; dismiss is the 24px circle described for the status line's cancel.

### ⌘K search

**Modal** (08's Modal): backdrop covers the window, L `rgba(0,0,0,0.28)` / D `rgba(0,0,0,0.45)`, blur 3px, fades in 220ms EASE-OUT. Panel: radius 18px, background bg-elevated, SH-MODAL, clips its content, enters with opacity 0 → 1 and `translateY(8px) scale(0.985)` → none over 220ms EASE-OUT; vertical layout.

**Search panel** size/placement: width `min(680px, windowWidth − 48px)`; top edge at 11% of the window height (horizontally centred); max-height `89vh − 48px`; height = content up to that, the results list scrolling.

**Field row**: horizontal, items centred, gap 10px, padding 14px 16px, bottom border 1px separator. Icon 16px text-tertiary: `Search` stroke 2.2, or `Loader2` spinning while loading. Input: fills, no border/outline, transparent, UI 15px (line 21px), text-primary, placeholder `Search what every session said and did` in text-tertiary, spellcheck off. Then a key cap `esc`.

**Results**: fills, scrolls vertically, padding 6px.
- Section header `Latest sessions`: padding 6px 10px 4px, UI 11px weight 600 text-tertiary.
- Row: horizontal, gap 10px, padding 8px 10px, radius 10px, cursor arrow. Highlighted: background bg-hover. Marked: background bg-selected (wins over highlighted).
- Check box: 16×16, top margin 1px, radius 5px, border 1.5px border-strong, transparent; hidden (opacity 0) unless the row is highlighted or marked, opacity fading 120ms `ease`; marked: border and fill accent with a white `Check` 11px stroke 3.
- Body: vertical, gap 3px, fills.
- Header line: horizontal, items centred, gap 7px. Status dot 7×7 circle: done fill status-done; working fill status-working with ring `0 0 0 2px working@22%` breathing to `0 0 0 3.5px working@10%` at 50% (1.6s `ease-in-out` infinite; L `rgba(255,149,0,…)` / D `rgba(255,159,10,…)`); waiting fill status-waiting with a pulse `0 0 0 0 waiting@55%` → `0 0 0 8px transparent` (1.1s `ease-out` infinite). Archived: `Archive` 12px stroke 2.2 text-tertiary. Title: fills, one line, ellipsised, UI 13px weight 600 text-primary; highlights per SRCH-020. Count and time: UI 11px text-tertiary, tabular figures, no shrink.
- Snippet line: horizontal, gap 8px, UI 12px, line-height 1.45 (17.4px), text-secondary.
  - Label column: no shrink, inline row, gap 4px, min-width 46px, max-width 36% of the line, clipped, no wrap, UI 11px weight 600, line-height 17px, text-tertiary (prompt: accent). `Agent` and the file name: weight 500; the file name in mono, ellipsised.
  - Text: fills, wraps anywhere, at most 2 lines with an ellipsis; tool snippets mono 11px, line-height 17px. Highlights: background mark, radius 3px, text colour unchanged.
  - Plain last-prompt line (no hit): one line, ellipsised, UI 12px text-secondary.
- Empty / no match: vertical, gap 6px, padding 28px 16px, centred text, text-secondary (13px); the query in weight 600 text-primary; second line UI 11px text-tertiary. Error: same box, text status-waiting.

**Footer**: horizontal, items centred, gap 14px, padding 8px 10px 8px 16px, top border 1px separator, UI 11px text-tertiary. Key caps here: min-width 17px, padding 0 4px. Hints: `[↑][↓] move`, `[↩] go to card` / `[↩] read it` (`[Enter]` off macOS), `[⇥] select`.
- Start button (right-aligned): inline-flex, centred, gap 6px, padding 4px 6px 4px 9px, radius 6px, no border, transparent, UI 11px weight 600, accent text; `Sparkles` 13px stroke 2.2, label, key caps `⌘` `↩` (`Ctrl` `Enter` off macOS). Hover (enabled) background bg-hover. Ready (marks): background accent, text text-on-accent, hover accent-hover; its key caps: transparent border, background `rgba(255,255,255,0.2)`, no shadow, text colour inherited. Disabled: 40% opacity.

Screenshot check (search.png / search-dark.png): two marked rows in the selection tint with filled checks, `N matches` + date at the right, `You` in accent, tool rows `Edit <file>` with the file in mono, the button filled `New session from 2 ⌘ ↩`.

### Merge dialog

**Panel**: review step width `min(820px, windowWidth − 48px)`, height `min(86vh, 780px)`; gathering step width `min(560px, windowWidth − 48px)`, height fits content; both centred in the window (never larger than window − 48px each way); same modal chrome as the search (backdrop, radius 18px, SH-MODAL, rise 220ms). Content fills the panel vertically: header, body (fills, scrolls), footer.

**Header**: horizontal, top-aligned, gap 12px, padding 20px 22px 16px, bottom border 0.5px separator. Icon tile 32×32, radius 9px, linear gradient 135° from accent (L `#007aff` / D `#0a84ff`) to `#5e5ce6`, white `Sparkles` 16px stroke 2.2. Title UI 15px weight 600 (no margins). Subtitle UI 12px text-secondary, 2px below.

**Body**: vertical, gap 14px, padding 16px 22px, scrolls.
- Gathering list: vertical, gap 4px. Row: height 38px, padding 0 12px, radius 9px, background bg-sunken, items centred, gap 10px; phase icon 15px; title fills, one line, ellipsised, weight 500 (13px); state UI 12px text-tertiary (`Failed` in status-waiting). Elapsed line UI 12px text-tertiary.
- Phase icons (15px): queued `Circle` text-tertiary; running `Loader2` spinning, status-working; done `CheckCircle2` status-done; error `AlertCircle` status-waiting.
- Review chips: wrapping row, gap 6px. Chip: max-width 100%, padding 5px 10px, radius 8px, background bg-sunken (failed: status-waiting@8%, L `rgba(255,59,48,0.08)` / D `rgba(255,69,58,0.08)`; skipped: whole chip 50% opacity). Chip line: items centred, gap 7px, UI 12px; title max 280px ellipsised. Actions: 4px left margin, gap 2px; buttons ghost `.btn` at height 22px, padding 0 7px, UI 11px: `RotateCcw` 12px stroke 2.2 + `Retry`, and `Skip`. `Skipped`: UI 11px text-tertiary. Error text: UI 11px status-waiting, max-width 560px, margin 4px 0 0 22px, selectable.
- Field block: vertical, gap 6px. Label: horizontal, items centred, gap 10px, UI 12px weight 600 text-secondary. `Reset to generated`: text button, no padding/border/background, accent, UI 12px weight 500, cursor arrow.
- Inputs (context, instruction, name): full width, border 0.5px border-strong, radius 8px, background bg-window (L `#f5f5f4` / D `#1e1e20`), no outline; focused: border accent + ring `0 0 0 3px focus-ring`; border and ring transition 120ms `ease`.
  - Context: grows to fill the remaining body height, min 220px for its block; padding 12px 14px; not resizable; mono 12px, line-height 1.55 (18.6px); spellcheck off; placeholder `No summaries yet. Retry the failed sessions or write the context yourself.` (unclear: board.css sets no placeholder colour for the merge fields, so it is Chromium's default placeholder colour; match it from a screenshot in both themes — same for the instruction's placeholder).
  - Instruction: 3 rows (UI 13px, line-height 1.45 → ~78px tall), padding 10px 12px, user-resizable vertically (grip at bottom-right), spellcheck on.
  - Name: height 32px, padding 0 10px, UI 13px, spellcheck off.

**Footer**: horizontal, items centred, gap 8px, padding 12px 22px, top border 0.5px separator. Note: fills, UI 12px text-tertiary (key caps `⌘` `↩` + ` to start`, `Ctrl` `Enter` off macOS); error text status-waiting. Buttons: `Cancel` (`.btn`), `Start session` (primary `.btn`, `Sparkles` 14px stroke 2.2, or `Loader2` 14px spinning while starting).

Screenshot check (context.png / context-dark.png): review step with two blue-check chips, mono context with `## Session 1 — <title>` sections, focused instruction field with the blue focus ring and resize grip, name `From: <title> + <title>`.

### Ink

**Dock** (`.ink-dock`): 16px from the board's left, bottom 58px, above the canvas (z 5, under the prompt bar column); vertical, items centred, gap 2px, padding 3px, radius 11px, frosted 86% (blur 20px, saturate 1.6), SH-MD. Overall 34×223px.
- Tool button: 28×28, radius 8px, no border, transparent, icon 15px stroke 2 text-secondary, cursor arrow; hover (enabled) background bg-hover, icon text-primary; pressed (enabled) bg-pressed; on (current tool, also on hover/press): background bg-selected, icon accent; disabled 35% opacity. Icons: Pointer `MousePointer2`, Pen `Pencil`, Arrow `MoveUpRight`, Text `Type`, Eraser `Eraser`, Clear `Trash2`.
- Separator: 16×1px, margin 3px 0, separator colour.
- Colour button: a tool button holding a 14×14 dot in the current ink colour with inner hairline `inset 0 0 0 0.5px rgba(0,0,0,0.25)`.
- Palette: to the right of the colour button (its left edge 10px past the button's right edge), vertically centred on it; horizontal, gap 8px, padding 8px 10px, radius 12px, frosted 92% (blur 24px, saturate 1.8), SH-LG; enters 120ms EASE-OUT from opacity 0 and 4px further left. Swatches 18×18 circles in the order graphite, red, orange, green, blue, purple, fill = the ink colour token, hairline `inset 0 0 0 0.5px rgba(0,0,0,0.25)`; current: rings `0 0 0 2px bg-elevated`, `0 0 0 3.5px accent`. ≈ 168×34px.

**Layer** (board coordinates, transformed with the viewport):
- Lines: no fill, stroke = ink colour token, width `3 × size` board units, round caps and joins. Erasing: 25% opacity.
- Labels: positioned at their top-left point, display font weight 600, size `16 × size`, line-height and height `20 × size`, colour = ink colour token, whitespace preserved, one line, no padding; erasing 25% opacity.
- Label input: same font/placement/colour; min-width 48 board units, auto-width to its text (or placeholder); no border, radius 2, transparent; dashed outline 1.5 board units in accent, 3 units outside; caret in the ink colour; placeholder `Label` in the ink colour at 40% opacity; spellcheck off.
- Selection: INKUI-026; stroke accent.
- Graphite flips with the theme (dark ink on the light canvas, light ink on the dark one); all ink colours re-resolve on a theme change.

**Cursors**: INKUI-016/017.

### Selection bar

Horizontal, items centred, gap 4px, padding 6px 6px 6px 16px, radius 14px, frosted 86% (blur 24px, saturate 1.8), SH-LG; in the column it rises in (opacity 0 → 1, translateY 6px → 0, 220ms EASE-OUT). Count: UI 13px weight 600, tabular figures, no wrap. Divider: 1×18px, margin 0 8px 0 10px, separator colour. Buttons: **New session from context** primary `.btn` at height 30px with 2px right margin, `Sparkles` 14px stroke 2.2; **Group** ghost `.btn`, `Group` 14px; **Archive** ghost `.btn`, `Archive` 14px; × ghost icon `.btn` (28×28), `X` 15px. Overall height ≈ 42px.

## Keyboard & mouse

| Where | Input | Effect | ID |
| --- | --- | --- | --- |
| Board (no field focused, no overlay) | `/` | Focus the bar (insert `/` if empty) | BAR-033 |
| Board | `V` | Record / stop | BAR-034 |
| Board | `P` / `A` / `R` / `X` | Pen / Arrow / Rectangle / Eraser | INKUI-002, INKUI-027 |
| Board | Esc | Ink tool → Pointer, else deselect stroke, else board Esc; voice cancel takes precedence | INKUI-023, BAR-038 |
| Board | ⌫ / Delete | Delete selected stroke first | INKUI-022 |
| Board | ⌘Z / ⇧⌘Z | Ink undo / redo | INKUI-021 |
| Board (incl. text fields) | ⌘K | Open search | SRCH-001 |
| Board | Space held | Ink ignores presses (pan) | INKUI-020 |
| Bar field | Return / ⇧Return | Submit / newline | BAR-009 |
| Bar field, menu open | ↓ ↑ / Return, Tab / Esc | Move (wrap) / pick / dismiss | BAR-015/016 |
| Bar field | Backspace (empty) | Remove token | BAR-021 |
| Bar field | Esc | Close picker, else blur | BAR-023 |
| Bar pill | Mouse-down on background | Focus field | BAR-008 |
| Search | ↓ ↑ | Move (clamped) | SRCH-012 |
| Search | Return / click | Open | SRCH-013 |
| Search | ⌘Return | Start from marks / current | SRCH-017 |
| Search | Tab / ⇧Tab / ⌘-click / check click | Mark | SRCH-016 |
| Search | ⌘K / Esc / backdrop | Close | SRCH-003 |
| Merge | ⌘Return | Start | MERGE-015 |
| Merge | Esc | Close (backdrop doesn't) | MERGE-002 |
| Ink label | Return / blur / Esc | Commit / commit / discard | INKUI-013 |
| Board | `B` / `M` | Bubble / Mic tool | INKUI-002 |
| Board, bubble tool | Press | Open the bubble there (or move an empty one) | BUBBLE-001/002 |
| Bubble field | Return / ⇧Return | Send at once / newline | BUBBLE-013 |
| Bubble field | Esc | Close, nothing sent | BUBBLE-003 |
| Board, mic tool | Press / press again | Listen there / send | BUBBLE-040/041 |
| Mic listening | Esc / × | Cancel, nothing sent | BUBBLE-041 |
| Ink, Pointer | Double-click label | Edit | INKUI-014 |

## Platform differences

- ⌘ is Ctrl off macOS for ⌘K, ⌘Return (search, merge), ⌘Z/⇧⌘Z, ⌘-click (search marks). ⌘K and ⌘Z require that the *other* of ⌘/Ctrl is not held (macOS: Ctrl not held; Windows: the Windows key not held).
- Shortcut text off macOS (`keys()`): `⌘` → `Ctrl`, `↩` → `Enter`, `⌫` → `Del`, combos joined with `+` (`⌘Z` → `Ctrl+Z`, `⌘G` → `Ctrl+G`). Affects: search footer key caps and start button caps, merge footer caps, `Group into a section (Ctrl+G)`, `Archive selected (Del)`, the ink clear toast. Unchanged everywhere: the send button's `(Return)`, the mic's `(V)`, ink tool titles `(Esc)`/`(P)`/`(A)`/`(X)`, `⇥`, `↑`, `↓`, `esc`.
- Voice notice's microphone button text: `Open System Settings` (macOS) / `Open Settings` (others) (05).
- Fonts per platform (UI Details). Letter keys (`/`, `V`, `P`, `A`, `X`) are matched on the produced character, so on a non-Latin layout (Arabic, Russian, Greek) the letter shortcuts don't fire (unclear: BoardView.tsx:2429/2443 use `e.key`, unlike `shortcutKey()` used elsewhere; preserve).
- Spell checking: Electron's default spellchecker is on in the bar's field and the merge instruction (red underlines, no suggestions menu); off in search, merge context and name, ink labels.
- Reduced motion: UI Details.

## Edge cases & errors (verbatim)

- Bar toasts: `Couldn't start a session, so your prompt is on a note: ${message}` · `That session is no longer on the board.` · `“${title}” is waiting for your answer. Answer it, then send this.` · `Couldn't send it: ${message}` · `Couldn't create the view: ${message}` (straight apostrophes in `Couldn't`).
- Ink toast: `Cleared the ink. Press ${keys('⌘Z')} to bring it back.`
- Search: `No session on this board mentions ${query}.` + `Prompts, replies, file paths, commands and edits are searched; tool output isn’t.` (curly apostrophe) · `No sessions on this board yet.` · error = the backend's message as is.
- Merge: `The session returned an empty summary.` · `Retry or skip the failed sessions to continue.` · start error = the create error's message.
- A bar submission while one is in flight, while the menu is open, or while voice isn't idle is ignored (BAR-010).
- Into target that became archived/removed between selection and Return → toast, words kept.
- Waiting session → card opens, words kept; the user answers, then presses Return again.
- `into` with a selected empty note still sends into the session (empty notes are not context).
- Jumping from search to a live card makes it the bar's into target (SRCH-014); a previously dropped id is forgotten because the into id changed.
- Search: an error stays shown until a later success, and suppresses the spinner and the no-match message meanwhile (SRCH-009). Returning to a previously answered query shows its old answer immediately while a fresh request runs. Enter during an IME composition in the search field is not filtered (opens the row) (unclear: SessionSearch.tsx:114 has no `isComposing` check; preserve). Esc during composition closes the dialog (Modal listens on the window).
- Search marks referencing rows no longer listed still count (`New session from N`).
- Fuzzy: a title whose lowercase form is longer than the title in UTF-16 units (e.g. containing `İ` U+0130) can make `isUpper(text[j])` read past the end and throw in JS (fuzzy.ts:38) — unclear: the port must not crash; treat a missing character as "not a separator, not upper case" and flag the divergence. Long titles can score negative: if the best score is negative, `floor = 0.5 × best` is above it and the stack shows **no** results (BoardView.tsx:580) — preserve.
- Merge: Esc in any field closes the dialog and loses edits. Progress `done` events arrive before the results, so the dialog can reach review a moment before summaries fill the context. Titles in the list change to the backend's titles once summaries arrive.
- Ink history: redo pushes onto the undo stack without trimming, so it can exceed 100 entries until the next new change trims it (InkLayer.tsx:269) — preserve.
- Ink: a pen press without movement commits a dot; an arrow shorter than 8 screen px is dropped silently; a label finished empty is dropped; editing a label to the same text records nothing; hit-testing a label uses the measured text width (empty label text → zero-width box, still hit within tolerance).
- Ink over a view: the stroke draws over the page (and while a drawing tool is out, the page doesn't get the pointer).
- Ink label Enter during an IME composition is not filtered (unclear: InkLayer.tsx:590; preserve).
- Ink colour storage unavailable → starts red, choices not remembered.
- The slash menu only opens with no target, and the colour picker only exists with the note target, so the two never show together; the session token hides while a target is set and comes back when it is removed.

## Tests to port

compose.test.ts → `nodus-board::compose` (or wherever `with_context`/slash parsing land):
- `matchCommands` · `offers every command for a lone slash`: `match('/')` → `['/view', '/note']`.
- `matchCommands` · `narrows as the word is typed, in any case`: `'/V'` → `['/view']`; `'/note'` → `['/note']`.
- `matchCommands` · `offers nothing for other words, or once there is a space`: `'/x'`, `'/view '`, `'fix the /view'`, `''` → `[]`.
- `takeCommand` · `takes a command followed by a space, with what comes after`: `'/view '` → `{ view, '' }`; `'/NOTE  ask finance\nby Friday'` → `{ note, 'ask finance\nby Friday' }`.
- `takeCommand` · `leaves a command still being typed, and anything else`: `'/view'`, `'/views of x'`, `'add a /view'` → `null`.
- `withContext` · `leaves a prompt alone when nothing is selected`: `([], 'Fix it')` → `'Fix it'`.
- `withContext` · `writes the selection out before the prompt`: the four-item example in Data & formats → exactly the text shown there.

fuzzy.test.ts → `nodus-board::fuzzy` (`score(q, t)` = score or −∞):
- `matches everything with an empty query`: `fuzzy('  ', 'anything')` = `{ score: 0, indices: [] }`.
- `needs every character, in order`: `('xyz', 'Fix the board')` → none; `('ba', 'abc')` → none.
- `ignores case and spaces in the query`: `('F B', 'fix bug').indices` = `[0, 4]`.
- `lands on word starts, including camelCase ones`: `('bv', 'open BoardView').indices` = `[5, 10]`; `('sr', 'session-rail').indices` = `[0, 8]`.
- `ranks a run of characters above scattered ones`: `score('board', 'Board view') > score('board', 'big old area red dog')`.
- `ranks a match at the start above one later on`: `score('fix', 'fix login') > score('fix', 'login fix')`.
- `prefers the shorter title when they match alike`: `score('api', 'api') > score('api', 'api docs')`.

ink.test.ts → `nodus-board::ink` (`pen(id, points, size = 1)` = red pen stroke):
- `penPath` · `draws a dot, a line, and curves through the midpoints`: `[[1,2]]` → `M1 2h0.01`; `[[0,0],[10,0]]` → `M0 0L10 0`; `[[0,0],[10,0],[10,10]]` → `M0 0Q10 0 10 5L10 10`.
- `arrowPath` · `puts the head on the tip`: `arrowPath([0,0], [100,0], 10)` starts with `M0 0L100 0M` and contains `L100 0L`.
- `simplify` · `drops points on a straight line and keeps corners`: `[[0,0],[1,0.01],[2,0],[3,0.02],[4,0]]`, 0.5 → `[[0,0],[4,0]]`; `[[0,0],[5,0],[10,0],[10,5],[10,10]]`, 0.5 → `[[0,0],[10,0],[10,10]]`.
- `hitStroke` · `hits near the line, not away from it` (line `[0,0,100,0]`, tol 3): `[50,4]` true; `[50,9]` false; `[120,0]` false.
- `hitStroke` · `reaches further for thicker ink`: size 4 line, `[50,9]`, tol 3 → true.
- `hitStroke` · `hits a label anywhere in its box`: label `Ship it` at `[10,10]` size 1 (blue): `[14,20]` tol 0 → true; `[14,40]` → false. (Node has no canvas, so the width is the fallback `7 × 16 × 0.56 = 62.72`; the Rust test should inject the same fallback measurer.)
- `strokeAt` · `finds the topmost stroke, labels first`: `[under, over]` both `[0,0,100,0]`, `[50,0]` tol 3 → `over`; label `here` at `[40,-10]` with `[label, under]`, `[45,0]` → `t`; `[under]`, `[50,50]` → none.
- `strokesAlong` · `catches strokes the eraser passes over between two pointer events`: `[pen a [50,-20,50,20], pen b [200,-20,200,20]]`, from `[0,0]` to `[100,0]`, tol 3 → `['a']`.
- `translate` · `moves every point`: `translate(pen [0,0,10,20], 5, -5).points` = `[5,-5,15,15]`.

Recommended additions (no new behaviour): Rust unit tests for merge `compose` / `defaultName` / `initialPrompt` and the bar's placeholder/label tables; screenshot tests of the bar (closed, open, each token, menu, picker, recording, transcribing), search (latest, results, marked, empty, error), merge (gathering, review, failed chip, start error), ink (each tool's cursor, selection box, palette) and the selection bar, in light and dark.

## Rust mapping

Pure logic (no GPUI):
- `nodus-board::compose` — `ContextItem`, `with_context`, `COMMANDS`, `match_commands`, `take_command` (+ compose tests).
- `nodus-board::merge` — `section`, `compose_handoff`, `default_name`, `initial_prompt`, `DEFAULT_INSTRUCTION`, the merge state machine (items, phases, dirty/append rule, `can_start`) so it's testable headless; `nodus-core` exposes `summarize` + the progress stream (03).
- `nodus-board::fuzzy` — `fuzzy_match(query, text) -> Option<FuzzyMatch { score: f64, indices: Vec<usize /* UTF-16 */> }>` over `Vec<u16>` of the original and lowercased strings (+ tests).
- `nodus-board::ink` — geometry (`pen_path`, `arrow_path` as path commands **and** their string form for tests, `simplify`, `stroke_box` with an injectable label measurer, `hit_stroke`, `stroke_at`, `strokes_along`, `translate`, `size_at`, `r1`) and the undo history (`InkHistory { past, future }`, cap 100) (+ tests).
- `nodus-board::search_view` (or in nodus-ui) — latest-sessions ordering, row mapping, start-ids rule.

UI (`nodus-ui::board::overlays`):
- `prompt_bar` (column layout, pill, tokens, slash menu, colour picker, send, status line, mic orb), `toast`, `selection_bar`, `search` (+ `highlight` runs from UTF-16 ranges), `merge_dialog`, `ink::{layer, dock, label_input}`. The board view owns the wiring (BAR-024…031, SRCH-014/015, MERGE-001/016) and calls `nodus-core` commands (`sessions.create/send/search/summarize`, `views.create`, `board.save_note/save_ink`).

Text input needs:
- Prompt field: multi-line, auto-grow 1–5 lines of 20px then scroll, placeholder (wrapping placeholder grows the field), IME with composition state visible to key handlers (Return/arrows/Tab/Backspace/Esc ignored while composing), key interception before default editing (Return, Tab, ↑/↓ only while the menu is open, Backspace-on-empty, Esc), programmatic text set + caret-to-end + focus, focus/blur events, paste of multi-line text (feeds `take_command`), hidden but keeping its value while listening, spellcheck underline (see risks). Tokens are separate elements before the field, not inline chips, so no rich-text editing is needed.
- Search field: single-line, select-all on focus, keydown interception (↑/↓/Return/⌘Return/Tab/⇧Tab/⌘K), no spellcheck.
- Merge: context = large multi-line mono editor with scrolling and focus ring; instruction = 3-row multi-line with a user drag-to-resize grip (vertical only); name = single line. All need IME.
- Ink label: single-line, auto-width (min 48 units), rendered **inside the zoomed canvas transform** (font size and outline scale with zoom), transparent background, dashed outline, caret and placeholder in the ink colour.
- gpui-component's `Input`/`InputState` (multi-line, auto-grow rows, placeholder, IME via `EntityInputHandler`) is the likely base; verify composition events and key-interception order.

Risks:
1. **Backdrop blur.** Every frosted surface here (pill, mic, menus, toast, selection bar, dock, palette, modal backdrop 3px) uses `backdrop-filter`; GPUI has no per-element backdrop blur. Without it the 86–96% surfaces look close but not identical over busy cards — needs a renderer extension or an agreed fallback, checked in screenshots.
2. **Ink over native webviews.** Views (D3, wry) are native child views painted above GPUI content, so ink (and the selection outline) can't draw over a view's page, and the page must stop taking the pointer while a drawing tool is out (INKUI-018). Same for any overlay that must cover a view.
3. **Capture-phase input routing.** Ink must see presses before cards, terminals, notes and the canvas, decide "on the board surface" exactly like `.react-flow__renderer` (excluding dock, controls, minimap, rails, bar column, modals), swallow the following click/double-click, blur in-board focus, and hold the pointer until release (GPUI window-level capture listeners + a global drag tracker). The same pattern is needed for the picker/palette "close on any outside pointer-down" and for voice's capture-phase Esc.
4. **Custom cursors.** The eraser's circle cursor and the "move" cursor have no GPUI `CursorStyle` equivalent; needs native cursor images (NSCursor/HCURSOR) or drawing the cursor in-app.
5. **UTF-16 offsets & JS semantics.** Highlight ranges (03) and fuzzy indices are UTF-16 code-unit offsets; GPUI text runs are UTF-8 byte ranges — convert carefully (surrogate pairs, emoji). Fuzzy must lowercase like JS `toLowerCase` and index UTF-16 to reproduce scores and indices exactly. `r1`/`sizeAt` must use JS rounding (half up), and path strings must print numbers like JS.
6. **Text inputs.** Auto-growing multi-line field with IME, spellcheck squiggles (Electron default; macOS `NSSpellChecker` would be needed for parity — unclear whether required), a vertically resizable textarea, and an in-canvas scaled label input are all beyond GPUI's basics.
7. **Ink colour preference.** It lives in Chromium's localStorage (`nodus.inkColor`) inside the userData folder's LevelDB; the Rust app won't read it unless it parses that store, so upgraders would fall back to red — decide (read once, or accept and record).
8. **Label measurement.** Hit boxes and the selection outline depend on the display font's measured width at weight 600; measure with the same font in GPUI so boxes match.
9. **Animations.** Width (flex-basis) transitions, the column rise, token scale-in, popover rise, card flash (box-shadow keyframes after the viewport animation resolves), status-dot breathing/pulse, meter at frame rate — GPUI needs explicit animation state for each, and reduced-motion handling.

## Parity checklist

- [x] BAR-001 Column placement, width formula, z-order, pass-through
- [x] BAR-002 Stack order (toast, selection bar, voice notice, row) and rise-in
- [x] BAR-003 Row: pill + mic, widths 520/600, shrink, transitions
- [x] BAR-004 Open-state conditions
- [x] BAR-005 Field auto-grow 1–5 lines, then scroll; spellcheck on
- [x] BAR-006 Placeholder table
- [x] BAR-007 `/ view or note` hint conditions
- [x] BAR-008 Pill background mouse-down focuses field
- [x] BAR-009 Return submits, ⇧Return newline, IME ignored
- [x] BAR-010 Submit guards
- [x] BAR-011 Submission priority, trimmed text
- [x] BAR-012 In-flight spinner; false keeps, true clears target/text and dismisses notice
- [x] BAR-013 Send label/tooltip table and disabled conditions
- [x] BAR-014 Slash menu open conditions and filtering
- [x] BAR-015 Menu keys (wrap), Return/Tab pick, hover/click
- [x] BAR-016 Esc dismisses menu; edit reopens; blur resets
- [x] BAR-017 Picking clears typed text
- [x] BAR-018 Typed/pasted `/view ` `/note ` become tokens with rest
- [x] BAR-019 Token visuals, × behaviours, enter animation
- [x] BAR-020 Session token show/drop rules; dropped id reset on into change
- [x] BAR-021 Backspace on empty removes target, then session token
- [x] BAR-022 Note colour default yellow, picker, outside-press close
- [x] BAR-023 Esc in field: picker, else blur
- [x] BAR-024 `into` and `contextCount` computation
- [x] BAR-025 Context items and order; withContext prompt
- [x] BAR-026 New card placement (expanded size, below sources or viewport centre, free spot, section)
- [x] BAR-027 Create options; sticker note; selection cleared only with context; pan; no window
- [x] BAR-028 Failure → free note + toast; bar clears
- [x] BAR-029 Into: missing/archived, waiting (opens card), send error, success
- [x] BAR-030 /view creation and error toast
- [x] BAR-031 /note creation with colour
- [x] BAR-032 Toast one at a time, 4200ms
- [x] BAR-033 Board `/` key
- [x] BAR-034 Board `V` key
- [x] BAR-035 Mic button states, labels, icons, disabled
- [x] BAR-036 Recording halo and pill ring
- [x] BAR-037 Status line (dot, clock, meter / Transcribing…, cancel)
- [x] BAR-038 Esc cancels voice first
- [x] BAR-039 Transcript appended with a space, caret to end
- [x] BAR-040 Voice notice above the row
- [x] BAR-041 Bar state per board mount
- [x] SRCH-001 ⌘K open guards; closes preview first
- [x] SRCH-002 Menu `Search Sessions…` closes terminal/preview
- [x] SRCH-003 Close on Esc, ⌘K, backdrop, open, start
- [x] SRCH-004 Query kept per board; selected on open
- [x] SRCH-005 Latest sessions list (30, live first, updated desc, last prompt); empty board message
- [x] SRCH-006 120ms debounce, trimmed query, stale answers dropped
- [x] SRCH-007 Spinner; stale rows kept; hits mapped to cards
- [x] SRCH-008 No-match message
- [x] SRCH-009 Error display and persistence
- [x] SRCH-010 Row header: dot/archive icon, title, matches, time
- [x] SRCH-011 Snippet line: label column, highlights, mono tool text, 2-line clamp
- [x] SRCH-012 ↑/↓ clamped, reset on new answer, scroll nearest, hover
- [x] SRCH-013 Return/click open; `read it` / `go to card`
- [x] SRCH-014 Live card: select only it, centre at ≥0.8 over 320ms, flash ×2
- [x] SRCH-015 Archived: transcript preview at the match
- [x] SRCH-016 Tab/⇧Tab/⌘-click/check marking; marks persist
- [x] SRCH-017 Start button labels, disabled, ⌘Return, ids rule, focus = query
- [x] SRCH-018 Searchable set excludes terminals, runs, view brains
- [x] SRCH-019 Highlight runs from UTF-16 ranges; mark background
- [x] SRCH-020 Title highlights from answered query terms; underline style
- [x] SRCH-021 fuzzyMatch exact; archive stack ranking and 50% floor
- [x] MERGE-001 Entry points and source filtering, focus
- [x] MERGE-002 Modal: no backdrop close; Esc/Cancel close; results dropped
- [x] MERGE-003 Focus normalization
- [x] MERGE-004 One summarize request on open; progress for own request ids; done is final
- [x] MERGE-005 Gathering UI, subtitles, states, elapsed counter
- [x] MERGE-006 Result mapping: error/empty → error; done → summary + title
- [x] MERGE-007 One-way move to review
- [x] MERGE-008 Review chips, failed/skipped visuals
- [x] MERGE-009 Retry one source
- [x] MERGE-010 Skip
- [x] MERGE-011 Context follows generated until edited; Reset to generated
- [x] MERGE-012 Edited context + retry → appended numbered sections
- [x] MERGE-013 Instruction autofocus/placeholder/resizable; Name default
- [x] MERGE-014 Start enabled rule
- [x] MERGE-015 Start: spinner, name fallback, initial prompt, parents
- [x] MERGE-016 Start error stays; success placement, close, clear selection, open terminal
- [x] MERGE-017 Footer hint priority
- [x] INKUI-001 Dock placement, order, tooltips, names, no focus steal
- [x] INKUI-002 Tool keys P/A/X; starts at Pointer
- [x] INKUI-003 Tool change finishes label; deselects off Pointer
- [x] INKUI-004 Colours, palette, persistence (default red), recolour selected
- [x] INKUI-005 Clear all: disabled, undoable, toast
- [x] INKUI-006 Press gating (primary, no Space, board surface only)
- [x] INKUI-007 Taking a press: stop, finish label, blur, hold pointer, swallow click
- [x] INKUI-008 Pen: 1.5px step, coalesced samples, simplify 0.5/zoom, dot
- [x] INKUI-009 Arrow: dot before move, < 8px discarded
- [x] INKUI-010 size = round2(clamp(1/zoom, 0.5, 4)) and derived sizes
- [x] INKUI-011 Eraser: 6px path sampling, 25% preview, one change
- [x] INKUI-012 Pointer cancel discards
- [x] INKUI-013 Text labels: place, Return/blur commit, Esc discard, empty dropped
- [x] INKUI-014 Double-click label edits; change/delete/no-op
- [x] INKUI-015 Pointer tool select/move (3px), clears card selection, pass-through
- [x] INKUI-016 Move cursor on hover over ink
- [x] INKUI-017 Tool cursors incl. eraser circle
- [x] INKUI-018 Views ignore pointer while a tool is out
- [x] INKUI-019 No new session on double-click while drawing / on a line
- [x] INKUI-020 Space held → ink ignores presses
- [x] INKUI-021 Undo/redo, 100 entries, per board, guards
- [x] INKUI-022 ⌫/Delete deletes selected stroke first
- [x] INKUI-023 Esc precedence
- [x] INKUI-024 Every change saved immediately
- [x] INKUI-025 Z-order (above cards, below open archive panel) and paint order
- [x] INKUI-026 Selection outline (dashed, non-scaling, padded)
- [x] INKUI-027 Rectangle: R, corner to corner, Shift square, < 8px discarded, outline hit only
- [x] INKUI-028 Server boards: ink saved and undone a stroke at a time (hosted, added after 1.0.12)
- [x] BUBBLE-001…003 Bubble tool press opens it; words kept, empty moves; Esc, focus and tool change
- [x] BUBBLE-004…008 Placement and flips, speech-bubble corner, dock-icon cursors, body and hint, dot, z-order
- [x] BUBBLE-013, BUBBLE-016 Return sends at once, no model while typing; keys stay out of the board
- [x] BUBBLE-020…024 Haiku routing arguments, prompt, reading, side by side, no answer is a session
- [x] BUBBLE-030…032 Marker while deciding, exact top-left landing without panning, model kept for resumes
- [x] BUBBLE-040…044 Mic tool: press to listen, press to send, same routing, pill and notice, cancel on tool change
- [x] SEL-001 Visibility (session-type selection, no merge dialog), position
- [x] SEL-002 `N selected` count
- [x] SEL-003 New session from context (disabled for terminals only)
- [x] SEL-004 Group (disabled when all archived), tooltip
- [x] SEL-005 Archive (disabled with no live Claude session), tooltip
- [x] SEL-006 Clear selection
