# 08 · Projects grid, title bar, settings & design tokens

Sources:
- `src/renderer/src/styles/tokens.css`, `src/renderer/src/styles/global.css`, `src/renderer/src/styles/app.css`
- `src/renderer/src/App.tsx`, `src/renderer/src/main.tsx` (platform attribute only), `src/renderer/src/store.ts`, `src/renderer/src/platform.ts` (+ `platform.test.ts`)
- `src/renderer/src/components/TitleBar.tsx`, `TitleBar.css`, `TitleBarActions.tsx`, `RunningCount.tsx`, `runningSessions.ts` (+ `runningSessions.test.ts`), `UsageStats.tsx`, `UsageStats.css`, `UpdateButton.tsx`
- `src/renderer/src/components/SettingsDialog.tsx`, `SettingsDialog.css`, `Modal.tsx`, `Modal.css`, `ContextMenu.tsx`, `ContextMenu.css`, `ColorSwatches.tsx`, `ColorSwatches.css`, `SplitButton.tsx`, `SplitButton.css`, `FolderIcon.tsx`
- `src/renderer/src/views/ProjectsView.tsx`, `ProjectsView.css`
- `src/renderer/src/components/projects/FolderItem.tsx`, `GroupPicker.tsx`, `GroupPicker.css`, `ProjectRail.tsx`, `ProjectRail.css`, `projectsUi.ts`, `focus.ts`, `focus.test.ts`
- Context read (owned by others, cited where this spec depends on them): `src/main/index.ts` (window chrome colours), `src/main/ipc.ts` (`applySettings`, `addProject`, groups handlers), `src/main/store.ts` (`normalizeSettings`), `src/main/menu.ts`, `src/main/voice.ts` (key-save error copy), `src/shared/types.ts`, `docs/how-it-works.md` (Focus, Your data, Windows, Shortcuts), `docs/screenshots/*.png`

## Summary

This domain is the app's frame and its shared visual language:

- **Design tokens and global rules** (the authoritative colour/typography/shadow/motion table every other UI spec references), the shared `.btn`, `.switch` and `.status-dot` primitives, scrollbars, focus rings, cursors, text selection and reduced motion.
- **App shell**: the root layout (title bar over a main area), routing between the projects grid and a board, the app-level layer stack (terminal modal, transcript preview, agent viewer, diff modal, Settings) and its ⌘W close order, event subscriptions from the core, and the zustand store (`store.ts`) that holds all UI state and the board's optimistic mutations.
- **Title bar**: breadcrumbs, the running-sessions readout, the Focus switch, the update button slot, each view's own action buttons, the macOS traffic-light inset and the Windows ☰ menu + caption-button inset.
- **Settings dialog**: Appearance (theme tiles + pin sidebar), Focus, Floating capture (with the macOS Accessibility permission row), Notifications, Voice prompts (xAI key field) and Updates.
- **Shared widgets**: Modal, ContextMenu, ColorSwatches, SplitButton, FolderIcon (the macOS-style folder drawing), UsageStats.
- **Projects grid** (Finder-like): sections for ungrouped projects, one per group (collapsible, drag-to-move), Archived, focus-mode filtering, selection and Finder keyboard semantics, inline rename, context menus, remove confirmation, group picker, folder drops from the OS, toasts, empty state.
- **Project rail** on a board: Dock-like auto-hiding strip on the window's left edge (or pinned as a sidebar), ⌘1–⌘9 switching with held-⌘ number badges, hover hints, focus filtering.

The renderer never chooses the theme itself: the main process sets the OS-level appearance (`nativeTheme.themeSource = settings.theme`) and the CSS follows `prefers-color-scheme`. The Rust UI must resolve the effective appearance the same way (UI-001…UI-004) and pick the light or dark column of the token table below.

## Design tokens

All values are the resolved values per effective appearance. "Light" applies when the effective appearance is light, "Dark" when dark. Every component in every UI spec takes its colours from this table (component-local hard-coded colours are listed in **Hard-coded colours** below).

### Fonts

| Token | Value (CSS font stack, in fallback order) | Resolves to |
| --- | --- | --- |
| `--font-ui` | `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI Variable Text', 'Segoe UI', 'Helvetica Neue', sans-serif` | macOS: the system UI font (SF Pro; CoreText picks the optical size automatically). Windows: Segoe UI Variable Text, else Segoe UI. |
| `--font-display` | `-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI Variable Display', 'Segoe UI', 'Helvetica Neue', sans-serif` | macOS: the same system font as `--font-ui` (the first family wins; no visible difference on a Mac). Windows: Segoe UI Variable Display, else Segoe UI. |
| `--font-mono` | `'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace` | macOS: SF Mono. Windows: Cascadia Mono, else Consolas. |
| FolderIcon letter glyph | `'SF Pro Rounded', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif` | unclear: `FolderIcon.tsx:154`. The family name `SF Pro Rounded` exists only where Apple's SF fonts were installed by hand; on a stock Mac Chromium falls back to `-apple-system` (SF Pro). Port as the system font, Semibold, and check against a screenshot of 0.9.4 on a stock Mac. Windows: Segoe UI Semibold. |

Base text: `--font-ui`, 13px (`--text-md`), line-height 1.4 (18.2px), colour `--text-primary`, grayscale antialiasing (`-webkit-font-smoothing: antialiased`).

| Size token | px | Line height when inherited (×1.4) |
| --- | --- | --- |
| `--text-xs` | 11 | 15.4 |
| `--text-sm` | 12 | 16.8 |
| `--text-md` | 13 | 18.2 |
| `--text-lg` | 15 | 21 |
| `--text-xl` | 20 | 28 |
| `--text-2xl` | 26 | 36.4 |

Letter-spacing used in this domain: `-0.01em` (title bar project name 15px → −0.15px; group name 20px → −0.2px; empty-state title 20px → −0.2px) and `-0.02em` (Projects heading 26px → −0.52px). Numerals marked "tabular" use the `tnum` OpenType feature.

### Colour tokens

"Used by" names this domain's components first, then other domains' style files: board.css / BoardView.tsx = 09/10, terminal.css = 12, search.css · sidekick.css · ink.css = 11, capture.css = 07, git.css · actions.css · voice.css = 05, views.css = 04.

| Token | Light | Dark | Used by |
| --- | --- | --- | --- |
| `--bg-window` | `#f5f5f4` | `#1e1e20` | body background, projects grid background, pinned rail background, native window background (UI-003); actions.css, board.css, capture.css, views.css |
| `--bg-toolbar` | `rgba(246, 246, 245, 0.82)` | `rgba(38, 38, 41, 0.82)` | title bar background (over `--bg-window`, see Derived colours) |
| `--bg-canvas` | `#f3f3f1` | `#19191b` | board canvas (09); BoardView.tsx, board.css |
| `--canvas-dot` | `rgba(0, 0, 0, 0.13)` | `rgba(255, 255, 255, 0.11)` | board dot grid (09, BoardView.tsx) |
| `--bg-elevated` | `#ffffff` | `#2a2a2d` | `.btn`, modal panel, context menu (88%), rail (82%), rail key badge, rail hint, running-count card, folder badges, focus mark, rename inputs, toast, settings key input, swatch gap ring, theme-tile checked gap ring, rail status-dot border; also actions, board, capture, git, ink, sidekick, terminal, views, voice |
| `--bg-sunken` | `rgba(0, 0, 0, 0.035)` | `rgba(255, 255, 255, 0.04)` | group picker "New group name" input; board, capture, git, terminal, views |
| `--bg-hover` | `rgba(0, 0, 0, 0.05)` | `rgba(255, 255, 255, 0.07)` | `.btn:hover`, crumb link hover, group toggle hover, group picker row hover, rail item hover; actions, board, capture, git, ink, search, sidekick, terminal, voice |
| `--bg-pressed` | `rgba(0, 0, 0, 0.09)` | `rgba(255, 255, 255, 0.12)` | `.btn:active`, crumb link active, group toggle active, picker row active, rail item active, switch track (off), selected folder icon box, open split-button chevron; board, git, ink, sidekick, terminal |
| `--bg-selected` | `rgba(0, 122, 255, 0.14)` | `rgba(10, 132, 255, 0.24)` | active rail item, current group-picker row; capture, ink, search, sidekick, terminal |
| `--text-primary` | `rgba(0, 0, 0, 0.88)` | `rgba(255, 255, 255, 0.9)` | body text and every control that inherits it; hover colour of secondary controls |
| `--text-secondary` | `rgba(0, 0, 0, 0.56)` | `rgba(255, 255, 255, 0.58)` | crumb link, back/menu buttons, Focus label, running count, settings hints, theme names, archived heading, empty-state paragraph, dimmed folder labels, rail add button, rail hint detail, usage model, split-button chevron, picker row icons, remove-dialog text |
| `--text-tertiary` | `rgba(0, 0, 0, 0.36)` | `rgba(255, 255, 255, 0.36)` | crumb chevron, counts next to headings, folder subtitles, group chevron, empty placeholders, focus notes, context-menu hints, input placeholders (settings), usage stats, remove-dialog path, "or drop a folder…" hint |
| `--text-on-accent` | `#ffffff` | `#ffffff` (not redefined) | `.btn-primary` text, hovered context-menu item, selected folder label, danger button text |
| `--border` | `rgba(0, 0, 0, 0.1)` | `rgba(255, 255, 255, 0.1)` | title bar bottom hairline, switch track ring (off), usage meter track; board, capture, git, terminal |
| `--border-strong` | `rgba(0, 0, 0, 0.18)` | `rgba(255, 255, 255, 0.18)` | `.btn` border, scrollbar thumb, dashed group placeholders, settings input border, picker input border, rail separators, rail key badge border, theme preview ring, current-swatch outer ring; actions, board, capture, search, views |
| `--separator` | `rgba(0, 0, 0, 0.07)` | `rgba(255, 255, 255, 0.07)` | settings header hairline, context-menu separators, archived/focus-note top rules, pinned-rail right hairline; many others |
| `--accent` | `#007aff` | `#0a84ff` | `.btn-primary`, switch (on), context-menu item hover/focus, rename rings, settings links, selected folder label, active-rail pill, focus mark, picker check, picker input focus border, theme tile checked ring, drop overlay border, drop-target ring (55%) |
| `--accent-hover` | `#0068d9` | `#3396ff` | `.btn-primary:hover`; capture, search, sidekick |
| `--focus-ring` | `rgba(0, 122, 255, 0.5)` | `rgba(10, 132, 255, 0.55)` | keyboard focus outline everywhere, rename-field outer ring, picker input focus ring, settings input focus outline |
| `--status-working` | `#ff9500` | `#ff9f0a` | working status dots, usage context ≥75%, mini-window working dot |
| `--status-done` | `#007aff` | `#0a84ff` | done status dots, mini-window done dot |
| `--status-waiting` | `#ff3b30` | `#ff453a` | waiting status dots, danger menu items, danger button, usage context ≥90%, settings error text |
| `--mark` | `rgba(255, 196, 0, 0.32)` | `rgba(255, 214, 10, 0.26)` | search highlights (11, 12) |
| `--mark-strong` | `rgba(255, 170, 0, 0.75)` | `rgba(255, 214, 10, 0.7)` | current search match (11, 12) |
| `--diff-add` | `#2da44e` | `#3fb950` | git diff (05) |
| `--diff-del` | `#cf222e` | `#f85149` | git diff (05) |
| `--archive-bg` | `rgba(120, 120, 128, 0.07)` | `rgba(160, 160, 170, 0.06)` | archive zone (09/10) |
| `--archive-border` | `rgba(120, 120, 128, 0.35)` | `rgba(160, 160, 170, 0.3)` | archive zone (09/10) |
| `--archive-label` | `rgba(0, 0, 0, 0.42)` | `rgba(255, 255, 255, 0.42)` | archive zone (09/10) |
| `--archive-drop` | `rgba(0, 122, 255, 0.08)` | `rgba(10, 132, 255, 0.12)` | projects section drop target, folder drop overlay; archive zone (09/10) |
| `--archive-panel` | `rgba(248, 248, 247, 0.8)` | `rgba(38, 38, 41, 0.8)` | archive panel (09/10) |
| `--note-paper` | `#ffffff` | `#b4b4b8` | sticky notes (10), capture (07) |
| `--note-ink` | `rgba(0, 0, 0, 0.82)` | `rgba(0, 0, 0, 0.82)` (not redefined) | sticky notes (10), capture (07) |
| `--note-ink-faint` | `rgba(0, 0, 0, 0.38)` | `rgba(0, 0, 0, 0.38)` (not redefined) | sticky notes (10) |
| `--folder-label-selected` | `#0063e1` | `#0a84ff` | sidekick (11) only |
| `--ink-graphite` | `#1d1d1f` | `#f2f2f4` | board ink (11, InkLayer.tsx via `var(--ink-<color>)`) |
| `--ink-red` | `#ff3b30` | `#ff453a` | board ink (11) |
| `--ink-orange` | `#ff9500` | `#ff9f0a` | board ink (11) |
| `--ink-green` | `#28cd41` | `#32d74b` | board ink (11) |
| `--ink-blue` | `#007aff` | `#0a84ff` | board ink (11) |
| `--ink-purple` | `#af52de` | `#bf5af2` | board ink (11) |
| `--term-bg` | `#1e1e20` | `#1e1e20` (not redefined) | terminal surfaces (12), board.css |
| `--term-fg` | `#e6e6e6` | `#e6e6e6` (not redefined) | no `var()` use found; the xterm theme hard-codes the same value (`terminal/registry.ts:53`, 12) |

### Shadow tokens (layer lists, outermost first as written)

| Token | Light | Dark |
| --- | --- | --- |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.06)`, `0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3)`, `0 0 0 0.5px rgba(255,255,255,0.08)` |
| `--shadow-md` | `0 4px 14px rgba(0,0,0,0.08)`, `0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35)`, `0 0 0 0.5px rgba(255,255,255,0.1)` |
| `--shadow-lg` | `0 18px 50px rgba(0,0,0,0.18)`, `0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5)`, `0 0 0 0.5px rgba(255,255,255,0.12)` |
| `--shadow-modal` | `0 30px 90px rgba(0,0,0,0.35)`, `0 0 0 0.5px rgba(0,0,0,0.2)` | `0 30px 90px rgba(0,0,0,0.6)`, `0 0 0 0.5px rgba(255,255,255,0.14)` |
| `--note-shadow` | `0 1px 1.5px rgba(0,0,0,0.08)`, `0 8px 16px -8px rgba(0,0,0,0.3)` | `0 1px 1.5px rgba(0,0,0,0.3)`, `0 8px 18px -8px rgba(0,0,0,0.6)` |
| `--note-shadow-lifted` | `0 2px 4px rgba(0,0,0,0.08)`, `0 20px 32px -12px rgba(0,0,0,0.36)` | `0 2px 4px rgba(0,0,0,0.3)`, `0 20px 34px -12px rgba(0,0,0,0.7)` |

Shadow syntax is CSS `x y blur spread colour`; blur is the CSS blur radius (Gaussian σ = blur / 2). The `0 0 0 0.5px` layers are hairline rings drawn outside the box, following its corner radius.

### Shape, motion and layout tokens (same in both themes)

| Token | Value | Used by |
| --- | --- | --- |
| `--radius-sm` | 6px | `.btn`, crumb link, group toggle, picker rows/input, settings input, running card, rail hint, split button, group rename |
| `--radius-md` | 10px | context menu, folder item + icon box, group placeholder, toast, rail items |
| `--radius-lg` | 14px | projects sections, rail (floating) |
| `--radius-xl` | 18px | modal panel, folder drop overlay |
| `--ease-out` | `cubic-bezier(0.2, 0.8, 0.2, 1)` | most enter animations and transforms |
| `--ease-spring` | `cubic-bezier(0.34, 1.4, 0.64, 1)` (overshoots) | new-folder pop; board/capture |
| `--ease-stack` | `linear(0, 0.05, 0.1697, 0.3218, 0.4803, 0.6281, 0.7553, 0.8579, 0.9355, 0.9902, 1.0254, 1.0452, 1.0534, 1.0536, 1.0487, 1.0411, 1.0326, 1.0242, 1.0166, 1.0104, 1.0055, 1.0019, 0.9995, 0.998, 0.9973, 0.9971, 0.9972, 0.9975, 1)` — 29 output values at equally spaced inputs 0, 1/28, …, 1, linearly interpolated between them | archive stack fly-out (10) |
| `--dur-fast` | 120ms | hovers, switches, small enters |
| `--dur-med` | 220ms | modal enter, toast enter, rail slide, folder icon lift |
| `--titlebar-h` | 52px | title bar height; Windows caption-button overlay height |
| `--rail-w` | 52px | project rail width; board inset when the rail is pinned; actions rail (05) |
| `--rail-inset` | 8px | floating rail's gap from the window edge; actions rail (05) |
| `--card-w` | 288px | session card width (10) |
| `--card-h` | 216px | session card height, 4:3 (10) |

CSS keyword easings used in this domain: `ease` = `cubic-bezier(0.25, 0.1, 0.25, 1)`; `ease-in-out` = `cubic-bezier(0.42, 0, 0.58, 1)`; `ease-out` (keyword, not the token) = `cubic-bezier(0, 0, 0.58, 1)`.

### Derived colours (resolved composites)

| What | Rule | Light | Dark |
| --- | --- | --- | --- |
| Title bar surface | `--bg-toolbar` over `--bg-window` (backdrop blur 24px saturate 1.6 has nothing but the window behind it) | `#f6f6f5` (245.8, 245.8, 244.8) | `#252527` (36.6, 36.6, 39.4) |
| Context menu surface | `--bg-elevated` at 88% alpha, backdrop blur 30px, saturate 1.8 | `rgba(255,255,255,0.88)` | `rgba(42,42,45,0.88)` |
| Floating rail surface | `--bg-elevated` at 82% alpha, backdrop blur 24px, saturate 1.8 | `rgba(255,255,255,0.82)` | `rgba(42,42,45,0.82)` |
| Section drop-target ring | `--accent` at 55% alpha | `rgba(0,122,255,0.55)` | `rgba(10,132,255,0.55)` |
| Working dot ring (rest) | `--status-working` at 22% | `rgba(255,149,0,0.22)` | `rgba(255,159,10,0.22)` |
| Working dot ring (breath peak) | `--status-working` at 10% | `rgba(255,149,0,0.10)` | `rgba(255,159,10,0.10)` |
| Waiting dot pulse (start) | `--status-waiting` at 55% | `rgba(255,59,48,0.55)` | `rgba(255,69,58,0.55)` |
| Danger button hover | `--status-waiting` 88% mixed with black | `#e0342a` (224.4, 51.9, 42.2) | `#e03d33` (224.4, 60.7, 51.0) |
| Modal backdrop | hard-coded per theme, plus backdrop blur 3px | `rgba(0,0,0,0.28)` | `rgba(0,0,0,0.45)` |

