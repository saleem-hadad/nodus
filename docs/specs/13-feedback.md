# 13 · Report a Problem

Added after 1.0.2; 0.9.4 had none of this (its `Report a Bug…` opened a GitHub issue page, PLAT-051).

Sources:
- `crates/nodus-ui/src/feedback/` (the dialog)
- `crates/nodus-core/src/feedback.rs` (posting, the screen shot, the remembered username)
- `crates/nodus-feedback/` (drafts, personal details, images, the request, the browser link)
- `feedback-server/` (the server that files the issue; its own README)

## Summary

Help › Report a Problem… opens a dialog over the main window. The user writes what happened, adds screenshots if they want, and can give their GitHub username. Review shows exactly what will be posted publicly. Post issue sends it to the Nodus feedback server, which holds the only GitHub credentials, uploads the screenshots and opens an issue on `saleem-hadad/nodus`. When the server can't be reached, GitHub's new-issue page opens in the browser, filled in.

## Opening and closing

- **RPT-001** `Report a Problem…` (Help menu, PLAT-044) sends `report-problem`. The dialog opens over the main window and Settings closes. Nothing happens with no main window (PLAT-036). Each opening starts empty, but for the remembered username (RPT-006).
- **RPT-002** Esc (from a field too), the × button and Cancel close it; so does ⌘W, before any other layer (UI-040). A press on the backdrop doesn't, so a stray click can't throw a draft away. While it's open the board's menu commands (New Session, New Terminal, Search Sessions) do nothing. Focus goes back where it was on close.
- **RPT-003** A row under the title shows the steps: `Write`, `Review`, `Done`; finished steps get a check. The cursor starts in the description. Tab and ⇧Tab go between the step's fields (description and username; title and description) instead of indenting.
- **RPT-004** ⌘↩ (Ctrl+Enter off a Mac) does the step's main button: Review, Post issue, Done. Return in the description is a new line.

## Write

