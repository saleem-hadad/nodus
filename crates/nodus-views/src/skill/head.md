---
name: views
description: Making views and pages on the user's Nodus board. Views are live dashboards, tables and charts that stay up to date (create_view); pages are mockups, comparisons and small tools beside this session (show_page). Use it when the user asks for a view, a dashboard or other custom UI on the board, and before you write a page's html yourself, with show_page or update_view or as a view's brain.
---

# Views and pages on the Nodus board

Pick the lightest thing that shows it:
- show_visual: a chart, stat tiles or a table in the chat, drawn natively. For a look at numbers now.
- ask: questions with choices, as a form in the chat.
- show_page: a page you write, on the board beside your card, for this conversation: a mockup, layouts side by side, a small tool, or choices whose answers come back to you through nodus.submit. It's yours to change with update_view.
- create_view: a view that lasts and stays up to date, such as a dashboard or a table of live data. It has a brain, a Claude session of its own, that builds its page and keeps it fresh.

## Asking for a view: hand it to its brain

When the user asks for a view, don't build it yourself. Call create_view with a title and a description, and leave out html. The view goes up beside your card straight away, and its brain starts building it.

The brain starts fresh and can't see this conversation, so the description is its whole brief. Put in it:
- What the view shows and what the user wants it for, in their words.
- Where the data comes from: command lines you know work and what their output looks like, the MCP tools to use, files in the repository, the env vars that hold tokens.
- How fresh it should be: every 15 minutes, once a day, or only when the user asks.
- What the user said about how it looks or what it does: the columns, the order, what a click opens, buttons that start sessions.
- What to leave out.

For example: "The open pull requests on this repository, so the user can see what's waiting for review. `gh pr list --json number,title,author,isDraft,reviewDecision,updatedAt,url` works here and gives a JSON array. Newest first, with the number, title, author and age; drafts marked, approved ones in green. Each row opens the PR. Refresh every 15 minutes."

Pass data you already have as data, so the brain can start from it. Then tell the user the view is being built beside this card, and carry on: don't wait for it or check on it.

Write the html yourself only for a small page whose content you already have, and then follow the rest of this skill.

## Writing a page

For show_page, for changing a page with update_view, and for a view's brain, which builds its view this way:
1. Get the data first. Find where it comes from and run the command yourself; shape its output with JSON flags, --format options or jq.
2. Start from the nearest example below and change it to fit. Keep what every page needs: when the data is from, a state for no data yet, one for a failed command, and Claude's look while you're fetching.
3. Put it up, read the errors the tool returns, and fix them.
4. From then on send fresh data with update_view's data, not new html.

A view's brain already has the guide below in its instructions; it only needs the examples.
