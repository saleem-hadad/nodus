# 21 · Goals

Added after 1.0.24. An automation (spec 20) starts a fresh session each time it runs, remembering nothing. A **goal** is the other way round: **one session pursued over days or weeks**. The board wakes it for a check-in every so often, resuming the same conversation so it remembers what it tried, and Claude says when it wants to look again. Its tile on the board shows how far along it is, as a % from where it started to its target. Modelled on Open Session's goals (`CONCEPTS.md`): one resumed session, a journal read at every wake, the agent setting its own next wake within a floor, and a cap on wakes.

Sources:
- `crates/nodus-domain/src/board.rs` (`BoardGoal`, `GoalMetric`, `GoalStep`, `GoalPoint`, `GoalEntry`, `GoalStatus`)
- `crates/nodus-store/src/state.rs` (`goals`)
- `crates/nodus-board/src/goals.rs` (starting one, check-ins, Claude's updates, reading when to check in next)
- `crates/nodus-tools/src/schema.rs`, `server.rs`, `handlers.rs` (`goal_update`, `goal_finish`)
- `crates/nodus-core/src/goals.rs` (`window.api.goals`), `schedules.rs` (the ticker)
- `crates/nodus-wire/src/call.rs` (`goals:*`), `crates/nodus-server/src/dispatch.rs`
- `crates/nodus-ui/src/board/cards/goal.rs` (the tile), `goals.rs` (its menu, opening, moving), `goal_dialog.rs`, `mod.rs` (`New Automation`'s menu), `menus.rs`, `scene.rs`, `render.rs`, `edges.rs`, `moments.rs`, `input.rs`

## Data

- **GOAL-001** A board's `goals` sit in `state.json` beside `tiles`, keyed by id, a lenient record map (SHAPE-030), left out while empty. `BoardGoal { id, title, mission, sessionId, x, y, status: active|paused|done|failed, metric?, progress?, steps?, phase?, nextAt?, every, checkIns, maxCheckIns, lastAt?, history?, journal?, reason?, createdAt }`.
  - `mission` is what the user asked for (up to 4000 UTF-16 units); `title` a few words for the tile (60), the mission's first line until Claude names it.
  - `sessionId` is the one session that pursues it.
  - `metric { name, unit?, start?, target?, value? }` is the number it moves. `steps [{ text, done }]` (at most 20) are for a goal without one. `progress` (0…100) is how far along Claude says it is when neither says.
  - `every` is the fewest minutes between check-ins; `checkIns` how many it has had; `maxCheckIns` when it pauses.
  - `history [{ at, value }]` and `journal [{ at, text }]` keep the last 200 each.
- **GOAL-002** How far along it is, 0…100: with a metric's start, target and value (start ≠ target), `(value − start) ÷ (target − start)`; else `progress`; else the share of steps done; else none. It's clamped to 0…100, so it can go down when the number does and stops at 100. From 3.1% to 4% at 3.7% is 67%.
- **GOAL-003** `every` is whole minutes within 30 (the least the user can pick) and a week; it defaults to 60. A goal pauses after 60 check-ins.

## Starting one

- **GOAL-010** The New goal dialog: titled `New goal` (`Make this a goal` for a session), `Say what you want to get to. Claude works on it in one session, checks in on its own and remembers what it tried.`; the mission (placeholder `For example: get signup conversion from 3.1% to 4% within a month.`); `Start from an example` (not for a session); `Check in at most every` with `30 min`, `1 hour` (picked), `6 hours`, `1 day`; a line saying Claude picks when to look again, never sooner, works in auto mode, asks only when it isn't sure, and pauses after 60 check-ins; `Cancel` and `Start goal` (`Starting…` while it starts; off without a mission). ⌘Enter (Ctrl+Enter elsewhere) starts it; Esc closes it. A failure shows as `Couldn’t start it: <why>`.
- **GOAL-011** Starting it (`goals:create { mission, every?, at?, sessionId? }`) puts its tile up and starts its session: a new Claude session in auto mode, named the goal's title, beside the tile, on the first check-in's prompt. The tile goes at `at` (its top left; the dialog sends the tile's middle at the clicked spot), or the nearest free spot to it clear of the board's things and the dock, else beside the rightmost card, else right of the dock. The first check-in counts as check-in 1; the next is a day off until Claude says otherwise. The board selects the tile and pans to it once it's there (SCHED-048).
- **GOAL-012** A Claude card's menu has `Make this a goal…` (not on an archived card, a run, a card already pursuing a goal, or a read-only board): the dialog with the card's title as the mission, and the goal is that session's; its first check-in is sent into it (SESS-141…145) and its tile goes beside it.
- **GOAL-013** The board's menu has `New goal here…` (after `New automation here…`; off over the dock or on a read-only board).
- **GOAL-014** `New Automation` (SCHED-045) is a split button: its menu, `More ways to automate`, has `New goal…`. The examples: `Signup conversion`, `Faster home page`, `Fewer errors`, `Test coverage`.

## Check-ins

- **GOAL-020** Every 30 s with the automations (SCHED-020) the core checks in on each goal that is due: active, `nextAt` passed, its session a card on the board that isn't archived, and that session not working or waiting for the user. A check-in sends the check-in prompt into the session (resuming it when it isn't running), counts it, and sets `nextAt` a day off until Claude says otherwise. At `maxCheckIns` it pauses instead, with `Checked in N times; resume it to go on.`
- **GOAL-021** A goal whose session is waiting for the user isn't woken; its tile says `Waiting for you`. Claude asks with AskUserQuestion (or the ask form) only when it isn't sure what the user would want.
- **GOAL-022** The check-in prompt: `Check-in #N for the goal you are pursuing on the user's Nodus board.`, the mission, then on the first `This is the first check-in…` (measure where it starts, or write the steps; call goal_update with a title, the metric or steps, a journal line and when to check in next), and after it `Where it stands: …` and the last 30 journal lines with when. It ends asking for one meaningful step, then goal_update with the number or steps, a journal line and the next check-in in words, leaving long enough for a change to show; to ask the user only when unsure; and to call goal_finish when it's reached or can't be.
- **GOAL-023** A goal's session gets two more board tools, listed after the chat tools and allowed like the others (`BOARD_TOOL_RULES` grows to 22); another session calling them gets `<name> is only for a goal’s session`. `goal_update { title?, metric?, value?, steps?, progress?, phase?, journal?, next_check_in? }` keeps what's given: a value updates the metric and its history (the first value measured is the start when none was said), steps replace the list, a journal line is added. It answers `{ ok, percent?, next_check_in?, note? }`. A title Claude gives names the session's card too.
- **GOAL-024** `next_check_in` is read without Claude: `in 7 days`, `in 4 hours`, `in an hour`, `in half an hour`, `2 weeks`, `tomorrow` (9:00), `tomorrow at 2pm`, a weekday (`monday`, the next one, 9:00 unless a time is given), `at 9` / `today at 15:00` (today if still ahead, else tomorrow), `2026-11-24` (9:00) and `2026-11-24 09:00`, in local time. Sooner than `every` allows, it's moved to that and the answer's `note` says so. Words it can't read fail with `Couldn’t read when “<words>” is. Try “in 7 days” or “tomorrow at 9”.`
- **GOAL-025** `goal_finish { reached, reason }` ends it: `done` (reached) or `failed`, no next check-in, the reason kept and added to the journal.

## The tile

- **GOAL-030** A goal's tile is 320×212 board units, as an automation's tile is drawn (SCHED-030). Top: a 46-unit ring with the % inside (`–` before there is one), the title (14, semibold) and where it stands (12, secondary): `3.7% now · target 4% · from 3.1%`, `3 of 5 steps`, or `Working out how to measure it`.
- **GOAL-031** Under it, the number at each check-in as a line, with the start (faint) and target (tertiary) dashed across, the last point a dot; or the first three steps, each with a dot that's filled when done.
- **GOAL-032** Then a state line: the phase while its session works, `Waiting for you`, `Next check-in in 3h` within two days (else `Next check-in Mon 9:00`), `Checking in soon`, `Paused` (or why), `Reached`, `Stopped`, with the session's status dot while it works or waits. Then, under a hairline, its last journal line (two lines at most), or `Its first check-in is measuring where it starts.`
- **GOAL-033** The ring is the accent while it's on its way, the waiting colour while it waits for the user, the done colour when reached, and tertiary when paused or stopped. The tile is dragged like a note and saved where it lands (`goals:change` with x and y); dropped on the dock it goes back; frames, groups and links leave it out. The board is drawn again every 20 s while it has goals or tiles.
- **GOAL-034** A double-click opens its session, as a click on the card does. Its menu: `Open its session`, `Check in now` (active; `goals:check-in`), then `Pause` and `Mark as reached` (active) or `Resume` (resumed, it gets another 60 check-ins and checks in now), `Check in at most hourly` / `Check in at most daily` (whichever it isn't), then `Remove goal` (red), which takes the tile off and leaves the session as an ordinary card.
- **GOAL-035** A dotted line runs from the tile to its session while the session is a card on the board, and at each check-in a light runs along it from the tile (SCHED-037); when the session's turn ends, a light runs back.
