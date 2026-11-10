#!/usr/bin/env python3
"""Checks the behaviors that wire the board, search and the terminal's windows
together (BOARD-037, MODAL-028…MODAL-031) end to end, with the Rust app.

usage: cross_wiring.py [check-prefix ...] [--harness <dir, default harness/>]
                       [--world <dir>] [--bin <nodus binary>]

For each check it seeds the parity harness's synthetic world (its own HOME and
NODUS_USER_DATA; the world's stand-ins answer for claude, never the real one),
starts `nodus` with NODUS_AUTOMATION, drives it through the test driver and
prints PASS or FAIL with what the driver's `route` reported. The app and the
session host it starts are stopped after each check. The window never takes
focus and ignores the real pointer.
"""
import hashlib
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
SOCK = "nodus-cross-wiring.sock"  # relative: macOS caps socket paths at 104 bytes
NODE = os.environ.get("PARITY_NODE") or next(
    (p for p in ["/opt/homebrew/bin/node", "/usr/local/bin/node"] if os.path.exists(p)), "node"
)


def uuid(name):
    h = hashlib.sha1(f"nodus-parity:{name}".encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}"


def sid(key):
    return uuid(f"session:{key}")


def pid(key):
    return uuid(f"project:{key}")


def vid(key):
    return uuid(f"view:{key}")


def T(action, **kw):
    return {"cmd": "terminal", "action": action, **kw}


def W(ms):
    return {"cmd": "wait", "ms": ms}


def K(keystroke):
    return {"cmd": "key", "keystroke": keystroke}


def typed(text):
    return [K("space") if c == " " else K(c) for c in text]


AWAY = {"cmd": "mouse", "kind": "move", "x": 720, "y": 26}


def board(key):
    return [{"cmd": "open", "project": key}, W(1000), AWAY, W(300)]


class Check:
    def __init__(self, f):
        self.f = f
        self.failed = []

    def drive(self, cmd):
        self.f.write(json.dumps(cmd) + "\n")
        self.f.flush()
        answer = json.loads(self.f.readline() or "{}")
        if not answer.get("ok"):
            print("    !", cmd, answer)
            self.failed.append(f"driver: {cmd} -> {answer}")
        return answer.get("result")

    def steps(self, steps):
        for step in steps:
            self.drive(step)
            self.drive(W(60))

    def route(self):
        return self.drive(T("route")) or {}

    def expect(self, what, ok, seen):
        print(f"    {'ok  ' if ok else 'FAIL'} {what}" + ("" if ok else f"  (saw {seen})"))
        if not ok:
            self.failed.append(what)


def card(route, session):
    return next((c for c in route.get("cards") or [] if c["id"] == session), None)


def board_037(c):
    """⌘K, a query only an archived session matches, Enter: its transcript at the match."""
    c.steps(board("storefront") + [K("cmd-k"), W(300)] + typed("apple pay") + [W(1500), K("enter"), W(400)])
    r = c.route()
    c.expect("search closed", r.get("search") is False, r.get("search"))
    c.expect("the archived card's preview open", r.get("preview") == sid("sf-arch-01"), r.get("preview"))
    find = r.get("find") or {}
    c.expect("opened at the trimmed query", find.get("query") == "apple pay", find)
    snippet = (find.get("snippet") or {}).get("text", "")
    c.expect("with the best snippet", "apple pay" in snippet.lower(), snippet)
    # TRANS-002: a preview the board opens from no search (the archived
    # card's menu: Preview) records none, not the last search's.
    stack = (60 + 172 * 0.8, 52 + 96 + 200 * 0.8)  # the archive stack's top card
    c.steps([T("close"), W(300), {"cmd": "mouse", "kind": "right", "x": stack[0], "y": stack[1]}, W(300),
             {"cmd": "mouse", "kind": "click", "x": stack[0] + 40, "y": stack[1] + 18}, W(400)])
    r = c.route()
    c.expect("the menu's Preview opened a transcript", r.get("preview") is not None, r.get("preview"))
    c.expect("…with no find", r.get("find") is None, r.get("find"))


