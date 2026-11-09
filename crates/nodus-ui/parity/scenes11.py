#!/usr/bin/env python3
"""Renders spec 11's parity scenes (prompt bar, ⌘K search, merge dialog, ink)
with the Rust app, beside the goldens.

usage: scenes11.py [scene-prefix ...] [--theme light|dark]
                   [--harness <migration/parity>] [--golden <dir>]
                   [--world <dir>] [--bin <nodus binary>]

As scenes.py does for the shell and the board: seeds the parity harness's
synthetic world, starts `nodus` on it with NODUS_AUTOMATION, drives it into
the scene (the steps of migration/parity/scenes/overlays.mjs) and saves
<golden>/<scene>.<theme>.rust.png. Never touches the real userData and never
runs the real claude (the world's stand-in answers its -p calls).
"""
import hashlib
import json
import os
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
NOW = 1792508400000  # 2026-10-20 15:00 UTC, the harness's frozen clock
SOCK = "nodus-parity11.sock"  # relative: macOS caps socket paths at 104 bytes


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


def W(ms):
    return {"cmd": "wait", "ms": ms}


def K(keystroke):
    return {"cmd": "key", "keystroke": keystroke}


def typed(text):
    keys = {" ": "space"}
    out = []
    for c in text:
        if c in keys:
            out.append(K(keys[c]))
        elif c.isupper():
            out.append(K("shift-" + c.lower()))
        else:
            out.append(K(c))
    return out


def AT(x, y):
    """storefront opens at x 60, y 96, zoom 0.8 under the 52px title bar: flow -> window."""
    return (60 + x * 0.8, 52 + 96 + y * 0.8)


def sid(key):
    h = hashlib.sha1(f"nodus-parity:session:{key}".encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}"


def STATUS(key, status, live=True):
    return {"cmd": "emit", "event": "status", "payload": {"sessionId": sid(key), "status": status, "live": live}}


