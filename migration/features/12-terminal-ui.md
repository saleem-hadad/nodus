# 12 · Terminal, session window, transcript & agents viewer

Sources:
- `src/renderer/src/terminal/registry.ts` (every line)
- `src/renderer/src/terminal/useAttachedTerminal.ts` (every line)
- `src/renderer/src/terminal/TerminalModal.tsx` (every line)
- `src/renderer/src/terminal/TranscriptPreview.tsx` (every line)
- `src/renderer/src/terminal/AgentViewer.tsx` (every line)
- `src/renderer/src/terminal/terminal.css` (every line)
- `src/renderer/src/components/board/ExpandedCard.tsx` (terminal side: wake/start, attach, focus, wheel arbitration, body overlays, mic)
- `src/renderer/src/views/BoardView.tsx` (terminal side only: lines 873-874, 881-893, 897-907, 1127-1133, 1924-1927, 2371-2380)
- `src/renderer/src/components/board/board.css` `.card-term` (lines 404-411)
- `src/renderer/src/styles/tokens.css`, `src/renderer/src/styles/global.css` (`.btn`, `.status-dot`, `.selectable`, reduced motion)
- `src/renderer/src/components/Modal.tsx`, `Modal.css` (the box all three windows live in; spec 08 owns the component)
- `src/renderer/src/App.tsx` (⌘W close order, Windows Ctrl+N/T/W), `src/main/menu.ts` (accelerators the terminal yields to), `src/main/index.ts:74-85` (link opening)
- `src/renderer/src/store.ts` (openTerminal/closeTerminal/openPreview/closePreview/openAgents/closeAgents, `cardTitle`, `isTerminal`, `tildePath`, `byUrgency`, `formatDuration`, `agentStep`, `agentStatusLabel`, `nestAgents`, dispose call sites)
- `src/renderer/src/components/board/stopSessions.ts` (+ `.test.ts`), `ConfirmDialog.tsx`, `AgentDeck.tsx` (`useNow`, `agentElapsed`), `components/UsageStats.tsx`, `components/Highlight.tsx`, `src/shared/search.ts` (`queryTerms`, `matchRanges`), `src/renderer/src/platform.ts`
- `node_modules/@xterm/xterm` 6.0.0 (`OptionsService.ts` defaults, `ThemeService.ts`, `Keyboard.ts`, `CoreBrowserTerminal.ts`, `Viewport.ts`, `SelectionService.ts`, `CoreMouseService.ts`, `Linkifier.ts`, `OscLinkProvider.ts`, `Clipboard.ts`, `InputHandler.ts`, `vs/base/browser/ui/scrollbar/*`, `css/xterm.css`), `@xterm/addon-fit` 0.11.0, `@xterm/addon-webgl` 0.19.0, `@xterm/addon-unicode11` 0.9.0, `@xterm/addon-web-links` 0.12.0
- `docs/how-it-works.md` (Sessions, Terminal cards, Expanded cards, Agents, Windows, Shortcuts), `docs/screenshots/terminal*.png`, `expanded*.png`

Other domains referenced: pty/host transport and ordering of `ev:pty-data` (02-session-host-status), `session:open` replay/resume semantics and transcript/agent parsing (03-sessions-transcripts), card chrome and the expanded card title bar buttons (10-board-cards-notes), Modal/ConfirmDialog/tokens (08-projects-shell-ui), board pan/zoom/shortcuts (09-board-canvas), ⌘K search (11-board-overlays), the mic (05-actions-voice-git).

## Summary

Every session (Claude or shell) has exactly one terminal emulator for the life of the window, kept in a registry keyed by session id. The same terminal is shown either in the **session window** (a modal over the board) or in the session's **expanded card** on the board, never both; hiding it parks it offscreen with its screen, scrollback and modes intact, and it keeps receiving output while parked. Opening a session asks the main process to start or resume the process and returns a replay of its buffered output; output that arrives while the open is in flight is already part of that replay and is dropped. The terminal is xterm.js 6 with fit, unicode11, web-links and WebGL addons and a custom key handler that adds ⇧Enter-newline for Claude, ⌘K clear, ⌘C copy and ⌘A select-all on a Mac, and Windows-Terminal-style Ctrl semantics off a Mac. The terminal colours are a fixed dark palette in both app themes.

Around it: the session window (status, title, branch/cwd, copyable session id, usage, mic, Resume/Restart, Stop with confirmation, Archive/Remove, close; Esc never closes it), a read-only **transcript preview** for archived sessions (search highlighting, collapse long messages, tool chips, Restore & Open), and the **agents viewer** (nested list of a session's subagents, the selected agent's live transcript, polling, stick-to-bottom, a live "what it's doing" line).

In Rust this becomes `nodus-terminal`: an `alacritty_terminal::Term` per session plus our own key encoder, mouse/selection/link handling and a GPUI element that draws the grid, and `nodus-ui` views for the three windows.

## Terminal emulation

### Options passed to `new Terminal()` and their effective values

Values marked *default* are xterm 6.0.0 defaults the app relies on (`node_modules/@xterm/xterm/src/common/services/OptionsService.ts:12-59`).

| Option | Effective value | Source | What the Rust terminal must do |
| --- | --- | --- | --- |
| `fontFamily` | `'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace` | app | First installed family wins: macOS → SF Mono (`/System/Library/Fonts/SFNSMono.ttf`); Windows → Cascadia Mono, else Consolas; last resort the platform monospace. Same list as `--font-mono`. |
| `fontSize` | `13` (CSS px) | app | 13 logical px. |
| `lineHeight` | `1.2` | app | Cell height = floor(device char height × 1.2) (see metrics). |
| `letterSpacing` | `0` | default | Cell width = glyph advance, no extra spacing. |
| `fontWeight` | `'normal'` (400) | default | Regular weight for normal text. |
| `fontWeightBold` | `'bold'` (700) | default | SGR 1 text drawn at weight 700. |
| `cursorBlink` | `true` | app | Blink while focused (600 ms phases). |
| `cursorStyle` | `'block'` | default | Filled block while focused. |
| `cursorInactiveStyle` | `'outline'` | default | 1 device-px outline of the cell in the cursor colour while unfocused, no blinking. |
| `cursorWidth` | `1` | default | Bar cursor (if an app asks for one via DECSCUSR) is 1 CSS px wide. |
| `scrollback` | `10000` | app | 10,000 lines of history per terminal. |
| `macOptionIsMeta` | `false` | app (= default) | On a Mac, ⌥ composes characters (⌥e é etc.); ⌥ is not Meta except for the non-printing keys listed in the key table. |
| `macOptionClickForcesSelection` | `false` | default | On a Mac there is no modifier that forces selection while an app has mouse reporting on. |
| `rightClickSelectsWord` | `true` on macOS, `false` elsewhere | default (`isMac`) | Mac right-click selects the word under the pointer (TERM-061). |
| `altClickMovesCursor` | `true` | default | ⌥/Alt+click moves the cursor (TERM-062). |
| `allowProposedApi` | `true` | app | Needed for `term.unicode.activeVersion`; no behavioural effect to port. |
| `scrollSensitivity` | `1` | default | Wheel multiplier 1. |
| `fastScrollSensitivity` | `5` | default | Alt (and, on the reporting/alt-buffer paths, Ctrl or Shift) multiplies wheel distance by 5. |
| `fastScrollModifier` | `'alt'` | default | — |
| `smoothScrollDuration` | `0` | default | Wheel scrolling is applied immediately, no animation. |
| `scrollOnUserInput` | `true` | default | Any key/paste the user sends scrolls the view to the bottom. |
| `scrollOnEraseInDisplay` | `false` | default | `CSI 2 J` clears the screen in place; it does not push the screen into scrollback. |
| `drawBoldTextInBrightColors` | `true` | default | Bold text whose foreground is ANSI 0-7 is drawn with ANSI 8-15. |
| `minimumContrastRatio` | `1` | default | No contrast adjustment of any colour. |
| `customGlyphs` | `true` | default | Box drawing, block elements, shades and powerline glyphs are drawn as geometry filling the whole cell (TERM-080). |
| `rescaleOverlappingGlyphs` | `false` | default | Wide glyphs from the font are not squeezed. |
| `allowTransparency` | `false` | default | Opaque background. |
| `tabStopWidth` | `8` | default | Tab stops every 8 columns. |
| `wordSeparator` | the characters space, `(`, `)`, `[`, `]`, `{`, `}`, `'`, `,`, `"` and backtick | default | Double-click word boundaries. |
| `convertEol` | `false` | default | `\n` is a pure line feed (no implied CR). |
| `windowsMode` / `windowsPty` | `false` / `{}` | default | No ConPTY reflow heuristics on Windows (wrapped-line flags come only from the data). |
| `windowOptions` | `{}` | default | All `CSI … t` window manipulation and window reports disabled: no replies to `CSI 14 t`, `CSI 18 t`, title stack etc. |
| `screenReaderMode` | `false` | default | No accessibility tree. |
| `ignoreBracketedPasteMode` | `false` | default | Paste honours mode 2004. |
| `reflowCursorLine` | `false` | default | On resize the cursor's own line is not reflowed. |
| `termName` | `'xterm'` | default | Only used by xterm internally; `TERM=xterm-256color`, `COLORTERM=truecolor` come from the session env (spec 02/01). |
| `linkHandler` | `null` | default | OSC 8 hyperlinks use xterm's built-in confirm prompt (TERM-068). |
| `logLevel` | `'info'` | default | — |
| `theme` | `TERMINAL_THEME` (below) | app | — |
| `cols`/`rows` | 80×24 until the first fit | default | The fit runs before `session:open`, so the pty is opened at the fitted size. |

Addons: `FitAddon` (fit), `Unicode11Addon` with `term.unicode.activeVersion = '11'`, `WebLinksAddon` with handler `(_event, uri) => window.open(uri, '_blank')`, `WebglAddon` (loaded after `open()`, in `try`; on WebGL context loss it is disposed and the DOM renderer takes over). The custom wheel handler is `ev => !ev.defaultPrevented`. `unscalePointer` patches `_core._mouseService.getCoords` and `getMouseReportCoords` (TERM-083).

### Font and cell metrics (what the WebGL renderer computes)

From `addon-webgl/src/WebglRenderer.ts:558-608` and `CharSizeService.ts` (measures the advance of `W` and `fontBoundingBoxAscent + fontBoundingBoxDescent` at 13 px):

- device char width = `floor(advanceW × dpr)`; device char height = `ceil((ascent + descent) × dpr)`
- device cell height = `floor(deviceCharHeight × 1.2)`; device cell width = device char width (+ `round(letterSpacing)` = 0)
- glyph top offset inside the cell = `round((deviceCellHeight − deviceCharHeight) / 2)` (vertically centred); left offset = `floor(letterSpacing/2)` = 0
- CSS cell size = device cell size / dpr; the grid's CSS size = `round(cols × deviceCellW / dpr)` × `round(rows × deviceCellH / dpr)`
- Computed for SF Mono (unitsPerEm 2048, advance 1266, hhea ascender 1980, descender −432, line gap 0): advance 8.036 px, ascent+descent 15.31 px. At dpr 2: char 16×31 device px, cell 16×37 device px = **8 × 18.5 CSS px**, glyph top offset 3 device px. At dpr 1: char 8×16, cell 8×19 CSS px. Verify against the running app (these are derived from the font file, not measured in Chromium).

### Theme — light AND dark

`--term-bg`/`--term-fg` are defined once in `:root` and **not** overridden in the dark block (`tokens.css:86-88`), and `TERMINAL_THEME` is a constant (`registry.ts:51-77`). The terminal therefore looks identical in the light and the dark app theme; the table is the palette for both.

