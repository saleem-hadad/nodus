# Visual parity harness

The Rust UI has to look exactly like Nodus 0.9.4 (decision D8 in `migration/plan.md`). This folder holds what proves it: a script that drives the real Electron app through every screen and state in light and dark, from a synthetic seeded world, and saves a golden screenshot of each. Rust UI work renders the same scenes from the same seed and is compared with them before it merges.

- `capture-electron.mjs`: captures the goldens.
- `scenes/`: the scenes, one definition each (what it shows, the specs it covers, the steps, and the code that drives the app there). `scenes.md` lists them; it's generated from these files.
- `seed/`: the synthetic world and its stand-ins for `claude` and the login shell.
- `diff.mjs`: compares a folder of shots with the goldens.
- `golden/`: where the shots land, `<scene>.<light|dark>.png`. Not committed (`.gitignore`); regenerate them.

Nothing here comes from a real machine: the projects, transcripts, repositories and authors are made up (`seed/world.mjs`), and the app runs with its own `HOME` and `NODUS_USER_DATA`, so it never reads the user's state, transcripts, keys or clipboard.

## Running it

macOS, Node 22 or newer, and an Electron checkout with its dependencies installed:

```sh
corepack pnpm install --frozen-lockfile        # in the app checkout
cd migration/parity && npm ci                   # playwright-core, pngjs, pixelmatch
node capture-electron.mjs                       # every scene, light and dark (~15 min)
node capture-electron.mjs --scene board-populated --theme dark
node capture-electron.mjs --scene 'capture-*'   # a prefix
node capture-electron.mjs --list
```

It builds the app into `<app>/node_modules/.cache/parity` first (never the shared `out/`; `--no-build` skips it), then for each scene and theme: writes a fresh world, launches the app on it, drives it into the scene, waits for it to settle, saves the shot and quits the app and its session host. A failed or never-settling shot is reported and the exit code is 1. `report.json` beside the shots has each one's timing and stability.

Other options: `--out <dir>` (default `golden/` here), `--work <dir>` (the world; default `nodus-parity` in macOS's Shared folder, see below), `--check` (capture into a temporary folder and compare with `--out` instead of overwriting it), `--no-chromium-sandbox` (when the script itself runs inside a sandbox, such as an agent's, Chromium's own sandbox can't start), `--write-md` (regenerate `scenes.md`).

The window appears on screen while it runs. It ignores the real pointer and keyboard (`setIgnoreMouseEvents`, `setFocusable(false)`), so working alongside it doesn't disturb the shots; Playwright's input still reaches the page. The capture panel scenes do bring the panel forward for a moment, as the real panel takes the keyboard when it opens. Only one capture runs at a time per world (`<work>/.lock`).

Nothing in a run reaches the network or the user's own data: Settings scenes report the Accessibility permission as not given (so the global key hook never starts, whatever the machine allows), notifications are silenced, the capture panel treats the board window as the app in front (so no other app's window or browser tab is read) and reads an empty clipboard, and native pickers (the folder picker, the file dialog, the screenshot crosshair) are replaced by IPC stubs in the scenes that need them.

### After cutover

Once `src/` is gone from `main`, capture from the tag, with this folder from `main`:

```sh
git worktree add ../nodus-0.9.4 v0.9.4
(cd ../nodus-0.9.4 && corepack pnpm install --frozen-lockfile)
node migration/parity/capture-electron.mjs --app ../nodus-0.9.4
```

The harness only reads `--app`'s sources, builds them into `--app`'s own `node_modules/.cache/parity`, and runs its Electron binary.

## How a shot is made

Every scene, in both apps, must be rendered under the same conditions:

