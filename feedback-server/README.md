# Nodus feedback server

A small Express server that turns **Report a Problem** submissions from the Nodus desktop app into GitHub issues. The app sends the report with any screenshots. The server checks it, uploads the screenshots to GitHub and opens an issue with the token it holds, then returns the issue link.

It has no database. Rate limits live in memory, so a restart (or scaling to zero) resets them. Screenshots stream to a temporary folder rather than memory and are deleted when their request ends; with at most two reports with screenshots handled at once, that needs up to about 200 MB of disk.

## Environment variables

| Name | Required | Default | What it is |
| --- | --- | --- | --- |
| `GITHUB_TOKEN` | yes | | A fine-grained personal access token for the repository (see below). |
| `NODUS_APP_KEY` | yes | | Shared key the app sends as `X-Nodus-Key`. It ships inside every app build, so it isn't a secret; it only turns away generic bots. |
| `GITHUB_REPO` | no | `saleem-hadad/nodus` | Where issues are opened, as `owner/name`. |
| `ISSUE_LABEL` | no | `feedback` | Label put on every issue. If it doesn't exist, the issue is created without it and a warning is logged. |
| `PORT` | no | `3000` | Set by the host. |

The server refuses to start if a required variable is missing.

## API

### `GET /health`

`200 {"ok": true}`. No key needed.

### `POST /v1/feedback`

Send the header `X-Nodus-Key: <NODUS_APP_KEY>` and a `multipart/form-data` body:

| Field | Rules |
| --- | --- |
| `title` | Required. Trimmed, 1 to 256 characters. |
| `body` | Required. Trimmed, 1 to 20,000 characters. The description as the user reviewed it. Its first line may be `Reported by @<reporter>`. |
| `reporter` | Optional GitHub username. |
| `version`, `os`, `arch` | Optional, for example `1.0.2`, `macOS 27.2`, `arm64`. Up to 40 characters each; anything outside `A-Z a-z 0-9 space . _ ( ) -` is removed. |
| `screenshots` | 0 to 5 files, each up to 10 MB and 40 million pixels. PNG, JPEG, GIF or WebP, checked by the file's bytes, not its declared type. |

Send the text fields before the screenshots (the app does). A report whose text is missing or invalid is turned away as its first screenshot starts to arrive, so the server never keeps screenshots it would throw away.

On success: `201 {"number": 42, "url": "https://github.com/saleem-hadad/nodus/issues/42"}`.

On failure the body is `{"error": "<one sentence for the user>"}` with:

- `400`: the report is invalid (missing title, bad username, too many screenshots, not an image, and so on).
- `401`: the app key is missing or wrong.
- `413`: a screenshot or field is too large.
- `429`: rate limited, or busy. The `Retry-After` header gives the seconds to wait. The limits are 5 reports per IP and 60 reports overall, each per rolling hour. A report counts once its text passes; one with screenshots counts as its first screenshot arrives, even if a screenshot is then turned down. Two reports with screenshots are handled at once; a third waits a minute.
- `502`: GitHub didn't accept the report. GitHub's own response is logged on the server, never returned.

### What the issue looks like

- It opens with where it came from: `Sent from Nodus feedback · 1.0.2 · macOS 27.2 · arm64`. That line goes first because the issue is opened under the token's account, and markdown the reporter leaves open (an unclosed `<!--` or `<details title='`) would hide anything after it.
- Every `@mention` in the title and body is broken with an invisible word joiner (`@⁠name`), so the server can't be used to ping people. That includes mentions GitHub only sees once the markdown is rendered, like `_@name_` and `&#64;name`. The one exception is a first line that reads exactly `Reported by @<reporter>`; that mention stays live so the reporter is notified. Email addresses aren't touched.
- Screenshots are decoded and encoded again in their own format, so nothing hidden in or after the image data and no metadata (such as a photo's location) reaches GitHub. One that comes out over 10 MB becomes a JPEG. They're uploaded the same way `gh issue create --attach` does, then added below the description as images.

## Run the tests

Needs Node 22 or newer. The tests use a fake GitHub, so no token or network is needed. `npm ci` fetches [sharp](https://sharp.pixelplumbing.com)'s prebuilt image library for the platform it runs on.

```sh
npm ci && npm test
```

## Run locally

```sh
npm ci
GITHUB_TOKEN=ghp_... NODUS_APP_KEY=dev-key npm start
```

This opens real issues in `GITHUB_REPO`, so point it at a test repository while you try things out.

## Deploy on Laravel Cloud

1. Create an application from the `nodus` repository.
2. Set the root directory to `feedback-server`.
3. Set the build command to `npm ci --audit false`.
4. Set the start command to `node server.js`.
5. Add the environment variables above (`GITHUB_TOKEN`, `NODUS_APP_KEY`, and any overrides).
6. Choose a Flex size and turn on **Scale to zero**.

The server listens on `PORT` on all interfaces, IPv6 included, and finishes in-flight reports when it receives `SIGTERM`.

## The GitHub token

Issues are opened under the account that owns the token.

1. In GitHub, go to Settings › Developer settings › Personal access tokens › Fine-grained tokens › Generate new token.
2. Repository access: **Only select repositories** → the repository in `GITHUB_REPO`.
3. Repository permissions: **Issues: Read and write** and **Contents: Read and write** (screenshot uploads need write access to the repository). Metadata: Read-only is added on its own.
4. Pick an expiry and note it: when the token expires, reports fail with a 502 until a new one is set.
5. Use the token as `GITHUB_TOKEN`.
6. Create the `feedback` label in the repository, or whichever label you set in `ISSUE_LABEL`.

The token can push to the repository, so keep `main` behind a branch protection rule.

GitHub doesn't notify you about your own activity, so reports opened under your account arrive without a notification. Check the `feedback` label: `https://github.com/<owner>/<repo>/issues?q=label%3Afeedback`.

GitHub App installation tokens can open issues, but GitHub's attachment upload answers them with 404 (cli/cli#14309), so screenshots wouldn't attach.
