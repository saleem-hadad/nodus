# 03 · Sessions & transcripts

Sources:
- `src/main/ipc.ts` (board:* and session:* handlers, `createSession`, `sendToSession`, `isTakeover`, `sessionTitle`, `clampSize`, `finishRun`, `claimRun`, `archiveIfAsked`, `findAction`)
- `src/main/discovery.ts` (transcript paths, line reader, metadata parser and cache, transcript preview, watcher)
- `src/main/board.ts` (board merge, archive layout, archive/drop/unstick, `toCard`, `sessionList`, `projectStats`, `hasTranscript`)
- `src/main/search.ts`, `src/main/search.test.ts`, `src/shared/search.ts` (⌘K session search)
- `src/main/summarize.ts` (handoff summaries for "new session from context")
- `src/main/naming.ts` (section naming)
- `src/shared/geometry.ts`, `src/shared/geometry.test.ts` (board sizes, free-spot search)
- `src/shared/sessions.ts` (`isEmptySession`)
- `src/renderer/src/components/board/stopSessions.ts`, `stopSessions.test.ts` (stop rules and confirmation)
- Launch-only excerpts read for context, owned elsewhere: `src/main/pty.ts` `commandFor`/`startSession`/`spawnSession`/`sendPrompt`/`kill` (02), `src/main/env.ts` `spawnEnv`/`terminalShell`/`userShell`/`requireClaude`/`gitBash` (01), `src/main/hooks.ts` `hookEnv`/`hookSettings`/`getStatus` (02), `src/main/boardTools.ts` `BOARD_MCP_CONFIG`/`BOARD_TOOL_RULES` (04), `src/main/actions.ts` `runArgs` (05), `src/main/views.ts` `brainArgs`/`startBrain`/`askBrain` (04), `src/main/store.ts` `getBoard`/`getProject` (01), `src/main/hostProtocol.ts` `windowsCommandLine` (02).
- `docs/how-it-works.md`: Sessions, Terminal cards, Expanded cards, Cards, New session from context, Known limitations.

## Summary

Nodus cards are ordinary Claude Code sessions. This domain covers:

1. **Discovery.** Reading Claude Code's transcripts in `~/.claude/projects/<encoded repo path>/<session id>.jsonl` into card metadata (title, prompts, last reply, branch, counts, times), incrementally and cached, and watching that folder (subagent transcripts included) to tell the board to reload.
2. **Board merge.** Combining what is on disk with the saved board in `state.json`: new sessions found on disk are imported as archived cards in free archive slots, deleted ones are forgotten, removed ones stay hidden forever. Archive layout (slot grid, growth), archiving, dropping empty sessions, unsticking notes.
3. **Session lifecycle.** Creating a card and starting its process (claude or a login shell), the exact `claude` command line (session id or resume, name, permission mode, board tools MCP config, allowed tools, extra system prompt, hook settings, first prompt), opening/resuming, typing input (and how typing takes over an action run), sending a prompt into a session, stopping it.
4. **Board persistence handlers.** Saving node patches, sections, notes, ink, archive rect, viewport; removing cards and notes; naming a section with Claude.
5. **Transcript reading** for the archived preview and the agent viewer.
6. **⌘K search** over prompts, replies and tool calls, subagents included, with scoring and snippets.
7. **Handoff summaries** (`claude -p --resume <id> --fork-session --no-session-persistence`, 3 at a time) and **section naming** (`claude -p --model haiku`).
8. **Shared geometry** (card/note/view/archive sizes, `findFreeSpot`, `findSpotBeside`) and the **stop rules** shared by the board and terminal modal.

Everything here is UI-free and lands in `nodus-sessions`, `nodus-transcripts` and `nodus-board`.

## Behaviors

### Transcript files and paths