| | |
| --- | --- |
| Size | The board window's page is 1440×900 CSS pixels at a device scale of 2, so a shot is 2880×1800. The size is emulated (CDP `Emulation.setDeviceMetricsOverride`), so a screen whose work area is shorter than 900 points doesn't matter. The capture panel's shots are its own window: 652 CSS px wide, as tall as the page asks for (CAP-010), with a transparent background (its see-through margin and shadow kept). |
| What's in it | The page only: no window frame, no traffic lights, no window shadow. The native menu bar, native menus and native tooltips (`title` attributes) aren't captured; spec 01 documents the menus. |
| Clock | The UI's "now" is frozen at **2026-10-20 15:00:00 UTC** (`NOW` in `seed/world.mjs`): `Date.now()` and `new Date()` return it in every page, so "2m ago", "1d ago", dates and running times always read the same. Time zone UTC, locale `en-US`. The main process keeps the real clock, so anything it stamps during a scene (a new card, a status, a usage report) is later than `NOW` and reads as "just now" or adds no running time. |
| Animations | Finite animations and transitions run to their end. Infinite ones (the working dot's breathing, the waiting dot's pulse, the section naming glow, spinners) are paused at **time 0**, their first keyframe. |
| Text caret | Hidden. Terminal cursors are hidden by the stand-ins, which draw their own block cursor (`ESC[7m ESC[27m`), as Claude Code does; so no blink phase shows. |
| Terminals | xterm.js with its default WebGL renderer, as users see it: SF Mono 13 px, the theme colours in `src/renderer/src/terminal/registry.ts`. |
| Fonts | The system UI font (SF Pro on macOS, `-apple-system`) and SF Mono for terminals and code, as the app's CSS asks for. Nothing is bundled, so shots are only comparable when taken on macOS with the same system fonts; regenerate the goldens after a macOS update before comparing. |
| Pointer | Wherever a scene leaves it; scenes that don't hover anything park it over the title bar's empty middle (720, 26). |
| Settling | A shot is taken once fonts are loaded, finite animations have ended and infinite ones are paused, and two shots 150 ms apart are byte-identical, or differ in at most 32 stray pixels (the GPU now and then flips a few anti-aliased pixels under a blurred backdrop between frames). The receipt scenes are taken within 1.5 s, as the receipt goes after 2.6 s. |

## The world

`seed/index.mjs` writes it to `--work` before every launch. Everything is fixed (ids from hashed names, times relative to `NOW`, commit dates and authors), so commit hashes, relative times and layout are the same on every run and machine.

- **Projects**: storefront (focused, has actions), docs-site (focused), mobile-app (no sessions), design-system, a group *Infra repos* (infra (focused), terraform-modules, k8s-config), a collapsed group *Experiments* (ml-playground, ar-prototype), and two archived projects (legacy-api, old-landing). Each is a small git repository under `<work>/repos/`; storefront's branch `feat/product-grid` is two commits ahead of its upstream and has modified, deleted, renamed and untracked files.
- **storefront's board**: cards in every status (once its sessions are started), an orange *Checkout* section, a card merged from two others (lineage edges), a terminal card, a sticker on a card, a note signed by a docs-site session, ink (pen, arrow, text, several colours), 16 archived sessions, the note pad, three actions and a global one, and a session with three subagents (agent deck). An action run is seeded only for the scene that shows a run finishing (`withExtra('run')`); others start runs from the rail.
- **docs-site's board**: three views (one with a live page and data, one being built, one with a command waiting for approval) and a note of every colour.
- **infra's board**: an expanded Claude card and an expanded terminal card.
- **mobile-app's board**: empty.
- **Capture**: four recent captures; Claude's guess comes from `responses.json`.

Most board scenes start storefront's sessions first (`storefront()` in `scenes/helpers.mjs`): sf-skeleton working with two agents working and one done, sf-tax waiting on a permission, sf-cart done but running, and the terminal card's shell, all through the stand-ins.

The world lives in `nodus-parity` inside macOS's public `Shared` folder (the one in `/Users` that every account can write to; `DEFAULT_WORK` in `seed/index.mjs`). The app abbreviates any path under a folder in `/Users` to `~`, so it shows `~/nodus-parity/…`: the same on every Mac, and not anyone's home folder. The remove-project dialog shows the full path, which is the same on every Mac too.

