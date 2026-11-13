# 16 · Rich sessions: the chat, Claude's visuals and the prompt queue

Added after 1.0.12; 0.9.4 had none of this. A Claude card no longer shows its terminal: it shows the session as a chat, read from the transcript Claude Code writes. Claude can put charts, stat tiles, tables, question forms and pages into that chat. The user can queue prompts while Claude works, and the session host hands each one to Claude as its turn ends, through the Stop hook, without typing into the terminal. The terminal is still there: it docks under the chat while Claude asks something only the terminal shows (a permission prompt, a picker). A shell card keeps its terminal.

Sources:
- `crates/nodus-domain/src/chat.rs` (`ChatEntry`, `QueuedPrompt`, `Answers`, the Stop hook's reason)
- `crates/nodus-transcripts/src/chat.rs` (`ChatLog`: the transcript read as a chat)
- `crates/nodus-visual` (the visuals' and forms' JSON, chart layout, fenced blocks)
- `crates/nodus-host-proto/src/message.rs`, `crates/nodus-host/src/{host,http}.rs` (the queue and the Stop hook's answer)
- `crates/nodus-sessions/src/hooks.rs` (the Stop hook's command), `client/` (the queue on the app's side)
- `crates/nodus-core/src/sessions.rs` (`transcript_file`, `queue`, `set_queue`, `enqueue`, `deliver`)
- `crates/nodus-tools/src/chat.rs`, `schema.rs` (`show_visual`, `ask`, `show_page`)
- `crates/nodus-views/src/views.rs` (`nodus.submit`), `page/bridge.js`
- `crates/nodus-ui/src/chat/` (the view, its rows, the composer, visuals and forms)

## Summary

The chat is a view of the transcript, never a second record: everything it shows can be rebuilt from the JSONL Claude Code writes, so it needs no storage of its own. Claude shows things in it two ways: by calling a board tool (`show_visual`, `ask`, `show_page`), which Claude Code records in the transcript where it happened, or by writing a fenced `visual` or `questions` block in its reply. Answers to a form or a page go back to Claude as the user's next message, in a `<nodus-answers>` envelope, through the same path as anything typed: typed in at once when Claude is idle, queued while it works.

## The chat (CHAT)

- **CHAT-001** A Claude card's chat follows `<home>/.claude/projects/<encoded folder>/<session id>.jsonl`, the folder the card runs in (its worktree when it has one). A missing file is an empty chat.
- **CHAT-002** The chat opens at the last 16 MiB of a long transcript, starting at the first whole line, then reads each line as it's appended. A line still being written waits for its newline.
- **CHAT-003** A transcript that shrinks was replaced, and the chat reads it again from the start.
- **CHAT-004** A subagent's lines (`isSidechain`) aren't the session's conversation and aren't shown.
- **CHAT-005** What the user typed shows as their message: slash commands as `/name args`, pasted blocks with their tags dropped, system reminders and tooling noise left out (SESS-014…020), and `[1 image]` / `[N images]` for pictures.
- **CHAT-006** A prompt the session host handed over through the Stop hook (QUEUE-005) shows as the user's message, marked `Queued while Claude worked`, with the host's lead line taken off.
- **CHAT-007** A message that is a `<nodus-answers to="…" title="…">` envelope shows as an answers card (the title, then each answer as `key: value`), and marks what it answers (a form by its id, a page by its view's id) answered.
- **CHAT-008** A tool's result finishes its call's line: a tick when it succeeded, a red cross when it failed. A call still running has a spinner.
- **CHAT-009** Claude's words show as Markdown (selectable), its fenced `visual` and `questions` blocks taken out and shown as what they describe, in place. A run of tool calls is one row of lines (`name  detail`), the last 3 shown with `Show N earlier calls` above to show the rest, and `Show fewer` to fold them again. Claude's first row after the user's message carries `Claude` and the time.
- **CHAT-010** `mcp__nodus__show_visual` shows its `visual`, `mcp__nodus__ask` its form, and `mcp__nodus__show_page` / `create_view` a page card. One the board turned down (its result an error) is a failed tool line instead, with the error's first line. Every other tool is a line.
- **CHAT-011** Thinking isn't shown.
- **CHAT-012** `[Request interrupted…]` is a notice: `Interrupted`.
- **CHAT-013** A background task's notification is a notice with its summary.
- **CHAT-014** What a slash command printed (`<local-command-stdout>`) is a notice, its terminal colours taken out.
- **CHAT-015** A notice is cut to 2000 characters.
- **CHAT-016** An API error Claude Code recorded is a notice with its text.
- **CHAT-017** A compaction is a notice: `Earlier messages were summarized to free up context`.
- **CHAT-018** While the chat is up, it looks at the transcript every 250 ms, off the main thread. The list follows the newest row while it's scrolled to the bottom, and stays put while the user reads further up.
- **CHAT-019** Under the last row: `Working…` with a spinner while Claude works, and `Claude needs you below` while it waits on the user.
- **CHAT-020** The composer: Return sends, ⇧Return is a new line, ⌘Return (Ctrl+Return) sends at once, and Esc stops Claude while it works. Its placeholder says what Return will do: `Message Claude…`, `Queue a message for when Claude is done…`, or `Message Claude to resume this session…`. Under it a line says the same in short: `↩ sends · ⇧↩ new line`, `Working · ↩ queues · ⌘↩ sends now · Esc stops`, `Claude is asking something below · ↩ queues`, or `Not running · sending resumes it`.
- **CHAT-021** While Claude works or waits on the user, what's sent is queued (QUEUE-001). The queue shows above the field, numbered, each prompt with `Edit` (back into the field, out of the queue) and `Remove`.
- **CHAT-022** ⌘Return, and anything sent while Claude is idle, is typed in at the prompt as a paste and Return (SESS-141…145): a running Claude takes it at its next step. A session that isn't running resumes with it. Esc and the stop button send Esc.
- **CHAT-023** When Claude is idle with prompts still queued (it was interrupted, so no turn ended), the queue says `Claude stopped before its turn ended` and the first prompt has `Send now`.
- **CHAT-024** The session's terminal docks under the chat while Claude waits on the user (status `waiting`: a permission prompt, a question), and goes again when it stops waiting, unless the user opened it. The dock is 320 px tall in the session window and half the card in an expanded card, with `Claude needs you here` (red dot) or `Terminal` and a close chevron. A card's dock wakes its terminal (XCARD-025). While the session window has the terminal, the card's dock says `The session window has this terminal open.`
- **CHAT-025** Sending a slash command docks the terminal too, as it may open a picker only the terminal shows.
- **CHAT-026** `Terminal` under the composer docks the terminal by hand.
- **CHAT-027** A form's answers are sent as a `<nodus-answers>` envelope (`Answers::to_message`) the way CHAT-021/022 send anything.
- **CHAT-028** A page card shows the page's title and `Putting it on the board…`, `On the board, beside this session` or `On the board · Answered`, with `Show on board`, which closes the session window, selects the view and pans the board to it.
- **CHAT-029** Claude cards show the chat in the session window and in the expanded card, where it draws at the board's zoom; the start, loading and error overlays are the dock's now. The session window's Resume docks the terminal, which attaches and resumes the session. A shell card keeps its terminal, and so does a card on a server board, whose transcript isn't on this computer.
- **CHAT-030** Words said into a Claude card's mic, or typed in from elsewhere, go into the composer at the caret, and a wake focuses the composer rather than the terminal.
- **CHAT-031** A view's page can call `nodus.submit(answers)`: the answers (any JSON) go to the session that put the view up, as an answers envelope `to` the view's id, sent or queued as CHAT-021/022. A view no session put up can't submit.
