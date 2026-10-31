<div align="center">

<img src="build/icon.png" width="96" alt="">

# Nodus

**Agentic coding, on one board.**

Every Claude Code session in a repo, as a live card. See which ones need you, and start new sessions from the context of old ones.

**[Download for Mac](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-arm64.dmg)** · [Intel](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-x64.dmg) · [Website](https://saleem-hadad.github.io/nodus/) · Free and open source

<a href="https://www.youtube.com/watch?v=c33K4QcVxFQ"><img src="https://img.youtube.com/vi/c33K4QcVxFQ/maxresdefault.jpg" width="640" alt="Watch the video: I Was Drowning in Claude Code Sessions, So I Built This"></a>

<sub>▶ <a href="https://www.youtube.com/watch?v=c33K4QcVxFQ">Watch the 2-minute video</a> · If it saves you time, a star helps other people find it.</sub>

</div>

## What you get

- **Every session at once.** One card per session, with Claude's latest reply, the branch and the last activity.
- **Know which one needs you.** 🟠 working · 🔴 needs you · 🔵 done, live from Claude Code's hooks.
- **The real terminal.** Click a card for the actual `claude` TUI, or expand it right on the board.
- **Never re-explain.** Select a few cards and start a session that already knows what they did.
- **Agents as a deck.** See what each of Claude's agents is doing, right under its card.
- **Nothing gets lost.** Finished sessions stack up in a searchable archive. Drag one out to pick it up again.
- **Sessions keep running** when you quit the app. Open it again and they're right where you left them.
- **For the small stuff:** action buttons Claude writes for your chores, sticky notes, and spoken prompts.

## See it in action

<table>
  <tr>
    <td colspan="2">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/board-dark.png">
        <img src="docs/screenshots/board.png" alt="The board: a card per session, grouped into sections, with the archive stack on the left">
      </picture>
      <p align="center">Every session in the repo on one board. Red cards need you.</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/terminal-dark.png">
        <img src="docs/screenshots/terminal.png" alt="A card open in the terminal modal, with Claude asking for permission">
      </picture>
      <p align="center">Click a card for the real <code>claude</code> TUI.</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/expanded-dark.png">
        <img src="docs/screenshots/expanded.png" alt="A card expanded on the board, showing its terminal">
      </picture>
      <p align="center">Or expand it in place and watch it work.</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/context-dark.png">
        <img src="docs/screenshots/context.png" alt="The new session from context dialog with handoff summaries of two sessions">
      </picture>
      <p align="center">Start a session from the context of others.</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/archive-dark.png">
        <img src="docs/screenshots/archive.png" alt="The archive stack open with search results for 404">
      </picture>
      <p align="center">Search the archive and drag a session back out.</p>
    </td>
  </tr>
</table>

## Get started

1. **Download** for [Apple Silicon](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-arm64.dmg) or [Intel](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-x64.dmg), open the DMG and drag Nodus into Applications.
2. **Open it once.** The app isn't notarized yet, so macOS blocks the first launch. Go to **System Settings → Privacy & Security** and click **Open Anyway**.
3. **Have Claude Code installed**, with `claude` on your `PATH`. ([Install Claude Code](https://code.claude.com/docs/en/overview))
4. **Click New Project**, pick a repo, and press **N** for your first session.

## Learn more

- **Private by design.** Everything stays on your machine. The app reads transcripts where Claude Code keeps them and saves only the board layout.
- **Spoken prompts** use xAI's speech to text with your own API key.
- **[How it works](docs/how-it-works.md)** covers the architecture, every shortcut, known limitations and [building from source](docs/how-it-works.md#development).

<sub>Nodus is an independent project, not affiliated with or endorsed by Anthropic. Claude is a trademark of Anthropic, PBC.</sub>
