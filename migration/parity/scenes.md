# Parity scenes

Every screen and state of Nodus 0.9.4 that has a golden screenshot, in light and dark. Generated from `scenes/*.mjs` by `node capture-electron.mjs --write-md`; edit the scene definitions, not this file.

Each scene starts from the same seeded world (`seed/`, `README.md` § The world) and a fresh launch of the app, 1440×900 CSS px at 2x, clock frozen at 2026-10-20 15:00 UTC. Its shot is `golden/<id>.<light|dark>.png`. "Live" means the session's stand-in process is running; statuses come from hooks posted to the session host, as Claude Code's would be.

93 scenes × 2 themes = 186 shots.

## Projects grid, title bar, Settings, project rail

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `projects-empty` | First run: no projects yet | UI-183, UI-184, UI-185, UI-186 | 1. Seed no projects, groups or boards.<br>2. Launch. |
| `projects-grid` | Populated grid: ungrouped projects with live badges, Infra repos group, collapsed Experiments group, archived section, running count | UI-188, UI-189, UI-191, UI-193, UI-233, UI-234, UI-236, UI-104 | 1. Start storefront’s sessions: sf-skeleton working (two agents working), sf-tax waiting, sf-cart done but running, the terminal card’s shell.<br>2. Stay on the projects grid; wait for the badges. |
| `projects-focus` | Focus mode on: projects in focus, a waiting project shown dimmed, the hidden-count note, the title bar switch | UI-195, UI-196, UI-198, UI-200, UI-201, UI-096 | 1. Seed settings.focusFeature and focusMode on (storefront, docs-site and infra are in focus).<br>2. Start design-system’s dsys-tokens and make it wait on a permission (so design-system shows, dimmed).<br>3. Stay on the grid. |
| `projects-focus-none` | Focus mode on with no project in focus: every project shows, with the note | UI-199, UI-096 | 1. Seed focus mode on and no project focused.<br>2. Launch. |
| `projects-selected` | A selected folder | UI-202, UI-206 | 1. Click storefront’s folder once.<br>2. Move the pointer away. |
| `projects-rename` | Renaming a project in place | UI-210 | 1. Click storefront’s folder.<br>2. Press Return: the name becomes a field with its text selected. |
| `projects-context-menu` | A project’s context menu | UI-216, UI-217, UI-158, UI-167 | 1. Right-click storefront’s folder. |
| `projects-context-menu-focus` | A project’s context menu with focus offered (Add to Focus) | UI-216, UI-201 | 1. Seed settings.focusFeature on (focus mode off).<br>2. Right-click mobile-app’s folder. |
| `projects-group-menu` | A group header’s context menu | UI-221, UI-158 | 1. Right-click the Infra repos header. |
| `projects-group-picker` | Move to Group… dialog | UI-241, UI-243, UI-153 | 1. Right-click infra’s folder.<br>2. Choose Move to Group… |
| `projects-remove-confirm` | Remove project confirmation | UI-247, UI-008, UI-153 | 1. Right-click storefront’s folder.<br>2. Choose Remove from Nodus… |
| `projects-drop` | Dragging a folder over the window | UI-228 | 1. Drag files over the grid (a dragenter carrying Files). |
| `projects-toast` | The grid’s toast (a folder that couldn’t be added) | UI-251, UI-290 | 1. Make adding a project fail (the folder picker is native, so its handler is replaced).<br>2. Click New Project. |
| `titlebar-running` | The title bar’s running count, pointed at | UI-095, UI-104, UI-105 | 1. Start storefront’s sessions (as in projects-grid).<br>2. Point at "N running" in the title bar. |
| `titlebar-update` | Restart to Update in the title bar (an update downloaded) | UI-106, UI-107 | 1. Report the update state as ready (0.9.5 downloaded): the status handler and an ev:update-status event. |
| `settings` | Settings, top: Appearance, Focus, Floating capture (Accessibility not allowed yet) | UI-116, UI-117, UI-125, UI-126, UI-129, UI-130, UI-131, UI-133 | 1. Open Settings (the menu’s Settings command). |
| `settings-bottom` | Settings, scrolled to the end: Notifications, Voice prompts (no key), Updates (up to date) | UI-136, UI-137, UI-139, UI-143, UI-147, UI-148, UI-150 | 1. Report updates as up to date (0.9.4).<br>2. Open Settings.<br>3. Scroll its body to the end. |
| `settings-ready` | Settings with the double tap ready, a saved xAI key, and an update downloading | UI-135, UI-140, UI-143, UI-148, UI-149 | 1. Report the capture shortcut as ready, the voice key as saved in Settings, and an update downloading (42%).<br>2. Open Settings; scroll so Floating capture through Updates show. |
| `rail-open` | The project rail slid in over the board, a project pointed at | UI-253, UI-262, UI-264 | 1. Open storefront’s board.<br>2. Move the pointer to the window’s left edge.<br>3. Move onto the rail and point at docs-site. |
| `rail-pinned` | The project rail pinned beside the board | UI-027, UI-270, UI-271 | 1. Seed settings.pinProjectRail on.<br>2. Open storefront’s board. |
| `rail-numbers` | Holding ⌘: the rail’s number hints | UI-275, UI-276 | 1. Seed settings.pinProjectRail on.<br>2. Open storefront’s board.<br>3. Hold ⌘ (Meta) for half a second. |

