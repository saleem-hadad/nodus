# 17 · Hosted Nodus: a server the app connects to

Added after 1.0.12; 0.9.4 had none of this. `nodus-server` runs the whole core (projects, boards, sessions, views, git, actions) on a Linux machine with no window. The desktop app connects to it over one WebSocket and shows the server projects each person picks beside their local ones. Every call about a server project goes to its server; everything else stays local. With no server added, none of this runs. Setting one up is in [`docs/server.md`](../server.md).

Sources:
- `crates/nodus-server/src/`: `serve.rs` (starting up, the core's config), `net.rs` (where it listens, the `Host` allowlist), `http.rs` (the routes), `socket.rs` (the WebSocket), `dispatch.rs`, `calls.rs` (answering calls), `auth.rs`, `setup.rs`, `store.rs`, `audit.rs` (members, devices, tokens, the setup code, the audit log), `manifest.rs`, `github.rs`, `app_key.rs` (the GitHub App), `hosting.rs`, `credential.rs` (sessions' tokens, installation tokens, git's credential helper, the `gh` shim), `pool.rs` (Claude accounts), `files.rs` (uploads and files), `presence.rs`, `terminals.rs` (working together), `update.rs` (updating itself), `data.rs` (the data folder, the sessions' environment)
- `crates/nodus-wire/src/` (`PROTOCOL`, the routes, frames and calls, presence frames; `client.rs`, the client under the app's)
- `crates/nodus-remote/src/lib.rs` (the app's connection to one server)
- `crates/nodus-core/src/hosting.rs` (what a server's core does differently), `routing.rs` (local core or server), `servers.rs`, `servers/` (the app's servers), `pool.rs` (checking a Claude token), `prompts.rs` (who sent a prompt)
- `crates/nodus-host` (`host --socket`, `exec`, `trust`; 02)
- `crates/nodus-ui/src/servers/` (connecting, the wizard), `settings/servers.rs`, `board/presence.rs`
- `packaging/server/` (`install-server.sh`, `package.sh`, `sign.sh`, `signing-key.pub.pem`), `scripts/server-signing-key.sh`

## Summary

```
 Mac / Windows app                        Linux server
┌──────────────────────────┐  Tailscale  ┌─ user nodus ──────────────────────────┐
│ UI (GPUI)                │  WebSocket  │ nodus serve          (nodus.service)  │
│  ├─ local core ── host   │◄───────────►│  ├─ core (projects, boards, views)    │
│  └─ remote client ───────┼─────────────┤  ├─ members, devices, Claude accounts │
│      (one per server)    │ HTTP :7420  │  └─ GitHub App key                    │
└──────────────────────────┘             ├─ user nodus-run ──────────────────────┤
                                         │ nodus host      (nodus-host.service)  │
                                         │  ├─ claude × N (a worktree per card)  │
                                         │  └─ git, claude -p, view commands     │
                                         │ rootless Docker (project envs)        │
                                         └───────────────────────────────────────┘
```

The server reuses what the app already has: `nodus-core`'s headless config (`NodusConfig`, `EnvSource::Given`), the session host and its protocol, hooks, the board tools' MCP server and the view runtime. They run on the server as on a Mac, because core and host sit on the same machine. What the server adds is the network side, a `Hosting` that gives the core sessions' tokens, GitHub and the Claude accounts, and a `Runner` that sends every child process to the host, which runs as a second user.

## 1 · The server binary

- **SRV-001** The crate `crates/nodus-server` builds the Linux binary `nodus-server`, with no GPUI or wry. The install script puts it on a server as `/opt/nodus/bin/nodus`, so it's `nodus` below. Commands:
  - `nodus serve [--bind <address>] [--host-socket <path>] [--run-home <folder>] [--dev-token <token>]`: the server.
    - `--host-socket` connects to a session host the system runs (SRV-004). Without it, `serve` starts its own as the app does, for development and tests on one user.
    - `--run-home` is the sessions' home (`/home/nodus-run`), where Claude Code keeps transcripts.
    - `--dev-token` lets one fixed token in as no member, for development and tests. `serve` refuses to start with it unless it listens on loopback.
  - `nodus host --socket <path>`: the session host as `nodus-host.service` runs it (HOST-022). `nodus host <socket> <key>`, as the app launches it, still works.
  - `nodus setup-code`: prints a fresh setup code while the server has no member (SRV-060), and refuses once it has one.
  - `nodus git-credential <get|store|erase|gh>`: git's credential helper in sessions (SRV-084).
  - Run under the name `gh` (the link `/opt/nodus/bin/shims/gh`), it's the `gh` shim (SRV-083).
  - Anything else prints the usage and exits 2.
- **SRV-002** `nodus serve` builds the core with the server's own environment (`EnvSource::Given`, SRV-141), the host and runner of SRV-004, the server's `Hosting`, no secret store, and the app's updater off (a server updates as SRV-200 says). It shows no notifications: a session's status reaches the apps as events, and each app tells only the card's starter (SRV-185). Capture, voice and the folder picker have nothing to do there. It holds `<data>/serve.lock`, so a second `serve` can't share the data folder. SIGTERM or SIGINT stops it, saving state and leaving sessions running in the host.
- **SRV-003** **Where data lives.**
  - **The server's data** is in `NODUS_USER_DATA`, else `$XDG_CONFIG_HOME/Nodus`, else `~/.config/Nodus`: `/home/nodus/.config/Nodus` on an installed server, inside `/home/nodus` (0700). The folder is 0700. `state.json` has the app's schema (01). Beside it, readable by `nodus` alone (files 0600):
    - `server.json`: the name, the wizard's steps, the GitHub App's ids and secrets;
    - `members.json`, `devices.json` (SHA-256 hashes of device tokens only);
    - `github-app.pem`, the App's private key;
    - `setup.json` (hashes of the setup code and setup tokens), with `setup.lock`;
    - `session-key`, which sessions' tokens are made with (SRV-143);
    - `claude-accounts.json` and `claude-waiting.json` (SRV-100, SRV-104);
    - `audit.jsonl`, appended to and never rewritten (SRV-210);
    - `env`, the sessions' environment, which a person writes (SRV-141);
    - `board-images/` and `views/` as on a Mac, and `uploads/` when `serve` isn't given `--uploads` (one user, development and tests).
  - **Uploads** go in `serve --uploads <folder>`, which the installer sets to `/srv/nodus/uploads` (2770, `nodus:nodus-run`), so sessions can read them. Each is written 0640 (SRV-163).
  - **The sessions' data** is in `/home/nodus-run` (0750, group `nodus-run`): `projects/<slug>/main` (the clone), `projects/<slug>/worktrees/`, `projects/<slug>/saved/` and `projects/<slug>/setup-logs/` (SESS-206…SESS-212), and Claude Code's own `.claude/` and `.claude.json`, transcripts included.
- **SRV-004** **The session host** runs as `nodus-run`, under its own unit, `nodus-host.service` (SRV-013); `serve` never starts it. `serve --host-socket /run/nodus/host.sock` connects to it (the folder 0750 and the socket 0660, both group `nodus-run`, which `nodus` belongs to) and retries every second until it's up.
  - Restarting or updating `serve` leaves sessions running, and the next `serve` adopts them.
  - When a `serve` connects with another host protocol, the host stops its sessions and exits (HOST-022, HOST-064), and systemd starts it again from the new binary. Each card resumes on open or on send, as it does locally after the host goes (SESS-136, SESS-145).
  - A reboot stops sessions too, and they resume the same way.
- **SRV-005** **Two users.** `nodus` holds everything that has to be trusted. `nodus-run` runs everything that touches repo content.
  - **No child process of the core runs as `nodus`.** Every process the core starts goes through the host as `nodus-run`: sessions and terminals, and also `git`, the background `claude -p` jobs (names, summaries, action drafts, forks, the Claude token check), view commands, `.nodus/setup` and clones. The host's `exec` message runs a process outside any terminal and answers its exit code and output (02, `exec`, `exec-cancel`). The core reaches it through one `Runner`, which on a Mac spawns locally as before. `serve` itself runs only `tailscale`, to find its address and who a device is.
    - This matters because repo config runs code: `core.fsmonitor`, `.git/hooks`, the hooks in `.claude/settings.json`.
  - **`nodus` only reads `nodus-run`'s files.** `nodus` is in the `nodus-run` group, and `nodus-host.service` sets `UMask=0027`, so transcripts and worktrees are group-readable. Reading a transcript is parsing data, not running it.
  - **`nodus-run` reads nothing of `nodus`'s.** `/home/nodus` is 0700, and `/opt/nodus/bin` belongs to `nodus` (0755), so `nodus-run` can run the binary but not change it.
  - **Everything from the host is untrusted.** `serve` checks each hook, status line, board tools request and credential request against that session's own token (SRV-143). It gives the host only what a session needs: its Claude token, its git identity, its environment.
  - **Pre-trusting a folder** for Claude Code is the host's `trust` message (SRV-126), since `.claude.json` is `nodus-run`'s.

## 2 · Install

- **SRV-010** One script, run as root on Ubuntu 22.04+ or Debian 12+ (x64 or arm64, kernel 5.11+, systemd):
  ```
  curl -fsSL https://github.com/saleem-hadad/nodus/releases/latest/download/install-server.sh | sudo bash -s -- [--tailscale]
  ```
  - Options: `--tailscale` (SRV-029); `--version <X.Y.Z>`, that release instead of the latest; `--uninstall` and `--purge` (SRV-012); `--local-binary <path>`, an unsigned binary, for testing only.
  - `NODUS_RELEASE_URL` downloads from a mirror, and the signature is checked all the same. `TS_AUTHKEY` joins Tailscale with that key instead of a login link.
  - The whole script is read before any of it runs, so `curl | bash` can't run half a download. Its log is `/var/log/nodus-install.log` (0600).
- **SRV-011** It does, in order, stopping at the first failure with the step's name and the log's last lines:
  1. Checks systemd, the OS and its version, the architecture and the kernel.
  2. Creates the users and folders:
     - `nodus-run` (home `/home/nodus-run`, 0750) and `nodus` (home `/home/nodus`, 0700), each with its own group, `/bin/bash` as its shell (terminal cards start the user's shell) and subordinate id ranges for rootless Docker;
     - `nodus` added to the `nodus-run` group;
     - `/srv/nodus/uploads`, 2770 `nodus:nodus-run`;
     - `/opt/nodus/bin` and `/opt/nodus/bin/shims`, owned by `nodus`, 0755.
  3. Installs `ca-certificates`, `curl`, `gnupg`, `openssl`, `git`, `uidmap`, `dbus-user-session`, `nftables`, and `gh` from GitHub's apt repository.
  4. Installs Claude Code for `nodus-run` with Anthropic's installer, unless it's there. It updates itself.
  5. Installs rootless Docker for `nodus-run` from Docker's apt repository (`dockerd-rootless-setuptool.sh install`, `loginctl enable-linger nodus-run`).
     - The system-wide daemon is turned off, since ports it publishes skip the firewall's input chain. One that was running already is left on, with a warning.
     - Where AppArmor restricts user namespaces (Ubuntu 23.10+), a profile lets `rootlesskit` make one.
  6. Downloads the release's `nodus-server-linux-<arch>.tar.gz` with its `.sha256` and `.sig`, checks the checksum and then the ed25519 signature against the public key written into the script (SRV-201), and installs `nodus` from it as `/opt/nodus/bin/nodus` (written beside it, then renamed over it). It links `shims/gh` to it. A script whose key is still the placeholder refuses to download, and checking needs OpenSSL 3.0+.
  7. **Firewall**: `/etc/nodus/nftables.nft`, its own table `inet nodus` (IPv4 and IPv6), loaded at boot by `nodus-firewall.service`, so `/etc/nftables.conf` stays the admin's.
     - Incoming traffic is dropped by default, except on `lo`, on `tailscale0`, for SSH (22, and any other port `sshd` listens on), for replies to connections the machine opened, and for IPv6 neighbour discovery and DHCPv6 replies.
     - Traffic from `nodus` and `nodus-run` to the cloud metadata addresses (`169.254.169.254`, `fd00:ec2::254`) is dropped, so sessions can't read the instance's credentials.
     - Rootless Docker's published ports are ordinary listeners owned by `nodus-run`, so these rules cover them too: dev servers and `docker compose` ports bound to `0.0.0.0` stay off the internet.
  8. Writes, enables and starts `nodus-host.service` and `nodus.service` (SRV-013), and waits up to 30 s for the host's socket and port 7420.
  9. With `--tailscale`, installs Tailscale, prints the policy to add (SRV-029) and runs `tailscale up --advertise-tags=tag:nodus`. Then it restarts `nodus.service`, so it listens on the Tailscale address.
  10. Prints the address and a setup code, `In Nodus: Connect to server… → http://<name>:7420 → code K7QM-4RXD`, the SSH tunnel to use while it listens on loopback only, where the logs are, and `Give this VM no cloud instance role, and add your provider's firewall if it has one.`
- **SRV-012** Running it again upgrades in place and keeps the data. A new binary restarts `nodus.service` only, so sessions keep running in the host (SRV-004); a changed host unit restarts the host, and its sessions stop. `--uninstall` stops and removes both services and the binary, and kills the host's leftover sessions. It keeps the data, the users, `nodus-run`'s Docker and the firewall, so nothing left running is exposed. `--uninstall --purge` removes those too. The packages stay.
- **SRV-013** **The units.** Paths are absolute, because in a system unit `%h` is root's home whatever `User=` says.
  - `nodus.service`: `User=nodus`, `SupplementaryGroups=nodus-run`, `ExecStart=/opt/nodus/bin/nodus serve --host-socket /run/nodus/host.sock --run-home /home/nodus-run --uploads /srv/nodus/uploads`, `UMask=0027`, `Restart=always`, `RestartSec=1`, after `nodus-host.service` and `tailscaled.service`.
  - `nodus-host.service`:
    - `User=nodus-run`, `ExecStart=/opt/nodus/bin/nodus host --socket /run/nodus/host.sock`;
    - `RuntimeDirectory=nodus`, `RuntimeDirectoryMode=0750`, `RuntimeDirectoryPreserve=yes`;
    - `UMask=0027`, `Environment=DOCKER_HOST=unix:///run/user/<uid>/docker.sock`;
    - `Restart=always`, `RestartSec=1`, `KillMode=process`, so restarting the host's own process leaves its sessions running.

## 3 · Network

- **SRV-020** `serve` listens on port 7420 of the machine's Tailscale IPv4 address when Tailscale is up (`tailscale ip -4`), else of `127.0.0.1`, for an SSH tunnel. `--bind <ip>` or `--bind <ip>:<port>` picks another, but only loopback or the Tailscale address, since there's no TLS: never `0.0.0.0`. A server that started on loopback because Tailscale wasn't up yet, as at boot, looks again every 5 s, and once Tailscale has an address it listens there too, without a restart.
- **SRV-021** Plain HTTP. Tailscale's WireGuard encrypts the traffic, and an SSH tunnel encrypts its own.
  - A request whose `Host` (with or without a port) isn't `localhost`, `127.0.0.1`, the address it listens on, or the machine's Tailscale address or MagicDNS names (full and short) gets 403. That blocks DNS rebinding.
- **SRV-022** Endpoints:

| Route | Auth | What |
|---|---|---|
| `GET /api/hello` | none | `{ name, version, protocol, needsSetup }` |
| `POST /api/setup/claim` | the setup code | `{ token }`: a setup token (SRV-060) |
| `POST /api/auth/device`, `POST /api/auth/device/poll` | none, or a setup token in the wizard | GitHub device sign-in (SRV-041). The poll names a random handle the server gave, never the user code. |
| `GET /api/ws` | device or setup token | the WebSocket (SRV-023) |
| `POST /api/github/setup/nonce` | setup or device token | a single-use nonce, good for 10 minutes |
| `GET /github/setup/start?nonce=…&owner=…`, `GET /github/setup/callback` | an unused nonce, then GitHub's code with that nonce as `state` | creating the GitHub App (SRV-062) |
| `PUT /api/uploads?name=…`, `GET /api/files/<id>` | device token | uploads, board pictures and views' pages (SRV-163). Only ids the server issued are looked up. |

  - Tokens travel only in the `Authorization: Bearer` header, never in a query string or a cookie. That also stops a web page in someone's browser from opening the WebSocket as them.
  - A request body is at most 64 KB, uploads aside.
- **SRV-023** **The WebSocket.** JSON text frames, written as JavaScript writes them (`nodus_domain::json`). A frame over 4 MB closes the connection. There are three kinds:
  - Call, from the app: `{ "id": 7, "call": "board:load", "args": {…} }`. The answer is `{ "id": 7, "ok": … }` or `{ "id": 7, "err": { "channel", "message" } }`.
  - Event, from the server: `{ "ev": "pty-data", "p": {…} }`, named after the core's `ev:*` channels, plus the server's own: `server-notice` (SRV-042, SRV-103), `accounts-changed`, `presence` (SRV-180), `pty-size` (SRV-144) and `server-update` (SRV-200).
  - Presence, from the app: `{ "presence": {…} }` (SRV-180).
  Calls start in the order they came, each up to its first wait at once, so keys typed reach a session in order.
- **SRV-024** Call names are the core's channel names, the ones `CommandError` carries (`board:load`, `session:create`, `views:call`, `git:status`, …). Arguments are an object named as the app's API names them (`{ "projectId", "sessionId" }`), and answers are the serde forms the domain types already have. `Event` serializes to the wire's form. Besides the core's calls, a connection makes `watch` and `session:close` (SRV-026), and the server answers its own:
  - setup: `setup:status`, `setup:name`, `setup:done`;
  - people: `members:list`, `members:invite`, `members:remove`, `devices:list`, `devices:revoke`, `audit:list`;
  - the server: `server:info`, `server:update`;
  - GitHub: `github:installations`, `github:repos`, `github:replace-key`, `projects:add-github`;
  - Claude accounts: `accounts:list`, `accounts:add`, `accounts:rename`, `accounts:remove`.
  A setup token may make only `setup:status`, `setup:name` and `github:installations`; every other call needs a device token. Every member may make every call (SRV-040).
- **SRV-025** **What the server answers:** everything about its projects, which is projects, groups, board, sessions, views, git and actions. What stays on the app: settings, zoom, local storage, capture, voice, the app's own updates, menus, Report a Problem and revealing a folder. A call the server doesn't answer fails with `Not available on a server`.
- **SRV-026** **Who gets which events.** A connection gets a project's events only after `watch { projects: [ids] }`, and each `watch` replaces the last. It gets a session's `pty-data` only between `session:open` and `session:close`, or until it disconnects. Status, agents, usage and `board-changed` go to every connection watching the project; `projects-changed`, `accounts-changed` and `server-update` go to every connection. `watch`, `session:open` and `session:close` take effect at exactly their place in the stream of events.
- **SRV-027** `protocol` is an integer, `PROTOCOL` in `nodus-wire`: 2 (1 until presence, `pty-size` and ink by the stroke came; SRV-144). It isn't the session host's protocol (02). The app reads the hello first and doesn't talk to a server of another protocol. Its status then says `This server runs Nodus <v>. Update it in Settings → Servers.`, or `This server runs a newer Nodus. Update the app.` for a newer one (UI-309, UI-315). `server:update` is the one call it makes across protocols (UPD-095).
- **SRV-028** **Dropped connection.** The app keeps showing the last board, read-only, under a `Reconnecting…` pill (UI-313, UI-314), from the snapshot it keeps in `server-boards/<projectId>.json` (01). Calls fail at once meanwhile with `Not connected to the server`. It tries again after 1, 2, 4… seconds, at most every 30. Once back it sends `watch` before anything else, reloads the boards it shows, and opens the terminals that were open again, which replay from the host's buffer (TERM-111).
- **SRV-029** **Tailscale access rules.** A tailnet isn't always just the team, and the server runs Claude in auto mode. With `--tailscale`, the installer prints this policy before it joins, and `docs/server.md` gives it too:
  ```
  "tagOwners": {
    "tag:nodus": ["autogroup:admin"]
  },
  "grants": [
    // Members' devices reach the server: SSH, Nodus, and the previews.
    { "src": ["autogroup:member"], "dst": ["tag:nodus"], "ip": ["tcp:22", "tcp:7420", "tcp:8100-8999"] },
    // Members reach each other. This replaces the default rule that
    // lets everything reach everything: tag:nodus is in no "src", so
    // the server, which runs Claude in auto mode, reaches nothing.
    { "src": ["autogroup:member"], "dst": ["autogroup:member"], "ip": ["*"] }
  ]
  ```
  - Tailscale refuses a tag nobody owns yet. When it refuses `tag:nodus`, the installer joins untagged and says to add the policy and run `sudo tailscale up --advertise-tags=tag:nodus`.
  - Settings → Servers warns while the server isn't tagged, or Tailscale isn't running on it (UI-327).

## 4 · People

- **SRV-040** A member is `{ githubId, login, name, avatarUrl, addedBy, addedAt }`. **Members are matched by `githubId`.** `login` is only for display and is refreshed at each sign-in, because GitHub logins can be renamed and then claimed by someone else.
  - All members are equal: anyone can invite and remove members, manage Claude accounts and the GitHub App, approve view commands and update the server. There are no per-call checks, because a member can type into any terminal and so is in effect a user on the server (§13). The last member can't remove themselves.
  - **Removing a member** revokes their devices, whose sockets close. With `removeAccounts`, which the app offers ticked, the Claude accounts they added go too (UI-332).
- **SRV-041** **Sign-in** uses GitHub's device flow with the server's GitHub App client id:
  1. The app asks the server to start, sending the computer's name. The server gives it the user code and a random poll handle. The app shows the code with a button that opens GitHub's page (UI-320).
  2. The server polls GitHub, gets the person's numeric id and login, and checks the id against the members.
  3. A member gets a device token. Anyone else gets `<login> isn't a member of this server. Ask a member to invite you.`, and the refusal is audited.
  - The App must have **Enable Device Flow** ticked on GitHub; without it the server can't start a sign-in (502).
  - In the wizard, the holder of a live setup token who signs in first becomes the first member (SRV-060).
- **SRV-042** **Device tokens.** 32 random bytes, sent as a Bearer token.
  - The server keeps only its SHA-256, with the member, the device's name, its Tailscale identity (`tailscale whois` of the address it signed in from), and when it was made and last seen.
  - The app seals the token with the platform's secret store (Keychain or DPAPI), as the voice key is (PLAT-115), in `servers` in its `state.json` (01 `ServerRecord`).
  - Settings → Servers → Devices lists every device with **Revoke** (UI-324). Revoking closes that device's live WebSocket at once.
  - A device signing in tells every member `<name> signed in on <device>` (UI-328). Device codes can be phished, and this is how you'd notice.
- **SRV-043** The GitHub user token from sign-in is dropped once the id and login are known. Members never give the server a GitHub token that pushes. The App pushes (§6).
- **SRV-044** **Inviting** means typing a GitHub login. The server resolves it to its numeric id (`GET /users/<login>`) and stores both. Nobody is emailed: the invited person connects with the server's address and signs in.
- **SRV-045** Avatars load only from `avatars.githubusercontent.com`. The app fetches each once and keeps it.

## 5 · Onboarding (in the app)

- **SRV-060** **Connect to Server…** sits in the projects grid's add menu (UI-308) and in Settings → Servers (UI-315). It asks for an address (`nodus-box`, `100.101.102.103` or a URL; port 7420 when none is given) and asks `/api/hello` (UI-316).
  - A server that's set up goes to sign-in (SRV-041), then to the project picker (SRV-161).
  - A server that needs setup asks for the setup code, claims it and runs the wizard (UI-317).
  - **The setup code** is 8 letters and digits with no look-alikes, shown as `K7QM-4RXD`. It's single-use, and a server takes 10 tries an hour at it, right or wrong. Only its hash is kept.
  - **The setup token** the claim gives:
    - works only for wizard steps 1 and 2 (SRV-024's three calls, the nonce, signing in);
    - expires after an hour unused, and a route that refuses it doesn't keep it alive;
    - stops working once the first member exists. Sockets that came in with one then close with `Setup is done; sign in to carry on`, and steps 3–5 use that member's device token.
  - `nodus setup-code` makes a new code, and cancels every earlier code, every setup token and the tries so far.
- **SRV-061** The wizard has five steps (UI-318). Each is saved on the server as it finishes (`setup:status`), so closing the app resumes where it stopped.
  1. **Name** (UI-318): the machine's name at first.
  2. **GitHub**: create the App (SRV-062), install it on repositories, then sign in (UI-319, UI-320). The person who signs in is the first member.
  3. **Members**: invite GitHub logins (UI-321). Can be skipped.
  4. **Claude accounts**: add at least one (SRV-100, UI-333). Can be skipped; sessions can't start until there's one.
  5. **First project**: pick a repository to clone (SRV-120, UI-335). Can be skipped.
- **SRV-062** **Creating the GitHub App** uses GitHub's manifest flow (UI-319).
  1. The app asks the server for a nonce (SRV-022), then opens `/github/setup/start?nonce=…&owner=<organization or empty>` in the browser.
  2. That page posts a manifest to GitHub with the nonce as `state`. The manifest asks for:
     - a private App named `Nodus · <server name>`;
     - permissions: contents, pull requests and issues (write); checks, actions and metadata (read);
     - no webhook and no events, and `redirect_url` `http://<the address the browser used>/github/setup/callback`.
  3. **Workflows (write)** is left out unless the wizard's checkbox "Let sessions change GitHub Actions workflows" is ticked, which it isn't at first. A session holding that permission could push a branch whose workflow sends the repo's secrets out. Without it, GitHub refuses pushes that touch `.github/workflows/`.
  4. The wizard tells you to tick **Enable Device Flow** on GitHub's page, because the manifest can't set it.
  5. GitHub sends the browser back to the callback, which it can reach because the browser is on your tailnet (or tunnel). The callback is refused unless its `state` is a nonce the server gave and nobody has used; it's used up first. Without that check, anyone who can reach port 7420 could hand the server an App they own.
  6. The server exchanges the code for the App's id, slug, client id and secrets and its private key, and keeps them (SRV-003). The key never goes to an app.
  7. The wizard then shows **Install on Repositories**, which opens the App's `installations/new`, and waits until the server sees an installation. It says: pick only repositories you'd trust every session with, never ones that take outside contributions (SRV-085).
- **SRV-063** **Replace App Key…** in Settings → Servers → GitHub (UI-325) takes a `.pem` made on the App's page, checks it against GitHub before it replaces the old key, and then says to delete the old key on GitHub.

## 6 · GitHub

- **SRV-080** The App is the only git credential on the server.
  - The server mints each installation token for one repository, with only the permissions the job needs (the token API takes `permissions`):
    - git: `contents: write`;
    - the `gh` shim: contents, pull requests and issues, write.
  - Tokens are cached per installation, repository and use for 50 minutes, then minted again.
  - That narrows what a leaked token can do. Sessions can't mint their own, because `github-app.pem` is `nodus`'s (SRV-005).
- **SRV-081** Cloning, fetching and pushing go through the credential helper (SESS-209). Tokens are never written into `.git/config`, remote URLs or a session's environment.
- **SRV-082** Commits are authored as the member who started the card: `GIT_AUTHOR_NAME` is their name (else login), and `GIT_AUTHOR_EMAIL` is `<githubId>+<login>@users.noreply.github.com`. The committer is the App's bot. GitHub shows the person's avatar on each commit, and pushes come from the App (SESS-209).
- **SRV-083** `gh` works in sessions through a shim placed first on `PATH`: `/opt/nodus/bin/shims/gh`, a link to the server binary. It asks for the repository's token the way the credential helper does, sets `GH_TOKEN`, then runs the real `gh`. Pull requests open as the App's bot.
- **SRV-084** `nodus git-credential` runs as `nodus-run` and holds no secret. It asks the host's local port (`NODUS_PORT`) with the session's own token and id (`NODUS_TOKEN`, `NODUS_SESSION`). The host relays the request to `serve`, which checks the token (SRV-143) and mints the token itself, since the App key never leaves `nodus`. It answers only for the repository of the requesting session's project, and only for `github.com` (or the GitHub a test points it at).
- **SRV-085** **What `docs/server.md` tells teams.** Nodus enforces none of this:
  - protect the default branch with a ruleset, so the App can't push to it directly;
  - keep deploy secrets in GitHub environments restricted to the default branch, so a workflow on a session's branch can't read them;
  - install the App only on repos you'd trust every session with, never on repos taking outside contributions. Anyone who can merge into an installed repo can change its `.nodus/setup` (SRV-123), which runs, pre-trusted (SRV-126), next to everyone's credentials.

## 7 · Claude accounts

Sessions on a server run on Claude accounts its members add, from `claude setup-token`. Consumer Claude plans are for individual use under Anthropic's consumer terms; each team decides whether pooling fits the plans its members have.

- **SRV-100** An account is `{ id, label, addedBy, token, addedAt, state: ok|limited|failed, resetsAt?, fiveHour?, sevenDay?, lastUsedAt? }`, kept in `claude-accounts.json` (0600).
  - A member adds one by running `claude setup-token` on their own computer and pasting the result with a label (UI-331).
  - The server tries a new token with one `claude -p "Reply OK" --output-format json --max-turns 1` run, in the host as `nodus-run`, with any API key left out of its environment. It takes the token only when that exits 0 and says `is_error: false`. Otherwise: `Claude turned that token down: …`. A token already there: `That token is in the pool already, as <label>`. A label is 1 to 64 characters.
  - Tokens never go back to an app. Apps see the label, who added it, its state, usage and last use.
- **SRV-101** **Picking an account** (SESS-213). A session start, and every background `claude -p` (names, summaries, action drafts, forks, views' brains), takes the account that:
  - has state `ok`;
  - has the lowest `fiveHour` (unknown counts as 0);
  - and, on a tie, was used least recently.
  The process gets it as `CLAUDE_CODE_OAUTH_TOKEN`, and the card records `account` and `accountLabel` (CARD-058). With none to take, the start fails with `No Claude accounts yet. Add one in Settings → Servers.`, `Every Claude account is at its limit until <time>.` or `Every Claude account has stopped working. Add one in Settings → Servers.` A `CLAUDE_CODE_OAUTH_TOKEN` in the server's `env` file is ignored, with a warning in the log, so it can't put everything on one account.
- **SRV-102** **Usage.** The status line reports each session's `rate_limits.five_hour` and `rate_limits.seven_day` (`used_percentage`, `resets_at`), which become its account's usage, only with the session's own token (HOST-226, SRV-143).
- **SRV-103** **Limits.** A server's sessions have a `StopFailure` hook (HOST-226).
  - With `rate_limit`, the account becomes `limited` until `resetsAt`, or for an hour when that's unknown.
  - With `authentication_failed`, `account_on_hold` or `billing_error`, it becomes `failed`, and every member is told `<label>'s Claude account stopped working.`
  - A limited account goes back to `ok` by itself at `resetsAt`.
- **SRV-104** **Moving a session** (SESS-214). When a turn fails with `rate_limit`, or its account fails, and another account is `ok`, the server stops the process and resumes it with `--resume <id>` on that account, sending `Continue.` to the one whose turn failed. Transcripts carry over, since every session runs as `nodus-run` in one home. The card shows `Moved to <label>` (CARD-059). With no account free, the card shows `Waiting for a Claude account until <time>`, and resumes the same way once one is `ok`, oldest first. Who's waiting is kept in `claude-waiting.json`, so a restart of `serve` still resumes them.
- **SRV-105** Settings → Servers → Claude accounts lists the accounts with their usage and reset times, plus **Add**, **Rename** and **Remove** (UI-330). Any member can remove any account.
  - Removing an account moves the sessions using it, as SRV-104 does.
  - Removing it from Nodus doesn't revoke the token: the dialog says it works until it expires or its owner revokes it in their Claude account.

## 8 · Projects, worktrees, environments

- **SRV-120** **Adding a project.** **Add from GitHub on <server>…** (UI-329) lists the repositories the App's installations reach (`github:repos`), with a link to install it on more. Picking one clones it to `projects/<slug>/main`, named and coloured from the repository, and links it into the grid. Any member can add one. Two members adding the same repository at once get one project. `projects:add` with a folder already on the server still works, for development and tests.
- **SRV-121** **The main checkout is never edited.** The server fetches into it and nothing more.
- **SRV-122** **A worktree per card** (SESS-206). Each new card, Claude or terminal, on a project from GitHub gets its own worktree, `projects/<slug>/worktrees/<cardId-8>`, on a branch `nodus/<title-slug>-<cardId-4>` cut from `origin/<default>` after a fetch. Its process starts there, not in the project's folder.
- **SRV-123** **Setting up a worktree** (SESS-208). If the repo has an executable `.nodus/setup`, it runs in the new worktree before the session starts, and the card shows `Setting up…` with the script's last line. The session starts even when the script fails or takes over 30 minutes, and the card shows why. A session can write the script when it's asked to set the project's environment up, so every later card starts ready.
- **SRV-124** **Removing a card** removes its worktree (SESS-212). Uncommitted changes or unpushed commits are saved first to `saved/<cardId>-<time>/`, as a patch and a `git bundle`, and kept for 30 days. Archived cards keep their worktrees.
- **SRV-125** **Transcripts** are found under the card's own folder, its worktree (SESS-211), and so is what ⌘K searches (SESS-215). Importing sessions from disk (SESS-073) looks only at the main checkout.
- **SRV-126** **Trust** (SESS-207). Before a session first starts in a new worktree, the host marks the folder trusted in `/home/nodus-run/.claude.json` (`trust`, 02), as accepting Claude Code's trust dialog would. Otherwise every new card would stop at that dialog.
- **SRV-127** **Preview port.** Each card gets a free port from 8100–8999, as `PORT` and `NODUS_PREVIEW_PORT`. While something listens on it, the card shows **Open preview**, which opens `http://<server>:<port>` in the browser (CARD-056). Dev servers and `docker compose` ports usually bind `0.0.0.0`; the firewall keeps them on the tailnet (SRV-011 step 7).
- **SRV-128** **Git panels** follow the selected card's worktree, and the main checkout with no card selected (GIT-056).
- **SRV-129** **Environments** are the project's own business. Sessions run Docker as `nodus-run` against its rootless daemon (`DOCKER_HOST`, SESS-209), so nobody needs `sudo`.

## 9 · Sessions on the server

- **SRV-140** Sessions start as they do locally (SESS-121, HOST-129): the real `claude`, unchanged, with the same hooks, status line and board tools.
- **SRV-141** **The environment** comes from `<data>/env` (`KEY=VALUE` lines; a line that isn't one stops `serve` from starting) instead of a login shell, read when `serve` starts (SESS-209). On top of it, Nodus sets:
  - `HOME=/home/nodus-run`; a `PATH` (`/usr/local/bin:/usr/bin:/bin` when the file gives none) with `/opt/nodus/bin/shims` first and Claude Code's own folders; `DOCKER_HOST`, unless the file sets it;
  - for each session: `CLAUDE_CODE_OAUTH_TOKEN` (SRV-101), `GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `GIT_CONFIG_*` for the credential helper, `NODUS_SESSION`, `NODUS_PORT`, `NODUS_TOKEN` (SRV-143), and `PORT` and `NODUS_PREVIEW_PORT` (SRV-127).
  Nothing else from the service's environment passes through.
- **SRV-142** **Anyone can type into any session.** Each prompt records who sent it: the member whose app last wrote to the session before Claude's `UserPromptSubmit` hook, else the card's starter. A card keeps the last 200 (`prompts: [{ at, by }]`). The session's preview shows the sender's avatar and login beside each of their turns (TRANS-025).
- **SRV-143** **One token per session** (SESS-210). A session's `NODUS_TOKEN` is an HMAC-SHA256 of its id under the server's `session-key`, which `serve` makes once (0600). So it holds across restarts of `serve`, and nobody without the key can work one out. `serve` gives it to the host with the spawn.
  - It covers everything a session sends back to Nodus: hooks (`StopFailure` among them), the status line, the board tools' MCP calls and the credential helper.
  - A request is taken only for the session its token belongs to, by the host and again by `serve`. There's no shared-token fallback. Otherwise one session could fake another's usage or failures, and mark other members' Claude accounts limited or failed.
  - Local Nodus keeps its one shared token.
- **SRV-144** **Terminal size** follows whoever typed last (TERM-114, XCARD-033).
  - Each app says its terminal's size when it opens a session and when it resizes. The session's terminal takes the size of the app it follows: whoever typed into it last, else the first to open it. Typing from an app whose size differs resizes the terminal to that size first.
  - Every app with the session open gets `ev:pty-size { sessionId, cols, rows }`, first when it opens, before the replay, and its terminal takes that size rather than its container's. Other viewers' terminals reflow.
  - The wire's `PROTOCOL` went from 1 to 2 with this, so servers and apps update together (SRV-027).
- **SRV-145** **View commands** run through the host as `nodus-run` in the main checkout, with SRV-141's environment and no Claude token. Any member can allow or turn down one, the view records who allowed it (`approvedBy`), and the audit log gets a line either way (SRV-210).

## 10 · The app's side

- **SRV-160** The app keeps one `Remote` (`nodus-remote`) per server, which keeps itself connected. Each API in `nodus-core` that takes a project or a session asks first whether it's a server's: if it is, the call goes to that server through its `Remote`, and the answer comes back as the type the local call gives; if not, it runs locally as before (`routing.rs`). A session resolves to its project through the board snapshot. Calls about every project, such as the title bar's running list (`projects:stats`), ask the local core and every connected server, and merge the answers. The UI's call sites didn't change.
- **SRV-161** **Linking a project.** The app keeps its servers in `state.json` (`servers`, 01 `ServerRecord`: `id`, `url`, `name`, the sealed `token`, and the `login` and `githubId` it signed in as). Each server project shown in the grid gets an entry in the app's own `projects` with `server: <id>` and an empty `repoPath`. That entry holds only per-person fields (group, focus, order, when it was opened); name, colour and actions come from the server.
  - **Show Server Projects…** (the add menu, Settings → Servers) lists each server's projects with a checkbox (UI-310).
  - Projects other members add aren't shown until you tick them.
- **SRV-162** **In the grid,** server projects mix with local ones: the same groups, ordering, focus and rail. A server badge sits on the tile, its tooltip naming the server (UI-311). While the server can't be reached, the badge dims and the tile opens the last board read-only (SRV-028).
- **SRV-163** **Local paths don't cross.**
  - Reveal in Finder (Show in File Explorer) and the folder picker are hidden for server projects (UI-312).
  - Files dropped on a server session's terminal and pictures added to a server board are uploaded first (`PUT /api/uploads`), and the server's path or name is typed or kept (TERM-112, SHAPE-041).
    - The server picks the stored name, a UUID with the original's extension; the original name is kept only as a label.
    - It keeps them in the `--uploads` folder (SRV-003), written 0640 so the session's user can read them through the group.
    - An upload over 50 MB is refused before anything is sent: `Files over 50 MB can’t go to a server`.
  - Board pictures and views' pages load through `GET /api/files/<id>`, and pictures are kept in `server-files/<server>/` (01).
  - A server view's page and its bridge calls go only to that server, never to the local core (VIEW-020).
  - Local sessions' board tools don't see server projects (TOOL-033), and floating capture doesn't offer them as targets (CAP-149).
- **SRV-164** **Per-person board state stays in the app.** The viewport and which cards are expanded are kept for server boards in the app's own `state.json` (`serverBoards`, 01), never sent, and put into every snapshot the server gives. Everything else on the board is shared.
- **SRV-165** **Settings → Servers** shows, for each server (UI-315):
  - its address, version and status, and who this device signed in as;
  - Members, invite and remove (UI-323);
  - Devices, revoke (UI-324);
  - Claude accounts (SRV-105, UI-330);
  - GitHub: the App, its installations, Install on more repositories, Replace App Key… (UI-325);
  - Recent activity, the audit log (UI-326);
  - Update server, while the server is older than the app (SRV-200);
  - Disconnect, which forgets the server and its projects on this device and changes nothing on the server.
  A notice from the server asks every list again (UI-323).

## 11 · Working together on a board

- **SRV-180** **Presence** (BOARD-182). While a server board is open, the app sends `{ "presence": { project, cursor, viewport } }` whenever its pointer or view changed, at most every 50 ms, and repeats its last frame every 4 s so someone reading quietly doesn't drop out. Opening another board, or leaving them all (`project: null`), takes it off this one at once.
  - `cursor` is the pointer on the board in flow units, null while it's off the canvas.
  - `viewport` is the board point at the centre of the sender's canvas, plus their zoom, not their canvas transform. People with different window sizes then follow the same spot.
  - The server relays `ev:presence { project, peers: [{ login, device, color, cursor, viewport }] }` to the other connections on that board, at most 20 times a second per board, keeping only each peer's newest frame. Each connection's outgoing presence holds only the newest, so a slow app never queues stale cursors.
  - A peer drops out 10 s after its last frame, or at once when it leaves the board or disconnects.
  - Who a peer is comes only from its connection's token; a `login` in the frame is ignored. A dev-token connection shows as `dev`, and a setup token is never on a board.
  - A read-only board and a local board send nothing.
- **SRV-181** The title bar shows the others on the board, each person once, as avatars ringed in their colour (UI-334). Their pointers are drawn on the canvas with their login, gliding between updates (BOARD-180). A person's colour is worked out from their login, so it's the same on every device.
- **SRV-182** **Follow mode** (BOARD-181). Clicking an avatar follows that person: your view flies to theirs at each update until you pan, zoom, move the view any other way or press Esc, or they leave. A frame and a `Following @<login>` pill in their colour show it.
- **SRV-183** **Who made what.** Cards, notes, views, shapes, links, sections and ink strokes made on a server board record `createdBy: <login>`. What a session makes with board tools records the session's starter. A card's menu and tooltip show `Started by <login>` (CARD-054), and a note's hover `Written by <login>` (NOTEUI-050).
- **SRV-184** **Concurrent edits.** Saves are per-object patches (`board:save-nodes`, `board:save-note`, `board:save-shapes`, …), so the last write to each thing wins. Ink, which a local board saves as one whole list, goes a stroke at a time on a server board: `board:add-ink { projectId, strokes }` and `board:remove-ink { projectId, ids }` (INK-013, INKUI-028). Two people drawing at once keep both drawings. Undo on a server board takes back only that step's own strokes, on top of the ink as it is now.
- **SRV-185** **Notifications.** "Needs attention" notifies only the card's starter: the app whose signed-in login is the card's `createdBy`. Terminals and views' brains never notify (HOST-207). Everyone still sees the status on the board.
- **SRV-186** **Board changes are announced.** After every board edit a member makes through the server (cards moved, archived or removed, sections, notes, ink, shapes, links), every card started, and every view saved or removed, the server sends `board-changed` to everyone watching that project, and each app loads the board again (BOARD-183).

## 12 · Updates and the audit log

- **SRV-200** **Update server** (UPD-086…UPD-095). The server reports its version in `/api/hello`. While the app is newer, Settings → Servers and the server's tiles show **Update Server** (UI-315, UI-336, UI-337), which any member can press. It sends `server:update {}`, and:
  1. The server works out the target itself: the newest release that isn't a draft or prerelease and has this machine's `nodus-server-linux-<arch>.tar.gz` and its `.sig`. One that isn't newer than what it runs changes nothing. The app never names a version, so the server never downgrades.
  2. It downloads both, at most 256 MB, and checks the archive's ed25519 signature against the public key built into the running binary (`packaging/server/signing-key.pub.pem`). A build with no key, as with the placeholder, refuses before it asks GitHub anything.
  3. It takes `nodus` out of the archive, checks it's built for this machine, writes it as `/opt/nodus/bin/nodus.new`, syncs it and renames it over the binary.
  4. It answers `restarting`, and a second later stops as SIGTERM stops it. systemd starts the new version (`Restart=always`).
  The host keeps running, and its sessions with it, unless the new version speaks another host protocol (SRV-004). Every member's app shows how it's going (`ev:server-update`: checking, downloading with a percentage, latest, restarting, or the error), whoever pressed it. Running `install-server.sh` again updates too (SRV-012).
- **SRV-201** **Releases** add `nodus-server-linux-x64.tar.gz`, `nodus-server-linux-arm64.tar.gz` and `install-server.sh`, each with a `.sha256` and a `.sig` (REL-048).
  - The server is built on Ubuntu 22.04, so it needs no newer glibc than 2.35.
  - Signatures are ed25519 over the whole file, 64 raw bytes, made in the release workflow with the private key in the `NODUS_SERVER_SIGNING_KEY` secret.
  - The public key is in `packaging/server/signing-key.pub.pem`, which the binary builds in, and in `install-server.sh` between its `SIGNING KEY` markers. `scripts/server-signing-key.sh` writes both. A test fails when they differ, and the release's signing step refuses unless both are the secret's.
  - A `.sha256` from the same release only catches a damaged download. The signature is what's trusted, because the install script runs as root.
  - The server's release jobs never hold the app's release up, and the app's asset names don't change.
- **SRV-210** **The audit log.** `audit.jsonl` gets one JSON line per event, and nothing ever rewrites it. Each line has the time, the member, the device and details. The events: `setup.claimed`, `server.named`, `member.signed-in`, `member.refused`, `member.added`, `member.invited`, `member.removed`, `device.revoked`, `github.app-created`, `github.key-replaced`, `github.installations-changed`, `account.added`, `account.renamed`, `account.removed`, `account.failed`, `view.command-approved`, `view.command-denied`, `server.updated`, `server.up-to-date` and `server.update-failed`. Settings → Servers → Recent activity shows the last 200 lines in words (UI-326).

## 13 · Security model

**A member is in effect a user on the server.** Anyone signed in can type into any terminal, so checking permissions call by call would protect nothing. The defences are against two things.

- **Non-members:**
  - the firewall and the Tailscale rules (SRV-011, SRV-029);
  - sign-in by GitHub id (SRV-040, SRV-041);
  - hashed, revocable device tokens (SRV-042);
  - the setup code and nonce rules (SRV-060, SRV-062);
  - Bearer tokens in headers only, and the `Host` allowlist (SRV-021, SRV-022).
- **Untrusted content steering a session:** repo code, dependencies, issues and web pages. The defences:
  - one token per session (SRV-143);
  - the credential helper limited to the session's own repository (SRV-084);
  - tokens limited by repository and permission (SRV-080);
  - no `workflows` permission by default (SRV-062);
  - the metadata block (SRV-011);
  - `tag:nodus` reaching nothing (SRV-029);
  - the rules on branches, environments and which repos to install on (SRV-085).

**Two users (SRV-005).** A compromised session runs as `nodus-run`. That can:
- read the repos, and the other sessions' environments through `/proc`, with the Claude tokens and session tokens of the sessions running at that moment;
- read the Docker volumes;
- stay on through `nodus-run`'s cron or user units.

It can't:
- read the GitHub App key, the other Claude accounts, the members, devices, session key or audit log;
- add a member or a device;
- change the server binary or `serve`'s state.

So rebuilding `nodus-run`'s side and replacing the Claude tokens that were live recovers the server. The App key and the device tokens stay safe.

**Recovery playbook**, also in `docs/server.md`:
1. Rebuild the VM from scratch.
2. Replace the App key and delete the old one (SRV-063).
3. Replace every Claude token, and have each owner revoke the old one.
4. Revoke all devices.
5. Read `audit.jsonl` from a copy, not on the old server.

## Testing

- CI's `linux` job lints and tests every crate but the app's UI and binary on Ubuntu, `nodus-server` and `crates/nodus-core/tests/headless.rs` included.
- `crates/nodus-server/tests/` runs the real binary on loopback with a scratch data folder and a stand-in `claude`:
  - `serve.rs`: a project from a local repo, its board, a session started, streamed, written to, killed and reconnected;
  - `remote.rs`: an app core adding the server, linking a project and using it through its ordinary API, across a restart of `serve`;
  - `auth.rs`, against a fake GitHub: claim, wizard, sign-in and a non-member refused; a renamed login with another id refused; a callback with a wrong, used or missing `state` refused; a revoked device's socket closed; setup tokens that stop working; the dev token only on loopback;
  - `github.rs`, with the host on its own socket as `nodus-host.service` runs it, so every git command runs there: two members' cards on their own branches, pushes through the fake GitHub with the right author, a removed dirty card leaving a patch and a bundle, the credential helper refusing another project's repository, requests without the session's own token refused, search in a worktree;
  - `files.rs`: uploads and files, ids the server didn't issue refused;
  - `pool.rs`: usage, a `rate_limit` moving the session, a failed account's sessions moved, waiting across a restart, a faked `StopFailure` without the session's own token changing nothing;
  - `together.rs`: presence named by token and relayed at most 20 times a second, dropped after 10 s or on disconnect; the terminal following the last typist; each prompt's sender; who allowed a view's command; two people drawing at once;
  - `update.rs`, against a fake releases API and a throwaway key (`NODUS_DEV_UPDATE_KEY`, debug builds only): no key, a bad signature and an older version refused; the newest signed release installed, then the server exiting.
- The app's automation driver has a `connect` command, so e2e flows can run on a server project.
- The two users, the firewall and the install script were checked by hand on an Ubuntu 24.04 VM, not in CI.

## Not yet

What the design has that isn't built:
- `nodus status` (version, address, members, accounts, live sessions, and a warning when the server isn't tagged). Settings → Servers shows these instead.
- permessage-deflate on the WebSocket.
- Installation tokens for reading CI (checks and actions, read only). The `gh` shim's token has contents, pull requests and issues, so `gh` can't read checks or workflow runs with it.
- The pull request credit `Started by @login in Nodus` in PR bodies.
- Removing a member doesn't suggest replacing the App key.
- Removing a whole server project leaves its worktrees and saved work on disk.
- The new tile showing a clone's progress: Add from GitHub shows `Cloning…` in its dialog.
- The exact steps for revoking a `setup-token` on Claude's side.
- Capture to a server project (CAP-149), the chat view and prompt queue on server cards (CHAT-029, QUEUE-013), and local sessions' board tools reaching server boards (TOOL-033).
- Until the release signing key is made (`docs/server.md`, for maintainers), no release carries the server: the install script refuses to download, and servers refuse to update.

## Not in this spec

Browser and phone clients. HTTPS and public ingress. Per-member isolation on the server, or roles: the two users separate the server from the sessions, not members from each other. Webhooks. Moving a project between a laptop and a server. Local projects in worktrees. Backups beyond git.
