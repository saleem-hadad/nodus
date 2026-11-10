# Track: final-sweep (P12, before 1.0.0)

- **Branch:** `rust/final-sweep`
- **Scope:** the last parity pass before the first Rust release: every golden scene in both themes against main, the
  unticked IDs that don't wait on Windows CI, the certificate or the release, the two known differences in
  plan.md's integration debts (Edit › Delete, the capture panel's zoom), and a live end-to-end pass.
- **Status:** ✅ ready to merge. One real bug found and fixed (the board's fields took no typed text); both known
  differences matched; 9 IDs ticked; the end-to-end pass is 42 of 42.

## How to check

```sh
cargo build -p nodus
# Node 22 first on PATH (the stand-ins need it); a folder of the goldens to write beside:
python3 crates/nodus-ui/parity/scenes.py board- projects- titlebar- settings rail- --golden <dir> --world <world>
python3 crates/nodus-ui/parity/terminal_scenes.py projects-grid titlebar-running terminal- transcript- agent- --golden <dir> --world <world>
python3 crates/nodus-ui/parity/scenes05.py --out <dir> --world <world>
python3 crates/nodus-ui/parity/scenes11.py ink- bar- search- merge- --golden <dir> --world <world>
python3 crates/nodus-ui/parity/views_scenes.py views- --golden <dir> --world <world>
python3 crates/nodus-ui/parity/capture_scenes.py capture- settings-ready --golden <dir> --world <world> --node <node 22>
NODE_PATH=migration/parity/node_modules node crates/nodus-ui/parity/compare.mjs <dir>
python3 crates/nodus-ui/parity/e2e.py --world <scratch folder>
```

`<world>` is best the goldens' own (`nodus-parity` in macOS's Shared folder, `DEFAULT_WORK` in the seed), so paths
the UI prints are the same length.

A Mac whose display has gone to sleep drops into dark wakes: the run stalls for 10–25 minutes at a time. Keep it
awake with `caffeinate -s -i -w <runner's pid>`. With the display asleep and the system awake, GPUI is asked for no
frames: the driver now draws by hand before every command and before typing, running what `on_next_frame` queued
first (GPUI patch 17), and the end-to-end pass draws before it reads a terminal or the viewport. The numbers in
report.md come from one run on the final build with the display asleep the whole time.

## Results

The numbers are in `migration/parity/report.md` (every scene, light and dark, with a reason for each over 1%):
median 0.39 %, 87 of 93 scenes under 1 % in both themes. Over 1 %: rail-open (4.4/4.2, the golden's page scrolled
14 px left), the light shots of bar-several-selected (6.7), merge-ready (3.3) and merge-failed (3.2) (the light
goldens' board scrolled left; their dark shots are 0.39–0.62), capture-kind-menu light (the light golden's window
8 px shorter than its dark one; 0.09 % over its height), settings-ready (1.0/1.2: 0.9.4's scene stubbed a saved xAI
key, which the driver can't). The scene runs before the fixes had board-archive-search 1.76, board-confirm-archive
1.24, actions-review 11.15, git-diff 34.3 and settings-bottom 1.61 %; all are now 0.17–0.40 %.

## Fixed

- **The board's fields took no typed text** (577a050). The archive's search, a note being written, a card's and a
  section's name each stopped every key from reaching the board (`.on_key_down(|_, _, cx| cx.stop_propagation())`).
  In GPUI a stopped key counts as handled, and macOS's `handle_key_event` then never calls `insertText:` (GPUI's
  `dispatch_keystroke` the same), so none of the four took a character. They now mark the key
  (`window.prevent_default()`), and the shell's board keys return early for a marked key. Found by the
  board-archive-search scene (typing “fix” changed nothing); checked with the driver's `native-keys`, which sends keys
  through AppKit's own path, and by the end-to-end pass writing a note. board-archive-search 1.76 % → 0.40 %.
- **Edit › Delete with only a caret** (7735803): now does nothing, as Chromium's `delete` role (Blink's menu Delete
  needs a range). `shell/menus.rs` keeps every text field's state (`observe_new`) and asks the focused one for a
  selection before sending gpui-component's `Delete`. Checked with the driver's `action` command on a project's name
  field: a caret at the start leaves “storefront”; “st” selected leaves “orefront”.