## Board

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `board-empty` | An empty board: no sessions, the hint, an empty archive and the note pad | BOARD-007, BOARD-146, ARCH-001, NOTEUI-040, NOTEUI-042 | 1. Open mobile-app (it has no sessions). |
| `board-populated` | storefront’s board: cards in every status, Checkout section, lineage edges, agent deck, sticker, signed note, ink, archive, note pad | CARD-001, CARD-004, CARD-011, CARD-012, CARD-013, CARD-014, CARD-016, CARD-019, CARD-025, DECK-002, DECK-004, BOARD-010, BOARD-020, NOTEUI-019, INKUI-025 | 1. Start storefront’s sessions: sf-skeleton working (two agents working, one done), sf-tax waiting, sf-cart done but running, the terminal card’s shell.<br>2. Open storefront from the grid (double-click). |
| `board-card-hover` | A card pointed at: its Archive and Expand buttons | CARD-003, CARD-020, CARD-021 | 1. As board-populated.<br>2. Point at sf-cart’s card. |
| `board-selected` | Two cards selected, with the selection bar | CARD-005, BOARD-040, SEL-001, SEL-002 | 1. As board-populated.<br>2. ⇧-click sf-cart and sf-coupons.<br>3. Move the pointer away. |
| `board-marquee` | Dragging a selection box over the board | BOARD-040, CARD-005 | 1. As board-populated.<br>2. Press on the empty board below the terminal card and drag up and left over the Plan card, without letting go. |
| `board-section-naming` | A new section while Claude names it | BOARD-086, BOARD-087, BOARD-089, BOARD-162 | 1. As board-populated, with the stand-in slow to name sections.<br>2. ⇧-click sf-coupons and sf-search-plan.<br>3. Press ⌘G. |
| `board-section-rename` | Renaming a section | BOARD-045, BOARD-093 | 1. As board-populated.<br>2. Double-click the Checkout section’s header. |
| `board-archive-hover` | The archive stack pointed at: its search field | ARCH-019, ARCH-020, ARCH-021 | 1. As board-populated.<br>2. Point at the archive stack’s top card. |
| `board-archive-open` | The archive fanned open | ARCH-004, ARCH-011, ARCH-014 | 1. As board-populated.<br>2. Click the archive stack.<br>3. Move the pointer away. |
| `board-archive-search` | Searching the archive: matches fanned out, matched letters marked | ARCH-022, CARD-018 | 1. As board-populated.<br>2. Point at the archive stack.<br>3. Type “fix”. |
| `board-note-editing` | Writing in a note | NOTEUI-010, NOTEUI-011 | 1. As board-populated.<br>2. Double-click the yellow note “Ask design about empty states…”. |
| `board-note-selected` | A selected note with its resize handles | NOTEUI-019, NOTEUI-021, NOTEUI-025 | 1. As board-populated.<br>2. Click the signed blue note.<br>3. Move the pointer away. |
| `board-pad-hover` | The note pad pointed at | NOTEUI-043, NOTEUI-044 | 1. As board-populated.<br>2. Point at the pad’s top sheet. |
| `board-deck-fanned` | The agent deck fanned out beside its card | DECK-005, DECK-008, DECK-010 | 1. As board-populated.<br>2. Point at the agent deck under sf-skeleton’s card. |
| `board-expanded-stopped` | Expanded cards whose processes aren’t running: Resume session and Start shell | XCARD-011, XCARD-022 | 1. Open infra: an expanded Claude card and an expanded terminal card, neither running. |
| `board-expanded` | Expanded cards running: a Claude session with its usage, and a terminal | XCARD-003, XCARD-005, XCARD-007, XCARD-012, TERM-002 | 1. Open infra.<br>2. Click Resume session on in-certs and Start shell on the terminal card.<br>3. Report in-certs’ model, context and cost through its status line. |
| `board-menu-pane` | The board’s context menu | BOARD-172, BOARD-173, BOARD-176, BOARD-177 | 1. As board-populated.<br>2. Right-click the empty board below the terminal card. |
| `board-menu-card` | A card’s context menu (in a section) | BOARD-172, BOARD-175, BOARD-087 | 1. As board-populated.<br>2. Right-click sf-cart’s card. |
| `board-menu-note` | A note’s context menu | BOARD-120, BOARD-175 | 1. As board-populated.<br>2. Right-click the yellow note. |
| `board-menu-section` | A section’s context menu | BOARD-096, BOARD-095 | 1. As board-populated.<br>2. Right-click the Checkout section’s header. |
| `board-menu-archived` | An archived card’s context menu | BOARD-175, BOARD-044 | 1. As board-populated.<br>2. Right-click the archive stack’s top card. |
| `board-confirm-archive` | Archiving a working session asks first | CARD-022, BOARD-110 | 1. As board-populated.<br>2. Point at sf-skeleton’s card and click its Archive button. |
| `board-toast` | The board’s toast: an action run that finished while the board was closed | BOARD-106, BAR-032 | 1. Seed the Run tests action run on storefront’s board, not running.<br>2. Open storefront. |
| `board-zoomed-out` | storefront’s board zoomed out to 40% | BOARD-033 | 1. Seed storefront’s viewport at zoom 0.4.<br>2. As board-populated. |
| `board-zoomed-in` | storefront’s board zoomed in to 160% | BOARD-033 | 1. Seed storefront’s viewport at zoom 1.6 over the Checkout section.<br>2. As board-populated. |

