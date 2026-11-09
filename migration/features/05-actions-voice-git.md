# 05 · Actions, voice & git

Sources:

- Actions: `src/main/actions.ts`, `src/main/actions.test.ts`, `src/shared/actions.ts`, `src/renderer/src/components/actions/ActionRail.tsx`, `ActionDialog.tsx`, `ActionIcon.tsx`, `actions.css`; in `src/main/ipc.ts`: `isTakeover` (l.94–102), `findAction` (l.105–107), `finishRun` / `claimRun` (l.204–241), `configureStatus` hook-up (l.267–282), `board:load` run sweep (l.431–437), `actions:*` handlers (l.606–646), `createSession` run branch (l.838–884), `session:write` (l.916–922).
- Voice: `src/main/voice.ts`, `src/renderer/src/components/voice/SessionMic.tsx`, `VoiceParts.tsx`, `recorder.ts`, `useVoice.ts`, `voice.css`; `voice:*` handlers in `src/main/ipc.ts` (l.648–669).
- Git: `src/main/git.ts`, `src/main/git.test.ts`, `src/renderer/src/components/git/GitControls.tsx`, `DiffModal.tsx`, `diff.ts`, `diff.test.ts`, `git.css`; `git:*` handlers in `src/main/ipc.ts` (l.586–604).
- Read for context (owned elsewhere, referenced not re-specified): `src/shared/types.ts` (ProjectAction, ActionLists, ActionDraft, ActionRunResult, VoiceKeySource, TranscribeResult, Git*), `src/shared/api.ts` (channels), `src/preload/index.ts`, `src/renderer/src/store.ts` (`refreshGit`, `openDiff`, `closeDiff`, `saveAction`, `removeAction`, `setActions`, `globalActions`, `git`, `diffOpen`), `src/renderer/src/views/BoardView.tsx` (`runAction`, `runFinished` toast, l.1093–1151), `src/main/store.ts` (`voiceKey`, `actions`), `src/main/env.ts` (`spawnEnv`, `envVar`, `requireClaude`), `src/renderer/src/styles/tokens.css`, `global.css`, `components/projects/ProjectRail.css` (rail base), `components/board/board.css` (`.fan-card`, `.spin`, `.card-tool`, `.board-toast`), `components/Modal.tsx`/`Modal.css`, `components/board/layout.ts` (`truncate`, `timeAgo`), `store.ts` (`formatDuration`), `board/AgentDeck.tsx` (`useNow`), `electron-builder.yml`, `build/entitlements.mac.plist`, `docs/how-it-works.md` (Actions, The prompt bar, Shortcuts).

## Summary

Three small features that share the board's chrome:

1. **Actions.** A per-project (and optional app-wide "global") list of buttons in an auto-hiding rail on the board's right edge. The user describes a button; a headless `claude -p` call (Sonnet, low effort, read-only tools, JSON-schema output) drafts a name, an icon (one of 26 lucide names) and a prompt; the user reviews and saves. Pressing a button starts an ordinary Claude session on the prompt in auto mode with an extra system prompt, flagged `runOf` so it is not drawn on the board. Its progress fans out beside the rail. When its turn ends (status `done`) the run is removed, its transcript hidden, and a toast shows Claude's last line. Typing into a run (or interrupting it) "takes it over": it loses `runOf` and joins the board.
2. **Voice.** Speech-to-text through xAI (`POST https://api.x.ai/v1/stt`, model `grok-voice-transcribe-2.0`), with up to 100 hint words built from the project's name, branch and changed file names. The key comes from Settings (encrypted with Electron `safeStorage`) or `XAI_API_KEY` from the login-shell env; a saved one wins. A shared recorder/state machine (`useVoice`) drives three mics: the prompt bar's (11), floating capture's (07) and the per-session mic in an open session's title bar (`SessionMic`, owned here).
3. **Git.** Read-only `git` queries behind the title bar's branch chip (with a hover popover of the last five commits) and a "Changes" button that opens a GitHub-Desktop-like diff modal (file list + unified diff with line numbers and intra-line highlights).

## Behaviors

### Actions · data and storage

- **ACT-001** — Action shape: a `ProjectAction` is `{ id: string, name: string, icon: string, description: string, prompt: string, global?: true, createdAt: number }` (ms epoch). `global` is either `true` or absent (never `false` in what main writes).
- **ACT-002** — Where actions live: a project's own actions are stored on the project object as `projects[i].actions` (array, in rail order); global actions at the top level of `state.json` as `actions`. When a list becomes empty its key is deleted (not written as `[]`). Actions are never written into the repository.
- **ACT-003** — Icon list: the allowed icons are exactly the 26 lucide names of `ACTION_ICONS` (see Data & formats), in that order; `DEFAULT_ACTION_ICON` is `zap`; `isActionIcon(x)` is true only for a string that is one of the 26.
- **ACT-004** — Clip rule: `clip(s, max)` returns `''` for a non-string, else `s.trim()` then its first `max` UTF-16 code units (JS `slice`).
- **ACT-005** — Normalize · prompt: `normalizeAction(raw, existing?)` clips `raw.prompt` to 20 000; if the result is empty it throws `An action needs a prompt`.
- **ACT-006** — Normalize · id: `id` = `existing.id` if an existing action was found; else `raw.id` if it is a non-empty string; else a new random UUID v4 (lowercase, 36 chars).
- **ACT-007** — Normalize · name: `name` = `clip(raw.name, 40)` or `Action` if that is empty.
- **ACT-008** — Normalize · icon: `icon` = `raw.icon` if `isActionIcon`, else `zap`.
- **ACT-009** — Normalize · description: `description` = `clip(raw.description, 4000)` (`''` when missing).
- **ACT-010** — Normalize · global: `global` = `true` only when `raw.global === true` (strict boolean); anything else (e.g. the string `yes`) gives absent.
- **ACT-011** — Normalize · createdAt: `createdAt` = `existing.createdAt` if found, else now (ms).
- **ACT-012** — Normalize · key order: the normalized object's keys are written in the order `id, name, icon, description, prompt, global, createdAt` (`global` omitted when absent).

### Actions · commands

- **ACT-013** — Global list command: `actions:global` returns `state.actions` or `[]`; it does not wait for the login-shell env.
- **ACT-014** — Save command: `actions:save(projectId, raw)` looks up the project (unknown id → throws `Unknown project <id>`), finds an existing action by `raw.id` across the project's own list then the global list, normalizes (`raw ?? {}`), then puts it into the list its scope names — replaced in place if its id is already there, else appended — and removes it from the other list. So turning `global` off moves a global action into the current project; turning it on moves it from the project to the global list. State is saved; returns `{ project, global }` (the updated project object and the global list).
- **ACT-015** — Remove command: `actions:remove(projectId, actionId)` removes the id from both the project's own list and the global list (deleting a global action from any project deletes it everywhere); saves; returns `{ project, global }`.
- **ACT-016** — Draft command: `actions:draft(projectId, description, global)` waits for the login-shell env, resolves the project's `repoPath`, and calls the drafter with `global === true` (strict).
- **ACT-017** — Renderer list update: after save/remove the renderer replaces the project in its project list, replaces `board.project` if that board is open, and sets `globalActions` to the returned global list.
- **ACT-018** — Global list loading: the renderer's `globalActions` is (re)loaded with every projects refresh (`refreshProjects`: projects, groups, stats and `actions.global()` together); initial value `[]`.

### Actions · drafting (`claude -p`)

