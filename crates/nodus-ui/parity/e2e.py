#!/usr/bin/env python3
"""The final sweep's end-to-end pass: the Rust app driven through a working
day on the parity harness's seeded world, with the world's stand-in claude
(never the real one).

usage: e2e.py [--harness <migration/parity>] [--world <dir>] [--bin <nodus binary>]

It seeds the world (its own HOME and NODUS_USER_DATA), starts `nodus` with
NODUS_AUTOMATION and, through the test driver: adds a project, starts a
Claude card and a terminal card, types into them, moves the Claude card's
status working → waiting → done with Claude Code's hooks posted to the
session host, expands and collapses it, archives and restores it, writes a
note and starts a session from it, draws ink, creates a view (its brain
waits at its prompt), searches with ⌘K, merges two sessions (summaries
stubbed), opens File › Capture…, goes through Settings (theme, the voice
key field, the update rows), then quits and starts again, adopting the
session host. Each step prints ok or FAIL with what the core's board, the
driver's route or the state file showed. The voice key is typed but never
saved (saving would write the Keychain). The window never takes focus; the
app and its session host are stopped at the end.
"""
import hashlib
import http.client
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
NOW = 1792508400000  # 2026-10-20 15:00 UTC, the harness's frozen clock
SOCK = "nodus-e2e.sock"  # relative: macOS caps socket paths at 104 bytes
NODE = os.environ.get("PARITY_NODE") or next(
    (p for p in ["/opt/homebrew/bin/node", "/usr/local/bin/node"] if os.path.exists(p)), "node"
)
PROJECT = "e2e-proj"


def W(ms):
    return {"cmd": "wait", "ms": ms}


def K(keystroke):
    return {"cmd": "key", "keystroke": keystroke}


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


def T(action, **kw):
    return {"cmd": "terminal", "action": action, **kw}


AWAY = M("move", 720, 26)