## Prompt bar, ⌘K search, merge dialog, ink

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `bar-focused` | The prompt bar, focused and empty | BAR-004, BAR-006, BAR-007 | 1. As board-populated.<br>2. Click the prompt bar. |
| `bar-typing` | Typing a multi-line prompt | BAR-005, BAR-009 | 1. As board-populated.<br>2. Click the prompt bar and type three lines (⇧↩ between them). |
| `bar-slash-menu` | Typing / in the prompt bar: the menu of what else it makes | BAR-014, BAR-015 | 1. As board-populated.<br>2. Press / on the board. |
| `bar-view` | The /view token, with a description typed | BAR-006, BAR-017, BAR-019 | 1. As board-populated.<br>2. Click the prompt bar, type “/view ”, then a description. |
| `bar-note` | The /note token, with its colour swatch | BAR-017, BAR-019, BAR-022 | 1. As board-populated.<br>2. Click the prompt bar, type “/note ”, then the note. |
| `bar-one-selected` | One session selected: the bar sends into it | BAR-020, BAR-024 | 1. As board-populated.<br>2. ⇧-click sf-cart (running, done).<br>3. Click the prompt bar. |
| `bar-several-selected` | Several sessions selected: the bar asks about them | BAR-006, BAR-025, SEL-001 | 1. As board-populated.<br>2. ⇧-click sf-coupons and sf-search-plan.<br>3. Click the prompt bar and type a question. |
| `search-empty` | ⌘K search, just opened: the latest sessions | SRCH-001, SRCH-005, SRCH-010 | 1. As board-populated.<br>2. Press ⌘K. |
| `search-results` | ⌘K search results with highlighted snippets | SRCH-006, SRCH-010, SRCH-011 | 1. As board-populated.<br>2. Press ⌘K and type “coupon”. |
| `search-none` | ⌘K search with no match | SRCH-008 | 1. As board-populated.<br>2. Press ⌘K and type “zebra”. |
| `merge-progress` | The merge dialog summarizing: one ready, one running, one queued | MERGE-001, MERGE-004, MERGE-005 | 1. As board-populated, with summaries reported as they go and never finishing.<br>2. ⇧-click sf-cart, sf-coupons and sf-search-plan; click New session from context. |
| `merge-failed` | The merge dialog with a summary that failed: Retry and Skip | MERGE-007, MERGE-009, MERGE-010 | 1. As merge-progress, with sf-coupons’ summary failing and the others ready. |
| `merge-ready` | The merge dialog ready: the summaries to start from | MERGE-007, MERGE-008, MERGE-011 | 1. As merge-progress, with every summary ready. |
| `ink-pen` | Drawing with the pen (purple) | INKUI-001, INKUI-002, INKUI-004, INKUI-008, INKUI-010 | 1. As board-populated.<br>2. Pick the Pen, then Purple from the colour button.<br>3. Draw a stroke on the empty board below the terminal card. |
| `ink-palette` | The ink dock with its colour palette open | INKUI-001, INKUI-004 | 1. As board-populated.<br>2. Click the ink colour button. |
| `ink-arrow` | Drawing an arrow | INKUI-009 | 1. As board-populated.<br>2. Pick the Arrow (A).<br>3. Drag from below the terminal card up towards it. |
| `ink-text` | Writing an ink label | INKUI-013 | 1. As board-populated.<br>2. Pick Text.<br>3. Click the empty board below the terminal card and type “needs tests”. |
| `ink-eraser` | The eraser over the wavy stroke, mid-erase | INKUI-011 | 1. As board-populated.<br>2. Pick the Eraser (X).<br>3. Press on the wavy graphite stroke and drag along it, without letting go. |
| `ink-selected` | A selected ink stroke | INKUI-015, INKUI-026 | 1. As board-populated.<br>2. With the pointer tool, click the green “ship Friday” label. |

