# Parity report: the Rust app against 0.9.4's goldens

The final sweep (`migration/tracks/final-sweep.md`) rendered every golden scene, 93 scenes in light and dark, with
`main` plus the sweep's fixes, and compared each with 0.9.4's golden from tag v0.9.4. Numbers only; no images kept.

- **How:** the scene runners in `crates/nodus-ui/parity/` (`scenes.py`, `terminal_scenes.py`, `scenes05.py`,
  `scenes11.py`, `views_scenes.py`, `capture_scenes.py`) on the goldens' own world (`nodus-parity` in macOS's
  Shared folder), debug build, Node 22, macOS 27.2, the display asleep for the whole run (the driver draws by hand; see the
  track file).
- **Measure:** pixelmatch at threshold 0.1 over the whole shot (2880×1800, or the capture panel's window at 2×), as
  `compare.mjs` counts it: the share of pixels that differ.
- **Where:** the box around every differing pixel in CSS px (x,y w×h) for the worse theme, then the three cells of an
  8×8 grid (180×112.5 px; top-left corner given) holding most of them, with their share.
- **Result:** median 0.39 %; 87 of 93 scenes under 1 % in both themes. Of the 6 over: rail-open and the light shots
  of bar-several-selected, merge-ready, merge-failed and capture-kind-menu are artefacts of how the goldens were
  taken (each one's other theme is under 1 %, except rail-open, scrolled in both); settings-ready is a stub the
  driver doesn't have. The one difference in the app among them is small: the merge dialog's Context wraps a word
  early (track file, Findings left).
- What stays under 1 % is text (anti-aliasing and half-pixel placement of the same glyphs, gpui-text's remaining
  differences), shadows and blur edges, and the minimap's rounding.

| Scene | Light % | Dark % | Where (CSS px: box; most-different cells) | Over 1%: why |
| --- | --- | --- | --- | --- |
| `actions-drafting` | 0.31 | 0.28 | 391,27 967×787; 1260,788:33%, 540,338:14%, 360,338:11% |  |
| `actions-new` | 0.34 | 0.31 | 391,27 967×787; 1260,788:29%, 540,338:15%, 360,338:11% |  |
| `actions-rail` | 0.69 | 0.69 | 28,21 1397×864; 1260,788:15%, 900,225:11%, 360,338:7% |  |
| `actions-review` | 0.19 | 0.17 | 391,27 966×787; 1260,788:52%, 1080,113:14%, 540,450:10% |  |
| `actions-run` | 0.69 | 0.68 | 28,21 1398×864; 1260,788:15%, 900,225:11%, 360,338:7% |  |
| `agent-viewer` | 0.14 | 0.14 | 199,117 1036×229; 180,225:26%, 360,225:15%, 180,113:13% |  |
| `bar-focused` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `bar-note` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `bar-one-selected` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `bar-several-selected` | 6.66 | 0.39 | 0,12 1426×886; 1260,788:7%, 1260,450:7%, 1080,450:7% | Light only: Chromium artefact in the light golden, the page scrolled left as the prompt bar took the text (the dark golden isn't; dark 0.39 %). |
| `bar-slash-menu` | 0.40 | 0.40 | 28,21 1397×864; 360,338:11%, 1260,788:6%, 180,450:6% |  |
| `bar-typing` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `bar-view` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-archive-hover` | 0.42 | 0.42 | 28,21 1397×864; 360,338:11%, 1260,788:6%, 180,450:6% |  |
| `board-archive-open` | 0.41 | 0.40 | 28,21 1397×864; 360,225:10%, 360,450:8%, 540,225:6% |  |
| `board-archive-search` | 0.40 | 0.39 | 28,21 1397×864; 540,225:9%, 360,225:8%, 1260,788:6% |  |
| `board-card-hover` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-confirm-archive` | 0.34 | 0.32 | 390,27 872×501; 540,338:27%, 360,338:25%, 720,450:20% |  |
| `board-deck-fanned` | 0.60 | 0.57 | 28,21 1397×864; 540,338:17%, 720,338:9%, 540,225:8% |  |
| `board-empty` | 0.34 | 0.34 | 28,21 1397×864; 540,113:28%, 720,113:28%, 1260,788:7% |  |
| `board-expanded` | 0.55 | 0.54 | 28,21 1397×864; 900,113:9%, 360,338:9%, 360,225:8% |  |
| `board-expanded-stopped` | 0.23 | 0.22 | 28,21 1397×864; 0,113:18%, 0,338:11%, 180,338:11% |  |
| `board-marquee` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-menu-archived` | 0.53 | 0.51 | 28,21 1397×864; 180,338:16%, 360,338:10%, 180,225:6% |  |
| `board-menu-card` | 0.59 | 0.58 | 28,21 1397×864; 1080,338:13%, 360,338:8%, 1080,450:7% |  |
| `board-menu-note` | 0.41 | 0.42 | 28,21 1391×851; 360,338:11%, 1260,338:6%, 180,450:6% |  |
| `board-menu-pane` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-menu-section` | 0.74 | 0.68 | 28,21 1397×864; 720,113:22%, 720,225:12%, 360,338:6% |  |
| `board-note-editing` | 0.43 | 0.43 | 28,21 1397×864; 360,338:11%, 1080,113:6%, 1260,788:6% |  |
| `board-note-selected` | 0.40 | 0.40 | 28,21 1397×864; 360,338:11%, 1260,788:6%, 180,450:6% |  |
| `board-pad-hover` | 0.63 | 0.62 | 28,21 1397×864; 180,563:33%, 360,338:7%, 360,563:6% |  |
| `board-populated` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-section-naming` | 0.48 | 0.49 | 28,21 1391×851; 360,338:14%, 360,450:6%, 360,563:6% |  |
| `board-section-rename` | 0.47 | 0.48 | 28,21 1391×851; 540,113:13%, 720,113:11%, 360,338:9% |  |
| `board-selected` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `board-toast` | 0.38 | 0.36 | 28,21 1397×864; 1260,788:7%, 180,450:6%, 360,450:6% |  |
| `board-zoomed-in` | 0.55 | 0.66 | 13,21 1428×871; 1260,788:13%, 180,788:7%, 360,788:7% |  |
| `board-zoomed-out` | 0.43 | 0.42 | 28,21 1397×864; 540,338:17%, 540,450:10%, 720,225:9% |  |
| `capture-attachments` | 0.16 | 0.18 | 83,179 486×195; 82,284:57%, 163,284:29%, 326,170:3% |  |
| `capture-empty` | 0.01 | 0.01 | 150,195 419×18; 326,183:89%, 82,183:10%, 489,183:2% |  |
| `capture-guess` | 0.14 | 0.01 | 138,54 445×159; 489,37:45%, 571,37:21%, 408,37:19% |  |
| `capture-guessing` | 0.01 | 0.01 | 150,195 419×18; 326,183:89%, 82,183:10%, 489,183:2% |  |
| `capture-kind-menu` | size ≠ | 0.08 | 318,108 135×180; 326,192:29%, 326,128:19%, 326,257:14% | Light only: golden artefact, the light golden's window is 8 px shorter than the dark golden's (its panel hadn't grown for the menu yet), so the sizes differ; over the golden's 505 px the shots differ by 0.09 % (dark 0.08 %). |
| `capture-permission` | 0.01 | 0.09 | 150,196 425×81; 489,223:90%, 326,178:6%, 571,223:2% |  |
| `capture-receipt-note` | 0.57 | 0.16 | 216,49 220×181; 245,194:41%, 326,194:30%, 245,78:12% |  |
| `capture-receipt-session` | 0.15 | 0.13 | 193,39 289×80; 408,37:66%, 408,75:29%, 163,37:2% |  |
| `capture-recents` | 0.03 | 0.03 | 55,195 528×149; 408,274:30%, 0,220:15%, 408,220:14% |  |
| `capture-screen-permission` | 0.01 | 0.01 | 150,241 419×18; 326,211:69%, 326,254:20%, 82,254:10% |  |
| `git-diff` | 0.31 | 0.30 | 155,27 1202×787; 720,113:27%, 540,113:25%, 1260,788:23% |  |
| `git-log` | 0.91 | 0.84 | 28,21 1397×864; 360,225:12%, 1260,788:11%, 900,225:9% |  |
| `ink-arrow` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `ink-eraser` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `ink-palette` | 0.40 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `ink-pen` | 0.39 | 0.39 | 28,21 1397×864; 360,338:12%, 1260,788:6%, 180,450:6% |  |
| `ink-selected` | 0.40 | 0.40 | 28,21 1397×864; 360,338:11%, 1260,788:6%, 180,450:6% |  |
| `ink-text` | 0.42 | 0.41 | 28,21 1397×864; 360,338:11%, 1260,788:6%, 180,450:6% |  |
| `merge-failed` | 3.24 | 0.56 | 19,16 1391×855; 1260,788:14%, 1260,450:13%, 1080,450:12% | As merge-ready (dark 0.56 %). |
| `merge-progress` | 0.04 | 0.04 | 391,27 871×561; 360,338:66%, 900,563:20%, 720,563:13% |  |
| `merge-ready` | 3.30 | 0.62 | 19,16 1391×855; 1260,788:14%, 1260,450:12%, 1080,450:12% | Light only: the light golden's board behind the dialog is scrolled left as in bar-several-selected (dark 0.62 %). In both themes, a real one: Context's first line wraps a word early (track file, Findings left). |
| `projects-context-menu` | 0.10 | 0.13 | 57,22 1356×839; 0,113:26%, 360,450:11%, 180,450:10% |  |
| `projects-context-menu-focus` | 0.10 | 0.14 | 51,22 1362×839; 360,113:19%, 0,338:10%, 360,450:10% |  |
| `projects-drop` | 0.25 | 0.24 | 12,22 1416×866; 1260,788:8%, 0,788:7%, 360,788:6% |  |
| `projects-empty` | 0.00 | 0.03 | 672,22 741×528; 540,338:47%, 720,338:41%, 1260,0:11% |  |
| `projects-focus` | 0.14 | 0.59 | 39,21 1374×593; 360,113:69%, 360,225:19%, 0,563:3% |  |
| `projects-focus-none` | 0.08 | 0.10 | 57,22 1356×839; 360,450:13%, 180,450:12%, 0,338:12% |  |
| `projects-grid` | 0.08 | 0.11 | 57,22 1356×839; 360,450:13%, 180,450:12%, 0,338:12% |  |
| `projects-group-menu` | 0.36 | 0.49 | 57,22 1356×839; 180,338:59%, 0,338:13%, 180,450:7% |  |
| `projects-group-picker` | 0.03 | 0.02 | 577,518 284×12; 540,450:79%, 720,450:21% |  |
| `projects-remove-confirm` | 0.04 | 0.04 | 526,377 359×100; 720,450:55%, 540,450:45%, 360,338:0% |  |
| `projects-rename` | 0.15 | 0.18 | 32,22 1381×839; 0,225:35%, 360,450:9%, 180,450:9% |  |
| `projects-selected` | 0.07 | 0.10 | 57,22 1356×839; 360,450:13%, 180,450:12%, 0,338:12% |  |
| `projects-toast` | 0.30 | 0.10 | 57,12 1369×849; 1260,0:76%, 0,338:4%, 360,338:4% |  |
| `rail-numbers` | 0.39 | 0.40 | 3,21 1416×854; 180,450:9%, 180,225:6%, 540,338:5% |  |
| `rail-open` | 4.36 | 4.15 | 1,12 1426×886; 0,563:7%, 0,450:7%, 1260,788:6% | Chromium artefact in the golden: the whole page, title bar and board included, is scrolled 14 px left once the rail has slid in. The Rust rail sits where rail-pinned and rail-numbers put the same rail (both 0.39 %). |
| `rail-pinned` | 0.39 | 0.39 | 3,21 1422×864; 180,450:8%, 180,225:7%, 1260,788:6% |  |
| `search-empty` | 0.20 | 0.19 | 407,27 855×814; 900,225:25%, 900,338:23%, 900,113:14% |  |
| `search-none` | 0.12 | 0.05 | 391,27 871×337; 360,338:52%, 540,338:20%, 540,225:9% |  |
| `search-results` | 0.15 | 0.14 | 407,27 855×352; 540,225:34%, 720,225:25%, 360,225:18% |  |
| `settings` | 0.14 | 0.14 | 523,52 415×818; 540,563:18%, 540,675:10%, 540,450:10% |  |
| `settings-bottom` | 0.21 | 0.20 | 523,52 415×821; 540,563:25%, 540,225:15%, 720,563:13% |  |
| `settings-ready` | 1.04 | 1.21 | 522,81 415×792; 720,450:12%, 540,450:9%, 540,338:9% | Harness: 0.9.4's scene stubbed the xAI key as saved (“Replace the saved xAI API key”, “Saved in your Keychain. Remove”); the driver has no stub for the key's source, so that row's copy differs and the body scrolls 18 px less. The update rows and the rest match. |
| `terminal-claude` | 0.11 | 0.02 | 78,64 1281×800; 360,788:58%, 540,788:20%, 1080,0:9% |  |
| `terminal-shell` | 0.01 | 0.02 | 76,64 1284×28; 1260,0:45%, 1080,0:45%, 0,0:10% |  |
| `terminal-stop-confirm` | 0.01 | 0.00 | 432,395 188×466; 360,788:91%, 540,788:9%, 540,338:0% |  |
| `titlebar-running` | 0.09 | 0.12 | 57,22 1356×839; 360,450:12%, 180,450:11%, 0,338:11% |  |
| `titlebar-update` | 0.08 | 0.11 | 57,21 1356×840; 360,450:13%, 180,450:12%, 0,338:12% |  |
| `transcript-preview` | 0.02 | 0.02 | 364,127 711×219; 360,225:90%, 360,338:9%, 900,113:0% |  |
| `views-board` | 0.49 | 0.49 | 28,21 1397×864; 0,113:12%, 1080,113:10%, 900,225:9% |  |
| `views-brain-waiting` | 0.54 | 0.54 | 28,21 1397×864; 0,113:11%, 1080,113:9%, 900,225:9% |  |
| `views-change` | 0.56 | 0.55 | 28,21 1397×864; 0,113:11%, 540,225:10%, 1080,113:8% |  |
| `views-large` | 0.06 | 0.06 | 58,27 1325×820; 0,225:41%, 0,113:19%, 1080,225:15% |  |
| `views-menu` | 0.53 | 0.52 | 28,21 1397×864; 0,113:11%, 1080,113:9%, 720,113:8% |  |
| `views-new` | 0.05 | 0.05 | 430,27 875×620; 540,563:22%, 540,338:22%, 360,338:13% |  |