- **ACT-019** — Empty description: the drafter clips the description to 4 000; if empty it rejects with `Describe what the button should do` before looking for Claude.
- **ACT-020** — Claude required: it then requires the Claude CLI path (01's `requireClaude`; missing → its `Claude Code CLI not found. Install it (…) and restart Nodus.` error).
- **ACT-021** — Draft argv: the process is `<claude> -p --model sonnet --effort low --tools Read,Glob,Grep --allowedTools Read,Glob,Grep --permission-mode dontAsk --strict-mcp-config --no-session-persistence --json-schema <SCHEMA_JSON> --output-format json --append-system-prompt <DRAFT_SYSTEM_PROMPT> <USER_PROMPT>`, each a separate argv element (no shell), with the exact strings in Data & formats.
- **ACT-022** — Draft user prompt: `<USER_PROMPT>` is `Draft a button that does this:\n\n<description>` for a project action, and that text plus `\n\nThe button is global: it shows in every project, so the prompt has to work in any repository. Don't rely on this one's commands, paths or tools; tell the run how to find them.` for a global one (`<description>` = the clipped description).
- **ACT-023** — Draft process: spawned with `cwd` = the project's `repoPath`, env = `spawnEnv()` (01), stdin ignored (closed / null — otherwise `claude -p` waits for piped input), stdout and stderr captured as UTF-8, no console window on Windows.
- **ACT-024** — Draft timeout: after 120 000 ms (2 min) the process is killed with SIGKILL (hard kill on Windows) and the call rejects with `Drafting the button took too long`.
- **ACT-025** — Draft spawn failure: if the process cannot be started, the call rejects with the OS error (Node's message, e.g. `spawn <path> ENOENT`).
- **ACT-026** — Draft success: on exit, stdout is trimmed and JSON-parsed (parse failure → treated as no result). If the parsed object exists, `is_error` is not truthy, and `structured_output` exists with a prompt that is non-empty after `clip(…, 20000)`, the call resolves `{ name: clip(name, 40) || 'Action', icon: valid ? icon : 'zap', prompt }`.
- **ACT-027** — Draft failure reason: otherwise it rejects with the first non-empty of: the parsed object's `result` (if a string), the last 3 lines of trimmed stderr joined with a single space, `claude exited with code <code>` (`<code>` is `null` if killed by a signal).

### Actions · runs

- **ACT-028** — Run args: `runArgs(action)` = `['--append-system-prompt', RUN_SYSTEM_PROMPT]` where the prompt embeds the action's name in straight double quotes (Data & formats).
- **ACT-029** — Starting a run (main): `sessions.create(projectId, { x, y, actionId })` looks the action up in the project's own list first, then the global list; if not found it throws `That action no longer exists`. Terminal-kind creates ignore `actionId`.
- **ACT-030** — Run node: the new board node has `runOf = action.id`, `title = action.name` (overriding any `name` option), `createdByApp: true`, `archived: false`, and x/y from the options. The session is spawned (03) with `name = action.name`, `initialPrompt = action.prompt` (any `initialPrompt` option ignored), `permissionMode = 'auto'`, `extraArgs = runArgs(action)`. If the spawn throws, the node is removed and the error rethrown.
- **ACT-031** — Starting a run (renderer): pressing an idle action computes the spot a new session card would take (`spotForNewSession`, 09) — kept for the run in case it is taken over — and calls `sessions.create` with `{ x: round(x), y: round(y), actionId }`; on success the card is upserted into the board store; on failure the board toast shows `Couldn't run <name>: <error message>` (straight apostrophe; the message carries Electron's IPC prefix, see ACT-083).
- **ACT-032** — Runs are hidden from the board: cards with `runOf` are kept in the store's `board.cards` (so the rail sees them) but are not drawn on the canvas, and are left out of search, note placement, capture targets and views' card lists (09/04/07; cross-reference only).
- **ACT-033** — Run tools: a session whose node has `runOf` gets the board's `list_sessions` and `archive_sessions` tools in addition to the common ones; `archive_self` refuses runs with `An action run leaves the board by itself when its turn ends`; `archive_sessions` skips the calling run with reason `it is the session running this action` (owned by 04; listed so the `isRun` predicate = "node has `runOf`" is wired).
- **ACT-034** — Finish trigger · status: whenever the status machine reports `done` for a session, main (after sending `ev:status` and after the notification hook, 02) runs `finishRun(sessionId)` and then the archive-when-done check.
- **ACT-035** — Finish trigger · board load: on `board:load`, every node with `runOf` whose PTY is not live is finished first (a run whose process ended while the app was closed).
- **ACT-036** — finishRun guard: it does nothing unless the session's project, board and node exist and the node has `runOf`.
- **ACT-037** — finishRun effects, in order: read the action (own then global; may be gone), read the session's transcript meta; delete the node from the board; append the session id to the board's `hidden` list if not already there; save state; start killing the PTY (its status is forgotten once the kill completes; nothing below waits for it); send `ev:run-finished` with `{ projectId, sessionId, actionName, message }`; send `ev:board-changed(projectId)`.
- **ACT-038** — Run result fields: `actionName` = the action's current name, else the node's title, else `Action`; `message` = the transcript's last assistant text (absent if none).
- **ACT-039** — Finish notice: the open board for `projectId` (only that one) disposes the run's terminal, closes the terminal modal if it shows that session, and toasts `<actionName>: <truncate(message, 180)>` when there is a message, else `<actionName> finished`. The toast is the board toast (09), cleared after 4 200 ms unless replaced.
- **ACT-040** — Takeover detection: `isTakeover(sessionId, data)` is true if `data` is exactly `\x1b` (Esc) or `\x03` (Ctrl+C). Otherwise false if the session's status is `waiting` (keys answer its question). Otherwise strip from `data`, in this order: string controls `/\x1b[P\]^_][\s\S]*?(?:\x07|\x1b\\)/g` (DCS/OSC/PM/APC terminated by BEL or ST), CSI sequences `/\x1b\[[0-?]*[ -/]*[@-~]/g`, and two-byte escapes `/\x1b[@-_]/g`; the result is true if what remains contains any character outside `\x00–\x1f` and `\x7f`, or a `\r`.
- **ACT-041** — Takeover quirks (keep): because of the order above, an SS3 key (`ESC O A`, arrow keys in application-cursor mode) leaves `A` and counts as takeover; an X10 mouse report (`ESC [ M` + 3 raw bytes) leaves printable bytes and counts; bracketed paste counts (its text survives); focus reports (`ESC[I`, `ESC[O`), SGR mouse reports, colour/cursor replies, Tab and Backspace alone do not.
- **ACT-042** — Takeover hook: every `session:write` (any session) first checks `isTakeover` and, if true, claims the run, before forwarding the bytes to the PTY (so an interrupt doesn't end the run's turn and remove it).
- **ACT-043** — claimRun: if the session's node has `runOf`, delete `runOf`, save state and send `ev:board-changed(projectId)`; otherwise nothing. The claimed session keeps its title (the action name) and becomes an ordinary card; its MCP session tools stop working (04 checks `isRun` per call).
- **ACT-044** — Prompt into a run: sending a prompt into a live session through the prompt bar/capture path also claims it (03's `sendToSession`; cross-reference).
- **ACT-045** — Taken-over placement: a run card that becomes an ordinary card joins the canvas at the spot reserved when it started, moved to a free spot if that has filled since (09; cross-reference).

### Actions · rail

- **ACT-046** — Rail presence: the rail renders only on a board whose project is loaded; it has an invisible 6 px reveal strip on the window's right edge and the floating rail itself.
- **ACT-047** — Rail order: the project's own actions first, then the global actions; each non-empty block is followed by a 0.5 px separator; the last item is the `+` (New Action) button. With no actions only `+` is shown.
- **ACT-048** — Run lookup: each action's run is the store card whose `runOf` equals the action id and which is not archived (if several, the last in card order).
- **ACT-049** — Reveal: pointer entering or moving over the edge strip: if the rail is already open, any pending hide is cancelled; otherwise, if any mouse button is held (dragging a card or panning) or a timer is already pending, nothing; else the rail opens after 100 ms.
- **ACT-050** — Hide: the pointer leaving the edge strip, the rail, or the run fan cancels the pending timer and schedules close after 300 ms if the rail is open, or 0 ms if it isn't (which also cancels a pending reveal); closing clears the hint.
- **ACT-051** — Keep open: the pointer entering the rail or the fan cancels any pending timer.
- **ACT-052** — Shown state: the rail is "shown" while open or while its context menu is up; when not shown it is inert (no focus, no pointer, not in the tab order).
- **ACT-053** — Overlay flag: while the action dialog, the context menu or the delete confirmation is open, the rail tells the board an overlay is open (the board holds its own shortcuts, 09); on unmount it reports false.
- **ACT-054** — Hint: pointer entering a non-running action button or the `+` button shows a tooltip level with the button's vertical centre (viewport coords); pointer leave, any rail scroll, or a right-click hides it; it renders only while the rail is shown and the hovered action still exists. Running actions show no hint.
- **ACT-055** — Hint text: for an action, the name, and a detail line `Every project · <desc>` for a global action or `<desc>` otherwise, where `<desc>` = `truncate(description || 'Right-click to edit', 80)`; for `+`, `New Action` and `Describe a button, and Claude makes it`.
- **ACT-056** — Press: clicking an action with a run closes the rail immediately (no delay) and opens the run's terminal modal; clicking an idle action starts a run (ACT-031) and leaves the rail as it is.
- **ACT-057** — Running look: a running action's tile turns orange (working tint) and gets a status dot in its top-right corner, red (`waiting`) when the run waits on the user, orange otherwise (including `done`).
- **ACT-058** — Global badge: a global action's tile has a 14 px globe badge on its bottom-right corner.
- **ACT-059** — Edge mark: while the rail is not shown and at least one run exists, a 3×36 px sliver sits on the edge strip, red and breathing faster if any run is `waiting`, else orange.
- **ACT-060** — Context menu: right-clicking an action button (default menu suppressed) opens the shared context menu (08) at the pointer with items `Run` (or `Show Run` when it has a run; lucide `play` 14), `Edit…` (`pencil` 14), a separator, `Delete…` (danger, no icon). `Run`/`Show Run` behave as a click (ACT-056).
- **ACT-061** — Edit / New: `Edit…` and the `+` button close the rail immediately and open the action dialog (with the action, or empty).
- **ACT-062** — Delete: `Delete…` opens the confirm dialog (08) titled `Delete “<name>”?` with message `It’s a global button, so it goes from every project. A run already going keeps going.` (global) or `The button and its prompt are deleted. A run already going keeps going.` (project), confirm label `Delete`, danger style; confirming calls `actions:remove(currentProjectId, actionId)`; cancelling does nothing. A run in progress is not stopped.
- **ACT-063** — Run fan: while the rail is shown and at least one action is running, a column of fan cards (one per running action, in rail order) appears to the left of the rail.
- **ACT-064** — Fan position: its top is the first running button's top − 4 px, clamped to [8, windowHeight − fanHeight − 8] (if the upper bound is below 8, top = 8); it is invisible until measured; it is re-measured when the anchor button or the set of running session ids changes (not on window resize).
- **ACT-065** — Fan card text: name = the action's name; step = `It needs you. Open it to answer` when the run is `waiting`, else `truncate(lastAssistantText, 140)` if the transcript has one, else `Starting…`; meta = status label (`Working` / `Needs you` / `Finishing` for working / waiting / done) and `formatDuration(now − card.createdAt)` with `now` ticking every 1 000 ms.
- **ACT-066** — Fan card click: closes the rail immediately and opens that run's terminal modal; tooltip `Open <name>`.

### Actions · dialog

- **ACT-067** — Dialog start: a new action opens on the `describe` step with empty fields, icon `zap`, global off; an existing one opens on the `review` step with its description, name, icon, prompt and global flag.
- **ACT-068** — Dialog header: a 32 px gradient tile with lucide `sparkles`; title `New action` or `Edit action`; subtitle `Say what the button should do. Claude writes the prompt each press runs.` on describe, `Each press runs a session on this prompt in auto mode, beside its button. It goes when it’s done.` on review.
- **ACT-069** — Describe step: a 5-row description textarea (autofocused, disabled while drafting) with placeholder `For example: archive the sessions that are done, so the board only shows what’s still going on.`; below it the label `Start from an example` and five chips; then the scope switch.
- **ACT-070** — Example chips: chip labels and texts are verbatim in Data & formats. Clicking a chip replaces the description with its text; a chip is highlighted while the description equals its text exactly; each chip's tooltip is its full text; chips are disabled while drafting.
- **ACT-071** — Scope switch: a row with title `Use in every project`, hint `A global button, in the actions rail of all your projects. Claude writes its prompt to work in any repository.` when on, `Only this project has it.` when off, and a switch (08's `.switch`) disabled while drafting or saving. It appears on both steps; toggling it on the review step does not redraft.
- **ACT-072** — Draft: `Draft with Claude` (or ⌘/Ctrl+Enter on describe) does nothing if the trimmed description is empty or a draft is in flight; else it bumps a request counter, sets drafting, clears the error, and calls `actions.draft(projectId, trimmedDescription, global)`. On success (if still the latest request) name, icon and prompt are replaced with the draft and the step becomes `review`. On failure (if latest) the error reads `Claude couldn’t draft it: <message>`. Drafting ends only if still the latest request.
- **ACT-073** — Stale drafts: closing the dialog invalidates the counter, so a draft that lands later is dropped.
- **ACT-074** — Review step: fields `Name` (single-line input, max 40 characters, autofocused), `Icon` (26 icon buttons in `ACTION_ICONS` order, single choice, tooltip = icon name; the current icon highlighted — none highlighted if the stored icon is not a valid name), `Prompt` (monospace textarea filling the remaining height, not resizable), then the scope switch.
- **ACT-075** — Save: `Add action` / `Save` (or ⌘/Ctrl+Enter on review) does nothing if the trimmed prompt is empty or a save is in flight; else sets saving, clears the error, and calls `actions.save(projectId, { id: action?.id ?? newUUID, name: name.trim() || 'Action', icon, description: description.trim(), prompt: prompt.trim(), global: global || undefined, createdAt: action?.createdAt ?? now })`; on success the store is updated (ACT-017) and the dialog closes; on failure the error shows the message and saving ends.
- **ACT-076** — Footer · left: on review, a ghost button `Redraft…` (editing) or `Back` (new) returns to describe (disabled while saving); on describe, a ghost `Back to the draft` returns to review, shown only if the prompt is non-empty, disabled while drafting.
- **ACT-077** — Footer · right: a flexible spacer, then `Cancel` (closes; never disabled), then the primary button: on describe `Draft with Claude` with lucide `sparkles` 14 (while drafting: `Drafting…` with a spinning `loader-circle` 14), disabled when the trimmed description is empty or drafting; on review `Add action` (new) / `Save` (editing), disabled when the trimmed prompt is empty or saving.
- **ACT-078** — Dialog keys: ⌘+Enter on macOS / Ctrl+Enter elsewhere, anywhere inside the dialog, drafts on describe and saves on review (default prevented). Plain Enter has no dialog meaning.
- **ACT-079** — Dialog close: Esc (anywhere, including in a field, even while drafting), a pointer-down on the dimmed backdrop, or `Cancel` closes the dialog without saving (08's Modal defaults).
- **ACT-080** — Dialog error: the error paragraph (selectable text) shows between the body and the footer and persists across step changes until the next draft or save clears it.
- **ACT-081** — Icon fallback: wherever an action's icon is drawn, an unknown icon name draws `zap`.
- **ACT-082** — Rail independent of dialog: the dialog closes the rail when it opens (ACT-061); the rail does not re-open by itself when the dialog closes.
- **ACT-083** — IPC error text (keep): errors from main reach the renderer as `Error invoking remote method '<channel>': <ErrorClass>: <message>` (e.g. `Error invoking remote method 'actions:draft': Error: Drafting the button took too long`). The action dialog's draft and save errors and the run toast show that full text, unstripped. The Rust UI must compose the same string from the command's channel name and the error.

### Voice · key

- **VOICE-001** — Key source: `voice.keySource()` returns `settings` when `state.voiceKey` is a non-empty string (even if it can't be decrypted), else `env` when `XAI_API_KEY` from the login-shell environment (01's `envVar`, not the app's own process env) is non-empty after trimming, else `null`.
- **VOICE-002** — Env wait: `voice:key-source`, `voice:set-key` and `voice:transcribe` wait for the login-shell env; `voice:microphone` and `voice:microphone-settings` don't.
- **VOICE-003** — Save key: `voice.setKey(key)` trims the key; if non-empty and the OS secret store is unavailable it throws `The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` on macOS, `Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` elsewhere (nothing changes).
- **VOICE-004** — Key storage: otherwise `state.voiceKey` = base64 of `safeStorage.encryptString(trimmedKey)` (Chromium OSCrypt ciphertext; plan D6); an empty/whitespace/null key deletes `voiceKey`. State is saved; the new key source is returned.
- **VOICE-005** — Non-string argument: `voice:set-key` with a non-string argument is treated as `null` (forget the key).
- **VOICE-006** — Key used: transcription uses the decrypted saved key; if decryption throws it logs `[voice] could not decrypt the saved key` with the error and falls back to the shell key; a key that decrypts to `''` also falls back.
- **VOICE-007** — Key confinement: the key (plain or sealed) is never sent to the UI; the UI only learns the source.
- **VOICE-008** — Key load/migration: on load `voiceKey` is kept only if it is a non-empty string; migrating state from the old app name drops `voiceKey` (01; cross-reference).

### Voice · microphone permission

- **VOICE-009** — Mic check: `voice.microphone()` returns `true` off macOS. On macOS it reads the microphone authorization: `granted` → `true`; `not-determined` → asks the system (the TCC prompt, using Info.plist `NSMicrophoneUsageDescription`) and returns the answer; anything else (`denied`, `restricted`, `unknown`) → `false`.
- **VOICE-010** — Mic settings link: `voice.openMicrophoneSettings()` opens `ms-settings:privacy-microphone` on Windows and `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` everywhere else, with the OS URL opener.

### Voice · transcription request

- **VOICE-011** — No key: with no usable key, `transcribe` returns `{ error: 'Add an xAI API key in Settings to speak prompts.', keyProblem: true }` without any request.
- **VOICE-012** — Hint words: built from `[project.name, status.branch (if git status succeeded and branch non-null), and for each changed file (in status order) the file name without its last extension (Node `path.parse(p).name`)]`; each is trimmed; only terms 3–50 UTF-16 units long are kept; duplicates removed keeping first occurrence; at most the first 100. A git status failure or a folder that isn't a repository gives just the project name. Status is read on the project's `repoPath` (GIT-004).
- **VOICE-013** — MIME and file name: the upload type is the part of the given MIME type before the first `;`, or `audio/webm` if that is empty; the multipart file name is `prompt.<part after the first '/'>` (or `prompt.webm` if there is no `/`). From the recorder this is `audio/webm` and `prompt.webm`.
- **VOICE-014** — Request: `POST https://api.x.ai/v1/stt` with header `Authorization: Bearer <xai-key>` and a `multipart/form-data` body whose parts are, in order: `model` = `grok-voice-transcribe-2.0`; one `keyterm` part per hint word (repeated field name); `file` = the audio bytes with the content type and file name above. No other headers are set by the app.
- **VOICE-015** — Timeout: the request (including reading the body) is aborted after 90 000 ms. An abort by timeout before a response → `{ error: 'xAI didn’t answer in time.' }`; any other failure to get a response → `{ error: 'Couldn’t reach xAI. Check your connection.' }`.
- **VOICE-016** — Response body: the body is parsed as JSON; a parse failure (or a body read aborted by the timeout) gives `null`.
- **VOICE-017** — HTTP error: on a non-2xx status the message is the body's `error` field if it is a string, with every `\s*\[WKE=[^\]]*\]` removed and trimmed; if that is empty, `xAI answered with an error (<status>).`. If the status is 401 or 403, or the message matches `/api key/i`, the result is `{ error: 'xAI didn’t accept the API key.', keyProblem: true }`; else `{ error: <message> }`.
- **VOICE-018** — Success: on 2xx the result is `{ text: body.text.trim() }` if `text` is a string, else `{ text: '' }`.
- **VOICE-019** — Transcribe command: `voice:transcribe(projectId, bytes, mimeType)` waits for the env, resolves the project (unknown → throws `Unknown project <id>`), and coerces `mimeType` with `String(mimeType ?? '')`.

### Voice · recorder

- **VOICE-020** — Capture: the default input device is opened with echo cancellation, noise suppression and automatic gain control on.
- **VOICE-021** — Encoding: audio is recorded as WebM with Opus (`audio/webm;codecs=opus`), as one blob delivered when recording stops (no timeslices); the blob's type is that full MIME string. unclear: bitrate and channel count are Chromium MediaRecorder defaults (`recorder.ts:21–22`).
- **VOICE-022** — Level meter: a spectrum analyser (FFT size 256 → 128 bins, time smoothing 0.7, Web Audio defaults min −100 dB / max −30 dB, Blackman window) runs on the same input at the audio context's sample rate (Chromium: the default output device's rate), read once per animation frame. With `hz = sampleRate / 256`, `lo = max(1, floor(100 / hz))`, `hi = min(128, ceil(3000 / hz))`, `per = (hi − lo) / 5`, band `b` (0–4) covers bins `[floor(lo + b·per), max(from + 1, floor(lo + (b+1)·per)))` and its level is the mean byte magnitude / 255 (0–1). At 48 kHz the bands are bins [1,4), [4,7), [7,10), [10,13), [13,16); at 44.1 kHz [1,4), [4,7), [7,11), [11,14), [14,18).
- **VOICE-023** — Stop / cancel: stopping resolves the recorded audio and then releases the microphone (stops the tracks, closes the analyser context); cancelling releases immediately and discards the audio.

### Voice · state machine (`useVoice`, shared with 07 and 11)

- **VOICE-024** — Phases: `idle` → `opening` → `recording` → `transcribing` → `idle`.
- **VOICE-025** — One mic at a time: within one window, starting a recording cancels any other mic instance's opening/recording/transcribing (a module-level "listening" slot); each instance clears the slot when it stops, cancels or unmounts. (The capture panel is a separate window with its own slot.)
- **VOICE-026** — Start: `record()` takes the slot, bumps the run counter, clears the notice, sets `opening`; if `keySource()` is `null` it returns to `idle` with no notice and opens Settings (only if not cancelled meanwhile); if `microphone()` is false the notice is `Nodus isn’t allowed to use the microphone.` with the System Settings action; if opening the device fails the notice is `Couldn’t open the microphone: <message>`; if cancelled while opening the new recording is cancelled; otherwise the start time is recorded, elapsed = 0, phase `recording`.
- **VOICE-027** — While recording: every animation frame the five levels are read; meter bar `i` gets level `levels[BAR_BANDS[i]]` with `BAR_BANDS = [3, 1, 0, 2, 4]` (lowest band in the middle) and the root gets `max(levels)`, both rounded to 3 decimals; the elapsed time updates every 250 ms; after 300 000 ms (5 min) the recording stops by itself as if the user stopped it. Leaving `recording` clears the root level.
- **VOICE-028** — Stop: `stop()` (no-op without a recording) releases the slot, waits for the audio; if a cancel happened meanwhile, it drops it; if, once the audio is ready, less than 600 ms have passed since recording started, it returns to `idle` silently (a double click, not a prompt); else it transcribes. (Docs say "about half a second"; the code's constant is 600 ms.)
- **VOICE-029** — Transcribe: bumps the run counter, sets `transcribing`, clears the notice, sends the bytes and the blob's MIME type; an exception becomes `{ error: <message> }` (with the IPC prefix of ACT-083). A result for an outdated run is dropped. Then phase `idle`, and: an error keeps the audio for Retry and shows the error with Retry, plus Settings when `keyProblem`; a non-empty text clears the kept audio and calls `onText(text)`; an empty text clears the kept audio and shows `Didn’t catch anything. Try again, a little closer to the mic.`.
- **VOICE-030** — Cancel: bumps the run counter (drops any pending answer), cancels any recording, releases the slot, phase `idle`; the notice is left as is.
- **VOICE-031** — Retry: re-sends the kept audio (if any) through VOICE-029.
- **VOICE-032** — Dismiss: forgets the kept audio and the notice.
- **VOICE-033** — Esc: while the phase is not `idle`, an Esc keydown is intercepted at window capture phase, before anything else, and cancels — unless its target is inside an `input`, `textarea` or `contenteditable="true"` element other than the caller's own field (the prompt bar's and capture's text fields count as their own; `SessionMic` has none, so Esc typed into a focused terminal or any text field goes there and the recording continues). When intercepted, the event goes no further (board and modal Esc handlers don't see it).
- **VOICE-034** — Notice timeout: a notice with no Settings, System Settings or Retry action disappears after 4 200 ms (if still the same notice). Only `Didn’t catch anything…` and `Couldn’t open the microphone: …` qualify.
- **VOICE-035** — Unmount: unmounting a mic bumps the run counter, cancels any recording and releases the slot.
- **VOICE-036** — Clock: elapsed ms → `<minutes>:<seconds two digits>` from whole seconds (floor), e.g. `0:00`, `0:07`, `1:05`, `12:30`.
- **VOICE-037** — Unhandled IPC failure (keep): if `keySource()` or `microphone()` itself fails, nothing catches it and the phase stays `opening` (button disabled) until cancel (Esc where applicable) or unmount.

### Voice · UI parts

- **VOICE-038** — Status (recording): a red dot, the clock, and a 5-bar meter.
- **VOICE-039** — Status (transcribing): a spinning `loader-circle` 13 px and the text `Transcribing…`; nothing in other phases.
- **VOICE-040** — Notice: an alert row with lucide `circle-alert` 14 (red), the message, then optional buttons in this order: `Settings` (lucide `settings` 12; opens the Settings dialog, notice stays), `Open System Settings` on macOS / `Open Settings` elsewhere (opens the microphone settings URL), `Retry` (lucide `rotate-ccw` 12, tooltip `Send the same recording again`), and finally a round `x` 13 dismiss button (tooltip and label `Dismiss`).
- **VOICE-041** — Session mic placement: an open Claude session (not a shell terminal) has a mic in its terminal window's title bar (variant `window`) and in its expanded card's header (variant `card`); what is said is typed at that session's prompt, not sent (12's `registry.typeIn`, which resumes a stopped session; cross-reference).
- **VOICE-042** — Session mic layout: left to right — while recording: the status (its red dot hidden) and a round cancel button (lucide `x` 13, tooltip `Cancel (Esc)`, label `Cancel`); then the mic button.
- **VOICE-043** — Session mic button: click toggles (recording → stop, else record); disabled while `opening` or `transcribing`; icon lucide `mic` 14 (idle/opening), filled `square` 9 with no stroke (recording), spinning `loader-circle` 14 (transcribing); tooltip `Stop and type it in` (recording), `Transcribing…` (transcribing), else `Speak into this session. It’s typed in for you to send.`; accessible label `Stop recording` (recording) else `Speak into this session`.
- **VOICE-044** — Session mic recording look: the button turns into a red circle with a white icon and a red halo whose spread is `2 + 6 × level` px (level = the root level of VOICE-027).
- **VOICE-045** — Session mic notice: the notice hangs 8 px below the mic row, right-aligned to it, above the terminal (z 20), as wide as its content up to `min(440px, 80vw)`. During `transcribing` the session mic shows no status text, only the spinner icon.

### Git · runner (main)

- **GIT-001** — Runner argv: every git call runs `git --no-optional-locks -c core.quotepath=false <args…>` (no shell; `git` resolved on the login-shell `PATH`), with env = `spawnEnv({ GIT_TERMINAL_PROMPT: '0' })` (01), UTF-8 output, no console window on Windows, and a default output cap of 32 MiB (33 554 432 bytes) per stream.
- **GIT-002** — Runner results: exit 0 → `{ stdout, code: 0 }`; a non-zero numeric exit → `{ stdout, code }` (an answer, not a failure); failing to start git, being killed by a signal, or exceeding the cap on stdout or stderr → a thrown error.
- **GIT-003** — Env wait: `git:status`, `git:diff` and `git:log` each wait for the login-shell env and resolve the project's `repoPath` (unknown → `Unknown project <id>`).

### Git · status

- **GIT-004** — Status argv: `status --porcelain=v2 --branch --untracked-files=all -z` in the project's folder.
- **GIT-005** — Not a repository: a non-zero exit returns `null`.
- **GIT-006** — Status headers: records are split on NUL; empty records are skipped. `# branch.head <v>` sets `branch` unless `<v>` is `(detached)`; `# branch.oid <v>` sets `oid` unless `(initial)`; `# branch.upstream <v>` sets `upstream`; `# branch.ab <v>` matching `^\+(\d+) -(\d+)$` sets `ahead` and `behind`. Defaults: `branch: null, oid: null, ahead: 0, behind: 0, files: []`, `upstream` absent. Other headers are ignored.
- **GIT-007** — Status entries: the path is the space-joined remainder after the first N space-separated fields (paths may contain spaces): type `1` → N=8, status from the XY field (field 1); type `2` → N=9, status `renamed`, `oldPath` = the next NUL record (consumed); type `u` → N=10, status `conflicted`; type `?` → path = record from char 2, status `untracked`. Other types (e.g. `!`) are ignored.
- **GIT-008** — XY mapping: X = `A` → `added`; else X or Y = `D` → `deleted`; else `modified` (so `AD` is `added`, `MD` is `deleted`, `T` changes are `modified`).
- **GIT-009** — Status order: files are sorted by path with JS `String.prototype.localeCompare` (ICU collation, default locale of the main process). unclear: exact locale (`git.ts:94`); the Rust port must use an ICU root/en collator to match (e.g. `B.txt` sorts between `a.txt` and `c.txt`).
- **GIT-010** — Status in use: the same status feeds the branch chip, the Changes button, the diff modal and voice hint words.

### Git · log

- **GIT-011** — Log argv: `log --max-count=<limit> --no-show-signature --format=%H%x1f%an%x1f%at%x1f%s -z HEAD --` (default limit 5).
- **GIT-012** — Log parse: non-zero exit (no commits yet, not a repository) → `[]`; else split on NUL, drop empties, split each on `\x1f` into oid, author, author-time (seconds), subject → `{ oid, subject, author, time: seconds × 1000 }`, newest first.
- **GIT-013** — Log limit: `git:log(projectId, limit?)` uses `limit` clamped to 1–50 when it is an integer, else 5.

### Git · diff

- **GIT-014** — Diff root: `rev-parse --show-toplevel` runs in the project folder; non-zero → throws `Not a git repository`; the trimmed output is the repository root used for everything below (status paths are relative to it, which may sit above the project folder).
- **GIT-015** — Path check: for `path` and, if present, `oldPath`: resolve it against the root and take the relative path back; if that is empty (the root itself), starts with `..` (note: also a root-level name like `..foo`), or is absolute, throw `Not in the repository: <p>`.
- **GIT-016** — Base revision: `rev-parse --verify --quiet HEAD` exit 0 → `HEAD`; else the empty tree from `hash-object -t tree /dev/null`, computed once per process and cached for all repositories.
- **GIT-017** — Diff argv: untracked → `diff --no-color --no-ext-diff --no-index -- /dev/null <path>`; anything else → `diff --no-color --no-ext-diff -M <base> -- [<oldPath>] <path>` (working tree incl. staged changes vs base); run in the repository root with a 2 MiB (2 097 152 bytes) cap.
- **GIT-018** — Too large: exceeding the 2 MiB cap (stdout or stderr) returns `{ patch: '', tooLarge: true }`.
- **GIT-019** — Diff exit: a non-zero exit throws `git diff failed for <path>`, except exit 1 for an untracked file (`--no-index` exits 1 when files differ).
- **GIT-020** — Binary: if any output line is exactly `Binary files … differ` or `GIT binary patch` (regex `/^(Binary files .* differ|GIT binary patch)$/m`) the result is `{ patch: '', binary: true }`; else `{ patch: stdout }`.

### Git · status in the UI

- **GIT-021** — Shared read: `refreshGit()` does nothing off a board; concurrent calls for the same project share one in-flight read; a failed read counts as `null`; a result that arrives after the route changed project is dropped; the store's `git` is replaced only when the JSON differs (so polls don't redraw).
- **GIT-022** — Reset: navigating anywhere resets `git` to `null` and closes the diff modal; showing a card from a notification closes the diff modal (08/09).
- **GIT-023** — Refresh triggers: while the board's title bar shows git controls, git status is re-read on mount and project change, on every session status event (any session), when the window gains focus, and every 4 000 ms while the window has focus.
- **GIT-024** — Hidden: with `git` null (not a repository, git missing, a read error, or not loaded yet) neither the chip nor the Changes button is shown.
- **GIT-025** — Changes button: shows lucide `file-diff` 14, the text `Changes`, and, when there are changes, a count badge (`999+` above 999); tooltip `Show <n> changed file` / `Show <n> changed files`, or `No local changes`; click opens the diff modal. It is shown even with no changes.
- **GIT-026** — Branch chip label: `branch`, else the first 7 chars of `oid`, else `HEAD`; followed by `↑<ahead>` when ahead > 0 and `↓<behind>` when behind > 0 (no space between arrow and number).
- **GIT-027** — Log hover: pointer entering the chip area opens the popover after 150 ms; leaving closes it after 150 ms; each enter/leave restarts the timer (so passing over does nothing and the pointer can cross the 6 px gap into the popover).
- **GIT-028** — Log fetch: when the popover opens and HEAD has a commit, `git.log(projectId)` (5 commits) is fetched unless the cached log's key equals `<projectId>:<oid>`; a fetch that lands after the popover closed or the key changed is ignored; errors are ignored (popover shows the head only).
- **GIT-029** — Popover head: `branch` or `Detached HEAD`, and a tracking line: no oid → `No commits yet`; detached → `At <oid7>`; no upstream → `Not published to a remote`; else `Tracking <upstream>, up to date` when ahead and behind are 0, or `Tracking <upstream> · <a> ahead`, `· <b> behind`, or `· <a> ahead, <b> behind`.
- **GIT-030** — Popover commits: shown only when HEAD has an oid and the log for the current key has ≥1 commit; each row: subject (one line, ellipsis, tooltip = subject), right-aligned relative time, and a second line `<oid7> · <author>`.
- **GIT-031** — Relative time: `timeAgo(time)` = `just now` (< 1 min), `<n>m ago` (< 1 h), `<n>h ago` (< 1 day), `<n>d ago` (< 7 days), else a locale short date (`month: 'short', day: 'numeric'`, plus `year: 'numeric'` when older than 300 days), e.g. `Oct 3` / `Oct 3, 2025`; computed when drawn, not ticking.

### Git · diff modal

- **GIT-032** — Modal open: the Changes modal shows while `diffOpen` is set and the route is a board; Esc, a pointer-down on the backdrop, the `x` button, or ⌘W / Ctrl+W (08's close order: settings, agents, terminal, preview, then diff) closes it.
- **GIT-033** — Fresh read: on opening it re-reads git status; until that read settles, a non-repository shows `Reading changes…` instead of the "Not a Git repository" message.
- **GIT-034** — Modal header: title `Changes`; when git is known a sub-line with lucide `git-branch` 12 + branch label (GIT-026 rule, without ahead/behind) and `<n> changed file` / `<n> changed files` or `No changes`; on the right `Refresh` (lucide `refresh-cw` 14; tooltip and label `Refresh`) and close (lucide `x` 16; tooltip `Close (Esc)`, label `Close`).
- **GIT-035** — Empty states: not a repository (after the read) → title `Not a Git repository` and `This project’s folder isn’t tracked by Git, so there are no changes to show.`; no changes → title `No local changes` and `There are no uncommitted changes on <branch label>.` with the branch label in monospace.
- **GIT-036** — File list: one row per changed file in status order: folder part (up to and including the last `/`, secondary colour, ellipsized first) then the file name, and a status icon at the right; tooltip `<Label>: <oldPath> → <path>` for renames, `<Label>: <path>` otherwise; labels `Modified`, `Added`, `New` (untracked), `Deleted`, `Renamed`, `Conflicted`.
- **GIT-037** — Status icons: lucide `square-plus` (added, untracked), `square-minus` (deleted), `square-arrow-right` (renamed), `triangle-alert` (conflicted), `square-dot` (modified); 15 px, stroke 2.
- **GIT-038** — Selection: the selection is remembered as `{ path, index }`; the selected row is the file with that path, else the file now at `min(index, count − 1)` (the remembered path is kept, so if it comes back it is re-selected); initially the first file.
- **GIT-039** — List input: pointer-down on a row selects it; with the list focused, ↓/↑ move by one (clamped), Home/End jump to first/last (default prevented); the selected row is scrolled into view (nearest edge) whenever the selected index changes; the list takes keyboard focus when the modal opens (only if the list exists at that moment).
- **GIT-040** — Focus look: the selected row is accent-filled with white text while the list has focus, and grey (pressed) otherwise.
- **GIT-041** — Diff loading: each file's diff is cached by `<status>\0<oldPath>\0<path>`; selecting a file shows its cached diff at once (if any) and reads it again; while selected it is re-read every 4 000 ms while the window has focus and whenever the window gains focus; `Refresh` re-reads git status and restarts the selected file's read loop. A re-read replaces the shown diff only if the patch text or binary flag changed (or the previous entry was an error).
- **GIT-042** — Uncached selection: while a newly selected, uncached file loads, the previous file's diff (and header) stays shown; with no file selected nothing is shown.
- **GIT-043** — Diff errors: a failed read stores and shows the error for that file.
- **GIT-044** — Pane header: the status icon, folder part + bold file name (selectable), and `from <oldPath>` for renames.
- **GIT-045** — Pane body precedence: error → `Couldn’t read this diff` with the error text below (monospace, selectable; includes the IPC prefix of ACT-083); else binary → `Binary file not shown`; else too large → `This diff is too large to show`; else a summary (GIT-055) → that text; else the lines.
- **GIT-046** — Scroll: switching to a different file scrolls the diff to top-left; a re-read of the same file keeps the scroll position.
- **GIT-047** — Line rendering: each line shows old number, new number, a sign (`+` add, `-` del, a space for context, nothing for hunk/note) and the text; number columns are `max(2, digits(maxLineNo))` monospace characters wide; text wraps (pre-wrap, break anywhere), tabs are 4 wide; text is selectable, numbers and signs are not; a line with a `changed` range draws that range highlighted.

### Git · diff parsing (`parseDiff`, pure)

- **GIT-048** — Lines: the patch is split on `\n`; a final empty element (trailing newline) is dropped.
- **GIT-049** — Hunk header: a line matching `^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@` starts a hunk: old/new counters are set to the two numbers and a `hunk` line is emitted with the whole header line as text and no numbers.
- **GIT-050** — Header lines: lines before the first hunk are collected as header, not emitted.
- **GIT-051** — Hunk lines: inside a hunk, `+…` → `add` (text without the `+`, `newNo` = counter, then +1); `-…` → `del` (`oldNo`, +1); `\…` → `note` with text from char 2 (no numbers); anything else → `context` with text from char 1 (so an empty line, a blank context line that lost its space, gives `''`), both numbers, both +1.
- **GIT-052** — Changed spans: ignoring notes, for each maximal run of `del` lines immediately followed by a run of `add` lines of the same length n > 0, the k-th del pairs with the k-th add; per pair, `pre` = common prefix length, `suf` = common suffix length bounded by `min(lenA, lenB) − pre`; if `pre + suf` is 0 nothing is marked; else each line's `changed` = `[pre, len − suf]` (may be an empty range). Runs of unequal length, or separated by context, are not paired. Lengths and indexes are in UTF-16 code units.
- **GIT-053** — maxLineNo: `max(0, oldCounter − 1, newCounter − 1)` after the last line.
- **GIT-054** — Summary trigger: `summary` is set only when no line was emitted.
- **GIT-055** — Summary text: from the header: `Renamed from <from> without changes` (when both `rename from ` and `rename to ` lines exist), `File mode changed from <old> to <new>` (when both `old mode ` and `new mode ` exist), `Empty file` (when a `new file mode ` line exists), joined with `. `; if none, `No changes to show`.

## Data & formats

### `ACTION_ICONS` (verbatim, in order; lucide names)

`zap`, `refresh-cw`, `git-merge`, `git-pull-request`, `git-commit-horizontal`, `upload`, `download`, `archive`, `brush-cleaning`, `eraser`, `sparkles`, `wand-sparkles`, `test-tube`, `bug`, `rocket`, `package`, `file-text`, `book-open`, `list-checks`, `search`, `shield-check`, `hammer`, `square-terminal`, `gauge`, `lightbulb`, `message-square` (26). Default `zap`. Icons are from lucide-react 1.49.0; ship those exact SVGs.

### Limits

| Constant | Value |
| --- | --- |
| `MAX_NAME_CHARS` | 40 |
| `MAX_DESCRIPTION_CHARS` | 4000 |
| `MAX_PROMPT_CHARS` | 20 000 |
| `DRAFT_TIMEOUT_MS` | 120 000 |
| Rail `REVEAL_DELAY` / `HIDE_DELAY` | 100 ms / 300 ms |
| Fan `FAN_MARGIN` | 8 px |
| Fan clock tick (`useNow`) | 1 000 ms |
| Board toast lifetime | 4 200 ms |
| xAI `TIMEOUT_MS` | 90 000 |
| `MAX_KEYTERMS` / `MAX_KEYTERM_CHARS` / min term length | 100 / 50 / 3 |
| Voice `MAX_MS` / `MIN_MS` | 300 000 / 600 |
| Voice elapsed tick / notice auto-hide | 250 ms / 4 200 ms |
| Analyser fftSize / smoothing / bands / range | 256 / 0.7 / 5 / 100–3000 Hz |
| Git default cap / diff cap | 32 MiB / 2 MiB |
| `GIT_POLL_MS` | 4 000 |
| Branch log open / close delay | 150 ms / 150 ms |
| Log default / IPC clamp | 5 / 1–50 |

### Draft call

`--json-schema` value (exact bytes):

```json
{"type":"object","properties":{"name":{"type":"string"},"icon":{"type":"string","enum":["zap","refresh-cw","git-merge","git-pull-request","git-commit-horizontal","upload","download","archive","brush-cleaning","eraser","sparkles","wand-sparkles","test-tube","bug","rocket","package","file-text","book-open","list-checks","search","shield-check","hammer","square-terminal","gauge","lightbulb","message-square"]},"prompt":{"type":"string"}},"required":["name","icon","prompt"],"additionalProperties":false}
```

`--append-system-prompt` value for drafting (`DRAFT_SYSTEM_PROMPT`; exact text, straight ASCII quotes, paragraphs separated by one blank line, no leading or trailing newline):

```text
You design action buttons for Nodus, a board of Claude Code sessions for one project. Pressing a button starts a fresh Claude Code session in the project's repository on the button's prompt. Nobody watches the session while it works, and it goes away when its turn ends, so the prompt has to stand on its own. It runs in auto mode: Claude Code approves routine steps by itself and blocks risky ones.

Besides the usual tools (reading and editing files, running commands), a run has these board tools:
- list_sessions: the board's sessions, with their status (working, waiting for the user, or done), whether they are archived, their section, Claude's last reply and when they were last active.
- archive_sessions: archives sessions by id. It skips ones that are working or waiting unless forced.
- create_note, update_note and list_notes: write sticky notes on the board, change them, and read the notes there. A note is short plain text, which suits a report the user should find on the board. They take an optional project, from list_projects, to use another project's board.

From the user's description, write:
- name: 1 to 3 words in Title Case, for the button.
- icon: the best fit from the list.
- prompt: the run's instructions, addressed to the session ("you"). Say what to do, step by step: what to check first, when to stop without changing anything, and how to handle the likely failures. If the run may need the user to decide something, tell it to ask with AskUserQuestion. Don't add requirements the user didn't ask for.

You can read the repository to fit the prompt to it (for example, to find the real test command), but be quick about it.
```

Positional prompt (last argv element):

```text
Draft a button that does this:

<description>
```

and, for a global action:

```text
Draft a button that does this:

<description>

The button is global: it shows in every project, so the prompt has to work in any repository. Don't rely on this one's commands, paths or tools; tell the run how to find them.
```

Full argv (one element per line):

```text
<claude path>
-p
--model
sonnet
--effort
low
--tools
Read,Glob,Grep
--allowedTools
Read,Glob,Grep
--permission-mode
dontAsk
--strict-mcp-config
--no-session-persistence
--json-schema
<schema JSON above>
--output-format
json
--append-system-prompt
<DRAFT_SYSTEM_PROMPT>
<positional prompt>
```

Expected stdout (one JSON object, `claude -p --output-format json`): fields read are `is_error` (bool), `structured_output` (`{ name, icon, prompt }`) and `result` (string, used as the failure reason).

### Run system prompt (`--append-system-prompt` added to the run's session argv by 03)

```text
You are running "<action name>", an action button the user pressed on the Nodus board for this project. Nobody is watching while you work, so do the job in the prompt without stopping to chat. If you need the user to decide something, ask with AskUserQuestion: that turns the action's button red to call them. When you are done, end with one line saying what you did. The user is shown that line, and this session then goes away. The mcp__nodus__* tools let you list and archive the board's sessions, and write sticky notes on the board.
```

(One line; the name is inserted raw between straight double quotes, unescaped.) The run's session also gets `--permission-mode auto`, its name and the action's prompt as the first message — argv shape owned by 03.

### Example chips (label → text, verbatim, curly apostrophes as shown)

| Label | Text |
| --- | --- |
| `Pull and push` | `Pull the latest changes from the remote, then push my local commits. If the pull conflicts, resolve the conflicts so both sides’ changes are kept, finish the merge, and push. Never force-push, and stop if there are uncommitted changes that would get in the way.` |
| `Tidy the board` | `Clean up the board: archive the sessions that are done and haven’t been active for a day. Leave anything that is working or waiting for me alone.` |
| `Run the tests` | `Run the project’s tests. If any fail, fix them, run the tests again, and tell me what you changed.` |
| `Commit my changes` | `Look at the uncommitted changes, split them into sensible commits with clear messages, and commit them. Don’t push.` |
| `Review the diff` | `Review the uncommitted changes for bugs and risky edits, and tell me what you found. Don’t change any files.` |

### Persisted (state.json, read and written as-is)

```jsonc
{
  "projects": [{ /* … */ "actions": [ProjectAction, …] /* omitted when empty */ }],
  "actions": [ProjectAction, …],            // global; omitted when empty
  "voiceKey": "<base64 of safeStorage ciphertext>", // omitted when unset
  "boards": { "<projectId>": { "nodes": { "<sessionId>": { /* … */ "runOf": "<actionId>" } }, "hidden": ["<sessionId>", …] } }
}
```

### Events and results

- `ev:run-finished` payload `ActionRunResult`: `{ projectId: string, sessionId: string, actionName: string, message?: string }`.
- `ActionLists`: `{ project: Project, global: ProjectAction[] }`. `ActionDraft`: `{ name, icon, prompt }`.
- `VoiceKeySource`: `'settings' | 'env' | null`. `TranscribeResult`: `{ text: string } | { error: string, keyProblem?: boolean }`.

### xAI request (example on the wire)

```http
POST /v1/stt HTTP/1.1
Host: api.x.ai
Authorization: Bearer <xai-key>
Content-Type: multipart/form-data; boundary=<random>

--<b>
Content-Disposition: form-data; name="model"

grok-voice-transcribe-2.0
--<b>
Content-Disposition: form-data; name="keyterm"

<project name>
--<b>
Content-Disposition: form-data; name="keyterm"

<branch>
… one keyterm part per hint word …
--<b>
Content-Disposition: form-data; name="file"; filename="prompt.webm"
Content-Type: audio/webm

<WebM/Opus bytes>
--<b>--
```

Response read: JSON `{ "text": "…" }` on success; `{ "error": "… [WKE=…]" }` on failure. unclear: Node's `fetch` (undici) adds its own default headers (user agent, accept, encoding); the app sets none of them (`voice.ts:102–107`), so any reasonable client defaults are acceptable.

### Git

Every git argv starts `git --no-optional-locks -c core.quotepath=false`, env adds `GIT_TERMINAL_PROMPT=0`.

| Call | Args after the prefix | cwd | cap |
| --- | --- | --- | --- |
| status | `status --porcelain=v2 --branch --untracked-files=all -z` | project folder | 32 MiB |
| log | `log --max-count=<n> --no-show-signature --format=%H%x1f%an%x1f%at%x1f%s -z HEAD --` | project folder | 32 MiB |
| toplevel | `rev-parse --show-toplevel` | project folder | 32 MiB |
| has HEAD | `rev-parse --verify --quiet HEAD` | repo root | 32 MiB |
| empty tree | `hash-object -t tree /dev/null` | repo root (first time only) | 32 MiB |
| diff (untracked) | `diff --no-color --no-ext-diff --no-index -- /dev/null <path>` | repo root | 2 MiB |
| diff (other) | `diff --no-color --no-ext-diff -M <HEAD or empty tree> -- [<oldPath>] <path>` | repo root | 2 MiB |

Porcelain v2 fields consumed: `1 XY sub mH mI mW hH hI <path>`; `2 XY sub mH mI mW hH hI Xscore <path>` NUL `<origPath>`; `u XY sub m1 m2 m3 mW h1 h2 h3 <path>`; `? <path>`.

Parsed shapes:

```ts
GitStatus   = { branch: string|null, oid: string|null, upstream?: string, ahead: number, behind: number, files: GitFileChange[] }
GitFileChange = { path: string /* repo-root relative */, oldPath?: string, status: 'modified'|'added'|'deleted'|'renamed'|'conflicted'|'untracked' }
GitFileDiff = { patch: string, binary?: true, tooLarge?: true }
GitCommit   = { oid: string, subject: string, author: string, time: number /* ms */ }
DiffLine    = { kind: 'hunk'|'context'|'add'|'del'|'note', text: string, oldNo?: number, newNo?: number, changed?: [number, number] }
ParsedDiff  = { lines: DiffLine[], maxLineNo: number, summary?: string }
```

Diff cache key in the modal: `` `${status}\0${oldPath ?? ''}\0${path}` ``.

## API surface

Commands (mirror `src/shared/api.ts`; channel names in parentheses are used to build ACT-083's error text):

| Command | Channel | Returns | Waits for env |
| --- | --- | --- | --- |
| `git.status(projectId)` | `git:status` | `GitStatus \| null` | yes |
| `git.diff(projectId, file: GitFileChange)` | `git:diff` | `GitFileDiff` | yes |
| `git.log(projectId, limit?)` | `git:log` | `GitCommit[]` | yes |
| `actions.global()` | `actions:global` | `ProjectAction[]` | no |
| `actions.save(projectId, action)` | `actions:save` | `ActionLists` | no |
| `actions.remove(projectId, actionId)` | `actions:remove` | `ActionLists` | no |
| `actions.draft(projectId, description, global)` | `actions:draft` | `ActionDraft` | yes |
| `voice.keySource()` | `voice:key-source` | `VoiceKeySource` | yes |
| `voice.setKey(key \| null)` | `voice:set-key` | `VoiceKeySource` | yes |
| `voice.microphone()` | `voice:microphone` | `boolean` | no |
| `voice.openMicrophoneSettings()` | `voice:microphone-settings` | `void` | no |
| `voice.transcribe(projectId, bytes, mimeType)` | `voice:transcribe` | `TranscribeResult` | yes |
| `sessions.create(projectId, { x, y, actionId })` (run part only; 03 owns) | `session:create` | `SessionCard` | yes |
| `session.write(sessionId, data)` (takeover hook only; 03 owns) | `session:write` | — | no |

Events: `on.runFinished(ActionRunResult)` (`ev:run-finished`); also emits `ev:board-changed(projectId)` from finishRun/claimRun; consumes `ev:status` (git refresh trigger and run finish).

Core-internal functions to provide (names are suggestions): `normalize_action(raw, existing) -> Result<ProjectAction>`, `draft_action(repo, description, global) -> Result<ActionDraft>`, `run_args(&ProjectAction) -> Vec<String>`, `is_takeover(status, data) -> bool`, `find_action(project, state, id)`, `finish_run(session_id)`, `claim_run(session_id)`; `key_source()`, `set_key(Option<String>)`, `microphone() -> bool`, `open_microphone_settings()`, `transcribe(project, bytes, mime) -> TranscribeResult`, `keyterms(project) -> Vec<String>`; `git_status(path)`, `git_log(path, limit)`, `git_diff(project_path, file)`, `parse_diff(patch) -> ParsedDiff`.

Shared UI pieces other domains reuse: the voice controller (`useVoice` equivalent with `phase, elapsed, notice, record, stop, cancel, retry, dismiss`, an optional own-field for Esc, root level and 5 bar levels), `VoiceStatus`, `VoiceNotice` (used by 07 capture and 11 prompt bar), `branchLabel(git)` (used by the diff modal), `GIT_POLL_MS`.

Store state (renderer today; UI model in Rust): `globalActions: ProjectAction[]`, `git: GitStatus | null`, `diffOpen: bool`, `refreshGit()`, `openDiff()`, `closeDiff()`, `saveAction(projectId, action)`, `removeAction(projectId, actionId)`.

## UI details

### Token values used here (light / dark)

| Token | Light | Dark |
| --- | --- | --- |
| `--bg-window` | `#f5f5f4` | `#1e1e20` |
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
| `--focus-ring` | `rgba(0,122,255,0.5)` | `rgba(10,132,255,0.55)` |
| `--status-working` | `#ff9500` | `#ff9f0a` |
| `--status-waiting` | `#ff3b30` | `#ff453a` |
| `--status-done` | `#007aff` | `#0a84ff` |
| `--diff-add` | `#2da44e` | `#3fb950` |
| `--diff-del` | `#cf222e` | `#f85149` |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.06), 0 0 0 0.5px rgba(0,0,0,0.08)` | `0 1px 2px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.08)` |
| `--shadow-md` | `0 4px 14px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.1)` | `0 4px 14px rgba(0,0,0,0.35), 0 0 0 0.5px rgba(255,255,255,0.1)` |
| `--shadow-lg` | `0 18px 50px rgba(0,0,0,0.18), 0 0 0 0.5px rgba(0,0,0,0.12)` | `0 18px 50px rgba(0,0,0,0.5), 0 0 0 0.5px rgba(255,255,255,0.12)` |

Theme-independent: `--text-xs` 11px, `--text-sm` 12px, `--text-md` 13px, `--text-lg` 15px; `--radius-sm` 6px, `--radius-md` 10px, `--radius-lg` 14px, `--radius-xl` 18px; `--dur-fast` 120ms, `--dur-med` 220ms; `--ease-out` `cubic-bezier(0.2, 0.8, 0.2, 1)`; `--rail-w` 52px; `--rail-inset` 8px. Fonts: `--font-ui` = `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI Variable Text', 'Segoe UI', 'Helvetica Neue', sans-serif`; `--font-display` = `-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI Variable Display', 'Segoe UI', 'Helvetica Neue', sans-serif`; `--font-mono` = `'SF Mono', ui-monospace, Menlo, Monaco, 'Cascadia Mono', Consolas, monospace`. Body text is `--font-ui` 13px, line-height 1.4, inherited by buttons and inputs; keyboard focus shows a 2px `--focus-ring` outline offset 2px (global.css, 08).

`color-mix(in srgb, X p%, transparent)` = X's RGB at alpha p (premultiplied mix with transparent). All resolved values below follow that.

### Actions rail (base from ProjectRail.css, right-edge overrides from actions.css)

- **Edge strip** `.rail-edge.is-right`: absolute, z-index 41, top 0, bottom 0, right 0, width 6px, transparent.
- **Edge mark** `.action-rail-mark`: absolute; top 50% with translateY(−50%); right 1px; 3×36px; radius 2px; background `--status-working` (L `#ff9500` / D `#ff9f0a`); animation `action-mark-breathe` 1.6s ease-in-out infinite (opacity 1 → 0.45 at 50% → 1). Waiting: background `--status-waiting` (L `#ff3b30` / D `#ff453a`), duration 1.1s.
- **Rail** `.rail.is-right`: absolute, z-index 40, right 8px, vertically centred (top 0, bottom 0, height fit-content, margin auto 0), max-height `calc(100% - 16px)`; column, items centred, gap 4px; width 52px; padding 6px 0; overflow-y auto with no visible scrollbar; radius 14px; background `--bg-elevated` at 82% (L `rgba(255,255,255,0.82)` / D `rgba(42,42,45,0.82)`) with backdrop blur 24px saturate 1.8; shadow `--shadow-lg`. Hidden: opacity 0, translateX(`100% + 8px`) (off the right edge). Open: opacity 1, no transform. Transition: transform 220ms `--ease-out`, opacity 220ms ease.
- **Rail item** `.rail-item`: 40×40, radius 10px, transparent, no border; hover `--bg-hover` (L `rgba(0,0,0,0.05)` / D `rgba(255,255,255,0.07)`); active `--bg-pressed` (L `rgba(0,0,0,0.09)` / D `rgba(255,255,255,0.12)`); background transition 120ms ease.
- **Action tile** `.action-tile`: 32×32, radius 9px, icon centred; background `--accent` at 14% (L `rgba(0,122,255,0.14)` / D `rgba(10,132,255,0.14)`); icon colour `--accent` (L `#007aff` / D `#0a84ff`); icon = the action's lucide icon at 17px, stroke 2; background and colour transition 120ms ease.
- **Running tile**: background `--status-working` at 16% (L `rgba(255,149,0,0.16)` / D `rgba(255,159,10,0.16)`); icon colour `--status-working`.
- **Running dot** `.status-dot.rail-dot`: absolute top 4px, right 3px, 9×9 circle with a 1.5px `--bg-elevated` border (L `#ffffff` / D `#2a2a2d`); fill and animation from the shared `.status-dot` (08's global.css: working orange with a breathing 3px→5px glow over 1.6s; waiting red pulsing over 1.1s).
- **Global badge** `.action-global`: absolute right −3px, bottom −3px; 14×14 circle; background `--bg-elevated`; shadow `0 0 0 0.5px --border-strong` (L `rgba(0,0,0,0.18)` / D `rgba(255,255,255,0.18)`); colour `--text-secondary` (L `rgba(0,0,0,0.56)` / D `rgba(255,255,255,0.58)`); lucide `globe` 9px, stroke 2.4.
- **Separator** `.rail-sep`: 24×0.5px, margin 2px 0, `--border-strong`.
- **Add button** `.rail-add`: lucide `plus` 18px stroke 2; colour `--text-secondary`, hover `--text-primary` (L `rgba(0,0,0,0.88)` / D `rgba(255,255,255,0.9)`).
- **Hint** `.rail-hint.is-right`: fixed, z-index 60; right `8 + 52 + 6 = 66px`; top = button centre with translateY(−50%); column; max-width 260px; padding 5px 9px; radius 6px; background `--bg-elevated`; shadow `--shadow-md`; no pointer events; animation `action-hint-in` 120ms `--ease-out` (from opacity 0, translate(3px, −50%)). Name: 12px, 600, `--text-primary`, one line with ellipsis. Detail: 11px, `--text-secondary`, wraps.
- **Run fan** `.action-fan`: fixed, z-index 45; right `8 + 52 + 10 = 70px`; column, gap 8px; width 260px; `--deal-x` 46px. Cards use 10's `.fan-card` (board.css): full width, padding 10px 12px 11px, radius 10px, background `--bg-elevated`, shadow `inset 3px 0 0 <edge>, --shadow-md` with edge = `--status-working` (working) / `--status-waiting` (waiting) / `--status-done` (done; L `#007aff` / D `#0a84ff`); text `--text-secondary`, left aligned; hover adds `0 0 0 1px --border-strong` and swaps to `--shadow-lg`; entrance `fan-deal` 520ms `--ease-stack` (the damped-spring `linear()` easing in tokens.css, 08), delay `i × 45ms + 30ms`, from opacity 0, translate(46px, 10px) scale(0.92) rotate(−2.5deg). Head: small status dot (7px, 08) + name 13px/600 `--text-primary` one line ellipsis, gap 7px. Step and meta: 11px, indented 14px; step clamps to 2 lines; meta `--text-tertiary` (L `rgba(0,0,0,0.36)` / D `rgba(255,255,255,0.36)`), tabular numbers, gap 0 8px; status label coloured `--status-working` / `--status-waiting` (done label tertiary).

### Action dialog

- Panel `.action-dialog` (inside 08's modal panel: radius 18px, `--bg-elevated`, `--shadow-modal`, column flex, overflow hidden, rise animation 220ms): width `min(620px, 100vw − 48px)`, max-height `min(86vh, 760px)`.
- Header `.action-head`: row, align start, gap 12px, padding 20px 22px 16px, bottom border 0.5px `--separator` (L `rgba(0,0,0,0.07)` / D `rgba(255,255,255,0.07)`). Icon tile: 32×32, radius 9px, linear-gradient 135° from `--accent` to `#5e5ce6` (both themes), white lucide `sparkles` 16 stroke 2.2. Title: 15px, 600, margin 0. Subtitle: 12px `--text-secondary`, margin-top 2px.
- Body `.action-body`: column, gap 14px, flex 1, padding 16px 22px, scrolls when tall.
- Field `.action-field`: column gap 6px; the prompt field grows (flex 1, min-height 180px). Label `.action-label`: 12px, 600, `--text-secondary`.
- Inputs `.action-input`: full width; border 0.5px `--border-strong`; radius 8px; background `--bg-window` (L `#f5f5f4` / D `#1e1e20`); no outline; focus: border `--accent` and ring `0 0 0 3px --focus-ring` (L `rgba(0,122,255,0.5)` / D `rgba(10,132,255,0.55)`); disabled opacity 0.6; transitions 120ms ease. Description: padding 10px 12px, vertically resizable, line-height 1.45, 5 rows. Name: height 32px, padding 0 10px. Prompt: flex 1, min-height 160px, padding 12px 14px, not resizable, `--font-mono` 12px, line-height 1.55.
- Examples: column gap 8px; chips wrap with gap 6px. Chip: height 26px, padding 0 11px, border 0.5px `--border-strong`, radius 13px, background `--bg-elevated`, `--text-primary`, 12px; hover `--bg-hover`; active: border `--accent`, background `--accent` at 12% (L `rgba(0,122,255,0.12)` / D `rgba(10,132,255,0.12)`), text `--accent`. Disabled chips (while drafting) have no style of their own: the author colours above override Chromium's disabled grey, so they look unchanged and just ignore clicks (unclear whether the hover tint still shows on a disabled chip; `actions.css:218–235`).
- Icon grid: wrap, gap 4px; option 30×30, radius 7px, no border, transparent, `--text-secondary`, icon 15px stroke 2; hover `--bg-hover` + `--text-primary`; selected: background `--accent`, colour `--text-on-accent` (#fff).
- Scope row `.action-scope`: row, space-between, align start, gap 16px, padding-top 12px, top border 0.5px `--separator`. Title 13px/500; hint 12px `--text-secondary` line-height 1.4; text column gap 3px. Switch: 08's `.switch` (32×18, track `--bg-pressed`, on = `--accent`, 14px white knob moving 14px in 120ms; disabled opacity 0.5).
- Error `.action-error`: margin 0 22px 12px; padding 8px 12px; radius 8px; background `--status-waiting` at 10% (L `rgba(255,59,48,0.10)` / D `rgba(255,69,58,0.10)`); `--text-primary` 12px; pre-wrap; breaks long words; selectable.
- Footer `.action-foot`: row, centred, gap 8px, padding 12px 22px, top border 0.5px `--separator`. Buttons are 08's `.btn` (28px tall, padding 0 12px, radius 6px, 13px/500; primary = `--accent` fill with white text, hover `--accent-hover` L `#0068d9` / D `#3396ff`; ghost = transparent; disabled opacity 0.45). Spinner `loader-circle` rotates 360° per 0.9s linear (`.spin`).
- Copy (verbatim): `New action`, `Edit action`, subtitles (ACT-068), placeholder (ACT-069), `Start from an example`, chip labels, `Use in every project`, scope hints (ACT-071), `Name`, `Icon`, `Prompt`, `Back`, `Redraft…`, `Back to the draft`, `Cancel`, `Draft with Claude`, `Drafting…`, `Add action`, `Save`, `Claude couldn’t draft it: <message>`.

### Session mic and voice parts

- `.session-mic`: relative row, centred, gap 6px.
- Mic button: variant `window` = 08's `.btn.btn-ghost.btn-icon` (28×28, radius 6px, transparent); variant `card` = 10's `.card-tool` (min-width 28, height 28, radius 6px, transparent, `--text-secondary`). While recording (`.is-recording`): radius 50%, background `--status-waiting` (L `#ff3b30` / D `#ff453a`), colour `#fff`, box-shadow `0 0 0 calc(2px + level × 6px)` of `--status-waiting` at 20% (L `rgba(255,59,48,0.2)` / D `rgba(255,69,58,0.2)`), box-shadow transition 80ms linear.
- `.voice-status`: inline row, gap 8px, `--text-secondary`, 12px, no wrapping.
- `.voice-rec-dot`: 8×8 circle `--status-waiting`, blink 1.2s ease-in-out infinite (opacity 0.3 at 50%); hidden inside `.session-mic`.
- `.voice-time`: min-width 28px, `--text-primary`, 500, tabular numbers.
- `.voice-meter`: inline row, centred, gap 2px, height 16px; `.voice-bar`: 3×16px, radius 2px, `--text-secondary`, vertically scaled by `clamp(0.18, level × 1.4, 1)` (from centre), transform transition 70ms linear. Bar order left→right shows bands 3, 1, 0, 2, 4.
- `.voice-icon-btn` (cancel, dismiss): 24×24 circle, no border, transparent, `--text-tertiary`; hover `--bg-hover` + `--text-primary`.
- `.voice-notice` (inside `.sidekick` or `.session-mic` only): row, centred, gap 8px, max-width 440px (session mic: `min(440px, 80vw)`), min-height 36px, padding 6px 6px 6px 12px, radius 18px, background `--bg-elevated` at 86% (L `rgba(255,255,255,0.86)` / D `rgba(42,42,45,0.86)`) with backdrop blur 20px saturate 1.6, shadow `--shadow-md`, 12px; entrance `voice-pill-in` 220ms `--ease-out` from opacity 0, translateX(10px) scale(0.96), transform origin right centre (session mic: top right). Icon `circle-alert` 14 `--status-waiting`, no shrink. Text: flexible, `--text-primary`, line-height 1.35. Action buttons: `.btn` with gap 4px, height 24px, padding 0 8px, 12px.
- `.session-mic-notice`: absolute, z-index 20, top `100% + 8px`, right 0, width max-content.
- Copy: `Transcribing…`, `Cancel (Esc)`, `Cancel`, `Stop and type it in`, `Speak into this session. It’s typed in for you to send.`, `Stop recording`, `Speak into this session`, `Settings`, `Open System Settings`, `Open Settings`, `Retry`, `Send the same recording again`, `Dismiss`, plus the notice messages in Edge cases.

### Branch chip and Changes button (title bar; placement owned by 08)

- Chip wrapper `.git-branch-wrap`: relative row, min-width 0, margin-left 6px, excluded from window dragging.
- Chip `.git-branch`: inline row, centred, gap 5px, max-width 260px, height 22px, padding 0 8px, radius 11px, background `--bg-sunken` (L `rgba(0,0,0,0.035)` / D `rgba(255,255,255,0.04)`), inset 0.5px `--border` ring (L `rgba(0,0,0,0.1)` / D `rgba(255,255,255,0.1)`), `--text-secondary`, 12px/500. lucide `git-branch` 13 stroke 2.2 (no shrink). Name: one line, ellipsis. Ahead/behind: `--text-tertiary`, tabular, no shrink.
- Log popover `.git-log`: absolute, top `100% + 6px`, left 0, width 360px, padding 5px, radius 10px, solid `--bg-elevated`, `--shadow-lg`, `--text-primary`, 12px; entrance `git-log-in` 90ms `--ease-out` from opacity 0, translateY(−2px). A transparent 6px strip above it bridges to the chip (hover continuity). Head: column gap 1px, padding 4px 8px 6px; branch 600 one line ellipsis; tracking `--text-secondary`. List: padding-top 4px, top border 0.5px `--separator`, no bullets. Commit row: 2-column grid (`minmax(0,1fr) auto`), column gap 10px, padding 4px 8px, radius 5px, hover `--bg-hover`; subject one line ellipsis; time `--text-tertiary` tabular no wrap; meta spans both columns, `--text-tertiary` 11px, one line ellipsis; oid in `--font-mono`.
- Changes button `.git-changes`: inline row, centred, gap 5px, no shrink, height 24px, margin-left 2px, padding 0 8px, no border, radius 6px, transparent, `--text-secondary`, 12px/500; hover `--bg-hover` + `--text-primary`; active `--bg-pressed`. lucide `file-diff` 14 stroke 2. Count `.git-count`: min-width 18px, padding 0 5px, radius 9px, background `--accent`, `--text-on-accent`, 11px/600, line-height 16px, centred, tabular.

### Changes modal

- Panel `.diff-modal`: width `min(1180px, 100vw − 48px)`, height `min(88vh, 900px)` (08's modal panel otherwise).
- Header `.diff-header`: row, centred, gap 16px, padding 16px 12px 14px 22px, bottom border 0.5px `--separator`. Title: `--font-display` 15px/600. Sub: wrap, gap 4px 14px, margin-top 3px, `--text-secondary` 12px; branch part inline row gap 4px. Actions: row gap 4px, ghost icon buttons 28×28.
- Body: row filling the rest.
- File list `.diff-files`: width `clamp(220px, 26%, 320px)`, padding 6px, vertical scroll, right border 0.5px `--separator`, background `--bg-sunken`; no focus outline. Row `.diff-file`: row, centred, gap 8px, height 28px, padding 0 8px 0 10px, radius 6px, 13px; hover `--bg-hover`; selected `--bg-pressed`; selected while the list is focused: background `--accent`, colour `--text-on-accent`, folder text and status icon inherit white at opacity 0.85. Path: flexible, one line; folder part `--text-secondary`, shrinks first with ellipsis; name doesn't shrink (ellipsis at 100%).
- Status icon colours: modified `--status-working` (L `#ff9500` / D `#ff9f0a`); added/untracked `--diff-add` (L `#2da44e` / D `#3fb950`); deleted/conflicted `--diff-del` (L `#cf222e` / D `#f85149`); renamed `--accent`.
- Pane: column, flexible. Head `.diff-pane-head`: row, centred, gap 8px, height 38px, padding 0 16px, bottom border 0.5px `--separator`, 13px, no wrap; path clipped; name 600; `from …` `--text-tertiary` 12px with ellipsis. Scroll area fills the rest, scrolls both ways.
- Lines `.diff-lines`: `--font-mono` 12px, line-height 20px, tab size 4, padding-bottom 12px, min-width min-content. Line `.diff-line`: grid `[num-w, num-w, 1ch, 1fr]`, column gap 10px, min-height 20px, padding-left 12px. Numbers and sign: `--text-tertiary`, right-aligned, tabular, not selectable. Text: padding-right 16px, pre-wrap, break anywhere, selectable with a text cursor.
- Line colours: add background `--diff-add` at 13% (L `rgba(45,164,78,0.13)` / D `rgba(63,185,80,0.13)`), sign `--diff-add`; del background `--diff-del` at 11% (L `rgba(207,34,46,0.11)` / D `rgba(248,81,73,0.11)`), sign `--diff-del`; changed span radius 2px, text colour unchanged, background add 34% (L `rgba(45,164,78,0.34)` / D `rgba(63,185,80,0.34)`) / del 30% (L `rgba(207,34,46,0.30)` / D `rgba(248,81,73,0.30)`); hunk line margin-top 8px (0 for the first line), background `--accent` at 8% (L `rgba(0,122,255,0.08)` / D `rgba(10,132,255,0.08)`), `--text-secondary`; note text `--text-tertiary` italic.
- Empty/notice: column, centred both ways, gap 8px, padding 48px 24px, `--text-secondary`, centred text; empty fills the body. Empty title `--text-primary` `--font-display` 15px/600. Notice detail and the branch name in the empty message: `--font-mono` 12px; detail max-width 520px, `--text-tertiary`, breaks long words.
- Copy: `Changes`, `<n> changed file(s)`, `No changes`, `Refresh`, `Close (Esc)`, `Close`, `Not a Git repository`, `This project’s folder isn’t tracked by Git, so there are no changes to show.`, `Reading changes…`, `No local changes`, `There are no uncommitted changes on <branch>.`, `Couldn’t read this diff`, `Binary file not shown`, `This diff is too large to show`, summaries (GIT-055), status labels (GIT-036), `from <oldPath>`.

### Icons used (lucide-react 1.49.0 names)

Actions: the 26 action icons, `globe`, `plus`, `play`, `pencil`, `sparkles`, `loader-circle`. Voice: `mic`, `square`, `x`, `loader-circle` (`Loader2` alias), `circle-alert` (`AlertCircle` alias), `rotate-ccw`, `settings`. Git: `git-branch`, `file-diff`, `refresh-cw`, `x`, `square-plus`, `square-minus`, `square-arrow-right`, `square-dot`, `triangle-alert`.

## Keyboard & mouse

| Where | Input | Effect |
| --- | --- | --- |
| Board, right edge | Point at the 6px edge strip (no button held) | Rail slides in after 100 ms |
| Rail / fan | Pointer leaves | Rail slides out after 300 ms |
| Rail button | Click | Run, or open the run if one is going |
| Rail button | Right-click | Menu: Run / Show Run, Edit…, Delete… |
| Rail `+` | Click | New action dialog |
| Fan card | Click | Open the run's terminal |
| Action dialog | ⌘Enter (macOS) / Ctrl+Enter | Draft (describe) / save (review) |
| Action dialog | Esc, backdrop pointer-down, Cancel | Close without saving |
| Session mic | Click | Start / stop recording |
| Any mic, not idle | Esc (not in another text field / terminal) | Cancel, keep nothing |
| Title bar chip | Hover 150 ms | Show the last five commits |
| Title bar `Changes` | Click | Open the Changes modal |
| Changes list (focused) | ↑ / ↓ / Home / End | Move the selection |
| Changes list row | Pointer-down | Select |
| Changes modal | Esc, backdrop, `x`, ⌘W / Ctrl+W | Close |

Shortcuts-table rows from docs in this domain: "Speak into an open session, typed in but not sent — The mic in its title bar". (The `V` key and the bar's mic belong to 11.)

## Platform differences

- **PD-1** Mod key for the dialog: ⌘ on macOS, Ctrl elsewhere (`modKey`).
- **PD-2** Microphone permission: asked/read on macOS only; elsewhere `microphone()` is always `true`, so the "not allowed" notice (and thus the `ms-settings:privacy-microphone` link and the `Open Settings` label) is unreachable on Windows in practice; a Windows privacy block surfaces as `Couldn’t open the microphone: <Chromium error message>` (auto-hides after 4.2 s).
- **PD-3** Settings URL: Windows `ms-settings:privacy-microphone`; macOS (and any other OS) `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`. Notice button label: `Open System Settings` on macOS, `Open Settings` elsewhere.
- **PD-4** Key encryption: Keychain-backed on macOS (error text names "The Keychain"), DPAPI-backed on Windows (error text "Encrypted storage"). Ciphertext compatibility: plan D6 / 01.
- **PD-5** Processes: `claude` and `git` start without a console window on Windows; the draft timeout kill is SIGKILL on POSIX, `TerminateProcess` on Windows.
- **PD-6** Git on Windows: `git.exe` from the login env `PATH`; `/dev/null` is passed literally (Git for Windows maps it); repository root paths come back with forward slashes and are resolved with Windows path rules for the containment check.
- **PD-7** macOS packaging needs `NSMicrophoneUsageDescription` = `Nodus listens only while you record with the mic button, to turn what you say into a prompt.` and the `com.apple.security.device.audio-input` entitlement (06 owns).
- **PD-8** Hint words use Node's platform `path.parse` (on Windows both `/` and `\` separate).

## Edge cases & errors (verbatim messages)

Actions (main):
- `An action needs a prompt` — save with an empty prompt.
- `Describe what the button should do` — draft with an empty description.
- `Drafting the button took too long` — draft over 2 minutes.
- `claude exited with code <code>` — draft failed with no `result` and no stderr.
- `That action no longer exists` — starting a run of an action id that is no longer in either list.
- `Unknown project <id>` — any project-scoped command with an unknown id.
- `An action run leaves the board by itself when its turn ends` / `it is the session running this action` — board tools (04).

Actions (UI):
- `Claude couldn’t draft it: Error invoking remote method 'actions:draft': Error: <message>`.
- `Couldn't run <name>: Error invoking remote method 'session:create': Error: <message>` (board toast).
- `<name>: <message ≤180 chars…>` / `<name> finished` (board toast on finish).
- Deleting an action while its run is going: the run continues; its result toast falls back to the node title (the action name at start).
- A run whose process died while the app was closed is finished on the next load of its board, before the board snapshot is built. unclear: whether its toast shows depends on the board view having subscribed to `ev:run-finished` before the load's event arrives (`store.ts` `navigate` → `loadBoard` vs the `BoardView.tsx:1127` effect).
- Two runs of one action can't be started from the rail (a press opens the existing run).

Voice:
- `Add an xAI API key in Settings to speak prompts.` (keyProblem) — only reachable if the key vanished after the mic's key check, since the mic opens Settings when there is no key.
- `xAI didn’t answer in time.` · `Couldn’t reach xAI. Check your connection.` · `xAI didn’t accept the API key.` (keyProblem) · `<xAI's error without [WKE=…]>` · `xAI answered with an error (<status>).`
- `Nodus isn’t allowed to use the microphone.` (System Settings button).
- `Couldn’t open the microphone: <message>`.
- `Didn’t catch anything. Try again, a little closer to the mic.` (auto-hides).
- `The Keychain isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` / `Encrypted storage isn’t available, so the key can’t be saved. Set XAI_API_KEY in your shell instead.` (shown by 08's Settings).
- Log line `[voice] could not decrypt the saved key` (+ error) when the saved key can't be decrypted; the source still reads `settings`.
- Recording under 600 ms: silently discarded. Recording reaching 5 minutes: auto-stops and transcribes.
- Every transcription error notice offers Retry (including the missing-key one, which also offers Settings).

Git:
- `Not a git repository` — diff outside a repository.
- `Not in the repository: <path>` — path escaping the root, absolute, or the root itself.
- `git diff failed for <path>` — non-zero diff exit (except untracked exit 1).
- Status output over 32 MiB, or git missing → the read throws → UI treats as `null` → chip and Changes button disappear.
- The empty-tree hash is cached for the process from the first repository that needed it (a SHA-256 repository after a SHA-1 one would reuse the SHA-1 hash; keep).
- A diff over 2 MiB → `This diff is too large to show`. A re-read that turns a previously empty-patch result into tooLarge (both `patch: ''`, no binary) is considered unchanged and not redrawn (keep).
- Renamed file whose old path is outside the repo → `Not in the repository: <oldPath>`.

## Tests to port

### `src/main/actions.test.ts` → `nodus-actions`

- `normalizeAction › needs a prompt`: `{ name: 'Test' }` and `{ prompt: '   ' }` both throw `An action needs a prompt`.
- `normalizeAction › fills in a new action`: `{ prompt: '  Run the tests.  ' }` → `{ id: /^[0-9a-f-]{36}$/, name: 'Action', icon: 'zap', description: '', prompt: 'Run the tests.', global: undefined, createdAt: <number> }`.
- `normalizeAction › trims and clips what the renderer sends`: name `'  ' + 'N'×60 + '  '` → length 40; prompt `'p'×25000` → length 20000; description `'d'×5000` → length 4000; icon `'not-an-icon'` → `'zap'`; global `'yes'` → undefined.
- `normalizeAction › keeps an existing action’s id and creation time`: `existing = normalize({ prompt: 'Old', icon: 'bug', global: true })`; `normalize({ id: 'other', prompt: 'New', icon: 'rocket', global: true }, existing)` matches `{ id: existing.id, createdAt: existing.createdAt, prompt: 'New', icon: 'rocket', global: true }`.
- `runArgs › tells the run which action it is`: for name `Tidy Up`, `args[0] === '--append-system-prompt'` and `args[1]` contains `"Tidy Up"`.

### `src/main/git.test.ts` → `nodus-git` (integration; needs `git` on PATH)

Setup: `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1` in the environment the runner uses (the TS test sets them before env capture; in Rust inject them into the runner's env); helper `git -c user.name=Test -c user.email=test@example.com …`; each test gets a realpath'd temp dir with `git init -q -b main`, removed afterwards.

- `gitStatus › is null outside a repository`: a plain temp dir → `null`.
- `gitStatus › reads a repository with no commits yet`: write `first.txt` → `{ branch: 'main', oid: null, ahead: 0, behind: 0, files: [{ path: 'first.txt', status: 'untracked' }] }` (no `upstream`).
- `gitStatus › lists every kind of change, sorted by path`: commit `a.txt`, `gone.txt`, `old name.txt`; modify `a.txt`; add+stage `c.txt`; delete `gone.txt`; `git mv 'old name.txt' 'new name.txt'`; write `dir/un tracked.txt` → branch `main`, oid 40 hex, files exactly `[a.txt modified, c.txt added, dir/un tracked.txt untracked, gone.txt deleted, new name.txt renamed (oldPath 'old name.txt')]`.
- `gitStatus › counts commits ahead of the upstream`: clone the repo, commit `b.txt` in the clone → matches `{ branch: 'main', upstream: 'origin/main', ahead: 1, behind: 0 }`.
- `gitStatus › reads no branch on a detached head`: commit, `checkout -q --detach` → `branch === null`.
- `gitDiff › diffs a changed file against HEAD`: `one\ntwo\n` → `one\nthree\n`; patch contains `-two\n+three\n`.
- `gitDiff › diffs an untracked file as all new`: patch contains `+fresh\n`.
- `gitDiff › diffs a staged file in a repository with no commits`: staged `first.txt` (`hi\n`), status `added` → contains `+hi\n`.
- `gitDiff › takes paths from the repository root when the project is a folder inside it`: project = `<root>/app`, file `app/a.txt` `one\n` → `two\n` → contains `+two\n`.
- `gitDiff › says when a file is binary`: `a\0b` → `c\0d` → exactly `{ patch: '', binary: true }`.
- `gitDiff › refuses paths outside the repository`: `../outside.txt` (untracked), `/etc/hosts` (untracked), `a.txt` with oldPath `../../etc/passwd` (renamed), `.` (modified) all reject with `Not in the repository`.
- `gitDiff › fails outside a repository`: plain dir → rejects `Not a git repository`.
- `gitLog › is empty before the first commit and outside a repository`: both `[]`.
- `gitLog › lists the newest commits on the branch first, up to the limit`: 7 commits with subject `Change <i>: tabs\tand | pipes`, body `A body that is left out`, date `<1700000000 + i×3600> +0000` → subjects for i = 7,6,5,4,3; `log[0]` equals `{ oid: <rev-parse HEAD>, subject: 'Change 7: tabs\tand | pipes', author: 'Test', time: (1700000000 + 7×3600) × 1000 }`; `gitLog(root, 2)` has length 2.
- `gitLog › follows the branch that is checked out`: on `feature` → `['On feature', 'files']`; back on `main` → `['files']`.

### `src/renderer/src/components/git/diff.test.ts` → `nodus-git` (pure)

Header used: `diff --git a/a.ts b/a.ts`, `index 1111111..2222222 100644`, `--- a/a.ts`, `+++ b/a.ts`.

- `parseDiff › numbers lines on each side`: `@@ -3,3 +3,4 @@ function f() {`, ` keep`, `-old`, `+new`, `+added`, ` end`, `` → lines `[hunk(text = header line, no numbers), context 'keep' 3/3, del 'old' old 4, add 'new' new 4, add 'added' new 5, context 'end' 5/6]`; `maxLineNo === 6`.
- `parseDiff › marks the changed part of a line that pairs with another`: `-const a = 1` / `+const a = 2` → both `changed = [10, 11]`.
- `parseDiff › marks nothing when the blocks differ in length or share nothing`: `-one -two +three` → no `changed`; `-abc +xyz` → no `changed`.
- `parseDiff › pairs lines across a missing-newline note`: `-let x = 1`, `\ No newline at end of file`, `+let x = 2` → `lines[2] = { kind: 'note', text: 'No newline at end of file' }` (no other keys); `lines[1].changed = lines[3].changed = [8, 9]`.
- `parseDiff › reads a blank context line that lost its space`: `@@ -1,2 +1,2 @@`, ``, ` b` → `lines[1] = { kind: 'context', text: '', oldNo: 1, newNo: 1 }`.
- `parseDiff › sums up a diff with no hunks`: `similarity index 100%`, `rename from a.ts`, `rename to b.ts` → `Renamed from a.ts without changes`; `old mode 100644`, `new mode 100755` → `File mode changed from 100644 to 100755`; `new file mode 100644` → `Empty file`; `''` → `No changes to show`; a diff with a hunk → `summary` undefined.

Recommended additions (not in TS, but pin behaviors above): `is_takeover` table tests (ACT-040/041), `keyterms` (VOICE-012), `error_text` WKE stripping (VOICE-017), `clock` (VOICE-036), tracking/branch labels (GIT-026/029), analyser band bins at 44.1/48 kHz (VOICE-022), `draft` argv snapshot (ACT-021).

## Rust mapping

- **nodus-actions** (no UI): `ACTION_ICONS`, `DEFAULT_ACTION_ICON`, `is_action_icon`, `clip` (UTF-16 semantics), `normalize_action`, `DRAFT_SYSTEM_PROMPT`, `DRAFT_SCHEMA_JSON` (store the exact string; don't rely on serde key order), `draft_action` (tokio `Command`, `stdin(Stdio::null())`, 120 s `tokio::time::timeout` + `kill()`, `CREATE_NO_WINDOW` on Windows, claude path + env from `nodus-platform`), `run_args`, `is_takeover` (use the `regex` crate with the same three patterns; `(?s)` for `[\s\S]`, lazy `*?`), `find_action`. `finish_run` / `claim_run` live in `nodus-core` (they touch board state, the PTY host and events) and call into `nodus-board`/`nodus-sessions`.
- **nodus-voice**: key source (`nodus-platform` secrets: decrypt OSCrypt ciphertext from `voiceKey`; env from the login-shell capture), `set_key`, `keyterms` (uses `nodus-git::status`), xAI client (`reqwest` with `multipart` + `rustls`; `Form::new().text("model", …)`, repeated `.text("keyterm", …)`, `.part("file", Part::bytes(..).file_name("prompt.webm").mime_str("audio/webm"))`; 90 s total timeout covering the body), result mapping. Recorder: `cpal` input stream → WebRTC APM for echo cancellation / noise suppression / AGC (`webrtc-audio-processing` crate) → Opus encoder (`opus`/`audiopus`, libopus) → WebM muxer (`webm` crate over libwebm, or a small EBML writer) so the upload stays `audio/webm` + `prompt.webm`. Meter: ring buffer of the last 256 samples, Blackman window, `rustfft`, |X|/N, exponential smoothing 0.7 per read, dB → byte with −100/−30, band means per VOICE-022. The headless voice controller (phases, run counter, single-listener slot, 600 ms floor, 5 min cap, Retry buffer, notice auto-hide) should live here too so 07, 11 and the session mic share it; the UI only renders it. Mic permission (`AVCaptureDevice authorizationStatusForMediaType:` / `requestAccessForMediaType:` via `objc2-av-foundation`) and opening the Settings URL belong in `nodus-platform`.
- **nodus-git**: runner (`std::process::Command` / tokio; prefix args; `GIT_TERMINAL_PROMPT=0`; read stdout and stderr concurrently with a byte cap, killing the child when exceeded to emulate Node's `maxBuffer`; non-zero exit is data), `status` (porcelain v2 `-z` parser), `log`, `diff` (toplevel, containment check with `path-clean`-style lexical resolution mirroring Node's `path.resolve`/`relative`, base revision with process-wide `OnceLock` cache, binary regex), `parse_diff` + changed spans (pure; re-exported by `nodus-core` for the UI). Sorting: `icu_collator` (ICU4X) with root/en locale to match `localeCompare`.
- **nodus-ui** (GPUI): `ActionRail` (edge strip, slide/opacity animation, hint tooltip, run fan, context menu, delete confirm), `ActionDialog` (two steps, icon grid, examples, scope switch), `ActionIcon` (bundle lucide 1.49.0 SVGs), `SessionMic`, `VoiceStatus`, `VoiceNotice`, `GitControls` + `BranchChip` popover, `DiffModal` (list + virtualized variable-height diff lines with selectable text). Precompute every `color-mix` (tables above); GPUI has no `backdrop-filter`, so the rail and notice blur need a platform vibrancy view or an approximation agreed in 08.
- **nodus-core**: commands and events of the API surface table; IPC-equivalent error formatting (ACT-083); git polling stays a UI concern (4 s timer while the window is active, refresh on window activation and on status events).

Risks:
1. **Audio pipeline parity**: reproducing Chromium's getUserMedia processing (AEC/NS/AGC), MediaRecorder's WebM/Opus output that xAI accepts, and the AnalyserNode maths for an identical meter; device sample-rate handling and permission prompts per OS.
2. **Saved key compatibility**: decrypting 0.9.4's `voiceKey` (Chromium OSCrypt as used by Electron `safeStorage`; plan D6 names Keychain "Nodus Safe Storage" on macOS and DPAPI on Windows — the exact ciphertext layout, e.g. the `v10` prefix, PBKDF2/AES-CBC on macOS and whether Windows goes through a `Local State` AES-GCM key, must be verified by 01) — any mismatch silently falls back to the env key while the source still says `settings`.
3. **Text semantics**: UTF-16 lengths for clips, keyterm length and diff spans; ICU `localeCompare` order; `toLocaleDateString` short dates; Node `path.parse/resolve/relative` semantics on both OSes.
4. **Process semantics**: Node `execFile` `maxBuffer` (per-stream byte cap, kill, distinct error), numeric-exit-as-answer, login-shell PATH resolution, SIGKILL timeout for `claude -p`, stdin closed.
5. **Behavioral quirks to keep**: Electron's IPC error prefix in user-visible strings (ACT-083), takeover regex edge cases (ACT-041), Esc not cancelling the session mic while a terminal is focused (VOICE-033), `opening` stuck on IPC failure (VOICE-037), selection memory in the diff list (GIT-038), stale diff shown while a new one loads (GIT-042).
6. **Performance**: a 2 MiB diff is tens of thousands of wrapped lines; GPUI needs a variable-height virtual list while keeping wrap, selection and the scroll-preserving re-read every 4 s.

## Parity checklist

- [x] ACT-001 Action shape
- [x] ACT-002 Where actions live
- [x] ACT-003 Icon list
- [x] ACT-004 Clip rule
- [x] ACT-005 Normalize · prompt
- [x] ACT-006 Normalize · id
- [x] ACT-007 Normalize · name
- [x] ACT-008 Normalize · icon
- [x] ACT-009 Normalize · description
- [x] ACT-010 Normalize · global
- [x] ACT-011 Normalize · createdAt
- [x] ACT-012 Normalize · key order
- [x] ACT-013 Global list command
- [x] ACT-014 Save command
- [x] ACT-015 Remove command
- [x] ACT-016 Draft command
- [x] ACT-017 Renderer list update
- [x] ACT-018 Global list loading
- [x] ACT-019 Empty description
- [x] ACT-020 Claude required
- [x] ACT-021 Draft argv
- [x] ACT-022 Draft user prompt
- [x] ACT-023 Draft process
- [x] ACT-024 Draft timeout
- [x] ACT-025 Draft spawn failure
- [x] ACT-026 Draft success
- [x] ACT-027 Draft failure reason
- [x] ACT-028 Run args
- [x] ACT-029 Starting a run (main)
- [x] ACT-030 Run node
- [x] ACT-031 Starting a run (renderer)
- [x] ACT-032 Runs are hidden from the board
- [x] ACT-033 Run tools
- [x] ACT-034 Finish trigger · status
- [x] ACT-035 Finish trigger · board load
- [x] ACT-036 finishRun guard
- [x] ACT-037 finishRun effects, in order
- [x] ACT-038 Run result fields
- [x] ACT-039 Finish notice
- [x] ACT-040 Takeover detection
- [x] ACT-041 Takeover quirks (keep)
- [x] ACT-042 Takeover hook
- [x] ACT-043 claimRun
- [x] ACT-044 Prompt into a run
- [x] ACT-045 Taken-over placement
- [x] ACT-046 Rail presence
- [x] ACT-047 Rail order
- [x] ACT-048 Run lookup
- [x] ACT-049 Reveal
- [x] ACT-050 Hide
- [x] ACT-051 Keep open
- [x] ACT-052 Shown state
- [x] ACT-053 Overlay flag
- [x] ACT-054 Hint
- [x] ACT-055 Hint text
- [x] ACT-056 Press
- [x] ACT-057 Running look
- [x] ACT-058 Global badge
- [x] ACT-059 Edge mark
- [x] ACT-060 Context menu
- [x] ACT-061 Edit / New
- [x] ACT-062 Delete
- [x] ACT-063 Run fan
- [x] ACT-064 Fan position
- [x] ACT-065 Fan card text
- [x] ACT-066 Fan card click
- [x] ACT-067 Dialog start
- [x] ACT-068 Dialog header
- [x] ACT-069 Describe step
- [x] ACT-070 Example chips
- [x] ACT-071 Scope switch
- [x] ACT-072 Draft
- [x] ACT-073 Stale drafts
- [x] ACT-074 Review step
- [x] ACT-075 Save
- [x] ACT-076 Footer · left
- [x] ACT-077 Footer · right
- [x] ACT-078 Dialog keys
- [x] ACT-079 Dialog close
- [x] ACT-080 Dialog error
- [x] ACT-081 Icon fallback
- [x] ACT-082 Rail independent of dialog
- [x] ACT-083 IPC error text (keep)
- [x] VOICE-001 Key source
- [x] VOICE-002 Env wait
- [x] VOICE-003 Save key
- [x] VOICE-004 Key storage
- [x] VOICE-005 Non-string argument
- [x] VOICE-006 Key used
- [x] VOICE-007 Key confinement
- [x] VOICE-008 Key load/migration
- [x] VOICE-009 Mic check
- [x] VOICE-010 Mic settings link
- [x] VOICE-011 No key
- [x] VOICE-012 Hint words
- [x] VOICE-013 MIME and file name
- [x] VOICE-014 Request
- [x] VOICE-015 Timeout
- [x] VOICE-016 Response body
- [x] VOICE-017 HTTP error
- [x] VOICE-018 Success
- [x] VOICE-019 Transcribe command
- [x] VOICE-020 Capture
- [x] VOICE-021 Encoding
- [x] VOICE-022 Level meter
- [x] VOICE-023 Stop / cancel
- [x] VOICE-024 Phases
- [x] VOICE-025 One mic at a time
- [x] VOICE-026 Start
- [x] VOICE-027 While recording
- [x] VOICE-028 Stop
- [x] VOICE-029 Transcribe
- [x] VOICE-030 Cancel
- [x] VOICE-031 Retry
- [x] VOICE-032 Dismiss
- [x] VOICE-033 Esc
- [x] VOICE-034 Notice timeout
- [x] VOICE-035 Unmount
- [x] VOICE-036 Clock
- [x] VOICE-037 Unhandled IPC failure (keep)
- [x] VOICE-038 Status (recording)
- [x] VOICE-039 Status (transcribing)
- [x] VOICE-040 Notice
- [x] VOICE-041 Session mic placement
- [x] VOICE-042 Session mic layout
- [x] VOICE-043 Session mic button
- [x] VOICE-044 Session mic recording look
- [x] VOICE-045 Session mic notice
- [x] GIT-001 Runner argv
- [x] GIT-002 Runner results
- [x] GIT-003 Env wait
- [x] GIT-004 Status argv
- [x] GIT-005 Not a repository
- [x] GIT-006 Status headers
- [x] GIT-007 Status entries
- [x] GIT-008 XY mapping
- [x] GIT-009 Status order
- [x] GIT-010 Status in use
- [x] GIT-011 Log argv
- [x] GIT-012 Log parse
- [x] GIT-013 Log limit
- [x] GIT-014 Diff root
- [x] GIT-015 Path check
- [x] GIT-016 Base revision
- [x] GIT-017 Diff argv
- [x] GIT-018 Too large
- [x] GIT-019 Diff exit
- [x] GIT-020 Binary
- [x] GIT-021 Shared read
- [x] GIT-022 Reset
- [x] GIT-023 Refresh triggers
- [x] GIT-024 Hidden
- [x] GIT-025 Changes button
- [x] GIT-026 Branch chip label
- [x] GIT-027 Log hover
- [x] GIT-028 Log fetch
- [x] GIT-029 Popover head
- [x] GIT-030 Popover commits
- [x] GIT-031 Relative time
- [x] GIT-032 Modal open
- [x] GIT-033 Fresh read
- [x] GIT-034 Modal header
- [x] GIT-035 Empty states
- [x] GIT-036 File list
- [x] GIT-037 Status icons
- [x] GIT-038 Selection
- [x] GIT-039 List input
- [x] GIT-040 Focus look
- [x] GIT-041 Diff loading
- [x] GIT-042 Uncached selection
- [x] GIT-043 Diff errors
- [x] GIT-044 Pane header
- [x] GIT-045 Pane body precedence
- [x] GIT-046 Scroll
- [x] GIT-047 Line rendering
- [x] GIT-048 Lines
- [x] GIT-049 Hunk header
- [x] GIT-050 Header lines
- [x] GIT-051 Hunk lines
- [x] GIT-052 Changed spans
- [x] GIT-053 maxLineNo
- [x] GIT-054 Summary trigger
- [x] GIT-055 Summary text
