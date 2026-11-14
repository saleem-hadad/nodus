# Running a Nodus server

A Nodus server runs your projects' boards and Claude Code sessions on a Linux machine your team shares. The app connects to it and shows the server's projects in your grid beside your own, and they open like any other project. How it's built is in [How it works](how-it-works.md#running-on-a-server), and every behavior is in [spec 18](specs/18-hosted.md).

## What a server is for

- **Working together.** Everyone sees the same boards: the same cards, notes and views, each other's pointers, and who started what. Anyone can type into any session.
- **Sessions that don't stop.** Sessions run on the server, so they keep going while your laptop sleeps or the app is closed.
- **A branch per card.** Each card gets its own git worktree and branch, so two people, or two sessions, in one repo don't step on each other.
- **No personal GitHub tokens on the server.** Members sign in with GitHub. Pushes and pull requests go through a GitHub App of the server's own.
- **Shared Claude accounts.** Sessions run on Claude accounts the members add, and move to another account when one hits its limit.

## Requirements

- A Linux machine, usually a cloud VM: Ubuntu 22.04 or later, or Debian 12 or later, on x64 or arm64, with systemd and kernel 5.11 or later. Give it disk for one checkout per card, plus their dependencies.
- Root on it, through `sudo`.
- [Tailscale](https://tailscale.com), recommended. The server listens only on its Tailscale address, or on `127.0.0.1` for an SSH tunnel. Without it, each preview of a dev server a session starts needs a tunnel of its own.
- A GitHub account that can create a GitHub App for the account or organization that owns your repositories.
- The Nodus app on each member's Mac or Windows PC. Keep the app and the server on the same version. Whoever adds a Claude account needs Claude Code on their own computer, for `claude setup-token`.

## Install

On the server:

```sh
curl -fsSL https://github.com/saleem-hadad/nodus/releases/latest/download/install-server.sh | sudo bash -s -- --tailscale
```

The script creates two users (below), installs git, `gh`, Claude Code and rootless Docker, downloads the server and checks its signature, turns on a firewall, starts two services, joins your tailnet tagged `tag:nodus`, and prints the address and a setup code:

```
In Nodus: Connect to server… → http://nodus-box:7420 → code K7QM-4RXD
```

- Running it again upgrades the server in place and keeps its data.
- `--uninstall` removes the services and the binary and keeps the data. `--uninstall --purge` removes the users, their data, their Docker and the firewall too.
- Its log is `/var/log/nodus-install.log`. The services log to the journal: `journalctl -u nodus -u nodus-host`.
- Without `--tailscale`, the server listens on `127.0.0.1:7420` only. Tunnel to it from your computer with `ssh -N -L 7420:127.0.0.1:7420 you@server`, and connect to `127.0.0.1:7420`.

### The Tailscale policy

The server runs Claude in auto mode, so it should reach none of your team's devices. Add this to your tailnet's policy file (**Access controls** in Tailscale's admin console), inside its top-level object:

```jsonc
"tagOwners": {
  "tag:nodus": ["autogroup:admin"]
},
"grants": [
  // Members' devices reach the server: SSH, Nodus, and the previews.
  { "src": ["autogroup:member"], "dst": ["tag:nodus"], "ip": ["tcp:22", "tcp:7420", "tcp:8100-8999"] },
  // Members reach each other. tag:nodus is in no "src", so the server reaches nothing.
  { "src": ["autogroup:member"], "dst": ["autogroup:member"], "ip": ["*"] }
]
```

- The second grant replaces Tailscale's default rule that lets everything reach everything. Remove that rule (`"src": ["*"]`), or the server can reach every device on your tailnet.
- Tailscale refuses a tag nobody owns yet. If you didn't add the policy first, the script joins untagged and says so. Add it, then run `sudo tailscale up --advertise-tags=tag:nodus` on the server.
- Settings → Servers warns while the server isn't tagged.

## Connect and set up

1. In the app, click the chevron beside **New Project** and choose **Connect to Server…** (or Settings → Servers → **Connect to Server…**).
2. Type the server's address: its Tailscale name (`nodus-box`), its Tailscale IP, or `127.0.0.1` through a tunnel. Port 7420 is assumed.
3. Type the setup code the installer printed. It works once, and the server takes 10 tries an hour. For a new one, run `sudo -u nodus -H /opt/nodus/bin/nodus setup-code` on the server. Do this with Tailscale on (or through the tunnel): the setup code is the one secret the app sends before the server has proved who it is. Every token after it goes only to a server that proves it keeps it.
4. The setup wizard has five steps. Each is saved as you finish it, so closing the app picks up where you left off.
   1. **Name.** Everyone who connects sees it, and so does the GitHub App.
   2. **GitHub.** Create the server's GitHub App, for your account or an organization. GitHub's page opens: **tick Enable Device Flow before you create the App**, since Nodus signs people in with it. Leave **Let sessions change GitHub Actions workflows** off unless you need it. Then install the App on the repositories sessions may work on (see [Security rules](#security-rules)), and sign in with GitHub by typing the code the app shows on GitHub's page. You're the server's first member.
   3. **Members.** Invite people by their GitHub login. Nobody is emailed: they connect with the server's address and sign in with GitHub.
   4. **Claude accounts.** On your own computer, run `claude setup-token`, sign in with the Claude account to share, and paste the token it prints with a label. The server tries it once and never shows it again. Sessions can't start until there's at least one. Consumer Claude plans are for individual use under Anthropic's consumer terms; your team decides whether pooling fits the plans you have.
   5. **First project.** Pick a repository to clone. Each card you start in it gets a branch of its own.
5. Tick the projects to show in your grid. Projects other people add stay out of your grid until you tick them in **Show Server Projects…**.

To join a server someone else set up, ask a member to invite your GitHub login, then **Connect to Server…** and sign in with GitHub.

Later, **Add from GitHub on <server>…**, in the same menu as **Connect to Server…**, adds more projects, and Settings → Servers manages members, devices, Claude accounts and the GitHub App, and shows recent activity.

## The two users

- **`nodus`** runs the server itself (`nodus.service`). It keeps the boards, the members and their devices, the GitHub App's private key, the Claude accounts and the audit log in `/home/nodus`, which only it can read. It never runs anything inside a project.
- **`nodus-run`** runs the session host (`nodus-host.service`), and through it everything that touches a repository: Claude Code, terminals, git, view commands, setup scripts and rootless Docker. It owns `/home/nodus-run`, with the projects, their worktrees and the transcripts. It can't read `nodus`'s files or change the server's binary.

So a session that a repository or a web page steers can't take the App key, the other Claude accounts or members' devices. A member, though, can type into any terminal: treat every member as someone with a shell on the server.

What sessions see:

- Their environment comes from `/home/nodus/.config/Nodus/env`, as `KEY=VALUE` lines, and nothing else of the service's. Edit it with `sudo`, then run `sudo systemctl restart nodus`; sessions keep running, and new ones get the change. A Claude token there is ignored.
- They run Docker as `nodus-run` against its rootless daemon, with no `sudo`. A repository can ship an executable `.nodus/setup`, which runs in each new card's worktree before its session starts.
- Each card gets a port from 8100–8999 as `PORT`. While something listens on it, the card shows **Open preview**.

## Security rules

Nodus can't enforce these. Do them before your team starts:

- **Protect the default branch** with a ruleset, so the App can't push to it directly. Sessions then work on their own branches and open pull requests.
- **Keep deploy secrets in GitHub environments** restricted to the default branch, so a workflow on a session's branch can't read them.
- **Install the App only on repositories you'd trust every session with,** never on ones that take outside contributions. Anyone who can merge into an installed repository can change its `.nodus/setup`, which runs next to every session's credentials.
- **Give the VM no cloud instance role** (an IAM role, a service account). The firewall keeps sessions away from the metadata service, but with no role there's nothing there to take.
- **Add your cloud provider's firewall** too, letting only SSH in. Tailscale works without any open port.

Also remove members who leave, and revoke devices you don't recognize. Every member is told when a new device signs in.

## Updating

When the app is newer than the server, Settings → Servers and the server's project tiles show **Update Server**. Any member can press it. The server downloads the newest release, checks its signature against the key built into it, never installs an older version, and restarts. Sessions keep running, unless the new version changed how the server talks to its session host: then they stop, and resume when you open them. Everyone's app shows how the update is going.

Running the install script again also updates the server.

An app and a server that speak different protocols don't talk. The app then offers to update the server, or, when the server is the newer one, says to update the app.

## If the server is compromised

A session that ran hostile code, as `nodus-run`, could read the repositories, the environments of the sessions running at the time (their Claude tokens included) and the Docker volumes, and could have left something behind to run later. Something left behind sees the Claude token of every account a session uses after that, so treat every account as exposed. It couldn't read the App key, the accounts file, members, devices or the audit log, or run anything as `nodus`. To recover:

1. Rebuild the VM from scratch.
2. Replace the App key (Settings → Servers → GitHub → **Replace App Key…**), and delete the old one on the App's page on GitHub.
3. Replace every Claude account's token, and have each owner revoke the old one in their Claude account.
4. Revoke every device (Settings → Servers → Devices). Members sign in again.
5. Read `audit.jsonl` from a copy, not on the old server. It's `/home/nodus/.config/Nodus/audit.jsonl`, and Settings → Servers → Recent activity shows its last 200 lines.

## For maintainers: the release signing key

Installed servers and the install script trust one ed25519 key, and only files signed with it. Before the first release that ships the server, make the key once, from the repository:

```sh
scripts/server-signing-key.sh --backup <file>
```

It makes the keypair, sets the repository secret `NODUS_SERVER_SIGNING_KEY` with `gh secret set`, writes the public key into `packaging/server/signing-key.pub.pem` (which the server builds in) and `packaging/server/install-server.sh`, and deletes the private key unless `--backup` names a file to keep it in. Commit both files. The release refuses to sign until both hold the secret's key. Replacing the key later (`--rotate`) breaks every installed server's updates until it runs a new install script. [Releasing](how-it-works.md#releasing) covers the rest.