- **RPT-005** The username field is optional and shows `@` before it. A leading `@` typed into it is dropped. GitHub's rule: letters, digits and single hyphens, not at either end, at most 39. Review with anything else shows `That isn't a GitHub username: letters, digits and single hyphens, up to 39.` under the field and focuses it; typing clears it. Under a valid or empty field: `The issue mentions you, so GitHub emails you when Saleem replies or ships a fix. No sign-in.`
- **RPT-006** After a post, the username used is kept in `state.json`'s `localStorage` values under `nodus.report.githubUsername` (a blank one removes it), and the next report starts with it.
- **RPT-007** `Screenshot of Nodus` (macOS only): the dialog steps aside, leaving a clear cover so the board's pages keep drawing as pictures (decision D3), and the app draws the window again into a picture (`Window::render_to_image`, no Screen Recording needed). It waits up to 2 s for the pages' pictures. Off a Mac the button isn't shown.
- **RPT-008** `Part of the screen` (`Screenshot of the screen` off a Mac): the dialog steps aside the same way while macOS's crosshair runs (`screencapture -i -x -o`, Space picks a window); off a Mac the screen under the pointer is taken (CAP-055…CAP-057). The shot is saved in `captures/` as capture's are. Cancelling adds nothing.
- **RPT-009** When macOS keeps the screen from Nodus (no Screen Recording permission): `macOS kept the screen from Nodus. Turn Nodus on in Screen & System Audio Recording, then try again.` with an `Open Settings` button that opens that pane (CAP-059).
- **RPT-010** At most 5 screenshots. At 5 both buttons are disabled; more pasted or dropped are left out with `That's 5 screenshots, the most one report takes.`
- **RPT-011** ⌘V in the description with a picture on the clipboard (image data, or image files) attaches it; anything else pastes as text. Image files dropped anywhere on the dialog attach too (the dialog tints while they're over it). Files read: `.png .jpg .jpeg .gif .webp .bmp .tif .tiff`, up to 64 MB. A drop with none of those shows `Only pictures can be attached: PNG, JPEG, GIF, WebP, BMP or TIFF.`
- **RPT-012** A picture goes as it is when it's PNG, JPEG, GIF or WebP (by its first bytes) and at most 10 MB, GitHub's limit. Anything else readable becomes a PNG. Over 10 MB it becomes a JPEG at quality 85, halved in size until it fits (6 tries); one that still doesn't fit shows `That picture is too large to attach, even made smaller.`
- **RPT-013** The window's picture is a PNG at the drawable's size (2× on Retina), made a JPEG by RPT-012's rule if it's over the limit. If the window can't be drawn: `Nodus couldn't take a picture of its window. Try Part of the screen.`
- **RPT-014** Screenshots show as 96×60 thumbnails inside the description's box, each with a remove button. A notice stays until the next screenshot action or removal.

## Review

- **RPT-020** `Review` is enabled once the description has text and no screenshot is being taken.
- **RPT-021** The title is drafted from the description's first line that isn't blank, spaces collapsed, cut to 80 characters with `…`; `Problem report` if there's nothing.
- **RPT-022** The description is drafted as what the user wrote, trimmed, under `Reported by @<username>` and a blank line when there's a username. The server keeps that one mention (RPT-033).
- **RPT-023** Under the description: `Added under it: Sent from Nodus feedback · <version> · <OS> <OS version> · <arch>` (`macOS 27.2.0`, Node's arch names), the footer the server adds.
- **RPT-024** Before drafting, personal details are taken out of what the user wrote: the home folder (as a whole path start; case and slashes ignored on Windows) becomes `~`; the account name and the computer's names (ComputerName, LocalHostName, `COMPUTERNAME`, the host name) become `<user>` and `<computer>` as whole words, any case, names under 3 characters left alone. A box says so: `Your home folder shows as ~ and your Mac's name is taken out. Nothing else from your Mac is sent.` (`PC` off a Mac).
- **RPT-025** Both fields can be edited. Going back and returning keeps the edits unless the description or username changed, which drafts again.
- **RPT-026** `Back` returns to Write with everything kept (disabled while posting).
- Screenshots show as 120×75 thumbnails under `Screenshots, added to the issue`; the footer says `Opened on GitHub under Saleem's account`.

## Posting

- **RPT-030** `Post issue` (enabled with a title and description, not while posting; `Posting…` meanwhile) sends the report. The app holds no GitHub credentials.
- **RPT-031** `POST <server>/v1/feedback` with `X-Nodus-Key: <the app's key>`. The key ships in every build and isn't a secret; it only turns generic bots away. The server is `nodus_feedback::SERVER_URL`, or `NODUS_FEEDBACK_URL` when set and not empty. Connect timeout 20 s, whole request 180 s.
- **RPT-032** `multipart/form-data`: `title`, `body`, `version`, `os`, `arch`, `reporter` (only with a username), then each screenshot as `screenshots` (`screenshot-<n>.<png|jpg|gif|webp>`, its type).
- **RPT-033** The server answers 201 `{"number", "url"}`; only a `https://github.com/` URL counts. It turns every `@mention` into plain text (a word joiner after the `@`) except a first line that is exactly `Reported by @<reporter>`, uploads each screenshot to GitHub's attachment upload, then opens the issue with the screenshots and footer under the description and the `feedback` label. Details: `feedback-server/README.md`.
- **RPT-034** No server set (RPT-040), no connection, a timeout, or an answer that isn't the server's own JSON: `Couldn't reach the Nodus feedback server` / `Check your connection and try again, or open the issue on GitHub yourself. It opens filled in.` A 502 (GitHub didn't take it): `GitHub didn't take the report` and the server's sentence. Both offer `Open the issue in your browser`, and the post button reads `Try again`.
- **RPT-035** A 400, 401, 413 or 429: `The report wasn't sent` and the server's sentence (a 429 without one: `Too many reports for now. Try again in <n> min.`), with no browser option. Typing clears it.
- **RPT-036** Done: a check, `Thanks, it's on GitHub`, `Issue #<n> is posted. GitHub will email @<username> when Saleem replies or ships a fix.` (without a username: `Follow it on GitHub to see replies.`), an `Open issue #<n>` link to the issue and `Done`. Return or Esc closes.

## The browser instead

- **RPT-040** The server is `https://nodus-feedback.laravel.cloud` (`feedback-server/` on Laravel Cloud). Were `SERVER_URL` empty with no `NODUS_FEEDBACK_URL`, every post would be unreachable (RPT-034) and the browser would take over.
- **RPT-041** `Open the issue in your browser` opens `https://github.com/saleem-hadad/nodus/issues/new?title=…&body=…` (form-encoded as `URLSearchParams`) with the review's title, and its description with `---` and `<sub><footer></sub>` under it. Nothing is posted until the user presses Create there.
- **RPT-042** If the link would be over 8,000 characters (GitHub answers 414), the description goes on the clipboard and the page's body says `_The description is on the clipboard: paste it here._` with the footer.
- **RPT-043** The screenshots are saved as `captures/report-<uuid>/screenshot-<n>.<ext>` and the first is shown in Finder (Explorer), opened before the browser so the page is in front. The dialog then shows `Finish on GitHub`: `The issue is open in your browser, filled in. Press Create there to post it.`, plus `Your screenshots are in the Finder window: drag them into the description.` when there were any and `The description was too long for the link, so it's on the clipboard: paste it in.` when RPT-042 applied.

## Checklist

- [x] RPT-001…RPT-004 opening, closing, steps, keys
- [x] RPT-005…RPT-014 Write: username, screenshots, paste and drop, images
- [x] RPT-020…RPT-026 Review: drafts, personal details, back and forth
- [x] RPT-030…RPT-036 posting and its answers
- [x] RPT-040…RPT-043 the browser instead