| Slot | Light theme | Dark theme | RGB |
| --- | --- | --- | --- |
| background | `#1e1e20` | `#1e1e20` | 30, 30, 32 |
| foreground | `#e6e6e6` | `#e6e6e6` | 230, 230, 230 |
| cursor | `#e6e6e6` | `#e6e6e6` | 230, 230, 230 |
| cursorAccent (glyph under a block cursor) | `#1e1e20` | `#1e1e20` | 30, 30, 32 |
| selectionBackground (focused) | `#0a84ff55` | `#0a84ff55` | 10, 132, 255 @ α 0x55 = 0.333 |
| selectionInactiveBackground (unfocused) | `#8e8e9340` | `#8e8e9340` | 142, 142, 147 @ α 0x40 = 0.251 |
| selectionForeground | unset | unset | text keeps its own colour under a selection |
| scrollbarSliderBackground | `#ffffff1f` | `#ffffff1f` | white @ 0.122 |
| scrollbarSliderHoverBackground | `#ffffff33` | `#ffffff33` | white @ 0.2 |
| scrollbarSliderActiveBackground | `#ffffff47` | `#ffffff47` | white @ 0.278 |
| 0 black | `#3a3a3c` | `#3a3a3c` | 58, 58, 60 |
| 1 red | `#ff6b62` | `#ff6b62` | 255, 107, 98 |
| 2 green | `#5fd068` | `#5fd068` | 95, 208, 104 |
| 3 yellow | `#f5c451` | `#f5c451` | 245, 196, 81 |
| 4 blue | `#5aa9ff` | `#5aa9ff` | 90, 169, 255 |
| 5 magenta | `#d68cf7` | `#d68cf7` | 214, 140, 247 |
| 6 cyan | `#5ad1e0` | `#5ad1e0` | 90, 209, 224 |
| 7 white | `#d8d8dc` | `#d8d8dc` | 216, 216, 220 |
| 8 brightBlack | `#6e6e73` | `#6e6e73` | 110, 110, 115 |
| 9 brightRed | `#ff8a80` | `#ff8a80` | 255, 138, 128 |
| 10 brightGreen | `#7fe08a` | `#7fe08a` | 127, 224, 138 |
| 11 brightYellow | `#ffd978` | `#ffd978` | 255, 217, 120 |
| 12 brightBlue | `#80bdff` | `#80bdff` | 128, 189, 255 |
| 13 brightMagenta | `#e2a8fb` | `#e2a8fb` | 226, 168, 251 |
| 14 brightCyan | `#80e2ee` | `#80e2ee` | 128, 226, 238 |
| 15 brightWhite | `#f5f5f7` | `#f5f5f7` | 245, 245, 247 |

Derived values the renderer must reproduce:
- Indices 16-231 (xterm default cube, `browser/Types.ts:203-214`): index `16 + 36r + 6g + b` with each channel from `[0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff]`. Indices 232-255: grey `8 + 10i` for i = 0…23.
- Selection colours are already translucent (α < 1), so they are drawn as-is over the cell background (xterm would force α 0.3 only for opaque selection colours).
- Dim (SGR 2): foreground drawn at 50 % opacity (`DIM_OPACITY = 0.5`, `addon-webgl/src/Constants.ts:8`); e.g. default foreground dim ≈ `rgb(130, 130, 131)` on the background.
- Bold + ANSI 0-7 foreground → ANSI 8-15 (`drawBoldTextInBrightColors`), plus weight 700.
- Inverse (SGR 7): fg/bg swap; default fg on inverse uses the theme background and vice versa.
- Truecolor (SGR 38/48;2) and 256-colour (38/48;5) are drawn exactly; `minimumContrastRatio` 1 means no adjustment.
- Apps may change palette/default colours with OSC 4/10/11/12 and restore them with OSC 104/110/111/112 (xterm applies these; `CoreBrowserTerminal.ts:195-232`).

CSS around the terminal: the scroll container's background is forced to `var(--term-bg)` (`terminal.css:151-153`); `.term-host .xterm` fills its host (`height: 100%`). The rule `.term-host .xterm-viewport::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.16); background-clip: content-box }` (`terminal.css:154-157`) — unclear: in xterm 6 the scrollbar is the custom VS Code scrollable element, so this native-scrollbar rule most likely has no visible effect; the visible scrollbar uses the theme slider colours above.

### Addons and built-in renderer behaviours

**Fit** (`addon-fit/src/FitAddon.ts:35-96`): parent = the `.term-host` div (100 % of `.term-container`, which is 100 % of the padded body). `availableWidth = parseInt(parentWidth) − 0 padding − 14` (14 = `ViewportConstants.DEFAULT_SCROLL_BAR_WIDTH`, reserved because scrollback ≠ 0), `availableHeight = parseInt(parentHeight)`. `cols = max(2, floor(availableWidth / cssCellWidth))`, `rows = max(1, floor(availableHeight / cssCellHeight))`. Sizes are CSS layout sizes, i.e. **unaffected by the board zoom transform**: an expanded card has the same cols/rows at any zoom. No fit happens if the cell size is 0 (font not measured yet) or the parent isn't laid out (the try/catch in `fitEntry`). If the proposed size equals the current size nothing happens; otherwise the renderer is cleared and `term.resize(cols, rows)` fires `onResize`.

**Unicode 11** (`addon-unicode11/src/UnicodeV11.ts:195-232`): per-code-point widths from Unicode 11 tables: C0/C1 controls 0, combining marks and zero-width (incl. U+200B-U+200F, variation selectors) 0, East Asian Wide/Fullwidth and emoji-presentation code points of Unicode 11 width 2, everything else 1. A width-0 code point joins the preceding cell (and if the preceding cell was wider it keeps that width). There is no grapheme clustering: a ZWJ emoji sequence occupies one 2-wide cell per emoji; U+2764 U+FE0F stays width 1. Code points assigned after Unicode 11 are width 1.

**Web links** (`addon-web-links`): on hover, each buffer line (joined with its wrapped neighbours, stopping at a line containing a space or after 2048 chars) is matched against

