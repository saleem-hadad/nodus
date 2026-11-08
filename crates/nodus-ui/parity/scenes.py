#!/usr/bin/env python3
"""Renders the shell's parity scenes with the Rust app, beside the goldens.

usage: scenes.py [scene-prefix ...] [--theme light|dark]
                 [--harness <migration/parity of the parity-harness track>]
                 [--golden <dir>] [--world <dir>] [--bin <nodus binary>]

For each scene and theme it seeds the parity harness's synthetic world,
starts `nodus` on it with NODUS_AUTOMATION, drives it into the scene through
the test driver (crates/nodus-ui/src/automation.rs) and saves the shot as
<golden>/<scene>.<theme>.rust.png, 1440x900 at 2x, in Display P3 like the
goldens. Compare with compare.mjs. Never touches the real userData: the
world has its own HOME and NODUS_USER_DATA.
"""
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
SOCK = "nodus-parity.sock"  # relative: macOS caps socket paths at 104 bytes


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


AWAY = M("move", 720, 26)  # the title bar's empty middle
STOREFRONT = (104, 220)
OPEN_STOREFRONT = [M("click", *STOREFRONT), {"cmd": "key", "keystroke": "cmd-o"}, {"cmd": "wait", "ms": 300}]

# scene: (seed mutations, steps)
SCENES = {
    "projects-empty": (["empty"], []),
    "projects-grid": ([], []),
    "projects-focus": (["focusOn"], []),
    "projects-focus-none": (["focusOn", "focusNone"], []),
    "projects-selected": ([], [M("click", *STOREFRONT), AWAY]),
    "projects-rename": ([], [M("click", *STOREFRONT), AWAY, {"cmd": "key", "keystroke": "enter"}]),
    "projects-context-menu": ([], [M("right", *STOREFRONT)]),
    "projects-context-menu-focus": (["focusFeature"], [M("right", 412, 220)]),
    "projects-group-menu": ([], [M("right", 140, 353)]),
    "projects-group-picker": ([], [M("right", 104, 460), {"cmd": "wait", "ms": 200}, M("click", 160, 574)]),
    "projects-remove-confirm": ([], [M("right", *STOREFRONT), {"cmd": "wait", "ms": 200}, M("click", 160, 421)]),
    "projects-drop": ([], [{"cmd": "files", "kind": "entered", "x": 720, "y": 450, "paths": ["/tmp"]},
                           {"cmd": "files", "kind": "pending", "x": 722, "y": 452}]),
    "titlebar-update": ([], [{"cmd": "emit", "event": "updateStatus",
                              "payload": {"state": "ready", "version": "0.9.5", "current": "0.9.4"}}]),
    "settings": (["capture"], [{"cmd": "menu", "command": "settings"}]),
    "settings-bottom": (["capture"], [
        {"cmd": "emit", "event": "updateStatus", "payload": {"state": "latest", "current": "0.9.4"}},
        {"cmd": "menu", "command": "settings"}, {"cmd": "wait", "ms": 300},
        {"cmd": "scroll", "x": 720, "y": 500, "dx": 0, "dy": -3000}]),
    "rail-open": ([], OPEN_STOREFRONT + [M("move", 2, 450), {"cmd": "wait", "ms": 200}, M("move", 3, 450),
                                          {"cmd": "wait", "ms": 400}, M("move", 34, 308)]),
    "rail-pinned": (["pinRail"], OPEN_STOREFRONT + [AWAY]),
    "rail-numbers": (["pinRail"], OPEN_STOREFRONT + [AWAY, {"cmd": "modifiers", "meta": True},
                                                      {"cmd": "wait", "ms": 600}]),
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
               SHELL=os.path.join(world, "bin", "fakeshell"), TZ="UTC", LANG="en_US.UTF-8")
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
            drive(f, {"cmd": "wait", "ms": 120})
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


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(REPO, "migration", "parity"),
        "golden": os.path.join(REPO, "migration", "parity", "golden"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-ui-parity"),
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
