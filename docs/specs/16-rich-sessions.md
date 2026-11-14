# 16 · Rich sessions: the chat, Claude's visuals and the prompt queue

Added in 1.0.14; 0.9.4 had none of this. A Claude card no longer shows its terminal: it shows the session as a chat, read from the transcript Claude Code writes. Claude can put charts, stat tiles, tables, question forms and pages into that chat. The user can queue prompts while Claude works, and the session host hands each one to Claude as its turn ends, through the Stop hook, without typing into the terminal. The terminal is still there: it docks under the chat while Claude asks something only the terminal shows (a permission prompt, a picker). A shell card keeps its terminal.

Sources:
- `crates/nodus-domain/src/chat.rs` (`ChatEntry`, `QueuedPrompt`, `Answers`, the Stop hook's reason)
- `crates/nodus-transcripts/src/chat.rs` (`ChatLog`: the transcript read as a chat)
- `crates/nodus-visual` (the visuals' and forms' JSON, chart layout, fenced blocks)
- `crates/nodus-host-proto/src/message.rs`, `crates/nodus-host/src/{host,http}.rs` (the queue and the Stop hook's answer)
- `crates/nodus-sessions/src/hooks.rs` (the Stop hook's command), `client/` (the queue on the app's side)
- `crates/nodus-core/src/sessions.rs` (`transcript_file`, `queue`, `set_queue`, `enqueue`, `deliver`)
- `crates/nodus-tools/src/chat.rs`, `schema.rs` (`show_visual`, `ask`, `show_page`)
- `crates/nodus-views/src/views.rs` (`nodus.submit`), `page/bridge.js`
- `crates/nodus-ui/src/chat/` (the view, its rows, the composer, visuals and forms; `look.rs`, the chat's look and arrivals)

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
- **CHAT-008** A tool's result finishes its call's line: a quiet tick at its end when it succeeded, a red cross when it failed. A call still running has a spinner in place of the tool's glyph.
- **CHAT-009** Claude's words show as Markdown (selectable), its fenced `visual` and `questions` blocks taken out and shown as what they describe, in place. A run of tool calls is one card of lines (`glyph  name  detail`, CHAT-038), the last 3 shown with `Show N earlier calls` above to show the rest, and `Show fewer` to fold them again. Claude's first row after the user's message carries Claude's mark, `Claude` and the time (CHAT-035).
- **CHAT-010** `mcp__nodus__show_visual` shows its `visual`, `mcp__nodus__ask` its form, and `mcp__nodus__show_page` / `create_view` a page card. One the board turned down (its result an error) is a failed tool line instead, with the error's first line. Every other tool is a line.
- **CHAT-011** Thinking isn't shown.
- **CHAT-012** `[Request interrupted…]` is a notice: `Interrupted`.
- **CHAT-013** A background task's notification is a notice with its summary.
- **CHAT-014** What a slash command printed (`<local-command-stdout>`) is a notice, its terminal colours taken out.
- **CHAT-015** A notice is cut to 2000 characters.
- **CHAT-016** An API error Claude Code recorded is a notice with its text.
- **CHAT-017** A compaction is a notice: `Earlier messages were summarized to free up context`.
- **CHAT-018** While the chat is up, it looks at the transcript every 250 ms, off the main thread. The list follows the newest row while it's scrolled to the bottom, and stays put while the user reads further up.
- **CHAT-019** Under the last row, while Claude works: Claude's mark with its sparkle twinkling (CHAT-035) and a light passing through `Thinking…` while the last row is still the user's, `Working…` once Claude has said or done something since. While it waits on the user: `Claude needs you below` beside a red dot. The line rises 6px into place over 320ms when Claude's turn starts.
- **CHAT-020** The composer: Return sends, ⇧Return is a new line, ⌘Return (Ctrl+Return) sends at once, and Esc stops Claude while it works. Its placeholder says what Return will do: `Message Claude…`, `Queue a message for when Claude is done…`, or `Message Claude to resume this session…`. Under it a line says the same in short: `↩ sends · ⇧↩ new line`, `Working · ↩ queues · ⌘↩ sends now · Esc stops`, `Claude is asking something below · ↩ queues`, or `Not running · sending resumes it`.
- **CHAT-021** While Claude works or waits on the user, what's sent is queued (QUEUE-001). The queue shows above the field, numbered, each prompt with `Edit` (back into the field, out of the queue) and `Remove`.
- **CHAT-022** ⌘Return, and anything sent while Claude is idle, is typed in at the prompt as a paste and Return (SESS-141…145): a running Claude takes it at its next step. A session that isn't running resumes with it. Esc and the stop button send Esc.
- **CHAT-023** When Claude is idle with prompts still queued (it was interrupted, so no turn ended), the queue says `Claude stopped before its turn ended` and the first prompt has `Send now`.
- **CHAT-024** The session's terminal docks under the chat while Claude waits on the user (status `waiting`: a permission prompt, a question), and goes again when it stops waiting, unless the user opened it. The dock is 320 px tall in the session window and half the card in an expanded card, with `Claude needs you here` (red dot) or `Terminal` and a close chevron. A card's dock wakes its terminal (XCARD-025). While the session window has the terminal, the card's dock says `The session window has this terminal open.`
- **CHAT-025** Sending a slash command docks the terminal too, as it may open a picker only the terminal shows.
- **CHAT-026** `Terminal` under the composer docks the terminal by hand.
- **CHAT-027** A form's answers are sent as a `<nodus-answers>` envelope (`Answers::to_message`) the way CHAT-021/022 send anything.
- **CHAT-028** A page card shows a tile with the page's glyph, the page's title and `Putting it on the board…`, `On the board, beside this session` or `On the board · Answered`, with `Show on board` (a round-ended button), which closes the session window, selects the view and pans the board to it.
- **CHAT-029** Claude cards show the chat in the session window and in the expanded card, where it draws at the board's zoom; the start, loading and error overlays are the dock's now. The session window's Resume docks the terminal, which attaches and resumes the session. A shell card keeps its terminal, and so does a card on a server board, whose transcript isn't on this computer.
- **CHAT-030** The mic sits in the composer, right of the field and left of Send (05's VOICE-041), not in the card's or window's title bar. Words said into it, or typed in from elsewhere, go into the composer at the caret, and a wake focuses the composer rather than the terminal.
- **CHAT-031** A view's page can call `nodus.submit(answers)`: the answers (any JSON) go to the session that put the view up, as an answers envelope `to` the view's id, sent or queued as CHAT-021/022. A view no session put up can't submit.
- **CHAT-032** A session that stops with prompts still queued (QUEUE-012) gives them back to the composer, one after another, and says `Claude stopped before your queued messages went out. They’re back in the box.`
- **CHAT-033** While Claude shows a dialog in its terminal, the terminal docks and nothing is typed into it. A dialog is a screen that says `Enter to confirm`, `Enter to select`, `Enter to continue`, `Esc to cancel`, `Esc to exit`, `Esc to go back` or `Press Enter to`, or has a line starting `❯ <number>.`: Claude Code's folder trust check, a permission prompt, a question, a picker. The screen is looked at when the terminal draws and every second. While one shows, the dock reads `Claude needs you here`, the line under the composer `Claude is asking something in the terminal below`, and a message that would be typed stays in the composer with `Claude is asking something in the terminal below. Answer it first.` (one that would be queued is queued). The dock goes again when the dialog does, unless the user opened it or Claude still waits on the user.
- **CHAT-034** While the session runs and the dock is down, its terminal is laid out off screen at the dock's size, so its screen can be read and docking it doesn't resize it. The session window's terminal attaches only while the session runs or the dock is up (so opening a stopped session's window no longer resumes it); a card's follows the card (XCARD-019…XCARD-026).

### The chat's look

The chat stays in the theme's neutrals: depth, spacing and motion lift it, not colour. The accent is on Send alone, once there's something to send.

- **CHAT-035** Claude's mark is the sparkle, in the secondary text colour, on a round tile of the sunken surface with a hairline: 22px beside `Claude` (semibold, the text colour) at the head of a reply, 20px on the working line, 40px over the empty chat's words. While Claude works, the working line's sparkle twinkles (MOTION-038).
- **CHAT-036** The list's top 14px and bottom 22px fade into the chat's background. Scrolled up away from the newest row, a round button with a chevron rises over the list's foot (180ms); it takes the list back to the newest row and following it again (CHAT-018).
- **CHAT-037** The user's message is a bubble of the hover surface, corners of 18px but 6px at the bottom right, at most 82% of the column wide, with `Queued while Claude worked` above it when it was queued. An answers card (CHAT-007) is the same bubble, its title after a tick.
- **CHAT-038** A run of tool calls sits on a card (the sunken surface, a hairline, corners of 12px). Each line has the tool's glyph (a terminal for `Bash`, an eye for `Read`, a pencil for `Edit`/`Write`, a magnifier for `Grep`/`Glob`, a globe for web tools, a robot for `Task`/`Agent`, the sparkle for anything else), the name and the detail in mono, and the outcome at its end (CHAT-008). The line of a call still running sits on the hover surface and a light passes through its name. Charts, layouts, forms and page cards carry the small shadow (`--shadow-sm`), which lifts them off the chat.
- **CHAT-039** A notice is a rounded pill in the middle, between hairlines that reach to the column's sides.
- **CHAT-040** What arrives while the chat is up moves in; what the chat opened with is already there. The user's bubble pops up from its bottom-right corner (420ms: rising 10px, scaling from 90% on `--ease-spring`). Claude's words show from the top down behind a soft edge a third of their height deep (900ms on `--ease-out`), rising 4px. A chart, form or page lifts 12px into place from 98% (480ms), and a notice or a run of tools fades up 4px (280ms). Each tool line slides in 8px from the left as its call arrives (320ms). Parity shots and reduced motion show every row in place.
- **CHAT-041** The composer is a field with corners of 22px, lifted off the chat by a shadow (white on the light theme, `#323236` on the dark). With the keyboard, its edge darkens and its shadow deepens (160ms). Send is a 30px circle, grey while the field is empty; with something to send it turns the accent (180ms) and springs from 90% to full size (420ms, `--ease-spring`). Stop is a solid 30px circle in the text colour with a rounded square on it. A queued prompt's number sits in a small round tile.

## The board tools that show things in the chat (TOOLCHAT)

- **TOOLCHAT-001** `show_visual {visual}` checks the visual (VIS-…) and answers `{"shown": true, "note": …}`, or a tool error `The visual can’t be shown: <why>`, or `visual is required: the chart, stats, table or layout, as JSON`. The chat draws it from the call in the transcript (CHAT-010).
- **TOOLCHAT-002** `ask {title, submit?, questions}` checks the form (FORM-…) and answers `{"shown": true, "questions": <count>, "note": …}`, telling Claude to end its turn; or `The questions can’t be shown: <why>`.
- **TOOLCHAT-003** `show_page {title, html, description?, data?, width?, height?}` is `create_view` with those arguments alone (no commands, schedule or sessions), so the page goes up beside the caller's card, from it, and its answer is `create_view`'s plus a `note` on `nodus.submit`.
- **TOOLCHAT-004** The three come after the view tools in every session's `tools/list` and in `--allowedTools` (after `mcp__nodus__list_views`), so they run without asking. The server's instructions end with a paragraph on them and on fenced `visual` and `questions` blocks.

## The prompt queue (QUEUE)

The session host keeps each session's queued prompts and hands the first to Claude as the answer to its Stop hook, so Claude carries on in the same turn and nothing is typed into the terminal. Verified with Claude Code 2.1.281: a Stop hook printing `{"decision":"block","reason":"<text>"}` makes Claude go on with `<text>`, which the transcript records as a user line `Stop hook feedback:\n<text>` with `isMeta: true` (CHAT-006 shows it as the user's).

- **QUEUE-001** **Announced.** A host that keeps queues says `queue: true` in every welcome (after `hookAuthHeader`, before `key`). An app treats a welcome without it (0.9.4, an earlier Rust host) as a host that doesn't, and keeps queues itself (QUEUE-011).
- **QUEUE-002** **Set.** `queue-set {sessionId, items}` from an app replaces a live session's queue in order (less QUEUE-003), and the host broadcasts `queue {sessionId, items}` with the result to every app, even when nothing changed. One for a session that isn't live, or whose `items` don't read as `[{id, text, at}]`, is ignored (HOST-040).
- **QUEUE-003** **Handed over stays gone.** Each session remembers the ids of the last 100 prompts it handed over, and a later `queue-set` leaves those out, so a set that crossed a hand-over can't put one back.
- **QUEUE-004** **Welcome.** Each session in a welcome carries `queue` (first to go first) as its last field, when it isn't empty.
- **QUEUE-005** **Hand over.** A `Stop` hook whose query has `reply=1`, for a live session with a non-empty queue and a payload with no non-empty string `agent_id`, takes the first queued prompt, remembers its id, and is answered `200`, `Content-Type: application/json`, `{"decision":"block","reason":<queued_reason(text)>}` with every non-ASCII character as a `\u` escape. `queued_reason` puts the line `[Nodus] The user queued this message while you were working. Treat it as their next message:` and a blank line before the text.
- **QUEUE-006** **Recorded as a prompt.** That hook is recorded in history and broadcast as `UserPromptSubmit` with payload `{"prompt": <text>, "nodusQueued": true}` (the token as HOST-077), never as `Stop`, then `queue` is broadcast with what's left. The card keeps working, no finished notification goes out, and a relaunched app replays a prompt.
- **QUEUE-007** **Otherwise unchanged.** A `Stop` without `reply=1` (a process an older app started, whose hook throws the answer away), one with an empty queue or from a subagent, and every other event are answered `204` and recorded and broadcast as HOST-072…078.
- **QUEUE-008** **The Stop hook's command.** Only the Stop hook's command changes: `&reply=1` ends its URL and it prints the host's answer (POSIX: only stderr to `/dev/null`; PowerShell: `$r.Content` written when there is any). Every other hook and the status line are byte for byte HOST-177…181 and print nothing.
- **QUEUE-009** **Mirror.** The app keeps each live session's queue from its welcome (after replaying its history) and from each `queue` message, and emits `ev:queue(sessionId, items)` whenever it changes. A session that isn't live has none.
- **QUEUE-010** **Setting.** Setting or adding to a live session's queue changes the mirror at once (emitting `ev:queue` when it changed) and, to a host that keeps queues, sends `queue-set` with the whole list. For a session that isn't live it does nothing.
- **QUEUE-011** **Typed when the host can't.** On a live `Stop` hook (no `agent_id`) for a session whose queue isn't empty and which isn't `waiting`, the app takes the first prompt, remembers its id (the last 100; a later set leaves them out), emits `ev:queue`, sends `queue-set` with the rest to a host that keeps queues, and types the prompt with `sendPrompt` (HOST-122). Hooks replayed from a welcome never type.
- **QUEUE-012** **Ends with the session.** A session's queue goes when its process exits: the host forgets it with the session, and the app, when it had any, emits `ev:queue` with `[]` after `ev:pty-exit` (also when the host is lost).
- **QUEUE-013** **Commands.** `sessions().queue(id)`, `set_queue(id, items)` and `enqueue(id, text)` (trimmed as SESS-141, a new uuid, `at` = now; `There’s nothing to queue` for only spaces) fail with `That session isn’t running` for a session that isn't live. For a session on a server board the queue is empty and changing it fails with `Prompts can’t be queued for a session on a server yet`. `sessions().deliver(project, id, text)` queues while a running session works or waits on the user, and otherwise sends as `session:send` does.

Wire:

```
{"type":"queue-set","sessionId":"<uuid>","items":[{"id":"<uuid>","text":"run the tests","at":1762600000000}]}
{"type":"queue","sessionId":"<uuid>","items":[]}
```