### Hard-coded colours (not tokens; same in both themes unless stated)

| Where | Value |
| --- | --- |
| Native window background | light `#f5f5f4`, dark `#1e1e20` (`src/main/index.ts:13`) |
| Windows caption-button overlay | light: background `#f6f6f5`, symbols `#1f1f1f`; dark: background `#252527`, symbols `#e6e6e6`; height 52 (`src/main/index.ts:20-23`) |
| Switch knob | fill `#ffffff`, shadow `0 1px 2px rgba(0,0,0,0.25)` |
| Swatch inner ring | inset `0 0 0 0.5px rgba(0,0,0,0.18)`; check mark `#fff` |
| Mini-window (Settings theme preview), light tone | canvas `#f3f3f1`, bar `#ebebea`, card `#ffffff`, border `rgba(0,0,0,0.09)`, line `rgba(0,0,0,0.12)`, dot grid `rgba(0,0,0,0.13)` |
| Mini-window, dark tone | canvas `#19191b`, bar `#262629`, card `#2a2a2d`, border `rgba(255,255,255,0.09)`, line `rgba(255,255,255,0.16)`, dot grid `rgba(255,255,255,0.11)` |
| Mini-window traffic lights | `#ff5f57`, `#febc2e`, `#28c840` |
| Mini-card shadow | `0 0 0 0.5px <mini border>`, `0 1px 2px rgba(0,0,0,0.08)` |
| Folder hues | see FolderIcon in UI details (identical in both themes) |

### Global rules

- **Box model**: every element uses border-box sizing (borders and padding are inside the stated width/height).
- **Root**: the window content is a column that fills the window; nothing scrolls at the root (`overflow: hidden`). Page/window title: `Nodus`.
- **Default text**: see Fonts. Buttons and inputs inherit the font, size and colour of their container unless stated.
- **Text selection**: off everywhere by default. On for text inputs/textareas and for elements explicitly marked selectable (in this domain: the path line in the remove dialog, which also shows the I-beam cursor). Selection highlight colour is not customised: the platform default (unclear: Chromium's default `::selection`, i.e. the system's selected-text background; no rule in `global.css`).
- **Cursor**: the arrow everywhere, including on buttons and links. Exceptions: switches show the pointing hand; selectable text and text inputs show the I-beam; disabled switches show the arrow.
- **Keyboard focus ring** (`:focus-visible`): a 2px solid `--focus-ring` outline drawn 2px outside the element's border box, with corners following the element's radius (+2px). Shown when focus arrives by keyboard (Tab/arrow keys) or on text inputs whenever focused; not after a mouse click on a button. Components override it where stated (context-menu items, rename fields, picker input, settings input, settings panel, theme tiles, folder items).
- **Scrollbars** (every scrollable area unless hidden): 10px wide (vertical) / 10px tall (horizontal), always visible while the content overflows and taking 10px of layout space (classic, not overlay). No track, no arrows. Thumb: `--border-strong`, a 4px-wide pill inset 3px from each side of the 10px gutter (outer radius 10px, the 3px transparent border is not painted). No hover/pressed change. Corner square: unclear (`global.css:206-215` styles no `::-webkit-scrollbar-corner`).
- **Reduced motion**: when the OS asks for reduced motion, every animation and transition in the app runs in 0.01ms (i.e. jumps to its end state; infinite animations effectively stop visibly).
- **Window drag**: the title bar's empty area drags the window; double-clicking it performs the OS title-bar action (zoom/maximize). Elements marked "no-drag" in this spec are excluded and receive clicks normally.
- **`color-scheme: light dark`**: native form controls follow the effective appearance (relevant only where a native control is still visible; this domain draws its own).

#### Shared button (`.btn`)

| Property | Value |
| --- | --- |
| Layout | inline row, items centred, gap 6px, height 28px, padding 0 12px, no wrap |
| Shape | radius 6px, border 0.5px `--border-strong` |
| Fill | `--bg-elevated`, shadow `--shadow-sm` |
| Text | 13px, weight 500, colour inherited (normally `--text-primary`) |
| Hover | background becomes `--bg-hover` — a translucent overlay **replacing** the opaque fill, so the button shows the surface behind it tinted (e.g. on the title bar, light ≈ `#f6f6f5` darkened 5%) |
| Pressed | background `--bg-pressed` (also translucent) |
| Transition | background 120ms `ease` |
| Disabled | opacity 0.45, ignores the pointer entirely (no hover, no tooltip, no clicks) |
| `.btn-primary` | fill `--accent`, border transparent, text `--text-on-accent`; hover fill `--accent-hover`; pressed while hovered stays `--accent-hover` (the hover rule wins); pressed after the pointer slid off shows `--bg-pressed` |
| `.btn-ghost` | fill transparent, border transparent, no shadow; hover `--bg-hover`, pressed `--bg-pressed` |
| `.btn-icon` | width 28px (square), padding 0, content centred |

#### Switch (`.switch`)

| Property | Value |
| --- | --- |
| Track | 32×18px, radius 9px, top margin 1px (optical alignment); off: `--bg-pressed` + inset 0.5px `--border` ring; on: `--accent`, no ring |
| Knob | 14×14px circle `#ffffff`, shadow `0 1px 2px rgba(0,0,0,0.25)`, at (2px, 2px); on: translated 14px right (x = 16px) |
| Motion | track colour 120ms `ease`; knob translate 120ms `ease` |
| Cursor | pointing hand; arrow when disabled |
| Focus | keyboard focus: 2px `--focus-ring` outline at 2px offset (radius 11px) |
| Disabled | opacity 0.5 |
| Semantics | a checkbox with role switch; Space toggles when focused |

#### Status dot (`.status-dot`, shared by folders, rail, cards, terminal header)

| Variant | Size | Fill | Ring / animation |
| --- | --- | --- | --- |
| working | 10×10 circle | `--status-working` | ring `0 0 0 3px` working@22%; animation "breathe": 1.6s, `ease-in-out` per half, infinite: at 50% the ring is `0 0 0 5px` working@10%, at 0%/100% it is the resting 3px@22% (spread and alpha interpolate linearly within each eased half) |
| done | 10×10 | `--status-done` | none |
| waiting | 10×10 | `--status-waiting` | animation "pulse": 1.1s `ease-out` (keyword: `cubic-bezier(0,0,0.58,1)`), infinite: ring from `0 0 0 0` waiting@55% to `0 0 0 8px` fully transparent, then restarts |
| small (`is-small`) | 7×7 | as status | working: resting ring 2px@22%, breathe peak 3.5px@10%, same timing; waiting: unchanged pulse |

#### Stacking order (back to front)

| Layer | z |
| --- | --- |
| Board content (09) | contained below everything here |
| Projects grid folder-drop overlay | 20 |
| Projects grid toast | 30 |
| Project rail (floating or pinned) | 40 |
| Rail reveal edge strip | 41 |
| Title bar (incl. running-count card) | 50 |
| Rail hover hint | 60 |
| Modal backdrop + panel (every Modal) | 500 |
| Project rail while ⌘ numbers show | 600 |
| Context menu | 1000 |

## Behaviors

### Theme and global rules

- **UI-001 Effective appearance.** The effective appearance is `light` when `settings.theme` is `light`, `dark` when it is `dark`, and the OS appearance when it is `system` (the default for a missing or unknown value).
- **UI-002 Live OS appearance.** With `system`, switching the OS between light and dark re-themes every surface of the open window at once, without a restart.
- **UI-003 Window background.** The native window background is `#f5f5f4` (light) / `#1e1e20` (dark); it is set when the window is created and again on every effective-appearance change, so resizing or reloading never flashes the other theme.
- **UI-004 Theme before first paint.** The saved theme is applied before the main window is created, so the first frame is already in the saved theme.
- **UI-005 Native chrome follows the theme.** Native menus (the macOS menu bar menus and the Windows ☰ popup), the folder picker and other native dialogs use the effective appearance, not the OS one, when the theme is forced.
- **UI-006 No text selection by default.** Dragging over labels, headings, buttons or folder names selects no text.
- **UI-007 Inputs select text.** Text in text inputs can be selected and edited normally.
- **UI-008 Selectable path.** The repository path in the remove-project dialog can be selected and shows the I-beam cursor.
- **UI-009 Arrow cursor.** The pointer is the arrow over every element except switches (pointing hand, unless disabled) and text/selectable text (I-beam).
- **UI-010 Keyboard focus ring.** A control focused from the keyboard draws a 2px `--focus-ring` outline 2px outside its border box with its radius + 2px; a mouse click on a button does not show the ring.
- **UI-011 Scrollbars.** Overflowing scroll areas show a 10px gutter with a 4px `--border-strong` pill thumb inset 3px, no track and no arrows; areas this spec marks "scrollbar hidden" show none.
- **UI-012 Reduced motion.** With the OS reduce-motion setting on, every animation and transition completes immediately (0.01ms).
- **UI-013 Stray file drops.** Dragging files over any part of the window that is not a drop target and releasing does nothing (the window never navigates to or opens the file).
- **UI-014 Title bar drags the window.** Pressing on empty title bar area and dragging moves the window; double-clicking empty title bar area performs the OS title-bar double-click action; no-drag controls receive their clicks instead.
- **UI-015 Button states.** A `.btn` shows `--bg-hover` while hovered and `--bg-pressed` while pressed, fading the background over 120ms `ease`; a disabled `.btn` is at 45% opacity and ignores the pointer.
- **UI-016 Primary button.** A `.btn-primary` is `--accent` with white text, `--accent-hover` while hovered (also while pressed and hovered).
- **UI-017 Ghost button.** A `.btn-ghost` has no fill, border or shadow at rest and shows `--bg-hover`/`--bg-pressed` on hover/press.
- **UI-018 Switch.** Toggling a switch slides its knob 14px and changes the track between `--bg-pressed` (+0.5px `--border` ring) and `--accent`, both over 120ms `ease`; a disabled switch is at 50% opacity and cannot be toggled.
- **UI-019 Working dot.** A working status dot is `--status-working` with a ring that breathes between 3px@22% and 5px@10% every 1.6s.
- **UI-020 Done dot.** A done status dot is solid `--status-done` with no ring or animation.
- **UI-021 Waiting dot.** A waiting status dot is `--status-waiting` with a ring that pulses from 0px@55% to 8px@0% every 1.1s.
- **UI-022 Small dot.** A small status dot is 7×7; when working its ring breathes between 2px@22% and 3.5px@10%.

### App shell and navigation

- **UI-023 Starts on the grid.** The app always opens on the projects grid; the last open board is not restored.
- **UI-024 Startup loads.** On start the app loads the projects, groups, per-project stats and global actions (in parallel) and the settings.
- **UI-025 Root layout.** The window is a column: the 52px title bar on top and the main area filling the rest; the main area shows either the projects grid or a board.
- **UI-026 Board route composition.** On a board the main area holds the board (09) and the project rail over it.
- **UI-027 Pinned rail column.** When `settings.pinProjectRail` is on, the board area starts 52px from the window's left edge and the pinned rail fills that column; otherwise the board fills the whole main area and the rail floats over it.
- **UI-028 Board remount per project.** Switching to another project's board creates a fresh board view (no board-local state carries over).
- **UI-029 Rail survives board switches.** The project rail is not recreated when switching from one board to another (its open/hint state carries over); it is created fresh when arriving from the projects grid.
- **UI-030 App-level overlays.** The terminal modal (12), transcript preview (12), agent viewer (12), diff modal (05) and Settings live above both routes and are independent of the route.
- **UI-031 Navigate resets.** Navigating (to the grid or to any board) clears the board snapshot, the board selection, the open terminal, the open transcript preview, the agent viewer, the git status and the diff modal; it keeps Settings open/closed, the preview search term and the pending open request.
- **UI-032 Navigate loads.** Navigating to a board loads that board; navigating to the grid refreshes the projects.
- **UI-033 Stats refresh.** Every session status event and every session process exit schedules a projects refresh 250ms later; another event within that window restarts the 250ms (trailing debounce).
- **UI-034 Board changed event.** A "board changed" event for project P reloads the board only when the board of P is the one open.
- **UI-035 Projects changed event.** A "projects changed" event (e.g. a session added a project with its board tools) refreshes the projects.
- **UI-036 Status event.** A status event updates the matching card on the open board (see UI-070/UI-071); with no board open it is ignored.
- **UI-037 Agent status event.** An agent-status event updates that agent on its card on the open board (see UI-072).
- **UI-038 Usage event.** A usage event replaces the matching card's usage on the open board (see UI-073).
- **UI-039 Settings command.** The app menu's Settings… command (⌘, / Ctrl+,) opens Settings from any route.
- **UI-040 ⌘W closes the top layer.** The Close command (⌘W / Ctrl+W) closes exactly one layer, the first that is open in this order: Settings, agent viewer, terminal modal, transcript preview, diff modal; with none open on a board it goes back to the projects grid; on the grid with none open it does nothing.
- **UI-041 ⌘W never closes the window.** No path of the Close command closes or hides the window.
- **UI-042 Ctrl shortcuts off a Mac.** Off a Mac, a keydown of Ctrl+N, Ctrl+T or Ctrl+W (no Shift, Alt or Meta; not an auto-repeat; not already handled; the focused element not inside a terminal) runs New Session, New Terminal or Close respectively and is consumed; the letter is read by key position on non-Latin layouts (UI-305).
- **UI-043 Notification opens its card.** Clicking an OS notification (02) runs "show card" for its project and session (UI-045).
- **UI-044 Launch open request.** A notification clicked while the window was closed is taken once at start and shown as in UI-043.
- **UI-045 Show card.** Show card goes to the request's board when another route is open; when that board is already open it closes the terminal modal, transcript preview, agent viewer and diff modal instead; in both cases it closes Settings and records the request (with a session) for the board to reveal, or clears it (without a session).
- **UI-046 New project from anywhere.** The New Project… command (⇧⌘N / Ctrl+Shift+N) runs "add project" with the native folder picker from any route (UI-289 to UI-292).
- **UI-047 Toggle focus command.** The Focus Mode command (⇧⌘F / Ctrl+Shift+F) flips `settings.focusMode` only while `settings.focusFeature` is on; otherwise it does nothing.

### Store (UI state and optimistic board mutations)

