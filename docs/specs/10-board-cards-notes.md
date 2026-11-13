# 10 · Cards, expanded cards, agent deck, archive & notes

Sources:
- `src/renderer/src/components/board/SessionCardNode.tsx` (321 lines, all read)
- `src/renderer/src/components/board/ExpandedCard.tsx` (236, all read)
- `src/renderer/src/components/board/AgentDeck.tsx` (247, all read)
- `src/renderer/src/components/board/ArchiveZoneNode.tsx` (176, all read)
- `src/renderer/src/components/board/NoteNode.tsx` (218, all read)
- `src/renderer/src/components/board/NotePadNode.tsx` (42, all read)
- `src/renderer/src/components/board/notes.ts` (152, all read)
- `src/renderer/src/components/Highlight.tsx` (15, read; **not used by cards** — used by `SessionSearch.tsx` (11) and `TranscriptPreview.tsx` (12). Cards use their own `highlight()` in SessionCardNode.tsx:299)
- `src/renderer/src/components/board/board.css` (1987, all read; this spec owns lines 61–1011 and 1243–1577)
- `src/renderer/src/styles/tokens.css` (all), `src/renderer/src/styles/global.css` (`.status-dot`, `.btn`, `.selectable`, `:focus-visible`, reduced motion)
- Supporting reads (owned elsewhere, quoted only where cards depend on them): `components/board/layout.ts`, `shared/geometry.ts`, `components/board/BoardContext.ts`, `views/BoardView.tsx` (buildNodes, stack, notes, click/drag handlers, confirms), `components/board/stopSessions.ts`, `components/board/LineageEdge.tsx`, `components/UsageStats.tsx` + `.css`, `components/voice/SessionMic.tsx` + `voice.css`, `terminal/terminal.css` (overlay rules), `terminal/useAttachedTerminal.ts`, `terminal/registry.ts` (`needsOpen`, `focus`, `typeIn`), `store.ts` (`cardTitle`, `isTerminal`, `tildePath`, `byUrgency`, `formatDuration`, `agentStep`, `agentStatusLabel`, `unstickNotes`), `components/FolderIcon.tsx` (`FOLDER_HUES`), `platform.ts` (`keys`, `modKey`), `@xyflow/react/dist/style.css` 12.12.0 (resize controls, animated edges)
- `docs/how-it-works.md` §Expanded cards, §Status, §Agents, §Notes, §Shortcuts
- Screenshots: `docs/screenshots/board*.png`, `expanded*.png`, `archive*.png`, `views*.png` (note pad, sticker, italic prompt preview). The board/expanded/archive shots predate the note pad and the current corner buttons; **the code wins** where they differ.

