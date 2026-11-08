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

The window appears on screen while it runs. It ignores the real pointer and keyboard (`setIgnoreMouseEvents`, `setFocusable(false)`), so working alongside it doesn't disturb the shots; Playwright's input still reaches the page.

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
| Size | The page is 1440×900 CSS pixels at a device scale of 2, so a shot is 2880×1800 (the capture panel's shots are the panel's own size, 652 wide). The size is emulated, so a screen shorter than 900 points doesn't matter. |
| What's in it | The window's content only: no window frame, no traffic lights, no shadow. The native menu bar and native menus aren't captured (spec 01 documents them). |
| Clock | The UI's "now" is frozen at **2026-10-20 15:00:00 UTC** (`NOW` in `seed/world.mjs`), so "2m ago", "1d ago", dates and running times always read the same. Time zone UTC, locale `en-US`. |
| Animations | Finite animations and transitions run to their end. Infinite ones (the working dot's breathing, the waiting dot's pulse, spinners) are held at their **first keyframe** (time 0). |
| Text caret | Hidden. Terminal cursors are hidden by the stand-ins, which draw their own block cursor (`ESC[7m ESC[27m`), as Claude Code does. |
| Fonts | The system UI font (SF Pro on macOS) and SF Mono for terminals and code, as the app's CSS asks for. Nothing is bundled. |
| Pointer | Wherever a scene leaves it; scenes that don't hover anything park it over the title bar's empty middle (720, 26). |
| Settling | A shot is taken once fonts are loaded, finite animations have ended, and two shots 150 ms apart are byte-identical. |

## The world

`seed/index.mjs` writes it to `--work` before every launch. Everything is fixed (ids from hashed names, times relative to `NOW`, commit dates and authors), so commit hashes, relative times and layout are the same on every run and machine.

- **Projects**: storefront (focused, has actions), docs-site (focused), mobile-app (no sessions), design-system, a group *Infra repos* (infra (focused), terraform-modules, k8s-config), a collapsed group *Experiments* (ml-playground, ar-prototype), and two archived projects (legacy-api, old-landing). Each is a small git repository under `<work>/repos/`; storefront's branch `feat/product-grid` is two commits ahead of its upstream and has modified, deleted, renamed and untracked files.
- **storefront's board**: cards in every status, an orange *Checkout* section, a card merged from two others (lineage edges), a terminal card, a sticker on a card, a note signed by a docs-site session, ink (pen, arrow, text, several colours), 16 archived sessions, the note pad, three actions and a global one, one action run, and a session with three subagents (agent deck).
- **docs-site's board**: three views (one with a live page and data, one being built, one with a command waiting for approval) and a note of every colour.
- **infra's board**: an expanded Claude card (usage stats) and an expanded terminal card.
- **Capture**: four recent captures; Claude's guess comes from `responses.json`.

The world lives in `nodus-parity` inside macOS's public `Shared` folder (the one in `/Users` that every account can write to; `DEFAULT_WORK` in `seed/index.mjs`). The app abbreviates any path under a folder in `/Users` to `~`, so it shows `~/nodus-parity/…`: the same on every Mac, and not anyone's home folder. The remove-project dialog shows the full path, which is the same on every Mac too.

### Stand-ins

The app never runs the real `claude` or the user's shell. `<work>/bin` is first on its `PATH`, and `SHELL` is `<work>/bin/fakeshell`:

- `claude --version` answers `2.1.0 (Claude Code)`. `claude -p …` (section names, summaries, action drafts, capture guesses) prints JSON from `<work>/responses.json`. An interactive `claude` draws a static Claude-Code-like screen from its session's transcript (last prompt, tool calls, reply), turns on bracketed paste and waits; it redraws whole on every resize.
- A terminal card's login shell draws a fixed test run and prompt.
- Session statuses come from hooks posted to the session host's hook endpoint (`UserPromptSubmit` → working, `PermissionRequest` → waiting, `Stop` → done, `SubagentStart` with an `agent_id` for agents, `StatusLine` for usage), exactly as Claude Code's own hooks would post them. The harness reads the endpoint's port and token from the host's welcome and never writes the token anywhere.

## Comparing the Rust UI

The Rust side produces the same shots and compares them:

1. **Same world.** Write it with the same generator: `node -e "import('./migration/parity/seed/index.mjs').then(m => m.seed(m.DEFAULT_WORK, { theme: 'dark' }))"` (a scene may change `state.json` first: its `seed` function). Start the Rust app with `HOME=<work>/home`, `NODUS_USER_DATA=<work>/userData`, `PATH=<work>/bin:/usr/bin:/bin`, `SHELL=<work>/bin/fakeshell`, `PARITY_RESPONSES=<work>/responses.json`, `TZ=UTC`, `LANG=en_US.UTF-8`, and the theme setting matching the shot.
2. **Same conditions.** The table above: a 1440×900 content area at scale 2, a parity mode that freezes the UI clock at `NOW` and holds infinite animations at time 0, no caret, content only.
3. **Same steps.** Drive the app through the scene's steps (`scenes.md`), with the same statuses posted to the host.
4. **Save** each shot as `<out>/<scene>.<theme>.png`, 2880×1800 (or the panel's size).
5. **Diff:** `node migration/parity/diff.mjs migration/parity/golden <out> --out <out>/diffs`. Every scene must come out `same`. Where a few pixels differ only from text rasterisation, look at the `.diff.png` and the two shots side by side before accepting a tolerance; never accept a difference in layout, colour, size or copy.

Before a slice merges, run the diff for every scene it touches, in both themes, and put the summary in the PR.

## Stability

A run is reproducible when capturing twice gives the same pixels: `node capture-electron.mjs --check` captures into a temporary folder and compares it with `golden/`. Shots that differ are listed with the box around the difference.

## When a scene breaks

The scene list follows 0.9.4, which doesn't change, so a scene that stops working usually means the world or the harness changed. Each scene is a short function in `scenes/*.mjs`; `--scene <id> --theme light` reruns one, and the PNG shows where it ended up.
