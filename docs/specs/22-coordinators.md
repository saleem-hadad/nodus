# 22 · Coordinators (beta)

Added after 1.0.26, as a beta. A **coordinator** is one Claude session per board that the user talks to and that looks after the board's other sessions, its **members**. It hears from them without polling. When a member ends a turn, waits for the user, or has a pull request change, Nodus hands it a short report. It can start sessions of its own, which stay **inside** it, off the board, until the user shows one there. It's an ordinary chat card with a mark of its own and a dot for each member.

How it works with them:
- A coordinator never polls.
- Its members report by ending their turn.
- It briefs each session it starts on its own.
- It asks the user only for the user's decisions.

The members are the board's cards, so the user's own sessions take part too.

Sources:
- `crates/nodus-domain/src/coordinator.rs`: `BoardCoordinator`, `CoordinatorEvent`, a node's `inside`, `startedBy` and `noCoordinator`, and a report's and a coordinator message's text.
- `crates/nodus-store/src/state.rs`: `coordinators`.
- `crates/nodus-board/src/coordinator.rs`: the playbook, the member's note, the ask rules, starting one, starting sessions inside, members, events and reports. Also `create.rs` (`create_session_with`), `notes.rs` (`active_cards`) and `handlers.rs` (search).
- `crates/nodus-core/src/coordinator.rs`: `window.api.coordinator`, events in from status and pull requests, the 5 s report loop, and the tools' work. Also `nodus.rs` (`spawn_session`, `follow_status`) and `drawer.rs`.
- `crates/nodus-tools/src/schema.rs`, `server.rs`, `handlers.rs`: `start_session` and `message_session`, and `list_sessions` and `archive_sessions` for a coordinator.
- `crates/nodus-capture/src/targets.rs`, `submit.rs`.
- `crates/nodus-ui/src/board/coordinator.rs`, `coordinator_dialog.rs`, `cards/coordinator.rs`, `cards/install.rs`, `cards/session_card.rs`, `cards/expanded_card.rs`, `canvas.rs`, `menus.rs`, `mod.rs` (the `New` menu), `chat/entries.rs` (quiet rows).

## Data

- **COORD-001** A board has at most one coordinator. It's kept in `coordinators` in its `state.json` entry, keyed by its session: a lenient record map (SHAPE-030), left out while empty.
  - The record is `BoardCoordinator { sessionId, createdAt, pending?, today? }`, where `sessionId` is its session, a Claude card on the board.
  - `pending` holds the events not yet reported (COORD-021), at most 50, oldest dropped first. Each is `CoordinatorEvent { at, sessionId, kind: turn|waiting|pr, text }`.
  - `today { day, count }` counts the reports sent on a local day, the day as chrono's days from the Common Era.
- **COORD-002** A node may say how it stands with the coordinator, in fields left out unless they apply:
  - `inside: true`: it's inside, kept in the board's cards but not drawn, as a run is (ACT-032).
  - `startedBy`: the coordinator's session id when the coordinator started it.
  - `noCoordinator: true`: the user took it out.
- **COORD-003** The members are the board's Claude cards that aren't archived, the coordinator itself, a run (`runOf`), a view's brain or a terminal, and that the user hasn't taken out (`noCoordinator`). Those inside are members too.
- **COORD-004** An inside session isn't drawn, and isn't among the active cards: note placement, `list_board`, `look_at_board`, obstacles. It's also left out of ⌘K search, capture's targets and capture's rightmost card. Views' pages still list it, as they do runs. The title bar's running count includes it. Archived, it goes to the archive like any card.

## Starting one

