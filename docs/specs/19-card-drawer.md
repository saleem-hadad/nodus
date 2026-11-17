# 19 · The card drawer

Added after 1.0.23. A session card shows what its session made outside the chat: the **pull requests** it opened and the **servers** it started. They sit in a drawer pulled out from under the card. At rest the drawer is a lip of small chips (`#53 ●`, `:5173`). Pointed at, it slides open over whatever is below, one row per item, and each row opens its page in the browser. A server's row can stop it.

Sources:
- `crates/nodus-domain/src/drawer.rs` (`CardPr`, `CardServer`, `PrRef`, a card's kept pull requests)
- `crates/nodus-transcripts/src/prs.rs` (the pull requests a transcript says the session opened)
- `crates/nodus-platform/src/listeners.rs` (servers by their environment, libproc on macOS, Stop)
- `crates/nodus-core/src/drawer.rs` (the server watch, the pull request watch through `gh`), `sessions.rs` (`stop_server`)
- `crates/nodus-board/src/board.rs` (`keep_opened_prs`), `cards.rs` (`servers` on the card)
- `crates/nodus-ui/src/board/cards/drawer.rs` (the drawer), `present.rs` (`drawer_items`, `pr_look`, `uptime`, `drawer_summary`), `session_card.rs`, `agent_deck.rs` (`rest_peek`)

## Pull requests

- **DRAWER-010** A pull request on a card is `CardPr { url, number, title, state, checks }`. `state` is `open`, `draft`, `merged` or `closed`, and `checks` (the last commit's checks together) is `pending`, `passed` or `failed`. Title, state and checks are left out until GitHub has been asked. `PrRef` reads a link `https://<host>/<owner>/<name>/pull/<number>`, with or without anything after the number.
- **DRAWER-011** A session opened a pull request when its transcript has a Bash tool call whose command runs `gh pr create` (or `gh pr new`), or a call to a tool named `…create_pull_request`, and that call's result has a pull request link. Each link counts once, in order. Links that other commands printed, or that Claude only wrote, don't count. When a board loads, an active card's opened pull requests that it doesn't keep yet join the ones it keeps.
- **DRAWER-012** The open pull request on a card's branch joins the card too: the transcript's branch, else a server card's worktree's, once GitHub says one is open there. Branches named `main`, `master`, `trunk`, `develop` and `HEAD`, and the repository's default branch, are never asked about. The repository is the one the card's folder's `origin` is (`git@host:o/n.git`, `https://host/o/n`, `ssh://…`; an SSH alias such as `github.com-work` is GitHub).
- **DRAWER-013** A card keeps its pull requests in `state.json` (`prs` on its node), as GitHub last described them, so they stay through restarts. A merged or closed one stays until the card is archived. An archived card's drawer is gone, and nothing asks about its pull requests.
- **DRAWER-014** The app asks GitHub about each local, unarchived project's active Claude cards every two minutes, and within 15 seconds of a card showing one GitHub hasn't been asked about. It sends one GraphQL query per repository through the user's `gh` (`gh api graphql --hostname <host>`): every kept pull request by number, the open one on each branch (branch names go as variables), and the default branch. A number GitHub can't find leaves only that one as it was. When `gh` is missing, signed out or fails, the project is asked again after ten minutes. Nothing is shown about it. Projects on a Nodus server aren't asked yet, and neither is anything on the server itself.
- **DRAWER-015** GitHub's `OPEN` is `open`, or `draft` when it's a draft; `MERGED` and `CLOSED` are as they say. The checks are `passed` for `SUCCESS`, `failed` for `FAILURE` or `ERROR`, `pending` for `PENDING` or `EXPECTED`, and none without checks. A card whose pull requests changed is saved, and its board told (`BoardChanged`).

## Servers

- **DRAWER-020** A server is a process on this machine, run by the user, listening on a TCP port, that came from a session: its environment has the card's id as `NODUS_SESSION` (the app gives it to the session's process), else as `CLAUDE_CODE_SESSION_ID` (Claude Code gives it to what its Bash tool runs). Both stay in a process's environment through `npm run dev`, `nohup` and a parent's exit. Claude Code's own process isn't one, and neither is what Claude Code runs for itself (an MCP server, a hook, which it marks `AI_AGENT=…_harness`). The card's servers are `CardServer { port, pid, label, startedAt }`, by port. They're never saved.
- **DRAWER-021** The app looks every 3 seconds on macOS, through libproc. Each process's arguments and environment are read once (by pid and start time), and only those from a session have their sockets listed. The look takes about a millisecond on a Mac with ~1000 processes. Boards whose cards' servers changed are told (`BoardChanged`). Windows and Linux don't look yet.
- **DRAWER-022** A server's label is what it runs, from its arguments: an interpreter's or runner's (`node`, `python`, `php`, `bun`, `npx`…) script or module, followed by a short plain word after it (`vite`, `next dev`, `artisan serve`, `http.server`, `manage runserver`). A plain-named script (`cli.js`, `server.php`) is named by its package, else by its interpreter (`php server`). Anything else is its program, followed by a short plain word (`rails server`, `hugo server`, `nodus host`). Addresses and ports a flag took are skipped, and inline code (`python -c`) gives the interpreter's name.
- **DRAWER-023** On a server board, a card's preview port (SRV-127) is a server row labelled `preview` while something listens on it (unless a found server has that port), opening `http://<server's host>:<port>`. It takes the place of CARD-056's glyph in the card's heading. The card's menu keeps `Open preview`.
- **DRAWER-024** **Stop** (a server row's) sends the process SIGTERM, only for a server the card shows on this machine. Its row dims until the next look drops it. A server that had already gone says `That server has stopped already` in the board's toast.

## The drawer

- **DRAWER-030** An active card's drawer lists its pull requests, then its servers. A card in the archive or its stack has none.
- **DRAWER-031** The drawer hangs under the card: 12 units in from each side, its top at the card's bottom edge, or below the agent deck's layers while it has some (the deepest's rest peek, DECK-004). Its corners are round at the bottom (12), and its top is square, as if it came out from under the card. It's `--bg-elevated` at 98% brightness (90% in dark), with `--shadow-sm`, `--shadow-md` while open, and an inner shade along its top edge.
- **DRAWER-032** At rest it's a lip, 22 units tall in 3 + 4 of padding: a chip for each item, at most four and then `+N`, and a chevron. A pull request's chip is its glyph in its colour, `#<number>` (mono 11, `--text-secondary`), and its checks' dot when it's open with checks. A server's is the green `AppWindowMac` glyph and `:<port>`. Each chip's tooltip says what it is.
- **DRAWER-033** A pull request's glyph and colour: open `GitPullRequest` green (`--diff-add`), draft `GitPullRequestDraft` quiet, merged `GitMerge` purple (the page theme's purple), closed `GitPullRequestClosed` red (`--diff-del`), and quiet before GitHub has been asked. Its row shows the glyph, `#<number>` (mono 11, tertiary), the title (12, primary, one line; `Pull request #<n>` until GitHub says), and on the right where it stands: `Running` with an orange dot, `Passed` green, `Failed` red, `Open`, `Draft`, `Merged`, `Closed`. Its tooltip is `#<n> <title>` and a line that says where it stands.
- **DRAWER-034** A server's row shows the green glyph, its address (`localhost:<port>`, mono 11.5, primary), its label (11, tertiary) and how long it has run (`12s`, `3m`, `2h`, `4d`). Rows are 26 units tall with 6-unit radius, and turn `--bg-hover` under the pointer. Then a row's right side shows its buttons instead: ↗ (open), and Stop for a server that can be stopped. A click anywhere else on a row opens its page in the browser: the pull request on GitHub, or the server's address.
- **DRAWER-035** The drawer slides out from under the card when its first item arrives (420ms, `ease-stack`), and back in after its last one goes. It shows its last rows while it slides back. At first sight, as a board opens, it's simply there.
- **DRAWER-036** Pointing at the drawer opens it (220ms `ease-out`): its rows slide out below the lip, the chips give way to a summary (`1 pull request · 2 servers`), and the chevron turns over. When the pointer leaves, it closes after 280ms unless the pointer comes back, as the agents fan does (DECK-010). Presses on the drawer never reach the card or the board.
- **DRAWER-037** A click on the lip pins the drawer open. Another click, a press anywhere else, or Escape lets it close.
- **DRAWER-038** While its drawer is open, the card is drawn above the others (z 1400, DECK-014), so the rows cover what's below. When it closes, the card goes back down unless its agents fan is still open.

## Not covered

- Servers on Windows and Linux, and a server project's pull requests (DRAWER-014, DRAWER-021).
- Stopping a server on a Nodus server.
- Notifications when a pull request's checks finish.