```
/(https?|HTTPS?):[/]{2}[^\s"'!*(){}|\\\^<>`]*[^\s"':,.!?{}|\\\^~\[\]`()<>]/
```

and a match is kept only if `new URL(text)` parses and the text starts (case-insensitively) with `protocol//[user[:password]@]host` of the parsed URL. Matches can span wrapped lines. Activation calls the app handler `window.open(uri, '_blank')`; the main process's window-open handler opens `http(s):` URLs with `shell.openExternal` and denies everything else (`src/main/index.ts:74-77`). Links never open inside the app.

**Linkifier (built into xterm)**: hovering a link underlines it (1 device-px line at the bottom of each cell of the range, in the cell's 256-palette foreground if it has one, else the theme foreground) and sets the pointer cursor. A link activates on mouse-up when the mouse-down happened on the same link and the pointer is still inside it (`Linkifier.ts:216-232`; unclear: no button check, so a right-click on a link may open it too).

**OSC 8 hyperlinks** (`OscLinkProvider.ts`): cells carrying an OSC 8 URI form links (http/https only; others ignored), always underlined with a pointer. With `linkHandler: null` activation runs `confirm("Do you want to navigate to ${uri}?\n\nWARNING: This link could potentially be dangerous")`, then `window.open()` with no URL and assigns the location. unclear: in the 0.9.4 app the main process's window-open handler receives `about:blank`, denies it, and nothing opens (`OscLinkProvider.ts:117-131`, `src/main/index.ts:74-77`) — verify in the running app before porting; the parity target is whatever 0.9.4 does.

**WebGL renderer** (`addon-webgl`): GPU texture-atlas renderer; on `webglcontextlost` it waits 3000 ms for restoration, then fires context loss, the app disposes the addon and xterm falls back to its DOM renderer (same look). Cursor blink manager: 600 ms visible / 600 ms hidden while focused; every cursor move or output restarts the cycle with the cursor visible; blur pauses blinking with the cursor visible (as an outline). Custom glyphs (`CustomGlyphs.ts`): block elements U+2580-U+259F (incl. shades U+2591-U+2593 and eighth blocks), box drawing U+2500-U+257F (light, heavy, double, dashed, rounded corners, diagonals; heavy variants drawn heavier for bold), and powerline shapes U+E0A0-U+E0A3, U+E0B0-U+E0BF (triangles, semicircles, separators) are drawn as vectors clipped to the cell, so adjacent cells join with no gaps at line-height 1.2. Claude Code's logo and its boxed UI rely on this.

### Keyboard → bytes (xterm `evaluateKeyboardEvent`, `common/input/Keyboard.ts`)

Applied after the app's custom key handler (TERM-036…TERM-047) lets a key through. `mod` = 1 + (Shift 1 | Alt 2 | Ctrl 4 | Meta 8). "App cursor" = DECCKM (mode 1) on.

| Key | Sent |
| --- | --- |
| Backspace | `\x7f`; Ctrl+Backspace `\b`; with Alt prefixed by `\x1b` |
| Tab | `\t`; Shift+Tab `\x1b[Z` (focus never leaves the terminal) |
| Enter | `\r`; Alt+Enter (⌥Enter on a Mac) `\x1b\r` |
| Escape | `\x1b`; Alt+Escape `\x1b\x1b` |
| ← → ↑ ↓ | `\x1b[D` `\x1b[C` `\x1b[A` `\x1b[B`; app cursor `\x1bOD`…; with any modifier `\x1b[1;{mod}D` etc.; with Meta (⌘) nothing |
| Home / End | `\x1b[H` / `\x1b[F`; app cursor `\x1bOH` / `\x1bOF`; modified `\x1b[1;{mod}H` / `F` |
| Insert | `\x1b[2~` (nothing with Shift or Ctrl) |
| Delete | `\x1b[3~`; modified `\x1b[3;{mod}~` |
| Page Up / Down | `\x1b[5~` / `\x1b[6~`; Ctrl `\x1b[5;{mod}~` / `\x1b[6;{mod}~`; **Shift scrolls the view by rows−1 and sends nothing** |
| F1-F4 | `\x1bOP` `\x1bOQ` `\x1bOR` `\x1bOS`; modified `\x1b[1;{mod}P`… |
| F5-F12 | `\x1b[15~` `\x1b[17~` `\x1b[18~` `\x1b[19~` `\x1b[20~` `\x1b[21~` `\x1b[23~` `\x1b[24~`; modified `\x1b[{n};{mod}~` |
| Ctrl+A…Z (no Shift/Alt/Meta) | `\x01`…`\x1a` |
| Ctrl+Space | `\x00`; Ctrl+3…7 → `\x1b` `\x1c` `\x1d` `\x1e` `\x1f`; Ctrl+8 `\x7f`; Ctrl+[ `\x1b`; Ctrl+\ `\x1c`; Ctrl+] `\x1d`; Ctrl+_ `\x1f`; Ctrl+@ `\x00` |
| Alt+key off a Mac (no Meta) | `\x1b` + the key (digits/punctuation per US mapping, letters lower/upper by Shift, Ctrl+Alt+letter `\x1b` + control char, Alt+Space `\x1b `); dead keys with Alt send `\x1b` + the letter from `code` |
| ⌥+printable on a Mac | not Meta: the OS-composed character is typed (third-level shift; dead keys compose) |
| AltGr / Ctrl+Alt on Windows | third-level shift: the composed character is typed |
| Printable keys | the character (`ev.key`), via the input/keypress path; IME composition commits text |
| ⌘+anything on a Mac | never reaches this table (custom handler, TERM-038) |

### Mouse, selection and wheel (xterm behaviours)

- Selection (when no app mouse reporting): drag selects characters; double-click selects a word (separators above; if the pointer is on a link, the link's range); triple-click selects the (wrapped) line; Shift+click extends the selection; Alt/⌥+drag makes a column (block) selection; dragging past the top or bottom edge auto-scrolls every 50 ms, up to 15 lines per tick (threshold 50 px).
- Copy text (`SelectionService.ts:203-262`): each row's trailing whitespace trimmed, wrapped rows joined without a break, non-breaking spaces become spaces, rows joined with `\n` on macOS and `\r\n` on Windows. Column selection takes the same columns from each row.
- Any user input to the program (keys, paste) clears the selection; a resize that changes the row count clears it.
- Alt/⌥+click (mouse-up < 500 ms after down, selection ≤ 1 char, view at the bottom) moves the cursor by sending arrow-key sequences to the clicked cell (`SelectionService.ts:703-720`).
- Mouse reporting: when the program enables modes 9/1000/1002/1003 (encodings: default X10-style, 1006 SGR, 1016 SGR-pixels; 1005 UTF-8 is not supported), presses/releases/drags/moves/wheel are reported per `CoreMouseService` encodings and selection is disabled. Off a Mac, Shift+drag forces a local selection instead of reporting; on a Mac nothing does. Wheel reports: one report per wheel event whose accumulated line delta ≠ 0.
- Wheel, normal buffer, no reporting: VS Code scrollable element. Pixel-mode deltas scroll `50 × (−deltaY/40)` px ⇒ **1.25 × the native pixel delta**; line-mode deltas scroll 50 px per line; Alt multiplies by 5; scroll position is applied immediately (no smooth animation since duration 0) and converted to rows with `round(scrollTop / cssCellHeight)`. Off a Mac, Shift+wheel is turned into horizontal scrolling, which the terminal doesn't have, so it does nothing. A wheel event already `defaultPrevented` is ignored.
- Wheel, alternate buffer (no scrollback), no reporting: each wheel event with a non-zero accumulated line count sends exactly one `\x1b[A`/`\x1b[B` (or `\x1bOA`/`\x1bOB` in app cursor mode). Line accumulation: pixel deltas ÷ cell height, ×0.3 when |deltaY| < 50 (trackpad), ×5 with Alt/Ctrl/Shift; Shift+wheel yields 0.
- Focus reporting (mode 1004): focusing sends `\x1b[I`, blurring sends `\x1b[O`.
- Mouse pointer: I-beam over the terminal, default arrow while mouse reporting is on, pointer over a hovered link, default over the scrollbar.

### Paste and clipboard

- Paste (menu Paste / ⌘V on a Mac, Ctrl+V / Ctrl+Shift+V off a Mac, drops and `typeIn`) goes through `paste()`: `\r?\n` → `\r`, then wrapped in `\x1b[200~` … `\x1b[201~` when the program has mode 2004 on. Only `text/plain` is read: an image-only clipboard pastes an **empty** bracketed paste (`\x1b[200~\x1b[201~`), which Claude Code reacts to; this must be preserved.
- Copy goes to the system clipboard only by explicit command (TERM-040, TERM-043, menu Copy). There is no copy-on-select, no OSC 52 clipboard access (no clipboard addon), no Linux-style primary selection in shipped builds.
- No context menu: the app installs no `context-menu` handler, so right-click shows nothing (Mac: selects the word).

### Protocol features the terminal supports (must match)

xterm 6 parses the full xterm-compatible set Claude Code and shells use: CSI/OSC/DCS, SGR incl. underline styles (single, double, curly, dotted, dashed via `4:n`) and underline colour (SGR 58), overline (SGR 53), strikethrough, italic, invisible; alternate screen 47/1047/1049; DECCKM; DECTCEM cursor show/hide; DECSCUSR cursor shapes (block/underline/bar, blinking or steady); bracketed paste 2004; focus events 1004; mouse modes 9/1000/1002/1003, encodings 1006/1016; **synchronized output 2026** (Claude Code's Ink renderer uses it); DECRQM replies; DA1 reply `\x1b[?1;2c`; DA2 reply `\x1b[>0;276;0c`; DSR/CPR; OSC 4/10/11/12 queries answered as `\x1b]{n};rgb:rrrr/gggg/bbbb\x1b\` with the theme colour (each channel's two hex digits doubled, e.g. background `rgb:1e1e/1e1e/2020`); OSC 8 hyperlinks. Not supported (must stay unsupported to keep Claude Code's behaviour identical): kitty keyboard protocol (no reply to `CSI ? u`), OSC 52, window ops/reports, title changes (parsed, unused), bell (no sound, no visual).

Cursor visibility: the cursor is not drawn until the terminal has been focused once, received a key, or received DECTCEM (`CoreBrowserTerminal.ts:870-873`, `InputHandler.ts:1965`).

## Behaviors

### Registry and lifecycle (TERM)

- **TERM-001** Each session id has at most one terminal emulator for the life of the window, created on its first attach; it is shared by the session window and the expanded card.
- **TERM-002** A terminal is shown in one place at a time: attaching it to a container moves it there from wherever it was.
- **TERM-003** Detaching parks the terminal offscreen and keeps its screen, scrollback, cursor, modes and selection; the next attach shows exactly the screen it left.
- **TERM-004** Detach with a container argument does nothing unless the terminal is currently shown in that container (it may already have moved to the modal or the card).
- **TERM-005** Detaching blurs the terminal (it loses keyboard focus).
- **TERM-006** The parking spot is an invisible, non-interactive 1200×800 px box at x −10000; a parked terminal is never refitted, so it and its pty keep the last size it had.
- **TERM-007** Parked terminals keep receiving and parsing their session's output.
- **TERM-008** Output events are subscribed once per window for all sessions; output for a session with no terminal is ignored (the main process buffers it for the next open's replay).
- **TERM-009** Output for a terminal in state `opening` is dropped, because the pending open's replay already contains it (spec 02/03 must keep that ordering guarantee).
- **TERM-010** Output for a terminal in state `ready` is written to it; non-empty output sets `hasOutput` and notifies every output listener of that session.
- **TERM-011** Output for a terminal in state `exited` is dropped.
- **TERM-012** A process-exit event for a terminal in state `opening` is remembered (`exitPending`) and applied after the replay is written.
- **TERM-013** A process-exit event for a terminal in state `ready` writes the exit line for its kind and moves it to `exited`.
- **TERM-014** A process-exit event for an `exited` terminal or for a session without a terminal does nothing.
- **TERM-015** The Claude exit line is `\r\n\x1b[2m[process exited · press Resume or reopen the card to continue]\x1b[0m\r\n`; the shell exit line is `\r\n\x1b[2m[shell exited · press Restart or reopen the card for a new shell]\x1b[0m\r\n` (dim text on its own line).
- **TERM-016** Attaching a session with no terminal creates it inside the target container, fits it, sets state `opening` and calls `sessions.open(projectId, sessionId, cols, rows)` with the fitted size.
- **TERM-017** When the open resolves and the terminal was disposed meanwhile, nothing more happens.
- **TERM-018** When the open resolves, the replay is written, the state becomes `ready`, `session:resize` is sent with the terminal's current size, then either the pending exit is applied (exit line) or queued words are typed (TERM-029).
- **TERM-019** When the open rejects, the state becomes `exited`, the error reaches the caller, and the next attach tries again.
- **TERM-020** Attaching while an open is already in flight waits for that same open and starts no second process.
- **TERM-021** Attaching an `exited` terminal first fully resets it (screen, scrollback, modes, selection cleared; key handler kept), clears `hasOutput`, then opens the process again.
- **TERM-022** Attaching a `ready` terminal moves it, refits it, and sends `session:resize` with its current size; no open.
- **TERM-023** While shown in a container, the terminal refits whenever the container's size changes, at most once per animation frame.
- **TERM-024** A fit that changes cols/rows sends `session:resize(sessionId, cols, rows)` only in state `ready`.
- **TERM-025** Everything the terminal emits as input (keys, pastes, focus reports, query replies) is sent with `session:write` unless the state is `exited`; in `opening` it is sent (typing ahead during the loading pill reaches the process).
- **TERM-026** `needsOpen(id)` is true when there is no terminal or its state isn't `ready`; `hasOutput(id)` is true once any output has been written since creation or the last reset.
- **TERM-027** Disposing removes the session's queued words, its output listeners and the terminal; it happens when a card is removed, when archiving drops an empty session from the board, when a view (and its brain session) is removed, and when an action run finishes.
- **TERM-028** `typeIn(id, text, {starting})` pastes `text.trim() + ' '` at once if the terminal is `ready` and nothing is queued for it; otherwise it appends that to the session's queue, and the queue is `starting` if any call said so.
- **TERM-029** Queued words of a non-starting session are pasted right after the replay has been parsed.
- **TERM-030** Queued words of a starting session are pasted as soon as the program has bracketed paste on (immediately if it already has), or after 30 000 ms without it.
- **TERM-031** Queued words are pasted only if the same terminal still exists and is `ready`; otherwise they wait for the next successful open.
- **TERM-032** Dragging files over a terminal shows the copy drop effect; dropping them pastes their paths, each shell-quoted, joined by single spaces, with a trailing space, and focuses the terminal.
- **TERM-033** Shell quoting leaves a path unchanged if it matches `^[\w@%+=:,./-]+$`; otherwise it wraps it in single quotes with each `'` written as `'\''`.
- **TERM-034** A drop without files (text, links) is ignored by the terminal and cannot navigate the window.
- **TERM-035** A paste converts `\r\n`/`\n` to `\r` and is bracketed with `\x1b[200~`…`\x1b[201~` when mode 2004 is on; an empty text paste still sends the brackets.
- **TERM-036** In a Claude terminal, Shift+Enter (no ⌘/Ctrl/Alt) sends `\x1b\r` straight to the session on every keydown (incl. auto-repeat), and the keypress/keyup are swallowed so no `\r` is ever sent; this write bypasses the exited check, scroll-to-bottom and selection clearing.
- **TERM-037** In a shell terminal, Shift+Enter sends `\r` like Enter.
- **TERM-038** On a Mac, any key with ⌘ held and neither Ctrl nor ⌥ is never sent to the program; only keydown acts on it.
- **TERM-039** ⌘K (Mac) clears the terminal locally: the cursor's current line becomes the top line, all other lines and the whole scrollback are discarded, the view scrolls to the top, nothing is sent to the program; it does nothing when the cursor is already on the first line with no scrollback.
- **TERM-040** ⌘C (Mac) with a selection copies the selection text to the clipboard and keeps the selection; without a selection the menu's Copy runs and copies nothing.
- **TERM-041** ⌘A (Mac) selects the whole buffer, scrollback included.
- **TERM-042** Every other ⌘ key (⌘V, ⌘W, ⌘N, ⌘T, ⌘1-⌘9, ⌘, …) goes on to the app menu/window handlers; ⌘V pastes through the Edit menu (bracketed).
- **TERM-043** Off a Mac, Ctrl+C with a selection, and Ctrl+Shift+C always, copy the selection if there is one, then clear the selection and send nothing; Ctrl+C without a selection sends `\x03`.
- **TERM-044** Off a Mac, Ctrl+V and Ctrl+Shift+V paste the clipboard as a (bracketed) paste and send no control character.
- **TERM-045** Off a Mac, Ctrl+Shift+A selects all, Ctrl+Shift+W closes the top layer (menu command `close`), Ctrl+Shift+T opens a new terminal (menu command `new-terminal`); any other Ctrl+Shift+letter is not sent to the program and goes on to the app (e.g. Ctrl+Shift+N new project, Ctrl+Shift+F focus mode).
- **TERM-046** Off a Mac, the letter of a Ctrl shortcut is read from the key code when the layout's `key` isn't a Latin letter (Arabic, Russian, Greek), so Ctrl+C/V/Shift+A/W/T work on any layout.
- **TERM-047** Off a Mac, all other Ctrl keys (Ctrl+W, Ctrl+K, Ctrl+N, Ctrl+T, Ctrl+A, Ctrl+R…) and any Ctrl+Alt (AltGr) combination go to the program; the window-level Ctrl+N/T/W shortcuts ignore keys whose target is inside a terminal.
- **TERM-048** Escape sends `\x1b` to the program; it never closes the session window and never reaches the board's Escape handling.
- **TERM-049** Keys the terminal consumes don't reach the board or app shortcut handlers; while a terminal has focus the board's single-key shortcuts (N, T, V, /, E, ⌫, Enter, ⌘A, ⌘G, ⌘Z, ink keys) never fire.
- **TERM-050** Keys that pass the custom handler are encoded exactly as the key table above.
- **TERM-051** On a Mac, ⌥ with a printable key types the OS-composed character (dead keys compose); ⌥ with arrows, Backspace, Enter, Escape, Home/End, Delete, F-keys uses the modifier encodings.
- **TERM-052** Off a Mac, Alt+printable sends `\x1b` followed by the key.
- **TERM-053** Shift+Page Up / Shift+Page Down scroll the view by rows−1 and send nothing.
- **TERM-054** Any key or paste sent to the program while scrolled up scrolls the view to the bottom and clears the selection.
- **TERM-055** IME composition shows the composing text at the cursor (black background `#000`, white text `#FFF`, terminal font) and sends the committed text when composition ends; the candidate window is anchored at the cursor cell.
- **TERM-056** Mouse selection follows the xterm rules above (drag, double-click word/link, triple-click line, Shift+click extend, Alt/⌥+drag column, edge auto-scroll).
- **TERM-057** The selection is cleared by user input and by a resize that changes the row count.
- **TERM-058** The selection is drawn `#0a84ff55` while the terminal is focused and `#8e8e9340` while it isn't; text colours under it are unchanged.
- **TERM-059** Copied text trims trailing whitespace per row, joins wrapped rows, turns NBSP into spaces, and separates rows with `\n` on macOS and `\r\n` on Windows.
- **TERM-060** Selecting never copies by itself.
- **TERM-061** Right-click shows no menu; on a Mac it selects the word under the pointer unless the pointer is inside the current selection; off a Mac it does nothing.
- **TERM-062** Alt/⌥+click (released within 500 ms, ≤ 1 character selected, view at the bottom) sends the arrow keys that move the cursor to the clicked cell.
- **TERM-063** While the program has mouse reporting on, mouse events and wheel go to the program and local selection is off; off a Mac Shift+drag still selects locally; on a Mac nothing forces selection.
- **TERM-064** The pointer is an I-beam over the terminal (and over the expanded card's terminal body), an arrow while mouse reporting is on and over the scrollbar, and a hand over a hovered link.
- **TERM-065** URLs matching the web-links regex and the `new URL` check are links, including across wrapped lines.
- **TERM-066** Hovering a link underlines it and shows the hand pointer; leaving removes both.
- **TERM-067** Releasing the mouse on the same link it was pressed on opens the URL in the default browser (http/https only); it never opens inside the app.
- **TERM-068** OSC 8 hyperlinks with http/https URIs are links; activating one shows the confirm text `Do you want to navigate to {uri}?\n\nWARNING: This link could potentially be dangerous` (unclear what happens after OK in 0.9.4; see Terminal emulation).
- **TERM-069** Wheel over a terminal in the normal buffer scrolls its scrollback by 1.25 × the pixel delta (×5 with Alt), immediately, in whole rows.
- **TERM-070** Wheel over a terminal showing the alternate buffer without mouse reporting sends one cursor-up/down sequence per wheel event that accumulates at least one line.
- **TERM-071** Wheel with mouse reporting on is reported to the program as wheel button presses.
- **TERM-072** A wheel event something else already handled (default prevented) neither scrolls the terminal nor reaches the program.
- **TERM-073** While the view is at the bottom, new output keeps it at the bottom; while scrolled up, the view stays on the same content as output arrives (until scrollback trimming moves it).
- **TERM-074** Each terminal keeps 10 000 lines of scrollback.
- **TERM-075** The scrollbar is a 14 px overlay strip on the right, shown while scrolling or hovered and faded out 500 ms later (fade 800 ms linear; appear 100 ms linear), slider `#ffffff1f`, hover `#ffffff33`, dragging `#ffffff47`; it only appears when there is something to scroll.
- **TERM-076** The cursor is a block (`#e6e6e6`, glyph under it `#1e1e20`) blinking 600 ms on/600 ms off while focused, restarting visible on every cursor move; unfocused it is a steady 1 device-px outline; the program can change its shape/blink (DECSCUSR) or hide it (DECTCEM); it isn't drawn before the first focus, key or DECTCEM.
- **TERM-077** With mode 1004 on, focusing the terminal sends `\x1b[I` and blurring sends `\x1b[O`.
- **TERM-078** OSC 4/10/11/12 colour queries are answered with the current theme colour in `rgb:rrrr/gggg/bbbb` form terminated by `\x1b\`; DA1 is answered `\x1b[?1;2c` and DA2 `\x1b[>0;276;0c`.
- **TERM-079** Text attributes render as specified: bold = weight 700 and bright for ANSI 0-7, dim = 50 % opacity, italic, underline styles and colour, overline, strikethrough, inverse, invisible, 256-colour and truecolor.
- **TERM-080** Box drawing, block elements, shades and powerline glyphs are drawn as geometry that fills the cell exactly (no gaps between rows at line-height 1.2).
- **TERM-081** Character widths follow Unicode 11 (TERM table above).
- **TERM-082** Synchronized output (mode 2026) holds rendering until the program ends the update.
- **TERM-083** On a zoomed board, pointer positions are mapped back to the terminal's unscaled coordinates, so clicks, selections, link hovers and mouse reports land on the right cell at any zoom.
- **TERM-084** The terminal colours are the same in the light and the dark app theme.
- **TERM-085** `focus(id)` gives the terminal keyboard focus (without scrolling anything); it has no effect while the terminal is parked.

### Attach hook (TERM, continued)

- **TERM-086** A host's terminal phase starts as `ready` if the terminal already has output and doesn't need opening, else `loading`.
- **TERM-087** On each attach the phase becomes `loading` if the terminal needs opening or has no output yet, and the error is cleared.
- **TERM-088** The phase becomes `ready` on the first output written after the attach, or when the attach resolves and the terminal has output; an attach that resolves with no output stays `loading` until output arrives.
- **TERM-089** A rejected attach sets the phase to `error` with the error's message (or its string form).
- **TERM-090** With `focus` on (the session window), the terminal is focused right after the attach starts and again when it resolves; results arriving after the host unmounted or re-attached are ignored.
- **TERM-091** `reattach()` runs the attach again, which resumes an exited process (TERM-021).
- **TERM-092** When the host goes away or is disabled, the terminal is detached from that host's container only.

### Expanded card hosting (TERM, continued)

- **TERM-093** An expanded card starts its session's terminal right away if the session is live, its terminal is already `ready`, or it was asked to wake before rendering; otherwise (e.g. after an app restart with the session stopped) it waits for the user.
- **TERM-094** While not started the card body shows a blocking overlay with one button, `Resume session` (Play icon) for Claude or `Start shell` (SquareTerminal icon) for a shell; pressing it wakes the card.
- **TERM-095** Waking a card records the time, starts it if not started, or reattaches if its terminal needs opening and isn't loading, and focuses the terminal if that happens within 1500 ms of the wake.
- **TERM-096** Asking to wake a card that hasn't rendered expanded yet (`wakeCardTerminal`) queues the wake; the card starts and focuses when it renders.
- **TERM-097** Opening an expanded card (click/Enter on the board) wakes it instead of opening the session window; expanding cards wakes each of them.
- **TERM-098** A card whose session becomes live elsewhere starts showing its terminal and keeps showing it after the process exits.
- **TERM-099** The card hosts the terminal only while started, not shown in the session window, and a project is open; while the session window has it, the card body is empty terminal background with no overlay.
- **TERM-100** The card's terminal is never focused by attaching alone; pointer-down on its body focuses it (when hosted), and a plain click on the expanded card's title bar focuses it (not with Shift/⌘/Ctrl, not while renaming).
- **TERM-101** The card body shows the loading pill and the error overlay with the same texts as the session window; its `Try again` reattaches without changing the loading label.
- **TERM-102** The card's mic types the words into the session (`starting` when not live) and wakes the card.
- **TERM-103** Clicks and double-clicks inside the card body never reach the board (no open, no rename), and the body is not a drag handle for the card.
- **TERM-104** A wheel gesture is a run of wheel events less than 250 ms apart; a gesture whose first event lands on a hosted card terminal (without Ctrl) scrolls that terminal and does not pan the board for its whole duration.
- **TERM-105** A gesture that starts anywhere else pans the board for its whole duration, including when a card terminal slides under the pointer; that terminal neither scrolls nor receives wheel reports.
- **TERM-106** Ctrl+wheel (trackpad pinch) over a card terminal is not claimed by the card and goes to the board's zoom (unclear: xterm's scrollable element has no Ctrl check and may scroll the terminal and stop propagation when it has room to scroll, `ExpandedCard.tsx:113-134`, `scrollableElement.ts:390-485`; verify).
- **TERM-107** Wheel over a card body that isn't hosting its terminal (not started, or shown in the session window) pans the board.
- **TERM-108** Closing the session window leaves a non-archived card expanded on the board, where the terminal then shows (BoardView, spec 09/10).
- **TERM-109** On the board, ⌘K (Ctrl+K off a Mac) with a terminal focused does not open search: the terminal keeps it.

### Session window (MODAL)

- **MODAL-001** The session window is open while a terminal session id is set and a board is loaded; it shows one session and switching session rebuilds its content.
- **MODAL-002** Opening the window closes the transcript preview; opening the preview, navigating, or a notification's `showCard` closes the window.
- **MODAL-003** Escape never closes the window.
- **MODAL-004** Pointer-down on the dimmed backdrop closes the window.
- **MODAL-005** ⌘W (Mac) / Ctrl+Shift+W in a terminal / Ctrl+W outside one (Windows) closes the topmost layer in this order: Settings, agents viewer, session window, transcript preview, diff; with none open on a board it goes back to the projects view.
- **MODAL-006** Closing the window never stops the session or shell.
- **MODAL-007** The window attaches the terminal with focus.
- **MODAL-008** Pointer-down anywhere in the terminal area (incl. its padding) focuses the terminal.
- **MODAL-009** While loading, a non-blocking pill reads `Starting shell…` for a shell, `Starting Claude…` if the session was live when the window opened, else `Resuming session…`; typing still reaches the terminal under it.
- **MODAL-010** The loading label is decided when the window opens and changes to `Resuming session…` only when the user presses Resume/Try again (it doesn't flip when the session goes live mid-load).
- **MODAL-011** On error a blocking overlay shows `Couldn’t start the shell` / `Couldn’t start Claude`, the error text (selectable, monospace) if any, and a primary `Try again` button that resumes.
- **MODAL-012** The status area shows a SquareTerminal icon for a shell (brighter when live) or a status dot for Claude, and a label: shell `Running`/`Exited`; Claude `Working`, `Needs you`, `Idle` (live, done) or `Not running`; the label's tooltip is the same text; with no card it is empty.
- **MODAL-013** The label is orange for working, red for waiting, secondary grey otherwise; the dot uses the card status (`done` when unknown).
- **MODAL-014** The title is the card's user title, else its transcript title, else `Terminal` for a shell or `New session`.
- **MODAL-015** For a shell with a project folder, the subtitle shows a folder icon and the folder path with the home folder abbreviated to `~` (full path as tooltip, selectable).
- **MODAL-016** If the card has a git branch, the subtitle shows a branch icon and the branch name (monospace, max 280 px, ellipsis).
- **MODAL-017** For Claude, the subtitle shows the first 8 characters of the session id as a button (tooltip and label `Copy session ID`); clicking copies the full id and shows a check icon and `Copied` for 1400 ms.
- **MODAL-018** For Claude with a card, the subtitle shows the usage stats (model, context meter `Ctx N%`, running time, cost) as in spec 10's UsageStats; nothing until Claude has reported.
- **MODAL-019** For Claude, the header has the mic (window variant); spoken words are typed in (`starting` when not live), the session resumes if Resume is available, and the terminal is focused.
- **MODAL-020** When the card isn't live and the terminal isn't loading, the header shows `Restart` (RotateCcw, tooltip `Start a new shell in the project folder`) for a shell or `Resume` (Play) for Claude; pressing it reattaches and sets the loading label to resuming.
- **MODAL-021** `Stop` (Square icon, tooltip `Stop the session. The card stays, and Resume picks it up.`) shows only for a live Claude session that isn't an action run.
- **MODAL-022** Stop on a session between turns kills it immediately; on a working or waiting session it first asks with the confirm dialog (MODAL-023) and kills only on confirm.
- **MODAL-023** The confirm reads title `Stop a running session?`, message `“{title}” is still working. Stopping it cuts off this turn. The transcript stays, so you can resume it.` (or `is still waiting for you`), buttons `Cancel` and a danger `Stop`; Escape or Cancel keeps the session.
- **MODAL-024** After Stop the window stays open on the stopped session, with Resume in Stop's place once the card is no longer live.
- **MODAL-025** For a shell, `Remove` (Trash2, tooltip `Stop the shell and remove this terminal from the board`) closes the window and removes the card (which disposes the terminal; the main process stops the shell).
- **MODAL-026** For a Claude session that isn't a view's brain, `Archive` (Archive icon, tooltip `Stop and move to the archive`) archives the card and closes the window; a view's brain has neither Archive nor Remove.
- **MODAL-027** The close button (X) has tooltip `Close (⌘W). The session keeps running.` / `… The shell keeps running.`, with `Ctrl+Shift+W` instead of `⌘W` off a Mac, and aria-label `Close`.
- **MODAL-028** A board reload in which the open session's card became archived (e.g. `archive_self`) closes the window.
- **MODAL-029** When an action run finishes, its terminal is disposed and the window closes if it shows that run.
- **MODAL-030** Removing a view closes the window if it shows that view's brain.
- **MODAL-031** While the window is open, board keyboard shortcuts and ⌘K search don't fire; the menu's Search Sessions… closes the window (and the preview) and opens search.
- **MODAL-032** The window is `min(1400px, 92vw)` wide and `88vh` tall (clamped to 100vw−48 / 100vh−48).

### Transcript preview (TRANS)

- **TRANS-001** The preview is open while a preview session id is set and a board is loaded; switching session rebuilds it.
- **TRANS-002** Opening the preview records what search it came from (query + best snippet, or none) and closes the session window.
- **TRANS-003** Escape, backdrop pointer-down, ⌘W and the X button (tooltip `Close (Esc)`) close it.
- **TRANS-004** On open it reads the transcript once and shows a skeleton while loading.
- **TRANS-005** A failed read shows `Couldn’t read this transcript.`, the error (selectable, monospace) and a `Try again` button that reads it again.
- **TRANS-006** An empty transcript shows `No messages in this session.`
- **TRANS-007** The header shows the card title (TITLE rules as MODAL-014), the created–updated range, the message count (`1 message` / `N messages`, from the loaded messages, else the card's count, hidden when unknown) and the git branch (monospace).
- **TRANS-008** Opened from a search, the found message is scrolled to the vertical centre, aligned on its first highlighted word; otherwise the list is scrolled to the bottom (latest messages).
- **TRANS-009** The found message is the last message with a timestamp > 0 and ≤ the snippet's timestamp; without a snippet time (or none qualifies) it is the first message whose text contains any search term; none → no found message.
- **TRANS-010** Every message highlights the search terms (case-insensitive, overlapping matches merged; terms are the query's unique lower-cased words, longest first).
- **TRANS-011** The found message has a sunken background with an inset 2 px left bar in `--mark-strong` and starts expanded.
- **TRANS-012** If the snippet is a tool call, the found message also shows it: tool name (`Agent · {tool}` when an agent made it), the file it acted on if any, and its text with the snippet's match ranges highlighted, wrapping.
- **TRANS-013** Each message shows its author (`You` for user, `Claude` for assistant, unless an author is given) and, if its timestamp > 0, its time.
- **TRANS-014** Message text is trimmed; an empty text shows no text block; text is shown as plain pre-wrapped text (no markdown).
- **TRANS-015** A message longer than 12 lines or 1200 characters is collapsed to 12 lines with a fade and a `Show more` / `Show less` toggle.
- **TRANS-016** User messages sit in a sunken rounded box; assistant messages are plain with an orange author; notices (`data-notice`) are plain, secondary colour, 12 px.
- **TRANS-017** A message with `calls` lists them (TRANS-018); otherwise a message with tool names shows them as chips.
- **TRANS-018** A call list shows the last 8 calls, each a name chip and a one-line ellipsised detail, with `Show N earlier call` / `Show N earlier calls` above to reveal all (no way back).
- **TRANS-019** Times show `h:mm`-style local time for today and `{Mon D}, {time}` otherwise (`{Mon D, YYYY}` for another year).
- **TRANS-020** The range shows `{day}, {start time} – {end time}` when both are on the same day, else `{start day} – {end day}`.
- **TRANS-021** `Restore & Open` (primary, ArchiveRestore icon) unarchives the card at the first free slot beside the archive, closes the preview and opens the session window.
- **TRANS-022** The free slot is searched right of the archive panel at its largest (`stackZone(archive, STACK_ROWS × 2)`), filling columns top to bottom: slot i is at x = zone.x + zone.w + 80 + floor(i / rows) × 328 (288 + 40), y = zone.y + (i mod rows) × 256 (216 + 40), with rows = `max(3, floor(zone.h / 256))`, for i < rows × 40; the first slot whose 288×216 box, grown by 40 px, doesn't overlap any other active card (each also treated as 288×216 at its x, y) wins; if none is free it uses x = zone.x + zone.w + 80, y = zone.y + 24 × (number of other active cards).
- **TRANS-023** Message text, tool details and errors are selectable; the rest of the window isn't.
- **TRANS-024** ⌘K (Ctrl+K off a Mac) in the preview closes it and opens search (back to the results).

### Agents viewer (AGV)

- **AGV-001** The viewer is open while an agent view is set and a board is loaded; it is rebuilt only when the session changes, not when another agent is picked.
- **AGV-002** Opening it removes keyboard focus from whatever had it, so no key reaches a terminal.
- **AGV-003** Escape, backdrop pointer-down, ⌘W (first after Settings) and the X button (tooltip `Close (Esc)`) close it.
- **AGV-004** The agent list is read on open, again whenever the card's agents change id/status/updatedAt, and every 1500 ms while any agent isn't done.
- **AGV-005** The list shows `Loading…` before the first result, the error text (selectable) if a read failed (keeping any list already shown), and `No agents yet.` for an empty list.
- **AGV-006** The list is newest first with each agent followed by the agents it started, indented `12 + min(depth, 3) × 14` px; agents whose parent isn't listed are top-level.
- **AGV-007** Each item shows a small status dot, the description (up to 2 lines) and `{agentType} · {time}`; the selected item is highlighted.
- **AGV-008** Clicking an item selects that agent.
- **AGV-009** The selected agent is the requested one if listed, else the most urgent agent still at work (waiting before working, then oldest), else the most recently started.
- **AGV-010** The header title is the selected agent's description, or `Agents` when none; the subtitle shows its status (dot + `Needs you`/`Working`/`Stopped`/`Done`), its type, `{start time} · {elapsed}`, `N tool call(s)` when > 0, and always the session's title (tooltip `The session that started these agents`).
- **AGV-011** Elapsed time counts up every second while the selected agent is working or waiting; for a done agent it is `updatedAt − startedAt`.
- **AGV-012** With no selected agent the detail shows `This session hasn’t started any agents.` once the list has loaded, and nothing before.
- **AGV-013** The selected agent's transcript is read when it is selected and again whenever its `updatedAt` or status changes; the skeleton shows only before the first read, later reads replace the content without flashing.
- **AGV-014** The transcript stays scrolled to the bottom as it updates unless the user has scrolled more than 48 px up from the bottom.
- **AGV-015** Authors in an agent transcript: the agent type for assistant messages, `Background task` for notices, `Task` for the first message, `Message` for later user messages.
- **AGV-016** A failed read shows `Couldn’t read this agent’s transcript.` and the error, with no retry button.
- **AGV-017** An empty transcript shows `Starting…` while the agent is at work, else `This agent left no transcript.`
- **AGV-018** While the agent isn't done, a live line under the transcript shows its dot and what it is doing: `{tool}: {text}` or `{tool}`; with no tool, `Waiting for you in the session’s terminal` when waiting, `Waiting for its background command to finish` / `Waiting for N background commands to finish` when it has background commands, otherwise `Working…`.
- **AGV-019** The live line is red-tinted while the agent waits.
- **AGV-020** Agent messages render with the transcript message component (TRANS-013…018), without search highlighting.

## UI details

All colours below are resolved; "L / D" = light / dark app theme. `--term-*` and the terminal palette are identical in both.

### Shared box (Modal, spec 08)
- Backdrop: fixed, inset 0, z-index 500, centred grid; `rgba(0,0,0,0.28)` L / `rgba(0,0,0,0.45)` D; backdrop blur 3 px; fade-in 220 ms `cubic-bezier(0.2,0.8,0.2,1)`.
- Panel: column flex, max-width `100vw − 48px`, max-height `100vh − 48px`, radius 18 px, background `#ffffff` L / `#2a2a2d` D, shadow `0 30px 90px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(0,0,0,0.2)` L / `0 30px 90px rgba(0,0,0,0.6), 0 0 0 0.5px rgba(255,255,255,0.14)` D, overflow clipped; rise-in 220 ms from opacity 0, translateY 8 px, scale 0.985.
- Buttons (`.btn`): height 28, padding 0 12, radius 6, border 0.5 px `rgba(0,0,0,0.18)` L / `rgba(255,255,255,0.18)` D, background panel colour, shadow `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)` L / `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)` D, 13 px weight 500, gap 6, hover `rgba(0,0,0,0.05)` / `rgba(255,255,255,0.07)`, pressed `rgba(0,0,0,0.09)` / `rgba(255,255,255,0.12)`, background transition 120 ms. Primary: `#007aff` / `#0a84ff`, hover `#0068d9` / `#3396ff`, white text, transparent border. Ghost: no background, border or shadow. Icon button: 28×28. Focus-visible ring 2 px `rgba(0,122,255,0.5)` / `rgba(10,132,255,0.55)`, offset 2.
- Fonts: UI `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI Variable Text', 'Segoe UI', 'Helvetica Neue', sans-serif`; display `… 'SF Pro Display', 'Segoe UI Variable Display' …`; mono as the terminal. Text sizes: xs 11, sm 12, md 13, lg 15.
- Text colours: primary `rgba(0,0,0,0.88)` / `rgba(255,255,255,0.9)`; secondary `rgba(0,0,0,0.56)` / `rgba(255,255,255,0.58)`; tertiary `rgba(0,0,0,0.36)` / `rgba(255,255,255,0.36)`. Separator `rgba(0,0,0,0.07)` / `rgba(255,255,255,0.07)`.
- Status dots: 10 px (7 px `is-small`); working `#ff9500` / `#ff9f0a` with a 3 px ring at 22 % breathing to 5 px at 10 % (1.6 s ease-in-out infinite; small: 2 px → 3.5 px); waiting `#ff3b30` / `#ff453a` pulsing a ring from 0 at 55 % to 8 px transparent (1.1 s ease-out infinite); done `#007aff` / `#0a84ff`.
- Reduced motion: every animation and transition runs in 0.01 ms.

### Session window
- Panel size `min(1400px, 92vw)` × `88vh`.
- Header: 48 px tall, flex row, centred, gap 14, padding 0 10 0 16, background panel colour, bottom border 0.5 px separator.
  - Status: gap 8. Dot 10 px, or lucide `SquareTerminal` 14 px stroke 2 in tertiary (secondary when live). Label 12 px weight 600, secondary; `#ff9500`/`#ff9f0a` when working, `#ff3b30`/`#ff453a` when waiting.
  - Heading: flex 1, min-width 0. Title: display font, 13 px, weight 600, one line, ellipsis. Subtitle: margin-top 1, 11 px tertiary, gap 10.
  - Cwd: inline-flex gap 4, mono, lucide `Folder` 12; path one line ellipsis.
  - Branch: inline-flex gap 4, mono, max-width 280, ellipsis, lucide `GitBranch` 12.
  - Session id: mono 11, padding 0 4, margin 0 −4, radius 4, no border/background, colour inherits; hover background `rgba(0,0,0,0.05)` / `rgba(255,255,255,0.07)` and secondary text; copied state = lucide `Check` 11 + ` Copied` (gap 3).
  - Usage (spec 10 UsageStats): 11 px tertiary, gap 10, tabular numbers; model in secondary 500; context meter 22×4 radius 2 track `--border`, fill currentColor; `≥75 %` orange, `≥90 %` red.
  - Actions: gap 6. Mic (spec 05). Resume: `Play` 13 + `Resume`; Restart: `RotateCcw` 13 + `Restart`; Stop: `Square` 12 stroke 2.4 + `Stop`; Remove: `Trash2` 13 + `Remove`; Archive: `Archive` 13 + `Archive`; divider 1×18 px separator colour, margin 0 2; close: ghost icon button with `X` 16.
- Body: flex 1, padding 12 6 10 14 (top right bottom left), background `#1e1e20`; inner container 100 %×100 %; terminal fills it.
- Loading overlay: covers the body, centred, passes pointer events through. Pill: inline-flex gap 8, height 30, padding 0 14, radius 999, background `rgba(255,255,255,0.07)`, ring `0 0 0 0.5px rgba(255,255,255,0.1)`, text `rgba(255,255,255,0.72)` 12 px; lucide `LoaderCircle` 14 rotating 360° every 0.9 s linear; the pill fades in over 300 ms after a 150 ms delay (invisible for the first 150 ms).
- Error overlay: blocks pointer events, background `rgba(30,30,32,0.82)` (term bg at 82 %); column, centred, gap 10, max-width 460, padding 24, text `rgba(255,255,255,0.85)`; title 15 px weight 600; detail mono 12 px `rgba(255,255,255,0.55)`, pre-wrap, break-word.

### Expanded card terminal body (chrome in spec 10)
- Body: flex 1, padding 8 4 8 12 (top right bottom left), background `#1e1e20`, I-beam cursor; it is scaled with the board zoom.
- Not-started overlay: blocking (`rgba(30,30,32,0.82)`), centred `.btn` with `Play` 13 / `SquareTerminal` 13.
- Loading pill and error overlay exactly as the session window.

### Transcript preview
- Panel `min(760px, 92vw)` × `80vh`.
- Header: flex row, centred, gap 16, padding 16 12 14 22, bottom border 0.5 px separator. Title: display 15 px weight 600, one line ellipsis. Subtitle: wrap, gap 4 (rows) 14 (columns), margin-top 3, 12 px secondary; branch mono 11 px line-height 18 tertiary. Actions gap 6: primary `ArchiveRestore` 14 + `Restore & Open`; ghost close `X` 16.
- Body: scrolls vertically, padding 8 22 24. List: column gap 18, padding-top 12.
- Message meta: baseline row gap 8, margin-bottom 5, 11 px tertiary; author weight 600, letter-spacing 0.01em, secondary (assistant `#ff9500` / `#ff9f0a`).
- Content: 13 px, line-height 1.55. User box: padding 10 14, radius 10, background `rgba(0,0,0,0.035)` / `rgba(255,255,255,0.04)`, inset ring 0.5 px separator. Notice: no box, secondary, 12 px.
- Text: pre-wrap, break-word. Collapsed: max-height 1.55 em × 12 (≈ 241.8 px at 13 px), overflow hidden, mask fading from opaque at 70 % to transparent at the bottom.
- `Show more/less`, `Show N earlier calls`: no chrome, accent colour (`#007aff`/`#0a84ff`), 12 px weight 500, margin-top 4 (calls list: 0 0 2, left-aligned), underline on hover.
- Tool chips: wrap, gap 4, margin-top 6; chip padding 1 7, radius 5, sunken background, inset ring 0.5 px `rgba(0,0,0,0.1)` / `rgba(255,255,255,0.1)`, mono 11 px secondary.
- Calls: column gap 3, margin-top 6; row baseline gap 8, mono 11 px line-height 18; name chip padding 0 7 radius 5 sunken + ring, secondary; detail tertiary, one line ellipsis.
- Found message: margin −8 −12, padding 8 12, radius 10, sunken background, inset left bar 2 px `rgba(255,170,0,0.75)` / `rgba(255,214,10,0.7)`. Highlight marks: radius 3, background `rgba(255,196,0,0.32)` / `rgba(255,214,10,0.26)`, text colour unchanged. Found call: margin-top 8, mono 11 px line-height 18, detail wraps anywhere in secondary, file margin-left 6 tertiary.
- Empty/error: column centred gap 10, padding 80 24, secondary; detail max-width 520, mono 12 px tertiary, break-word.
- Skeleton (aria-label `Loading transcript`): column gap 22, padding-top 16; 5 rows (gap 8) each a short line 64×9 px and a 12 px line at 72 %, 94 %, 58 %, 86 %, 40 % width; lines radius 6 with a sunken→pressed→sunken gradient (200 % wide) shimmering from 100 % to −100 % over 1.3 s ease-in-out infinite.

### Agents viewer
- Panel `min(1080px, 94vw)` × `82vh`; header as the preview (title = description).
- Status in subtitle: inline-flex gap 6; orange when working, red when waiting. Session title: max-width 300, ellipsis, tertiary.
- Body: row. List: 272 px wide, padding 8, column gap 2, scrolls, right border 0.5 px separator, sunken background. Notes: padding 12, tertiary 12 px.
- Item: row aligned top, gap 9, full width, padding 7 12 (left grows with depth), radius 6, transparent; hover `rgba(0,0,0,0.05)` / `rgba(255,255,255,0.07)`; selected `rgba(0,122,255,0.14)` / `rgba(10,132,255,0.24)`; background transition 120 ms; dot 7 px with margin-top 5. Name 13 px primary, 2-line clamp, wrap anywhere; sub 11 px tertiary, one line ellipsis, gap 1 below the name.
- Detail: flex 1 column; its scroll area is the preview's body (padding 8 22 24).
- Live line: row centred gap 8, margin-top 18, padding 8 12, radius 10, sunken, mono 11 px secondary (red when waiting), text one line ellipsis.

### Verbatim copy (all strings)
`Starting shell…`, `Starting Claude…`, `Resuming session…`, `Couldn’t start the shell`, `Couldn’t start Claude`, `Try again`, `Running`, `Exited`, `Working`, `Needs you`, `Idle`, `Not running`, `Terminal`, `New session`, `Copy session ID`, `Copied`, `Restart`, `Start a new shell in the project folder`, `Resume`, `Stop`, `Stop the session. The card stays, and Resume picks it up.`, `Remove`, `Stop the shell and remove this terminal from the board`, `Archive`, `Stop and move to the archive`, `Close`, `Close (⌘W). The session keeps running.`, `Close (⌘W). The shell keeps running.`, `Close (Ctrl+Shift+W). The session keeps running.`, `Close (Ctrl+Shift+W). The shell keeps running.`, `Resume session`, `Start shell`, `Stop a running session?`, `“{title}” is still working. Stopping it cuts off this turn.`, `“{title}” is still waiting for you. Stopping it cuts off this turn.`, `The transcript stays, so you can resume it.`, `Cancel`, `Restore & Open`, `Close (Esc)`, `Couldn’t read this transcript.`, `No messages in this session.`, `message`, `messages`, `You`, `Claude`, `Show more`, `Show less`, `Show {n} earlier call`, `Show {n} earlier calls`, `Agent · {tool}`, `Loading transcript`, `Agents`, `The session that started these agents`, `Loading…`, `No agents yet.`, `This session hasn’t started any agents.`, `Couldn’t read this agent’s transcript.`, `Starting…`, `This agent left no transcript.`, `Task`, `Message`, `Background task`, `Working…`, `Waiting for you in the session’s terminal`, `Waiting for its background command to finish`, `Waiting for {n} background commands to finish`, `Stopped`, `Done`, `{n} tool call`, `{n} tool calls`, the two exit lines (TERM-015), the OSC 8 confirm (TERM-068). Curly quotes and apostrophes (’ “ ”) and the ellipsis character (…) are as written.

### Icons
lucide-react: `LoaderCircle`, `X`, `Play`, `RotateCcw`, `Square`, `SquareTerminal`, `Trash2`, `Archive`, `Folder`, `GitBranch`, `Check`, `ArchiveRestore` (sizes above).

## Keyboard & mouse

| Context | Input | Mac | Windows |
| --- | --- | --- | --- |
| Claude terminal | ⇧Enter | `\x1b\r` (newline in Claude's prompt) | same |
| Shell terminal | ⇧Enter | `\r` | same |
| Terminal | Enter | `\r` | same |
| Terminal | ⌥Enter / Alt+Enter | `\x1b\r` | `\x1b\r` |
| Terminal | Esc | `\x1b` (interrupts Claude); never closes the window | same |
| Terminal | ⌘K | clear (local) | — (Ctrl+K sends `\x0b`) |
| Terminal | ⌘C / Ctrl+C | copy if selection, else nothing | copy + clear selection if selection, else `\x03` |
| Terminal | Ctrl+Shift+C | nothing sent (xterm has no encoding for Ctrl+Shift+letter) | copy if selection, clear selection, never `\x03` |
| Terminal | ⌘V / Ctrl+V / Ctrl+Shift+V | paste (Edit menu) | paste (browser default) |
| Terminal | ⌘A / Ctrl+Shift+A | select all | select all (Ctrl+A sends `\x01`) |
| Terminal | ⌘W / Ctrl+Shift+W | close top layer (menu) | close top layer (menu command) |
| Terminal | ⌘T / Ctrl+Shift+T | new terminal | new terminal (Ctrl+T sends `\x14`) |
| Terminal | ⌘N | new session | Ctrl+N sends `\x0e` |
| Terminal | ⌘⇧N / Ctrl+Shift+N, ⌘⇧F / Ctrl+Shift+F, ⌘, / Ctrl+, | app (new project, focus mode, settings) | app (registered accelerators) |
| Terminal | ⌘1-⌘9 | switch project (spec 08) | Ctrl+digit goes to the terminal |
| Terminal | ⌥⌘Q | quit and stop sessions (menu) | — |
| Terminal | other ⌘ keys (⌘←, ⌘⌫, ⌘Z…) | nothing sent; menu if any | — |
| Terminal | ⇧PgUp / ⇧PgDn | scroll rows−1 | same |
| Terminal | Tab / ⇧Tab | `\t` / `\x1b[Z` | same |
| Terminal | ⌥ + letter | composed character | Alt+letter: `\x1b` + letter |
| Terminal | IME | composition at cursor | same |
| Terminal | drag | select | select |
| Terminal | double / triple click | word (or link) / line | same |
| Terminal | ⇧-click | extend selection | same |
| Terminal | ⌥-drag / Alt-drag | column selection | same |
| Terminal | ⌥-click / Alt-click | move cursor to cell | same |
| Terminal | right-click | select word, no menu | nothing |
| Terminal | click on link | open in browser | same |
| Terminal | wheel / two-finger scroll | scroll scrollback ×1.25 px; alt buffer → arrows; reporting → program | same (Shift+wheel does nothing) |
| Terminal | ⌥/Alt + wheel | ×5 | same |
| Terminal, mouse reporting on | ⇧-drag | reported to program | local selection |
| Terminal | drop files | paste quoted paths + space | same |
| Session window | Esc | nothing (goes to terminal) | same |
| Session window | click backdrop | close | same |
| Session window | pointer-down in body | focus terminal | same |
| Session window | ⌘K | not search | — |
| Transcript preview | Esc / ⌘W / backdrop | close | Esc / Ctrl+W / backdrop |
| Transcript preview | ⌘K | close preview, open search | Ctrl+K |
| Agents viewer | Esc / ⌘W / backdrop | close | Esc / Ctrl+W / backdrop |
| Agents viewer | click item | select agent | same |
| Expanded card body | pointer-down | focus terminal (if hosted) | same |
| Expanded card title bar | click (no modifiers) | focus terminal | same |
| Expanded card body | wheel gesture starting here | scroll terminal, board doesn't pan | same |
| Expanded card body | wheel gesture started elsewhere | board pans, terminal ignores | same |
| Expanded card body | pinch / Ctrl+wheel | board zoom | Ctrl+wheel: board zoom |

Note on the Ctrl+Shift+C row on a Mac: the custom handler doesn't touch it and `evaluateKeyboardEvent` has no encoding for Ctrl+Shift+letter, so nothing is sent.

## Platform differences

- `isMac` is `navigator.platform` starting with `Mac`; Windows and Linux take the non-Mac paths (only Mac and Windows ship).
- Mac: ⌘ is the app's key and is never sent to the program; ⌘K/⌘C/⌘A act in the terminal; ⌥ composes characters (`macOptionIsMeta: false`); right-click selects a word; no modifier forces selection under mouse reporting; copied rows join with `\n`; close key shown as `⌘W`.
- Windows: Ctrl is the terminal's; Ctrl+C copies only with a selection; Ctrl+Shift+C always copies-or-nothing; Ctrl+V/Ctrl+Shift+V paste; Ctrl+Shift+A/W/T select all / close / new terminal; other Ctrl+Shift+letters go to the app; Ctrl keys are read by key code on non-Latin layouts; Alt is Meta (`\x1b` prefix); Ctrl+Alt/AltGr compose; Shift forces local selection under mouse reporting; Shift+wheel does nothing; copied rows join with `\r\n`; no terminal clear shortcut; close key shown as `Ctrl+Shift+W`; the window takes Ctrl+N/T/W only outside terminals.
- Fonts: SF Mono on Mac, Cascadia Mono (else Consolas) on Windows; cell size differs accordingly.
- Shells: `$SHELL -l` on Mac, PowerShell on Windows (spec 02/03); dropped Windows paths contain `\` and `:`, so they are single-quoted (`'C:\…\file.txt'`).
- Menu accelerators off a Mac are mostly unregistered (`registerAccelerator: false`) so Ctrl keys reach the terminal; Ctrl+Shift+N, Ctrl+Shift+F and Ctrl+, are registered and win over the terminal.

## Edge cases & errors (verbatim)

- Open fails → session window and card show `Couldn’t start Claude` / `Couldn’t start the shell` + the error message (e.g. the main process's `Error: …` text) + `Try again`; the terminal is left `exited` so the retry reopens.
- Process exits while the open is in flight → the replay is written, then the exit line; the terminal is `exited`; the session window shows Resume/Restart once the card isn't live.
- Process exits → exit line (TERM-015) appended under the last output; the next attach (reopen, Resume, Restart, wake) resets the screen and starts a fresh process; for Claude the resumed session redraws the whole conversation.
- A process started elsewhere (prompt bar send, capture) while the terminal is `exited`: its output is not shown until the next attach, which resets and replays.
- Output during `opening` isn't double-written (dropped, covered by the replay).
- React StrictMode double attach → one open (TERM-020); Rust must likewise make attach idempotent.
- Fit while the container isn't measurable → ignored, next resize refits.
- Shift+Enter in an exited Claude terminal is still written to the session (the write bypasses the exited guard; `registry.ts:196-209`); the main process decides what happens to it.
- Typing during `opening` is written to the session (`registry.ts:262-264`); unclear whether the main process/host keeps it if the process isn't spawned yet (`src/main/ipc.ts:916-922`, spec 02/03).
- Words spoken into a session that never turns on bracketed paste are pasted after 30 s anyway (possibly echoed).
- Words queued for a terminal that is disposed are dropped.
- ⌘W / Ctrl+Shift+W pressed in an expanded card's terminal with no window open returns to the projects view (`App.tsx:53-62`).
- Copy failures (`navigator.clipboard.writeText` rejected) are silently ignored, both for the selection and for the session id (no `Copied`).
- WebGL context lost and not restored within 3000 ms → DOM renderer (look unchanged); not applicable to GPUI.
- Chromium caps live WebGL contexts (~16); many terminals fall back to the DOM renderer. Not applicable to GPUI, but the Rust port must stay fast with dozens of terminals.
- Transcript read error → `Couldn’t read this transcript.` + message + `Try again`; agent transcript error → `Couldn’t read this agent’s transcript.` + message (no retry; the next status/update change re-reads).
- Agent list error after a successful read → error note above the stale list.
- The preview's `foundCall` appears only on the found message and only for snippets of kind `tool`.
- A `calls` array that is empty still replaces the tool chips (renders an empty calls block).
- Restore & Open with no board loaded does nothing.
- An agent id in the view that isn't in the list → the default agent is shown.
- Archived card reopened from the board opens the preview, not the session window (BoardView `openCard`).
- unclear: right-click on a link may open it (`Linkifier.ts:220-232`).
- unclear: OSC 8 link after confirm (TERM-068).
- unclear: whether `.xterm-viewport::-webkit-scrollbar-thumb` has any effect in xterm 6 (`terminal.css:154-157`).
- unclear: Ctrl+wheel over a card terminal with scroll room (TERM-106).

## Tests to port

Existing Vitest coverage touching this domain:
- `src/renderer/src/components/board/stopSessions.test.ts` → `nodus-ui` (or the crate that owns board stop logic, shared with spec 09/10): `stoppable` keeps live Claude sessions, drops shells and action runs, keeps view brains; `stopConfirmation` returns null between turns, names a single working/waiting session (`“Fix login” is still working`, `waiting for you`), asks once for several (`Stop 3 sessions?`, `Some of these sessions are still working`), names the lone mid-turn session among several and says `The transcripts stay`.

No Vitest exists for the registry, the modal, the preview or the viewer. New Rust tests required (made-up data only):
- `nodus-terminal` state machine: data dropped in opening/exited, written in ready; exit during opening → exit line after replay; exit line strings per kind; open failure → exited and retry reopens; attach during opening doesn't reopen; attach of exited resets and reopens; resize only in ready; input dropped only in exited.
- typeIn queue: immediate paste vs queue, concatenation, `starting` OR, trailing space and trim, wait for bracketed paste, 30 s limit (mock clock), drop when disposed.
- `shell_quote`: safe set unchanged; spaces and quotes quoted (`it's here` → `'it'\''s here'`).
- Key encoder: every row of the key table, incl. app cursor mode, modifiers, Mac ⌥ vs Windows Alt, Shift+PgUp scroll, custom rules (⇧Enter per kind, ⌘K/⌘C/⌘A, Windows Ctrl+C with/without selection, Ctrl+Shift+C/V/A/W/T, non-Latin key codes).
- Paste: newline normalisation, bracketing on/off, empty bracketed paste.
- Selection text: trailing-space trim, wrapped join, NBSP, `\n` vs `\r\n`, column mode.
- URL detection: regex + `isUrl` cases (trailing punctuation excluded, wrapped URL, `https://a.example/x).` → `https://a.example/x`).
- Fit math: cols/rows from logical size with the 14 px reservation and minimums 2×1.
- Wheel math: pixel → scroll px (1.25×), Alt ×5, alt-buffer one arrow per event, trackpad 0.3 factor for line accumulation.
- Colour query replies (`\x1b]11;rgb:1e1e/1e1e/2020\x1b\`), DA1/DA2 replies.
- Transcript helpers: `found_index`, collapse rule (12 lines / 1200 chars), `format_time`, `format_range`, `free_slot_beside_archive`, `ToolCalls` hidden count text.
- Agents helpers: `default_agent`, `nest_agents` (newest first, depth, orphan parents), `live_line`, `agent_step`, `agent_status_label`, `format_duration` (`42s`, `4m 12s`, `12m`, `1h 5m`), stick-to-bottom threshold.
- Wheel gesture arbitration (250 ms gap, first event decides) as a pure state machine.

## Rust mapping

**Crate split.** The plan says only `nodus-ui` knows GPUI, but describes `nodus-terminal` as a GPUI element. Proposal: `nodus-terminal` = GPUI-free core (`Term` wrapper, registry state machine, key/mouse/paste encoding, selection text, link detection, fit and wheel math, colour/theme tables) with a `gpui` cargo feature (or a `nodus-ui::terminal` module) for the element. Decide with the S1 spike.

**Core types.**
- `TerminalEntry { kind: SessionKind, term: Arc<FairMutex<Term<Listener>>>, parser: alacritty_terminal::vte::ansi::Processor<StdSyncHandler>, state: Opening|Ready|Exited, exit_pending, has_output, size: (cols, rows), output_subscribers, pending_text }`, held in a `TerminalRegistry` (GPUI `Entity`/global) keyed by session id for the whole window lifetime, independent of any view, so parked terminals keep parsing output (TERM-007).
- `Term` config: `scrolling_history: 10_000`, `semantic_escape_chars` = xterm's word separators, `kitty_keyboard: false`, `osc52: Disabled`, default cursor `Block` blinking.
- `Listener: EventListener`: `PtyWrite(s)` → `session_write` (query replies; DA replies differ from xterm's: alacritty answers DA1/DA2 itself — override by intercepting `CSI c`/`CSI > c` before the parser or accept the difference after checking Claude Code doesn't care; also suppress its `CSI 18 t` reply to match `windowOptions: {}`); `ColorRequest(i, fmt)` → reply using our palette in xterm's `rgb:rrrr/gggg/bbbb` + `ESC \` format; `ClipboardStore/Load` → ignore; `Title`/`ResetTitle`/`Bell` → ignore; `CursorBlinkingChange` → blink state; `Wakeup`/`MouseCursorDirty` → `cx.notify()` (coalesced to one repaint per frame); `TextAreaSizeRequest` → ignore.
- Ordering contract with spec 02/03: `open()` must return a replay that covers every `pty-data` event delivered before the reply, and events after it must be delivered after the reply (Electron gets this from in-order IPC). In Rust deliver both on the same ordered channel.

**Rendering (GPUI element).**
- Cell metrics from the font's advance and ascent+descent at 13 px with the xterm rounding (floor width, ceil char height, floor ×1.2, centred offset) at the window's scale factor; zoom-scaled for expanded cards (TERM-083). Decision needed: Electron scales the WebGL bitmap with the board's CSS transform (soft at zoom ≠ 1); GPUI will naturally render crisp glyphs at `13 × zoom`. Keep cols/rows computed from unscaled size either way.
- Per row: merge background runs into quads, then shape text runs with `TextRun`s (font weight/italic/colour), placing each run at `col × cell_w` so the grid never drifts (glyph advances from fallback fonts must be snapped to cells; wide chars take 2 cells).
- Custom glyphs: draw U+2500-U+259F and the powerline range as paths/quads in-cell (TERM-080); do not use the font for them.
- Colours: palette table above; bold→bright; dim α 0.5; inverse; underline styles (alacritty flags `UNDERLINE`, `DOUBLE_UNDERLINE`, `UNDERCURL`, `DOTTED_UNDERLINE`, `DASHED_UNDERLINE` + underline colour); strikethrough; overline is not tracked by alacritty_terminal — risk, low impact.
- Cursor: block/outline/bar/underline from `term.cursor_style()` and focus; 600 ms blink timer restarted on cursor movement/output; hidden when `SHOW_CURSOR` is off; not drawn before first focus/key/DECTCEM (track a flag).
- Selection: alacritty `Selection` (`Simple`, `Block` for Alt-drag, `Semantic` for double-click, `Lines` for triple-click), colours per focus; double-click on a hovered link selects the link range; selection text via `term.selection_to_string()` then post-process (trailing spaces, NBSP, `\r\n` on Windows) and compare against xterm's rules in tests.
- Scrollback: `term.scroll_display(Scroll::Delta(n))` with our pixel accumulator (1.25×, Alt ×5, round to rows); `display_offset` keeps content stable while scrolled up (alacritty does this natively); scroll to bottom on user input; Shift+PgUp/PgDn = rows−1.
- Scrollbar: our own 14 px overlay with the slider colours and fade timings.
- Links: on hover compute links for the hovered (wrapped) line with the ported regex + URL check; also `cell.hyperlink()` for OSC 8; underline + pointer; open via `nodus-platform::open_url` for http(s) only.
- Mouse reporting: port `CoreMouseService` encodings against `TermMode::{MOUSE_REPORT_CLICK, MOUSE_DRAG, MOUSE_MOTION, SGR_MOUSE, UTF8_MOUSE}`; X10 (mode 9) and SGR-pixels (1016) are not modes alacritty_terminal tracks — risk, Claude Code doesn't use them today.
- Keyboard: GPUI `KeyDownEvent` → app rules (TERM-036…047) → ported `evaluateKeyboardEvent` using `TermMode::APP_CURSOR`; printable text and IME through GPUI's `InputHandler` (`replace_text_in_range`, `replace_and_mark_text_in_range`, `bounds_for_range` = cursor cell) so dead keys, ⌥ composition and CJK IMEs work; marked text drawn at the cursor in `#000`/`#FFF`.
- Paste: `\r?\n → \r`, bracket when `TermMode::BRACKETED_PASTE`; read text from the GPUI clipboard; empty text still bracketed.
- Focus: GPUI `FocusHandle` per entry; focus events → `\x1b[I`/`\x1b[O` when `TermMode::FOCUS_IN_OUT`; parked terminal never holds focus.
- Drag and drop: GPUI `ExternalPaths` drop on the element → quote and paste.
- Unicode widths: alacritty uses `unicode-width` (current Unicode), xterm used Unicode 11. Emoji and symbols added after Unicode 11 become width 2 in Rust but were 1 in 0.9.4; Claude Code (Ink, `string-width`) computes its own widths. Options: vendor xterm's Unicode 11 tables into a custom width function (alacritty_terminal has no hook; would need a patched fork or pre-processing), or accept and test with Claude Code output. Flag in S1.
- Synchronized updates: use `Processor<StdSyncHandler>` (BSU/ESU 2026 with alacritty's timeout).
- Performance: parse pty bytes off the UI thread into the `Term` under `FairMutex`, notify once per frame; render only visible rows; damage tracking via `term.damage()`; 10 000-line history × many terminals (memory budget); avoid re-shaping unchanged rows (cache shaped lines keyed by row content).

**Views.** `SessionWindow`, `TranscriptPreview`, `AgentViewer` in `nodus-ui`, fed by `nodus-core` commands (`sessions.open/write/resize/kill/transcript/agents/agent_transcript`) and events (`pty_data`, `pty_exit`, `status`); agent polling 1500 ms with a GPUI timer; `Intl`-equivalent date formatting from the OS locale (`h:mm a` vs 24 h per locale).

**Risks.**
1. Key encoding and IME parity (we must re-implement xterm's encoder, ⌥ composition, dead keys, Windows Ctrl semantics) — everything Claude Code reads goes through it.
2. Unicode width differences (Unicode 11 vs current) misaligning Claude Code's UI.
3. Rendering parity: cell metrics, custom box/block glyphs, bold-bright, dim, cursor and selection visuals, crisp vs scaled text on a zoomed board.
4. Replay/live-data ordering across the new IPC (duplicated or lost output), and keeping parked terminals parsing.
5. Terminal identity differences (DA replies, kitty keyboard, window-op replies, colour queries) changing what Claude Code enables.

## Parity checklist

- [x] TERM-001 one terminal per session for the window's life, shared by window and card
- [x] TERM-002 shown in one place at a time
- [x] TERM-003 detach keeps screen/scrollback/modes/selection
- [x] TERM-004 detach with container only if still shown there
- [x] TERM-005 detach blurs
- [x] TERM-006 parked offscreen 1200×800, never refitted, keeps size
- [x] TERM-007 parked terminals keep parsing output
- [x] TERM-008 one subscription; output without a terminal ignored
- [x] TERM-009 output during opening dropped (replay covers it)
- [x] TERM-010 output in ready written; hasOutput; listeners notified
- [x] TERM-011 output in exited dropped
- [x] TERM-012 exit during opening remembered
- [x] TERM-013 exit in ready writes exit line, state exited
- [x] TERM-014 exit otherwise ignored
- [x] TERM-015 exit line strings per kind
- [x] TERM-016 new attach: create, fit, open with fitted size
- [x] TERM-017 disposed during open → nothing
- [x] TERM-018 replay, ready, resize, then exit line or queued words
- [x] TERM-019 open failure → exited, retry on next attach
- [x] TERM-020 attach during opening shares the open
- [x] TERM-021 attach of exited resets then reopens
- [x] TERM-022 attach of ready moves, fits, resizes
- [ ] TERM-023 refit on container resize, once per frame
- [x] TERM-024 resize sent only in ready
- [x] TERM-025 input written unless exited (type-ahead during opening)
- [x] TERM-026 needsOpen / hasOutput semantics
- [x] TERM-027 dispose triggers and cleanup
- [x] TERM-028 typeIn immediate vs queued, trim + space, starting OR
- [x] TERM-029 queued words after replay (not starting)
- [x] TERM-030 starting: on bracketed paste or after 30 s
- [x] TERM-031 queued words only into the same ready terminal
- [x] TERM-032 file drop pastes quoted paths + space, focuses
- [x] TERM-033 shell quoting rule
- [ ] TERM-034 drops without files ignored, no navigation
- [x] TERM-035 paste newline conversion and bracketing (empty too)
- [x] TERM-036 ⇧Enter in Claude → ESC CR, swallowed, bypasses guards
- [x] TERM-037 ⇧Enter in shell → CR
- [x] TERM-038 Mac ⌘ keys never sent to the program
- [x] TERM-039 ⌘K local clear semantics
- [x] TERM-040 ⌘C copies selection, keeps it
- [x] TERM-041 ⌘A select all
- [x] TERM-042 other ⌘ keys go to the app; ⌘V pastes via menu
- [x] TERM-043 Windows Ctrl+C / Ctrl+Shift+C rules
- [x] TERM-044 Windows Ctrl+V / Ctrl+Shift+V paste
- [x] TERM-045 Windows Ctrl+Shift+A/W/T and other Ctrl+Shift letters
- [x] TERM-046 Ctrl shortcut letters by key code on non-Latin layouts
- [x] TERM-047 other Windows Ctrl keys and AltGr go to the program
- [x] TERM-048 Esc → `\x1b`, never closes, never reaches board
- [ ] TERM-049 consumed keys don't reach board/app shortcuts
- [x] TERM-050 key encoding table
- [x] TERM-051 Mac ⌥ composes; ⌥ with special keys uses modifiers
- [x] TERM-052 Windows Alt+printable → ESC prefix
- [x] TERM-053 ⇧PgUp/⇧PgDn scroll rows−1
- [x] TERM-054 input scrolls to bottom and clears selection
- [ ] TERM-055 IME composition at cursor
- [ ] TERM-056 selection gestures
- [x] TERM-057 selection cleared on input and row-count resize
- [x] TERM-058 selection colours focused/unfocused
- [x] TERM-059 copied text format per platform
- [x] TERM-060 no copy on select
- [x] TERM-061 right-click: Mac selects word, no menu
- [x] TERM-062 Alt/⌥-click moves cursor
- [x] TERM-063 mouse reporting and forced selection rules
- [ ] TERM-064 pointer shapes
- [x] TERM-065 URL detection incl. wrapped lines
- [ ] TERM-066 link hover underline + hand
- [ ] TERM-067 link click opens http(s) in browser only
- [x] TERM-068 OSC 8 links and confirm text
- [x] TERM-069 wheel scroll math in normal buffer
- [x] TERM-070 wheel in alt buffer → one arrow per event
- [x] TERM-071 wheel reported with mouse reporting
- [x] TERM-072 prevented wheel ignored
- [x] TERM-073 follow output at bottom; stable when scrolled up
- [x] TERM-074 10 000 lines scrollback
- [ ] TERM-075 scrollbar size, colours, fade
- [ ] TERM-076 cursor shape, colours, blink, inactive outline, init
- [x] TERM-077 focus reporting
- [x] TERM-078 colour query and DA replies
- [x] TERM-079 text attributes
- [ ] TERM-080 custom box/block/powerline glyphs
- [x] TERM-081 Unicode 11 widths
- [x] TERM-082 synchronized output 2026
- [ ] TERM-083 pointer mapping at board zoom
- [x] TERM-084 terminal colours same in light and dark
- [x] TERM-085 focus(id) semantics
- [x] TERM-086 initial phase
- [x] TERM-087 phase loading on attach when needed
- [x] TERM-088 phase ready on output or resolved-with-output
- [x] TERM-089 phase error with message
- [ ] TERM-090 window focuses on attach and on resolve
- [x] TERM-091 reattach resumes
- [x] TERM-092 detach only from own container
- [ ] TERM-093 expanded card start rule
- [ ] TERM-094 not-started overlay buttons
- [ ] TERM-095 wake semantics and 1500 ms focus window
- [ ] TERM-096 wake queued before card renders
- [ ] TERM-097 open/expand wakes expanded cards
- [ ] TERM-098 card follows a session started elsewhere
- [ ] TERM-099 card hosts only when started and not in the window
- [ ] TERM-100 card focus rules (body pointer-down, title click)
- [ ] TERM-101 card loading/error overlays
- [ ] TERM-102 card mic types in and wakes
- [ ] TERM-103 card body clicks don't reach board, not a drag handle
- [x] TERM-104 wheel gesture starting on terminal scrolls it, board stays
- [x] TERM-105 gesture started elsewhere keeps panning board
- [x] TERM-106 Ctrl+wheel/pinch goes to board zoom
- [x] TERM-107 wheel over non-hosting card body pans board
- [ ] TERM-108 closing the window leaves the card expanded with the terminal
- [ ] TERM-109 board ⌘K yields to a focused terminal
- [ ] MODAL-001 open condition, one session, rebuild on switch
- [ ] MODAL-002 window/preview/navigation/showCard exclusivity
- [ ] MODAL-003 Esc never closes
- [ ] MODAL-004 backdrop closes
- [ ] MODAL-005 ⌘W / Ctrl+Shift+W close order
- [ ] MODAL-006 closing keeps the process running
- [ ] MODAL-007 attaches with focus
- [ ] MODAL-008 pointer-down in body focuses
- [ ] MODAL-009 loading pill texts, non-blocking
- [ ] MODAL-010 loading label fixed at open, resuming after Resume
- [ ] MODAL-011 error overlay + Try again
- [ ] MODAL-012 status icon/dot and label texts
- [ ] MODAL-013 status colours
- [ ] MODAL-014 title fallback
- [ ] MODAL-015 shell cwd with ~
- [ ] MODAL-016 branch
- [ ] MODAL-017 session id copy, Copied 1400 ms
- [ ] MODAL-018 usage stats
- [ ] MODAL-019 mic types in, resumes, focuses
- [ ] MODAL-020 Resume/Restart availability and action
- [ ] MODAL-021 Stop availability
- [ ] MODAL-022 Stop immediate vs confirmed
- [ ] MODAL-023 confirm dialog copy
- [ ] MODAL-024 window stays open after Stop
- [ ] MODAL-025 Remove for shells
- [ ] MODAL-026 Archive for Claude; none for view brains
- [ ] MODAL-027 close button tooltip per platform/kind
- [ ] MODAL-028 archived from outside closes window
- [ ] MODAL-029 finished action run disposes and closes
- [ ] MODAL-030 removed view closes its brain's window
- [ ] MODAL-031 board shortcuts/search suppressed; menu search closes window
- [ ] MODAL-032 window size
- [ ] TRANS-001 open condition, rebuild on switch
- [ ] TRANS-002 records search, closes session window
- [ ] TRANS-003 close by Esc/backdrop/⌘W/X
- [ ] TRANS-004 read once, skeleton
- [ ] TRANS-005 error + Try again
- [ ] TRANS-006 empty text
- [ ] TRANS-007 header title/range/count/branch
- [ ] TRANS-008 initial scroll (match centred or bottom)
- [ ] TRANS-009 found message rule
- [ ] TRANS-010 term highlighting
- [ ] TRANS-011 found message styling, expanded
- [ ] TRANS-012 found tool call line
- [ ] TRANS-013 author and time
- [ ] TRANS-014 trimmed plain text, no markdown
- [ ] TRANS-015 collapse 12 lines / 1200 chars
- [ ] TRANS-016 user box, assistant plain, notice quiet
- [ ] TRANS-017 calls vs tool chips
- [ ] TRANS-018 last 8 calls + Show N earlier
- [ ] TRANS-019 time format
- [ ] TRANS-020 range format
- [ ] TRANS-021 Restore & Open
- [ ] TRANS-022 free slot beside archive algorithm
- [ ] TRANS-023 selectable parts
- [ ] TRANS-024 ⌘K back to search
- [ ] AGV-001 open condition, rebuild per session only
- [ ] AGV-002 blur on open
- [ ] AGV-003 close by Esc/backdrop/⌘W/X
- [ ] AGV-004 list refresh triggers and 1500 ms poll
- [ ] AGV-005 list loading/error/empty
- [ ] AGV-006 nested newest-first list with indent
- [ ] AGV-007 item content and selection highlight
- [ ] AGV-008 click selects
- [ ] AGV-009 default selection rule
- [ ] AGV-010 header content
- [ ] AGV-011 elapsed ticking
- [ ] AGV-012 no-agent detail text
- [ ] AGV-013 transcript refresh triggers, no flash
- [ ] AGV-014 stick to bottom within 48 px
- [ ] AGV-015 transcript authors
- [ ] AGV-016 transcript error text
- [ ] AGV-017 empty transcript texts
- [ ] AGV-018 live line texts
- [ ] AGV-019 live line red while waiting
- [ ] AGV-020 messages via transcript component, no highlight