- **COORD-010** The title bar's `New` menu (09 BOARD-147) ends with `New coordinator…` (the `Network` icon, hint `Beta`), after `New goal…`. On a board that has a coordinator it's `Open coordinator` instead, which moves the board to its card (⌘K's jump). A server's or read-only board has neither.
- **COORD-011** The dialog:
  - Its title is `New coordinator` with a `Beta` tag, then `It looks after this board’s sessions: they report to it when they finish, and it can start sessions of its own. Tell it what to look after.`
  - A field, placeholder `For example: when a good batch of work has landed on main, cut a release.`
  - `Start from an example`: `Release in batches`, `Keep main green` and `Review pull requests`, each filling in the field.
  - A line: `Every session on this board reports to it, including the ones you start. It works in auto mode, and asks you before it merges, pushes, publishes a release or deletes a branch.`
  - `Cancel` and `Start coordinator` (`Starting…` while it starts; off while the field is empty).
  - ⌘Enter (Ctrl+Enter elsewhere) starts it and Esc closes it. A failure shows as `Couldn’t start it: <why>`.
- **COORD-012** Starting it (`coordinator:create`):
  - It starts a Claude session in auto mode where a new session would go, on the field's words. Claude Code names it from them, as any new session.
  - The coordinator's record is on the board before the process starts, so the launch gives it the playbook (COORD-030) and the ask rules (COORD-031).
  - The board moves to it and the session window opens on it.
  - A board that already has one fails with `This board already has a coordinator`. Empty words fail with `Say what it should look after`.
- **COORD-013** The coordinator card's menu has `Stop coordinating`. It takes the record away, and the inside sessions come out onto the board beside its card. The session stays an ordinary card, keeping its playbook until it next starts. A coordinator whose card is archived or gone stops the same way, within 5 s.

## Reports

- **COORD-020** A member sends the coordinator an event when:
  - its turn ends (status `done`, from a `Stop` or a `StopFailure`, 02 HOST-226): `turn`, `ended its turn: <Claude’s last words>`, the last words as the card has them (at most 600 UTF-16 units), or `ended its turn` without them;
  - it's still waiting for the user 60 s after it started to: `waiting`, `is waiting for the user`, once per wait;
  - one of its pull requests (19 DRAWER-010) shows up with a state (`opened`), changes state (`open`, `draft`, `merged`, `closed`), or has its checks settle (`checks passed`, `checks failed`): `pr`, `#<number> <title> · <what changed>`.

  A turn the host hands a queued prompt at (16 QUEUE-005) doesn't end, so it sends nothing.
- **COORD-021** Every 5 s, a board whose oldest pending event is at least 15 s old sends its pending events to the coordinator together, as one report, through `deliver` (16 QUEUE-013). A report is typed in when the coordinator is idle, resumes it when it isn't running, and is queued while it works or waits for the user, so it reads it when its turn ends. A board sends at most 100 reports a day; after that, events wait for the next day. The coordinator's own turns send nothing, and nor do cards that aren't members.
- **COORD-022** A report is the line `[Nodus] Board report`, then a line per event: `- <H:MM> · <session id> · “<title>” <text>`, each on one line. Its chat draws it as quiet rows (COORD-052).

## The coordinator's session

- **COORD-030** Its playbook, after `--append-system-prompt`, says:
  - It's this board's coordinator. The user talks to it. Its members are the board's other sessions and those it starts.
  - It keeps its turns short.
  - It never polls, sleeps or watches. Nodus sends `[Nodus] Board report` messages, which are information from the board, not the user's words.
  - It reads a report with care, does what's needed or leaves it, and tells the user only the news. It never messages a member just to acknowledge a report.
  - Its tools. A session it starts gets one task and a brief that stands on its own: what to achieve, what matters, what not to touch, how to check.
  - It asks the user only for the user's decisions, and before anything that merges, pushes, publishes or deletes. It names a member that's waiting for the user.
  - It does what the user asks of it, such as a release, itself, following the project's CLAUDE.md, skills and memory. It hands other real work to sessions.
- **COORD-031** Its `--settings` (02 HOST-180) also has `permissions.ask` for `Bash(gh pr merge:*)`, `Bash(git push:*)`, `Bash(gh workflow run:*)`, `Bash(gh release create:*)`, `Bash(gh release delete:*)` and `Bash(git branch -D:*)`. So Claude Code asks the user before them even in auto mode.
- **COORD-032** Its tools: an action run's (04 TOOL-019: `list_sessions`, `archive_sessions`), then `start_session` and `message_session`, in that order, allowed like the others. `BOARD_TOOL_RULES` grows to 24.
  - Another session calling `list_sessions` or `archive_sessions` gets 04's error; calling `start_session` or `message_session` gets `<name> is only for the board’s coordinator`.
  - For a coordinator, `list_sessions` also gives each session `member` (true or false), `inside: true` when it's inside, and `pullRequests` (the card's, as its drawer keeps them).