def modal_028(c):
    """The session window closes when a reload brings its card back archived."""
    project = pid("storefront")
    c.steps(board("storefront") + [T("liven", project=project, sessions=[sid("sf-cart")]),
                                   T("open", session=sid("sf-cart")), W(1200), T("draw"), W(300)])
    c.expect("window open", c.route().get("terminal") == sid("sf-cart"), c.route().get("terminal"))
    c.steps([T("save-nodes", project=project, patches=[{"sessionId": sid("sf-tax"), "archived": True}]), W(800)])
    r = c.route()
    c.expect("another card archived outside: reloaded", (card(r, sid("sf-tax")) or {}).get("archived") is True,
             card(r, sid("sf-tax")))
    c.expect("…and the window stays", r.get("terminal") == sid("sf-cart"), r.get("terminal"))
    c.steps([T("save-nodes", project=project, patches=[{"sessionId": sid("sf-cart"), "archived": True}]), W(800)])
    r = c.route()
    c.expect("its card archived outside: reloaded", (card(r, sid("sf-cart")) or {}).get("archived") is True,
             card(r, sid("sf-cart")))
    c.expect("…and the window closed", r.get("terminal") is None, r.get("terminal"))


def modal_029(c):
    """A finished run's terminal is disposed and its window closes."""
    project = pid("storefront")
    c.steps(board("storefront") + [T("liven", project=project, sessions=[sid("sf-cart")]),
                                   T("open", session=sid("sf-cart")), W(1500), T("draw"), W(300)])
    c.expect("window open on the run", c.route().get("terminal") == sid("sf-cart"), c.route().get("terminal"))
    c.expect("its terminal exists", c.drive(T("screen", session=sid("sf-cart"))) is not None, None)
    run = lambda s: {"projectId": project, "sessionId": s, "actionName": "Ship it"}  # noqa: E731
    c.steps([T("run-finished", run=run(sid("sf-tax"))), W(400)])
    c.expect("another run finished: the window stays", c.route().get("terminal") == sid("sf-cart"),
             c.route().get("terminal"))
    finished = c.drive(T("run-finished", run=run(sid("sf-cart")))) or {}
    c.drive(W(400))
    c.expect("its run finished: the window closed", c.route().get("terminal") is None, c.route().get("terminal"))
    # Disposed as the window closed. Its card, still on this board (0.9.4's
    # core says the run finished before the board changed), then expands and
    # attaches a new terminal (BOARD-103, XCARD-021), as in 0.9.4.
    c.expect("…and its terminal disposed", finished.get("disposed") is True, finished)


def modal_030(c):
    """Removing a view closes its brain's window and disposes the brain's terminal."""
    brain = sid("ds-brain-build")
    c.steps(board("docs-site") + [T("open", session=brain), W(1500), T("draw"), W(300)])
    c.expect("the brain's window open", c.route().get("terminal") == brain, c.route().get("terminal"))
    c.expect("its terminal exists", c.drive(T("screen", session=brain)) is not None, None)
    c.steps([T("remove-view", view=vid("ds-search-size")), W(300)])
    c.expect("another view removed: the window stays", c.route().get("terminal") == brain, c.route().get("terminal"))
    c.steps([T("remove-view", view=vid("ds-build")), W(300)])
    r = c.route()
    c.expect("the view gone", all(v["id"] != vid("ds-build") for v in r.get("views") or []), r.get("views"))
    c.expect("…its brain's window closed", r.get("terminal") is None, r.get("terminal"))
    c.expect("…and its card off the board", card(r, brain) is None, card(r, brain))
    screen = c.drive(T("screen", session=brain))
    c.expect("…and its terminal disposed", screen is None, screen)