- **⌘+ / ⌘- / ⌘0 and the capture panel** (2f4e5dc). 0.9.4: the zoom roles act on the focused page
  (`webContents.getFocusedWebContents()`), and Chromium keys a `file://` page's zoom by its whole URL
  (`net::GetHostOrSpecFromURL` → the spec, `#…` included). The main window loaded `…/out/renderer/index.html`
  (`loadFile`), the panel `…/index.html#capture` (`loadURL` in `src/main/capture.ts`): same origin, different keys,
  so with the panel in front the panel zoomed alone, by a level kept apart and restored on the next launch. Its window
  stayed 652 wide and as tall as the page's CSS height (`capture:resize`), so a zoomed-in panel was clipped. Now the
  same: `nodus_core::zoom` keeps `captureZoomLevel` beside `zoomLevel` (brought over once from Preferences'
  `#capture` key), `shell::window::zoom` zooms the panel while it has the keyboard, and the panel is created at its
  level. Test `zoom::tests::keeps_the_capture_panels_level_apart`; driver: `capture zoom` twice → panel 1.0, window 0,
  the panel drawn 1.2× in the same 652×439 window; a relaunch draws it byte-identical; View › Zoom In with the main
  window in front moved only `zoomLevel`.
- **Test driver** (c5ccfb6, 7735803, 55eb06f, 591a1c0): `stub pickFolder` (projects-toast), `action` (a GPUI action
  by name), `native-keys`, `board` (a board as the core has it, and its live viewport), `pick` (an open context
  menu's row by label), `capture zoom`; App Nap kept away while driven; typing draws first and, in the main window,
  goes a key at a time with a frame between; and every frame the driver draws by hand first runs what
  `on_next_frame` queued (GPUI patch 17, `Window::run_next_frame_callbacks`), so a field that takes the focus a frame
  or two after it shows (the New action box, the grid's and a group's rename) gets it with the display asleep:
  actions-review 11.15 % → 0.19 %.
- **Scene runners** (c5ccfb6, 030fc1b, e638be5): the three goldens no runner rendered (board-archive-search,
  projects-toast, titlebar-running; projects-grid now with storefront's sessions live, as 0.9.4's), `try-scheme`
  for PLAT-009; board-confirm-archive presses the card's Archive button while pointed at, as 0.9.4's scene did
  (⇧-click and ⌫ left the selection bar and ring behind the dialog: 1.24 % → 0.34 %); git-diff points at Changes
  before pressing it, as Playwright's click does (34 % → 0.31 %); settings-bottom and settings-ready set the update
  status after Settings has asked for its own, and again once scrolled (0.9.4's scenes replaced the handler; a debug
  build's own status is "off": 1.6 % → 0.21 %).
- **Smoke test** (0d24bc7): `packaging/smoke.py` shoots the new project's board with its terminal card in light and
  dark (`2-board-light.png`, `3-board-dark.png`), as REL-032 asks, where it shot the grid three times. Passed on the
  debug build with stand-in HOME and PATH (never the real claude) and `/bin/sh`: `nodus-42` on the card in both.

## IDs resolved

- **PLAT-009** — `views/webview.rs` registers `nodus-view` with wry's `with_custom_protocol`; checked live
  (`views_scenes.py try-scheme`, the page's `setState` read from `runtime.json`): `isSecureContext` true,
  `crypto.subtle` an object, `location.protocol` `nodus-view:`, host the view id, path `/`, `self.origin` `null`
  (the sandboxed frame's opaque origin, as 0.9.4's iframe).
- **PLAT-026** — `views/shell.rs` `allow_navigation` behind `with_navigation_handler`, which wry calls for every
  frame's navigation (`wkwebview/navigation.rs` has no main-frame filter): only this webview's own shell once, then
  only its own view's `nodus-view://<id>/…`; never opened in the browser. Test `keeps_a_frame_on_its_own_page`.
- **PLAT-027** — no `window.api` anywhere (GPUI draws the app); a view's webview gets only the shell's relay, whose
  messages are taken only from the shell document itself (`webview.rs` `with_ipc_handler` → `shell::is_shell`) and
  answer the view's own `window.nodus`.
- **PLAT-045** — the driver's `menus` on this macOS against `src/main/menu.ts` at v0.9.4: the same menus, items and
  order (Nodus, File, Edit, View, Window, Help). AppKit's additions are the same because both register the same
  menus with it: Edit (standard selectors) gets Writing Tools, AutoFill, Start Dictation… and Emoji & Symbols after
  Search Sessions…; Window is `NSApp.windowsMenu` (the window list); Services and Help (`setHelpMenu`, its search
  field) likewise. View gets no second full-screen item (Toggle Full Screen is there) and no tab items (neither
  allows automatic tabbing). The only differences are the user's decisions: Force Reload and Toggle Developer Tools
  gone, Reload in debug builds only.
- **PLAT-136** — the shell document's CSP `frame-src nodus-view:` (`shell_html`, `frame-src https://nodus-view.<id>`
  on Windows) plus PLAT-026's handler.
- **UI-066** — `board/store.rs` `save_ink`: replaces the board's strokes and saves them through `board.save_ink`;
  nodus-board test `saving_replaces_the_boards_ink`; the end-to-end pass's stroke is in the core's board after.
- **REL-037** — no `postinstall.cjs` (or any postinstall) in the tree since the cutover.
- **REL-038** — README.md and docs/index.html link `releases/latest/download/` + `Nodus-arm64.dmg`, `Nodus-x64.dmg`,
  `Nodus-Setup-x64.exe`, `Nodus-Setup-arm64.exe`, exactly the four names `release.yml` uploads.
- **REL-032** — implemented in 0d24bc7 (above); the same script runs in CI on both OSes (`package.yml`,
  `release.yml`), on Windows through the driver's named pipe.

Left open: PLAT-007, PLAT-008, PLAT-018, PLAT-035 (decision), UI-099, CAP-056, REL-034 (Windows); REL-008, REL-009,
REL-041, REL-046 (certificate); REL-002, REL-013, REL-014, REL-039 (the release: `release.yml` already refuses a
version that isn't X.Y.Z or newer than 0.9.4, and the workspace is 1.0.0), REL-016…REL-028, REL-044, REL-040 (Windows
packaging and the handoff gates in CI).

## Findings left

- ~~**The merge dialog's Context wraps a word early.**~~ Fixed after the sweep (rust/merge-wrap): 0.9.4 drew all of `--font-mono` in Menlo (Chromium can't find the hidden SF Mono by name), so `fonts::mono()` is Menlo on macOS app-wide, and multi-line fields stack at their exact line height (GPUI PATCHES 18). merge-ready dark 0.62 → 0.04 %. With 0.9.4's handoff text, its first line ends at “handoff”
  where 0.9.4's ended at “summary”: the text starts at the same x, so the field's wrap width is a few px narrower than
  0.9.4's `padding: 12px 14px` textarea (gpui-component's `RIGHT_MARGIN` and padding against CSS's soft wrap, which
  lets a trailing space hang). Small; in merge-ready and merge-failed. File: `board/overlays/merge/view.rs` (the
  context `frame`'s right inset, as `board/cards/note.rs` widens its field by the margin).
- Everything else over 1 % in report.md is in how 0.9.4's goldens were taken, not in the app.
- Checked with the patch: typing into the grid's rename (“shop”) and a section's rename (“Payments”) both land.

## End-to-end pass

`crates/nodus-ui/parity/e2e.py`, on the seeded world with the stand-ins (the shell's echoes what's typed, Claude's
logs its input and arguments; never the real claude), the window never in front. 42 of 42:

| Step | Result |
| --- | --- |
| 1. New Project (picker stubbed to a fresh folder) | added; its board opens |
| 2. File › New Session, New Terminal | each opens its window; one Claude card, one terminal card |
| 3. Typing | the shell's process echoes `ls -la`; Claude's logs `add a sort menu` |
| 4. Hooks posted to the host | UserPromptSubmit → working, PermissionRequest → waiting, Stop → done (HTTP 204) |
| 5. Expand / collapse | closing the window left it expanded (BOARD-103); its menu's Collapse; E expands; Collapse |
| 6. Archive / restore (storefront's sf-cart) | ⌫ archives; the archived card's menu Restore to board restores |
| 7. Note | New note here, typed, Escape: text saved; its menu's Start session starts one with the note as prompt |
| 8. Ink | P, a stroke: one more stroke in the core's board |
| 9. View | New view here…, described, ⌘↩: a view with a brain started interactive with the description, at its prompt |
| 10. ⌘K | opens; Escape closes |
| 11. Merge | two Claude cards, New session from 2 contexts…, Start session: a card whose parents are both |
| 12. File › Capture… | the panel is up (652×439) |
| 13. Settings | Dark, then Light (saved); the xAI key field takes typing, masked, nothing saved; update rows checking → downloading 42% → ready → latest |
| 14. Quit and relaunch | the host outlives the app and is adopted (same pid); the terminal and Claude still live; the terminal still takes typing |

Notes from it: a Claude session nothing was said in is dropped rather than archived (SESS-091, as 0.9.4), so the
archive step uses a seeded session with a transcript; a terminal card isn't part of a merge (no transcript), as in
0.9.4.