### Stand-ins

The app never runs the real `claude` or the user's shell. `<work>/bin` is first on its `PATH`, and `SHELL` is `<work>/bin/fakeshell`:

- `claude --version` answers `2.1.0 (Claude Code)`. `claude -p …` (section names, summaries, action drafts, capture guesses) prints JSON from `<work>/responses.json`. An interactive `claude` draws a static Claude-Code-like screen from its session's transcript (last prompt, tool calls, reply), turns on bracketed paste and waits; it redraws whole on every resize.
- A terminal card's login shell draws a fixed test run and prompt.
- Session statuses come from hooks posted to the session host's hook endpoint (`UserPromptSubmit` → working, `PermissionRequest` → waiting, `Stop` → done, `SubagentStart` with an `agent_id` for agents, `StatusLine` for usage), exactly as Claude Code's own hooks would post them. The harness reads the endpoint's port and token from the host's welcome and never writes the token anywhere.

## Comparing the Rust UI

The Rust side produces the same shots and compares them:

1. **Same world.** Write it with the same generator, with the scene's own changes: `import { seed, DEFAULT_WORK } from './migration/parity/seed/index.mjs'`, then `seed(DEFAULT_WORK, { theme, mutate: scene.seed, responses: scene.responses })` with the scene from `scenes/index.mjs`. Start the Rust app with `HOME=<work>/home`, `NODUS_USER_DATA=<work>/userData`, `PATH=<work>/bin:/usr/bin:/bin`, `SHELL=<work>/bin/fakeshell`, `PARITY_RESPONSES=<work>/responses.json`, `TZ=UTC`, `LANG=en_US.UTF-8`; `state.json`'s theme already matches the shot.
2. **Same conditions.** The table above: a 1440×900 content area at scale 2, a parity mode that freezes the UI clock at `NOW` and holds infinite animations at time 0, no caret, content only. The Rust app needs that parity mode (an environment variable such as `NODUS_PARITY_NOW=<ms>`, read only in debug or test builds), since nothing outside the app can freeze its clock or its animations.
3. **Same steps.** Drive the app through the scene's steps (`scenes.md`), with the same statuses posted to the host: `lib/app.mjs`'s `hook()`, `status()` and `usage()` work against any host that speaks the 0.9.4 protocol, and the stubs a scene installs (`stub()` in `scenes/helpers.mjs`) say what the core must answer.
4. **Save** each shot as `<out>/<scene>.<theme>.png`: 2880×1800, or for the capture panel its window at scale 2 with a transparent background.
5. **Diff:** `node migration/parity/diff.mjs migration/parity/golden <out> --out <out>/diffs`. Every scene must come out `same`. Where a few pixels differ only from text rasterisation, look at the `.diff.png` and the two shots side by side before accepting a tolerance; never accept a difference in layout, colour, size or copy.

Before a slice merges, run the diff for every scene it touches, in both themes, and put the summary in the PR.

## Stability

A run is reproducible when capturing twice gives the same pixels: `node capture-electron.mjs --check` captures into a temporary folder and compares it with `golden/` (byte-identical, else pixelmatch's count and the box around the difference). `diff.mjs` does the same for any two folders.

What could still move between machines: the system fonts (a macOS update can change SF Pro's metrics), the GPU's rasterisation of the WebGL terminals, and the user's macOS "Show scroll bars" setting (the app styles its own scroll bars, so it shouldn't). Regenerate the goldens on the machine that compares, from the tag, rather than comparing against shots from another Mac.

## When a scene breaks

The scene list follows 0.9.4, which doesn't change, so a scene that stops working usually means the world or the harness changed. Each scene is a short function in `scenes/*.mjs`; `--scene <id> --theme light` reruns one, and the PNG shows where it ended up.