def modal_031(c):
    """With the session window up, no board shortcut or ⌘K; the menu's Search closes it."""
    project = pid("storefront")
    c.steps(board("storefront") + [T("liven", project=project, sessions=[sid("sf-cart")]),
                                   T("open", session=sid("sf-cart")), W(1500), T("draw"), W(300)])
    before = len(c.route().get("cards") or [])
    c.steps([K("cmd-k"), W(300), K("n"), K("t"), W(800)])
    r = c.route()
    c.expect("⌘K opens no search", r.get("search") is False, r.get("search"))
    c.expect("N and T start nothing", len(r.get("cards") or []) == before, len(r.get("cards") or []))
    c.expect("the window stays", r.get("terminal") == sid("sf-cart"), r.get("terminal"))
    c.steps([{"cmd": "menu", "command": "search"}, W(300)])
    r = c.route()
    c.expect("menu Search: the window closed", r.get("terminal") is None, r.get("terminal"))
    c.expect("…and search open", r.get("search") is True, r.get("search"))
    c.steps([K("escape"), W(200), T("preview", session=sid("sf-arch-01")), W(400),
             {"cmd": "menu", "command": "search"}, W(300)])
    r = c.route()
    c.expect("menu Search over the preview: it closed", r.get("preview") is None, r.get("preview"))
    c.expect("…and search open", r.get("search") is True, r.get("search"))


CHECKS = {
    "BOARD-037": board_037,
    "MODAL-028": modal_028,
    "MODAL-029": modal_029,
    "MODAL-030": modal_030,
    "MODAL-031": modal_031,
}


def stop_host(world):
    """Stops the session host the app started (it outlives the app)."""
    out = subprocess.run(["pgrep", "-f", "nodus host"], capture_output=True, text=True).stdout.split()
    for p in out:
        try:
            env = subprocess.run(["ps", "eww", "-p", p], capture_output=True, text=True).stdout
            if world in env:
                os.kill(int(p), signal.SIGTERM)
        except (OSError, ValueError):
            pass


def run(name, opts):
    world = opts["world"]
    check = CHECKS[name]
    subprocess.run([NODE, os.path.join(HERE, "seed.mjs"), opts["harness"], world, "light"], check=True,
                   capture_output=True)
    env = dict(os.environ, NODUS_USER_DATA=os.path.join(world, "userData"), NODUS_AUTOMATION=SOCK,
               HOME=os.path.join(world, "home"), PATH=os.path.join(world, "bin") + ":/usr/bin:/bin:/usr/sbin",
               SHELL=os.path.join(world, "bin", "fakeshell"), TZ="UTC", LANG="en_US.UTF-8",
               PARITY_RESPONSES=os.path.join(world, "responses.json"))
    if os.path.exists(SOCK):
        os.remove(SOCK)
    log = open(os.path.join(world, "app.log"), "w")
    app = subprocess.Popen([opts["bin"]], env=env, stdout=log, stderr=log)
    print(name, check.__doc__)
    failed = ["no driver"]
    try:
        for _ in range(400):
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(SOCK)
                break
            except OSError:
                time.sleep(0.05)
        c = Check(s.makefile("rw"))
        c.drive({"cmd": "resize", "width": 1440, "height": 900})
        c.drive({"cmd": "parity", "frozen": True, "now": NOW})
        c.drive(AWAY)
        c.drive({"cmd": "idle"})
        check(c)
        failed = c.failed
        try:
            c.f.write(json.dumps({"cmd": "quit"}) + "\n")
            c.f.flush()
        except OSError:
            pass
    finally:
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            app.kill()
        stop_host(world)
    print(f"  {'PASS' if not failed else 'FAIL'} {name}")
    return not failed


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(HERE, "harness"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-cross-wiring"),
        "bin": os.path.join(REPO, "target", "debug", "nodus"),
    }
    for flag in ["--harness", "--world", "--bin"]:
        if flag in args:
            i = args.index(flag)
            opts[flag[2:]] = os.path.abspath(args[i + 1])
            del args[i : i + 2]
    os.makedirs(opts["world"], exist_ok=True)
    os.chdir(opts["world"])
    names = [n for n in CHECKS if not args or any(n.startswith(a) for a in args)]
    results = [run(n, opts) for n in names]
    sys.exit(0 if all(results) else 1)


main()