- **UI-048 What persists.** No UI state is stored by the UI itself; only settings, projects, groups (including `collapsed`) and board data persist, through the core API. Route, board selection, open modals, grid selection, toast, rail open state and rename state are in-memory and reset on relaunch.
- **UI-049 Refresh projects.** Refreshing projects fetches projects, groups, stats and global actions in parallel and replaces all four together.
- **UI-050 Board loading flag.** Loading a board sets "board loading" only when there is no board snapshot or it is for a different project (so every navigation to a board sets it, and reloading the open board does not), and clears it when the load ends, success or failure.
- **UI-051 Stale board loads.** A board load whose project is no longer the open route when it returns is discarded.
- **UI-052 Archived terminal closes.** If a loaded board has the session whose terminal modal is open as archived, the terminal modal closes.
- **UI-053 Patch cards: guards.** Patching cards with no board open or with an empty patch list does nothing.
- **UI-054 Patch cards: empty sessions leave.** A card archived by a patch that was not archived before, is not a terminal card and whose session is empty (no first prompt and no last reply) is removed from the board instead of archived; notes stuck to it fall off (UI-060), it leaves the selection and its terminal instance is disposed.
- **UI-055 Patch cards: merge.** Each other patched card takes the patch's fields; a patch `sectionId` of `null` removes the card's section, `undefined` leaves it unchanged.
- **UI-056 Patch cards: archiving.** A patch with `archived` true sets the card's status to `done`; when the card was not archived before it also sets `archivedAt` to now; any card archived after the patch loses its section and its expanded state.
- **UI-057 Patch cards: persist.** All patches (including those of removed empty sessions) are saved to the core in one call after the local update.
- **UI-058 Upsert card.** Upserting a card replaces the card with the same session id or appends it; nothing happens with no board open.
- **UI-059 Remove card.** Removing a card drops it from the board, unsticks its notes, drops it from the selection, disposes its terminal and then asks the core to remove it.
- **UI-060 Notes fall off.** A note stuck to a card that leaves the board loses its card; it moves to the card's position when the card was active, and keeps its own stored position when the card was archived.
- **UI-061 Archive rect.** Setting the archive rect updates the board and saves it.
- **UI-062 Save section.** Saving a section replaces the section with the same id or appends it, then saves it.
- **UI-063 Remove section.** Removing a section deletes it and makes its cards ungrouped (they stay on the board), then saves.
- **UI-064 Save note.** Saving a note replaces the note with the same id or appends it, then saves it.
- **UI-065 Remove notes.** Removing notes (non-empty list) deletes them locally and saves.
- **UI-066 Save ink.** Saving ink replaces the board's strokes and saves them.
- **UI-067 Upsert view.** Upserting a view replaces the view with the same id or appends it (no save).
- **UI-068 Patch view.** Patching a view merges only the patch fields that are defined, then saves the patch.
- **UI-069 Remove view.** Removing a view deletes it and, when it has a brain session, that session's card too, closes the terminal modal if it shows that session, disposes the session's terminal, then asks the core to remove the view.
- **UI-070 Status on a card.** A status update sets the card's `live` flag and its status, except that an archived card's status stays `done`.
- **UI-071 Stopped session's agents.** A status update with `live` false marks every not-done agent of that card `done`.
- **UI-072 Agent status.** An agent status update changes only an agent already on its card, and only when the status differs; unknown agents wait for the next board load.
- **UI-073 Usage.** A usage update replaces the card's usage only when the card is on the open board.
- **UI-074 Git read sharing.** Reading git status off a board does nothing; while a read for the same project is in flight, another request shares it.
- **UI-075 Git result.** A git read that fails yields "no repository" (null); a result is applied only if that board is still open and it differs (deep-equal) from the current value.
- **UI-076 Terminal vs preview.** Opening a terminal modal closes the transcript preview; opening a preview closes the terminal modal and records its search (or none).
- **UI-077 Agents and diff flags.** Opening the agent viewer records the session and optional agent; opening/closing the diff modal sets its flag.
- **UI-078 Action lists.** Saving or removing an action replaces the project's entry in the project list, the open board's project when it is that project, and the global action list, with what the core returns.
- **UI-079 Optimistic settings.** Updating settings merges the change into the loaded settings at once (if loaded), sends it to the core, and then replaces the settings with the core's returned copy.
- **UI-080 Settings survive navigation.** The Settings dialog stays open across navigation; only show card (UI-045), its close button, Escape, a backdrop press and ⌘W close it.
- **UI-081 Duration format.** A duration is rounded to whole seconds (minimum 0): under 60s → `42s`; under 60 minutes → `4m 12s` when minutes < 10 and seconds ≠ 0, else `12m`; otherwise `1h 5m`.
- **UI-082 Tilde path.** A path starting `/Users/<name>` or `/home/<name>` (followed by `/` or the end) shows with that prefix replaced by `~`.
- **UI-083 Card title.** A card's title is its user title, else its transcript title, else `Terminal` for terminal cards or `New session`.
- **UI-084 Agent urgency order.** Agents sort waiting, then working, then done; ties by start time, earliest first.
- **UI-085 Agent step line.** An agent's step line is `Waiting for you in the session’s terminal` when waiting; `Waiting for its background command to finish` with one background command; `Waiting for N background commands to finish` with N > 1; otherwise its current tool and activity text.
- **UI-086 Agent status label.** `Needs you` (waiting), `Working` (working), `Stopped` (done and stopped), `Done` (done).
- **UI-087 Nested agents.** Agents are listed in start order (or newest first), each followed depth-first by the agents it started at depth + 1; an agent whose parent is not in the list is top level.

### Title bar

- **UI-088 Always there.** The title bar shows on both routes at 52px tall.
- **UI-089 Grid crumb.** On the projects grid the title bar shows only the word `Projects` (title style, no folder icon, no back button).
- **UI-090 Board crumbs.** On a board the title bar shows, left to right: a back chevron button, a `Projects` link, a chevron separator, the project's folder icon (18px, its colour) and name, then the git controls (05).
- **UI-091 Back to grid.** Clicking the back chevron or the `Projects` link goes to the projects grid.
- **UI-092 Project name source.** The board crumb shows the loaded board's project, falling back to the project list entry for the route's id while the board loads, and an empty name (blue folder) when neither is known.
- **UI-093 Project crumb tooltip.** Hovering the board's project crumb shows `<repo path>` and, on a second line, `Double-click to reveal in Finder` (macOS) / `Double-click to show in File Explorer` (elsewhere).
- **UI-094 Reveal from the crumb.** Double-clicking the board's project crumb reveals the repository folder in Finder/File Explorer and does not zoom the window.
- **UI-095 Right cluster order.** The right end shows, left to right with 8px gaps: the running count, the Focus switch, the update button, then the open view's own buttons (grid: New Project; board: 09's buttons).
- **UI-096 Focus switch visibility.** The Focus switch shows only while `settings.focusFeature` is on.
- **UI-097 Focus switch toggles.** Toggling the Focus switch, or clicking its `Focus` label, sets `settings.focusMode` to the switch's new state.
- **UI-098 View action slot.** Each route contributes its own buttons to the end of the right cluster; they appear and disappear with the route.
- **UI-099 Menu button off a Mac.** Off a Mac the title bar starts with a ☰ button that pops up the full application menu with its top-left 4px below the button's bottom-left corner.
- **UI-100 Crumbs hug the menu button.** Off a Mac the crumbs sit right after the ☰ button (8px gap) instead of being centred between the button and the right cluster.
- **UI-101 Long names.** A project name too long for the space truncates with an ellipsis; the right cluster never shrinks.

### Running count

- **UI-102 Hidden at zero.** The running count is absent when no Claude session runs in any project.
- **UI-103 What counts.** The count is the sum over all projects of Claude sessions drawn as cards plus background Claude sessions (view brains, action runs); terminal shells are never counted; hidden and focus-filtered projects are included.
- **UI-104 Label.** The readout is a terminal icon and `N running`.
- **UI-105 Breakdown on hover.** While the pointer is over the readout a card hangs below it, right-aligned to it, listing one row per project with Claude sessions running: `<project name> (<count>)`, in project list order; it disappears when the pointer leaves.

### Update button (placement; logic in 06)

- **UI-106 Shown when ready.** The title bar's `Restart to Update` primary button exists only while the update state is `ready`.
- **UI-107 Update tooltip.** Its tooltip is `Nodus <version> is ready. Sessions keep running while Nodus restarts.` on macOS and `Nodus <version> is ready. Restarting stops the sessions; you can resume them after.` elsewhere.
- **UI-108 Update click.** Clicking it asks the core to install the update (06).

### Usage stats (component; shown by 10 and 12)

- **UI-109 Nothing until reported.** The usage line renders nothing until Claude has reported usage for the session.
- **UI-110 Model.** When a model is known it shows first, in `--text-secondary` weight 500, with tooltip `Model`.
- **UI-111 Context meter.** When a context percentage is known it shows a 22×4px meter filled to min(100, p)% and `Ctx <round(p)>%`; the item (meter fill and text) turns `--status-working` at p ≥ 75 and `--status-waiting` at p ≥ 90.
- **UI-112 Context tooltip.** The context item's tooltip is `<tokens> of <size> tokens in context`, or `Context window in use` when tokens are unknown or size is unknown/zero.
- **UI-113 Running time.** When a duration is known it shows the formatted duration (UI-081); while the session is live it adds the time since the report and re-renders every 1s; tooltip `Time the session has been running`.
- **UI-114 Cost.** When a cost is known it shows `$1.23` (two decimals) under $100 and `$123` (rounded) from $100; tooltip `Cost so far`.
- **UI-115 Token format.** Tokens show as `950`; `9.5k` (one decimal, `.0` dropped) under 10,000; `106k` (rounded) under 1,000,000; `1M` / `1.5M` (one decimal, `.0` dropped) above.

### Settings dialog

- **UI-116 Opening.** Settings opens from the app menu's Settings… (⌘, / Ctrl+,) on any route.
- **UI-117 Dialog frame.** Settings is a 440px-wide Modal with a fixed header and a body that scrolls when the window is too short.
- **UI-118 Reload on open.** Opening Settings re-reads the settings from the core.
- **UI-119 Focus in and back.** Opening Settings moves keyboard focus into the dialog (the panel itself, with no ring); closing it returns focus to whatever had it before (often a terminal).
- **UI-120 Keys stay inside.** No key pressed while focus is in Settings reaches the grid's or board's shortcuts or the rail's ⌘ numbers.
- **UI-121 Escape closes Settings.** Escape pressed with focus inside Settings closes it.
- **UI-122 Backdrop closes Settings.** Pressing on the dimmed backdrop outside the panel closes Settings.
- **UI-123 Tab stays inside.** Tab from the last tab stop goes to the first; Shift+Tab from the first (or from the panel itself) goes to the last. Tab stops are the dialog's enabled buttons and inputs in document order, counting only the checked theme radio.
- **UI-124 Disabled until loaded.** Theme tiles and every switch are disabled until settings are loaded.
- **UI-125 Theme tiles.** Appearance shows three tiles `System`, `Light`, `Dark`; clicking a tile (preview or name) selects it and sets `settings.theme`; Left/Right/Up/Down arrows move the choice between tiles when one has focus.
- **UI-126 Theme tile states.** The selected tile's preview has a 2px `--bg-elevated` gap ring plus 2px `--accent` ring and its name is `--text-primary` weight 500; a hovered unselected tile's shadow grows to `--shadow-md`; a keyboard-focused tile shows a 2px `--focus-ring` outline 5px outside its preview.
- **UI-127 Theme previews.** Each tile shows a miniature board window: `Light` the light tone, `Dark` the dark tone, `System` the light tone with the dark tone over its right part split on a diagonal; the previews keep their tone's colours whatever the app's theme.
- **UI-128 Appearance hint.** Under the tiles: `System follows the appearance set in macOS.` on a Mac, `System follows the appearance set in Windows.` elsewhere.
- **UI-129 Pin sidebar row.** `Always show the project sidebar` / `When off, point at the window’s left edge to slide it in.` with a switch bound to `settings.pinProjectRail`.
- **UI-130 Focus row.** Group `Focus`: `Focus mode` / `Adds a Focus switch to the title bar that shows only the projects in focus. Right-click a project and choose Add to Focus.` with a switch bound to `settings.focusFeature`.
- **UI-131 Capture switch.** Group `Floating capture`: `Double-tap ⇧ in any app to capture` / `A panel floats up over what you’re doing. Type or say what you want, and Claude works out whether it’s a new session, a note, a view or a reply to a session.` with a switch bound to `settings.capture`.
- **UI-132 Capture state polling.** While Settings is open the capture shortcut state is read at once and then every 2000ms; it is read again at once when `settings.capture` changes.
- **UI-133 Accessibility row.** When the shortcut state is `needs-permission` a row shows `Allow Nodus in Accessibility` / `macOS lets an app hear a key pressed in other apps only with this permission. Click Allow… and turn Nodus on in Privacy & Security › Accessibility. Already on there? It was given to another copy of Nodus: Allow… clears it so macOS asks again.` with a primary `Allow…` button that asks the core to request the permission (07) and shows the state it returns.
- **UI-134 Unavailable row.** When the shortcut state is `unavailable` a hint-only row shows `The double tap couldn’t start on this computer. File › Capture… still opens the panel.`
- **UI-135 Try row.** When `settings.capture` is on a row shows the hint `Ready. File › Capture… opens it too.` (the `Ready. ` prefix only when the state is `ready`, else `File › Capture… opens it too.`) and a `Try It` button that opens the capture panel (07).
- **UI-136 Notify row.** Group `Notifications`: `Notify me when a session needs me or finishes` / `Only while Nodus is in the background. Click one to open its card.` bound to `settings.notify`.
- **UI-137 Dock badge row.** On a Mac only: `Count sessions that need me on the Dock icon` (no hint) bound to `settings.dockBadge`.
- **UI-138 Whole row toggles.** Clicking anywhere on a switch row (title or hint text) toggles its switch; rows with buttons (Allow…, Try It, Check Now, Restart to Update) and hint-only rows do not toggle anything.
- **UI-139 Key field loading.** Group `Voice prompts`: the password field (masked) is disabled until the key source is known.
- **UI-140 Key placeholder.** The field's placeholder is `Replace the saved xAI API key` when a key is saved in Settings, else `xAI API key`.
- **UI-141 Save enabled.** `Save` is enabled only when the field holds non-whitespace text.
- **UI-142 Save key.** Clicking Save or pressing Enter in the field (with non-whitespace text) clears any error, saves the key (the core trims it), updates the source, empties the field and keeps focus in the field.
- **UI-143 Key hint by source.** Under the field: saved in Settings → `Saved in your Keychain.` (Mac) / `Saved encrypted for your Windows account.` (elsewhere) followed by a `Remove` link; from the shell → `Using XAI_API_KEY from your shell. A key saved here takes its place.`; none → `Add a key to speak prompts with the mic button. Get one at console.x.ai.` where `console.x.ai` links to `https://console.x.ai`.
- **UI-144 Remove key.** `Remove` deletes the saved key and updates the source (to shell or none).
- **UI-145 Key error.** A failed save shows the error message (without any IPC prefix) in `--status-waiting` below the hints, until the next save attempt.
- **UI-146 Key privacy hint.** `Recordings are sent to xAI to be turned into text.` always shows under the source hint.
- **UI-147 Version row.** Group `Updates`: the first row's title is `Nodus <current version>` (just `Nodus` before the status arrives) with the status text (UI-148) as its hint when there is one.
- **UI-148 Update status text.** off → the reason given by the updater; idle → none; checking → `Checking for updates…`; latest → `You’re up to date.`; downloading → `Downloading version <v>… <percent>%`; ready → `Version <v> is ready to install.`; error → the error message.
- **UI-149 Update button in Settings.** State `ready` → primary `Restart to Update` (installs); state `off` → no button; otherwise `Check Now` (checks now), disabled before the status arrives and while checking or downloading.
- **UI-150 Auto-update row.** Unless the state is `off`: `Download updates automatically` / `They install when you quit Nodus. Sessions keep running.` (Mac) or `They install when you choose Restart to Update, which stops the sessions.` (elsewhere), bound to `settings.autoUpdate`.
- **UI-151 Download link on error.** In state `error` a hint shows `You can also download the latest version yourself.` with `download the latest version` linking to `https://github.com/saleem-hadad/nodus/releases/latest`.
- **UI-152 Links open in the browser.** Links in Settings open in the default web browser, never in the app.

### Modal

- **UI-153 Modal frame.** A Modal draws a full-window dimmed, blurred backdrop above everything but context menus and the numbered rail, with its panel centred.
- **UI-154 Modal enter.** On open the backdrop fades in and the panel fades in rising 8px and growing from 98.5%, both 220ms `--ease-out`; closing removes both at once with no animation.
- **UI-155 Modal Escape.** Escape anywhere closes a Modal, unless that Modal opted out (Settings handles its own, UI-121; terminal modals opt out, 12).
- **UI-156 Modal backdrop press.** A pointer press on the backdrop itself (not inside the panel) closes the Modal unless it opted out.
- **UI-157 Modal size limits.** A panel is never larger than the window minus 48px in either dimension; it clips its content to its 18px corners.

### Context menu