## Views

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `views-board` | docs-site’s board: a live view, a view being built, a command waiting for approval, notes of every colour | VIEW-122, VIEW-124, VIEW-125, VIEW-126, VIEW-135, VIEW-137, VIEW-027 | 1. Start the Broken links report’s brain and report it working.<br>2. Open docs-site.<br>3. Wait for the two pages to draw. |
| `views-brain-waiting` | A view whose brain needs the user | VIEW-124, VIEW-125, VIEW-137 | 1. As views-board, with the brain waiting on a permission. |
| `views-menu` | A view’s menu (⋯) | VIEW-134, VIEW-143 | 1. As views-board.<br>2. Point at Docs build status and click its More button. |
| `views-large` | A view opened large | VIEW-133, VIEW-147 | 1. As views-board.<br>2. Point at Docs build status and click Open large. |
| `views-change` | Asking Claude to change a view (✎) | VIEW-130, VIEW-131 | 1. As views-board.<br>2. Point at Docs build status, click Change view and type a request. |
| `views-new` | The New view dialog | VIEW-151, VIEW-152, VIEW-153, BOARD-144 | 1. As views-board.<br>2. Right-click the empty board and choose New view here…<br>3. Type a description. |

## Actions rail

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `actions-rail` | The actions rail slid out, an action pointed at | ACT-046, ACT-047, ACT-049, ACT-054, ACT-055, ACT-058 | 1. As board-populated.<br>2. Move the pointer to the window’s right edge, then onto Open a PR. |
| `actions-run` | An action running, fanned out beside the rail | ACT-056, ACT-057, ACT-063, ACT-064, ACT-065 | 1. As board-populated.<br>2. Slide the actions rail out and click Run tests (its run starts on the action’s prompt, so it reads as working).<br>3. Move the pointer away until the rail hides, then slide it out again and point at the running action. |
| `actions-new` | New action: describing it | ACT-061, ACT-067, ACT-068, ACT-069, ACT-070, ACT-071 | 1. As board-populated.<br>2. Slide the actions rail out and click +.<br>3. Type what the button should do. |
| `actions-drafting` | New action: Claude drafting it | ACT-072 | 1. As actions-new, with the stand-in slow to answer.<br>2. Click Draft with Claude. |
| `actions-review` | New action: reviewing Claude’s draft | ACT-074, ACT-076, ACT-077 | 1. As actions-drafting, with the stand-in answering at once (Check bundle size, gauge icon). |