- **SESS-001 · Transcript folder** — The transcripts of a project live in `<home>/.claude/projects/<encodeRepoPath(repoPath)>/`, where `<home>` is the OS home directory (`os.homedir()`: `$HOME` on macOS, `%USERPROFILE%` on Windows).
- **SESS-002 · Real repo path** — `realRepoPath(p)` resolves `p` with Node `fs.realpathSync` semantics (symlinks resolved component by component; the case of non-symlink components is kept as given; no `\\?\` prefix on Windows). If resolving fails, `p` is returned unchanged.
- **SESS-003 · Path encoding** — `encodeRepoPath(p)` = `realRepoPath(p)` with every UTF-16 code unit that is not `[a-zA-Z0-9]` replaced by `-` (regex `/[^a-zA-Z0-9]/g`). A character outside the BMP (two UTF-16 units) becomes `--`; a non-ASCII BMP character becomes one `-`. Example: `/work/my.repo` → `-work-my-repo`.
- **SESS-004 · Transcript path** — `transcriptPath(repoPath, id)` = `<transcript folder>/<id>.jsonl`.
- **SESS-005 · Session files** — Listing a project's sessions reads the transcript folder (non-recursive) and keeps only names matching `/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.jsonl$/i`; the session id is the name without `.jsonl`, case preserved. Order: sorted byte by byte on macOS and Linux (Node's `readdir` goes through libuv's `scandir`, which sorts names with `strcmp`), and in NTFS order on Windows. An unreadable or missing folder yields an empty list.
- **SESS-006 · Subagents folder** — `subagentsDir(repoPath, id)` = `<transcript folder>/<id>/subagents`.
- **SESS-007 · Agent files** — Listing a session's agents reads its subagents folder and keeps names matching `/^agent-([\w-]+)\.jsonl$/` (`\w` = ASCII `[A-Za-z0-9_]`), giving `{ agentId, file, metaFile: "<dir>/agent-<agentId>.meta.json" }` in directory order (sorted byte by byte on macOS and Linux (Node's `readdir` goes through libuv's `scandir`, which sorts names with `strcmp`), and in NTFS order on Windows). Unreadable or missing folder → empty list.
- **SESS-008 · Agent id shape** — `AGENT_ID` = `/^[\w-]+$/` with ASCII `\w`; it gates any agent or session id that ends up in a path for the agent viewer.
- **SESS-009 · Agents signature** — For the watcher, a session's subagents fingerprint is `` `${entryCount}:${totalBytes}:${latestMtimeMs}` `` over every entry of its subagents folder (any file, `.meta.json` included). Entries whose stat fails still count in `entryCount` but add no bytes/mtime. An unreadable folder gives `''`.

### Reading JSONL lines (`readLines`)

- **SESS-010 · Byte range read** — `readLines(fd, start, end, skipFirstPartial, onLine, keep?)` reads bytes `[start, end)` with positional reads, looping until the range is read or a read returns 0. If `end - start <= 0` it returns `start` and parses nothing.
- **SESS-011 · Skip partial first line** — With `skipFirstPartial`, everything up to and including the first `\n` is skipped. If there is no `\n` within the bytes read, it returns `start` and parses nothing.
- **SESS-012 · Complete lines only** — Only bytes up to the last `\n` are parsed. The return value is the absolute offset just past that last `\n`; if there is no `\n` at or after the parse start it returns `start + skipped` (the position after the skipped partial line, or `start`). A trailing line without `\n` is left for the next read.
- **SESS-013 · Line parsing** — The parsed bytes are decoded as UTF-8 (invalid sequences → U+FFFD) and split on `\n`. Empty lines are skipped; if `keep` is given, a raw line it rejects is skipped before parsing. Each remaining line is `JSON.parse`d; a parse error, or an error thrown while handling the value (e.g. the line is `null`), is silently ignored. A line that is valid JSON but not an object contributes nothing.

### Prompt text (`userText`)

- **SESS-014 · Meta lines** — A user line yields no prompt when `isMeta`, `isCompactSummary` or `isVisibleInTranscriptOnly` is truthy.
- **SESS-015 · Origin** — When `origin.kind` is present and is not `'human'`, the line yields no prompt, unless the caller passes `anyOrigin` (agent transcripts).
- **SESS-016 · Content shapes** — `message.content` as a string is the text. As an array: if any block has `type === 'tool_result'` the line yields no prompt; otherwise the `text` of blocks with `type === 'text'` and a string `text` are joined with `\n`. Any other content shape gives `''`.
- **SESS-017 · Empty** — The text is trimmed; empty → no prompt.
- **SESS-018 · Slash commands** — If the trimmed text starts with `<command-name>` or `<command-message>`: name = first `/<command-name>([\s\S]*?)<\/command-name>/` capture, trimmed; args = first `/<command-args>([\s\S]*?)<\/command-args>/` capture, trimmed. No name → no prompt. Otherwise the prompt is `name` (prefixed with `/` if it doesn't start with `/`) followed by `' ' + args` when args is non-empty, flagged `isCommand: true`. This check runs before the noise check.
- **SESS-019 · Noise prefixes** — A trimmed text starting with any of `<local-command-`, `<system-reminder>`, `Caveat:`, `[Request interrupted`, `<task-notification`, `<bash-input>`, `<bash-stdout>`, `<bash-stderr>`, `<user-prompt-submit-hook>`, `Base directory for this skill`, `This session is being continued from a previous conversation` yields no prompt.
- **SESS-020 · Reminders stripped** — Otherwise every `/<system-reminder>[\s\S]*?<\/system-reminder>/g` block is removed and the result trimmed; empty → no prompt; else `{ text, isCommand: false }`.
- **SESS-021 · Prompt summary** — For titles and card previews, `promptSummary` replaces each `/<pasted_content[^>]*>[\s\S]*?<\/pasted_content[^>]*>/g` block with `[Pasted text]`, then removes any leftover `/<\/?pasted_content[^>]*>/g` tag.
- **SESS-022 · Prompt full** — For the transcript preview and search, `promptFull` removes the `/<\/?pasted_content[^>]*>/g` tags only (pasted content kept) and trims.
- **SESS-023 · Truncate** — `truncate(text, max)`: every `/\s+/` run (JS `\s`, Unicode whitespace incl. U+FEFF) becomes one space, then trim. If the length in UTF-16 code units exceeds `max`, the result is the first `max - 1` units, right-trimmed, plus `…` (U+2026).

### Assistant text and tool calls

- **SESS-024 · String reply** — `assistantParts` of an assistant line whose `message.content` is a string: `text` = that string trimmed; no tools, no uses, `handback: false`. Non-string non-array content: empty text, no tools.
- **SESS-025 · Block reply** — For array content: texts = `text` of `type === 'text'` blocks with string `text`, in order. Each `type === 'tool_use'` block with a string `name`: if the name is `SubagentHandback`, `handback = true` and, when `input.message` is a string, it is appended to the texts (after all text blocks); any other tool_use block goes into `uses`. `text` = texts joined with `\n`, trimmed; `tools` = the names of `uses`.
- **SESS-026 · Tool display name** — `toolName(n)`: if `n` starts with `mcp__`, the part after the last `__` (or `n` itself if that part is empty); else `n`. `mcp__nodus__create_note` → `create_note`.
- **SESS-027 · Tool call summary** — `describeToolUse(use)` → `{ name: toolName(name), detail }`, where `input` is `use.input` if it is an object (else `{}`), a field counts only if it is a string that is non-empty after trimming (the untrimmed value is used), `file` = `file_path` ?? `notebook_path`, and `detail` = first of `command`, basename of `file`, `pattern`, `url`, `query`, `description`, `skill`, `path`, `prompt`, then `truncate(detail, 160)`; `detail` is omitted when none is present.
- **SESS-028 · Timestamps** — `timeOf(line)` = `Date.parse(line.timestamp)` when `timestamp` is a string and parses (integer ms since epoch); otherwise none.

### Metadata parsing (`applyLine`)

- **SESS-029 · Created/updated** — Every line with a timestamp, of any type, lowers `createdAt` to the minimum and raises `updatedAt` to the maximum seen.
- **SESS-030 · Branch** — Every line, of any type, with a non-empty string `gitBranch` sets the branch (last one wins).
- **SESS-031 · Custom title** — A `type: 'custom-title'` line with a string `customTitle` that is non-empty after trim sets the custom title (trimmed; last one wins).
- **SESS-032 · AI title** — A `type: 'ai-title'` line with a non-empty trimmed string `aiTitle` sets the AI title (trimmed; last one wins).
- **SESS-033 · Legacy summary title** — A `type: 'summary'` line with a non-empty trimmed string `summary` sets the AI title only if none is set yet; a later `ai-title` line overrides it.
- **SESS-034 · Sidechain flag** — The first `user` or `assistant` line that has a boolean `isSidechain` fixes the file's sidechain flag; later lines don't change it.
- **SESS-035 · Prompts** — A `user` line for which `userText` (human origin only) yields a prompt increments `prompts` and, if it has a timestamp, sets `lastPromptAt = max(lastPromptAt ?? 0, t)`. A command sets `firstCommand` if unset. A non-command sets `firstPrompt` if unset and always sets `lastPrompt`, both as `promptSummary(text)`.
- **SESS-036 · Replies** — An `assistant` line sets `hasAssistant` when it has text or tools. Only lines with text continue: `lastAssistantText` = text; `replies` increments unless the line's `message.id` is set and equals the remembered id; then, when the line has an id, it becomes the remembered id (a text line without an id counts and leaves the remembered id as it was). Tool-only lines neither count nor change the remembered id.
- **SESS-037 · Sidechain lines counted** — The metadata parser does not skip `isSidechain` lines in a session's own transcript; they count like any other.
- **SESS-038 · Other records** — Lines of any other `type` only contribute their timestamp and `gitBranch`.

### Metadata cache

- **SESS-039 · Cache hit** — Parsed state is cached per file path for the life of the app; when the file's size and mtime (ms, float) both equal the cached ones, the cached state is used without reading.
- **SESS-040 · Append-only growth** — When the file is larger than cached, only the bytes from the cached offset to the new size are parsed (no partial-line skip), continuing the cached state; size, mtime and offset are updated.
- **SESS-041 · Fresh parse** — With no cache, a smaller file, or the same size with a different mtime, the file is parsed from scratch.
- **SESS-042 · Large files** — A fresh parse of a file over 4 MiB (`4 * 1024 * 1024`) reads only the first 256 KiB (`256 * 1024`, no skip, its trailing partial line dropped) and the last 1 MiB (`1024 * 1024`, first partial line skipped); the offset is the tail's end. The middle is never read, so counts, first/last prompts and times come from head and tail only.
- **SESS-043 · Unreadable file** — If the file can't be stat'ed, its cache entry is dropped and there is no metadata. If it can be stat'ed but not opened, the stale cached state is returned (or none).

### Session metadata (`readSession`)

- **SESS-044 · Title precedence** — `title` = custom title, else AI title, else `truncate(firstPrompt ?? firstCommand, 80)`, else `Untitled session`. Custom and AI titles are not truncated.
- **SESS-045 · Preview fields** — `firstPrompt` and `lastPrompt` are truncated to 500, `lastAssistantText` to 400 (SESS-023); absent when unset. `gitBranch` as parsed.
- **SESS-046 · Message count** — `messageCount = prompts + replies` (slash commands count as prompts; tool results, meta and noise don't count).
- **SESS-047 · Time fallback** — `createdAt`/`updatedAt` fall back to the file's mtime in ms (which may be fractional) when no line had a timestamp. `transcriptPath` is the file.
- **SESS-048 · Eligible** — A session is eligible for import when its sidechain flag is not `true`, it has at least one prompt, and `hasAssistant`.
- **SESS-049 · Has content** — `hasContent` = at least one prompt, or a custom title, or an AI title. Cards only carry `meta` when `hasContent`.
- **SESS-050 · Scan** — `scanRepo(repoPath)` returns, in directory order (see SESS-005), every session file whose metadata could be read, keyed by session id, with `{ meta, eligible, hasContent, lastPromptAt }`.
- **SESS-051 · Meta signature** — `metaSignature(meta)` = `[title, messageCount, updatedAt, gitBranch, lastAssistantText?.length].join('|')` (missing values become empty strings).
- **SESS-052 · Empty session** — `isEmptySession(meta)` is true when there is no meta, or it has neither `firstPrompt` nor `lastAssistantText` (nothing typed, slash commands aside, and no reply).

### Transcript preview (`readTranscript`)

- **SESS-053 · Missing file** — A transcript that can't be stat'ed reads as `[]`. One that can be stat'ed but not opened fails the call with the OS error.
- **SESS-054 · Tail only** — Only the last 16 MiB (`16 * 1024 * 1024`) is read; when that starts after byte 0 the first partial line is skipped.
- **SESS-055 · Sidechain** — Lines with truthy `isSidechain` are skipped, except in agent mode (subagent transcripts, where every line is a sidechain line).
- **SESS-056 · User messages** — A `user` line: in agent mode, a task notification first becomes `{ role: 'user', text: <summary>, timestamp, notice: true }`. Otherwise `userText(line, anyOrigin = agent)`; if it yields a prompt, push `{ role: 'user', text: promptFull(text), timestamp }`. Either push ends the current assistant turn. Lines yielding nothing are skipped and don't end the turn.
- **SESS-057 · Task notifications** — A line is a task notification when `origin.kind === 'task-notification'` or its message text (string content, or every block's string `text` joined with `\n`) contains `<task-notification>`. Its text is the trimmed first `/<summary>([\s\S]*?)<\/summary>/` capture, or `A background task finished` when missing or empty.
- **SESS-058 · Assistant turns** — An `assistant` line with neither text nor tools is skipped (and doesn't update the last message id). Consecutive assistant lines merge into one turn `{ role: 'assistant', text, timestamp, tools }` whose timestamp is its first line's. A line's text is appended to the turn's text with `\n` when its `message.id` is set and equals the previous kept assistant line's id, else `\n\n`; the first text is used as is. The last id is then set from this line (possibly to none).
- **SESS-059 · Agent steps** — In agent mode, a turn also keeps `calls` (`describeToolUse` of each use, in order), and a line with text arriving after the turn already has calls starts a new turn.
- **SESS-060 · Tools** — Each kept assistant line's tool names are appended to the turn's `tools`. Empty `tools` and `calls` arrays are removed from the result.
- **SESS-061 · Timestamps default** — Messages without a parseable timestamp get `timestamp: 0`.
- **SESS-062 · Message cap** — Only the last 500 messages are returned.

### Watching transcript folders

- **SESS-063 · One watch per project** — `watchProject(projectId, repoPath, onChange)` does nothing when the project is already watched for the same `repoPath` (the first `onChange` stays); a different `repoPath` replaces the watch.
- **SESS-064 · Baseline** — A new watch first records every session's signature (SESS-067) without emitting.
- **SESS-065 · Recursive watch** — When the transcript folder exists, it is watched recursively (so `<id>/subagents/` changes count). Any event schedules a check 500 ms later unless one is already scheduled; further events do not push it back.
- **SESS-066 · Throttle** — A check that runs within 1500 ms of the last emit reschedules itself for the remaining time instead of comparing.
- **SESS-067 · Change test** — A check rescans all sessions (cached metadata) and builds `` `${metaSignature(meta)}|${agentsSignature}` `` per session. It is a change when the number of sessions differs or any session's signature differs from the stored one. The new signatures are stored either way; on a change the emit time is recorded and `onChange` runs, which sends `ev:board-changed(projectId)`.
- **SESS-068 · Folder not there yet** — If the folder doesn't exist (or watching it throws), it is polled every 3000 ms; once a watch attaches, polling stops and a check is scheduled (SESS-065).
- **SESS-069 · Watch error** — If the watcher reports an error it is closed and not re-attached, and no polling restarts; that project's board gets no more change events until it is unwatched and watched again (app restart or project removal).
- **SESS-070 · Lifetime** — Watches are started by `board:load` and stay after the user leaves the board. `unwatchProject` (on project removal, 01) and `unwatchAll` (on app shutdown, 01) close the watcher, stop polling and cancel a pending check.

### Loading a board (`board:load`, `loadBoard`)

- **SESS-071 · Load order** — `board:load(projectId)`: (1) tells capture this board was opened (`boardOpened`, 07); (2) looks up the project (`Unknown project <id>` if missing); (3) finishes every node with `runOf` whose process isn't live (SESS-155, SESS-156); (4) `loadBoard`; (5) `watchProject` with `onChange` sending `ev:board-changed(projectId)`; (6) returns the snapshot.
- **SESS-072 · Forget deleted transcripts** — Only when the transcript folder exists: every node not `createdByApp` whose id isn't among the sessions on disk and whose process isn't live is deleted from the board. Cards started in the app (terminals included) are never forgotten this way; a missing folder never wipes the layout.
- **SESS-073 · What gets imported** — Sessions on disk that are eligible (SESS-048), not already a node, and not in `hidden` are imported, sorted by `meta.updatedAt` descending (stable: ties keep directory order).
- **SESS-074 · Import placement** — Imported sessions go into archive slots: slots already taken are those computed from every archived node's position (SESS-084); then for each import in order, the lowest free slot index (counting up from 0) is used and marked taken. Each import is the node `{ sessionId, x, y, archived: true, createdAt: meta.createdAt }` (no `kind`, `title`, `createdByApp`).
- **SESS-075 · Archive grows** — After importing, the archive grows to fit (SESS-085).
- **SESS-076 · Section cleanup** — A node keeps `sectionId` only when it is not archived and that section still exists; otherwise `sectionId` is removed.
- **SESS-077 · Sticker cleanup** — A note whose `sessionId` names a node no longer on the board loses its `sessionId` (and stays where it is).
- **SESS-078 · Opened time** — The project's `openedAt` is set to now and the state is saved (debounced, 01).
- **SESS-079 · Cards** — Every node (archived, terminals, runs and views' brains included) becomes a card via `toCard(node, diskEntry)`, then `attachAgents(card, repoPath, diskEntry?.lastPromptAt)` (02) adds `agentCount`/`agents`. Cards come in node insertion order.
- **SESS-080 · Card shape** — `toCard(node, found?)` = all node fields, plus `meta` (only when `found.hasContent`), `status` (`'done'` when archived, else the status machine's `getStatus`, which is `'done'` for a process that isn't live), `live` (process running; an archived card can be live) and `usage` (02's `getUsage`, may be absent).
- **SESS-081 · Snapshot** — The snapshot is `{ project, cards, archive, sections, notes, views, ink, viewport }`: sections, notes and views each sorted by `createdAt` ascending (notes oldest first so newer draw on top), `ink` = stored ink or `[]`, `viewport` as stored (may be absent), `archive` the (grown) archive rect.

### Archive layout

- **SESS-082 · Archive constants** — Archive layout uses card 288 × 216, gap 24, side padding 40, top padding 64, bottom padding 40 (these differ from the shared geometry's gap of 32). A new board's archive is `{ x: 0, y: 0, w: 1928, h: 320 }` (`2*40 + 6*288 + 5*24` wide, store default, 01).
- **SESS-083 · Columns and slots** — `columns = max(1, floor((archive.w - 80 + 24) / 312))`. Slot `i` is at `x = archive.x + 40 + (i % columns) * 312`, `y = archive.y + 64 + floor(i / columns) * 240`.
- **SESS-084 · Slot of a card** — An archived card's slot is `col = round((x - archive.x - 40) / 312)`, `row = round((y - archive.y - 64) / 240)`; it has none when `col < 0`, `col >= columns` or `row < 0`; else `row * columns + col`.
- **SESS-085 · Growing** — The archive never shrinks: `right = max(archive.x + archive.w, every archived node's x + 288 + 40)`, `bottom = max(archive.y + max(archive.h, 320), every archived node's y + 216 + 40)`; `w = right - x`, `h = bottom - y`.
- **SESS-086 · Archiving outside the board** — `archiveNode(board, node)` (used by board tools and archive_self) does nothing for an archived node; otherwise it stops the process (not awaited), moves the node to slot 0 of the archive, sets `archived: true` and `archivedAt` = now, and removes `sectionId` and `expanded`. It doesn't save; callers do.
- **SESS-087 · Empty session test** — `isEmpty(project, node)` is false for terminals; otherwise `isEmptySession` of the node's transcript metadata (a missing transcript is empty).
- **SESS-088 · Dropping** — `dropNode(board, node)` unsticks its notes (SESS-089), deletes the node, and stops its process, forgetting its status once the process is gone (`forgetStatus`, 02). It does not add to `hidden`.
- **SESS-089 · Unsticking notes** — When a card leaves the board, each note with `sessionId` equal to it loses `sessionId`; if the card wasn't archived, the note moves to the card's `x`/`y`; from an archived card the note keeps its own position.

### Saving node patches (`board:save-nodes`)

- **SESS-090 · Unknown nodes** — Patches for session ids not on the board are skipped.
- **SESS-091 · Empty sessions are dropped** — A patch with `archived: true` for a node not yet archived whose session is empty (SESS-087) drops the node instead (SESS-088), and the rest of that patch is ignored.
- **SESS-092 · Position** — `x`/`y` are applied when they are finite numbers (not rounded).
- **SESS-093 · Title** — A string `title` is trimmed; an empty result removes the custom title.
- **SESS-094 · Archived flag** — A boolean `archived` applies only to nodes that are not terminals and not views' brains. Going from not archived to archived stops the process (not awaited, status not forgotten) and sets `archivedAt` to now. Un-archiving keeps `archivedAt`.
- **SESS-095 · Section** — `sectionId: null` removes the section; a string naming an existing section sets it; any other string is ignored.
- **SESS-096 · Expanded** — `expanded: true` sets `expanded: true`; `false` removes the field.
- **SESS-097 · Size** — Finite `w`/`h` are stored as `round(min(4000, max(200, n)))`.
- **SESS-098 · Archived cards fold** — After the patch, an archived node loses `sectionId` and `expanded`.
- **SESS-099 · Save and notify** — The state is saved after all patches; `ev:board-changed(projectId)` is sent only if some node was dropped.

### Other board handlers

- **SESS-100 · Save section** — `board:save-section(projectId, section)` ignores input without a string `id` or with non-finite `x`/`y`. Otherwise it stores `{ id, name: trimmed string or '', color: one of the folder colours else 'blue', x, y, createdAt: existing createdAt ?? now }` and saves.
- **SESS-101 · Remove section** — `board:remove-section` deletes the section, removes `sectionId` from its nodes (they stay where they are) and saves.
- **SESS-102 · Name section sources** — Since frames (added after 1.0.14) a frame is named after anything in it, and the prompt and error changed: see spec 17, FRAME-031. 0.9.4: `board:name-section(projectId, sessionIds)` waits for the login-shell env (01), then for each id whose node exists and isn't a terminal: title = the node's custom title, else the transcript's title when the transcript `hasContent`; if there is a title, `{ title, firstPrompt }` (firstPrompt from that meta) is a source. No sources → error `These sessions have nothing to name the section after yet`. Else resolves with `nameSection(sources)` (SESS-109).
- **SESS-103 · Save note** — `board:save-note(projectId, note)` ignores input without a string `id` or with non-finite `x`/`y`. Otherwise it replaces the stored note with exactly `{ id, text, color, x, y, w, h, sessionId, from, createdAt }`: `text` = string sliced to 20 000 UTF-16 units (no trim) or `''`; `color` = a folder colour else `'yellow'`; `x`/`y` JS-rounded; `w`/`h` = `round(min(1200, max(120, n)))` when finite, else absent; `sessionId` kept only if it is a string naming a node on this board; `from` = the stored note's `from` (never the renderer's); `createdAt` = stored or now. `fromProject` and `fromTitle` of the stored note are not carried over (they are lost on the first save from the renderer). Then saves.
- **SESS-104 · Remove notes** — `board:remove-notes(projectId, noteIds)` deletes each id and saves.
- **SESS-105 · Save ink** — `board:save-ink(projectId, strokes)` replaces the board's ink with `cleanInk(strokes)` (04) and saves.
- **SESS-106 · Archive rect and viewport** — `board:save-archive(projectId, rect)` and `board:save-viewport(projectId, viewport)` store the value as given, without validation, and save.
- **SESS-107 · Remove a card** — `board:remove-node(projectId, sessionId)`: if the node exists its notes are unstuck (SESS-089); the node is deleted; unless it was a terminal, the id is appended to `hidden` if not already there (also when no node existed); the state is saved; then the call waits for the process to stop (SESS-148) and forgets its status. The transcript is never touched.
- **SESS-108 · Hidden is forever** — `hidden` only grows (remove-node, finished action runs); a hidden session is never imported again and isn't counted as undiscovered.

### Naming a section with Claude (`nameSection`)

- **SESS-109 · Naming call** — Runs `claude` (the resolved binary, `requireClaude`) with the argv in *Data & formats › Section naming*, cwd = the OS temp folder, env = `spawnEnv()` (01), stdin ignored, stdout/stderr piped, no console window on Windows.
- **SESS-110 · Naming prompt** — The prompt is `Name this group of sessions:\n\n` followed by one entry per source joined with `\n`: `` `${i + 1}. ${title}` ``, or, when the source has a first prompt whose clipped form (SESS-111, 280) differs from the title, `` `${i + 1}. ${title}\n   Opening prompt: ${clipped}` ``.
- **SESS-111 · Clip** — Naming's `clip(s, max)` collapses `/\s+/` runs to one space and trims; if longer than `max` UTF-16 units, the first `max - 1` units plus `…` (no right-trim, unlike `truncate`).
- **SESS-112 · Clean name** — From the reply: trim, take the first line (split on `\n`), remove a leading `/^(name|title)\s*:\s*/i`, remove every character in ``[*_`#"“”]``, strip leading/trailing `'` runs (`/^'+|'+$/g`), strip a trailing `/[.!?:;,]+$/` run, collapse `/\s+/` to one space, trim; if longer than 48 units, the first 48 units right-trimmed.
- **SESS-113 · Naming result** — After 30 000 ms the process is SIGKILLed and the call fails with `Naming timed out`. A spawn error fails with that error. On exit, stdout (trimmed) is parsed as JSON; if it parsed, `is_error` is falsy, `result` is a string and its clean name is non-empty, that name is the result. Otherwise the call fails with: `result` when it is a non-empty string, else the last 3 lines of trimmed stderr joined with spaces, else `` `claude exited with code ${code}` `` (`null` when killed by a signal).

### Creating a session (`session:create`, `createSession`)

- **SESS-114 · Create start** — Waits for the login-shell env (01), looks up the project (`Unknown project <id>`), and makes a new lowercase v4 UUID as the session id.
- **SESS-115 · Action lookup** — `terminal` = `opts.kind === 'terminal'`. For a non-terminal with `actionId`, the action is the first with that id among the project's own actions, then the global ones. If `opts.actionId` is set and no action was found (including any terminal given an `actionId`), the call fails with `That action no longer exists` before anything is stored.
- **SESS-116 · New node** — The node is `{ sessionId, kind: 'terminal' | absent, x: opts.x, y: opts.y, archived: false, title: opts.name trimmed or absent, parents: opts.parents when non-terminal and non-empty, createdByApp: true, runOf: action id or absent, expanded: true when opts.expanded is truthy else absent, autoMode: true when non-terminal and opts.autoMode truthy else absent, fromView: opts.fromView when non-terminal, a string, and an own key of this board's views else absent, h: 960, createdAt: now }`. A new card so expands to 720 × 960. `x`/`y` aren't validated or rounded; `parents` aren't validated. A terminal keeps the trimmed `name` as its title.
- **SESS-117 · Action title** — For an action run, the node's title is the action's name (overriding `opts.name`).
- **SESS-118 · Node before process** — The node is added to the board before the process starts; if starting fails, the node is removed and the error is the call's error (nothing saved).
- **SESS-119 · Create launch options** — The process starts with: cwd = project folder (a card's worktree on a server project from GitHub, SESS-206); kind = node kind; `resume: false`; `name` = node title for Claude sessions (none for terminals); first prompt = the action's prompt for a run, else `opts.initialPrompt` trimmed (none if empty), none for terminals; permission mode `auto` for a run or `autoMode` node, else default; extra args = `runArgs(action)` for a run; no size (defaults 120 × 36).
- **SESS-120 · Create result** — After the process started, the state is saved and the call returns `toCard(node)` with no `meta`; its status is `working` when a first prompt was given, else `done` (initial status, SESS-129), and `live: true`.

### How processes are launched

- **SESS-121 · Claude argv** — A Claude session runs the resolved `claude` binary with, in this order: `--session-id <id>` for a new session or `--resume <id>` to resume; `--name <name>` when a name is given; `--permission-mode <mode>`; `--mcp-config <BOARD_MCP_CONFIG> --allowedTools <12 rules>` when the session host relays board tools; the extra args; `--settings <hookSettings()>`; and `-- <prompt>` when there is a first prompt. Templates verbatim in *Data & formats*.
- **SESS-122 · Permission mode** — The mode is `auto` when asked for (action runs, `autoMode` nodes, views' brains), else `acceptEdits`.
- **SESS-123 · Board tools gate** — `--mcp-config` and `--allowedTools` are added only when the connected session host's welcome said `mcp: true` (02); a host from an older app gets neither. Both lists are followed by another option so neither swallows the prompt.
- **SESS-124 · Prompt after `--`** — The first prompt goes after `--` so a prompt starting with `-` isn't read as an option.
- **SESS-125 · Name only at creation** — `--name` is only passed by `session:create` (and only for Claude sessions with a title). Opening, resuming, sending and views' brains never pass `--name`.
- **SESS-126 · Claude env** — A Claude process's env is `spawnEnv` (01: the login-shell env minus nesting markers, with `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG` defaulting to `en_US.UTF-8`) plus, on Windows when Git Bash is found and the user's env has no `CLAUDE_CODE_GIT_BASH_PATH`, `CLAUDE_CODE_GIT_BASH_PATH=<bash path>`, plus `hookEnv(id)` (02): `NODUS_PORT`, `NODUS_TOKEN`, `NODUS_SESSION=<id>`, and `NODUS_HOOK_CONFIG` when the host gave a hook config file. These added vars override same-named ones from the base env (any inherited `NODUS_*` was already stripped by `spawnEnv`).
- **SESS-127 · Terminal shell** — A terminal runs the user's shell as a login shell: macOS `<shell> -l` where shell = the login env's `SHELL` (or the app's own `SHELL`) when it is an absolute path, else `/bin/zsh`; Windows `pwsh.exe` found on `PATH`, else `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` (SystemRoot default `C:\Windows`), with `-NoLogo`. Its env is `spawnEnv()` with no `NODUS_*` (no hooks, no board tools), and resume, name, first prompt, permission mode and extra args are ignored.
- **SESS-128 · Process placement** — Every process runs in the project folder (`repoPath`; on a server, a card's worktree, SESS-206) inside a pseudo-terminal named `xterm-256color`, sized `max(20, cols ?? 120)` × `max(5, rows ?? 36)` (02).
- **SESS-129 · Initial status** — A process started with a first prompt starts as `working`; otherwise `done` (02's status machine).
- **SESS-130 · One process per session** — Starting a session that is already live does nothing; a start already in flight for the same id is shared rather than repeated (02).
- **SESS-131 · Host first** — Starting a process connects to the session host first, starting one if none runs (the hook endpoint in the command comes from the host) (02). Host errors fail the start (see Edge cases).
- **SESS-132 · Action run args** — An action run's extra args are `--append-system-prompt <runSystemPrompt(action)>` (verbatim in *Data & formats*), only on the run's first start.
- **SESS-133 · Brain args** — A view's brain gets `--append-system-prompt <brainPrompt(view)>` every time it starts or resumes, via `session:open`, `session:send` and 04's `startBrain`/`askBrain`; no args when the view is gone.
- **SESS-134 · Brain launches** — 04 starts a brain with node `{ title: "View: <view title>", createdByApp: true, viewOf, autoMode: true, x/y of the view }`, `resume: false`, permission `auto`, optional first prompt; and resumes one with `resume` = transcript exists, the asked prompt as first prompt, permission `auto`. Neither passes `--name`.

### On a server (`nodus serve`, hosted spec SRV-120…145)

None of this runs in the app: it's what a server's core does, given the server's `Hosting`. The app's sessions launch exactly as above.

- **SESS-206 · A worktree per card** — A new card (Claude or terminal) on a server project added from GitHub (the project has `github: { fullName, defaultBranch, installationId }`) gets its own worktree before its process starts: `git fetch --prune origin` in the main checkout (which is never edited; a failed fetch is logged and the branch is cut from what was fetched last), then `git worktree add -b <branch> <projects/<slug>/worktrees/<first 8 of the id>> origin/<defaultBranch>`, where the branch is `nodus/<title slug>-<first 4 of the id>` (lower-case ASCII letters and digits joined by `-`, at most 40, `session` or `terminal` when the title has none; the id's next 4 are added when that branch exists). The card records `worktree: { path, branch }` and a `previewPort`, and every start of its process, from any call, runs in the worktree. Views' brains run in the main checkout. A worktree that can't be made fails the create as SESS-118 does.
- **SESS-207 · Trust** — The worktree and the main checkout are marked trusted for Claude Code (the host's `trust`) before the first start, so a new card never stops at the trust dialog.
- **SESS-208 · Setup** — When the worktree has an executable `.nodus/setup`, the card gets `setup: { state: "running", line }` and `session:create` answers at once; the script runs in the worktree with the card's environment (SESS-209), its output in `projects/<slug>/setup-logs/<id8>.log`, whose last non-empty line (colours stripped, 200 units at most) becomes `line` as it changes (`board-changed`). Every start of the card's process waits for it. Success removes `setup`; failure (or 30 minutes) leaves `setup: { state: "failed", line }` with the last line, `Setup failed (exit N)` or `Setup took too long`, and the process starts anyway.
- **SESS-209 · Server environment** — The base env is `<data>/env`'s, with the shims folder (`/opt/nodus/bin/shims`, or `NODUS_SHIMS`) first on `PATH` when it exists, and on Linux `DOCKER_HOST=unix:///run/user/<run home's owner uid>/docker.sock` unless the file sets it. On top of SESS-126 (and SESS-127 for a terminal), every session gets `NODUS_SESSION`, `NODUS_PORT` and its own `NODUS_TOKEN` (SESS-210); one on a project from GitHub also gets `GIT_CONFIG_COUNT=3` with `credential.helper=` (dropping every other helper), `credential.helper=!'<nodus>' git-credential` and `credential.useHttpPath=true`, `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` = its starter's name (else login) and `<githubId>+<login>@users.noreply.github.com` (the bot's, below, for a card nobody started), and `GIT_COMMITTER_NAME`/`GIT_COMMITTER_EMAIL` = the App's bot, `<slug>[bot]` and `<botId>+<slug>[bot]@users.noreply.github.com`; a card with a preview port gets `PORT` and `NODUS_PREVIEW_PORT`. No GitHub token is ever in it.
- **SESS-210 · One token per session** — A session's `NODUS_TOKEN` is its own (an HMAC of its id with the server's `session-key`, so it holds across restarts of `serve`), sent to the host as the spawn's `sessionToken`. The host writes it to the session's own curl config (`NODUS_HOOK_CONFIG`, `hooks/<id>.curlrc` beside its socket, removed when it exits) and takes a hook, status line or board tools request for that session only with that token, never the shared one; the server checks it again on what the host relays, and drops the rest. The app's sessions keep the shared token.
- **SESS-211 · Transcripts by folder** — A card in a worktree writes its transcript under the worktree's encoded path: loading the board, the board tools' lists, stats, `has transcript`, previews, agents, titles and summaries read each card's from its own folder, and its folder is watched too. Sessions found on disk are imported (SESS-073) only from the main checkout's folder.
- **SESS-213 · A Claude account from the pool** — Every start of a Claude card's process on a server (a view's brain too; not a terminal) takes an account from the server's pool (hosted spec SRV-101: `ok`, the lowest five-hour use, unknown counting as 0, then the least recently used), and gets its token as `CLAUDE_CODE_OAUTH_TOKEN` in the spawn's environment, nowhere else. The card records `account` (its id) and `accountLabel` (`board-changed` when either changes). With no account to take, the start fails with the pool's words: `No Claude accounts yet. Add one in Settings → Servers.`, `Every Claude account is at its limit until <HH:MM UTC>.` (the day too when it's 20 hours or more away) or `Every Claude account has stopped working. Add one in Settings → Servers.` Every background `claude -p` the server's core runs (names, summaries, action drafts, forks) takes an account the same way, through its runner, unless its environment already holds a token. `NODUS_PORT` always comes from the connected host's hook endpoint, never a value read before the first connect (the first card after a start once got port 0).
- **SESS-214 · Moving a session** — When a session's turn fails with `rate_limit` (its `StopFailure`, 02 HOST-226) and some account is `ok`, the server stops its process (`session:kill`) and sends it `Continue.` as `session:send` does (SESS-145: resumed, `--resume <id>`, with the prompt), which takes another account; the card's `accountNote` becomes `Moved to <label>` (`Resumed on <label>` when it's the same account). With none `ok`, the process is left as it is, the note reads `Waiting for a Claude account until <HH:MM UTC>`, and once an account is `ok` (its window reset, or one was added) the waiting sessions resume that way, oldest first. Removing an account moves its live sessions the same way, with `Continue.` only for one mid-turn (working or waiting) and a plain resume for the rest; with nowhere to go, those mid-turn are stopped and wait, the rest are stopped.
- **SESS-212 · Removing a card** — `board:remove-node`, or a card dropped rather than archived (SESS-091), removes its worktree once its process has stopped. First, when it has uncommitted changes or commits no remote has, they're saved to `projects/<slug>/saved/<cardId>-<yyyymmdd-HHMMSS>/`: `changes.patch` (`git add -A`, then `git diff --binary --cached HEAD`) and `branch.bundle` (the branch's commits no remote has, or its last commit when none). Then `git worktree remove --force` and `git branch -D`. Saved work older than 30 days goes then too. When saving fails the worktree is kept. Archived cards keep their worktrees.

### Opening a session (`session:open`)

- **SESS-135 · Open a live session** — If the process is live, it is resized to `cols × rows` and the call returns `{ replay: <buffered output>, live: true }` (02's snapshot, flushed first). No project lookup happens.
- **SESS-136 · Open a stopped session** — Otherwise it waits for the env, looks up the project and node (the node may be missing), and starts the process with: kind = node kind (a terminal always gets a fresh shell); `resume` = the transcript file exists (a card never used starts with `--session-id <same id>`); permission `auto` when the node has `autoMode`; brain args when the node is a view's brain; the given `cols`/`rows`; no name, no first prompt. Returns `{ replay: '', live: true }`. It does not check whether the node is archived or exists.
- **SESS-137 · Reopened runs** — An action run that was taken over (no longer `runOf`) and later reopened resumes with `acceptEdits` and without the run's system prompt, since the node has no `autoMode` and run args are only passed at creation.

### Typing into a session (`session:write`)

- **SESS-138 · Write** — `session:write(sessionId, data)` is fire-and-forget; non-string data is ignored. It (1) claims an action run when the input is a takeover (SESS-139), (2) tells 04 the user typed into it (`brainInput`), (3) forwards the data to the process (a no-op when not live; 02 records keypresses for status).
- **SESS-139 · Takeover test** — Input is a takeover when it is exactly `\x1b` (Esc) or `\x03` (Ctrl+C). Otherwise, if the session's status is `waiting`, it is not (keys answer its question). Otherwise, remove in order: string sequences `/\x1b[P\]^_][\s\S]*?(?:\x07|\x1b\\)/g`, CSI sequences `/\x1b\[[0-?]*[ -/]*[@-~]/g`, two-character escapes `/\x1b[@-_]/g`; it is a takeover when what remains matches `/[^\x00-\x1f\x7f]|\r/` (a printable character or Return). Input wholly removed by those patterns (focus `\x1b[I`/`\x1b[O`, arrow keys, SGR mouse reports like `\x1b[<0;10;5M`, OSC colour replies) is not a takeover; a bracketed paste of text is. (An X10 mouse report `\x1b[M` + 3 bytes leaves printable bytes and counts as a takeover; keep it.)
- **SESS-140 · Claiming a run** — Claiming removes `runOf` from the session's node (in whatever project's board holds it), saves, and sends `ev:board-changed(projectId)`; it does nothing for a node that isn't a run. Claiming happens before the write so an interrupt doesn't end and remove the run.

### Sending a prompt (`session:send`, `sendToSession`)

- **SESS-141 · Blank prompt** — A non-string or whitespace-only text resolves immediately and does nothing (checked before anything else).
- **SESS-142 · Must be on the board** — A missing node, an archived node or a terminal fails with `That session is no longer on the board`.
- **SESS-143 · Waiting sessions** — A live session whose status is `waiting` fails with `It’s waiting for your answer. Open it to reply.` (Return would answer its question).
- **SESS-144 · Live send** — A live session (working or done) is claimed (SESS-140), then the trimmed text is typed as a bracketed paste `\x1b[200~<text>\x1b[201~` and `\r` is written 200 ms later. There is no app-side queue: while Claude works, Claude Code queues the typed prompt itself. These writes don't go through the takeover test or `brainInput`.
- **SESS-145 · Resume to send** — A session that isn't live waits for the env, looks up the project, and starts as a Claude session with `resume` = transcript exists, first prompt = trimmed text, permission `auto` when `autoMode`, brain args for a view's brain, default size; then it is claimed.
- **SESS-146 · Shared with capture** — `createSession`, `sendToSession` and `sessionTitle` are the same functions floating capture uses (07).

### Resizing, stopping, and the stop rules

- **SESS-147 · Resize** — `session:resize(sessionId, cols, rows)` is fire-and-forget and passes to 02 (ignored unless live and both finite; floored, min 20 × 5).
- **SESS-148 · Kill** — `session:kill(sessionId)` asks the host to stop the process and resolves when it exits or after 4000 ms, whichever is first; immediately when not live. The card stays (not archived, not removed), its status isn't forgotten, and opening it resumes the session.
- **SESS-149 · Stoppable** — The sessions a stop applies to are the cards that are live, not terminals, and not action runs (`runOf`). Views' brains are stoppable.
- **SESS-150 · Stop without asking** — No confirmation is asked when every stoppable card's status is `done`.
- **SESS-151 · Stop confirmation title** — With more than one card: `` `Stop ${n} sessions?` `` (n = all cards being stopped, busy or not); with one: `Stop a running session?`. Confirm label `Stop`, danger styling.
- **SESS-152 · Stop confirmation message** — When exactly one card is mid-turn (status not `done`), even among several: `` `“${title}” is still ${status === 'waiting' ? 'waiting for you' : 'working'}. Stopping it cuts off this turn.` `` (curly quotes U+201C/U+201D), where title is the card's display title (custom title > transcript title > `Terminal`/`New session`, 10). When more than one is mid-turn: `Some of these sessions are still working. Stopping them cuts off their turns.` (also when they are waiting). Then a space and `The transcripts stay, so you can resume them.` for several cards, or `The transcript stays, so you can resume it.` for one.
- **SESS-153 · Stop waits** — Stopping kills every target in parallel and completes once all have exited (or timed out, SESS-148).

### Action runs and archive_self (session lifecycle part)

- **SESS-154 · On done** — On every status change the app sends `ev:status(sessionId, status, live)`, lets notifications see it (02), and, when the status is `done`, first finishes an action run (SESS-155) and then archives a session that asked to be (SESS-157); then tells 04 (`brainStatus`).
- **SESS-155 · Finishing a run** — For a node with `runOf` on some board: the action is looked up (project's own then global); the transcript's `lastAssistantText` is read; the node is deleted; the id is appended to `hidden` if absent; state saved; the process is stopped and its status forgotten once gone (not awaited); `ev:run-finished({ projectId, sessionId, actionName: action name ?? node title ?? 'Action', message: lastAssistantText })` then `ev:board-changed(projectId)` are sent. Nothing happens for a node that isn't a run.
- **SESS-156 · Stale runs** — On `board:load`, runs whose process isn't live (it ended while the app was closed) are finished the same way before the board is assembled.
- **SESS-157 · Archive when done** — A session that called archive_self (04) is remembered; at its next `done` it is forgotten from that set and, if its node still exists and isn't archived, archived with `archiveNode` (SESS-086), saved, and `ev:board-changed` is sent. The set is in memory only.

### Titles, session lists and project stats

- **SESS-158 · Session title** — `sessionTitle(projectId, sessionId)`: a view's brain is called by its view's title, or `A view` when the view is gone or untitled. Otherwise the node's custom title, else the transcript title (even when the transcript has no content, i.e. possibly `Untitled session`), else `Terminal` for a terminal node, else `New session`. Throws `Unknown project <id>` for an unknown project.
- **SESS-159 · Session list** — `sessionList(project, self, includeArchived)` (for board tools and views, 04) runs a full `loadBoard` (with all its side effects: imports, save, `openedAt`), then lists cards other than `self` and views' brains, archived ones only with `includeArchived`, each `{ id, title, kind, status, running, section, actionRun, branch, lastReply, lastActive }`: title = custom title, transcript title (when it has content), `Terminal` or `New session`; kind = node kind or `claude`; status = `archived` for archived cards, `waiting for the user` for waiting, else the status; running = live; section = the section's name or `Untitled section` when unnamed (absent when none); actionRun = `true` for runs else absent; branch = `meta.gitBranch`; lastReply = `meta.lastAssistantText`; lastActive = ISO 8601 (`YYYY-MM-DDTHH:mm:ss.sssZ`) of `meta.updatedAt` or the node's `createdAt`, fractional ms truncated.
- **SESS-160 · Project stats** — `projectStats(project)`: `total` = nodes that aren't views' brains + undiscovered sessions (on disk, eligible, not a node, not hidden); `active` = non-brain nodes not archived (terminals and runs included); `working`/`waiting` = non-brain, non-archived, live nodes whose status is working/waiting; `running` = over all nodes that are live: `background` for brains and runs, `terminals` for terminals, `sessions` for the rest.
- **SESS-161 · Has transcript** — `hasTranscript(project, id)` is whether `transcriptPath` exists.

### Reading transcripts for the UI

- **SESS-162 · Session transcript** — `session:transcript(projectId, sessionId)` returns `readTranscript(transcriptPath(...))` (SESS-053…062), not in agent mode.
- **SESS-163 · Agent transcript** — `session:agent-transcript(projectId, sessionId, agentId)` fails with `Not an agent of this session` unless both ids match `AGENT_ID`; then reads `<subagents folder>/agent-<agentId>.jsonl` in agent mode.
- **SESS-164 · Agents list** — `session:agents(projectId, sessionId)` returns 02's `readAgents(repoPath, sessionId)`.

### Handoff summaries (`session:summarize`, `summarizeSessions`)

- **SESS-165 · No conversation** — For each requested id, after waiting for the env: if its transcript metadata can't be read or the file doesn't exist, its result is `{ sessionId, title, summary: '', error: 'This session has no conversation yet' }` and `ev:summarize-progress({ requestId, sessionId, phase: 'error', error })` is sent at once; the rest go on to be summarized (a transcript with no prompts still goes on).
- **SESS-166 · Summary titles** — Each result's title is the node's custom title, else the transcript title, else `New session`.
- **SESS-167 · Focus** — A string `focus` (even empty) is passed on; anything else means none. When `focus.trim()` is non-empty the focused prompt is used (with `focus` trimmed and whitespace runs collapsed, JSON-quoted); otherwise the full summary prompt.
- **SESS-168 · Queued first** — Before any work, `phase: 'queued'` is sent for every session to summarize, in order.
- **SESS-169 · Three at a time** — `min(3, n)` workers each take the next session in order: send `phase: 'running'`, run the fork, then send `phase: 'done'` or `phase: 'error'` with the message, and take the next. Each finished worker picks up the next index right after reporting.
- **SESS-170 · Fork command** — Each fork runs `claude -p --resume <id> --fork-session --no-session-persistence --output-format json <prompt>` in the project folder with `spawnEnv()`, stdin ignored, no console window on Windows. No hook settings or MCP config are added (the user's own config applies). The original session is never written to.
- **SESS-171 · Fork timeout** — After 5 minutes (300 000 ms) the fork is SIGKILLed and fails with `Timed out after 5 minutes`.
- **SESS-172 · Fork result** — On exit, trimmed stdout parsed as JSON with falsy `is_error` and a `result` string that is non-empty after trim gives that trimmed string as the summary. Otherwise the error is `result` when it is a non-empty string, else the last 3 lines of trimmed stderr joined with spaces, else `` `claude exited with code ${code}` ``. A spawn error fails with its own message.
- **SESS-173 · Results order** — The call resolves with one result per requested id, in the requested order (`{ sessionId, title, summary, error? }`).
- **SESS-174 · No claude** — If the claude binary isn't found, the whole call fails with the `requireClaude` message (01), even after the per-session no-conversation errors were sent.

### ⌘K session search (`session:search`, `searchSessions`)

- **SESS-175 · Search targets** — `session:search(projectId, query)` searches the board's nodes that aren't terminals, action runs or views' brains, archived ones included, in node order. Each target's title is the node's custom title, else the transcript title (even without content), else `''`. The query is `String(query ?? '')`.
- **SESS-176 · Query terms** — Terms = the query lowercased, split on `/\s+/`, empties dropped, duplicates removed (first occurrence kept), the first 8 kept, then sorted by length descending (stable).
- **SESS-177 · Term pattern** — The match pattern is the terms, regex-escaped (`/[.*+?^${}()|[\]\\]/g` → `\$&`), joined with `|`, case-insensitive and Unicode (`giu`); at a given position the earliest-listed (longest) term wins.
- **SESS-178 · Match ranges** — `matchRanges(text, terms)` = sorted `[start, end)` ranges (UTF-16 offsets) of every match of the pattern, merging a match into the previous range when it starts at or before that range's end (touching ranges merge). No terms → `[]`.
- **SESS-179 · Empty query** — No terms → `[]`.
- **SESS-180 · Phrase** — With more than one term, the phrase is the query lowercased, trimmed, whitespace runs collapsed to one space (all words, duplicates and words past 8 included). One term → no phrase.
- **SESS-181 · Repo roots** — Paths are shown relative to the repo: roots are `repoPath` and `realRepoPath(repoPath)`, deduplicated.
- **SESS-182 · Files searched** — Per target: its own transcript (not agent), then each subagent transcript (agent mode) in directory order.
- **SESS-215 · A card's own folder** — On a server project from GitHub, a card that runs in a worktree of its own (SRV-122, SRV-125) has its transcript and its subagents' under that worktree's transcript folder, so search reads them there, and paths in its snippets are relative to the worktree (SESS-181's roots, for that folder). Other cards are read from the project's folder.
- **SESS-183 · Yield** — Before reading a target any of whose files is not indexed yet (even a missing one) or has a different size or mtime than indexed (a cached file that can't be stat'ed doesn't count), the search yields to other work once. (Responsiveness only.)
- **SESS-184 · Searchable documents** — Lines with truthy `isSidechain` are skipped in a session's own transcript. A `user` line, only outside agent mode, gives a `prompt` document of `promptFull(text)` when `userText` (human origin) yields a prompt (slash commands as `/name args`). An `assistant` line gives a `reply` document of its text (handback included) when non-empty, then one `tool` document per tool use (SESS-185). Agent documents are flagged `agent`. Each document's time is `timeOf(line) ?? 0`.
- **SESS-185 · Tool documents** — From a tool use's `input` (object, else `{}`), with fields counted only when they are strings non-empty after trim: paths = `file_path`, `notebook_path`, `path` (in that order), each made relative by removing the first root followed by the path separator it starts with; rest = `command`, `pattern`, `url`, `query`, `description`, `skill`, `prompt`, `content`, `new_string`, `old_string` (in that order). No paths and no rest → no document. Text = the paths joined with `\n` (if any), then the rest, all joined with `\n`; `tool` = `toolName(name)`; `file` = basename of the first path; `pathEnd` = length of the joined paths (0 without paths).
- **SESS-186 · Tool output left out** — Raw lines containing the exact substring `"type":"tool_result"` are skipped before parsing; tool output is never searched.
- **SESS-187 · Search index cache** — Each file's documents are cached in memory by path with its size, mtime and offset. Same size and mtime → cached. Larger → only the new bytes are read and their documents appended. Otherwise rebuilt from byte 0. The whole file is indexed (no size cap, no head/tail). A file that can't be stat'ed drops its cache and has no documents; one that can't be opened keeps its cached documents.
- **SESS-188 · Title coverage** — Each term found (substring) in the lowercased title counts as covered. `titleScore` = 4 when the phrase is in the title or every term is, else (terms in title) / (terms).
- **SESS-189 · Document score** — A document matches when its lowercased text contains at least one term (each found term marks the term covered). `fit` = 3 when the phrase is in it, else 2 when all terms are, else found/terms; `score = fit × kindWeight × (0.8 if agent else 1)` with kind weights prompt 1.3, tool 1.1, reply 1.
- **SESS-190 · Every word required** — A session is a hit only when every term is covered by its title or some document.
- **SESS-191 · Hit statistics** — A hit's documents are sorted by score descending, then time descending (stable). `matches` = the number of documents containing every term, or, when none does, the number of matching documents. `best` = the sum of the top 5 scores.
- **SESS-192 · Snippets** — Up to 3 snippets from the documents in order, skipping one whose text and tool equal an earlier snippet's.
- **SESS-193 · Snippet window** — The anchor is the first match of the phrase pattern (when there is a phrase) else of the term pattern (position 0, length 0 if none). `inPath` = anchor before `pathEnd`. `base` = `pathEnd + 1` for a tool document matched outside its path with `pathEnd > 0`, else 0. `limit` = `pathEnd` when inPath, else the text length. `start = max(base, anchor − 48)`; if `start > base`, it moves just past the first `/\s/` character in `[start, anchor)` if any. `end = min(limit, start + 160)`; if `end < limit` and the last space character (U+0020 only) in `[matchEnd, end)` is at offset > 0 from matchEnd, `end` = that space's position. Text = the slice with whitespace runs collapsed and trimmed, prefixed with `…` when `start > base`, suffixed with `…` when `end < limit`. Snippet = `{ kind, tool, file (only for a tool matched outside its path), agent, text, ranges: matchRanges(text, terms), timestamp }`.
- **SESS-194 · Session ranking** — Session score = `titleScore + best + log2(1 + matches) / 2`; `latest` = the newest time among its matching documents (0 if none). Hits sort by score descending, then latest descending (stable, ties in target order); at most 50 are returned as `{ sessionId, matches, snippets }`.
- **SESS-195 · Title-only hit** — A session covered only by its title is a hit with `matches: 0` and `snippets: []`, even when it has no transcript.

### Shared geometry

- **SESS-196 · Size constants** — Card 288 × 216 (`CARD_SIZE`), `GAP` 32, expanded card 720 × 480 (a new card's 720 × 960, `NEW_CARD_H`), archive header 52, archive pad 28, stack search at y 306 (`52 + 216 + 38`) and 34 tall, stack 344 wide (`28*2 + 288`) and 362 tall (`306 + 34 + 22`), note 216 × 216, pad header 52, pad width 344, pad gap 24, pad bottom 28, view 480 × 360 (min 240 × 160, max 2400 × 1800).
- **SESS-197 · Card size** — A card is expanded only when `expanded` is truthy and it isn't archived; an expanded card is `w ?? 720` × `h ?? 480`, any other (or none) is 288 × 216.
- **SESS-198 · Note and view sizes** — A note is `w ?? 216` × `h ?? 216`; a view `w ?? 480` × `h ?? 360`. `clampViewSize` rounds each finite dimension into 240…2400 (w) and 160…1800 (h), leaving non-finite ones absent.
- **SESS-199 · Dock rects** — The note pad is `{ x: archive.x, y: archive.y + 362 + 24, w: 344, h: 52 + 216 + 28 }`; the dock (folded stack plus pad) is `{ x: archive.x, y: archive.y, w: 344, h: 682 }`.
- **SESS-200 · Intersects** — `intersects(a, b, margin = 0)` = `a.x < b.x + b.w + margin && a.x + a.w + margin > b.x && a.y < b.y + b.h + margin && a.y + a.h + margin > b.y` (touching edges don't intersect at margin 0).
- **SESS-201 · Free spot** — `findFreeSpot(preferred, obstacles, avoid?, size = 288 × 216)`: a spot is blocked when the rect at it intersects `avoid` with margin 32 or any obstacle with margin 16. The preferred spot is returned if free. Otherwise for rings 1…39: candidates are grid offsets `(dx, dy)` with `max(|dx|, |dy|) = ring`, enumerated dx from −ring to ring (outer) and dy from −ring to ring (inner), placed at `preferred + (dx × 320, dy × 248)`, stably sorted by `hypot(Δx, Δy) × (1.4 if Δx < 0 else 1) × (1.2 if Δy < 0 else 1)`; the first free one is returned. If none, `{ x: preferred.x, y: preferred.y + 40 × 248 }`.
- **SESS-202 · Spot beside** — `findSpotBeside(anchor, size, obstacles, avoid?, gap = 32)`: `reach = 2 × max(w, h)`; right = `anchor.x + anchor.w + gap`, left = `anchor.x − gap − w`, below = `anchor.y + anchor.h + gap`, above = `anchor.y − gap − h`. Candidates per axis: for `d = 0, 32, …, ≤ reach`, push `after + d` then `before − d`; then from `start − floor((start − before)/32) × 32` stepping 32 while `< after`, push each. xs = axis(anchor.x, left, right), ys = axis(anchor.y, above, below). For each x (outer) and y (inner): `gx = max(anchor.x − (x + w), x − (anchor.x + anchor.w))`, `gy` likewise; skip if `max(gx, gy) < gap − 1`; sideways = `gx >= gy`; weight = sideways ? (x > anchor.x ? 1 : 1.3) : (y > anchor.y ? 1.15 : 1.45); offset = sideways ? `|y − anchor.y|` : `|x − anchor.x|`; score = `hypot(max(gx,0), max(gy,0)) × weight + offset/2`; skip if `score >= best`; skip if it intersects `avoid` (margin 32) or any obstacle (margin 16); else it becomes the best. No best → `findFreeSpot({ x: right, y: anchor.y }, obstacles, avoid, size)`. Steps and margins are fixed at 32/16 whatever `gap` is.

### Misc

- **SESS-203 · Events without a window** — Events (`ev:board-changed`, `ev:summarize-progress`, `ev:run-finished`, `ev:status`) sent while no window exists are dropped, not queued.
- **SESS-204 · Unknown project ids** — Handlers that don't look up the project first (`board:save-section`, `remove-section`, `save-note`, `remove-notes`, `save-ink`, `save-archive`, `save-viewport`, `remove-node`, and `session:send`) create a default board record (archive `{0,0,1928,320}`, empty `hidden`, `sections`, `notes`, `views`) for an unknown project id in `state.json` (01's `getBoard`).
- **SESS-205 · Known limitations kept** — Status is only tracked for processes the app started; a session run elsewhere reads as done. `/clear` in a card makes a new session id, which appears as a new archived card on the next load (SESS-073). A first session in a repo Claude Code hasn't opened shows its folder-trust prompt; nothing in the app answers it.

## Data & formats

### Transcript JSONL records the parser relies on

One JSON object per line, appended by Claude Code. Fields read (all optional; anything else is ignored):

```jsonc
{
  "type": "user" | "assistant" | "custom-title" | "ai-title" | "summary" | <other: ignored>,
  "timestamp": "2026-11-01T10:00:00.000Z",   // Date.parse
  "gitBranch": "main",
  "isSidechain": false,
  "isMeta": true, "isCompactSummary": true, "isVisibleInTranscriptOnly": true,   // user: not a prompt
  "origin": { "kind": "human" | "task-notification" | ... },
  "customTitle": "…",          // type custom-title
  "aiTitle": "…",              // type ai-title
  "summary": "…",              // type summary (older transcripts' title)
  "message": {
    "id": "msg_…",             // assistant: one API message may span several lines
    "content": "text" | [
      { "type": "text", "text": "…" },
      { "type": "tool_use", "id": "…", "name": "Edit" | "mcp__server__tool" | "SubagentHandback", "input": { … } },
      { "type": "tool_result", "tool_use_id": "…", "content": … }
    ],
    "stop_reason": "end_turn"  // read by 02 (agents.ts)
  },
  "toolUseResult": { "backgroundTaskId": "…" }  // read by 02
}
```

Text conventions inside user content: `<command-name>/x</command-name>`, `<command-message>…</command-message>`, `<command-args>…</command-args>`; `<pasted_content …>…</pasted_content>`; `<system-reminder>…</system-reminder>`; `<task-notification>…<summary>…</summary>…<task-id>…</task-id>…</task-notification>`. `SubagentHandback` input: `{ "message": "<agent's report>" }`.

Layout on disk:

```
~/.claude/projects/<encoded>/<session-uuid>.jsonl
~/.claude/projects/<encoded>/<session-uuid>/subagents/agent-<agentId>.jsonl
~/.claude/projects/<encoded>/<session-uuid>/subagents/agent-<agentId>.meta.json   (02)
```

### Constants

| Name | Value |
| --- | --- |
| Large transcript (head/tail mode) | > 4 194 304 bytes |
| Head read | 262 144 bytes |
| Tail read | 1 048 576 bytes |
| Preview read | last 16 777 216 bytes |
| Preview max messages | 500 |
| Title truncation | 80 |
| firstPrompt / lastPrompt truncation | 500 |
| lastAssistantText truncation | 400 |
| Tool call detail truncation | 160 |
| Watch check delay | 500 ms (not restarted) |
| Watch min interval between emits | 1500 ms |
| Watch poll for missing folder | 3000 ms |
| Expanded size clamp | 200…4000, rounded |
| Note text max | 20 000 UTF-16 units |
| Note size clamp | 120…1200, rounded |
| Send: Return after paste | 200 ms |
| Kill wait | 4000 ms (host: SIGTERM, SIGKILL after 3000 ms, 02) |
| Default pty size | 120 × 36 (min 20 × 5) |
| Summary concurrency / timeout | 3 / 300 000 ms |
| Naming timeout / prompt clip / name max | 30 000 ms / 280 / 48 |
| Search: max results / snippets / snippet chars / lead / max terms | 50 / 3 / 160 / 48 / 8 |
| Search weights | prompt 1.3, tool 1.1, reply 1, agent × 0.8 |

### Output shapes (IPC JSON, `src/shared/types.ts`)

```ts
SessionMeta { sessionId; title; firstPrompt?; lastPrompt?; lastAssistantText?; gitBranch?; messageCount; createdAt; updatedAt; transcriptPath }
TranscriptMessage { role: 'user'|'assistant'; text; timestamp; tools?: string[]; calls?: { name; detail? }[]; notice?: boolean }
SessionCard = BoardNode & { meta?; status: 'working'|'waiting'|'done'; live: boolean; agents?: AgentInfo[]; agentCount?: number; usage?: SessionUsage }
BoardSnapshot { project; cards; archive: Rect; sections; notes; views; ink; viewport? }
OpenSessionResult { replay: string; live: boolean }
SummaryResult { sessionId; title; summary; error? }
SummarizeProgress { requestId; sessionId; phase: 'queued'|'running'|'done'|'error'; error? }
SearchSnippet { kind: 'prompt'|'reply'|'tool'; tool?; file?; agent?; text; ranges: [number, number][]; timestamp }
SessionSearchHit { sessionId; matches; snippets }
ActionRunResult { projectId; sessionId; actionName; message? }
ProjectStats { total; active; working; waiting; running: { sessions; background; terminals } }
```

Times are ms since epoch. `createdAt`/`updatedAt` may be fractional when they fell back to a file mtime, and an imported node's `createdAt` copies that value into `state.json`, so the Rust types must be `f64` (or accept and re-emit fractional numbers).

### Board state in `state.json` (fields this domain reads and writes)

```ts
BoardState {
  nodes: Record<sessionId, BoardNode>   // insertion order matters: card order; keep with IndexMap / serde_json preserve_order
  archive: Rect
  sections?: Record<id, BoardSection>
  notes?: Record<id, BoardNote>
  views?: Record<id, BoardView>          // 04
  ink?: InkStroke[]                      // 04
  viewport?: { x; y; zoom }
  hidden?: string[]                      // removed sessions, never re-imported
}
BoardNode { sessionId; kind?: 'claude'|'terminal'; x; y; archived; archivedAt?; title?; parents?: string[];
            createdByApp?; sectionId?; expanded?; w?; h?; runOf?; viewOf?; fromView?; autoMode?; createdAt }
```

A node created by `session:create` is written with its keys in this order (absent ones omitted): `sessionId, kind, x, y, archived, title, parents, createdByApp, runOf, expanded, autoMode, fromView, createdAt`; later fields (`archivedAt`, `sectionId`, `w`, `h`) are appended when first set. An imported node: `sessionId, x, y, archived, createdAt`.

### NodePatch semantics (`board:save-nodes`)

| Field | Effect |
| --- | --- |
| `sessionId` | Node to patch; unknown → patch skipped |
| `archived: true` on a live-board, empty session | Node dropped (removed, process stopped, notes unstuck), rest ignored, `ev:board-changed` |
| `x`, `y` | Set when finite |
| `title` (string) | Trimmed; `''` removes the custom title |
| `archived` (boolean) | Ignored for terminals and brains; archiving stops the process and stamps `archivedAt` |
| `sectionId: null` / existing id | Leave section / join it; unknown id ignored |
| `expanded` | `true` sets, `false` removes |
| `w`, `h` | `round(clamp(200, 4000))` when finite |
| (after) node archived | `sectionId` and `expanded` removed |

### Claude session argv (verbatim templates)

New session:

```
<claude> --session-id <uuid> [--name <title>] --permission-mode <auto|acceptEdits>
         [--mcp-config <BOARD_MCP_CONFIG> --allowedTools <rule1> … <rule16>]
         [<extra args>]
         --settings <hookSettings()>
         [-- <first prompt>]
```

Resumed session: same with `--resume <uuid>` instead of `--session-id <uuid>`, and never `--name`.

`BOARD_MCP_CONFIG` (one argument, exactly this string; `${…}` are expanded by Claude Code from the env, not by Nodus):

```
{"mcpServers":{"nodus":{"type":"http","url":"http://127.0.0.1:${NODUS_PORT}/mcp","headers":{"Authorization":"Bearer ${NODUS_TOKEN}","X-Nodus-Session":"${NODUS_SESSION}"}}}}
```

`--allowedTools` is followed by 16 separate arguments, in this order (0.9.4 has 12, without the four drawing tools added since, spec 04 TOOL-101):

```
mcp__nodus__create_note mcp__nodus__update_note mcp__nodus__list_notes mcp__nodus__list_projects
mcp__nodus__create_project mcp__nodus__archive_self mcp__nodus__list_board mcp__nodus__draw_on_board
mcp__nodus__update_drawing mcp__nodus__erase_from_board mcp__nodus__create_view mcp__nodus__update_view
mcp__nodus__get_view mcp__nodus__list_views mcp__nodus__list_sessions mcp__nodus__archive_sessions
```

`hookSettings()` is the `--settings` JSON built in 02 (hooks for every hook event plus a status line; curl or PowerShell variants).

Extra args:
- Action run, first start only: `--append-system-prompt <runSystemPrompt(action)>`.
- View's brain, every start: `--append-system-prompt <brainPrompt(view)>`.

Env added for Claude sessions (on top of `spawnEnv()`): `NODUS_PORT=<host hook port>`, `NODUS_TOKEN=<host token>`, `NODUS_SESSION=<session id>`, `NODUS_HOOK_CONFIG=<path>` (when the host provides a curl config), and on Windows `CLAUDE_CODE_GIT_BASH_PATH=<git bash>` when found and not set by the user.

Terminal argv: macOS `<SHELL> -l`; Windows `<pwsh.exe | powershell.exe> -NoLogo`. Env `spawnEnv()` only.

On Windows the host joins argv into one command line with `windowsCommandLine` (02): an argument is left bare when non-empty without whitespace or `"`; otherwise it is wrapped in `"…"`, backslashes before a `"` are doubled and the `"` escaped as `\"`, and trailing backslashes are doubled.

#### Action run system prompt (`runSystemPrompt`, 05)

```
You are running "${action.name}", an action button the user pressed on the Nodus board for this project. Nobody is watching while you work, so do the job in the prompt without stopping to chat. If you need the user to decide something, ask with AskUserQuestion: that turns the action's button red to call them. When you are done, end with one line saying what you did. The user is shown that line, and this session then goes away. The mcp__nodus__* tools let you list and archive the board's sessions, and write sticky notes on the board.
```

#### View brain system prompt (`brainPrompt`, 04; `${VIEW_GUIDE}` verbatim in 04's spec)

```
You are the brain of a view on the user's Nodus board: a small live web page on a card, beside their Claude Code sessions. The view is "${view.title}" (id ${view.id}). The user asked for:

${view.description}

You build the page and keep it up to date with the mcp__nodus__* board tools. update_view sets its html, data, title, commands and refresh schedule. get_view shows it as it is now, with its commands' last output and any JavaScript errors it hit. Leave out view_id: both act on this view. If you're picking up a view that already has a page, read it with get_view before changing it.

${VIEW_GUIDE}

Nobody watches you while you work, so don't stop to chat. Work out where the data comes from by looking around the repository, the CLIs on the PATH and your MCP tools. If you need the user, for a missing API token or a choice only they can make, ask with AskUserQuestion, which shows on the view that you need them. End each turn with one short line on what you did.
```

### Sending a prompt into a live session

```
write  "\x1b[200~" + text.trim() + "\x1b[201~"
wait   200 ms
write  "\r"
```

### Handoff summary fork

argv (cwd = project folder, env = `spawnEnv()`, stdin ignored):

```
<claude> -p --resume <session id> --fork-session --no-session-persistence --output-format json <prompt>
```

`SUMMARY_PROMPT` (no focus):

```
Write a handoff summary of this session so that a NEW Claude Code session, with no access to this conversation, can pick up the work.

Cover, under short markdown headings:
- Goal: what the user is trying to achieve.
- Key decisions: what was decided and why, including approaches that were rejected.
- Current state: files created or changed (with paths), what works, and what was verified.
- Open issues and next steps: unfinished work, known bugs, and the obvious next actions.
- Gotchas: important commands, conventions, environment details, or pitfalls discovered.

Answer from the conversation alone: do not call tools, run commands, or change files. Leave out environment notices that are not part of the work (for example connector or MCP authorization messages). Be concise but complete, and prefer specifics (paths, names, commands) over generalities. Output only the summary, with no preamble.
```

`focusedSummaryPrompt(focus)`, where `${topic}` = `JSON.stringify(focus.trim().replace(/\s+/g, ' '))` (a double-quoted JSON string, e.g. `"auth middleware"`):

```
Write a handoff summary of what this session did about ${topic}, so that a NEW Claude Code session, with no access to this conversation, can pick up that work.

Cover only what relates to ${topic}, under short markdown headings:
- What was asked: what the user wanted done about it.
- Key decisions: what was decided and why, including approaches that were rejected.
- Current state: files created or changed (with paths), what works, and what was verified.
- Open issues and next steps: unfinished work and known bugs.
- Gotchas: commands, conventions, or pitfalls discovered along the way.

If the session only touched on it in passing, say so in a sentence or two instead. Answer from the conversation alone: do not call tools, run commands, or change files. Be concise, and prefer specifics (paths, names, commands) over generalities. Output only the summary, with no preamble.
```

Expected stdout: one JSON object, e.g. `{"type":"result","is_error":false,"result":"…"}`; only `is_error` and `result` are read.

### Section naming

argv (cwd = OS temp folder, env = `spawnEnv()`, stdin ignored); 14 arguments after the binary, the empty string after `--tools` included:

```
<claude> -p --model haiku --settings {"alwaysThinkingEnabled":false} --tools "" --strict-mcp-config --no-session-persistence --system-prompt <SYSTEM_PROMPT> --output-format json <prompt>
```

`SYSTEM_PROMPT`:

```
You name groups of Claude Code sessions on a board. Reply with the group's name only: 1 to 4 words in Title Case that say what the sessions have in common. No quotes, no trailing punctuation, no preamble.
```

User prompt (example with two sources, the second without a distinct opening prompt):

```
Name this group of sessions:

1. Fix login
   Opening prompt: The login redirect loops. Check the auth middleware please
2. Write the README
```

No Claude call in the app names sessions: session titles come from the transcript (`custom-title` and `ai-title` records written by Claude Code), with the user's own title from the board (`BoardNode.title`) overriding. unclear: whether Claude Code records `--name` as a `custom-title` line (not determinable from this repo).

## API surface

Commands (IPC channel → Rust core command). All `invoke` style unless marked fire-and-forget.

| Channel | Args | Returns |
| --- | --- | --- |
| `board:load` | `projectId` | `BoardSnapshot` |
| `board:save-nodes` | `projectId, patches: NodePatch[]` | `void` |
| `board:save-archive` | `projectId, rect: Rect` | `void` |
| `board:save-viewport` | `projectId, viewport: Viewport` | `void` |
| `board:remove-node` | `projectId, sessionId` | `void` (after the process stopped or 4 s) |
| `board:save-section` | `projectId, section: BoardSection` | `void` |
| `board:remove-section` | `projectId, sectionId` | `void` |
| `board:name-section` | `projectId, sessionIds: string[]` | `string` |
| `board:save-note` | `projectId, note: BoardNote` | `void` |
| `board:remove-notes` | `projectId, noteIds: string[]` | `void` |
| `board:save-ink` | `projectId, strokes: InkStroke[]` | `void` |
| `session:create` | `projectId, opts: CreateSessionOptions` | `SessionCard` |
| `session:open` | `projectId, sessionId, cols, rows` | `OpenSessionResult` |
| `session:write` | `sessionId, data` | fire-and-forget |
| `session:send` | `projectId, sessionId, text` | `void` |
| `session:resize` | `sessionId, cols, rows` | fire-and-forget |
| `session:kill` | `sessionId` | `void` |
| `session:transcript` | `projectId, sessionId` | `TranscriptMessage[]` |
| `session:agents` | `projectId, sessionId` | `AgentInfo[]` (02) |
| `session:agent-transcript` | `projectId, sessionId, agentId` | `TranscriptMessage[]` |
| `session:summarize` | `projectId, sessionIds, requestId, focus?` | `SummaryResult[]` |
| `session:search` | `projectId, query` | `SessionSearchHit[]` |

`CreateSessionOptions { x; y; kind?: 'claude'|'terminal'; name?; initialPrompt?; parents?: string[]; actionId?; expanded?; autoMode?; fromView? }`.

Events emitted by this domain:

| Event | Payload | When |
| --- | --- | --- |
| `ev:board-changed` | `projectId` | Transcript folder change (throttled), empty session dropped by save-nodes, run claimed or finished, archive_self done |
| `ev:summarize-progress` | `SummarizeProgress` | queued / running / done / error per session |
| `ev:run-finished` | `ActionRunResult` | An action run's turn ended (or it ended while the app was closed) |
| `ev:status` | `sessionId, status, live` | Forwarded from the status machine (02), before run/archive handling |

Internal API used by other domains (`nodus-core` exposes them to 04, 05, 07):
- `create_session(projectId, CreateSessionOptions) -> SessionCard`, `send_to_session(projectId, sessionId, text)`, `session_title(projectId, sessionId) -> String` (07 capture, 04 notes).
- `load_board`, `session_list(project, self, include_archived)`, `project_stats(project)` (01/08 `projects:stats`), `archive_node`, `drop_node`, `is_empty`, `unstick_notes`, `to_card`, `has_transcript` (04 board tools, 04 views).
- `read_session(id, file) -> Option<DiscoveredSession>`, `scan_repo`, `transcript_path`, `transcript_dir`, `subagents_dir`, `list_agent_files`, `real_repo_path`, `encode_repo_path`, `read_lines`, `user_text`, `assistant_parts`, `describe_tool_use`, `tool_name`, `truncate`, `time_of`, `message_text`, `is_task_notification`, `prompt_full`, `HANDBACK_TOOL`, `AGENT_ID` (02 agents.ts, 07 capture).
- `watch_project(projectId, repoPath, on_change)`, `unwatch_project`, `unwatch_all` (01 lifecycle).
- `stoppable(cards)`, `stop_confirmation(cards, title_of) -> Option<ConfirmRequest>` (09 board, 12 terminal modal).
- Geometry functions (04 notes/views placement, 07 capture, 09/10 board UI).

## Platform differences

| Concern | macOS | Windows |
| --- | --- | --- |
| claude binary | `claude` found on the login-shell PATH plus fallbacks (01) | `claude.exe`, or the `.exe` behind an npm `claude.cmd` shim (01) |
| Missing claude message | `Claude Code CLI not found. Install it (npm i -g @anthropic-ai/claude-code) and restart Nodus.` | `Claude Code CLI not found. Install it (irm https://claude.ai/install.ps1 \| iex, in PowerShell) and restart Nodus.` |
| Terminal shell | `$SHELL -l` (default `/bin/zsh`) | `pwsh.exe -NoLogo`, else Windows PowerShell 5 `-NoLogo` |
| Process argv | argv array to the pty | one command line (`windowsCommandLine`, 02) |
| Extra env | — | `CLAUDE_CODE_GIT_BASH_PATH` when Git Bash is found and unset |
| Hooks | curl (02) | Git Bash curl, or PowerShell when no Git Bash (02) |
| Stopping a process | SIGTERM, SIGKILL after 3 s (02) | pseudo-terminal closed (02) |
| `-p` timeouts (summary, naming) | SIGKILL | TerminateProcess |
| `claude -p` children | — | started without a console window (`windowsHide`) |
| Naming cwd | `$TMPDIR` | `%TEMP%` |
| Home | `$HOME` | `%USERPROFILE%` |
| Path encoding | `/work/repo` → `-work-repo` | `C:\work\repo` → `C--work-repo` (drive letter case as stored) |
| Search relative paths | root + `/` | root + `\` |
| Basename (tool detail, search `file`) | splits on `/` only | splits on `\` and `/` |
| Recursive watch | FSEvents | ReadDirectoryChangesW |

## Edge cases & errors

Errors thrown to the caller (message verbatim):
- `Unknown project ${id}` — any handler that looks the project up (`board:load`, `board:save-nodes`, `board:name-section`, `session:create`, stopped `session:open`, not-live `session:send`, `session:transcript`, `session:agents`, `session:agent-transcript`, `session:summarize`, `session:search`, `sessionTitle`).
- `That action no longer exists` — `session:create` with an `actionId` that isn't found, or any terminal with an `actionId`.
- `That session is no longer on the board` — `session:send` to a missing, archived or terminal node.
- `It’s waiting for your answer. Open it to reply.` — `session:send` to a live waiting session (U+2019 apostrophe).
- `These sessions have nothing to name the section after yet` — `board:name-section` with no usable sources.
- `Naming timed out` — naming over 30 s.
- `Not an agent of this session` — `session:agent-transcript` with an id outside `/^[\w-]+$/`.
- `` `claude exited with code ${code}` `` — naming/summary fallback reason (`code` may be `null`).
- `Claude Code CLI not found. Install it (…) and restart Nodus.` — any launch, summary or naming without a claude binary (01).
- Host start errors surfacing from `session:create`/`open`/`send` (02): the host's spawn error string, `The session host stopped`, `` `The session host didn't start. See ${log}` ``, `The session host speaks another protocol`, `The session host didn’t answer`, `Another program answered as the session host, so Nodus didn’t trust it with your sessions.`
- A transcript that exists but can't be opened makes `session:transcript` / `session:agent-transcript` fail with the OS error message.

Per-item results (not thrown):
- `This session has no conversation yet` — summary result and progress error for a session without a transcript.
- `Timed out after 5 minutes` — a summary fork over 5 minutes.

Fallback strings: `Untitled session` (no title, no prompt), `New session` (card/summary title), `Terminal`, `A view`, `Untitled section` (session list), `A background task finished` (task notification without summary), `[Pasted text]` (pasted block in titles/previews), `Action` (run result name).

Edge cases to keep:
- A `board:save-nodes` archive patch on a view's brain or run with no conversation drops that node (the empty check runs before the terminal/brain guard).
- `board:save-nodes` archive patch on a terminal changes nothing about `archived`; position and size still apply.
- `board:remove-node` for an id with no node still hides it.
- `session:send` with blank text never errors, even for unknown sessions.
- `session:open` for a session not on the board still starts `claude` for it.
- `session:create` for a terminal keeps `opts.name` as its title.
- A transcript line split across a read boundary is parsed only once complete; a malformed line is skipped forever (the offset moves past it).
- In large-file mode a file with no `\n` in its last 1 MiB parses nothing from the tail.
- A watcher error stops change events for that project for the rest of the run.
- `board:save-note` drops `fromProject`/`fromTitle`.
- `sessionTitle` and search titles can be `Untitled session` (title read even without content); card meta and `name-section` require content.
- Duplicate ids in `session:summarize` are summarized twice and both slots get the later result.
- Snippet ranges are computed for the terms only, on the final snippet text (ellipses included in offsets).

## Tests to port

### `src/main/search.test.ts` (→ `nodus-transcripts`)

Fixture: a temp home (home dir overridden), repo `<home>/repo`. Clock starts at `2026-11-01T10:00:00Z` and every line is 1 s later. Helpers: `prompt(text)` (user, string content), `reply(text)` (assistant text block, `message.id = m<clock>`), `tool(name, input)` (assistant tool_use), `result(content)` (user tool_result). Sessions (rewritten before each test, index cleared):
- auth `00000000-0000-4000-8000-000000000001`, title `Fix login`: prompt `The login redirect loops. Check the auth middleware please`; reply `The session cookie is read before it is set.`; Edit `{ file_path: <repo>/src/middleware/auth.ts, old_string: 'const token = req.cookies.session', new_string: 'const token = req.signedCookies.session' }`; tool result `export function authMiddleware() { /* the whole file, which mentions README */ }`; reply `Fixed: the middleware now reads the signed cookie.`
- docs `…0002`, title `Write the README`: prompt `Write a README for the project`; Write `{ file_path: <repo>/README.md, content: '# Project\n\nAuth is handled elsewhere.' }`; reply `Added the README.`
- agent `…0003`, title `Tidy up`: prompt `Tidy up the repo`; reply `I asked an agent to look around.`; subagent file `<dir>/…0003/subagents/agent-a1.jsonl` with sidechain prompt `Look for dead code (the session’s prompt to its agent)` and sidechain Bash `{ command: 'rg -l signedCookies src' }`.

searchSessions:
1. `finds the session whose transcript mentions every word, path matches included` — `auth middleware` → only auth; `matches` 2; snippet 0 is a `prompt` containing `auth middleware`; snippet 1 is `{ kind: 'tool', tool: 'Edit', text: 'src/middleware/auth.ts' }`.
2. `names the file when the match is in what a tool wrote` — `signedCookies` → first hit auth; its non-agent tool snippet is `{ tool: 'Edit', file: 'auth.ts' }` with text containing `req.signedCookies.session` and not `src/middleware`.
3. `searches subagents’ work, but not the prompt the session gave them twice` — `signedCookies` → the agent session's snippets are exactly one `{ kind: 'tool', tool: 'Bash', agent: true }`; `dead code` → `[]`.
4. `leaves tool output out` — `authMiddleware` → `[]`; `readme` → only docs.
5. `needs every word somewhere in the session or its title` — `auth readme` → only docs; `login cookie` → only auth.
6. `ranks the whole phrase above scattered words` — docs rewritten to prompt `Middleware for auth, maybe later` + reply `Noted.`; `auth middleware` → `[auth, docs]`.
7. `returns title-only matches without snippets` — `tidy` → first hit agent; a target `…000f` with no file titled `Tidy notes` → `[{ sessionId: …000f, matches: 0, snippets: [] }]`.
8. `reads what a live session adds to its transcript` — `websocket` → `[]`; after appending prompt `Now add a websocket endpoint` to docs → `[docs]`.
9. `cuts long text down to a line around the match` — docs prompt `'word '×100 + 'the needle is here ' + 'more '×100`; `needle` → snippet text starts and ends with `…`, is shorter than 180, and its ranges select exactly `['needle']`.
10. `returns nothing for an empty query` — `'   '` → `[]`.

query helpers:
11. `splits, lowercases and dedupes words, longest first` — `queryTerms('  Auth  middleware AUTH ')` → `['middleware', 'auth']`.
12. `merges overlapping highlights and ignores case` — `matchRanges('Auth middleware: authmiddleware', ['middleware', 'auth'])` → `[[0,4],[5,15],[17,31]]`.
13. `treats regex characters literally` — `matchRanges('call foo.bar() now', ['foo.bar()'])` → `[[5,14]]`.

### `src/shared/geometry.test.ts` (→ `nodus-board`)

`rect(x, y, w = 288, h = 216)`.
1. intersects `tells overlapping rects from ones that only touch` — rect(0,0) vs rect(100,100) true; vs rect(288,0) false; vs rect(0,217) false.
2. intersects `counts rects within the margin as overlapping` — rect(0,0) vs rect(298,0): margin 11 true, margin 10 false.
3. cardSize `is compact unless expanded, and never expanded in the archive` — `undefined` → 288×216; `{expanded, !archived}` → 720×480; with w 900 h 600 → 900×600; archived → 288×216.
4. findFreeSpot `keeps the preferred spot when it is free` — (0,0) with obstacle rect(1000,1000) → (0,0).
5. findFreeSpot `moves to the nearest free grid cell, below before above` — (0,0) with rect(0,0) → (0, 248).
6. findFreeSpot `finds room among many cards` — 5×5 cards at (i·320, j·248), i,j ∈ −2…2 → a spot clear of all (margin 16) on ring 3.
7. findFreeSpot `keeps out of the area to avoid` — avoid rect(0,0,2000,2000), preferred (100,100) → no intersection with avoid at margin 32.
8. findFreeSpot `fits something of another size` — size 600×400, obstacle rect(300,0) → no intersection at margin 16.
9. findSpotBeside `goes right of the anchor, lined up with its top` — anchor rect(0,0,480,360), size 720×480, obstacles [anchor] → (512, 0).
10. findSpotBeside `goes below, then left, when the right is taken` — plus rect(512,0) → (0, 392); plus rect(0,392) → (−752, 0).
11. findSpotBeside `stays next to the anchor where findFreeSpot steps past what is in the way` — obstacles anchor + cards at (512, i·248), i ∈ −1…2 → spot clear, its gap to the anchor is exactly 32; `findFreeSpot((512,0), same, none, size)`'s gap is > 288.
12. findSpotBeside `keeps the gap it is given` — gap 96 → (576, 0).
13. findSpotBeside `keeps out of the area to avoid` — avoid rect(480,−2000,4000,4000) → no intersection at margin 32 and `x < 0`.
14. findSpotBeside `falls back to findFreeSpot when there is no room nearby` — wall rect(−5000,−5000,10000,10000) → spot clear of the wall.

### `src/renderer/src/components/board/stopSessions.test.ts` (→ `nodus-board` or wherever the stop rules live)

Card default: `{ x: 0, y: 0, archived: false, createdAt: 0, status: 'done', live: true }`; `titleOf = title ?? sessionId`.
1. stoppable `keeps the Claude sessions that are running` — [a, b (not live), c (terminal), d (kind claude)] → [a, d].
2. stoppable `leaves out action runs, which have no card to keep` — [a (runOf), b (viewOf)] → [b].
3. stopConfirmation `doesn’t ask when every session is between turns` — [a, b] → null.
4. stopConfirmation `asks for a session that’s working, by name` — [working `Fix login`] → title `Stop a running session?`, label `Stop`, danger; message contains `“Fix login” is still working`.
5. stopConfirmation `says when a session is waiting for you` — [waiting] → message contains `waiting for you`.
6. stopConfirmation `asks once for several, when any of them is mid-turn` — [done, working, working] → title `Stop 3 sessions?`, message contains `Some of these sessions are still working`.
7. stopConfirmation `names the one session among several that’s mid-turn` — [done, waiting `Fix login`, done] → title `Stop 3 sessions?`, message contains `“Fix login” is still waiting for you` and `The transcripts stay`.

### New Rust tests worth adding (no TS counterpart; guard parity risks)
- Path encoding of a non-BMP character (two dashes) and of a repo reached through a symlink (e.g. `/tmp/…` resolving to `/private/tmp/…`).
- `truncate` / `clip` counting UTF-16 units.
- `readLines` with a partial trailing line, a partial first line, and a malformed line.
- Incremental metadata parse after append equals a fresh parse; large-file head/tail mode.
- `readSession` title precedence (custom > ai > summary > first prompt > command > `Untitled session`) and reply counting across split message ids.
- `isTakeover` on Esc, Ctrl+C, arrow keys, focus reports, OSC replies, bracketed paste, and while waiting.
- Claude argv for: new session with name and prompt; resume; host without MCP; action run; brain.
- Archive slot placement and growth for an archive with occupied slots.

## Rust mapping

| Source | Crate / module |
| --- | --- |
| discovery.ts paths, `readLines`, `userText`, `assistantParts`, `describeToolUse`, `applyLine`, cache, `readSession`, `scanRepo`, `metaSignature`, `readTranscript` | `nodus-transcripts::{paths, lines, parse, meta, preview}` |
| discovery.ts watching | `nodus-transcripts::watch` |
| search.ts, shared/search.ts | `nodus-transcripts::search` (query helpers also used by `nodus-ui` for highlighting) |
| summarize.ts | `nodus-transcripts::summarize` |
| naming.ts | `nodus-transcripts::naming` |
| board.ts | `nodus-board::{merge, archive, stats, list}` |
| shared/geometry.ts | `nodus-board::geometry` (pure; used by UI) |
| shared/sessions.ts | `nodus-domain` or `nodus-board` (`is_empty_session`) |
| stopSessions.ts (`stoppable`, `stopConfirmation`) | `nodus-board::stop` (pure); the kill-all lives in `nodus-core` |
| ipc.ts session lifecycle (`createSession`, open, write/takeover, send, kill, finishRun, claimRun, archive-when-done, `sessionTitle`) | `nodus-sessions::{lifecycle, launch, takeover}`; board handlers in `nodus-board::handlers`; wired as commands/events in `nodus-core` |

Suggested crates:
- `serde_json` (`Value` per line, `preserve_order` feature for state), `memchr` for newline scanning, positional reads via `std::os::unix::fs::FileExt::read_at` / `std::os::windows::fs::FileExt::seek_read`.
- `notify` (`RecommendedWatcher`, `RecursiveMode::Recursive`; FSEvents / ReadDirectoryChangesW). Implement the 500 ms / 1500 ms / 3000 ms logic with `tokio::time`, not `notify-debouncer-*`, so timing matches.
- `regex` for the takeover, command, reminder, pasted-content and search patterns. Use ASCII classes where JS has no `u` flag (`(?-u:\w)` or `[A-Za-z0-9_]`), and a JS-compatible whitespace class where JS uses `\s`.
- `tokio::process` for `claude -p` (stdin null, piped stdout/stderr, `kill_on_drop`, `CREATE_NO_WINDOW` on Windows via `creation_flags(0x08000000)`).
- `uuid` (v4, lowercase hyphenated), `indexmap`, `chrono` or `time` for `Date.parse`-style ISO timestamps and `toISOString` output.
- `dunce` or a hand-written realpath that matches Node's JS `fs.realpathSync` (see risks).

Risks:
1. **Path encoding parity.** The encoded folder must match what Claude Code computes, or no transcripts are found. Node's `fs.realpathSync` (JS implementation) resolves symlinks but keeps the given case. `std::fs::canonicalize` calls `realpath(3)` (which can change case on macOS) and returns `\\?\C:\…` on Windows. The non-alphanumeric replacement works on UTF-16 units, so a non-BMP character gives two dashes.
2. **UTF-16 semantics.** Lengths and slices in `truncate` (80/160/400/500), `clip` (280/48), note text (20 000), snippet offsets (48/160) and `matchRanges` are in UTF-16 code units. Rust strings are UTF-8. Count with `encode_utf16`, and don't split surrogate pairs. JS can emit a lone surrogate there; Rust can't, so this is a known tiny deviation.
3. **Regex and case differences.** JS `\s` includes U+FEFF and Rust's doesn't. JS `\w` without `u` is ASCII and Rust's is Unicode. Case-insensitive matching (`giu`) and `toLowerCase` vs `to_lowercase` (special casing) can disagree on rare characters. `Date.parse` is lenient: no offset means local time, and a date alone means UTC.
4. **Numbers.** Times can be fractional (mtime fallback) and are persisted into `state.json` for imported nodes. JS `Math.round` rounds .5 up, while Rust `f64::round` rounds half away from zero, which matters for negative note coordinates: use `(x + 0.5).floor()`. `Math.hypot` and libm `hypot` can differ in the last ulp, which can reorder near-ties in the free-spot search. Both sorts must be stable.
5. **Ordering and side effects.** Node insertion order drives card order and search target order, so it must survive load/save with IndexMap. `sessionList` runs a full `loadBoard`, with import, save and `openedAt`. The watcher's no-restart debounce and the never-reattached watcher on error are behaviors to keep. The status → finishRun → archiveIfAsked → brainStatus order matters.
6. **Process launch.** On Windows the argv carries JSON (`--mcp-config`, `--settings`) and an empty `--tools ""`, so quoting must follow `windowsCommandLine` for the pty and MSVC rules for `-p` children. `-p` children must have stdin closed, or they wait for piped input.

Cross-domain dependencies: 01 (store `getBoard`/`getProject`/`save`, env `spawnEnv`/`requireClaude`/`terminalShell`/envReady, lifecycle calls `unwatchAll`), 02 (pty spawn/kill/write/resize/snapshot/isLive, `hookEnv`/`hookSettings`, status machine `getStatus`/`forgetStatus`/`getUsage`, `attachAgents`/`readAgents`, host welcome `mcp`), 04 (board tools MCP config and rules, `brainArgs`/`brainInput`/`brainStatus`/`startBrain`/`askBrain`, notes constants, `cleanInk`, consumers of `sessionList`/`archiveNode`/`dropNode`/`isEmpty`), 05 (`findAction`, `runArgs`, run results), 07 (`boardOpened`, uses `createSession`/`sendToSession`/`sessionTitle`/`readSession`), 09–12 (renderer callers: stop UI, merge dialog, ⌘K overlay, transcript preview, agent viewer, `cardTitle`).

## Parity checklist

- [x] SESS-001 Transcript folder
- [x] SESS-002 Real repo path
- [x] SESS-003 Path encoding
- [x] SESS-004 Transcript path
- [x] SESS-005 Session files
- [x] SESS-006 Subagents folder
- [x] SESS-007 Agent files
- [x] SESS-008 Agent id shape
- [x] SESS-009 Agents signature
- [x] SESS-010 Byte range read
- [x] SESS-011 Skip partial first line
- [x] SESS-012 Complete lines only
- [x] SESS-013 Line parsing
- [x] SESS-014 Meta lines
- [x] SESS-015 Origin
- [x] SESS-016 Content shapes
- [x] SESS-017 Empty
- [x] SESS-018 Slash commands
- [x] SESS-019 Noise prefixes
- [x] SESS-020 Reminders stripped
- [x] SESS-021 Prompt summary
- [x] SESS-022 Prompt full
- [x] SESS-023 Truncate
- [x] SESS-024 String reply
- [x] SESS-025 Block reply
- [x] SESS-026 Tool display name
- [x] SESS-027 Tool call summary
- [x] SESS-028 Timestamps
- [x] SESS-029 Created/updated
- [x] SESS-030 Branch
- [x] SESS-031 Custom title
- [x] SESS-032 AI title
- [x] SESS-033 Legacy summary title
- [x] SESS-034 Sidechain flag
- [x] SESS-035 Prompts
- [x] SESS-036 Replies
- [x] SESS-037 Sidechain lines counted
- [x] SESS-038 Other records
- [x] SESS-039 Cache hit
- [x] SESS-040 Append-only growth
- [x] SESS-041 Fresh parse
- [x] SESS-042 Large files
- [x] SESS-043 Unreadable file
- [x] SESS-044 Title precedence
- [x] SESS-045 Preview fields
- [x] SESS-046 Message count
- [x] SESS-047 Time fallback
- [x] SESS-048 Eligible
- [x] SESS-049 Has content
- [x] SESS-050 Scan
- [x] SESS-051 Meta signature
- [x] SESS-052 Empty session
- [x] SESS-053 Missing file
- [x] SESS-054 Tail only
- [x] SESS-055 Sidechain
- [x] SESS-056 User messages
- [x] SESS-057 Task notifications
- [x] SESS-058 Assistant turns
- [x] SESS-059 Agent steps
- [x] SESS-060 Tools
- [x] SESS-061 Timestamps default
- [x] SESS-062 Message cap
- [x] SESS-063 One watch per project
- [x] SESS-064 Baseline
- [x] SESS-065 Recursive watch
- [x] SESS-066 Throttle
- [x] SESS-067 Change test
- [x] SESS-068 Folder not there yet
- [x] SESS-069 Watch error
- [x] SESS-070 Lifetime
- [x] SESS-071 Load order
- [x] SESS-072 Forget deleted transcripts
- [x] SESS-073 What gets imported
- [x] SESS-074 Import placement
- [x] SESS-075 Archive grows
- [x] SESS-076 Section cleanup
- [x] SESS-077 Sticker cleanup
- [x] SESS-078 Opened time
- [x] SESS-079 Cards
- [x] SESS-080 Card shape
- [x] SESS-081 Snapshot
- [x] SESS-082 Archive constants
- [x] SESS-083 Columns and slots
- [x] SESS-084 Slot of a card
- [x] SESS-085 Growing
- [x] SESS-086 Archiving outside the board
- [x] SESS-087 Empty session test
- [x] SESS-088 Dropping
- [x] SESS-089 Unsticking notes
- [x] SESS-090 Unknown nodes
- [x] SESS-091 Empty sessions are dropped
- [x] SESS-092 Position
- [x] SESS-093 Title
- [x] SESS-094 Archived flag
- [x] SESS-095 Section
- [x] SESS-096 Expanded
- [x] SESS-097 Size
- [x] SESS-098 Archived cards fold
- [x] SESS-099 Save and notify
- [x] SESS-100 Save section
- [x] SESS-101 Remove section
- [x] SESS-102 Name section sources
- [x] SESS-103 Save note
- [x] SESS-104 Remove notes
- [x] SESS-105 Save ink
- [x] SESS-106 Archive rect and viewport
- [x] SESS-107 Remove a card
- [x] SESS-108 Hidden is forever
- [x] SESS-109 Naming call
- [x] SESS-110 Naming prompt
- [x] SESS-111 Clip
- [x] SESS-112 Clean name
- [x] SESS-113 Naming result
- [x] SESS-114 Create start
- [x] SESS-115 Action lookup
- [x] SESS-116 New node
- [x] SESS-117 Action title
- [x] SESS-118 Node before process
- [x] SESS-119 Create launch options
- [x] SESS-120 Create result
- [x] SESS-121 Claude argv
- [x] SESS-122 Permission mode
- [x] SESS-123 Board tools gate
- [x] SESS-124 Prompt after `--`
- [x] SESS-125 Name only at creation
- [x] SESS-126 Claude env
- [x] SESS-127 Terminal shell
- [x] SESS-128 Process placement
- [x] SESS-129 Initial status
- [x] SESS-130 One process per session
- [x] SESS-131 Host first
- [x] SESS-132 Action run args
- [x] SESS-133 Brain args
- [x] SESS-134 Brain launches
- [x] SESS-135 Open a live session
- [x] SESS-136 Open a stopped session
- [x] SESS-137 Reopened runs
- [x] SESS-138 Write
- [x] SESS-139 Takeover test
- [x] SESS-140 Claiming a run
- [x] SESS-141 Blank prompt
- [x] SESS-142 Must be on the board
- [x] SESS-143 Waiting sessions
- [x] SESS-144 Live send
- [x] SESS-145 Resume to send
- [x] SESS-146 Shared with capture
- [x] SESS-147 Resize
- [x] SESS-148 Kill
- [x] SESS-149 Stoppable
- [x] SESS-150 Stop without asking
- [x] SESS-151 Stop confirmation title
- [x] SESS-152 Stop confirmation message
- [x] SESS-153 Stop waits
- [x] SESS-154 On done
- [x] SESS-155 Finishing a run
- [x] SESS-156 Stale runs
- [x] SESS-157 Archive when done
- [x] SESS-158 Session title
- [x] SESS-159 Session list
- [x] SESS-160 Project stats
- [x] SESS-161 Has transcript
- [x] SESS-162 Session transcript
- [x] SESS-163 Agent transcript
- [x] SESS-164 Agents list
- [x] SESS-165 No conversation
- [x] SESS-166 Summary titles
- [x] SESS-167 Focus
- [x] SESS-168 Queued first
- [x] SESS-169 Three at a time
- [x] SESS-170 Fork command
- [x] SESS-171 Fork timeout
- [x] SESS-172 Fork result
- [x] SESS-173 Results order
- [x] SESS-174 No claude
- [x] SESS-175 Search targets
- [x] SESS-176 Query terms
- [x] SESS-177 Term pattern
- [x] SESS-178 Match ranges
- [x] SESS-179 Empty query
- [x] SESS-180 Phrase
- [x] SESS-181 Repo roots
- [x] SESS-182 Files searched
- [x] SESS-215 A worktree card's transcripts are searched in its worktree
- [x] SESS-183 Yield
- [x] SESS-184 Searchable documents
- [x] SESS-185 Tool documents
- [x] SESS-186 Tool output left out
- [x] SESS-187 Search index cache
- [x] SESS-188 Title coverage
- [x] SESS-189 Document score
- [x] SESS-190 Every word required
- [x] SESS-191 Hit statistics
- [x] SESS-192 Snippets
- [x] SESS-193 Snippet window
- [x] SESS-194 Session ranking
- [x] SESS-195 Title-only hit
- [x] SESS-196 Size constants
- [x] SESS-197 Card size
- [x] SESS-198 Note and view sizes
- [x] SESS-199 Dock rects
- [x] SESS-200 Intersects
- [x] SESS-201 Free spot
- [x] SESS-202 Spot beside
- [x] SESS-203 Events without a window
- [x] SESS-204 Unknown project ids
- [x] SESS-205 Known limitations kept