- **UI-158 Menu position.** A context menu opens with its top-left at the given point, then is moved so its right edge stays ≥ 8px inside the window and its left edge ≥ 8px; its bottom is kept ≥ 8px above the window bottom by moving it up (no top clamp).
- **UI-159 Right-aligned menu.** A menu dropped from a button can be right-aligned: the point becomes its top-right corner (then clamped as in UI-158).
- **UI-160 Menu closes.** A context menu closes on a pointer press outside it, on Escape (which goes no further), when the window loses focus, and when the window is resized.
- **UI-161 Anchor presses.** A press on the button that opened a menu is left to that button (so it can toggle the menu) rather than closing it as an outside press.
- **UI-162 Choosing an item.** Clicking an enabled item closes the menu and then runs the item.
- **UI-163 Item states.** An enabled item under the pointer, or keyboard-focused, is `--accent` with `--text-on-accent` text; a disabled item is at 40% opacity and does nothing; a danger item's text and icon are `--status-waiting` except while hovered.
- **UI-164 Item hint.** An item's hint (e.g. a shortcut) sits at its right end in `--text-tertiary` 12px; while the item is hovered the hint takes the item's text colour at 80% opacity.
- **UI-165 Separators and custom rows.** A separator is a 1px `--separator` line inset 8px with 4px above and below; a custom row (e.g. colour swatches) is drawn as given.
- **UI-166 No native menu over a menu.** Right-clicking on an open context menu shows no other menu.
- **UI-167 Menu enter.** A context menu fades in from 98% scale over 90ms `--ease-out`.

### Colour swatches

- **UI-168 Swatch row.** The swatch row shows the eight folder colours in order blue, purple, pink, red, orange, yellow, green, graphite, each a 16px circle filled top-to-bottom from the colour's front-top to front-bottom hue; tooltips `Blue`, `Purple`, `Pink`, `Red`, `Orange`, `Yellow`, `Green`, `Graphite`.
- **UI-169 Current swatch.** The current colour's swatch shows a white check and a ring (2px `--bg-elevated` gap, then 1.5px `--border-strong`).
- **UI-170 Swatch hover.** A hovered swatch grows to 118% over 120ms `--ease-out`.
- **UI-171 Pick a colour.** Clicking a swatch reports that colour (the caller closes its menu and saves).

### Split button

- **UI-172 Main action.** Clicking the split button's main part runs the default action.
- **UI-173 Chevron toggles on press.** Pressing the chevron part opens its menu right-aligned 4px below the whole split button's bottom-right corner, or closes it if open.
- **UI-174 Chevron from the keyboard.** Activating the focused chevron with the keyboard (Enter/Space) toggles the menu too.
- **UI-175 Open state.** While its menu is open the chevron part shows `--bg-pressed` and `--text-primary`.
- **UI-176 Split button look.** The two parts read as one button: one shared `--shadow-sm`, the main part's right border is the divider, the chevron part is 22px wide in `--text-secondary` (hover `--text-primary`).

### Folder icon