## Git

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `git-log` | The branch chip pointed at: tracking and the last five commits | GIT-026, GIT-027, GIT-029, GIT-030, GIT-031 | 1. As board-populated.<br>2. Point at the branch chip in the title bar. |
| `git-diff` | The Changes window: modified, added, deleted and renamed files, a diff shown | GIT-025, GIT-032, GIT-034, GIT-036, GIT-037, GIT-040 | 1. As board-populated.<br>2. Click Changes in the title bar. |

## Terminal modal, transcript preview, agent viewer

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `terminal-claude` | A Claude session’s terminal window | MODAL-001, MODAL-014, MODAL-019, MODAL-021, TERM-002 | 1. As board-populated.<br>2. Click sf-cart’s card. |
| `terminal-shell` | A terminal card’s window | MODAL-012, MODAL-015 | 1. As board-populated.<br>2. Click the terminal card. |
| `terminal-stop-confirm` | Stopping a working session asks first | MODAL-022, MODAL-023 | 1. As board-populated.<br>2. Click sf-skeleton’s card (working).<br>3. Click Stop in its window. |
| `transcript-preview` | An archived session’s transcript | TRANS-001, TRANS-007 | 1. As board-populated.<br>2. Right-click the archive stack’s top card and choose Preview. |
| `agent-viewer` | The agent viewer: a session’s subagents, the first in the fan shown | AGV-001, AGV-005, AGV-006, AGV-009 | 1. As board-deck-fanned.<br>2. Click the first agent in the fan. |

## Floating capture

| Scene | Shows | Specs | Steps |
| --- | --- | --- | --- |
| `capture-empty` | The capture panel, just opened, nothing captured before | CAP-081, CAP-082, CAP-086, CAP-087, CAP-088, CAP-089, CAP-105, CAP-106 | 1. Seed no earlier captures.<br>2. Open File › Capture… |
| `capture-recents` | The capture panel with recent captures and their live status | CAP-118, CAP-150, CAP-152, CAP-153 | 1. Start storefront’s sessions (sf-tax waiting, sf-cart running).<br>2. Open File › Capture… |
| `capture-guessing` | Typing: Claude working out where it goes | CAP-086, CAP-088 | 1. Seed no earlier captures; the stand-in is slow to guess.<br>2. Open the panel and type a request. |
| `capture-guess` | Typing, with Claude’s guess: a new session in storefront | CAP-080, CAP-086, CAP-088, CAP-106 | 1. Seed no earlier captures.<br>2. Open the panel and type a request.<br>3. Wait for Claude’s guess (the stand-in answers: new session, storefront). |
| `capture-kind-menu` | Where it goes: the kind and project menu | CAP-107, CAP-108, CAP-109, CAP-110, CAP-112 | 1. As capture-guess.<br>2. Click the header (where it goes). |
| `capture-attachments` | Attachments: the browser tab, a screenshot, a file, and copied text | CAP-091, CAP-092, CAP-093, CAP-094, CAP-095, CAP-098 | 1. Seed no earlier captures. Stand in for the screenshot, the file picker and the clipboard.<br>2. Open the panel; it learns the browser tab in front and offers the copied text.<br>3. Take a screenshot, attach a file, take the copied text, type a request. |
| `capture-receipt-note` | The receipt after capturing a note | CAP-140, CAP-141, CAP-146 | 1. Seed no earlier captures; the stand-in guesses a note in storefront.<br>2. Open the panel, type a reminder, wait for the guess and press ↩. |
| `capture-receipt-session` | The receipt after starting a session, with its sticker | CAP-140, CAP-142, CAP-143, CAP-146 | 1. As capture-guess, with making the session stood in for (it would start Claude).<br>2. Press ↩. |
| `capture-permission` | The panel asking for Accessibility, for the double tap | CAP-032, CAP-117 | 1. Seed no earlier captures.<br>2. Open the panel and tell it the shortcut needs permission. |
| `capture-screen-permission` | A screenshot macOS refused: the Screen Recording notice | CAP-100, CAP-101 | 1. Seed no earlier captures; the screenshot reports that Screen Recording isn’t allowed.<br>2. Open the panel and click Take a screenshot. |