Cross-domain owners: canvas mechanics, node z-order, selection, drag/drop resolution, context menus, keyboard shortcuts, edges routing, archive stack placement/flights → **09-board-canvas.md**. Overlays (prompt bar, ⌘K, merge, selection bar, confirm dialog chrome) → **11**. Terminal element, modal, AgentViewer, transcript preview → **12**. Mic/voice → **05**. Card/note data semantics, `state.json`, notes placement by board tools → **03**/**04**. Theme tokens and shared buttons → **08**.

## Summary

A session on the board is a **card** (288×216 flow units): status dot (or a shell glyph for terminal cards), a two-line title, a "merged from N" chip, a 4-line preview of Claude's last reply, and a footer (branch, message count, agents, time). Hover reveals Archive and Expand buttons in the top-right corner. A card can **expand in place** (default 720×480, a new card's 720×960, min 360×240, resizable from every edge/corner) into a 40px title bar over the live terminal, with usage stats and tool buttons (🤖 agents, mic, Resume/Restart, Stop, Archive, Open in a window, Collapse). A session's current-run **agents** peek out under its card as up to three coloured layers (the **deck**); pointing at the deck fans the agents out beside the card as clickable mini-cards, a click pins the fan. The **archive** is a dashed frame holding all archived cards as one stack (top card plus two slabs peeking below); hovering it reveals a search field that takes focus so you can just type; searching or clicking fans results out into a floating panel. **Sticky notes** are paper-coloured squares (8 colours, light and dark paper) that you write in place; a ▶ button starts a session from a note, after which the note rides on the card as a small tilted **sticker**. A **note pad** under the archive offers a top sheet to drag off or click.

All values below are resolved for **light** and **dark**. The theme follows `nativeTheme.themeSource` (System/Light/Dark, setting owned by 01/08); the CSS only uses `prefers-color-scheme`, so "dark" below means whenever the resolved scheme is dark.

## Behaviors

### Session card (compact) — CARD

- **CARD-001** A non-expanded card (any card with `expanded` falsy, or any archived card) renders at exactly 288×216 flow px (`CARD_W`×`CARD_H`), box-sizing border-box, padding 14px top / 16px left+right / 11px bottom, vertical flex column with 8px gap.
- **CARD-002** The card surface is `--bg-elevated` (light `#ffffff`, dark `#2a2a2d`), corner radius 14px, shadow `--shadow-sm`.
- **CARD-003** Pointer over the card switches its shadow to `--shadow-md`.
- **CARD-004** A card whose `status === 'waiting'` draws a 1px ring in `--status-waiting` at 45% alpha (light `rgba(255,59,48,0.45)`, dark `rgba(255,69,58,0.45)`) plus `--shadow-md`, hovered or not (waiting beats hover).
- **CARD-005** A selected card draws a 2px `--accent` ring (light `#007aff`, dark `#0a84ff`) plus `--shadow-md`; selection overrides the waiting ring (the waiting ring is not drawn while selected).
- **CARD-006** A card being dragged draws the 2px accent ring plus `--shadow-lg` and scales to 1.015 around its centre; this overrides selected and waiting. (Exception: ARCH-017.)
- **CARD-007** Shadow changes animate over 220ms `cubic-bezier(0.2,0.8,0.2,1)`, opacity over 220ms `ease`, transform (the drag scale) over 220ms `cubic-bezier(0.2,0.8,0.2,1)`.
- **CARD-008** An archived card has opacity 0.78, restored to 1 on hover, selected or dragging; in 0.9.4 every rendered archived card also matches the stack rules, so the effective opacity is 1 (stack depth 0–2 and results) or 0 (depth 3) — see ARCH-014. Port the rule and the override so the cascade result is identical.
- **CARD-009** The pointer cursor over a card is the default arrow; while a card is being dragged the cursor is `grabbing`.
- **CARD-010** The header row is a horizontal flex, items aligned to the top, 9px gap, minimum height 20px.
- **CARD-011** A Claude card (`kind` absent or `'claude'`) shows a 10×10 status dot as the first header item, offset 5px from the top of the header; its tooltip is the status label: `Working`, `Needs you` or `Done`. A status's colour fades into the next (MOTION-012); a card stopping to ask raises its hand (MOTION-013), and one finishing a long run is ringed (MOTION-031).
- **CARD-012** Status `working`: dot fill `--status-working` (light `#ff9500`, dark `#ff9f0a`) with a 3px spread ring of the same colour at 22% alpha, breathing: at 50% of a 1.6s cycle the ring is 5px at 10% alpha; easing `ease-in-out` per half; infinite.
- **CARD-013** Status `done`: dot fill `--status-done` (light `#007aff`, dark `#0a84ff`), no ring, no animation.
- **CARD-014** Status `waiting`: dot fill `--status-waiting` (light `#ff3b30`, dark `#ff453a`), pulsing: a ring grows from 0px spread at 55% alpha to 8px spread at 0 alpha over 1.1s, `ease-out`, infinite (restarts at 0 each cycle).
- **CARD-015** The small dot variant (agent rows, footer running badge) is 7×7; small `working` has a 2px ring at 22% breathing to 3.5px at 10% (1.6s ease-in-out infinite); small `waiting` and `done` are as CARD-013/014 (the waiting pulse still reaches 8px).
- **CARD-016** A terminal card (`kind === 'terminal'`) shows a lucide `SquareTerminal` icon (14px, stroke 2) instead of the dot, 3px from the top of the header, coloured `--text-tertiary` when the shell is not live and `--text-primary` when live; tooltip `Terminal · Running` or `Terminal · Exited`.
- **CARD-017** The title text is `card.title` if non-empty, else `card.meta.title` if non-empty, else `Terminal` for terminal cards or `New session` otherwise; it is clamped to 2 lines with a trailing `…`, breaks anywhere (even mid-word) to avoid overflow, and its tooltip is the full title.
- **CARD-018** When the card is an archive search result with matched character indices, each maximal run of matched characters is drawn in `--accent` (no background), the rest in the title colour.
- **CARD-019** A live Claude card (`live && !terminal`) shows a `SquareTerminal` icon (13px, stroke 2) at the end of the header row, 3px from the top, `--text-tertiary`, tooltip `Claude process running`. Terminal cards never show it.
- **CARD-020** A non-archived compact card has a corner group at top 9px / right 9px (horizontal, 2px gap) containing, left to right, an Archive button (Claude cards only) and an Expand button; both are invisible (opacity 0) until the pointer is over the card, then fade to opacity 1 over 120ms `ease`.
- **CARD-021** Each corner button is 26×26, radius 6px, background `--bg-elevated`, icon colour `--text-secondary`; hovering a button sets its background to `--text-primary` mixed 7% into `--bg-elevated` (light `rgba(239,239,239,0.992)`, dark `rgba(56,56,58,0.993)`) and its icon to `--text-primary`; background transitions 120ms `ease`.
- **CARD-022** The corner Archive button (lucide `Archive` 13px, stroke 2; tooltip `Archive (⌫)` on macOS, `Archive (Del)` on Windows; accessible name `Archive`) archives the card through the board's archive action, which asks first when the session is live and not `done` (copy in Edge cases).
- **CARD-023** The corner Expand button (lucide `Maximize2` 13px, stroke 2; tooltip `Expand on the board (E)`; accessible name `Expand on the board`) expands the card in place (board action `expand([id], true)`, which also wakes the card's terminal — XCARD-025).
- **CARD-024** Clicks and double-clicks inside the corner group (buttons or the gap between them) never reach the card (no open, no rename), and pressing there never starts a card drag.
- **CARD-025** A card with `parents.length > 0` shows a chip under the header: lucide `GitMerge` 11px stroke 2.2, then text `merged from N` (N = `parents.length`), tooltip `Started from the context of earlier sessions`; chip height 19px, horizontal padding 7px, pill radius, 4px icon-text gap, background accent at 10% (light `rgba(0,122,255,0.10)`, dark `rgba(10,132,255,0.10)`), text `--accent`, 11px weight 500, left-aligned (does not stretch).
- **CARD-026** The preview is chosen in this order: terminal card → the project's repo path through `tildePath` (or empty if no repo path) as kind `path`; else `meta.lastAssistantText` (non-empty) through `stripMarkdown` as kind `reply`; else `meta.firstPrompt` (non-empty) as kind `prompt`; else the text `No messages yet` as kind `empty`. A new preview rises in over the old (MOTION-014), and Claude's light passes through it while the session works (MOTION-030).
- **CARD-027** `stripMarkdown` applies, in order: replace every fenced block ```` ```…``` ```` (non-greedy, across lines) with one space; replace `[text](url)` with `text`; delete every `**`, `__` and `` ` ``; delete, at the start of any line, up to 3 spaces followed by 1–6 `#` or one `>` followed by whitespace.
- **CARD-028** The preview text is 12px `--font-ui`, line-height 1.45 (17.4px), `--text-secondary`, whitespace runs (including newlines) collapse to single spaces, breaks anywhere, clamped to 4 lines with `…`; it is not stretched to fill (its box is only as tall as its lines).
- **CARD-029** When the merged chip is shown, the preview clamps to 3 lines instead of 4.
- **CARD-030** A `prompt` preview is italic.
- **CARD-031** An `empty` preview (`No messages yet`) is `--text-tertiary`.
- **CARD-032** A `path` preview is `--font-mono` 11px (line-height 1.45 = 15.95px), clamped to 2 lines, colour `--text-secondary`.
- **CARD-033** The footer sits at the bottom of the card (pushed down by auto top margin), with 9px top padding above its content and a 0.5px top border in `--separator` (light `rgba(0,0,0,0.07)`, dark `rgba(255,255,255,0.07)`); horizontal flex, centred vertically, 12px gap, `--text-tertiary`, 11px, tabular figures, no wrapping.
- **CARD-034** If `meta.gitBranch` is set, the footer starts with lucide `GitBranch` 11px stroke 2.2 and the branch name (4px gap), the name truncated with `…` at max 130px; tooltip is the branch name.
- **CARD-035** If `meta.messageCount > 0`, the footer shows lucide `MessageSquare` 11px stroke 2.2 and the count, tooltip `N messages`.
- **CARD-036** If the card has a deck (active card with ≥1 current-run agent), the footer shows a button: lucide `Bot` 11px stroke 2.2, the number of current-run agents, and, if any of them is not `done`, a running badge 3px to its right: a small status dot (`waiting` if any agent is waiting, else `working`) and the running count, in `--text-secondary`, 4px gap.
- **CARD-037** The footer agents button (with a deck) fans the deck out when the pointer enters it, toggles the pin on click (DECK-009), does not open/rename the card on click or double-click, has `aria-expanded` = whether the fan is open, and tooltip `R of N agents running` (R running > 0) or `N agents, all done`.
- **CARD-038** If the card has no deck, is not archived and `agentCount > 0`, the footer shows a button with `Bot` and `agentCount`; clicking it opens the Agents viewer for the session (12) without an agent preselected; tooltip `1 agent: show what they did` or `N agents: show what they did`; click/double-click do not reach the card.
- **CARD-039** Both footer agents buttons are 18px tall with a -2px vertical / -5px horizontal negative margin and 5px horizontal padding, radius 5px, transparent; hovering sets background `--bg-hover` (light `rgba(0,0,0,0.05)`, dark `rgba(255,255,255,0.07)`) and text `--text-primary`, background transition 120ms `ease`; pressing on them never starts a card drag.
- **CARD-040** A terminal card's footer shows its status label `Running` (live) or `Exited` (not live) after the agent item.
- **CARD-041** The last footer item is the relative time, right-aligned (auto left margin): empty if the timestamp is 0/absent; `just now` (<1 min); `Nm ago` (<1 h, floored); `Nh ago` (<24 h); `Nd ago` (<7 d); else the date in the system locale with short month and numeric day (`Oct 3`), plus the numeric year when older than 300 days (`Oct 3, 2025`). Negative ages count as 0.
- **CARD-042** The time is computed from `meta.updatedAt`, falling back to `card.createdAt`; there is no ticking timer — it is recomputed whenever the card re-renders (any change to the card's data, selection, drag state, or the renaming target).
- **CARD-043** The card's accessible label is `<title>, <status label>` (status label per CARD-011/016).
- **CARD-044** While the board's renaming target is this card, the title is replaced by a single-line text field pre-filled with `card.title || meta.title || ''` (never the `New session`/`Terminal` placeholder), focused with all text selected, spell-check off.
- **CARD-045** The rename field is 24px tall with -2px vertical margin, 5px horizontal padding, no border, radius 5px, background `--bg-window` (light `#f5f5f4`, dark `#1e1e20`), a 2px `--accent` ring, `--font-display` 15px weight 600, text `--text-primary`, no focus outline, filling the remaining header width.
- **CARD-046** Enter or losing focus commits once: if the trimmed value differs from the trimmed initial value, the card's title override is set to the trimmed value (empty clears the override, falling back per CARD-017); renaming then ends.
- **CARD-047** Escape ends renaming without saving.
- **CARD-048** The rename field consumes every key (board shortcuts never fire while typing) and its pointer-down/click/double-click never reach the card or start a drag or a pan.
- **CARD-049** Renaming starts on a double-click on the title text, or the card menu's `Rename` (both owned by 09); a single click on the title opens the card only after a 240ms wait so a double-click can cancel it (09).
- **CARD-050** Cards carry no visible connection handles (React Flow's two 1×1 transparent handles at top-centre and bottom-centre exist only for edges; a native port does not need them). Lineage edges attach to whichever sides of the two cards face each other (`LineageEdge.tsx`, owned by 09) and are drawn per CARD-051/052.
- **CARD-051** A lineage edge (parent → merged child, both active) is a bezier stroke `--border-strong` (light `rgba(0,0,0,0.18)`, dark `rgba(255,255,255,0.18)`) 1.5px wide; while the child's status is `working` it is `--status-working` and dashed `5 5` with the dash offset animating from 10 to 0 every 0.5s linear, infinite.
- **CARD-052** A view → session edge (session started by a view's page) is 2px, accent at 70% (light `rgba(0,122,255,0.7)`, dark `rgba(10,132,255,0.7)`), dashes `6 4`, round caps, always animated: dash offset 10 → 0 every 1s linear, infinite ("dashes marching out of the view").
- **CARD-053** When the board moves a card (archive, restore, stack deal-out, layout), its node position glides over 420ms `cubic-bezier(0.2,0.8,0.2,1)`; a card under the pointer's drag follows directly with no glide, and no card glides while the archive or a section is being dragged.
- **CARD-054** On a server board, a card a member started (`createdBy`) says so: its title's tooltip is the title, a line break and `Started by <login>`, and its menu ends with a separator and a disabled `Started by <login>` row. A card with no `createdBy` (every local card) shows neither.
- **CARD-055** While a server card's worktree is set up (`setup.state` `running`), its preview reads `Setting up…` and the setup's last line (`Setting up… <line>`), styled as `No messages yet`; once it failed, `Setup failed: <line>` (`Setup failed` with no line). This comes before a terminal's folder and before Claude's reply.
- **CARD-056** While something listens on a server card's preview port (`previewLive`), its heading ends with an **Open preview** glyph (`AppWindowMac` 13px, stroke 2, `--accent`, tooltip `Open preview (port N)`) and its menu has `Open preview` after the open items; both open `http://<server's host>:<port>` in the browser and never open the card.
- **CARD-057** A server card's footer shows the transcript's branch, else its worktree's branch, and a server terminal's preview shows its worktree's folder rather than the project's.
- **CARD-058** A server card that took a Claude account from the pool (`accountLabel`, SRV-101) says which: its title's tooltip adds a line `Claude account: <label>` after `Started by <login>`, and its menu ends with a disabled `Claude account: <label>` row after `Started by`.
- **CARD-059** A server card with an `accountNote` (SRV-104) shows it as a pill under its heading, where the `merged from` chip goes (after it when both are there, and the preview clamps to 3 lines either way): `Moved to <label>` (or `Resumed on <label>`) in `--accent` on 10% accent with an arrow, tooltip `Its turn hit its Claude account’s limit, so it carried on on another.`; `Waiting for a Claude account until <time>` in `--status-waiting` on 10% of it with a loader, tooltip `<the note>. Every Claude account on the server is at its limit; it carries on by itself once one is free.` An open card's title bar shows the same pill after its title (`Waiting for an account` there, the time in its tooltip).

### Expanded card — XCARD

- **XCARD-001** A card renders expanded iff `card.expanded && !card.archived`; its node size is `card.w`×`card.h`, defaulting to 720×480 when unset.
- **XCARD-002** The expanded card fills its node, has no padding and no gap, clips its children to its 14px rounded corners, and uses the same surface, hover, waiting, selected and dragging shadows as CARD-002…006 except that it never scales while dragged.
- **XCARD-003** The title bar is 40px tall, horizontal flex centred vertically, 9px gap, padding 0 6px 0 14px, never shrinking; it contains the heading items (dot or shell glyph, title or rename field, live glyph), then the usage stats (Claude cards only), then the tool group pushed to the right edge.
- **XCARD-004** In the title bar the title is a single line truncated with `…`, 13px (`--font-display`, weight 600, line-height 1.3, letter-spacing -0.01em); the status dot, shell glyph and live glyph are vertically centred (no top offset). The rename field keeps 15px (CARD-045).
- **XCARD-005** Claude cards show usage stats (shared component, also in 12) when `card.usage` exists: model (`--text-secondary`, weight 500, tooltip `Model`), context meter + `Ctx N%` (tooltip `<tokens> of <size> tokens in context` or `Context window in use`), running time (tooltip `Time the session has been running`), cost (tooltip `Cost so far`) — details in UI details §Usage stats.
- **XCARD-006** When the title bar is ≤560px wide the model item is hidden; when ≤440px the whole usage stats group is hidden.
- **XCARD-007** The tool group (right-aligned, 2px gap) contains in order: Agents (only with a deck), Mic (Claude only), Resume/Restart (only when resumable), Stop (Claude and live), Archive (Claude only), Open in a window, Collapse.
- **XCARD-008** Each tool button is at least 28×28, radius 6px, transparent, icon/text `--text-secondary`, 12px weight 500, 5px gap between icon and text; hover: background `--bg-hover`, colour `--text-primary`; pressed: background `--bg-pressed` (light `rgba(0,0,0,0.09)`, dark `rgba(255,255,255,0.12)`); a button with `aria-expanded=true` (Agents while the fan is open) shows `--bg-pressed` and `--text-primary`; background transition 120ms `ease`.
- **XCARD-009** The Agents tool (lucide `Bot` 14px stroke 2, horizontal padding 7px, accessible name `Agents`) behaves like CARD-037 (point fans, click toggles pin, same tooltip) and shows a running-count badge when R > 0: min 16×16, padding 0 4px, radius 8px, background working at 18% (light `rgba(255,149,0,0.18)`, dark `rgba(255,159,10,0.18)`), text `--status-working`, 11px weight 600, line-height 16px, tabular.
- **XCARD-010** The Mic tool (05) appears on Claude cards only, styled as a tool button; text it transcribes is typed into the session's prompt (not sent), with `starting = !card.live`, and the card is woken (XCARD-025).
- **XCARD-011** When the card is resumable (`started && !live && phase !== 'loading'`), a labelled tool shows: Claude → lucide `Play` 12px stroke 2.2 + `Resume`, tooltip `Resume the session`; terminal → lucide `RotateCcw` 12px stroke 2.2 + `Restart`, tooltip `Start a new shell in the project folder`; padding 0 9px, text `--text-primary`; clicking wakes the card (XCARD-025).
- **XCARD-012** A live Claude card shows Stop (lucide `Square` 14px stroke 2, tooltip `Stop the session. The card stays, and Resume picks it up.`, accessible name `Stop`); clicking stops the session through the board's stop action, which asks first only when the session is not `done` (copy in Edge cases) and keeps the card.
- **XCARD-013** Claude cards show Archive (lucide `Archive` 14px stroke 2, tooltip `Archive (⌫)`/`Archive (Del)`, accessible name `Archive`), behaving as CARD-022.
- **XCARD-014** Every expanded card shows Open in a window (lucide `AppWindowMac` 14px stroke 2, tooltip and accessible name `Open in a window`), which opens the session in the terminal modal (12).
- **XCARD-015** Every expanded card shows Collapse (lucide `Minimize2` 14px stroke 2, accessible name `Collapse`, tooltip `Collapse (E). The session keeps running.` or, for terminals, `Collapse (E). The shell keeps running.`), which folds the card back to compact (`expand([id], false)`); the process keeps running.
- **XCARD-016** Clicks and double-clicks in the tool group never reach the card, and pressing there never drags it.
- **XCARD-017** The title bar (outside the tool group and rename field) is the card's drag handle; a click on it focuses the card's terminal (09); a double-click on the title text starts renaming (09).
- **XCARD-018** The body below the title bar fills the rest of the card, padding 8px top / 4px right / 8px bottom / 12px left, background `--term-bg` `#1e1e20` in both themes, I-beam cursor; pressing in it never drags the card; its clicks/double-clicks never reach the card; pointer-down in it focuses the terminal when the terminal is enabled. The terminal (12) fills the padded area.
- **XCARD-019** On first render the card is "started" iff the session is live, or the terminal registry already has a ready terminal for it (`!needsOpen`), or a wake was requested before the card rendered.
- **XCARD-020** If the session becomes live later (started from the modal or elsewhere), the card becomes started and stays started after the process exits.
- **XCARD-021** The terminal is attached (enabled) iff started, the session is not currently shown in the terminal modal (one terminal shows in one place), and a project is loaded; while the modal has it, the card body shows only the empty `--term-bg` area (no overlay unless not started).
- **XCARD-022** While not started, a blocking overlay covers the body (`rgba(30,30,32,0.82)` both themes) with a centred `.btn`: Claude → lucide `Play` 13px + `Resume session`; terminal → lucide `SquareTerminal` 13px + `Start shell`; clicking wakes the card.
- **XCARD-023** While enabled and the terminal phase is `loading`, a non-blocking overlay (input still reaches the terminal) shows a centred pill with a spinning lucide `LoaderCircle` 14px and the text: terminal → `Starting shell…`; Claude woken while live → `Starting Claude…`; Claude woken while not live → `Resuming session…` (the label is fixed at wake time so it does not flip when the session goes live). The pill fades in over 300ms `cubic-bezier(0.2,0.8,0.2,1)` after a 150ms delay (invisible during the delay).
- **XCARD-024** While enabled and the phase is `error`, a blocking overlay shows a column: title `Couldn’t start the shell` / `Couldn’t start Claude` (U+2019 apostrophe), the error text if any (selectable, monospace), and a primary button `Try again` that re-runs the attach.
- **XCARD-025** Waking a card (`wakeCardTerminal(sessionId)`, called by Expand, by opening an expanded card from the board, by Resume/Restart/Resume session/Start shell, and by the mic) records the wake time and the current `live` flag, then: if not started → becomes started (attaches, starting or resuming the process); else if the registry needs an open and the phase is not `loading` → re-attaches; then bumps a wake counter. A wake requested for a card that has not rendered expanded yet is remembered and applied when it renders (the card then starts immediately).
- **XCARD-026** The terminal takes keyboard focus after attaching only if the last wake was less than 1500ms ago; a card restored expanded after a restart never steals focus.
- **XCARD-027** Wheel arbitration: wheel events more than 250ms apart start a new gesture. A gesture whose first event lands over an enabled card terminal scrolls that terminal for the rest of the gesture and does not pan the board; a gesture that started anywhere else keeps panning the board even when a card terminal slides under the pointer (the terminal ignores those events).
- **XCARD-028** A pinch (wheel with Ctrl held, including trackpad pinch) is never claimed by the terminal; it always reaches the board (zoom). A disabled terminal (not started / in the modal) never claims wheels.
- **XCARD-029** Expanded cards are resizable from all four edges (invisible 10px-thick grab zones centred on each edge) and all four corners (invisible 18×18 grab zones centred on each corner, above the edge zones), with resize cursors `ew-resize` (left/right), `ns-resize` (top/bottom), `nwse-resize` (top-left/bottom-right), `nesw-resize` (bottom-left/top-right); handles are present whether or not the card is selected.
- **XCARD-030** Resizing cannot go below 360×240 (no maximum); on release the card's x, y, w, h are saved rounded to integers.
- **XCARD-031** While the pointer is over the expanded card's node, an 11×11 diagonal grip appears 4px from the bottom-right corner (two white 32% diagonal stripes), fading over 120ms `ease`; it never takes the pointer.
- **XCARD-032** Expanded cards stack above compact cards (node z-index 1 vs 0; 09); the deck (DECK) and stickers (NOTEUI) render on expanded cards exactly as on compact ones, sized to the expanded node.
- **XCARD-033** An expanded server card's terminal (hosted Nodus, added after 1.0.12) is the size the session's terminal has, which follows whoever typed into it last (SRV-144, TERM-114), not the card's: a wider one is cut at the card's edge, a narrower or shorter one leaves the terminal's background beside it. Typing into it resizes the session to the card's own size first.

### Agent deck & fan — DECK

- **DECK-001** A card has a deck iff it is not archived and `card.agents` (current-run agents) is non-empty; the deck is drawn behind the card body, covering the node box, ignored by assistive tech.
- **DECK-002** The deck shows up to 3 layers: agents sorted by urgency (`waiting` first, then `working`, then `done`; ties by `startedAt` ascending); the most urgent is depth 1 (nearest, drawn last among layers), the 2nd depth 2, the 3rd depth 3; further agents are only counted.
- **DECK-003** Each layer is a full-size copy of the card shape: radius 14px, `--bg-elevated`, `--shadow-sm`, plus a 3px strip along its bottom edge (inset) in the agent's status colour (`--status-working`, `--status-waiting`, or `--status-done` for done).
- **DECK-004** At rest, layers are offset/scaled about their bottom-centre: depth 1 `translateY(7px) scale(0.955)` brightness 0.975; depth 2 `translateY(13px) scale(0.91)` brightness 0.95; depth 3 `translateY(18px) scale(0.865)` brightness 0.925 — so they peek 7/13/18px below the card.
- **DECK-005** While the pointer is over any layer, or the fan is open, the layers lift further: depth 1 → 10px, depth 2 → 19px, depth 3 → 27px (scales unchanged), and the cursor over layers is `pointer`.
- **DECK-006** Layer transforms animate over 420ms with the `ease-stack` spring; brightness over 220ms `ease`.
- **DECK-007** A layer whose agent is `waiting` glows: at 50% of a 1.1s `ease-out` cycle its outer shadow becomes `0 4px 12px` waiting at 35% (light `rgba(255,59,48,0.35)`, dark `rgba(255,69,58,0.35)`), returning to `--shadow-sm`; the bottom strip is constant; infinite.
- **DECK-008** The pointer entering any deck layer, the footer agents button (CARD-037) or the title-bar Agents tool (XCARD-009) opens the fan (hover-open) and cancels any pending close.
- **DECK-009** A click on a deck layer, the footer agents button or the Agents tool toggles the pin and keeps the fan open (hover-open); the click never opens the card; double-click never renames.
- **DECK-010** While the fan is open, the pointer leaving the card's node (card, deck, fan, stickers together) schedules a close of the hover-open after 280ms; re-entering the node or entering the fan cancels it. A pinned fan stays open regardless.
- **DECK-011** While pinned, a pointer-down anywhere outside the card's node, or Escape anywhere, closes the fan and unpins.
- **DECK-012** Toggling an already pinned fan unpins it but leaves it hover-open (it then closes per DECK-010 once the pointer leaves).
- **DECK-013** Starting to drag the card, or the card losing its last current-run agent (or being archived), closes and unpins the fan immediately.
- **DECK-014** While a card's fan is open, that card's node is drawn above every other node on the board except the raised archive panel (node z 1400).
- **DECK-015** When the fan opens it is placed to the right of the card if, on screen, it fits before the board's right edge; else to the left if it fits after the board's left edge; else inside the card over its right edge. The side is decided once per opening (not updated while panning/zooming).
- **DECK-016** Right side: 18px right of the card, top 4px above the card's top; left side: 18px left of the card, same top; inside: 52px below the card's top, 14px from its right edge. Width 260px, vertical column, 8px gap.
- **DECK-017** The fan header reads `Agents` (12px weight 600 `--text-primary`) then, baseline-aligned 8px after, `R of N running` (R > 0) or `All done` (11px `--text-tertiary`, tabular), 4px side padding; in the inside placement the header is a pill hugging its text (padding 4px 10px, radius 12px, `--bg-elevated`, `--shadow-sm`).
- **DECK-018** If the run has ≤6 agents the fan lists all of them; with more, it lists the 5 most urgent (DECK-002 order), shown in the run's original order (oldest started first).
- **DECK-019** Each fan row is a button: full width, padding 10px 12px 11px, radius 10px, `--bg-elevated`, `--shadow-md`, a 3px strip on its left edge (inset) in the agent's status colour, text `--text-secondary`, left-aligned, 4px gap between lines; tooltip is the agent's description.
- **DECK-020** Hovering a fan row adds a 1px `--border-strong` ring and swaps `--shadow-md` for `--shadow-lg` (strip kept), over 120ms `ease`.
- **DECK-021** Row line 1: small status dot + description (13px weight 600 `--text-primary`, one line, `…`), 7px gap. Line 2 (only if the agent step has a tool or text): tool name (monospace, `--text-primary`) then 6px later the text — when there is a tool, the text is one line with `…`, monospace, `--text-tertiary`; without a tool, the text wraps to at most 2 lines (breaks anywhere) in the row colour. Line 3: status label (`--status-working` when working, `--status-waiting` when waiting, else row colour), agent type, elapsed time, and `N tool`/`N tools` when the tool count > 0, wrapping with 8px column gap. Lines 2 and 3 are both indented 14px (past the dot) and set at 11px; line 3 is `--text-tertiary` with tabular figures.
- **DECK-022** The agent step: `waiting` → text `Waiting for you in the session’s terminal`; else with `background` = 1 → `Waiting for its background command to finish`; `background` = n > 1 → `Waiting for n background commands to finish`; otherwise tool = `agent.tool`, text = `agent.activity`.
- **DECK-023** The status label: `waiting` → `Needs you`; `working` → `Working`; done → `Stopped` if `agent.stopped`, else `Done`.
- **DECK-024** Elapsed = `formatDuration(updatedAt − startedAt)` for done agents, `formatDuration(now − startedAt)` otherwise; `formatDuration`: seconds rounded, negative → 0; `<60s` → `42s`; `<60m` → `4m 12s` when minutes < 10 and seconds ≠ 0, else `4m`/`12m`; otherwise `1h 5m`. While any agent is running the fan re-renders every 1s.
- **DECK-025** The last fan item is a pill button (26px tall, padding 0 10px, radius 13px, `--bg-elevated`, `--shadow-sm` → `--shadow-md` on hover, text `--accent` 11px weight 500, lucide `ArrowRight` 12px stroke 2.2 after a 5px gap, left-aligned): text `K more, and every agent so far` (K = hidden agents) when not all are listed, else `All T agent of this session` / `All T agents of this session` (T = `card.agentCount`, falling back to the run size).
- **DECK-026** The fan deals in: the header animates 380ms, each row 520ms with delay `i × 45ms + 30ms` (i = row index; the "all" pill uses i = number of rows), all with `ease-stack`, from opacity 0, `translate(dx, 10px) scale(0.92) rotate(-2.5deg)` to rest, where dx = −46px for the right placement and +46px for left/inside; items are invisible before their delay elapses.
- **DECK-027** Clicking a fan row closes the fan and opens the Agents viewer (12) on that agent; clicking the "all" pill closes the fan and opens the viewer with no agent selected.
- **DECK-028** Clicks, double-clicks and right-clicks inside the fan never reach the card or the board (no open, no rename, no context menu); pressing in the fan never drags the card; wheel over the fan never pans the board.
- **DECK-029** The fan's accessible name is `Agents`.

### Archive frame & stack — ARCH

- **ARCH-001** The archive frame, anchored at the stored archive x/y, is 344×362 folded; open with r results it is `356 + c·288 + (c−1)·32 + 28` wide (c = 2 if r > 1 else 1 → 672 or 992) and `max(362, 52 + w·216 + (w−1)·32 + 28)` tall (w = min(2, max(1, ⌈r/2⌉)) rows → 362 or 544).
- **ARCH-002** Folded, the frame has radius 24px, a 1.5px dashed border `--archive-border` (light `rgba(120,120,128,0.35)`, dark `rgba(160,160,170,0.3)`) and fill `--archive-bg` (light `rgba(120,120,128,0.07)`, dark `rgba(160,160,170,0.06)`).
- **ARCH-003** While a dragged non-terminal card's centre is over the frame ("hot"), the border becomes solid `--accent`, the fill `--archive-drop` (light `rgba(0,122,255,0.08)`, dark `rgba(10,132,255,0.12)`), the label colour `--accent`, and the hint is visible reading `Drop to archive`.
- **ARCH-004** While open, the frame is a floating panel: solid border `--border` (light `rgba(0,0,0,0.1)`, dark `rgba(255,255,255,0.1)`), fill `--archive-panel` (light `rgba(248,248,247,0.8)`, dark `rgba(38,38,41,0.8)`) over a 30px backdrop blur with saturation 1.8, shadow `--shadow-lg`.
- **ARCH-005** Background and border colour change over 220ms `ease`, shadow over 320ms `cubic-bezier(0.2,0.8,0.2,1)`; the frame's width and height animate over 460ms `cubic-bezier(0.2,0.8,0.2,1)` between folded and open sizes.
- **ARCH-006** The frame sits below every card and note (z −1) except while open and for 700ms after it closes, when it and its cards sit above everything (z 1500; results 1510; pile 1500 + (3 − depth)) (09).
- **ARCH-007** The top 52px of the frame is its header and drag handle: padding 0 22px, colour `--archive-label` (light `rgba(0,0,0,0.42)`, dark `rgba(255,255,255,0.42)`), cursor `grab` (`grabbing` while the archive is dragged), tooltip `Drag to move the archive`.
- **ARCH-008** The header label is lucide `Archive` 15px stroke 2, `Archive`, the archived-card count in a pill, and the hint, 8px apart, 13px weight 600 letter-spacing 0.01em, single line; it scales by `clamp(1/zoom, 1, 2.2)` about its left-centre so it stays legible when zoomed out.
- **ARCH-009** The hint (6px extra left margin, weight 400, 12px) reads `Drag cards out to reactivate`, or `Drop to archive` while hot; it is hidden (opacity 0) unless the header is hovered or the frame is hot, fading over 220ms `ease`.
- **ARCH-010** With no archived cards, a placeholder card outline sits at the stack spot (left 28, top 52, 288×216): radius 14px, 1.5px dashed `--archive-border`, centred text `Drag a card here to archive it` 12px `--archive-label`; it ignores the pointer.
- **ARCH-011** A click anywhere on the folded frame (not the end of a drag) opens the stack panel.
- **ARCH-012** Every archived card not dealt out as a result sits at the same spot (archive x + 28, y + 52), ordered most recent first (`max(archivedAt, meta.updatedAt ?? createdAt)` descending); depth 0 is the top card, depths 1 and 2 peek below it, deeper cards are not drawn (except briefly while sinking — 09).
- **ARCH-013** Stack depth visuals, about each card's bottom-centre: depth 1 `translateY(10px) scale(0.94)` brightness 0.955; depth 2 `translateY(20px) scale(0.88)` brightness 0.91; depth 3 (sinking) `translateY(20px) scale(0.84)` opacity 0; at depths 1–3 the card's contents (header, chip, preview, footer) are invisible, leaving plain slabs.
- **ARCH-014** Archived cards in the stack or among the results have opacity 1 (depth 3: 0), overriding CARD-008.
- **ARCH-015** Stack cards animate transform over 560ms `ease-stack`, opacity and brightness over 260ms `ease`, shadow over 220ms `cubic-bezier(0.2,0.8,0.2,1)`; their contents fade over 200ms `ease`.
- **ARCH-016** While the stack is folded and either the pointer is over the frame (with no button pressed, not dismissed — ARCH-022) or the frame is hot, the stack lifts: top card `translateY(-3px)` with `--shadow-md`; depth 1 `translateY(14px) scale(0.94)`; depth 2 `translateY(28px) scale(0.88)`.
- **ARCH-017** While lifted, the top card's lift transform and `--shadow-md` override its own drag/selected visuals — so a top card dragged with its centre still over the frame shows `translateY(-3px)` + `--shadow-md` (no accent ring, no 1.015 scale) until its centre leaves the frame.
- **ARCH-018** Only the top card and dealt-out results take the pointer; stack cards at depth ≥ 1 are click-through to the frame.
- **ARCH-019** The search field sits under the stack (left 28, top 306, 288×34); it is shown when there is at least one archived card and (the pointer is over the frame and not dismissed, or the panel is open, or the query is non-empty); hidden it is opacity 0, shifted `translateY(-6px) scale(0.97)`, and ignores the pointer; it animates opacity over 180ms `ease` and transform over 360ms `ease-stack`.
- **ARCH-020** When the search field becomes shown (or the panel opens while it is shown), it takes keyboard focus without scrolling — unless another text field (input, textarea, select, editable) already has focus; when hidden while focused, it loses focus.
- **ARCH-021** The field's placeholder is `Search N sessions` (N = archived count) and its accessible name `Search archived sessions`; spell-check off; it is out of the tab order while hidden.
- **ARCH-022** Typing sets the query, resets the results scroll to 0, and opens the panel when the trimmed query is non-empty.
- **ARCH-023** Escape in the field closes the panel, clears the query, resets the scroll, blurs the field, and dismisses the search until the pointer leaves the frame; the Escape does not reach the board.
- **ARCH-024** Enter in the field: open → blurs the field then opens the transcript preview (12) of the first result (nothing if no results); folded → opens the panel.
- **ARCH-025** ArrowDown in the field while folded opens the panel.
- **ARCH-026** A click on the search field (anywhere in its box) opens the panel and focuses the input, and does not reach the frame.
- **ARCH-027** When the query is non-empty a clear button shows at the field's right end (15×15 circle, fill `--text-tertiary`, lucide `X` 9px stroke 3 in `--bg-elevated`, accessible name `Clear search`, not focusable); pressing it keeps focus in the field; clicking clears the query and refocuses the field.
- **ARCH-028** While the field has focus it shows a 3px `--focus-ring` (light `rgba(0,122,255,0.5)`, dark `rgba(10,132,255,0.55)`) around it plus `--shadow-sm`, over 120ms `ease`.
- **ARCH-029** Open results are laid out in two columns starting at (archive x + 356, y + 52), columns 320px apart, rows 248px apart, scrolled up by the scroll offset; title matches highlight per CARD-018.
- **ARCH-030** A result entirely outside the 464px results band (from y + 52) is not drawn; one crossing the band's top or bottom is clipped at the band edge, with 24px of overflow allowed sideways (and at most 24px past the cut) so its shadow still shows.
- **ARCH-031** Results glide to their slots over 620ms `ease-stack` with a `min(slot, 4) × 30ms` stagger; while the results are being wheel-scrolled they follow without glide (scrolling flag clears 160ms after the last wheel — 09).
- **ARCH-032** A card that was hidden deep in the stack and is dealt out flies from the stack top: animation 620ms `ease-stack` with the same stagger, from `translate(fromX, fromY) scale(0.88)` opacity 0 (from = stack top − result spot) to rest; invisible before its delay.
- **ARCH-033** When the panel is open and every archived card is a result, a placeholder outline sits at the stack spot (as ARCH-010) with, centred and 3px apart, bold `1 archived session` / `N archived sessions` (13px weight 600) and `Scroll the list, or search to filter` (when the results can scroll) or `Search to filter` (12px); it fades in over 240ms `ease` after 200ms.
- **ARCH-034** A 1px vertical divider in `--separator` sits at x = 336 from the frame's left, from y = 52 to 28px above the frame's bottom; hidden folded, it fades in over 220ms `ease` 120ms after the panel opens (fades out with no delay).
- **ARCH-035** When the panel is open with zero results, the results area shows `No archived sessions match “<trimmed query>”` (curly quotes) centred in a 288×216 box at (356, 52), 13px `--text-tertiary`, fading in over 240ms `ease` after 160ms.
- **ARCH-036** When the results can scroll (more than 2 rows), a scrollbar shows 11px from the frame's right edge, top 52, height 464, 5px wide: thumb height `max(32, 464² / (464 + max))`, offset `scroll / max × (464 − thumb)`, radius 3px, `--text-tertiary` at 55% opacity; it ignores the pointer; it fades in over 240ms `ease` after 160ms.
- **ARCH-037** Pointer-down anywhere on the board outside the archive frame and outside archived cards closes the panel (as Escape, including the dismissal) (09).
- **ARCH-038** Archived cards travel with the archive while it is dragged (no glide), as do the note pad and its top sheet (09).
- **ARCH-039** Dropping a non-terminal card with its centre over the frame archives it (asking first if live and not `done`); a terminal card dropped there returns to where its drag started (09).
- **ARCH-040** The count pill always equals the number of archived cards on the board (action runs and view brains excluded upstream).

### Notes, stickers & pad — NOTEUI

- **NOTEUI-001** A note's size is its saved `w`×`h`, defaulting to 216×216 (`NOTE_W`×`NOTE_H`); resizing cannot go below 120×120 (no maximum).
- **NOTEUI-002** A note's paper is its colour's hue (`FOLDER_HUES[color].frontTop`) mixed 72% into the theme paper (`#ffffff` light, `#b4b4b8` dark) — resolved hex in UI details §Note colours.
- **NOTEUI-003** Over the paper, a vertical gradient from `rgba(0,0,0,0.035)` at the top edge to transparent at 22px darkens the strip where it sticks.
- **NOTEUI-004** The note box: padding 16px 18px, radius 4px, shadow `--note-shadow`, text colour `--note-ink` `rgba(0,0,0,0.82)` (both themes).
- **NOTEUI-005** A selected note, and a note being written in, draws a 2px `--accent` ring outside `--note-shadow`.
- **NOTEUI-006** A note being dragged rotates −1.5° and scales 1.02 about its centre with `--note-shadow-lifted` (plus the 2px accent ring if selected).
- **NOTEUI-007** Shadow and transform changes animate over 220ms `cubic-bezier(0.2,0.8,0.2,1)`.
- **NOTEUI-008** The note text is `--font-ui` 15px, line-height 1.4 (21px), whitespace and newlines preserved, wrapping anywhere, clipped to the note's content box, with the bottom 18px of that box fading linearly to transparent; it is not selectable.
- **NOTEUI-009** A note with empty text shows `Double-click to write` in `--note-ink-faint` `rgba(0,0,0,0.38)`; the pad's top sheet shows nothing.
- **NOTEUI-010** Double-clicking a note (not the pad's top sheet) starts writing in it; so does Enter with exactly one note and no card selected (09) and the note menu's `Edit` (09).
- **NOTEUI-011** Writing replaces the text with a multi-line field holding the current text, focused without scrolling, caret at the end; the field has no border, background or outline, does not resize, I-beam cursor, caret `--note-ink`, same typography as NOTEUI-008, placeholder `Type a note…` in `--note-ink-faint`.
- **NOTEUI-012** In the field, Enter inserts a newline; Escape, or Enter with ⌘ (macOS) / Ctrl (Windows), ends writing (focus leaves the field, default action suppressed).
- **NOTEUI-013** A click anywhere else (focus loss) ends writing; ending writing saves the current text if it changed since the last save and clears the renaming target.
- **NOTEUI-014** While typing, the text is saved 500ms after the last keystroke if it changed since the last save; when the field is destroyed for any other reason (board closed mid-note with ⌘W, note removed), pending text is saved.
- **NOTEUI-015** A save made after the board has switched to another project (or closed) is written straight to the note's own project (`board.saveNote(projectId, note)`) rather than the current board.
- **NOTEUI-016** The field consumes every key (board shortcuts never fire), its pointer-down/click/double-click never reach the note or start a drag or pan, and wheel over it scrolls the field, not the board.
- **NOTEUI-017** Ending writing never deletes a note, even an empty one.
- **NOTEUI-018** A note written by a session whose card is on this board is signed along its bottom (when not writing): `— <card title>` (em dash + space, CARD-017 title); tooltip `Written by Claude in “<card title>”. Click to open that session.`; clicking opens that session as clicking its card would (09).
- **NOTEUI-019** A note written from another project's board (that project still exists) and whose writer card is not here is signed `— <fromTitle> · <project name>`; tooltip `Written by Claude in “<fromTitle>”, on the <project name> board. Click to open that session.`; clicking switches to that project and shows the session's card.
- **NOTEUI-020** The signature sits 18px from the left, 46px from the right, 13px from the bottom; 12px `--font-ui`, line-height 16px, one line with `…`, `--note-ink-faint`, default cursor; hover → `--note-ink` with an underline offset 2px; colour transition 120ms `ease`; clicks/double-clicks on it do not reach the note; pressing it never drags the note.
- **NOTEUI-021** When a note is signed, its text box height is the content height minus 22px.
- **NOTEUI-022** A note with non-blank text (not the pad's top sheet, not being written) has a ▶ button in its bottom-right corner (10px in), invisible (opacity 0, scale 0.85) until the pointer is over the note node, then opacity 1 / scale 1; hidden again while the note is dragged.
- **NOTEUI-023** The ▶ button is a 28×28 circle with 2px extra left padding (optical centring), background `--note-ink` at 80% (`rgba(0,0,0,0.656)`), icon = the note's paper colour, shadow `0 2px 6px rgba(0,0,0,0.18)`, lucide `Play` 12px stroke 2.5 filled with the icon colour; hover → background `--accent`, icon `#ffffff`; pressed → scale 0.92; opacity 120ms `ease`, transform 220ms `cubic-bezier(0.34,1.4,0.64,1)`, background 120ms `ease`; tooltip and accessible name `Start a Claude session on this note`.
- **NOTEUI-024** ▶ (or the note menu's `Start session`) starts a Claude session with the note's text as its first prompt in auto mode, without opening its window: ignored if the note is already stuck to a session, its text is blank, or a start for it is in flight; the card is placed at the nearest free spot to the note's top-left (searched with a new card's expanded footprint 720×960, clear of all cards, the other free notes, views, and the archive + pad; the note itself is not an obstacle, so the card can take its place); it joins the section under it; the note (as it is now) then sticks to the new card; the card is panned into view; failure shows the toast `Couldn't start a session: <message>` (ASCII apostrophe).
- **NOTEUI-025** A selected note (not being written) shows resize controls: four 9×9 corner handles (1.5px `--accent` border, radius 2px, `--bg-elevated` fill) centred on the corners, and invisible 10px grab zones along the four edges; cursors as XCARD-029; the pad's top sheet never has them.
- **NOTEUI-026** On resize release the note's x, y, w, h are saved rounded to integers.
- **NOTEUI-027** Free notes sit above cards (z 2), below the open archive panel and any selected card (09 select elevation).
- **NOTEUI-028** When the board moves a note (pad click, bounce back) it glides over 420ms `cubic-bezier(0.2,0.8,0.2,1)`; a note being dragged, and notes/pad while the archive is dragged, follow directly.
- **NOTEUI-029** A note dropped with its centre on the archive frame or the pad returns to where its drag started; notes never join sections (09).
- **NOTEUI-030** Deleting notes (⌫/Del with notes selected, note menu `Delete note`/`Delete N notes`) removes them immediately with no confirmation, written on or not. (docs/how-it-works.md §Notes claims "Deleting a note that has writing on it asks first" — the code at BoardView.tsx:1209 does not; the code wins.)
- **NOTEUI-031** New notes (pad, menu, board tools default) are `yellow`; a note's colour is one of `blue`, `purple`, `pink`, `red`, `orange`, `yellow`, `green`, `graphite` (recolour via the note menu, 09).
- **NOTEUI-032** A note whose `sessionId` is a card currently on the board (active) is drawn as a sticker on that card instead of on the board; a note whose card is not on the board stays a free note; an archived card's stickers are not drawn (they reappear on restore); removing a card drops its notes back onto the board at the card's position (active) or where the note was (archived) (03).
- **NOTEUI-033** Stickers line up along the card's top edge from the right: sticker i (0-based, in note order) has its right edge `18 + 30·i` px from the card's right edge and its bottom edge 6px below the card's top edge, above the card (z 2 within the node).
- **NOTEUI-034** A sticker is 84×54, padding 6px 8px, radius 3px, paper per NOTEUI-002 with a top gradient `rgba(0,0,0,0.04)` → transparent over 10px, `--note-shadow`, ink `--note-ink`, rotated 3° about its bottom-centre.
- **NOTEUI-035** Hovering a sticker rotates it to 1° and lifts it 2px with `--note-shadow-lifted`, over 220ms `cubic-bezier(0.2,0.8,0.2,1)`; its tooltip is the note's full text.
- **NOTEUI-036** Sticker text: 11px, line-height 1.25, whitespace preserved, wrapping anywhere, clamped to 3 lines with `…`, not selectable.
- **NOTEUI-037** Hovering a sticker reveals (opacity 0 → 1, 120ms `ease`) a 16×16 circle at its top-right corner (6px outside each edge), fill `--note-ink`, lucide `X` 9px stroke 3 in `--note-paper`; tooltip and accessible name `Delete note`; clicking deletes the note immediately with no confirmation and does not open the card; double-click does nothing on the card.
- **NOTEUI-038** If the card was created less than 4000ms before the render (a session just started from the note), the sticker plays a landing animation: 520ms `cubic-bezier(0.34,1.4,0.64,1)` from `translateY(-14px) rotate(-6deg) scale(1.35)` opacity 0 to rest; skipped under reduced motion.
- **NOTEUI-039** Pressing/dragging a sticker (outside its ×) drags the card; clicking it acts as clicking the card.
- **NOTEUI-040** The note pad is a frame at (archive x, archive y + 386), 344×296, never selectable or draggable by itself, z −1, styled like the folded archive (radius 24px, 1.5px dashed `--archive-border`, fill `--archive-bg`).
- **NOTEUI-041** The pad header (52px, padding 0 22px, `--archive-label`, no grab cursor) shows lucide `StickyNote` 15px stroke 2, `Notes`, and the always-visible hint `Drag one onto the board` (6px extra left margin, 12px weight 400), 8px apart, 13px weight 600 letter-spacing 0.01em; it scales by `clamp(1/zoom, 1, 2.2)` like ARCH-008.
- **NOTEUI-042** The pad draws three yellow note sheets at (64, 52) inside it, each 216×216: the deepest `translateY(8px) scale(0.97)` brightness 0.88, the middle `translateY(4px) scale(0.985)` brightness 0.94, the top flat — about their bottom-centre — so a block of paper shows 4px and 8px below the top sheet.
- **NOTEUI-043** On top of the pad sits the "draft" top sheet: a real yellow note node (216×216, z 2, not selectable, empty, no ▶, no resize), cursor `grab` (`grabbing` while dragged), tooltip `Drag a note onto the board, or click for one beside the pad`.
- **NOTEUI-044** Hovering the top sheet peels it: `rotate(-2deg) translateY(-2px)` about its bottom-left corner with `--note-shadow-lifted`; while dragged: `rotate(-1.5deg) scale(1.02)` (bottom-left origin) with `--note-shadow-lifted`.
- **NOTEUI-045** Dropping the top sheet with its centre outside the pad and the archive creates a new yellow note exactly there (rounded), immediately in writing mode, and a fresh top sheet appears on the pad.
- **NOTEUI-046** Dropping the top sheet with its centre on the pad or the archive returns it to the pad spot.
- **NOTEUI-047** A single click on the top sheet creates a new note in writing mode at the nearest free spot to (pad right + 32, pad y + 52) (clear of cards, notes, views, archive and pad), gliding there from the pad, and pans it into view; the second click of a double-click does nothing.
- **NOTEUI-048** Right-clicking the pad or its top sheet offers only `New note` (same as NOTEUI-047) (09).
- **NOTEUI-049** The pad and its top sheet move with the archive (ARCH-038).
- **NOTEUI-050** On a server board, hovering a note a member wrote (`createdBy`, the member who saved it first, or the starter of the session whose board tools wrote it) shows the tooltip `Written by <login>`; never on the pad's top sheet or while writing.

## UI details

### Resolved tokens used by this domain

| Token | Light | Dark |
| --- | --- | --- |
| `--bg-window` | `#f5f5f4` | `#1e1e20` |
| `--bg-canvas` | `#f3f3f1` | `#19191b` |
| `--bg-elevated` | `#ffffff` | `#2a2a2d` |
| `--bg-sunken` | `rgba(0,0,0,0.035)` | `rgba(255,255,255,0.04)` |
| `--bg-hover` | `rgba(0,0,0,0.05)` | `rgba(255,255,255,0.07)` |
| `--bg-pressed` | `rgba(0,0,0,0.09)` | `rgba(255,255,255,0.12)` |
| `--text-primary` | `rgba(0,0,0,0.88)` | `rgba(255,255,255,0.9)` |
| `--text-secondary` | `rgba(0,0,0,0.56)` | `rgba(255,255,255,0.58)` |
| `--text-tertiary` | `rgba(0,0,0,0.36)` | `rgba(255,255,255,0.36)` |
| `--text-on-accent` | `#ffffff` | `#ffffff` |
| `--border` | `rgba(0,0,0,0.1)` | `rgba(255,255,255,0.1)` |
| `--border-strong` | `rgba(0,0,0,0.18)` | `rgba(255,255,255,0.18)` |
| `--separator` | `rgba(0,0,0,0.07)` | `rgba(255,255,255,0.07)` |
| `--accent` | `#007aff` | `#0a84ff` |
| `--accent-hover` | `#0068d9` | `#3396ff` |
| `--focus-ring` | `rgba(0,122,255,0.5)` | `rgba(10,132,255,0.55)` |
| `--status-working` | `#ff9500` | `#ff9f0a` |
| `--status-done` | `#007aff` | `#0a84ff` |
| `--status-waiting` | `#ff3b30` | `#ff453a` |
| `--archive-bg` | `rgba(120,120,128,0.07)` | `rgba(160,160,170,0.06)` |
| `--archive-border` | `rgba(120,120,128,0.35)` | `rgba(160,160,170,0.3)` |
| `--archive-label` | `rgba(0,0,0,0.42)` | `rgba(255,255,255,0.42)` |
| `--archive-drop` | `rgba(0,122,255,0.08)` | `rgba(10,132,255,0.12)` |
| `--archive-panel` | `rgba(248,248,247,0.8)` | `rgba(38,38,41,0.8)` |
| `--note-paper` | `#ffffff` | `#b4b4b8` |
| `--note-ink` | `rgba(0,0,0,0.82)` | same |
| `--note-ink-faint` | `rgba(0,0,0,0.38)` | same |
| `--term-bg` | `#1e1e20` | same |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)` |
| `--shadow-md` | `0 4px 14px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(255,255,255,0.1)` |
| `--shadow-lg` | `0 18px 50px rgba(0,0,0,0.18), 0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5), 0 0 0 0.5px rgba(255,255,255,0.12)` |
| `--note-shadow` | `0 1px 1.5px rgba(0,0,0,0.08), 0 8px 16px -8px rgba(0,0,0,0.3)` | `0 1px 1.5px rgba(0,0,0,0.3), 0 8px 18px -8px rgba(0,0,0,0.6)` |
| `--note-shadow-lifted` | `0 2px 4px rgba(0,0,0,0.08), 0 20px 32px -12px rgba(0,0,0,0.36)` | `0 2px 4px rgba(0,0,0,0.3), 0 20px 34px -12px rgba(0,0,0,0.7)` |

Shadow syntax is CSS `x y blur spread colour`; CSS blur radius B is a Gaussian with σ = B/2. Shadows are drawn outside the box only (outer box-shadow), except where marked **inset**.

Radii: `--radius-sm` 6px, `--radius-md` 10px, `--radius-lg` 14px.

Easings:
- `--ease-out` = `cubic-bezier(0.2, 0.8, 0.2, 1)`
- `--ease-spring` = `cubic-bezier(0.34, 1.4, 0.64, 1)` (overshoots)
- `--ease-stack` = piecewise-linear `linear()` with 29 evenly spaced stops: `0, 0.05, 0.1697, 0.3218, 0.4803, 0.6281, 0.7553, 0.8579, 0.9355, 0.9902, 1.0254, 1.0452, 1.0534, 1.0536, 1.0487, 1.0411, 1.0326, 1.0242, 1.0166, 1.0104, 1.0055, 1.0019, 0.9995, 0.998, 0.9973, 0.9971, 0.9972, 0.9975, 1` (stop k at input k/28; ~5% overshoot)
- CSS `ease` = `cubic-bezier(0.25, 0.1, 0.25, 1)`; `ease-in-out` = `cubic-bezier(0.42, 0, 0.58, 1)`; `ease-out` keyword = `cubic-bezier(0, 0, 0.58, 1)`; `linear`.
- Durations: `--dur-fast` 120ms, `--dur-med` 220ms.
- Keyframes that define only `50%` take their 0%/100% values from the element's resting style; the easing applies **per segment** (0→50% and 50→100% each eased).
- `fill-mode: backwards`/`both`: during the delay the element shows the `from` frame (i.e. invisible when `from` has opacity 0).

Fonts (inherited by everything here from `body`: `--font-ui` 13px, line-height 1.4, antialiased, text not selectable unless stated):
- `--font-ui`: macOS system font (SF Pro, optical size automatic); Windows `Segoe UI Variable Text` → `Segoe UI`.
- `--font-display`: macOS system font (same as UI on macOS); Windows `Segoe UI Variable Display` → `Segoe UI`.
- `--font-mono`: macOS `SF Mono` → `Menlo`; Windows `Cascadia Mono` → `Consolas`.
- Type sizes: xs 11px, sm 12px, md 13px, lg 15px.

Reduced motion (system setting; macOS "Reduce motion", Windows "Show animations" off): every animation and transition in this spec completes instantly (global rule sets all durations to 0.01ms); the sticker landing (NOTEUI-038) is removed entirely.

Keyboard focus ring (global `:focus-visible`): focusable buttons here (corner buttons, card tools, footer agents, fan rows/pill, ▶, note signature, sticker ×) draw `outline: 2px solid --focus-ring` offset 2px when focused from the keyboard. The rename field, note field and archive search input suppress it.

### Session card (compact)

Box: 288×216, `box-sizing: border-box`, padding `14px 16px 11px` → content 256×191. Column flex, gap 8px. Radius 14. Background `--bg-elevated`.

State → shadow / transform / opacity (precedence top wins):

| State | box-shadow | transform | opacity |
| --- | --- | --- | --- |
| stack-lift top card (ARCH-016/017) | `--shadow-md` | `translateY(-3px)` | — |
| dragging (compact) | `0 0 0 2px --accent, --shadow-lg` | `scale(1.015)` (centre) | archived → 1 |
| dragging (expanded) | same | none | — |
| selected | `0 0 0 2px --accent, --shadow-md` | — | archived → 1 |
| waiting | `0 0 0 1px rgba(255,59,48,0.45)` / dark `rgba(255,69,58,0.45)`, then `--shadow-md` | — | — |
| hover | `--shadow-md` | — | archived → 1 |
| base | `--shadow-sm` | none | archived 0.78 (stack overrides, ARCH-014) |

Transitions: `box-shadow 220ms ease-out(token)`, `opacity 220ms ease`, `transform 220ms ease-out(token)`.

Header (`.card-head`): flex row, `align-items: flex-start`, gap 9px, min-height 20px.
- Status dot: 10×10 circle, `margin-top: 5px` → dot top at 5px, centred on the first title line (19.5px line, centre 9.75px).
- Shell glyph: `SquareTerminal` 14px stroke 2, `margin-top: 3px`, colour tertiary / primary when live.
- Title (`h3`): flex 1, min-width 0, margin 0, `--font-display` 15px, weight 600, line-height 1.3 (19.5px), letter-spacing −0.15px, `--text-primary`, 2-line clamp with `…`, `overflow-wrap: anywhere`. Max visible height 39px.
- Live glyph: `SquareTerminal` 13px stroke 2, `margin-top: 3px`, flex none, `--text-tertiary`.
- Title match mark: no background, colour `--accent`.

Corner (`.card-corner`, compact, not archived): absolute top 9 right 9, row, gap 2px. Each button 26×26 grid-centred, radius 6, no border, background `--bg-elevated`, icon 13px stroke 2 `--text-secondary`, opacity 0 → 1 on card hover (`opacity 120ms ease, background 120ms ease`). Button hover: background `color-mix(--text-primary 7%, --bg-elevated)` = light `rgba(239,239,239,0.992)` ≈ `#efefef`; dark `rgba(56,56,58,0.993)` ≈ `#38383a`; icon `--text-primary`. Order: Archive (`Archive`), Expand (`Maximize2`). When visible they cover the live glyph (glyph sits 16px from the right edge; Expand spans 9–35px from the right).

Chip: inline-flex, centred, gap 4, `align-self: flex-start`, height 19, padding 0 7, radius 999, bg accent 10% (light `rgba(0,122,255,0.10)`, dark `rgba(10,132,255,0.10)`), colour `--accent`, 11px weight 500; icon `GitMerge` 11px stroke 2.2.

Preview (`p.card-preview`): margin 0, min-height 0, not flex-grown, `--text-secondary`, 12px, line-height 1.45 (17.4px), clamp 4 (3 after a chip), `overflow-wrap: anywhere`, normal whitespace (collapse). Kinds: `is-prompt` italic; `is-empty` `--text-tertiary`; `is-path` `--font-mono` 11px, clamp 2.

Footer (`.card-foot`): flex row centred, gap 12, `margin-top: auto`, `padding-top: 9px`, `border-top: 0.5px solid --separator`, `--text-tertiary`, 11px, tabular-nums, nowrap; line-height 1.4 (15.4px).
- Item: inline-flex centred, gap 4. Icons 11px stroke 2.2: `GitBranch`, `MessageSquare`, `Bot`.
- Branch: min-width 0; name max-width 130px, `…`.
- Agents button: height 18, margin −2px −5px, padding 0 5px, no border, radius 5, transparent; hover bg `--bg-hover` + `--text-primary`; `background 120ms ease`. Font inherits (11px tertiary).
- Running badge: inline-flex centred, gap 4, margin-left 3px, `--text-secondary`; small dot 7×7.
- Time: margin-left auto.

Title rename field: see CARD-045 (height 24, margin −2px 0, padding 0 5px, radius 5, bg `--bg-window`, ring `0 0 0 2px --accent`, `--font-display` 15px 600, no outline, `user-select: text`).

### Status dot (shared with folders/rail/modal header; spec here because cards define it)

| Variant | Size | Fill | Ring (box-shadow spread) | Animation |
| --- | --- | --- | --- | --- |
| working | 10 | `#ff9500` / `#ff9f0a` | 3px `rgba(255,149,0,0.22)` / `rgba(255,159,10,0.22)` | `status-breathe` 1.6s ease-in-out infinite: 50% → 5px at 0.10 alpha |
| done | 10 | `#007aff` / `#0a84ff` | none | none |
| waiting | 10 | `#ff3b30` / `#ff453a` | (animated) | `status-pulse` 1.1s ease-out infinite: 0% `0 0 0 0` at 0.55 alpha → 100% `0 0 0 8px` transparent |
| small working | 7 | as working | 2px at 0.22 | `status-breathe-small` 1.6s ease-in-out infinite: 50% → 3.5px at 0.10 |
| small waiting / done | 7 | as above | as above | as above (pulse still 8px) |

All dots: circle, `flex: none`. The waiting pulse colour fades in premultiplied space (hue constant, alpha 0.55 → 0).

### Expanded card

Node: `card.w ?? 720` × `card.h ?? 480`, min 360×240. Card fills node: padding 0, gap 0, `overflow: hidden` (children clipped to radius 14), `isolation: isolate`. Shadows as compact; no drag scale.

Title bar (`.card-xhead`): height 40, flex row centred, gap 9, padding `0 6px 0 14px`, flex none. Container width thresholds: ≤560px hide `.usage-model`; ≤440px hide `.usage`.
- Title: `display: block`, single line, `…`, 13px (font-display, 600, lh 1.3 = 16.9px, letter-spacing −0.13px).
- Dot/glyphs: no top margin (centred).

Usage stats (`.usage`; component `UsageStats`, shared with 12): flex row centred, gap 10, flex none, 11px, `--text-tertiary`, tabular-nums, nowrap. Rendered only when `usage` exists. Items inline-flex gap 5, in order:
1. Model (if set): text `usage.model`, `--text-secondary` weight 500.
2. Context (if `contextPercent` defined): meter 22×4 radius 2 bg `--border`, fill `currentColor` width `min(100, pct)%` (left-anchored, inheriting radius); then `Ctx <round(pct)>%`. Level ≥90 → colour `--status-waiting`; ≥75 → `--status-working`; else tertiary. Tooltip `<a> of <b> tokens in context` with tokens formatted `950`, `9.5k` (<10k one decimal, `.0` dropped), `106k`, `1M`/`1.5M`; falls back to `Context window in use` when tokens or size unknown.
3. Running time (if `durationMs` defined): `formatDuration(durationMs + (live ? max(0, now − usage.at) : 0))`, ticking every 1s while live.
4. Cost (if `costUsd` defined): `$X.XX` when < 100, else `$N` rounded.

Tool group (`.card-tools`): flex row centred, gap 2, flex none, margin-left auto.
- Tool button: inline-flex centred, gap 5, min-width 28, height 28, padding 0, no border, radius 6, transparent, `--text-secondary`, 12px weight 500, `background 120ms ease`. Hover bg `--bg-hover` + primary; active bg `--bg-pressed`; `aria-expanded=true` bg `--bg-pressed` + primary.
- Labelled (`Resume`/`Restart`): padding 0 9px, `--text-primary`; icon 12px stroke 2.2.
- Agents: padding 0 7px; icon `Bot` 14/2; badge (XCARD-009).
- Icons: `Square` 14/2 (Stop), `Archive` 14/2, `AppWindowMac` 14/2, `Minimize2` 14/2. Mic: `Mic` 14/2 idle, `Loader2` 14 spinning while transcribing, filled `Square` 9 while recording (recording look owned by 05: circle, `--status-waiting` fill, white, level ring).

Body (`.card-term`): relative, flex 1, min-height 0, padding `8px 4px 8px 12px`, bg `#1e1e20`, cursor text. Terminal container fills the content box.

Overlays (cover the body's padding box, `inset: 0`, grid-centred):
- Blocking: pointer-events on, bg `rgba(30,30,32,0.82)` (both themes).
- Non-blocking (loading): pointer-events off, no bg.
- Start button `.btn`: inline-flex centred, gap 6, height 28, padding 0 12, radius 6, border 0.5px `--border-strong`, bg `--bg-elevated`, `--shadow-sm`, 13px weight 500, nowrap, text `--text-primary`; hover bg `--bg-hover` (translucent — composited over the dark overlay); active `--bg-pressed`; `background 120ms ease`; icon 13px (default stroke 2).
- Loading pill: inline-flex centred, gap 8, height 30, padding 0 14, radius 999, bg `rgba(255,255,255,0.07)`, ring `0 0 0 0.5px rgba(255,255,255,0.1)`, text `rgba(255,255,255,0.72)` 12px; animation `term-fade-in` 300ms `--ease-out` 150ms delay backwards (opacity 0 → 1). Spinner `LoaderCircle` 14 rotating 360° every 0.9s linear infinite.
- Error column: flex column centred, gap 10, max-width 460, padding 24, centred text `rgba(255,255,255,0.85)`; title 15px 600; detail `--font-mono` 12px `rgba(255,255,255,0.55)`, pre-wrap, break-word, selectable (I-beam); button `.btn.btn-primary`: bg `--accent`, transparent border, text `#ffffff`, hover `--accent-hover`.

Resize controls (React Flow `NodeResizer`, always visible on expanded cards):
- Edge zones: left/right 10px wide × full height, top/bottom 10px tall × full width, each centred on its edge (half inside, half outside), transparent, z 2.
- Corner zones: 18×18 centred on each corner, transparent, no border, z 3.
- Cursors per XCARD-029.

Grip (`::after`): absolute right 4 bottom 4, 11×11, z 1, `linear-gradient(135deg, transparent 0–46%, rgba(255,255,255,0.32) 46–54%, transparent 54–72%, rgba(255,255,255,0.32) 72–80%, transparent 80%)` (two parallel diagonal stripes, top-right → bottom-left, toward the corner), opacity 0 → 1 while the node is hovered, `opacity 120ms ease`, pointer-events none. Same in both themes.

### Agent deck & fan

Session node wrapper (`.session-node`): relative, 100%×100% of the node; children in paint order: deck layers, fan (z 5), card body, stickers (z 2).

Deck (`.agent-deck`): absolute inset 0, pointer-events none (layers re-enable).
Layer (`.agent-deck-layer`): absolute inset 0, radius 14, bg `--bg-elevated`, shadow `inset 0 -3px 0 <edge>, --shadow-sm`, `transform-origin: 50% 100%`, transitions `transform 420ms ease-stack, filter 220ms ease`. Edge: working `#ff9500`/`#ff9f0a`, waiting `#ff3b30`/`#ff453a`, done `#007aff`/`#0a84ff`.

| Depth | Rest transform | Lifted transform | brightness | Resolved layer fill (light / dark) |
| --- | --- | --- | --- | --- |
| 1 (most urgent) | `translateY(7px) scale(0.955)` | `translateY(10px) scale(0.955)` | 0.975 | `#f9f9f9` / `#29292c` |
| 2 | `translateY(13px) scale(0.91)` | `translateY(19px) scale(0.91)` | 0.95 | `#f2f2f2` / `#28282b` |
| 3 | `translateY(18px) scale(0.865)` | `translateY(27px) scale(0.865)` | 0.925 | `#ececec` / `#27272a` |

Brightness multiplies the whole painted layer (fill, edge strip and its shadow) by the factor. Waiting layer: `deck-waiting` 1.1s ease-out infinite; at 50%: `inset 0 -3px 0 <edge>, 0 4px 12px rgba(255,59,48,0.35)` (dark `rgba(255,69,58,0.35)`).

Fan (`.agent-fan`): absolute, z 5, width 260, column, gap 8, cursor default.
- right: `top: -4px; left: calc(100% + 18px)`; deal-x −46px.
- left: `top: -4px; right: calc(100% + 18px)`; deal-x +46px.
- inside: `top: 52px; right: 14px`; deal-x +46px.
- Placement test (screen px, at open): `gap = fan.left − card.right` (the 18px × zoom); keep right if `card.right + gap + fan.width ≤ board.right`; else left if `card.left − gap − fan.width ≥ board.left`; else inside. `board` = the board canvas element's bounds.

Header: flex row baseline, gap 8, padding 0 4px, 11px; `fan-deal` 380ms ease-stack, both, no delay. Title 12px 600 primary; count tertiary tabular. Inside variant: `align-self: flex-start; padding: 4px 10px; border-radius: 12px; background: --bg-elevated; box-shadow: --shadow-sm`.

Row (`.fan-card`, a button): column, gap 4, width 100%, padding `10px 12px 11px`, no border, radius 10, bg `--bg-elevated`, shadow `inset 3px 0 0 <edge>, --shadow-md`, text `--text-secondary`, left-aligned, `box-shadow 120ms ease`; animation `fan-deal 520ms ease-stack both`, delay `i×45ms + 30ms`. Hover shadow: `inset 3px 0 0 <edge>, 0 0 0 1px --border-strong, --shadow-lg`.
- Head: flex centred, gap 7, min-width 0; small dot; name 13px 600 primary, one line `…`, flex 1.
- Step: padding-left 14, 11px, flex baseline, gap 6. Tool: flex none, mono, primary. Detail alone: flex 1, 2-line clamp, break anywhere. Detail after tool: block, nowrap, `…`, mono, tertiary.
- Meta: padding-left 14, 11px, flex wrap, gap `0 8px`, tertiary, tabular. Status span coloured working/waiting.

All pill (`.fan-all`): inline-flex centred, gap 5, `align-self: flex-start`, height 26, padding 0 10, no border, radius 13, bg `--bg-elevated`, `--shadow-sm` (hover `--shadow-md`), `--accent`, 11px 500; same deal animation with i = rows shown. Icon `ArrowRight` 12/2.2.

`fan-deal` keyframes: `from { opacity: 0; transform: translate(<deal-x>, 10px) scale(0.92) rotate(-2.5deg) }` → resting (transform origin: element centre).

Node with an open fan: z-index 1400 (forced).

### Archive frame & stack

Geometry (flow px; constants from `shared/geometry.ts` and `layout.ts`): `ARCHIVE_HEADER_H` 52, `ARCHIVE_PAD` 28, `CARD_W` 288, `CARD_H` 216, `GAP` 32, `STACK_SEARCH_Y` 306, `STACK_SEARCH_H` 34, `STACK_W` 344, `STACK_H` 362, `STACK_RESULTS_X` 356, `STACK_BAND_H` 464, `STACK_ROWS` 2, `STACK_DEPTH` 3, `CLIP_BLEED` 24, `FLIGHT_MS` 700, `RAISED_Z` 1500.

Frame node: width/height transition `460ms --ease-out`; cursor default.
Frame (`.archive-zone`): 100%×100%, radius 24, `border: 1.5px dashed --archive-border`, bg `--archive-bg`; transitions `background 220ms ease, border-color 220ms ease, box-shadow 320ms --ease-out`.
- hot: border solid `--accent`, bg `--archive-drop`.
- open: border solid `--border`, bg `--archive-panel`, `--shadow-lg`, `backdrop-filter: blur(30px) saturate(1.8)`.
- (hot and open can't co-occur in practice: unclear: BoardView.tsx:1739 — dragging while open is possible; both classes would apply, open rules come later in the stylesheet so open wins for border/background.)

Header: height 52, flex centred, padding 0 22px, `--archive-label`, cursor grab / grabbing (archive dragged), radius `24px 24px 0 0`, pointer-events all.
Label: flex centred, gap 8, `transform-origin: left center`, `transform: scale(min(2.2, max(1, 1/zoom)))`, 13px 600, letter-spacing 0.13px, nowrap. hot → colour `--accent`.
- Icon `Archive` 15/2.
- Count pill: padding 0 7, radius 999, bg `--bg-sunken`, 11px, line-height 18px, tabular.
- Hint: margin-left 6 (plus the 8px gap), weight 400, 12px, opacity 0 → 1 (header hover or hot), `opacity 220ms ease`.

Placeholder (`.archive-placeholder`): absolute at (28, 52) 288×216, grid-centred, radius 14, `1.5px dashed --archive-border`, `--archive-label`, 12px, pointer-events none. Dealt variant: content centred as a column with 3px gap, centred text, `archive-fade 240ms ease 200ms backwards`; `strong` 13px 600.

Search (`.archive-search`): absolute (28, 306) 288×34, flex centred, gap 7, padding `0 9px 0 11px`, radius 10, bg `--bg-elevated`, `--shadow-sm`, colour `--text-tertiary` (icon), cursor text. Hidden: opacity 0, `translateY(-6px) scale(0.97)`, pointer-events none. Shown: opacity 1, no transform. Transitions `opacity 180ms ease, transform 360ms ease-stack, box-shadow 120ms ease`. Focus-within shadow `0 0 0 3px --focus-ring, --shadow-sm`.
- Icon `Search` 13/2.2, flex none.
- Input: flex 1, min-width 0, height 100%, padding 0, no border, transparent, no outline, `--font-ui` 13px `--text-primary`; placeholder `--text-tertiary`.
- Clear: flex none, 15×15 grid-centred circle, no border, bg `--text-tertiary`, icon colour `--bg-elevated`, `X` 9/3, cursor default.

Divider: absolute left 336, top 52, bottom 28, width 1, bg `--separator`, opacity 0; open → 1 with `transition: opacity 220ms ease 120ms`.
Empty results text (`.archive-empty`): absolute (356, 52) 288×216, grid-centred, margin 0, `--text-tertiary`, 13px, centred; `archive-fade 240ms ease 160ms backwards`.
Scrollbar: absolute right 11, top 52, height 464, width 5, pointer-events none, `archive-fade 240ms ease 160ms backwards`; thumb full width, radius 3, bg `--text-tertiary`, opacity 0.55, height/offset per ARCH-036 (offset applied as translateY).
`archive-fade`: `from { opacity: 0 }`.

Stack card visuals (applied to the card body inside a stack node; transform origin `50% 100%`):

| Class | Transform | brightness | Content | Resolved card fill light / dark |
| --- | --- | --- | --- | --- |
| d0 | none | 1 | visible | `#ffffff` / `#2a2a2d` |
| d0 lifted | `translateY(-3px)` + `--shadow-md` | 1 | visible | — |
| d1 | `translateY(10px) scale(0.94)` | 0.955 | hidden | `#f4f4f4` / `#28282b` |
| d1 lifted | `translateY(14px) scale(0.94)` | 0.955 | hidden | — |
| d2 | `translateY(20px) scale(0.88)` | 0.91 | hidden | `#e8e8e8` / `#262629` |
| d2 lifted | `translateY(28px) scale(0.88)` | 0.91 | hidden | — |
| d3 | `translateY(20px) scale(0.84)` | 1 | hidden, opacity 0 | — |

Transitions on stack cards: `transform 560ms ease-stack, opacity 260ms ease, filter 260ms ease, box-shadow 220ms --ease-out`; children `opacity 200ms ease`.
Result node glide: `transform 620ms ease-stack` delayed `min(slot,4) × 30ms`; none while results scroll. Rise animation `stack-rise`: `from { transform: translate(fromX, fromY) scale(0.88); opacity: 0 }`, 620ms ease-stack, same delay, backwards.
Result slot position (flow): `x = ax + 356 + (slot % 2) × 320`, `y = ay + 52 + floor(slot/2) × 248 − scroll`. Clip: `inset(max(cutTop,−24) px, −24px, max(cutBottom,−24) px)` where `cutTop = bandTop − y`, `cutBottom = y + 216 − bandBottom`; drawn only when both cuts < 216.

### Notes

Note node: z 2; glide `transform 420ms --ease-out` (none while dragging / archive moving); cursor default.

Note (`.note`): 100%×100%, padding 16px 18px, radius 4, background = two layers: top `linear-gradient(to bottom, rgba(0,0,0,0.035) 0px, transparent 22px)` over the paper colour; shadow `--note-shadow`; colour `--note-ink`; transitions `box-shadow 220ms --ease-out, transform 220ms --ease-out`; transform origin centre (draft: bottom-left).

| State | Shadow | Transform |
| --- | --- | --- |
| base | `--note-shadow` | none |
| selected / editing | `0 0 0 2px --accent, --note-shadow` | none |
| dragging | `--note-shadow-lifted` | `rotate(-1.5deg) scale(1.02)` |
| selected + dragging | `0 0 0 2px --accent, --note-shadow-lifted` | `rotate(-1.5deg) scale(1.02)` |
| draft, node hovered | `--note-shadow-lifted` | `rotate(-2deg) translateY(-2px)` (origin 0 100%) |
| draft, dragging | `--note-shadow-lifted` | `rotate(-1.5deg) scale(1.02)` (origin 0 100%) |

(Rotations are clockwise-positive as in CSS; −1.5° tilts counter-clockwise.)

#### Note colours (paper = hue 72% + paper 28%, sRGB)

| Colour | Hue (`frontTop`) | Light paper | Light top edge (×0.965) | Dark paper | Dark top edge |
| --- | --- | --- | --- | --- | --- |
| blue | `#6dd0fc` | `#96ddfd` | `#91d5f4` | `#81c8e9` | `#7cc1e1` |
| purple | `#c8a2f7` | `#d7bcf9` | `#d0b5f1` | `#c2a7e5` | `#bca1dd` |
| pink | `#fba0c8` | `#fcbbd7` | `#f3b4d0` | `#e7a6c4` | `#dfa0bd` |
| red | `#ff9189` | `#ffb0aa` | `#f6aaa4` | `#ea9b96` | `#e29591` |
| orange | `#ffc374` | `#ffd49b` | `#f6cc95` | `#eabf87` | `#e2b882` |
| yellow | `#ffe27a` | `#ffea9f` | `#f6e29a` | `#ead58b` | `#e2ce86` |
| green | `#93e19c` | `#b1e9b8` | `#abe1b1` | `#9cd4a4` | `#97cd9e` |
| graphite | `#bdbdc2` | `#cfcfd3` | `#c8c8cc` | `#bababf` | `#b4b4b9` |

"Top edge" is the paper under the 3.5% black overlay at y = 0; the overlay fades linearly to 0 at 22px. Stickers use a 4% overlay over 10px (top edge = paper × 0.96: e.g. yellow light `#f5e199`, dark `#e1cd86`).

Text (`.note-text`): 100% of content box (minus 22px when signed), margin 0, `--font-ui` 15px, lh 1.4, `pre-wrap`, `overflow-wrap: anywhere`, inherit colour, overflow hidden, mask `linear-gradient(#000 calc(100% − 18px), transparent)` (opaque until 18px from the bottom, then linear to 0), not selectable. Empty: `--note-ink-faint`.

Field (`.note-input`, textarea): block, 100%×100%, padding 0, no border, transparent, no outline, no resize, caret `--note-ink`, cursor text, same font; placeholder `--note-ink-faint`; text selectable.

Signature (`.note-from`): absolute left 18, right 46, bottom 13; no padding/border/background; `--font-ui` 12px, lh 16px, left, nowrap, `…`; `--note-ink-faint`; hover `--note-ink`, underline, offset 2px; `color 120ms ease`.

▶ (`.note-play`): absolute right 10 bottom 10, 28×28 grid-centred, padding `0 0 0 2px`, no border, circle, bg `rgba(0,0,0,0.656)`, icon colour = paper (table), shadow `0 2px 6px rgba(0,0,0,0.18)`, opacity 0, `scale(0.85)`, cursor default; node hover → opacity 1, `transform: none`; dragging → opacity 0; hover → bg `--accent`, icon `#ffffff`; active `scale(0.92)`. Transitions `opacity 120ms ease, transform 220ms --ease-spring, background 120ms ease`. Icon `Play` 12, stroke 2.5, `fill: currentColor`.

Resize (selected, not editing): edge zones as XCARD (10px, transparent, z 2); corner handles 9×9, `border: 1.5px solid --accent`, radius 2, bg `--bg-elevated` (light `#ffffff`, dark `#2a2a2d`), z 3, centred on corners.

#### Sticker

Absolute: `right: 18px + i×30px`, `bottom: calc(100% − 6px)` (relative to the session node), z 2, 84×54, padding 6px 8px, radius 3; background `linear-gradient(rgba(0,0,0,0.04), transparent 10px)` over paper; `--note-shadow`; `--note-ink`; `transform: rotate(3deg)`, origin `50% 100%`; transitions `transform 220ms --ease-out, box-shadow 220ms --ease-out`. Hover: `rotate(1deg) translateY(-2px)`, `--note-shadow-lifted`.
Text: margin 0, 3-line clamp, 11px, lh 1.25, `pre-wrap`, anywhere, not selectable.
Remove ×: absolute top −6 right −6, 16×16 grid-centred circle, no border, bg `--note-ink` (`rgba(0,0,0,0.82)`), icon colour `--note-paper` (light `#ffffff`, dark `#b4b4b8`), opacity 0 → 1 on sticker hover (`120ms ease`), cursor default; `X` 9/3.
Landing: `sticker-land` 520ms `--ease-spring` backwards: `from { transform: translateY(-14px) rotate(-6deg) scale(1.35); opacity: 0 }` (origin bottom-centre).

#### Note pad

Pad node at `notePadRect(archive)` = (ax, ay + 386, 344, 296); z −1; glide off while the archive moves.
Frame (`.notepad`): 100%×100%, radius 24, `1.5px dashed --archive-border`, bg `--archive-bg`.
Header: height 52, flex centred, padding 0 22, `--archive-label`. Label: flex centred, gap 8, origin left-centre, scale per zoom, 13px 600, letter-spacing 0.13px, nowrap. Icon `StickyNote` 15/2. Hint: margin-left 6, 400, 12px.
Sheets container: absolute (64, 52) 216×216. Each sheet is a full `.note` (yellow paper, gradient, `--note-shadow`, padding) at inset 0, origin `50% 100%`:

| Sheet | Transform | brightness | Resolved paper light / dark |
| --- | --- | --- | --- |
| depth 2 (bottom, drawn first) | `translateY(8px) scale(0.97)` | 0.88 | `#e0ce8c` / `#cebc7b` |
| depth 1 | `translateY(4px) scale(0.985)` | 0.94 | `#f0dc96` / `#dcc883` |
| top (under the draft) | none | 1 | `#ffea9f` / `#ead58b` |

Draft top sheet: at (ax + 64, ay + 386 + 52) i.e. `padSheetSpot`, 216×216, z 2, data `{ id: <next note id>, text: '', color: 'yellow', createdAt: 0 }`.

### Edges (visual only; routing in 09)

Lineage: stroke `--border-strong`, width 1.5, no dash; animated (child working): stroke `--status-working`, `stroke-dasharray: 5` (5 on, 5 off), `dashdraw` 0.5s linear infinite (`stroke-dashoffset` 10 → 0). View edge: stroke accent 70% (`rgba(0,122,255,0.7)` / `rgba(10,132,255,0.7)`), width 2, `stroke-dasharray: 6 4`, round caps, `dashdraw` 1s linear infinite. Path: bezier between the facing-side midpoints of the two nodes (`getBezierPath`, default curvature 0.25).

## Keyboard & mouse

| Where | Input | Effect | ID |
| --- | --- | --- | --- |
| Card | hover | shadow-md; corner buttons appear | CARD-003, CARD-020 |
| Card corner | click Archive / Expand | archive (confirm if running) / expand | CARD-022/023 |
| Card title | double-click | rename (09) | CARD-049 |
| Rename field | Enter / blur / Escape | commit / commit / cancel; all keys swallowed | CARD-046…048 |
| Footer agents (deck) | point / click | fan open / pin toggle | CARD-037, DECK-008/009 |
| Footer agents (no deck) | click | open Agents viewer | CARD-038 |
| Deck layer | point / click | fan open / pin toggle | DECK-008/009 |
| Anywhere (fan pinned) | pointer-down outside card node / Escape | close fan | DECK-011 |
| Fan row / pill | click | close fan, open Agents viewer (on agent / all) | DECK-027 |
| Fan | wheel | not the board's | DECK-028 |
| Expanded title bar | click / double-click title / drag | focus terminal / rename / move | XCARD-017 |
| Expanded tools | click | per tool | XCARD-009…015 |
| Expanded body | pointer-down | focus terminal (if attached) | XCARD-018 |
| Expanded body | wheel (gesture started here) / pinch | scroll terminal / zoom board | XCARD-027/028 |
| Expanded edges/corners | drag | resize, min 360×240 | XCARD-029/030 |
| Archive frame | hover (no buttons) | stack lift + search field shows and focuses | ARCH-016, ARCH-019/020 |
| Archive frame | click (folded) | open panel | ARCH-011 |
| Archive header | drag | move archive (+ pad, archived cards) | ARCH-007, ARCH-038 |
| Archive search | type / Enter / ↓ / Escape | filter+open / preview first or open / open / close+dismiss | ARCH-022…025 |
| Archive search clear | click | clear, keep focus | ARCH-027 |
| Board (panel open) | pointer-down outside archive & archived cards | close panel | ARCH-037 |
| Note | double-click / Enter (single note selected) | write | NOTEUI-010 |
| Note field | Enter / Escape / ⌘↩ (Ctrl+Enter) / click away | newline / done / done / done | NOTEUI-012/013 |
| Note | hover → ▶ click | start session | NOTEUI-022…024 |
| Note signature | click | open writer session (here or its project) | NOTEUI-018/019 |
| Note (selected) | drag corners/edges | resize, min 120×120 | NOTEUI-025/026 |
| Note(s) selected | ⌫ / Del | delete, no confirm (09) | NOTEUI-030 |
| Sticker | hover / click × | lift + × / delete note | NOTEUI-035/037 |
| Pad top sheet | hover / drag off / drop back / single click | peel / new note at drop / return / new note beside pad | NOTEUI-044…047 |

## Platform differences

- **Shortcut text**: `keys('⌫')` renders `⌫` on macOS and `Del` on Windows/Linux → tooltips `Archive (⌫)` vs `Archive (Del)` (CARD-022, XCARD-013). `(E)` is literal on both.
- **Note "done" chord**: ⌘Enter on macOS, Ctrl+Enter on Windows (`modKey`) (NOTEUI-012).
- **Paths**: `tildePath` only abbreviates `/Users/<name>` or `/home/<name>` prefixes to `~`; Windows paths (`C:\Users\…`) are shown in full in terminal-card previews (CARD-026/032).
- **Fonts**: macOS system font for UI and display (identical there); Windows `Segoe UI Variable Text`/`Display` (fallback `Segoe UI`); mono `SF Mono` vs `Cascadia Mono`/`Consolas`. Line heights are multiples of the font size, so box heights match; glyph widths differ.
- **Date format**: CARD-041's dates use the OS locale (`toLocaleDateString(undefined, …)`).
- **Wheel**: trackpad pinch on macOS and Ctrl+wheel on Windows both arrive with Ctrl set and go to the board (XCARD-028).
- **Reduced motion**: macOS "Reduce motion" / Windows "Show animations in Windows" off.
- **Icon** `AppWindowMac` is used on both platforms (no Windows variant).
- No CSS in this domain is platform-conditional (`data-platform` is only used by the title bar, 08).

## Edge cases & errors

Confirm dialogs triggered from this domain's buttons (dialog chrome owned by 09/11; copy verbatim, curly quotes U+201C/U+201D, apostrophes as shown):

- Archive (corner button, title-bar Archive, drop on archive) when at least one target is `live && status !== 'done'`:
  - one: title `Archive a running session?`, message `“<title>” is still working. The Claude process will be stopped.` or `“<title>” is still waiting for you. The Claude process will be stopped.`
  - several: title `Archive N running sessions?`, message `Some of these sessions are still working. Their Claude processes will be stopped.`
  - confirm `Stop & Archive`, danger. Cancel leaves everything; for a drop, the dragged cards return to their drag start.
- Stop (title-bar Stop) when the session is not `done`: title `Stop a running session?`, message `“<title>” is still working. Stopping it cuts off this turn. The transcript stays, so you can resume it.` (or `… is still waiting for you. …`), confirm `Stop`, danger. A `done` live session stops without asking.

Other copy and edge cases:
- Toast on a failed note start: `Couldn't start a session: <error message>` (ASCII `'`; toast shown 4200ms, owned by 09/11).
- Terminal error titles use U+2019: `Couldn’t start the shell`, `Couldn’t start Claude`; button `Try again`.
- Agent step waiting text uses U+2019: `Waiting for you in the session’s terminal`.
- Footer agent tooltip has no singular form: `1 agents, all done` / `1 of 1 agents running` are shown as such (CARD-037). The no-deck tooltip and fan pill do pluralise.
- `timeAgo` returns `''` for a missing/zero timestamp; a future timestamp reads `just now`.
- A card with empty `title` and no `meta.title` renames from an empty field; committing an empty field when the initial was empty is a no-op.
- Archived cards never render expanded, never show a deck, corner buttons or stickers (stickers kept in data, hidden).
- `agentCount` absent → no agents button unless a deck exists.
- An expanded card with no project loaded (`projectId` empty) never attaches its terminal.
- The fan's side is computed once on open; panning the card offscreen while open does not move it.
- Stack search never steals focus from a field being edited (card rename, note, section name).
- Notes dropped on archive/pad bounce; terminals dropped on archive bounce; views too (09).
- Empty notes persist; a note with only whitespace has no ▶ and `Start session` is disabled (09 menu).
- Clicking ▶ twice quickly starts only one session (in-flight guard).
- A note stuck to a card whose card leaves the board (removed) drops onto the board at the card's last position; from an archived card, back where it was (03).
- Discrepancy: docs say deleting a written note asks first; code deletes immediately (NOTEUI-030).
- Screenshots `board*.png`/`expanded*.png` show an older corner/tool layout (single glyph top-right, 3 title-bar icons); the code's layout (CARD-020, XCARD-007) is the spec.

## Tests to port

Existing Vitest cases touching this domain (port in the owning crate; listed so nothing is dropped):
- `src/renderer/src/components/board/stopSessions.test.ts` → `nodus-board` (or `nodus-sessions`), owned with 09: `stoppable` "keeps the Claude sessions that are running", "leaves out action runs, which have no card to keep"; `stopConfirmation` "doesn’t ask when every session is between turns", "asks for a session that’s working, by name", "says when a session is waiting for you", "asks once for several, when any of them is mid-turn", "names the one session among several that’s mid-turn". (Drives XCARD-012's confirm.)
- `src/shared/geometry.test.ts` → `nodus-board::geometry` (owned by 03/09): "cardSize is compact unless expanded, and never expanded in the archive" (XCARD-001).
- `src/renderer/src/platform.test.ts` → `nodus-ui::platform` (08): `keys` "writes a Mac shortcut as Windows does", "leaves it be on a Mac" (tooltips here).

No Vitest exists for the components in this spec. New Rust tests required (pure functions in `nodus-board::present` or `nodus-ui::board::cards` with `#[cfg(test)]`):
- `strip_markdown`: fenced blocks → space; links → text; `**`/`__`/backticks removed; heading/quote markers at line starts (≤3 leading spaces) removed; text elsewhere untouched.
- `highlight_runs(text, indices)`: alternating runs, adjacent indices merged, empty indices → one plain run.
- `preview(card, repo_path)`: path / reply / prompt / `No messages yet` precedence; terminal with no repo → empty path.
- `time_ago`: 0 → `''`; 59s → `just now`; 60s → `1m ago`; 59m → `59m ago`; 1h → `1h ago`; 23h → `23h ago`; 1d → `1d ago`; 6d → `6d ago`; 7d → date without year; 301d → date with year; future → `just now`.
- `card_title`: title > meta.title > `Terminal`/`New session`.
- `tilde_path` (regex `^/(Users|home)/[^/]+(?=/|$)` → `~`): `/Users/<name>/x` → `~/x`; `/home/<name>` → `~`; `/Users/<name>` → `~`; `/Usersx/<name>/x` and `/opt/<name>` unchanged; Windows `C:\Users\<name>\x` unchanged.
- `format_duration`: 0 → `0s`; 42.4s → `42s`; 59.6s → `1m`; 252s → `4m 12s`; 600s → `10m`; 3900s → `1h 5m`; negative → `0s`.
- `by_urgency`, `agent_step` (waiting, background 1, background 3, tool+activity), `agent_status_label` (stopped vs done).
- Fan selection: ≤6 → all in order; 7 agents with mixed status → 5 most urgent in start order; pill text for both cases and `total === 1`.
- Fan placement: right fits; right doesn't but left does; neither → inside (inputs as rects).
- Deck state machine: point → open; leave → closes after 280ms; stay cancels; toggle pins; toggle again unpins but stays open; outside pointer-down / Escape closes only when pinned; drag closes.
- Wheel arbiter: first event over terminal → terminal scrolls whole gesture; first event elsewhere → board pans across terminal; 250ms gap resets; Ctrl → board.
- Expanded start state: live / ready terminal / pending wake → started; later `live` → started; wake semantics (start vs reattach vs nothing while loading); focus only within 1500ms; loading label fixed at wake time.
- Archive: `label_scale(zoom)` clamps 1…2.2; scrollbar thumb/offset formula; stack zone sizes for r = 0, 1, 2, 3, 5; result slot positions and clip insets; dealt-placeholder condition; search visibility condition; Escape/Enter/↓ handling.
- Notes: `split_notes` (stuck vs free, card absent → free); `pad_sheet_spot`; `is_on_pad` (centre test); `build_pad_nodes` draft keeps dragged position; note editor debounce (500ms, unchanged skip, flush on drop); sticker `landing` iff `now − createdAt < 4000`; signature text both variants.
- Visual regression: screenshot tests of a compact card in each state (base/hover/waiting/selected/dragging/archived-in-stack), expanded card (not started, loading, error, live with usage at 720/500/400 wide), deck closed/lifted/fanned (right/left/inside), archive folded/hovered/hot/open with 1, 3, 5 results and no-match, every note colour in both themes, sticker, pad — light and dark, compared with the Electron app (plan.md "Done means").

## Rust mapping

Module: `crates/nodus-ui/src/board/cards/` (UI) with pure presentation logic in `crates/nodus-board/src/present.rs` (headless-testable: `strip_markdown`, `highlight_runs`, `preview`, `time_ago`, `card_title`, `tilde_path`, `format_duration`, `by_urgency`, `agent_step`, `agent_status_label`, `fan_pick`, `split_notes`, stack geometry).

Suggested files:
- `session_card.rs` — compact card (`SessionCard` view), header, chip, preview, footer, corner buttons, rename field.
- `status_dot.rs` — dot + animations (shared with rail/projects/modal header).
- `expanded_card.rs` — title bar, tools, body host for `nodus-terminal` element, overlays, start/wake state, wake registry (`HashMap<SessionId, WakeHandle>` + pending set, replacing the module globals), resize handles, grip.
- `usage_stats.rs` — shared with 12.
- `agent_deck.rs` — deck layers, `DeckState` (hover/pinned/close timer), fan.
- `archive_zone.rs` — frame, header label, placeholders, search field, divider, scrollbar; stack depth visuals applied by the canvas (09) through a `StackStyle { depth, lifted, result, rise }` passed to the card.
- `note.rs`, `note_editor.rs`, `sticker.rs`, `note_pad.rs`.
- `motion.rs` — easings: cubic-bezier solver, `ease_stack` piecewise-linear table (29 stops), keyframe helpers with per-segment easing and fill-modes; reduced-motion switch from `nodus-platform`.
- `paint.rs` — helpers for things GPUI's div styling lacks (below).

Custom GPUI elements / capabilities needed (verify each in spike S1):
1. **Per-element affine transforms** (translate + scale + **rotate**) about a chosen origin, composed with the board's pan/zoom: card drag scale, deck layers, stack depths, note drag tilt (−1.5°), draft peel (−2°), sticker tilt (3°/1°), sticker landing (−6°, ×1.35), fan deal (−2.5°). GPUI only rotates `svg()` via `Transformation`; quads and text are axis-aligned. Needs a custom element (or GPUI patch) that paints quads, shadows and **text** under a rotation — highest risk.
2. **Multi-line clamp with ellipsis** and `overflow-wrap: anywhere` (title 2, preview 4/3/2, fan detail 2, sticker 3) plus whitespace collapsing vs `pre-wrap`.
3. **Inset shadows** (deck bottom strip 3px, fan row left strip 3px): draw as a clipped child quad inside the rounded shape.
4. **Gradient mask** (note text fades out in its last 18px): GPUI has no alpha masks; draw a paper-coloured gradient overlay (the paper is flat at the bottom, so a `paper → transparent` overlay is exact for unselected text).
5. **Brightness filter** on layers/slabs/sheets: precompute multiplied colours (tables above), including the shadows' colours.
6. **Dashed 1.5px borders** with radius (archive, pad, placeholders).
7. **Backdrop blur + saturate** behind the open archive panel (30px, 1.8) over board content — GPUI has window-level blur only; needs a custom pass or an accepted approximation (decision required; plan says "exact").
8. **0.5px hairline rings** (all shadows' ring layer, footer separator) at fractional device pixels.
9. **Hover propagation**: ancestor hover drives descendants (`group_hover` for corner buttons, ▶, grip, sticker ×; "any deck layer hovered" lifts all layers; archive header hover shows hint).
10. **Container width query** for usage-stats hiding (measure title bar width each layout).
11. **Event capture phase** for wheel arbitration (XCARD-027) and window-level pointer-down capture (DECK-011, ARCH-037), plus "consume all keys" in text fields so board shortcuts don't fire.
12. **Text fields**: single-line (rename, archive search, with select-all on focus) and multi-line (note) with placeholder, caret colour, spell-check off — likely `gpui-component` input, restyled.
13. **Hit-testing for resize zones** that extend outside the node (edge zones straddle the border; corners 18px / 9px).
14. **CSS blur semantics**: confirm GPUI `BoxShadow.blur_radius` maps to CSS blur (σ = B/2) or convert.

Risks (biggest first): (1) rotated text/elements (notes, stickers, fan deal) have no GPUI primitive; (2) backdrop blur on the open archive panel; (3) the `ease-stack` spring + staggered, fill-mode-aware keyframes across many nodes at 60fps; (4) wheel/pointer capture semantics (gesture arbitration between terminal and board) — must match exactly or trackpad scrolling regresses; (5) line-clamp + `anywhere` breaking + optical-size fonts producing different line breaks than Chromium (titles/previews visibly differ in screenshot diffs).

Cross-domain dependencies: 09 (node z-order incl. select elevation +1000, drag/drop resolution, click/double-click routing, context menus, stack placement/flights, `ensureVisible`, toasts/confirm), 12 (terminal element and registry `attach/needsOpen/focus/typeIn/hasOutput`, modal `terminalSessionId`, Agents viewer `openAgents(sessionId, agentId?)`, transcript preview `openPreview`), 05 (mic `SessionMic`), 03/04 (card fields, notes persistence `saveNote/removeNotes`, `unstickNotes`, `sessions.create` with `initialPrompt`/`autoMode`), 08 (tokens, `.btn` styles, platform `keys/modKey`, theme source), 01 (reduced-motion and appearance from the OS).

## Parity checklist

- [x] CARD-001 compact size 288×216, padding 14/16/11, column gap 8
- [x] CARD-002 surface bg-elevated, radius 14, shadow-sm
- [x] CARD-003 hover shadow-md
- [x] CARD-004 waiting 1px ring 45% + shadow-md
- [x] CARD-005 selected 2px accent ring + shadow-md, beats waiting
- [x] CARD-006 dragging ring + shadow-lg + scale 1.015
- [x] CARD-007 card transitions 220ms
- [x] CARD-008 archived opacity rule + stack override
- [x] CARD-009 cursor default / grabbing
- [x] CARD-010 header flex-start gap 9 min-h 20
- [x] CARD-011 status dot 10px, top 5px, tooltip labels
- [x] CARD-012 working dot + breathe
- [x] CARD-013 done dot
- [x] CARD-014 waiting dot + pulse
- [x] CARD-015 small dot variants
- [x] CARD-016 terminal glyph instead of dot, live colour, tooltip
- [x] CARD-017 title precedence, typography, 2-line clamp, tooltip
- [x] CARD-018 search match runs in accent
- [x] CARD-019 live glyph + tooltip
- [x] CARD-020 corner group on hover (Archive + Expand)
- [x] CARD-021 corner button styling + hover mix colours
- [x] CARD-022 corner Archive (tooltip per platform, confirm)
- [x] CARD-023 corner Expand (tooltip, wake)
- [x] CARD-024 corner clicks don't reach card / no drag
- [x] CARD-025 merged chip
- [x] CARD-026 preview kind precedence
- [x] CARD-027 stripMarkdown rules
- [x] CARD-028 preview typography, collapse, clamp 4
- [x] CARD-029 clamp 3 after chip
- [x] CARD-030 prompt italic
- [x] CARD-031 empty tertiary `No messages yet`
- [x] CARD-032 path mono 11px clamp 2
- [x] CARD-033 footer layout + separator
- [x] CARD-034 branch item max 130 + tooltip
- [x] CARD-035 message count item
- [x] CARD-036 agents footer button with running badge
- [x] CARD-037 footer agents point/click/tooltip/aria-expanded
- [x] CARD-038 no-deck agents button opens viewer
- [x] CARD-039 footer agents button styling
- [x] CARD-040 terminal footer Running/Exited
- [x] CARD-041 timeAgo formats
- [x] CARD-042 timestamp source, no ticking
- [x] CARD-043 accessible label
- [x] CARD-044 rename field prefill + select all
- [x] CARD-045 rename field styling
- [x] CARD-046 rename commit rules
- [x] CARD-047 rename Escape cancels
- [x] CARD-048 rename swallows keys/pointer
- [x] CARD-049 rename triggers (09) + 240ms title click
- [x] CARD-050 no visible handles; facing-side edges
- [x] CARD-051 lineage edge stroke + working animation
- [x] CARD-052 view edge dashed marching
- [x] CARD-053 card node glide 420ms
- [x] CARD-054 `Started by` in the title tooltip and the menu (server)
- [x] CARD-055 `Setting up…` / `Setup failed` preview (server)
- [x] CARD-056 Open preview glyph and menu item (server)
- [x] CARD-057 worktree branch and folder (server)
- [x] CARD-058 `Claude account` in the title tooltip and the menu (server)
- [x] CARD-059 `Moved to` / `Waiting for a Claude account` pill (server)
- [x] XCARD-001 expanded condition + default 720×480
- [x] XCARD-002 expanded surface, clip, no drag scale
- [x] XCARD-003 title bar 40px layout
- [x] XCARD-004 title single line 13px, centred glyphs
- [x] XCARD-005 usage stats content
- [x] XCARD-006 usage hides at ≤560 / ≤440
- [x] XCARD-007 tool order
- [x] XCARD-008 tool button styling/states
- [x] XCARD-009 Agents tool + badge
- [x] XCARD-010 Mic types in + wakes
- [x] XCARD-011 Resume / Restart
- [x] XCARD-012 Stop + confirm
- [x] XCARD-013 Archive tool
- [x] XCARD-014 Open in a window
- [x] XCARD-015 Collapse + tooltip variants
- [x] XCARD-016 tools swallow clicks / no drag
- [x] XCARD-017 title bar drag handle, click focuses, dblclick renames
- [x] XCARD-018 body padding, bg, cursor, focus on press
- [x] XCARD-019 initial started rule
- [x] XCARD-020 started when live later
- [x] XCARD-021 enabled rule; blank while in modal
- [x] XCARD-022 not-started overlay + button copy
- [x] XCARD-023 loading pill copy + fade
- [x] XCARD-024 error overlay + Try again
- [x] XCARD-025 wake semantics + pending wake
- [x] XCARD-026 focus within 1500ms of wake only
- [x] XCARD-027 wheel gesture arbitration (250ms)
- [x] XCARD-028 pinch/ctrl always to board
- [x] XCARD-029 resize zones + cursors
- [x] XCARD-030 min 360×240, save rounded
- [x] XCARD-031 grip on hover
- [x] XCARD-032 z above compact; deck & stickers on expanded
- [x] XCARD-033 a server card's terminal at the size the session follows (hosted)
- [x] DECK-001 deck existence
- [x] DECK-002 3 layers by urgency, nearest = most urgent
- [x] DECK-003 layer surface + 3px status strip
- [x] DECK-004 rest offsets/scales/brightness
- [x] DECK-005 lifted offsets on hover/open + pointer cursor
- [x] DECK-006 layer transitions 420/220ms
- [x] DECK-007 waiting layer glow
- [x] DECK-008 point opens fan
- [x] DECK-009 click toggles pin
- [x] DECK-010 280ms close on leaving node
- [x] DECK-011 pinned closes on outside press / Escape
- [x] DECK-012 unpin leaves hover-open
- [x] DECK-013 drag / no agents closes
- [x] DECK-014 fan node z 1400
- [x] DECK-015 fan side decision once
- [x] DECK-016 fan geometry per side
- [x] DECK-017 fan header + inside pill
- [x] DECK-018 ≤6 all, else 5 most urgent in start order
- [x] DECK-019 fan row visuals + tooltip
- [x] DECK-020 fan row hover
- [x] DECK-021 fan row lines content/typography
- [x] DECK-022 agentStep texts
- [x] DECK-023 agentStatusLabel
- [x] DECK-024 elapsed + 1s tick
- [x] DECK-025 all-agents pill text + visuals
- [x] DECK-026 deal-in animation + stagger
- [x] DECK-027 row/pill open Agents viewer
- [x] DECK-028 fan swallows clicks/menu/wheel/drag
- [x] DECK-029 fan accessible name
- [x] ARCH-001 frame sizes folded/open
- [x] ARCH-002 folded frame visuals
- [x] ARCH-003 hot visuals + `Drop to archive`
- [x] ARCH-004 open panel visuals + blur
- [x] ARCH-005 frame transitions + 460ms size
- [x] ARCH-006 z −1 / raised 1500 + 700ms
- [x] ARCH-007 header drag handle, cursor, tooltip
- [x] ARCH-008 label, count pill, zoom scale
- [x] ARCH-009 hint text + visibility
- [x] ARCH-010 empty placeholder
- [x] ARCH-011 click opens panel
- [x] ARCH-012 stack order + spot + depth ≤3
- [x] ARCH-013 depth visuals + hidden contents
- [x] ARCH-014 stack opacity 1 (d3 0)
- [x] ARCH-015 stack transitions
- [x] ARCH-016 stack lift on hover/hot
- [x] ARCH-017 lift overrides top card drag visuals
- [x] ARCH-018 only top card + results take pointer
- [x] ARCH-019 search field visibility + geometry + animation
- [x] ARCH-020 search auto-focus rules
- [x] ARCH-021 placeholder + accessible name
- [x] ARCH-022 typing opens + resets scroll
- [x] ARCH-023 Escape closes + dismisses
- [x] ARCH-024 Enter preview first / open
- [x] ARCH-025 ArrowDown opens
- [x] ARCH-026 click field opens + focuses
- [x] ARCH-027 clear button
- [x] ARCH-028 focus ring
- [x] ARCH-029 result slots 2 columns
- [x] ARCH-030 band clipping + 24px bleed
- [x] ARCH-031 result glide + stagger, none while scrolling
- [x] ARCH-032 rise-from-stack animation
- [x] ARCH-033 dealt placeholder copy
- [x] ARCH-034 divider
- [x] ARCH-035 no-match message
- [x] ARCH-036 scrollbar geometry
- [x] ARCH-037 outside press closes panel
- [x] ARCH-038 archived cards + pad travel with archive
- [x] ARCH-039 drop to archive / terminal bounce
- [x] ARCH-040 count = archived cards
- [x] NOTEUI-001 note size default + min 120
- [x] NOTEUI-002 paper colours both themes
- [x] NOTEUI-003 top shading gradient 22px
- [x] NOTEUI-004 note box padding/radius/shadow/ink
- [x] NOTEUI-005 selected/editing ring
- [x] NOTEUI-006 dragging tilt + lifted shadow
- [x] NOTEUI-007 note transitions
- [x] NOTEUI-008 text typography + bottom fade
- [x] NOTEUI-009 empty placeholder text
- [x] NOTEUI-010 write triggers
- [x] NOTEUI-011 editor focus, caret end, styling, placeholder
- [x] NOTEUI-012 Enter newline; Escape / mod+Enter done
- [x] NOTEUI-013 click away done + save
- [x] NOTEUI-014 500ms autosave + flush on destroy
- [x] NOTEUI-015 save to note's project after board switch
- [x] NOTEUI-016 editor swallows keys/pointer/wheel
- [x] NOTEUI-017 empty notes kept
- [x] NOTEUI-018 signature (this board) + click
- [x] NOTEUI-019 signature (other project) + click
- [x] NOTEUI-020 signature styling + hover
- [x] NOTEUI-021 signed text height −22
- [x] NOTEUI-022 ▶ visibility rules
- [x] NOTEUI-023 ▶ styling/hover/press
- [x] NOTEUI-024 start session from note semantics + toast
- [x] NOTEUI-025 note resize handles when selected
- [x] NOTEUI-026 note resize saves rounded
- [x] NOTEUI-027 notes z 2
- [x] NOTEUI-028 note glide 420ms
- [x] NOTEUI-029 note bounces off archive/pad
- [x] NOTEUI-030 delete without confirm
- [x] NOTEUI-031 new notes yellow; 8 colours
- [x] NOTEUI-032 sticker vs free note rules
- [x] NOTEUI-033 sticker placement
- [x] NOTEUI-034 sticker visuals + 3° tilt
- [x] NOTEUI-035 sticker hover + tooltip
- [x] NOTEUI-036 sticker text clamp 3
- [x] NOTEUI-037 sticker × delete, no confirm
- [x] NOTEUI-038 sticker landing < 4000ms
- [x] NOTEUI-039 sticker press drags/opens card
- [x] NOTEUI-040 pad frame position + visuals
- [x] NOTEUI-041 pad header + hint + zoom scale
- [x] NOTEUI-042 pad sheets stack
- [x] NOTEUI-043 draft top sheet properties + tooltip
- [x] NOTEUI-044 draft peel hover / drag tilt
- [x] NOTEUI-045 drop draft off pad → new note
- [x] NOTEUI-046 drop draft back → returns
- [x] NOTEUI-047 click pad → note beside pad
- [x] NOTEUI-048 pad menu `New note`
- [x] NOTEUI-050 `Written by` tooltip (server)
- [x] NOTEUI-049 pad moves with archive