- **UI-177 Folder drawing.** A folder icon is the macOS-style folder of FolderIcon (UI details), scaled to the requested square size, in the project's colour.
- **UI-178 Colour fallback.** A missing or unknown colour draws blue.
- **UI-179 Small folders.** Below 32px a folder has no drop shadow and no glyph.
- **UI-180 Letter glyph.** At 32px and up, a folder given a letter shows its first character upper-cased, embossed on the front panel (a white copy 1.2 units lower at 35% opacity under the colour's glyph hue at 62%).
- **UI-181 Icon glyph.** A folder given an icon (Plus, FolderPlus) shows it embossed the same way, 30 units square, stroke 2.4.
- **UI-182 Theme-independent hues.** Folder hues are the same in light and dark.

### Projects grid

- **UI-183 Loads on show.** Showing the grid refreshes the projects; the empty state and the "new folder" tracking wait for that first refresh.
- **UI-184 Empty state timing.** With no projects at all, nothing shows until the first refresh ends; then the empty state shows.
- **UI-185 Empty state.** The empty state shows a 148px folder with the FolderPlus glyph at 55% opacity, `Add your first project`, `Link a local repository to organize its Claude sessions on a board.`, a primary `Choose Folder…` button (folder picker, UI-289) and `or drop a folder anywhere in this window`.
- **UI-186 New Project button.** The grid adds a `New Project` button (plus icon) to the title bar, tooltip `New Project (⇧⌘N)` / `New Project (Ctrl+Shift+N)`, that runs add project with the folder picker.
- **UI-187 Recent order.** Within every section folders are ordered by last opened, most recent first (never-opened count as 0), ties by newest added first.
- **UI-188 Top section.** With at least one project, the top section always shows a heading — `Projects`, or `In Focus` while focusing — followed by the ungrouped folders.
- **UI-189 Heading count.** The top heading's count is the number of listed non-archived projects, grouped ones included.
- **UI-190 Group sections.** After the top section comes one section per group, in group creation order, each headed by its name and its member count.
- **UI-191 Collapse a group.** Clicking a group's name button toggles its `collapsed` flag at once (saved to the core); collapsed, its folders are hidden, its chevron turns −90° over 120ms `--ease-out`, and the button's tooltip reads `Show projects` (expanded: `Hide projects`); the count still shows.
- **UI-192 Empty group.** An expanded group with no listed members shows a dashed `Drag projects here` placeholder.
- **UI-193 Archived section.** Archived projects show last under an `Archived` heading with their count, separated by a hairline; the section is absent with no archived projects.
- **UI-194 All archived.** When every project is archived (and nothing is being dragged), the top section shows `All projects are archived.` under its heading.
- **UI-195 Focus filtering.** While focusing (focus mode on and some non-archived project in focus, UI-297), the top section and groups list only projects in focus plus projects with a session waiting on the user.
- **UI-196 Focus hides archived.** While focusing the Archived section is not shown.
- **UI-197 Focus hides empty groups.** While focusing, groups with no listed member are not shown at all.
- **UI-198 Hidden-count note.** While focusing with N > 0 projects hidden (archived ones included), a note under the grids reads `N other project is hidden by focus mode.` (N = 1) or `N other projects are hidden by focus mode.`
- **UI-199 Nothing-in-focus note.** With focus mode on but no project in focus and at least one non-archived project, the note reads `Focus mode is on, but no project is in focus yet. Right-click a project and choose Add to Focus.`
- **UI-200 Dimmed folders.** While focusing, a listed folder not in focus (there only because a session waits) is faded like an archived folder.
- **UI-201 Focus mark.** With `focusFeature` on, a non-archived folder in focus shows a target badge at its icon's top-left, tooltip `In focus`.
- **UI-202 Select on press.** A primary-button press on a folder selects it (single selection).
- **UI-203 Deselect on empty press.** Any press in the grid's scroll area outside a folder (headings, gaps, group buttons included) clears the selection.
- **UI-204 Selection persists.** The grid's selection survives visiting a board and coming back.
- **UI-205 Stale selection.** When the selected project disappears from the list, the selection clears.
- **UI-206 Keep selection visible.** Whenever the selection changes, the grid scrolls the minimum needed to show the selected folder.
- **UI-207 Double-click opens.** Double-clicking a folder opens its board.
- **UI-208 Open shortcut.** ⌘O or ⌘↓ (Ctrl+O / Ctrl+Down elsewhere) opens the selected project's board; with no selection it does nothing.
- **UI-209 Remove shortcut.** ⌘⌫ (Ctrl+Backspace elsewhere) opens the remove confirmation for the selected project.
- **UI-210 Enter renames.** Enter (no modifiers) starts renaming the selected folder.
- **UI-211 Escape deselects.** Escape (no modifiers) clears the selection.
- **UI-212 Arrows start at the first.** An arrow key with no selection selects the first folder in reading order.
- **UI-213 Left/Right.** Left/Right select the previous/next folder in reading order (ungrouped, then each expanded group, then archived), stopping at the ends (no wrap); folders of collapsed groups are skipped.
- **UI-214 Up/Down.** Up/Down select, in the nearest row above/below (by on-screen position across all grids), the folder whose left edge is closest to the current one's (first in reading order on a tie); at the top/bottom row nothing changes.
- **UI-215 Keys ignored when busy.** Grid shortcuts are ignored while a context menu, the remove confirmation, the group picker or a rename is open, and when the key goes to a text field.
- **UI-216 Project menu.** Right-clicking a folder selects it and opens its menu at the pointer (UI details lists the items).
- **UI-217 Change colour.** Picking a swatch in the folder menu closes the menu and saves the project's colour.
- **UI-218 Archive / Unarchive.** `Archive` archives the project and takes it out of focus; `Unarchive` restores it.
- **UI-219 Focus from the grid.** With `focusFeature` on, a non-archived project's menu offers `Add to Focus` / `Remove from Focus`, which saves `focused`.
- **UI-220 Reveal.** `Reveal in Finder` / `Show in File Explorer` reveals the repository folder.
- **UI-221 Group header menu.** Right-clicking a group heading opens `Rename Group`, a separator, and `Delete Group` (danger).
- **UI-222 Rename group.** `Rename Group` replaces the heading with an inline field (UI-237); a changed non-empty name is applied at once and saved.
- **UI-223 Delete group.** `Delete Group` deletes the group at once with no confirmation; its projects become ungrouped.
- **UI-224 Drag to a section.** A non-archived folder that is not being renamed can be dragged onto another section (top section or a group, collapsed ones included) to file it there.
- **UI-225 Drop highlight.** While dragging a folder, the section under the pointer is highlighted (UI details) only when it differs from the folder's own section.
- **UI-226 Drop moves.** Dropping on a different section moves the project to that group (or out of its group for the top section); dropping on its own section does nothing.
- **UI-227 Take-out placeholder.** While a grouped folder is dragged and the top section has no folders, the top section shows `Drop here to take it out of its group` instead of its usual content.
- **UI-228 OS folder drop overlay.** While files from the OS are dragged over the grid view, a dashed overlay with a folder-plus icon reads `Drop a folder to add it as a project`; it hides when the drag leaves the view or drops.
- **UI-229 OS folder drop adds.** Dropping files adds each one as a project in turn (UI-289), each becoming selected; non-folders show the failure toast (UI-290).
- **UI-230 New folders pop in.** A folder that appears while the grid is showing (after its first refresh) pops in once (opacity 0→1, scale 0.6→1, 520ms `--ease-spring`).
- **UI-231 Folder hover and press.** Hovering a folder lifts its drawing 1px over 220ms `--ease-out`; pressing it shrinks the icon box to 97% over 120ms `--ease-out`.
- **UI-232 Folder tooltip.** Hovering a folder shows its repository path as a tooltip.

### Folder item

- **UI-233 Subtitle.** Under the name: a blank line of the same height while stats are loading; `No sessions` with none; `1 session` / `N sessions`, followed by ` · M active` when M > 0.
- **UI-234 Live badges.** When the project has sessions waiting or working, pills at the icon's top-right show the waiting count (pulsing waiting dot) above the working count (static working dot); tooltips `N waiting for you` / `N working`.
- **UI-235 Label.** The name shows centred in at most two lines, clamped with an ellipsis; when selected the name sits on an `--accent` pill with white text and the icon box is tinted `--bg-pressed`.
- **UI-236 Faded folders.** Archived and dimmed folders draw their folder at 42% opacity and 40% saturation (70% opacity while hovered or selected) and their name in `--text-secondary` unless selected; badges stay at full strength.

### Rename field

- **UI-237 Rename field opens.** Starting a rename shows an inline field holding the current name, focused with all text selected.
- **UI-238 Rename commits.** Enter or leaving the field commits; Escape cancels; the result counts only if the trimmed text is non-empty and differs from the old name; it fires once.
- **UI-239 Rename isolation.** Keys, presses and double-clicks in the field do not reach the grid (no deselect, no open, no shortcuts).
- **UI-240 Project rename saves.** A committed project name is saved and the projects refreshed.

### Group picker (Move to Group…)

- **UI-241 Picker opens.** `Move to Group…` opens a 340px Modal titled `Move “<project name>” to`.
- **UI-242 Picker rows.** When groups exist the picker lists them in creation order, then `No Group`, scrolling past 260px.
- **UI-243 Current row.** The project's current group (or `No Group` when ungrouped or its group no longer exists) is tinted `--bg-selected` with an `--accent` check.
- **UI-244 Pick a group.** Clicking a row closes the picker and, if it is not the current one, moves the project there.
- **UI-245 New group.** Submitting a non-empty `New group name` (Enter or `Create`) closes the picker, creates the group with the trimmed name and moves the project into it; `Create` is disabled while the field is blank.
- **UI-246 Picker focus.** With no groups yet, the name field takes focus when the picker opens; otherwise nothing is focused.

### Remove confirmation

- **UI-247 Remove dialog.** `Remove from Nodus…` or ⌘⌫ opens a 440px Modal: the project's folder (56px), `Remove “<name>”?`, `The project and its board layout are removed from Nodus. The repository and its Claude session transcripts stay on disk, untouched.`, the repository path, and `Cancel` / `Remove`.
- **UI-248 Remove is focused.** `Remove` has keyboard focus when the dialog opens, so Enter or Space confirms.
- **UI-249 Confirm removes.** `Remove` closes the dialog, removes the project, clears the selection if it was that project, and refreshes.
- **UI-250 Cancel.** `Cancel`, Escape or a backdrop press closes the dialog without removing.

### Toast

- **UI-251 Toast.** A toast shows at the bottom centre of the grid, rising in over 220ms; it disappears after 3500ms, and a new toast replaces the current one and restarts the 3500ms.
- **UI-252 Toast belongs to the grid.** Toasts show only on the grid; one raised while a board is open shows if the grid opens before its 3500ms ends.

### Project rail

- **UI-253 Rail only on boards.** The rail and its ⌘ number shortcuts exist only while a board is open.
- **UI-254 Rail order.** The rail lists non-archived projects in the order they were added (oldest first, never reordered by opening): ungrouped first, then each group's projects in group creation order, then a `+` button.
- **UI-255 Rail focus filter.** While focusing, the rail lists only projects in focus, projects with a session waiting and the open project.
- **UI-256 Rail separators.** A short hairline follows every non-empty block (ungrouped, each group), so the last one separates the projects from the `+` button.
- **UI-257 Rail item states.** A rail item shows `--bg-hover` hovered, `--bg-pressed` pressed, and `--bg-selected` with a 3×18px `--accent` pill on the rail's left edge when it is the open project.
- **UI-258 Rail status dot.** A rail item shows a 9px status dot at its top-right: waiting when any session waits, else working when any works, else none.
- **UI-259 Rail unfocused fade.** While focusing, a rail item not in focus draws its folder at 45% opacity and 40% saturation.
- **UI-260 Rail click.** Clicking a rail item opens that project's board; clicking the open project does nothing.
- **UI-261 Rail add.** The `+` button runs add project with the folder picker and, on success, opens the new (or existing) project's board.
- **UI-262 Edge reveal.** With the rail not pinned, entering the 6px strip at the window's left edge (below the title bar) and staying in it for 100ms (moving within it does not restart the wait) slides the rail in.
- **UI-263 No reveal while dragging.** The edge does not reveal the rail while any mouse button is held.
- **UI-264 Hide grace.** When the pointer leaves the open rail (or the edge strip), the rail slides out after 300ms unless the pointer comes back to the rail or the edge first.
- **UI-265 Quick sweeps.** Leaving the edge strip before the 100ms reveal elapses cancels the reveal.
- **UI-266 Rail keeps itself open.** Entering the rail (even while it is still sliding) keeps it open.
- **UI-267 Menu holds the rail.** While the rail's context menu is open the rail does not hide; when the menu closes and the pointer is not over the rail, it slides out after 300ms.
- **UI-268 Slide animation.** The rail slides in from 60px left of its place (its width + 8px) and fades in, both 220ms (`--ease-out` for the slide, `ease` for the fade); sliding out reverses.
- **UI-269 Hidden rail is inert.** While hidden the rail cannot be clicked, hovered or reached with Tab.
- **UI-270 Pinned rail.** Pinned, the rail is always shown as a full-height 52px sidebar flush with the window's left edge (no slide, no edge strip, no hide).
- **UI-271 Rail hint.** Hovering a rail item (rail shown) shows a hint card 6px right of the rail, vertically centred on the item, fading in from 3px left over 120ms `--ease-out`.
- **UI-272 Hint content.** A project's hint shows its name and a detail line joining, with ` · `, its group name, `N waiting` and/or `N working`, `Not in focus` (while focusing and not in focus), and its shortcut `⌘N`/`Ctrl+N` when it is among the first nine; parts that don't apply are left out, and with none the detail line is absent. The `+` button's hint shows `New Project` and `⇧⌘N` / `Ctrl+Shift+N`.
- **UI-273 Hint hides.** The hint hides when the pointer leaves the item, when the rail scrolls, when a rail menu opens, and when the rail hides.
- **UI-274 Rail menu.** With `focusFeature` on, right-clicking a rail item opens a menu with `Add to Focus` / `Remove from Focus` for that project; with it off, right-click does nothing.
- **UI-275 Held ⌘ shows numbers.** Holding ⌘ (Ctrl off a Mac) on its own for 300ms shows number badges 1–9 on the first nine rail items, sliding the rail out if hidden.
- **UI-276 Number guard.** ⌘ numbers (badges and switching) don't start when Shift, Alt or the other modifier (Ctrl on a Mac, Meta elsewhere) is held, while Settings is open, or off a Mac while focus is in a terminal; an auto-repeat ⌘ keydown does not restart the 300ms.
- **UI-277 ⌘1–⌘9 switch.** ⌘ plus digit N (by key position, top row or numpad) opens the Nth rail project's board (unless it is already open) and is consumed; with no Nth project the key passes through.
- **UI-278 Numbers stay while held.** After ⌘N switches boards the badges stay while ⌘ is held, to go on to another.
- **UI-279 Numbers hide.** The badges (and a pending 300ms wait) end when ⌘ is released, on any keyup while ⌘ is not held, on any keydown other than ⌘ itself or an unblocked ⌘-digit (⌘ is part of another shortcut, or Shift/Alt joined it), and when the window loses focus. An unblocked ⌘-digit with no project at that position leaves them as they are.
- **UI-280 ⌘-click and ⌘-scroll.** A pointer press or a wheel event while ⌘ is held before the 300ms elapse cancels the pending badges (badges already shown stay).
- **UI-281 Numbers above modals.** While the badges show, the rail draws above modals (z 600) so the targets are visible over an open terminal; when ⌘ is released over the rail, the rail stays open under the pointer.
- **UI-282 Key badge.** Each number badge pops in from 70% scale and transparent over 120ms `--ease-out`.
- **UI-283 Rail stats.** Opening a board (mounting the rail) refreshes the projects so badges and dots are current.
- **UI-284 Rail overflow.** When the projects don't fit, the rail scrolls vertically with no visible scrollbar.

### Add project and grouping helpers (`projectsUi.ts`)

- **UI-285 Group order.** Groups are always ordered by creation time, oldest first.
- **UI-286 Group split.** Splitting projects into ungrouped and per-group lists keeps their order; a project whose group no longer exists counts as ungrouped.
- **UI-287 Move to group.** Moving a project saves its `groupId` (null to ungroup) and refreshes the projects.
- **UI-288 Optimistic group change.** Renaming or collapsing a group updates it locally at once and then saves it.
- **UI-289 Add project.** Adding a project uses the given path or, without one, the native folder picker (title/button/message owned by 01: `Add Project`, `Add Project`, `Choose the local repository for this project`); the core returns the new project, or the existing project for a folder already added (unarchived, and joining the focus while focusing).
- **UI-290 Add failure toasts.** If the core rejects the folder, the grid toast shows `Couldn’t add “<last path segment>”. Only folders can be projects.` for a given path, or `Couldn’t add that folder.` from the picker.
- **UI-291 Picker cancelled.** Cancelling the folder picker does nothing.
- **UI-292 Add then show.** Without "open", the added project becomes the grid selection and the grid shows (navigating there from a board, else refreshing); with "open" (rail `+`) the projects refresh and its board opens unless already open.
- **UI-293 Set focused.** Bringing a project into or out of focus saves `focused` and refreshes.
- **UI-294 Toggle focus mode.** Toggling focus mode flips `settings.focusMode` only while `focusFeature` is on.
- **UI-295 Folder glyph letter.** A project's folder glyph is the first Unicode letter or digit anywhere in its name; with none the folder has no glyph.

### Focus (`focus.ts`)

- **UI-296 Focus mode on.** Focus mode is on only when `focusFeature` and `focusMode` are both on (missing settings: off).
- **UI-297 Focusing.** The lists narrow only while focus mode is on and at least one non-archived project is in focus.
- **UI-298 Shows in focus.** While focusing a project shows if it is in focus, if any of its sessions waits on the user, or if it is the open project (rail only).
- **UI-299 Focus everywhere off.** With `focusFeature` off nothing of focus shows: no title-bar switch, no menu items, no target marks, no filtering, no notes.
- **UI-300 Focusing definition shared.** The core's add-project uses the same focusing test, so a project added while focusing joins the focus.

### Platform helpers (`platform.ts`)

- **UI-301 Platform detection.** The app treats macOS as `mac`, Windows as `windows` and anything else as `linux`; every non-Mac platform gets the Windows look (☰ button, caption inset, Ctrl shortcuts).
- **UI-302 Shortcut modifier.** The shortcut modifier is ⌘ on a Mac and Ctrl elsewhere; "the other" modifier (which a shortcut must not have) is Ctrl on a Mac and Meta elsewhere.
- **UI-303 Shortcut text.** Shortcuts are written the Mac way (`⇧⌘F`) on a Mac; elsewhere each symbol becomes a name (⌘ and ⌃ → `Ctrl`, ⌥ → `Alt`, ⇧ → `Shift`, ↩ → `Enter`, ⌫ → `Del`) and they are joined with `+` in the order Ctrl, Alt, Shift, then the key (`Ctrl+Shift+F`, `Ctrl+Enter`, `Ctrl`, `Del`).
- **UI-304 Shortcut letter (Latin).** A shortcut's key is the pressed key in lower case when it is a Latin letter or not a letter key at all (`Enter` → `enter`, `é` on the digit row → `é`).
- **UI-305 Shortcut letter (other alphabets).** On a non-Latin layout a letter key (key codes 65–90) is read from its key code (Arabic `ر` on V → `v`, Russian `с` on C → `c`).

## UI details

Conventions: sizes in logical px; "L / D" = light / dark resolved value; padding is written top right bottom left; icons are lucide (v1.49 in 0.9.4) at the stated size and stroke width (stroke in the icon's 24-unit viewBox), drawn in the current text colour unless stated. Tooltips marked "native" are the platform's own tooltip (HTML `title`), shown after the OS hover delay; reproduce their look and delay as closely as the toolkit allows (see Rust mapping, risks).

### App root

| Box | Value |
| --- | --- |
| Window | default 1440×920, minimum 960×620, title `Nodus` (01 owns; listed for layout) |
| Root | column, fills the window, background `--bg-window` (L `#f5f5f4` / D `#1e1e20`) |
| Title bar | 52px tall, full width, not shrinkable |
| Main area | fills the remaining height, position context for the grid, board area, rail and edge strip |
| Board area | fills the main area; left edge at 52px when the rail is pinned |

### Title bar

| Part | Value |
| --- | --- |
| Bar | height 52px; row, items vertically centred, content spread to both ends (`space-between`), gap 16px between its children; padding macOS `0 14 0 84` (84px leaves room for the traffic lights at x=18, y=18); off a Mac `0 (14 + caption-area width) 0 8` where the caption area is the Windows minimise/maximise/close overlay (fallback width 138px); background `--bg-toolbar` over the window = L `#f6f6f5` / D `#252527`; bottom hairline 0.5px `--border` (L `rgba(0,0,0,0.1)` / D `rgba(255,255,255,0.1)`); z 50 |
| Drag | the whole bar is a window-drag region except: ☰ button, back button, `Projects` link, the board's project crumb, the right cluster (everything in it), git controls (05) |
| ☰ button (off Mac) | `.btn .btn-ghost .btn-icon`: 28×28, radius 6; icon `menu` 16px stroke 2; colour `--text-secondary`, hover `--text-primary` + `--bg-hover`; right margin −8px; native tooltip `Menu`; accessible name `Menu`. Click: pop up the application menu at (button left, button bottom + 4) in window coordinates |
| Crumbs | row, centred, gap 4px, may shrink (min width 0). Off a Mac: pushed left against the ☰ button (takes the free space on its right) |
| Back button (board) | `.btn .btn-ghost .btn-icon` 28×28; icon `chevron-left` 18px stroke 2; colour `--text-secondary`, hover `--text-primary`; right margin 2px; native tooltip and name `Back to Projects` |
| `Projects` link (board) | height 24px, padding `0 6`, no border, radius 6; transparent; text `Projects` 13px weight 500 `--text-secondary`; hover background `--bg-hover`, text `--text-primary`; pressed `--bg-pressed`; no transition |
| Separator (board) | icon `chevron-right` 13px stroke 2.2, colour `--text-tertiary`, not shrinkable |
| Current crumb | row, centred, gap 7px, padding-left 4px, may shrink. Board: folder icon 18px (project colour, no shadow, no glyph) + name. Grid: name `Projects` only (no icon; part of the drag region) |
| Current name | `--font-display` 15px weight 600, letter-spacing −0.15px, `--text-primary`, single line, ellipsis when truncated; line height 21px |
| Board crumb tooltip | native, two lines: `<repo path>` / `Double-click to reveal in Finder` (macOS) or `Double-click to show in File Explorer` |
| Git controls | 05, directly after the current crumb inside the crumbs row (gap 4px) |
| Right cluster | row, centred, gap 8px, never shrinks; children in order: running count, Focus switch, update button, view-actions slot (itself a row with gap 8px) |
| Focus switch label | row, centred, gap 7px, padding `0 4`; text `Focus` 13px weight 500 `--text-secondary`, then a `.switch`; native tooltip `Show only the projects in focus (⇧⌘F)` / `(Ctrl+Shift+F)`; clicking the text toggles too |
| Grid's view action | `.btn`: icon `plus` 15px stroke 2.2, gap 6, text `New Project`; native tooltip `New Project (⇧⌘N)` / `New Project (Ctrl+Shift+N)` |
| Update button | `.btn .btn-primary`: icon `rotate-cw` 14px stroke 2.2, gap 6, text `Restart to Update`; native tooltip per UI-107 |

### Running count

| Part | Value |
| --- | --- |
| Readout | inline row, centred, gap 5px, height 24px, padding `0 6`; 12px tabular numerals, `--text-secondary`; no wrap; arrow cursor; no background or hover change. Content: icon `square-terminal` 13px stroke 2, then `N running` |
| Breakdown card | appears on pointer enter, disappears on pointer leave (no animation, no delay); positioned 6px below the readout's bottom, right edges aligned; column, gap 2px; max width 280px; padding `5 9`; radius 6; `--bg-elevated`; `--shadow-md`; text 12px `--text-primary`, line height 16.8px; ignores the pointer; drawn above the main area (title bar layer). Rows: `<name> (<count>)`, single line, ellipsis |

### Settings dialog

| Part | Value |
| --- | --- |
| Panel | Modal (below) with width 440px (max window − 48px), height to content (max window − 48px) |
| Inner | column; keyboard-focusable itself (no focus ring) |
| Header | not scrolling; row, centred, space-between, gap 16; padding `14 12 12 22`; bottom hairline 0.5px `--separator` (L `rgba(0,0,0,0.07)` / D `rgba(255,255,255,0.07)`) |
| Title | `Settings`, `--font-display` 15px weight 600, line height 21px |
| Close button | `.btn .btn-ghost .btn-icon` 28×28, icon `x` 16px stroke 2; native tooltip `Close (Esc)`; name `Close` |
| Body | fills the rest, scrolls vertically (standard scrollbar); padding `18 22 22`; content width 396px |
| Group | no border/padding; 26px above every group but the first |
| Group label | 13px weight 600 `--text-primary`; 12px below it. Labels in order: `Appearance`, `Focus`, `Floating capture`, `Notifications`, `Voice prompts`, `Updates` |
| Row | row, centred vertically, space-between, gap 16px; 14px above a row that follows another row or a hint |
| Row text | column, gap 3px, may shrink. Title 13px weight 500 `--text-primary` (18.2px line). Hint 12px `--text-secondary`, line height 1.4 (16.8px), wraps |
| Hint paragraph | 12px `--text-secondary`; 14px above it; 4px above a hint that follows a hint |
| Error | 12px `--status-waiting`; 8px above it |
| Links | `Remove` button and `console.x.ai` / `download the latest version` links: no padding/border/background, `--accent`, same font as the hint, no underline; underline on hover; arrow cursor |
| Row controls | switches (`.switch`), buttons (`.btn`, `.btn-primary`), all keep their natural size at the right |

Theme tiles:

| Part | Value |
| --- | --- |
| Tile row | row, gap 16px; three tiles of equal width ((396 − 32) / 3 ≈ 121.33px) |
| Tile | column, centred, gap 8px; the whole tile (preview + name) is the click target; a hidden radio inside keeps keyboard behaviour (arrows switch, only the checked radio is a tab stop) |
| Preview | full tile width, aspect 16:10 (≈ 75.8px tall), radius 7px, clips its content; shadow `0 0 0 0.5px --border-strong` + `--shadow-sm`; hover: `0 0 0 0.5px --border-strong` + `--shadow-md`; checked: `0 0 0 2px --bg-elevated`, `0 0 0 4px --accent` (checked wins over hover); shadow change animates 120ms `ease`; keyboard focus: 2px `--focus-ring` outline 5px outside the preview (radius 12px) |
| Name | 12px `--text-secondary`; checked: `--text-primary` weight 500 |
| Mini window | fills the preview; column; background mini canvas colour |
| Mini title bar | 13px tall, row, centred, gap 3px, padding `0 6`; mini bar colour; bottom hairline 0.5px mini border; three 4×4 circles `#ff5f57`, `#febc2e`, `#28c840` |
| Mini canvas | fills the rest; row, top-aligned, gap 6px, padding `9 8`; dot grid: one dot per 6×6px tile, centred in the tile, solid mini-dot colour to radius 0.6px fading to transparent at 0.9px |
| Mini cards | two, equal width, column, gap 4px, padding 5px, radius 4px, mini card colour, shadow `0 0 0 0.5px <mini border>`, `0 1px 2px rgba(0,0,0,0.08)`. Contents: a 5×5 circle (first card `--status-working`, second `--status-done`, both from the **app's current** theme), a 3px line (radius 2px, mini line colour, full width), a 3px line at 60% width |
| Split (System) | the dark mini window is drawn over the light one, clipped to the polygon (62% 0, 100% 0, 100% 100%, 38% 100%) of the preview box |
| Tone colours | see Hard-coded colours (light tone / dark tone) |

Voice key form:

| Part | Value |
| --- | --- |
| Form | row, gap 8px: input then `Save` (`.btn`, disabled = 45% opacity) |
| Input | fills the row; height 28px; padding `0 10`; border 0.5px `--border-strong`; radius 6; `--bg-elevated`; 13px `--text-primary`; masked (password); no spellcheck, no autocomplete; placeholder `--text-tertiary`; focused: 2px `--focus-ring` outline at 0 offset (radius 6) and the border turns transparent |

Copy, in order (verbatim, see also UI-128…UI-151):

1. `Appearance` — tiles `System` / `Light` / `Dark`; hint `System follows the appearance set in macOS.` (Mac) / `System follows the appearance set in Windows.`; row `Always show the project sidebar` / `When off, point at the window’s left edge to slide it in.`
2. `Focus` — row `Focus mode` / `Adds a Focus switch to the title bar that shows only the projects in focus. Right-click a project and choose Add to Focus.`
3. `Floating capture` — row `Double-tap ⇧ in any app to capture` / `A panel floats up over what you’re doing. Type or say what you want, and Claude works out whether it’s a new session, a note, a view or a reply to a session.`; conditional rows `Allow Nodus in Accessibility` / `macOS lets an app hear a key pressed in other apps only with this permission. Click Allow… and turn Nodus on in Privacy & Security › Accessibility. Already on there? It was given to another copy of Nodus: Allow… clears it so macOS asks again.` + `Allow…`; `The double tap couldn’t start on this computer. File › Capture… still opens the panel.`; `Ready. File › Capture… opens it too.` + `Try It`.
4. `Notifications` — `Notify me when a session needs me or finishes` / `Only while Nodus is in the background. Click one to open its card.`; Mac: `Count sessions that need me on the Dock icon`.
5. `Voice prompts` — field (`xAI API key` / `Replace the saved xAI API key`) + `Save`; hint per source; `Recordings are sent to xAI to be turned into text.`; error line.
6. `Updates` — `Nodus <version>` + status hint + `Check Now` / `Restart to Update`; `Download updates automatically` + platform hint; error: `You can also download the latest version yourself.`

### Modal

| Part | Value |
| --- | --- |
| Backdrop | covers the window, z 500; L `rgba(0,0,0,0.28)` / D `rgba(0,0,0,0.45)` over a 3px backdrop blur of the window; centres the panel; enter: opacity 0→1, 220ms `--ease-out` |
| Panel | column; max width window − 48px, max height window − 48px; radius 18px; `--bg-elevated` (L `#ffffff` / D `#2a2a2d`); `--shadow-modal`; clips content; enter: opacity 0→1 and translateY 8px→0 and scale 0.985→1, 220ms `--ease-out` (transform origin centre) |
| Exit | none: both disappear immediately |
| Semantics | dialog, modal, labelled by its title |

### Context menu

| Part | Value |
| --- | --- |
| Menu | floating, z 1000; min width 200px (grows to the widest row); padding 5px; radius 10px; background L `rgba(255,255,255,0.88)` / D `rgba(42,42,45,0.88)` over a 30px backdrop blur with saturation 1.8; `--shadow-lg`; 13px text; enter: opacity 0→1, scale 0.98→1 (from centre), 90ms `--ease-out` |
| Item | row, centred, gap 8px, full width, height 26px, padding `0 10 0 6`, radius 5px, transparent, left-aligned text in `--text-primary` |
| Item hover / keyboard focus | `--accent` background, `--text-on-accent` text and icon, no focus ring (no transition) |
| Disabled item | opacity 0.4, no hover |
| Danger item | text and icon `--status-waiting` (L `#ff3b30` / D `#ff453a`) unless hovered; hovered: white on `--accent`; keyboard-focused but not hovered: `--status-waiting` text on `--accent` (rule precedence; reproduce) |
| Icon slot | 16px wide, centred, opacity 0.85; icon 14×14 (stroke 2) |
| Label | fills the row |
| Hint | 12px `--text-tertiary`; hovered item: item colour at 80% opacity |
| Separator | 1px tall, margin `4 8`, `--separator` |
| Custom row | rendered as given (e.g. swatches) |

### Colour swatches

| Part | Value |
| --- | --- |
| Row | row, gap 6px, padding `5 8 5 30` (30px lines the swatches up with item labels) |
| Swatch | 16×16 circle; vertical gradient front-top → front-bottom of the colour (FolderIcon table); inset ring 0.5px `rgba(0,0,0,0.18)`; arrow cursor; hover: scale 1.18, 120ms `--ease-out` |
| Current | outer rings `0 0 0 2px --bg-elevated`, `0 0 0 3.5px --border-strong` (a 1.5px ring after a 2px gap) plus the inset ring; icon `check` 10px stroke 3.5 `#fff` centred |
| Tooltip | native, capitalised colour name; accessible name the lower-case colour; group name e.g. `Folder colour` |

### Split button (used by 09 in the title bar)

| Part | Value |
| --- | --- |
| Group | inline row; radius 6; one `--shadow-sm` for both parts (parts drop their own shadow); no-drag |
| Main part | `.btn` with right corners square |
| Chevron part | `.btn` 22px wide, no padding, centred, no left border, left corners square; icon `chevron-down` 13px stroke 2.2; `--text-secondary`; hover or open `--text-primary`; open: `--bg-pressed` background; native tooltip and name = menu label (default `More`) |
| Menu | ContextMenu, right-aligned at (group right, group bottom + 4) |

### FolderIcon

Drawn in a 128×128 box scaled to `size`×`size` (sizes used here: 18 title bar, 34 rail, 56 remove dialog, 88 drop overlay, 112 grid, 148 empty state). Template (SVG, `{…}` filled per colour; gradient units are each shape's bounding box, so the back gradient spans y 18–110 and the front ones y 39–110):

```svg
<svg width="{size}" height="{size}" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="back" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{backTop}"/>
      <stop offset="1" stop-color="{backBottom}"/>
    </linearGradient>
    <linearGradient id="front" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{frontTop}"/>
      <stop offset="1" stop-color="{frontBottom}"/>
    </linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.55"/>
      <stop offset="0.05" stop-color="#fff" stop-opacity="0.12"/>
      <stop offset="0.5" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="130%">
      <feDropShadow dx="0" dy="1.5" stdDeviation="1.6" flood-color="#000" flood-opacity="0.2"/>
    </filter>
  </defs>
  <!-- filter="url(#shadow)" only when size >= 32 -->
  <g filter="url(#shadow)">
    <path d="M10 30C10 23.4 15.4 18 22 18H46C49.5 18 52.6 19.6 54.8 22.3L58 26.2C59.8 28.3 62.4 29.5 65.2 29.5H108C114.6 29.5 118 33 118 39.5V100C118 106.6 114.6 110 108 110H20C13.4 110 10 106.6 10 100Z" fill="url(#back)"/>
    <rect x="10" y="39" width="108" height="71" rx="9" fill="url(#front)"/>
    <rect x="10" y="39" width="108" height="71" rx="9" fill="url(#sheen)"/>
    <path d="M19 39.8H109" stroke="#fff" stroke-opacity="0.6" stroke-width="1.2" stroke-linecap="round"/>
  </g>
  <!-- glyph (only when given and size >= 32), see below -->
</svg>
```

Hues per `FolderColor` (identical in light and dark):

| Colour | backTop | backBottom | frontTop (= swatch top) | frontBottom (= swatch bottom) | glyph |
| --- | --- | --- | --- | --- | --- |
| blue | `#3aa8f0` | `#1d8be0` | `#6dd0fc` | `#2da4f3` | `#1478c8` |
| purple | `#a67ae8` | `#8b5bd9` | `#c8a2f7` | `#a77beb` | `#7447c4` |
| pink | `#f06fa8` | `#e0508f` | `#fba0c8` | `#f074aa` | `#c83a78` |
| red | `#f2615a` | `#e0443e` | `#ff9189` | `#f76760` | `#c4302b` |
| orange | `#fa9d3a` | `#ee8420` | `#ffc374` | `#fba343` | `#d06d10` |
| yellow | `#f5c73a` | `#e6b020` | `#ffe27a` | `#f9cb45` | `#be8e0c` |
| green | `#5cc96a` | `#3db352` | `#93e19c` | `#5dcd6b` | `#2a9340` |
| graphite | `#8e8e93` | `#76767b` | `#bdbdc2` | `#9a9aa0` | `#636368` |

Letter glyph (two copies, the light one first/underneath):

```svg
<text x="64" y="76.2" text-anchor="middle" dominant-baseline="central" font-size="34" font-weight="600"
      font-family="'SF Pro Rounded', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
      fill="#fff" fill-opacity="0.35">{LETTER}</text>
<text x="64" y="75" (same attributes) fill="{glyph}" fill-opacity="0.62">{LETTER}</text>
```

`{LETTER}` = first UTF-16 unit of the glyph string, upper-cased (UI-295 picks the string). "central" baseline: the glyph's vertical centre sits on y.

Icon glyph (two copies, 30×30 box at x=49): light copy at y=61.2 with `color="#fff" opacity="0.35"`, coloured copy at y=60 with `color="{glyph}" opacity="0.62"`; each copy is the lucide icon `<svg x y width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">`. Glyph icons used:

- `plus`: `<path d="M5 12h14"/>`, `<path d="M12 5v14"/>` (drop overlay, blue folder)
- `folder-plus`: `<path d="M12 10v6"/>`, `<path d="M9 13h6"/>`, `<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>` (empty state, blue folder)

### Usage stats

| Part | Value |
| --- | --- |
| Line | row, centred, gap 10px, not shrinkable; 11px `--text-tertiary` tabular numerals; no wrap |
| Item | inline row, centred, gap 5px; native tooltips per UI-110…UI-114 |
| Model | `--text-secondary`, weight 500 |
| Meter | 22×4px, radius 2px, track `--border`, clips; fill from the left, width min(100, p)%, same radius, colour = the item's text colour (`--text-tertiary`, or `--status-working` ≥75%, `--status-waiting` ≥90%) |
| Order | model, context, running time, cost (each only when known) |

### Projects grid

| Part | Value |
| --- | --- |
| View | fills the main area; column; background `--bg-window` |
| Scroll area | fills the view; scrolls vertically (standard scrollbar); padding `28 32 48` |
| Section (top and each group) | horizontal margin −10px with 10px horizontal padding (content stays aligned, the highlight overhangs 10px), bottom padding 6px; radius 14px; background/shadow transition 120ms `ease` |
| Section drop highlight | background `--archive-drop` (L `rgba(0,122,255,0.08)` / D `rgba(10,132,255,0.12)`), inset ring 1.5px L `rgba(0,122,255,0.55)` / D `rgba(10,132,255,0.55)` |
| Group section | 20px above it |
| Heading row | row aligned on text baseline, gap 10px, margin `0 0 22 6` |
| Top heading | `Projects` / `In Focus`: `--font-display` 26px weight 600, letter-spacing −0.52px, `--text-primary`, line height 36.4px |
| Heading count | 15px `--text-tertiary` tabular |
| Group heading row | as heading row but 14px below and 8px padding above; count 13px |
| Group toggle | inline row, centred, gap 4px, left margin −6px, padding `2 8 2 3`, radius 6, transparent; hover `--bg-hover`, pressed `--bg-pressed`; chevron `chevron-down` 16px stroke 2.2 `--text-tertiary`, rotation 0 (expanded) / −90° (collapsed), transform 120ms `--ease-out`; name `--font-display` 20px weight 600, letter-spacing −0.2px, `--text-primary`, single line, ellipsis; native tooltip `Hide projects` / `Show projects` |
| Group rename field | width 260px, left margin −4px, padding `1 6`, no border, radius 6, `--bg-elevated`, rings `0 0 0 1px --accent` + `0 0 0 4px --focus-ring` (a 3px focus band outside a 1px accent line), `--font-display` 20px weight 600, −0.2px, no outline; accessible name `Group name` |
| Group / take-out placeholder | centred text; height 72px; margin `0 6 4`; 1.5px dashed `--border-strong` border; radius 10; 12px `--text-tertiary` |
| `All projects are archived.` | margin-left 6px; 13px `--text-tertiary` |
| Archived heading row | 36px above, 16px below, 18px padding above, top hairline 0.5px `--separator` (starting at the 6px heading indent); `Archived` `--font-display` 15px weight 600 `--text-secondary`; count 13px `--text-tertiary` |
| Grid | columns of 144px, as many as fit (left-aligned), column gap 10px, row gap 14px; items top-aligned |
| Focus note | margin `28 0 0 6`; padding-top 16px; top hairline 0.5px `--separator`; 13px `--text-tertiary` |

Folder item:

| Part | Value |
| --- | --- |
| Item | 144px wide; column, centred; padding `2 4 6`; radius 10; no focus ring |
| Icon box | 124×116, content centred, radius 10; transitions background 120ms `ease`, transform 120ms `--ease-out`; selected: `--bg-pressed` (L `rgba(0,0,0,0.09)` / D `rgba(255,255,255,0.12)`); item pressed: scale 0.97 |
| Folder | FolderIcon 112px, project colour, glyph = UI-295; transform transition 220ms `--ease-out`; item hovered: translateY −1px |
| Faded folder | archived or dimmed: opacity 0.42 and saturation 0.4 (opacity transition 120ms `ease`); hovered or selected: opacity 0.7 |
| Focus mark | at (2, 18) inside the icon box (top-left); 20×20 circle; `--bg-elevated`; `--shadow-md`; icon `target` 12px stroke 2.4 in `--accent`; native tooltip `In focus` |
| Badges | column at the icon box's top-right (top 18px, right 2px), right-aligned, gap 4px; waiting pill first |
| Badge pill | inline row, centred, gap 5px; height 20px; padding `0 7 0 6`; radius 10; `--bg-elevated`; `--shadow-md`; 11px weight 600 tabular `--text-primary`; dot 7×7 (waiting: pulse animation kept; working: no ring, no animation); native tooltips `N waiting for you`, `N working` |
| Label | 4px below the icon box; max width 136px; centred. Text 13px weight 500, line height 1.3 (16.9px), padding `1 6`, radius 5, at most 2 lines with an ellipsis, breaking long words. The pill hugs the text width for one line and spans up to 136px when wrapped |
| Label selected | `--accent` background, `--text-on-accent` text |
| Label faded | archived or dimmed and not selected: `--text-secondary` |
| Rename field | width 136px, 4px below the icon box, padding `1 4`, no border, radius 4, `--bg-elevated`, rings `0 0 0 1px --accent` + `0 0 0 4px --focus-ring`, 13px weight 500, line height 1.3, centred text, single line, no outline; accessible name `Project name` |
| Subtitle | 2px below; 11px `--text-tertiary` tabular; single line |
| New folder pop | whole item: opacity 0→1, scale 0.6→1, 520ms `--ease-spring` (once) |
| Tooltip | native, the repository path |

Empty state (no projects):

| Part | Value |
| --- | --- |
| Box | fills the view; column, centred both ways, gap 6px; padding 32px, bottom padding 12% of the window height; centred text |
| Icon | FolderIcon 148px, blue, `folder-plus` glyph; opacity 0.55; 10px extra below |
| Title | `Add your first project`, `--font-display` 20px weight 600, −0.2px |
| Text | `Link a local repository to organize its Claude sessions on a board.`, max width 360px, 13px `--text-secondary`, 14px below |
| Button | `.btn .btn-primary` `Choose Folder…` |
| Hint | `or drop a folder anywhere in this window`, 6px extra above, 12px `--text-tertiary` |

Folder drop overlay:

| Part | Value |
| --- | --- |
| Overlay | inset 12px from the view's edges, z 20; 2px dashed `--accent` border; radius 18; background `--archive-drop`; 2px backdrop blur; ignores the pointer; fades in 120ms `ease` |
| Content | column, centred, gap 12px; FolderIcon 88px blue with `plus` glyph; `Drop a folder to add it as a project` 15px weight 600 `--text-primary` |

Toast:

| Part | Value |
| --- | --- |
| Box | 24px above the view's bottom, horizontally centred, z 30; max width min(520px, view width − 48px); padding `9 14`; radius 10; `--bg-elevated`; `--shadow-lg`; 13px text |
| Enter | opacity 0→1 and 8px lower → in place, 220ms `--ease-out`; no exit animation |

Context menus on the grid (icons 14px in the menu's icon slot):

| Menu | Items, in order |
| --- | --- |
| Project (active) | `Open` (`folder-open`, hint `⌘O` / `Ctrl+O`) · `Rename` (`pencil`, hint `↩` / `Enter`) · [focusFeature only] `Add to Focus` or `Remove from Focus` (`target`) · separator · colour swatches (`Folder colour`) · separator · `Move to Group…` (`folder-input`) · `Reveal in Finder` / `Show in File Explorer` (`search`) · separator · `Archive` (`archive`) · `Remove from Nodus…` (`trash-2`, danger) |
| Project (archived) | `Open` · `Rename` · separator · swatches · separator · `Reveal in Finder` / `Show in File Explorer` · separator · `Unarchive` (`archive-restore`) · `Remove from Nodus…` |
| Group heading | `Rename Group` (`pencil`) · separator · `Delete Group` (`trash-2`, danger) |

Remove confirmation:

| Part | Value |
| --- | --- |
| Panel | Modal 440px wide |
| Body | row, gap 16px, padding `22 22 8`: FolderIcon 56px (project colour, no glyph), then a column |
| Title | `Remove “<name>”?` 15px weight 600 (UI font), margin `4 0 6` |
| Text | 13px `--text-secondary`, margin-bottom 8px |
| Path | `--font-mono` 11px `--text-tertiary`, single line, ellipsis, selectable |
| Actions | row, right-aligned, gap 8px, padding `12 22 18`: `Cancel` (`.btn`), `Remove` (`.btn` with fill `--status-waiting`, transparent border, white text; hover L `#e0342a` / D `#e03d33`; has focus on open) |

### Group picker

| Part | Value |
| --- | --- |
| Panel | Modal 340px wide, padding `18 16 16` |
| Title | `Move “<name>” to`, 15px weight 600, margin `0 4 12`, single line, ellipsis |
| List | only when groups exist; column, gap 1px, max height 260px then scrolls, 12px below |
| Row | row, centred, gap 9px, height 32px, padding `0 10`, radius 6, transparent, 13px, left-aligned; icon colour `--text-secondary`; hover `--bg-hover`, pressed `--bg-pressed`; current `--bg-selected` (also while hovered or pressed) |
| Row icon | groups `folders` 15px stroke 2; `No Group` `folder-minus` 15px stroke 2 |
| Row name | fills the row, `--text-primary`, single line, ellipsis |
| Current check | `check` 15px stroke 2.4, `--accent` |
| New-group form | row, gap 8px: input + `.btn .btn-primary` `Create` |
| Input | fills the row; height 28; padding `0 9`; border 0.5px `--border-strong`; radius 6; `--bg-sunken`; 13px; no outline; placeholder `New group name` (unclear: platform default placeholder colour, `GroupPicker.css:65-79` sets none); focused: border `--accent` and ring `0 0 0 3px --focus-ring` |

### Project rail

| Part | Value |
| --- | --- |
| Edge strip (not pinned) | invisible; at the main area's left edge, full main-area height, 6px wide; z 41 |
| Rail (floating) | z 40; 52px wide; left 8px; vertically centred in the main area; height to content, at most main-area height − 16px (then scrolls, scrollbar hidden, no horizontal overflow); column, items centred, gap 4px; padding `6 0`; radius 14; background L `rgba(255,255,255,0.82)` / D `rgba(42,42,45,0.82)` over a 24px backdrop blur with saturation 1.8; `--shadow-lg` |
| Hidden | opacity 0; translateX −60px; transitions transform 220ms `--ease-out` and opacity 220ms `ease` |
| Shown | opacity 1; no translate |
| Numbered | z 600 |
| Pinned | left 0, full main-area height (top to bottom), not centred; padding `8 0`; right hairline 0.5px `--separator`; no radius; `--bg-window`; no blur, no shadow, no transitions |
| Item | 40×40, content centred, radius 10, transparent, not shrinkable; background transition 120ms `ease`; hover `--bg-hover`; pressed `--bg-pressed`; open project `--bg-selected` (L `rgba(0,122,255,0.14)` / D `rgba(10,132,255,0.24)`), which also holds while it is hovered or pressed; accessible name = project name |
| Folder | FolderIcon 34px (drop shadow on, glyph = UI-295) |
| Open-project pill | 3×18px, radius `0 3 3 0`, `--accent`, its left edge 6px left of the item (on the rail's left edge), vertically centred |
| Unfocused folder | opacity 0.45, saturation 0.4 |
| Separator | 24×0.5px `--border-strong`, margin `2 0` |
| `+` button | an item; icon `plus` 18px stroke 2; `--text-secondary`, hover `--text-primary`; name `New Project` |
| Status dot | at top 4px, right 3px of the item; 9×9 circle including a 1.5px `--bg-elevated` border (fill area 6×6); waiting/working animations as `.status-dot` (rings outside the border) |
| Number badge | bottom-right of the item (right 0, bottom 1px); min width 16px; padding `0 4`; border 0.5px `--border-strong`; radius 5; `--bg-elevated`; `--shadow-sm`; `--font-ui` 10.5px weight 600 tabular, line height 15px (badge 16px tall), centred, `--text-primary`; ignores the pointer; enter: opacity 0→1, scale 0.7→1, 120ms `--ease-out` |
| Hint | z 60; left = 66px from the window's left (pinned: 58px), vertically centred on the hovered item; column; max width 260px; padding `5 9`; radius 6; `--bg-elevated`; `--shadow-md`; ignores the pointer; enter: opacity 0→1 and 3px further left → in place, 120ms `--ease-out` |
| Hint name | 12px weight 600 `--text-primary`, single line, ellipsis |
| Hint detail | 11px `--text-secondary` |
| Rail menu | ContextMenu with `Add to Focus` / `Remove from Focus` (`target`) |

## Keyboard & mouse

"⌘" means Ctrl off a Mac unless stated. Menu accelerators are owned by 01; this domain owns what the UI does when they fire.

| Where | Input | Effect | IDs |
| --- | --- | --- | --- |
| Anywhere | ⌘, (menu Settings…) | open Settings | UI-039, UI-116 |
| Anywhere | ⌘W (menu Close) | close the top layer, else board → grid; never the window | UI-040, UI-041 |
| Anywhere | ⇧⌘N (menu New Project…) | folder picker → add project → grid with it selected | UI-046, UI-292 |
| Anywhere | ⇧⌘F (menu View › Focus Mode) | flip focus mode (only with focusFeature on) | UI-047 |
| Anywhere, off a Mac | Ctrl+N / Ctrl+T / Ctrl+W outside a terminal | New Session / New Terminal / Close | UI-042 |
| Anywhere | drop OS files outside a drop target | nothing | UI-013 |
| Title bar | press-drag on empty area | move window | UI-014 |
| Title bar | double-click empty area | OS zoom/maximize | UI-014 |
| Title bar (board) | click ‹ or `Projects` | go to grid | UI-091 |
| Title bar (board) | double-click project crumb | reveal folder | UI-094 |
| Title bar | point at `N running` | per-project breakdown | UI-105 |
| Title bar (off Mac) | click ☰ | app menu popup | UI-099 |
| Title bar | click Focus label or switch | toggle focus mode | UI-097 |
| Settings | Esc | close | UI-121 |
| Settings | Tab / ⇧Tab | cycle inside the dialog | UI-123 |
| Settings | arrows on a focused theme tile | change theme | UI-125 |
| Settings | Space on a focused switch; click anywhere on a switch row | toggle | UI-018, UI-138 |
| Settings | Enter in the key field | save key (if non-blank) | UI-142 |
| Any Modal | Esc (unless opted out) / press on backdrop | close | UI-155, UI-156 |
| Context menu | click item | close, then run | UI-162 |
| Context menu | Esc / press outside / window blur / window resize | close | UI-160 |
| Context menu | arrow keys | nothing (no arrow navigation; items are plain buttons) | — |
| Grid | press on folder (primary) | select | UI-202 |
| Grid | press on empty area | deselect | UI-203 |
| Grid | double-click folder | open board | UI-207 |
| Grid | ⌘O · ⌘↓ | open selected | UI-208 |
| Grid | ⌘⌫ | remove selected (asks) | UI-209 |
| Grid | ↩ | rename selected | UI-210 |
| Grid | Esc | deselect | UI-211 |
| Grid | ← → ↑ ↓ (Shift ignored) | move selection | UI-212…UI-214 |
| Grid | right-click folder / group heading | menu | UI-216, UI-221 |
| Grid | click group name | collapse/expand | UI-191 |
| Grid | drag folder onto another section | move to group | UI-224…UI-226 |
| Grid | drop folders from Finder/Explorer | add projects | UI-228, UI-229 |
| Rename field | ↩ / blur · Esc | commit · cancel | UI-238 |
| Group picker | click row · ↩ in name field | move · create and move | UI-244, UI-245 |
| Remove dialog | ↩ / Space (Remove focused) · Esc | remove · cancel | UI-248, UI-250 |
| Board (rail) | rest pointer on left edge 100ms | slide rail in | UI-262 |
| Board (rail) | leave rail/edge | slide out after 300ms | UI-264 |
| Board (rail) | click item · click `+` | open board · add project and open | UI-260, UI-261 |
| Board (rail) | right-click item (focusFeature) | Add/Remove from Focus | UI-274 |
| Board | hold ⌘ 300ms | number badges, rail out | UI-275 |
| Board | ⌘1…⌘9 (top row or numpad, by key position) | open Nth rail project | UI-277 |

## Platform differences

| Topic | macOS | Windows (and any non-Mac) |
| --- | --- | --- |
| Window chrome | hidden-inset title bar; traffic lights at (18, 18); title bar left padding 84px | hidden title bar with a caption-button overlay drawn over the title bar's right end: 52px tall, background L `#f6f6f5` / D `#252527`, symbols L `#1f1f1f` / D `#e6e6e6`, updated on theme change; title bar left padding 8px, right padding 14px + overlay width (fallback 138px) |
| App menu | the macOS menu bar (01) | no menu bar; the ☰ button at the title bar's left pops the same menu up at (button left, button bottom + 4); its File menu holds Settings, Check for Updates… and Quit and Stop Sessions (01) |
| Crumb placement | crumbs at x = 84, spread `space-between` with the right cluster | crumbs pinned right after ☰ (8px gap) |
| Shortcut modifier | ⌘ | Ctrl; Ctrl+N/T/W handled by the window outside terminals (UI-042); ⌘ digit becomes Ctrl+digit and is ignored when a terminal has focus (UI-276) |
| Shortcut text | `⇧⌘N`, `⌘O`, `↩`, `⌘1` | `Ctrl+Shift+N`, `Ctrl+O`, `Enter`, `Ctrl+1` (UI-303) |
| Reveal | `Reveal in Finder`; crumb tooltip `Double-click to reveal in Finder` | `Show in File Explorer`; `Double-click to show in File Explorer` |
| Settings › Appearance hint | `System follows the appearance set in macOS.` | `System follows the appearance set in Windows.` (also on Linux) |
| Settings › Notifications | includes `Count sessions that need me on the Dock icon` | no Dock row |
| Settings › Voice | `Saved in your Keychain.` | `Saved encrypted for your Windows account.` |
| Settings › Updates | `They install when you quit Nodus. Sessions keep running.` | `They install when you choose Restart to Update, which stops the sessions.` |
| Update button tooltip | `Nodus <v> is ready. Sessions keep running while Nodus restarts.` | `Nodus <v> is ready. Restarting stops the sessions; you can resume them after.` |
| Key-save error | `The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` | `Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` |
| Fonts | SF Pro / SF Mono | Segoe UI Variable Text/Display (else Segoe UI) / Cascadia Mono (else Consolas) |
| Accessibility row | shows when the double tap needs the permission | the state is never `needs-permission` in practice (07) |

## Edge cases & errors

- **No projects, before load**: the grid shows nothing (not the empty state) until the first refresh ends (UI-184).
- **Every project archived**: top heading `Projects` with count `0`, `All projects are archived.`, then the Archived section (UI-194).
- **Grouped projects only**: the top heading and count still show, with nothing under them (no placeholder unless a grouped folder is being dragged, UI-227).
- **Focus on, nothing in focus**: every project shows normally plus the note `Focus mode is on, but no project is in focus yet. Right-click a project and choose Add to Focus.` (UI-199); the title-bar switch reads on.
- **Focusing hides everything in the top section**: heading `In Focus` with its count and nothing under it.
- **Hidden-count wording**: `1 other project is hidden by focus mode.` / `3 other projects are hidden by focus mode.` (counts archived projects too, UI-198).
- **Group that no longer exists**: a project pointing at it is ungrouped everywhere, and the picker marks `No Group` current (UI-243, UI-286).
- **Deleting a group**: immediate, no confirmation; its projects become ungrouped (UI-223).
- **Group named with only spaces**: the rename is ignored (UI-238); the core also falls back to `New Group` for a blank create (01/03), which the UI never sends.
- **Add folder failures**: `Couldn’t add “<last path segment>”. Only folders can be projects.` (dropped/given path) and `Couldn’t add that folder.` (picker) as grid toasts (UI-290). The last path segment is split on `/` only (a Windows path with `\` shows whole; reproduce).
- **Adding an existing folder**: no error; the existing project is unarchived (and joins the focus while focusing), selected, and shown (UI-289, UI-292).
- **Picker cancelled**: nothing happens (UI-291).
- **Toast raised off the grid**: visible only if the grid opens within 3500ms (UI-252).
- **Selected project removed elsewhere**: selection clears (UI-205).
- **Stats not loaded**: folder subtitle keeps its line height with a non-breaking space; no badges (UI-233).
- **Project name with no letter or digit** (e.g. `---`): folder without glyph (UI-295). unclear: `FolderIcon.tsx:156` takes the first UTF-16 unit and upper-cases it, so a letter outside the BMP renders a broken glyph and `ß` renders `SS`; the port should take the first character (code point) and upper-case it — identical for every BMP letter except the `ß`-style multi-letter upper-casings, which should be kept as the full upper-case string to match.
- **Title bar before the board loads**: name from the project list; empty name with a blue folder if the id is unknown (UI-092).
- **Context menu taller than the window**: it is moved up so its bottom is 8px above the window bottom, and may then start above the window's top edge (no top clamp, UI-158).
- **Settings before load**: theme tiles and switches disabled; `Check Now` shown disabled; version row reads `Nodus` (UI-124, UI-147, UI-149).
- **Update state `off`**: the reason string from the updater (06) is the version row's hint; no Check Now and no auto-update row (UI-148…UI-150).
- **Key save error**: verbatim from the core (see Platform differences); shown until the next save (UI-145). The Electron IPC prefix `Error invoking remote method '…': ` and an optional `Error: ` are stripped; the Rust core should return the bare message.
- **Key with surrounding spaces**: Save is enabled for any non-blank text; the core trims before storing.
- **Rail with no non-archived projects**: only the `+` button, no separator (UI-256).
- **⌘ digit beyond the rail's count**: nothing; the key passes through (UI-277).
- **⌘ digit for the open project**: consumed, nothing changes (UI-277).
- **Rail while dragging on the board**: never reveals while a mouse button is held (UI-263).
- **Notification click while Settings is open**: Settings closes (UI-045).
- **Archived session's terminal open when the board reloads**: the terminal modal closes (UI-052).
- **Running count with only shells**: hidden (UI-102, UI-103).
- **Selected folder not in reading order** (its group is collapsed, or focus hides it): arrows restart at the first folder (UI-212) and Enter, ⌘O, ⌘↓ and ⌘⌫ do nothing.
- **Settings links and the Tab trap**: links are not counted as tab stops by the trap (UI-123). `console.x.ai` sits between counted stops so Tab reaches it; the error-state `download the latest version` link follows the last counted stop (the auto-update switch), so Tab wraps before reaching it and it is reachable only with the pointer. Reproduce.
- **Enter on a focused title-bar button while a folder is selected**: the grid's Enter handler (window level, UI-210) takes the key and starts a rename instead of activating the button (unclear: `ProjectsView.tsx:186-211` only skips text fields). Reproduce.

## Tests to port

Owned test file `src/renderer/src/components/projects/focus.test.ts` → Rust `nodus-domain` (or wherever `focus` lands, see Rust mapping). Fixture: `project(id, extra)` = `{ id, name: id, repoPath: "/repos/<id>", color: blue, createdAt: 0, ...extra }`; `stats(waiting)` = `{ total: 1, active: 1, working: 0, waiting, running: { sessions: 0, background: 0, terminals: 0 } }`.

| Suite › case | Assertions |
| --- | --- |
| focusModeOn › is on only while focus is offered in Settings and switched on | `{focusFeature: true, focusMode: true}` → true; `{true, false}` → false; `null` → false |
| focusModeOn › stays off while focus is turned off in Settings, whatever the switch says | `{focusFeature: false, focusMode: true}` → false |
| isFocusing › is off while focus mode is off, even with projects in focus | `[a(focused)]`, `false` → false; `[a(focused)]`, `undefined` → false |
| isFocusing › narrows the lists once focus mode is on and a project is in focus | `[a(focused), b]`, `true` → true |
| isFocusing › shows everything when nothing is in focus | `[a, b]`, `true` → false |
| isFocusing › ignores archived projects in focus | `[a(focused, archived), b]`, `true` → false |
| showsInFocus › shows the projects in focus | `a(focused)` → true |
| showsInFocus › hides the others | `a` → false; `a`, `stats(0)` → false |
| showsInFocus › shows a project out of focus while a session in it waits on you | `a`, `stats(1)` → true |
| showsInFocus › shows the project whose board is open | `a`, none, open `"a"` → true; `a`, none, open `"b"` → false |

Also used by this domain (port with the RunningCount code; `src/renderer/src/components/runningSessions.test.ts`). Fixture `stats(running)` = all counts 0 with `running = { sessions: 0, background: 0, terminals: 0, ...running }`; project names are arbitrary (the source uses three sample names; here `p1`, `p2`, `idle`).

| Suite › case | Assertions |
| --- | --- |
| runningTotal › counts Claude sessions in every project, hidden ones too, but not shells | `{a: {sessions 2, terminals 3}, b: {background 1}}` → 3 |
| runningTotal › is zero with nothing running | `{}` → 0; `{a: {terminals 1}}` → 0 |
| runningByProject › gives each project with Claude sessions running and its count, in project order | projects `[a p1, b p2, c idle]`, stats `{b: {sessions 1}, a: {sessions 2, terminals 1}, c: {}}` → `["p1 (2)", "p2 (1)"]` |
| runningByProject › counts a project’s hidden sessions with its cards | `{a: {sessions 1, background 2}}` → `["p1 (3)"]` |
| runningByProject › leaves out a project that’s only running shells | `{a: {terminals 2}, b: {sessions 1}}` → `["p2 (1)"]` |

Also used by this domain (`src/renderer/src/platform.test.ts`; 01 may list it too — port once, in `nodus-ui::platform` or `nodus-platform`):

| Suite › case | Assertions |
| --- | --- |
| keys › writes a Mac shortcut as Windows does | platform `Win32` → `windows`; `⇧⌘F` → `Ctrl+Shift+F`; `⌘↩` → `Ctrl+Enter`; `⌘` → `Ctrl`; `⌫` → `Del`; `↩` → `Enter` |
| keys › leaves it be on a Mac | platform `MacIntel` → `mac`; `⇧⌘F` → `⇧⌘F` |
| modKey › is ⌘ on a Mac and Ctrl elsewhere | Mac: `{meta: true, ctrl: false}` → true; Windows: same → false; `{meta: false, ctrl: true}` → true; `otherModKey({meta: true, ctrl: false})` → true |
| shortcutKey › reads a Latin layout’s letter as it is | (`c`, 67) → `c`; (`W`, 87) → `w`; (`a`, 65) → `a` |
| shortcutKey › reads the letter from the key code on another alphabet’s layout | (`ر`, 86) → `v`; (`с`, 67) → `c` |
| shortcutKey › leaves keys that aren’t letters be | (`Enter`, 13) → `enter`; (`é`, 50) → `é` |

The pure helpers in `store.ts` (UI-081…UI-087) and `projectsUi.ts` (UI-285, UI-286, UI-295, `sortProjects` UI-187) have no Vitest cases in 0.9.4; their behaviour IDs are the acceptance criteria.

## Rust mapping

Proposed `nodus-ui` modules (only `nodus-ui` and the `nodus` binary know GPUI):

| Module | Ports | Notes |
| --- | --- | --- |
| `theme::tokens` | tokens.css | `struct Tokens` with one `const LIGHT` and one `const DARK` (every row of the colour, shadow, radius, motion and layout tables); `Theme::current(cx)` picks by effective appearance |
| `theme::appearance` | `applySettings` + `nativeTheme` | resolves UI-001/002; observes the OS appearance (`window.appearance()` / `observe_window_appearance`); sets the native appearance for menus/dialogs (macOS `NSApp.appearance`; Windows immersive dark mode for the popup menu) and the window background (UI-003…005) |
| `theme::motion` | easings, durations | cubic-bezier solver for `--ease-out`, `--ease-spring`, `ease`, `ease-in-out`, `ease-out`; piecewise-linear `--ease-stack`; a `reduced_motion()` flag from the OS (UI-012) |
| `components::{button, switch, status_dot, modal, context_menu, color_swatches, split_button, folder_icon, usage_stats, rename_field, tooltip, scrollbar}` | global.css primitives and the shared widgets | used by 05, 07, 09–12 |
| `shell::{app_root, title_bar, running_count, layers}` | App.tsx, TitleBar*, RunningCount | `layers` owns the ⌘W order (UI-040) and the app-level overlay slots; title bar exposes a "view actions" slot instead of a DOM portal (UI-098) |
| `settings` | SettingsDialog | polling capture state every 2s while open (UI-132) |
| `projects::{grid, folder_item, group_picker, remove_dialog, rail, ui_state, actions}` | ProjectsView, FolderItem, GroupPicker, ProjectRail, projectsUi.ts | `ui_state` = the `useProjectsUi` store (selection + toast, global, survives navigation) |
| `state` | store.ts | `AppState` GPUI entity: route, projects, groups, stats, global actions, board, flags; actions call `nodus-core` commands; subscriptions to core events (UI-033…038) |
| `nodus-domain::focus` | focus.ts | pure; shared with the core's add-project (UI-300); tests above |
| `nodus-domain` (or `nodus-ui::format`) | `formatDuration`, `tildePath`, `cardTitle`, `byUrgency`, `agentStep`, `agentStatusLabel`, `nestAgents`, `runningTotal`, `runningByProject`, `sortProjects`, `groupProjects`, `projectGlyph` | pure, unit-testable |

gpui-component candidates (restyle to the tokens; verify every pixel):

- Fits with restyling: `Input` (rename fields, group name, xAI key with masking; needs IME and select-all-on-focus), `Switch` (if its geometry can be set to 32×18 / 14px knob; else custom), `Radio` (theme tiles need a fully custom look; may be simpler as custom focusable tiles), `Scrollbar`/scrollable containers (custom thumb metrics: 4px pill in a 10px gutter, always visible while overflowing, plus a hidden-scrollbar variant for the rail), `Tooltip` (for the custom hint-like cards).
- Custom: title bar, folder grid (Finder-like geometry and keyboard), rail, context menu (exact metrics, backdrop blur, anchor semantics), modal (backdrop blur + no exit animation), FolderIcon, status dot animations, mini-window previews, split button.

Risks:

1. **Backdrop blur.** Context menu (30px, saturate 1.8), floating rail (24px, saturate 1.8), modal backdrop (3px), drop overlay (2px) blur what is behind them. GPUI has no per-element backdrop filter; needs a custom Metal/DirectX pass or an agreed approximation (the title bar's blur is invisible and resolves to a solid colour). Decide in spike S1 and record the acceptance rule.
2. **FolderIcon.** Gradients + feDropShadow + embossed text/icon can't go through GPUI's monochrome `svg()`. Rasterize the SVG template above (resvg/tiny-skia) per (colour, size, glyph, scale factor) and cache, or draw with GPUI paths + gradients and a separate shadow; text must use the same system font/weight.
3. **Shadow and outline primitives.** Inset rings (switch off-state, swatches, section drop target), outlines with offset (focus rings, theme tile 5px offset), 0.5px hairline rings, multi-layer shadows and dashed borders need verification in GPUI; fallbacks: inner borders and absolutely positioned ring overlays.
4. **Native tooltips and Windows chrome.** HTML `title` tooltips are OS-native in 0.9.4; GPUI tooltips are drawn by the app. Windows: the Electron caption-button overlay (colours, hover/pressed states, 138px width) and the ☰ native popup menu (`Menu.popup`) must be reproduced (custom caption buttons + Win32 `TrackPopupMenu` or equivalent).
5. **Text metrics and typography.** Letter-spacing (−0.01em/−0.02em), two-line clamp with ellipsis, `tnum`, `-webkit-box` label pill sizing, line heights of 1.3/1.4 and Chromium's text layout vs CoreText/DirectWrite in GPUI must match side by side; `:focus-visible` heuristics (keyboard vs mouse) need an explicit "last input was keyboard" flag.
6. **Drag and drop.** Internal folder drags (section highlight rules, drag preview = browser default snapshot) and OS file drops with enter/leave depth counting map to GPUI's `on_drag`/`on_drop` and external-paths drops; the browser's drag image and drop-effect cursors are unclear targets.
7. **Global key handling.** Key-position digits (`e.code` Digit/Numpad), held-modifier timing (300ms), keyup semantics and capture-phase Escape for menus must be rebuilt on GPUI's keystroke/modifier-change events; the "keys don't leak out of Settings" rule (UI-120) needs focus-scoped dispatch.

## Parity checklist

- [x] UI-001 Effective appearance
- [x] UI-002 Live OS appearance
- [x] UI-003 Window background
- [x] UI-004 Theme before first paint
- [x] UI-005 Native chrome follows the theme
- [x] UI-006 No text selection by default
- [x] UI-007 Inputs select text
- [x] UI-008 Selectable path
- [x] UI-009 Arrow cursor
- [x] UI-010 Keyboard focus ring
- [x] UI-011 Scrollbars
- [x] UI-012 Reduced motion
- [x] UI-013 Stray file drops
- [x] UI-014 Title bar drags the window
- [x] UI-015 Button states
- [x] UI-016 Primary button
- [x] UI-017 Ghost button
- [x] UI-018 Switch
- [x] UI-019 Working dot
- [x] UI-020 Done dot
- [x] UI-021 Waiting dot
- [x] UI-022 Small dot
- [x] UI-023 Starts on the grid
- [x] UI-024 Startup loads
- [x] UI-025 Root layout
- [x] UI-026 Board route composition
- [x] UI-027 Pinned rail column
- [x] UI-028 Board remount per project
- [x] UI-029 Rail survives board switches
- [x] UI-030 App-level overlays
- [x] UI-031 Navigate resets
- [x] UI-032 Navigate loads
- [x] UI-033 Stats refresh
- [x] UI-034 Board changed event
- [x] UI-035 Projects changed event
- [x] UI-036 Status event
- [x] UI-037 Agent status event
- [x] UI-038 Usage event
- [x] UI-039 Settings command
- [x] UI-040 ⌘W closes the top layer
- [x] UI-041 ⌘W never closes the window
- [x] UI-042 Ctrl shortcuts off a Mac
- [x] UI-043 Notification opens its card
- [x] UI-044 Launch open request
- [x] UI-045 Show card
- [x] UI-046 New project from anywhere
- [x] UI-047 Toggle focus command
- [x] UI-048 What persists
- [x] UI-049 Refresh projects
- [x] UI-050 Board loading flag
- [x] UI-051 Stale board loads
- [x] UI-052 Archived terminal closes
- [x] UI-053 Patch cards: guards
- [x] UI-054 Patch cards: empty sessions leave
- [x] UI-055 Patch cards: merge
- [x] UI-056 Patch cards: archiving
- [x] UI-057 Patch cards: persist
- [x] UI-058 Upsert card
- [x] UI-059 Remove card
- [x] UI-060 Notes fall off
- [x] UI-061 Archive rect
- [x] UI-062 Save section
- [x] UI-063 Remove section
- [x] UI-064 Save note
- [x] UI-065 Remove notes
- [ ] UI-066 Save ink
- [x] UI-067 Upsert view
- [x] UI-068 Patch view
- [x] UI-069 Remove view
- [x] UI-070 Status on a card
- [x] UI-071 Stopped session's agents
- [x] UI-072 Agent status
- [x] UI-073 Usage
- [x] UI-074 Git read sharing
- [x] UI-075 Git result
- [x] UI-076 Terminal vs preview
- [x] UI-077 Agents and diff flags
- [x] UI-078 Action lists
- [x] UI-079 Optimistic settings
- [x] UI-080 Settings survive navigation
- [x] UI-081 Duration format
- [x] UI-082 Tilde path
- [x] UI-083 Card title
- [x] UI-084 Agent urgency order
- [x] UI-085 Agent step line
- [x] UI-086 Agent status label
- [x] UI-087 Nested agents
- [x] UI-088 Always there
- [x] UI-089 Grid crumb
- [x] UI-090 Board crumbs
- [x] UI-091 Back to grid
- [x] UI-092 Project name source
- [x] UI-093 Project crumb tooltip
- [x] UI-094 Reveal from the crumb
- [x] UI-095 Right cluster order
- [x] UI-096 Focus switch visibility
- [x] UI-097 Focus switch toggles
- [x] UI-098 View action slot
- [ ] UI-099 Menu button off a Mac
- [x] UI-100 Crumbs hug the menu button
- [x] UI-101 Long names
- [x] UI-102 Hidden at zero
- [x] UI-103 What counts
- [x] UI-104 Label
- [x] UI-105 Breakdown on hover
- [x] UI-106 Shown when ready
- [x] UI-107 Update tooltip
- [x] UI-108 Update click
- [x] UI-109 Nothing until reported
- [x] UI-110 Model
- [x] UI-111 Context meter
- [x] UI-112 Context tooltip
- [x] UI-113 Running time
- [x] UI-114 Cost
- [x] UI-115 Token format
- [x] UI-116 Opening
- [x] UI-117 Dialog frame
- [x] UI-118 Reload on open
- [x] UI-119 Focus in and back
- [x] UI-120 Keys stay inside
- [x] UI-121 Escape closes Settings
- [x] UI-122 Backdrop closes Settings
- [x] UI-123 Tab stays inside
- [x] UI-124 Disabled until loaded
- [x] UI-125 Theme tiles
- [x] UI-126 Theme tile states
- [x] UI-127 Theme previews
- [x] UI-128 Appearance hint
- [x] UI-129 Pin sidebar row
- [x] UI-130 Focus row
- [x] UI-131 Capture switch
- [x] UI-132 Capture state polling
- [x] UI-133 Accessibility row
- [x] UI-134 Unavailable row
- [x] UI-135 Try row
- [x] UI-136 Notify row
- [x] UI-137 Dock badge row
- [x] UI-138 Whole row toggles
- [x] UI-139 Key field loading
- [x] UI-140 Key placeholder
- [x] UI-141 Save enabled
- [x] UI-142 Save key
- [x] UI-143 Key hint by source
- [x] UI-144 Remove key
- [x] UI-145 Key error
- [x] UI-146 Key privacy hint
- [x] UI-147 Version row
- [x] UI-148 Update status text
- [x] UI-149 Update button in Settings
- [x] UI-150 Auto-update row
- [x] UI-151 Download link on error
- [x] UI-152 Links open in the browser
- [x] UI-153 Modal frame
- [x] UI-154 Modal enter
- [x] UI-155 Modal Escape
- [x] UI-156 Modal backdrop press
- [x] UI-157 Modal size limits
- [x] UI-158 Menu position
- [x] UI-159 Right-aligned menu
- [x] UI-160 Menu closes
- [x] UI-161 Anchor presses
- [x] UI-162 Choosing an item
- [x] UI-163 Item states
- [x] UI-164 Item hint
- [x] UI-165 Separators and custom rows
- [x] UI-166 No native menu over a menu
- [x] UI-167 Menu enter
- [x] UI-168 Swatch row
- [x] UI-169 Current swatch
- [x] UI-170 Swatch hover
- [x] UI-171 Pick a colour
- [x] UI-172 Main action
- [x] UI-173 Chevron toggles on press
- [x] UI-174 Chevron from the keyboard
- [x] UI-175 Open state
- [x] UI-176 Split button look
- [x] UI-177 Folder drawing
- [x] UI-178 Colour fallback
- [x] UI-179 Small folders
- [x] UI-180 Letter glyph
- [x] UI-181 Icon glyph
- [x] UI-182 Theme-independent hues
- [x] UI-183 Loads on show
- [x] UI-184 Empty state timing
- [x] UI-185 Empty state
- [x] UI-186 New Project button
- [x] UI-187 Recent order
- [x] UI-188 Top section
- [x] UI-189 Heading count
- [x] UI-190 Group sections
- [x] UI-191 Collapse a group
- [x] UI-192 Empty group
- [x] UI-193 Archived section
- [x] UI-194 All archived
- [x] UI-195 Focus filtering
- [x] UI-196 Focus hides archived
- [x] UI-197 Focus hides empty groups
- [x] UI-198 Hidden-count note
- [x] UI-199 Nothing-in-focus note
- [x] UI-200 Dimmed folders
- [x] UI-201 Focus mark
- [x] UI-202 Select on press
- [x] UI-203 Deselect on empty press
- [x] UI-204 Selection persists
- [x] UI-205 Stale selection
- [x] UI-206 Keep selection visible
- [x] UI-207 Double-click opens
- [x] UI-208 Open shortcut
- [x] UI-209 Remove shortcut
- [x] UI-210 Enter renames
- [x] UI-211 Escape deselects
- [x] UI-212 Arrows start at the first
- [x] UI-213 Left/Right
- [x] UI-214 Up/Down
- [x] UI-215 Keys ignored when busy
- [x] UI-216 Project menu
- [x] UI-217 Change colour
- [x] UI-218 Archive / Unarchive
- [x] UI-219 Focus from the grid
- [x] UI-220 Reveal
- [x] UI-221 Group header menu
- [x] UI-222 Rename group
- [x] UI-223 Delete group
- [x] UI-224 Drag to a section
- [x] UI-225 Drop highlight
- [x] UI-226 Drop moves
- [x] UI-227 Take-out placeholder
- [x] UI-228 OS folder drop overlay
- [x] UI-229 OS folder drop adds
- [x] UI-230 New folders pop in
- [x] UI-231 Folder hover and press
- [x] UI-232 Folder tooltip
- [x] UI-233 Subtitle
- [x] UI-234 Live badges
- [x] UI-235 Label
- [x] UI-236 Faded folders
- [x] UI-237 Rename field opens
- [x] UI-238 Rename commits
- [x] UI-239 Rename isolation
- [x] UI-240 Project rename saves
- [x] UI-241 Picker opens
- [x] UI-242 Picker rows
- [x] UI-243 Current row
- [x] UI-244 Pick a group
- [x] UI-245 New group
- [x] UI-246 Picker focus
- [x] UI-247 Remove dialog
- [x] UI-248 Remove is focused
- [x] UI-249 Confirm removes
- [x] UI-250 Cancel
- [x] UI-251 Toast
- [x] UI-252 Toast belongs to the grid
- [x] UI-253 Rail only on boards
- [x] UI-254 Rail order
- [x] UI-255 Rail focus filter
- [x] UI-256 Rail separators
- [x] UI-257 Rail item states
- [x] UI-258 Rail status dot
- [x] UI-259 Rail unfocused fade
- [x] UI-260 Rail click
- [x] UI-261 Rail add
- [x] UI-262 Edge reveal
- [x] UI-263 No reveal while dragging
- [x] UI-264 Hide grace
- [x] UI-265 Quick sweeps
- [x] UI-266 Rail keeps itself open
- [x] UI-267 Menu holds the rail
- [x] UI-268 Slide animation
- [x] UI-269 Hidden rail is inert
- [x] UI-270 Pinned rail
- [x] UI-271 Rail hint
- [x] UI-272 Hint content
- [x] UI-273 Hint hides
- [x] UI-274 Rail menu
- [x] UI-275 Held ⌘ shows numbers
- [x] UI-276 Number guard
- [x] UI-277 ⌘1–⌘9 switch
- [x] UI-278 Numbers stay while held
- [x] UI-279 Numbers hide
- [x] UI-280 ⌘-click and ⌘-scroll
- [x] UI-281 Numbers above modals
- [x] UI-282 Key badge
- [x] UI-283 Rail stats
- [x] UI-284 Rail overflow
- [x] UI-285 Group order
- [x] UI-286 Group split
- [x] UI-287 Move to group
- [x] UI-288 Optimistic group change
- [x] UI-289 Add project
- [x] UI-290 Add failure toasts
- [x] UI-291 Picker cancelled
- [x] UI-292 Add then show
- [x] UI-293 Set focused
- [x] UI-294 Toggle focus mode
- [x] UI-295 Folder glyph letter
- [x] UI-296 Focus mode on
- [x] UI-297 Focusing
- [x] UI-298 Shows in focus
- [x] UI-299 Focus everywhere off
- [x] UI-300 Focusing definition shared
- [x] UI-301 Platform detection
- [x] UI-302 Shortcut modifier
- [x] UI-303 Shortcut text
- [x] UI-304 Shortcut letter (Latin)
- [x] UI-305 Shortcut letter (other alphabets)
