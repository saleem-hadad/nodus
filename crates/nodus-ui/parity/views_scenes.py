#!/usr/bin/env python3
"""Renders the views' parity scenes (spec 04) with the Rust app, beside the goldens.

usage: views_scenes.py [scene-prefix ...] [--theme light|dark]
                       [--harness <migration/parity>] [--golden <dir>] [--world <dir>] [--bin <nodus>]

As scenes.py: seeds the harness's synthetic world, starts `nodus` on it with
NODUS_AUTOMATION, opens docs-site (three views: a live page, one being built,
one with a command waiting) and drives it into the scene. In parity mode the
pages show as snapshots GPUI draws, so the window's own capture holds them.
Shots land as <golden>/<scene>.<theme>.rust.png.
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
SOCK = "nodus-views-parity.sock"


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


AWAY = M("move", 720, 26)


def sid(key):
    h = hashlib.sha1(f"nodus-parity:session:{key}".encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}"


def STATUS(key, status, live=True):
    return {"cmd": "emit", "event": "status", "payload": {"sessionId": sid(key), "status": status, "live": live}}


# docs-site opens at x 40, y 70, zoom 0.62 under the 52px title bar.
def AT(x, y):
    return (40 + x * 0.62, 52 + 70 + y * 0.62)


def DOCS(brain="working"):
    return [{"cmd": "open", "project": "docs-site"}, {"cmd": "wait", "ms": 800},
            STATUS("ds-brain-links", brain), {"cmd": "wait", "ms": 1500}]


BUILD = AT(960, 180)  # the middle of Docs build status
# Its tools, right-aligned in the 38-unit title bar: ⟳ ✎ ⌨ ⤢ ⋯, 28 units each, 1 apart.
TOOL_Y = AT(0, 19)[1]
def TOOL(i):  # 0 = ⋯ (rightmost)
    right = AT(720 + 480 - 6, 0)[0]
    return (right - (14 + i * 29) * 0.62, TOOL_Y)


SCENES = {
    "views-board": DOCS() + [AWAY],
    "views-brain-waiting": DOCS("waiting") + [AWAY],
    "views-menu": DOCS() + [M("move", *BUILD), {"cmd": "wait", "ms": 200}, M("click", *TOOL(0))],
    "views-large": DOCS() + [M("move", *BUILD), {"cmd": "wait", "ms": 200}, M("click", *TOOL(1)),
                             {"cmd": "wait", "ms": 1500}],
    "views-change": DOCS() + [M("move", *BUILD), {"cmd": "wait", "ms": 200}, M("click", *TOOL(3)),
                              {"cmd": "wait", "ms": 300}, {"cmd": "type", "text": "Show the failed build’s error"},
                              {"cmd": "wait", "ms": 600}],
    # The pane menu snaps up from (1100, 720); New view here… is its fifth row.
    "views-new": DOCS() + [M("right", 1100, 720), {"cmd": "wait", "ms": 300}, M("click", 1150, 787),
                           {"cmd": "wait", "ms": 400}, {"cmd": "type", "text": "Open issues labelled docs, grouped by page"}],
}


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd, answer)
    elif cmd.get("cmd") == "views":
        print(json.dumps(answer.get("result"), indent=1))
    return answer


def run(scene, theme, opts):
    steps = SCENES[scene]
    subprocess.run(["node", os.path.join(HERE, "seed.mjs"), opts["harness"], opts["world"], theme],
                   check=True, capture_output=True)
    world = opts["world"]
    env = dict(os.environ, NODUS_USER_DATA=os.path.join(world, "userData"), NODUS_AUTOMATION=SOCK,
               HOME=os.path.join(world, "home"), PATH=os.path.join(world, "bin") + ":/usr/bin:/bin:/usr/sbin",
               SHELL=os.path.join(world, "bin", "fakeshell"), TZ="UTC", LANG="en_US.UTF-8",
               PARITY_RESPONSES=os.path.join(world, "responses.json"))
    if os.path.exists(SOCK):
        os.remove(SOCK)
    log = open(os.path.join(world, f"app-{scene}-{theme}.log"), "w")
    app = subprocess.Popen([opts["bin"]], env=env, stdout=log, stderr=log)
    try:
        s = None
        for _ in range(400):
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(SOCK)
                break
            except OSError:
                time.sleep(0.05)
        f = s.makefile("rw")
        drive(f, {"cmd": "resize", "width": 1440, "height": 900})
        if not os.environ.get("VIEWS_LIVE"):
            drive(f, {"cmd": "parity", "frozen": True, "now": NOW})
        drive(f, AWAY)
        drive(f, {"cmd": "idle"})
        for step in steps:
            drive(f, step)
            drive(f, {"cmd": "wait", "ms": 120})
        drive(f, {"cmd": "idle"})
        if os.environ.get("VIEWS_DEBUG"):
            drive(f, {"cmd": "views"})
        if os.environ.get("VIEWS_SCREENCAPTURE"):
            # The window as the screen shows it, native webviews included.
            number = drive(f, {"cmd": "window"}).get("result", {}).get("number")
            subprocess.run(["screencapture", "-x", "-o", "-l", str(number),
                            os.environ["VIEWS_SCREENCAPTURE"]], check=False)
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
        "world": os.path.join(tempfile.gettempdir(), "nodus-views-parity"),
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