- **COORD-033** `start_session { title, prompt }` starts a Claude session inside the coordinator:
  - It runs in auto mode, opened out (it's an open card once shown), named `title` (cut to 60 UTF-16 units), on `prompt`, with `startedBy`, at the free spot beside the coordinator's card.
  - It answers `{ session_id, title, inside: true }`.
  - Errors: `Give the session a title and a prompt`, and `Couldn’t start it: <why>`.
- **COORD-034** `message_session { session_id, text }` sends a member `[Nodus] From the coordinator “<title>”:`, a blank line and the text, through `deliver`.
  - It answers `{ ok: true, queued }`, where `queued` says it waits for the member's turn to end.
  - Errors: `That session isn’t one of your members` and `There’s nothing to send`.

## Members

- **COORD-040** A member starts, and starts again, with a note after `--append-system-prompt`. The note says:
  - The board has a coordinator, “<title>”.
  - Nodus tells it what the member did each time its turn ends and when its pull requests change, so the member needn't report.
  - A message starting `[Nodus] From the coordinator` is a teammate's request, to act on within what the user allowed.
  - The member still asks the user before it merges, pushes or publishes.
  - Releases are the coordinator's unless the user asks.

  A session the coordinator started is also told to end each turn with a short report, since its last message is what the coordinator reads. A member running when the coordinator came gets the note when it next starts.
- **COORD-041** A member's chat draws the coordinator's message as one quiet row, `From <title>` over the text (COORD-052).

## On the board

- **COORD-050** The coordinator's card:
  - Its title starts with the `Network` icon (secondary; 14 on a card, 13 open).
  - Its members show as a dot each in board order, at most 8: a card's status dot, or a ring in its status colour for an inside session. Then `N sessions` (`No sessions`, `1 session`), then `· N waiting for you` (secondary) while some wait.
  - On a compact card they're a button in its footer; on an open card, one in its title bar after the title.
- **COORD-051** Pressing the dots opens the members menu there:
  - A row per member: its title, the `Maximize2` icon for a card or `Eye` for an inside session, and a hint, `On the board` or `Inside`, with `· waiting for you` while it waits.
  - A card's row moves the board to it and flashes it (⌘K's jump). An inside session's row opens it in the session window, where it can be answered.
  - Then a separator and `Show “<title>” on board` for each inside session. That makes it a card at the free spot beside the coordinator, and the board moves to it.
  - With no members, the menu's one line is `No sessions yet. Start one, or ask the coordinator to.`
- **COORD-052** Quiet rows in a chat:
  - A report in the coordinator's chat is a quiet row per event, not the user's bubble. Each row is on the sunken surface with 8-unit corners: the member's status colour as a 7-unit dot, its title (12, medium), and the text (12, secondary) below, at most two lines.
  - A click on a member's row opens it beside the chat, in a pane while the session window is open, else in the window.
  - A coordinator's message in a member's chat is one such row without a dot.
- **COORD-053** A member card's footer shows the `Network` icon (tertiary). Its tooltip is `Reports to <title>`, or `Started by <title>`. A press moves the board to the coordinator.
- **COORD-054** A Claude card's menu, on a board with a coordinator (not archived or read-only):
  - The coordinator has `Stop coordinating`.
  - A member has `Tuck into <title>`, which puts it inside, and `Leave the coordinator`, which takes it out (an inside one comes out onto the board first).
  - A card taken out has `Report to <title>`.

## Not covered yet

- Checks without Claude, such as a command's output changing (main's CI), and a heartbeat. Members' pull requests and their checks cover much of it.
- Lights along a line when a report or message travels.
- Making an existing session the coordinator: a session's tools are fixed when it starts.
- `Show on board` in an inside session's own window. The members menu has it.
- A failed turn told apart from one that ended.
- Boards on a Nodus server (spec 18).
