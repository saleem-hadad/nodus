<p align="center">
<a href="https://saleem-hadad.github.io/nodus/"><img src=".github/assets/screenshot.webp" alt="The Nodus board for a storefront repo, with focus mode on in the title bar and the projects rail on the left. Session cards sit in a Checkout group beside the archive stack, one card is expanded to show its Claude Code terminal, a Tests view lists a failing test that someone circled in red ink with the note Fix before release, and a red ring marks a session that needs you. A sticker on the window reads Free, open source."></a>
</p>

<h1 align="center">Agentic coding, on one board</h1>

<p align="center">
Every Claude Code session in a repo, as a live card. <br>
See which ones need you, and start new sessions from the context of old ones.
</p>

<p align="center">
<a href="https://github.com/saleem-hadad/nodus/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/saleem-hadad/nodus/ci.yml?branch=main&label=build" alt="Build"></a>
<a href="https://github.com/saleem-hadad/nodus/releases"><img src="https://img.shields.io/github/v/release/saleem-hadad/nodus?label=release" alt="Latest release"></a>
<img src="https://img.shields.io/badge/macOS-Apple%20Silicon%20%7C%20Intel-blue" alt="macOS, Apple Silicon and Intel">
<img src="https://img.shields.io/badge/Windows-x64%20%7C%20Arm-blue" alt="Windows, x64 and Arm">
<a href="LICENSE"><img src="https://img.shields.io/github/license/saleem-hadad/nodus" alt="MIT license"></a>
</p>

<p align="center">
<a href="https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-arm64.dmg"><strong>Download for Mac</strong></a> ·
<a href="https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-x64.dmg">Intel</a> ·
<a href="https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-Setup-x64.exe"><strong>Windows</strong></a> ·
<a href="https://saleem-hadad.github.io/nodus/">Website</a>
</p>

---

## 🚀 Get started

1. Download Nodus for [Apple Silicon](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-arm64.dmg) or [Intel](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-x64.dmg) and drag it into Applications. On Windows, run the installer for [x64](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-Setup-x64.exe) or [Arm](https://github.com/saleem-hadad/nodus/releases/latest/download/Nodus-Setup-arm64.exe).
2. Click **New Project** and pick a repo.
3. Press **N** to start your first session.

You need [Claude Code](https://code.claude.com/docs/en/overview) installed, with `claude` on your `PATH`.

## ✨ Features

- 🟠 working · 🔴 needs you · 🔵 done, on every session's card
- The real `claude` terminal, in a window or right on the board
- Start a session from what other sessions already did
- ⌘K (Ctrl+K on Windows) searches everything every session said and did
- An archive of finished sessions. Drag one out to pick it back up
- Sessions keep running when you quit the app
- Focus mode hides every project but this week's
- Views: ask Claude for a live table of your deploys or tickets
- Sticky notes, action buttons and spoken prompts
- Double-tap ⇧ in any app to start a session, jot a note or reply to one

## 🔒 Private by design

Everything stays on your machine. Nodus reads transcripts where Claude Code keeps them and saves only the board layout. Spoken prompts use xAI's speech to text with your own key. Floating capture asks Claude where each capture goes, so what you type there reaches Anthropic as you type it.

## 📖 Learn more

[How it works](docs/how-it-works.md) covers the architecture, every shortcut and the known limitations.

<details>
<summary>Build from source</summary>

You need macOS or Windows, Node.js 22+ and pnpm.

```sh
pnpm install
pnpm dev          # run with hot reload
pnpm typecheck
pnpm test
pnpm dist         # build the DMGs (or on Windows, the installers) into dist/
```

More in [Development](docs/how-it-works.md#development).

</details>

## 📄 License

MIT. Made by [Saleem Hadad](https://github.com/saleem-hadad).

⭐ If Nodus saves you time, star the repo. It helps other people find it.

<sub>Nodus is an independent project, not affiliated with or endorsed by Anthropic. Claude is a trademark of Anthropic, PBC.</sub>