def stroke(points, steps=6):
    """Press at the first point, move to each next one in `steps` steps, release."""
    out = [M("down", *points[0])]
    for a, b in zip(points, points[1:]):
        for i in range(1, steps + 1):
            t = i / steps
            out.append(M("drag", a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    out.append(M("up", *points[-1]))
    return out


AWAY = M("move", 720, 26)
STOREFRONT = [{"cmd": "open", "project": "storefront"}, W(1000),
              STATUS("sf-skeleton", "working"), STATUS("sf-tax", "waiting"),
              STATUS("sf-cart", "done"), STATUS("sf-terminal", "done")]

# The ink dock (16px in, its bottom 58px above the board's): tool centres.
DOCK_X = 33
POINTER, PEN, ARROW, TEXT, ERASER, COLOUR, CLEAR = 636, 666, 696, 726, 756, 795, 825
PURPLE = (206, COLOUR)

PEN_A = AT(1080, 650)
FIELD = (600, 856)  # the prompt bar's field, closed
SF_CART = AT(1216, 148)
SF_COUPONS = (495, 560)
SF_SEARCH_PLAN = (776, 560)
ARROW_A = AT(1300, 700)

SCENES = {
    "ink-pen": ([], STOREFRONT + [AWAY, M("click", DOCK_X, PEN), M("click", DOCK_X, COLOUR), W(200),
                                  M("click", *PURPLE)]
                + stroke([PEN_A, (PEN_A[0] + 60, PEN_A[1] + 30), (PEN_A[0] + 140, PEN_A[1] - 10),
                          (PEN_A[0] + 220, PEN_A[1] + 40)]) + [AWAY]),
    "ink-palette": ([], STOREFRONT + [AWAY, M("click", DOCK_X, COLOUR), W(300), AWAY]),
    "ink-arrow": ([], STOREFRONT + [AWAY, M("click", DOCK_X, ARROW)] + stroke([ARROW_A, AT(1230, 610)]) + [AWAY]),
    "ink-text": ([], STOREFRONT + [AWAY, M("click", DOCK_X, TEXT), M("click", *AT(1100, 660)), W(200), {"cmd": "idle"}, W(100)]
                 + typed("needs tests") + [AWAY]),
    "ink-eraser": ([], STOREFRONT + [AWAY, M("click", DOCK_X, ERASER), M("down", *AT(430, 700))]
                   + [M("drag", AT(430, 700)[0] + (AT(520, 700)[0] - AT(430, 700)[0]) * i / 8, AT(430, 700)[1])
                      for i in range(1, 9)]),
    "ink-selected": ([], STOREFRONT + [AWAY, M("click", *AT(470, 652)), AWAY]),
    "board-populated": ([], STOREFRONT + [AWAY]),
    "bar-focused": ([], STOREFRONT + [AWAY, M("click", *FIELD)]),
    "bar-typing": ([], STOREFRONT + [AWAY, M("click", *FIELD)] + typed("Add a sort menu to the search results:")
                   + [K("shift-enter")] + typed("- price, low to high and high to low") + [K("shift-enter")]
                   + typed("- newest first")),
    "bar-slash-menu": ([], STOREFRONT + [AWAY, W(300), K("/")]),
    "bar-view": ([], STOREFRONT + [AWAY, M("click", *FIELD)] + typed("/view ") + typed("failing tests by file")),
    "bar-note": ([], STOREFRONT + [AWAY, M("click", *FIELD)] + typed("/note ")
                 + typed("Check the coupon edge cases with Sam")),
    "bar-one-selected": ([], STOREFRONT + [AWAY, M("click", *SF_CART, shift=True), M("click", *FIELD)]),
    "bar-several-selected": ([], STOREFRONT + [AWAY, M("click", *SF_COUPONS, shift=True),
                                               M("click", *SF_SEARCH_PLAN, shift=True), M("click", *FIELD)]
                             + typed("What do these two have in common?")),
    "board-selected": ([], STOREFRONT + [M("click", 1030, 300, shift=True), M("click", 495, 560, shift=True), AWAY]),
}


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd, answer)
    return answer


def run(scene, theme, opts):
    mutations, steps = SCENES[scene]
    subprocess.run(["node", os.path.join(HERE, "seed.mjs"), opts["harness"], opts["world"], theme, *mutations],
                   check=True, capture_output=True)
    world = opts["world"]
    env = dict(os.environ, NODUS_USER_DATA=os.path.join(world, "userData"), NODUS_AUTOMATION=SOCK,
               HOME=os.path.join(world, "home"), PATH=os.path.join(world, "bin") + ":/usr/bin:/bin:/usr/sbin",
               SHELL=os.path.join(world, "bin", "fakeshell"), TZ="UTC", LANG="en_US.UTF-8",
               PARITY_RESPONSES=os.path.join(world, "responses.json"))
    if os.path.exists(SOCK):
        os.remove(SOCK)
    log = open(os.path.join(world, "app.log"), "w")
    app = subprocess.Popen([opts["bin"]], env=env, stdout=log, stderr=log)
    try:
        for _ in range(400):
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(SOCK)
                break
            except OSError:
                time.sleep(0.05)
        f = s.makefile("rw")
        drive(f, {"cmd": "resize", "width": 1440, "height": 900})
        drive(f, {"cmd": "parity", "frozen": True, "now": NOW})
        drive(f, AWAY)
        drive(f, {"cmd": "idle"})
        for step in steps:
            drive(f, step)
            drive(f, W(60))
        drive(f, {"cmd": "idle"})
        out = os.path.join(opts["golden"], f"{scene}.{theme}.rust.png")
        drive(f, {"cmd": "screenshot", "path": out})
        print("shot", out)
        try:
            f.write(json.dumps({"cmd": "quit"}) + "\n")
            f.flush()
        except OSError:
            pass
    finally:
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            app.kill()
        subprocess.run(["pkill", "-f", os.path.join(world, "userData", "host")], check=False)


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(REPO, "migration", "parity"),
        "golden": os.path.join(REPO, "migration", "parity", "golden"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-ui-parity11"),
        "bin": os.path.join(REPO, "target", "debug", "nodus"),
    }
    themes = ["light", "dark"]
    for flag in ["--harness", "--golden", "--world", "--bin", "--theme"]:
        if flag in args:
            i = args.index(flag)
            value = os.path.abspath(args[i + 1]) if flag != "--theme" else args[i + 1]
            del args[i : i + 2]
            if flag == "--theme":
                themes = [value]
            else:
                opts[flag[2:]] = value
    os.makedirs(opts["world"], exist_ok=True)
    os.chdir(opts["world"])
    names = [n for n in SCENES if not args or any(n.startswith(a) for a in args)]
    for name in names:
        for theme in themes:
            run(name, theme, opts)


main()