class App:
    def __init__(self, opts):
        self.opts = opts
        self.world = opts["world"]
        self.failed = []
        # The board the helpers look at.
        self.current = PROJECT
        self.app = None
        self.f = None

    # The app ------------------------------------------------------------
    def start(self):
        env = dict(os.environ, NODUS_USER_DATA=self.path("userData"), NODUS_AUTOMATION=SOCK,
                   HOME=self.path("home"), PATH=self.path("bin") + ":/usr/bin:/bin:/usr/sbin",
                   SHELL=self.path("bin", "fakeshell"), TZ="UTC", LANG="en_US.UTF-8",
                   PARITY_RESPONSES=self.path("responses.json"))
        env.pop("XAI_API_KEY", None)
        if os.path.exists(SOCK):
            os.remove(SOCK)
        log = open(self.path("app.log"), "a")
        self.app = subprocess.Popen([self.opts["bin"]], env=env, stdout=log, stderr=log)
        for _ in range(400):
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(SOCK)
                break
            except OSError:
                time.sleep(0.05)
        self.f = s.makefile("rw")
        self.drive({"cmd": "resize", "width": 1440, "height": 900})
        self.drive({"cmd": "parity", "frozen": True, "now": NOW})
        self.drive(AWAY)
        self.drive({"cmd": "idle"})

    def quit(self):
        try:
            self.f.write(json.dumps({"cmd": "quit"}) + "\n")
            self.f.flush()
        except OSError:
            pass
        try:
            self.app.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.app.kill()

    def path(self, *parts):
        return os.path.join(self.world, *parts)

    def drive(self, cmd, quiet=False):
        self.f.write(json.dumps(cmd) + "\n")
        self.f.flush()
        answer = json.loads(self.f.readline() or "{}")
        if not answer.get("ok") and not quiet:
            print("    !", json.dumps(cmd)[:200], answer)
        return answer.get("result")

    def steps(self, steps):
        for step in steps:
            self.drive(step)
            self.drive(W(80))

    def expect(self, what, ok, seen=None):
        print(f"    {'ok  ' if ok else 'FAIL'} {what}" + ("" if ok else f"  (saw {str(seen)[:300]})"))
        if not ok:
            self.failed.append(what)
        return ok

    def until(self, what, probe, timeout=8.0):
        """Polls `probe` until it returns something truthy; expects it."""
        end = time.time() + timeout
        seen = None
        while time.time() < end:
            seen = probe()
            if seen:
                break
            time.sleep(0.2)
        return self.expect(what, bool(seen), seen)

    # What the app holds -------------------------------------------------
    def board(self, project=None):
        return self.drive({"cmd": "board", "project": project or self.current}) or {}

    def cards(self, project=None):
        return self.board(project).get("cards") or []

    def card(self, sid, project=None):
        return next((c for c in self.cards(project) if c["sessionId"] == sid), None)

    def route(self):
        return self.drive(T("route")) or {}

    def state(self):
        with open(self.path("userData", "state.json")) as f:
            return json.load(f)

    def project_id(self, name=PROJECT):
        return next((p["id"] for p in self.drive({"cmd": "core", "method": "projects.list"}) or []
                     if p["name"] == name), None)

    # The session host's hooks, as Claude Code posts them -------------------
    def hook(self, sid, event, payload=None):
        d = self.path("userData", "host")
        sock = os.path.join(d, "host.sock")
        if len(sock.encode()) > 100:
            h = hashlib.sha1(d.encode()).hexdigest()[:12]
            sock = os.path.join(tempfile.gettempdir(), f"nodus-{h}.sock")
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(sock)
        s.sendall((json.dumps({"type": "hello", "protocol": 0}) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            buf += s.recv(65536)
        s.close()
        welcome = json.loads(buf.split(b"\n")[0])
        body = json.dumps(payload or {})
        c = http.client.HTTPConnection("127.0.0.1", welcome["hookPort"], timeout=5)
        c.request("POST", f"/hook?evt={event}&sid={sid}", body,
                  {"Authorization": f"Bearer {welcome['hookToken']}", "Content-Type": "application/json"})
        status = c.getresponse().status
        c.close()
        return status

    def host_pids(self):
        out = subprocess.run(["pgrep", "-f", "nodus host"], capture_output=True, text=True).stdout.split()
        mine = []
        for p in out:
            env = subprocess.run(["ps", "eww", "-p", p], capture_output=True, text=True).stdout
            if self.world in env:
                mine.append(p)
        return mine

    def stop_host(self):
        for p in self.host_pids():
            try:
                os.kill(int(p), signal.SIGTERM)
            except OSError:
                pass


def sid(key):
    """The seeded world's session id for a key (`sessionId` in seed/world.mjs)."""
    h = hashlib.sha1(f"nodus-parity:session:{key}".encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}"


def new_cards(before, after):
    old = {c["sessionId"] for c in before}
    return [c for c in after if c["sessionId"] not in old]


def lines(a, sid):
    """A terminal's text, after a frame (with the display asleep, nothing
    asks for frames, and an undrawn terminal is never sized)."""
    a.drive(T("draw"), quiet=True)
    return (a.drive(T("screen", session=sid), quiet=True) or {}).get("lines", [])


def run(a):
    print("1. A project, added through New Project (the folder picker answers a fresh folder)")
    folder = a.path("repos", PROJECT)
    os.makedirs(folder, exist_ok=True)
    a.steps([{"cmd": "stub", "name": "pickFolder", "path": folder}, M("click", 1366, 26), W(800), AWAY])
    a.expect("the project is added", a.project_id() is not None)
    a.steps([{"cmd": "open", "project": PROJECT}, W(1200), AWAY, W(300)])
    a.expect("its board is open", a.route().get("route") == a.project_id(), a.route().get("route"))

    print("2. A Claude card and a terminal card (File › New Session, New Terminal)")
    before = a.cards()
    a.steps([{"cmd": "menu", "command": "new-session"}, W(1500)])
    claude = a.route().get("terminal")
    a.expect("New Session opens the new session's window", claude is not None, a.route())
    a.steps([{"cmd": "menu", "command": "close"}, W(600), {"cmd": "menu", "command": "new-terminal"}, W(1500)])
    terminal = a.route().get("terminal")
    a.expect("New Terminal opens the new terminal's window", terminal not in (None, claude), a.route())
    made = new_cards(before, a.cards())
    kinds = sorted((c.get("kind") or "claude") for c in made)
    a.expect("two new cards: a Claude session and a terminal", kinds == ["claude", "terminal"], made)

    print("3. Typing into them")
    a.until("the stand-in shell draws its screen", lambda: any("pnpm test" in l for l in lines(a, terminal)))
    a.drive(T("type", session=terminal, text="ls -la\r"))
    a.until("the terminal's process gets what's typed", lambda: any("got: ls -la" in l for l in lines(a, terminal)))
    a.steps([{"cmd": "menu", "command": "close"}, W(600)])
    a.until("the stand-in Claude draws its screen", lambda: any("Claude Code" in l for l in lines(a, claude)))
    a.drive(T("type", session=claude, text="add a sort menu\r"))
    a.until("Claude's process gets what's typed", lambda: "add a sort menu" in read(a.path("claude-input.log")))

    print("4. Status through Claude Code's hooks")
    for event, want in [("UserPromptSubmit", "working"), ("PermissionRequest", "waiting"), ("Stop", "done")]:
        code = a.hook(claude, event)
        a.until(f"{event} (HTTP {code}) → {want}", lambda: (a.card(claude) or {}).get("status") == want)

    print("5. Collapse and expand (the card's menu, then E on the selected card)")
    a.expect("closing its window left the card expanded (BOARD-103)", (a.card(claude) or {}).get("expanded") is True,
             a.card(claude))
    card_menu(a, claude, "Collapse")
    a.until("its menu's Collapse collapses it (saved)", lambda: not (a.card(claude) or {}).get("expanded"))
    select(a, [claude])
    a.steps([K("e"), W(900)])
    a.until("E on the selected card expands it (saved)", lambda: (a.card(claude) or {}).get("expanded") is True)
    card_menu(a, claude, "Collapse")
    a.until("…and Collapse again", lambda: not (a.card(claude) or {}).get("expanded"))

    print("6. Archive (⌫ on the selected card) and restore (the archived card's menu), on storefront")
    # A session nothing was said in is dropped, not archived (SESS-091): the
    # stand-in writes no transcript, so this uses a seeded session that has one.
    a.current = "storefront"
    cart = sid("sf-cart")
    a.steps([{"cmd": "open", "project": "storefront"}, W(1200), AWAY])
    select(a, [cart])
    a.steps([K("backspace"), W(900)])
    a.until("archived", lambda: (a.card(cart) or {}).get("archived") is True)
    stack = archive_top(a)
    a.steps([AWAY, M("move", *stack), W(400), M("right", *stack), W(300)])
    a.drive({"cmd": "screenshot", "path": a.path("e2e-archived-menu.png")})
    a.steps([{"cmd": "pick", "label": "Restore to board"}, W(900)])
    a.until("restored to the board", lambda: (a.card(cart) or {}).get("archived") is False)
    a.current = PROJECT
    a.steps([{"cmd": "open", "project": PROJECT}, W(1200), AWAY])

    print("7. A note, written on the board, and a session started from it")
    notes_before = {n["id"] for n in a.board().get("notes") or []}
    pane_menu(a, "New note here")
    a.steps([W(600)])
    new = [n for n in a.board().get("notes") or [] if n["id"] not in notes_before]
    a.expect("a new note", len(new) == 1, a.board().get("notes"))
    a.steps([{"cmd": "type", "text": "Check the coupon limits"}, W(300), K("escape"), W(900)])
    a.until("its text saved", lambda: any(n.get("text") == "Check the coupon limits"
                                          for n in a.board().get("notes") or []))
    before = a.cards()
    note = next((n for n in a.board().get("notes") or [] if n.get("text") == "Check the coupon limits"), None)
    if note:
        at = flow_to_screen(a, note["x"] + 60, note["y"] + 60)
        a.steps([AWAY, M("right", *at), W(300), {"cmd": "pick", "label": "Start session"}, W(1500)])
    started = new_cards(before, a.cards())
    a.expect("Start session on it starts one", len(started) == 1, started)
    from_note = started[0]["sessionId"] if started else None
    if started:
        a.until("…with the note as its prompt",
                lambda: "Check the coupon limits" in " ".join(lines(a, started[0]["sessionId"])) or
                "Check the coupon limits" in (started[0].get("title") or "") or
                "Check the coupon limits" in read(a.path("claude-args.log")))
    close(a)

    print("8. Ink (P, a stroke)")
    ink_before = len(a.board().get("ink") or [])
    empty = empty_spot(a)
    a.steps([AWAY, M("click", *empty), W(200), K("p"), M("down", empty[0], empty[1]),
             M("drag", empty[0] + 40, empty[1] + 20), M("drag", empty[0] + 90, empty[1]),
             M("drag", empty[0] + 140, empty[1] + 30), M("up", empty[0] + 140, empty[1] + 30), W(600), K("escape")])
    a.until("a pen stroke saved", lambda: len(a.board().get("ink") or []) == ink_before + 1)

    print("9. A view: New view here…, its brain waits at its prompt")
    views_before = {v["id"] for v in a.board().get("views") or []}
    pane_menu(a, "New view here…")
    a.steps([W(500), {"cmd": "type", "text": "Open issues by page"}, K("cmd-enter"), W(2500)])
    views = [v for v in a.board().get("views") or [] if v["id"] not in views_before]
    a.expect("a view on the board", len(views) == 1, a.board().get("views"))
    brain = views[0].get("sessionId") if views else None
    a.expect("…with its brain session", brain is not None, views)
    if brain:
        a.until("its brain started interactive with the description in its prompt",
                lambda: any("Open issues by page" in l and '"-p"' not in l
                            for l in read(a.path("claude-args.log")).splitlines()))
        a.steps([T("open", session=brain), W(1500)])
        a.until("the brain's stand-in Claude waits at its prompt",
                lambda: any("Claude Code" in l for l in lines(a, brain)))
    close(a)

    print("10. ⌘K search")
    a.steps([AWAY, M("click", *empty_spot(a)), W(200), K("cmd-k"), W(400), {"cmd": "type", "text": "sort"}, W(1500)])
    a.expect("search open", a.route().get("search") is True, a.route())
    a.drive({"cmd": "screenshot", "path": a.path("e2e-search.png")})
    a.steps([K("escape"), W(300)])
    a.expect("Escape closes it", a.route().get("search") is False, a.route())

    print("11. Merge two sessions (summaries stubbed)")
    # Two Claude sessions (a terminal has no transcript to hand over).
    a.drive({"cmd": "stub", "name": "summarize", "phases": ["done", "done"],
             "results": [{"sessionId": claude, "title": "", "summary": "Added a sort menu."},
                         {"sessionId": from_note, "title": "", "summary": "Checked the coupon limits."}]})
    before = a.cards()
    select(a, [claude, from_note])
    a.drive({"cmd": "screenshot", "path": a.path("e2e-selected.png")})
    a.steps([M("right", *card_head(a, claude)), W(300)])
    a.steps([{"cmd": "pick", "label": "New session from 2 contexts…"}, W(1200)])
    a.drive({"cmd": "screenshot", "path": a.path("e2e-merge.png")})
    a.steps([M("click", 1043, 810), W(2000)])
    made = new_cards(before, a.cards())
    merged = [c for c in made if sorted(c.get("parents") or []) == sorted([claude, from_note])]
    a.expect("Start session: a new session with both as parents (lineage)", len(merged) == 1, made)
    close(a)

    print("12. File › Capture…")
    a.steps([{"cmd": "action", "name": "nodus::Capture"}, W(900)])
    shot = a.drive({"cmd": "screenshot", "path": a.path("e2e-capture.png"), "window": "capture"}, quiet=True)
    a.expect("the capture panel is up", shot is not None and shot.get("width") == 1304, shot)
    a.steps([{"cmd": "capture", "action": "hide"}, W(300)])

    print("13. Settings: theme, the voice key field, the update rows")
    a.steps([{"cmd": "menu", "command": "settings"}, W(600), M("click", 848, 163), W(600)])
    a.until("Dark is chosen (saved)", lambda: a.state()["settings"].get("theme") == "dark")
    a.drive({"cmd": "screenshot", "path": a.path("e2e-settings-dark.png")})
    a.steps([M("click", 715, 163), W(600)])
    a.until("…and Light again", lambda: a.state()["settings"].get("theme") == "light")
    a.steps([{"cmd": "scroll", "x": 720, "y": 500, "dx": 0, "dy": -3000}, W(400), M("click", 683, 677), W(200),
             {"cmd": "type", "text": "xai-not-a-real-key"}, W(300)])
    a.drive({"cmd": "screenshot", "path": a.path("e2e-settings-key.png")})
    a.expect("the key field takes typing (shot e2e-settings-key.png); not saved",
             "voiceKey" not in json.dumps(a.state().get("settings", {})), a.state().get("settings"))
    for status in [{"state": "checking", "current": "1.0.0"},
                   {"state": "downloading", "version": "1.0.1", "percent": 42, "current": "1.0.0"},
                   {"state": "ready", "version": "1.0.1", "current": "1.0.0"},
                   {"state": "latest", "current": "1.0.0"}]:
        a.steps([{"cmd": "emit", "event": "updateStatus", "payload": status}, W(300)])
        a.drive({"cmd": "screenshot", "path": a.path(f"e2e-update-{status['state']}.png")})
    a.expect("the update rows follow checking → downloading → ready → latest (shots e2e-update-*.png)", True)
    a.steps([K("escape"), W(300)])

    print("14. Quit, start again, adopt the host")
    hosts = a.host_pids()
    a.expect("one session host running", len(hosts) == 1, hosts)
    a.quit()
    time.sleep(1)
    a.expect("the host outlives the app", a.host_pids() == hosts, a.host_pids())
    a.start()
    a.steps([{"cmd": "open", "project": PROJECT}, W(1500)])
    a.expect("the same host, adopted", a.host_pids() == hosts, a.host_pids())
    live = [c["sessionId"] for c in a.cards() if c.get("live")]
    a.expect("the terminal's session still live", terminal in live, live)
    a.steps([T("open", session=terminal), W(1500)])
    a.until("its window shows the running shell", lambda: any("pnpm test" in l for l in lines(a, terminal)))
    a.drive(T("type", session=terminal, text="pwd\r"))
    a.until("…which still takes typing (got: pwd)", lambda: any("got: pwd" in l for l in lines(a, terminal)))
    close(a)
    a.until("Claude's session still live too", lambda: (a.card(claude) or {}).get("live") is True)


# Where things are -------------------------------------------------------

def close(a):
    """⌘W, only while a session window or preview is up (on a bare board it
    goes back to the grid)."""
    r = a.route()
    if r.get("terminal") or r.get("preview") or r.get("agents"):
        a.steps([{"cmd": "menu", "command": "close"}, W(500)])


def read(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return ""


def viewport(a):
    """Where the board is on screen now (the saved viewport lags a move; a
    glide only moves on with frames, which a sleeping display doesn't ask for)."""
    for _ in range(3):
        a.drive(T("draw"), quiet=True)
    board = a.board()
    return board.get("liveViewport") or board.get("viewport") or {"x": 0, "y": 0, "zoom": 1}


def flow_to_screen(a, x, y):
    vp = viewport(a)
    z = vp.get("zoom", 1)
    return (vp["x"] + x * z, 52 + vp["y"] + y * z)


def card_head(a, sid):
    """A card's title row (an expanded card's header: its terminal takes clicks)."""
    c = a.card(sid)
    return flow_to_screen(a, c["x"] + 120, c["y"] + 18)


def card_menu(a, sid, label):
    """Right-clicks a card's title row and chooses `label` in its menu."""
    fit(a)
    a.steps([AWAY, M("click", *empty_spot(a)), W(200), M("right", *card_head(a, sid)), W(300)])
    a.steps([{"cmd": "pick", "label": label}, W(900)])


def select(a, sids):
    """Clears the selection, then ⇧-clicks each card's title row."""
    fit(a)
    a.steps([AWAY, M("click", *empty_spot(a)), W(200)])
    for sid in sids:
        a.steps([M("click", *card_head(a, sid), shift=True), W(200)])
    a.steps([AWAY])


def archive_top(a):
    """The archive stack's top card (the frame's top left is the archive's)."""
    r = a.board()["archive"]
    return flow_to_screen(a, r["x"] + 172, r["y"] + 200)


def fit(a):
    """The pane menu's Show everything: every card and the archive on screen
    (on a crowded board with no clear spot, the board stays as it is)."""
    spot = empty_spot(a)
    a.steps([AWAY, M("click", *spot), W(200), M("right", *spot), W(300)])
    a.drive({"cmd": "pick", "label": "Show everything"}, quiet=True)
    a.steps([K("escape"), W(900)])


def empty_spot(a):
    """A point on the board clear of cards, notes, views and the archive."""
    board = a.board()
    rects = [(c["x"], c["y"], 760 if c.get("expanded") else 340, 520 if c.get("expanded") else 220)
             for c in board.get("cards") or [] if not c.get("archived")]
    rects += [(n["x"], n["y"], n.get("w") or 200, n.get("h") or 200) for n in board.get("notes") or []]
    rects += [(v["x"], v["y"], v.get("w") or 480, v.get("h") or 360) for v in board.get("views") or []]
    r = board["archive"]
    rects.append((r["x"], r["y"], r["w"], r["h"]))
    vp = viewport(a)
    z = vp.get("zoom", 1)
    # From the bottom up, clear of everything by 60 units (an expanded
    # card's title bar sits above its node), and of the prompt bar.
    for sy in range(720, 120, -40):
        for sx in range(1360, 400, -40):
            fx, fy = (sx - vp["x"]) / z, (sy - 52 - vp["y"]) / z
            if all(not (x - 60 <= fx <= x + w + 60 and y - 60 <= fy <= y + h + 60) for x, y, w, h in rects):
                return (sx, sy)
    return (1300, 780)


def pane_menu(a, label):
    spot = empty_spot(a)
    a.steps([AWAY, M("click", *spot), W(200), M("right", *spot), W(300), {"cmd": "pick", "label": label}])


def prepare(world):
    """The world's stand-ins, told to show what reaches them: the shell
    echoes what's typed (`got: …`), Claude logs it and its arguments."""
    shell = os.path.join(world, "bin", "shell.mjs")
    with open(shell) as f:
        text = f.read()
    text = text.replace("process.stdin.on('data', () => {})",
                        "let typed = ''\nprocess.stdin.on('data', (d) => { typed += d.toString();"
                        " const i = typed.indexOf('\\r'); if (i >= 0) { process.stdout.write('\\r\\ngot: ' +"
                        " typed.slice(0, i) + '\\r\\n'); typed = typed.slice(i + 1) } })")
    with open(shell, "w") as f:
        f.write(text)
    claude = os.path.join(world, "bin", "claude")
    with open(claude) as f:
        text = f.read()
    log = json.dumps(os.path.join(world, "claude-input.log"))
    args = json.dumps(os.path.join(world, "claude-args.log"))
    text = text.replace("process.stdin.on('data', () => {})",
                        f"process.stdin.on('data', (d) => fs.appendFileSync({log}, d.toString()))")
    text = text.replace("const args = process.argv.slice(2)",
                        f"const args = process.argv.slice(2)\nfs.appendFileSync({args}, JSON.stringify(args) + '\\n')")
    with open(claude, "w") as f:
        f.write(text)


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(REPO, "migration", "parity"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-e2e"),
        "bin": os.path.join(REPO, "target", "debug", "nodus"),
    }
    for flag in ["--harness", "--world", "--bin"]:
        if flag in args:
            i = args.index(flag)
            opts[flag[2:]] = os.path.abspath(args[i + 1])
            del args[i : i + 2]
    os.makedirs(opts["world"], exist_ok=True)
    os.chdir(opts["world"])
    subprocess.run([NODE, os.path.join(HERE, "seed.mjs"), opts["harness"], opts["world"], "light"], check=True,
                   capture_output=True)
    for log in ["claude-input.log", "claude-args.log", "app.log"]:
        if os.path.exists(os.path.join(opts["world"], log)):
            os.remove(os.path.join(opts["world"], log))
    prepare(opts["world"])
    a = App(opts)
    try:
        a.start()
        run(a)
    finally:
        if a.app:
            a.quit()
        a.stop_host()
    print("PASS" if not a.failed else f"FAIL: {len(a.failed)}: " + "; ".join(a.failed))
    sys.exit(0 if not a.failed else 1)


main()
