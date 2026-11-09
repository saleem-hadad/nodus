#!/usr/bin/env python3
"""Renders the terminal's parity scenes (spec 12) with the Rust app, beside the goldens.

usage: terminal_scenes.py [scene-prefix ...] [--theme light|dark]
                          [--harness <migration/parity>] [--golden <dir>] [--world <dir>] [--bin <nodus>]

As scenes.py, plus the steps the harness's `storefront()` takes: the seeded
sessions started through the core (their stand-ins, never the real claude),
statuses and usage posted to the Rust session host's hook endpoint
(terminal.mjs), and the board loaded through the test driver's `terminal`
command. Shots land as <golden>/<scene>.<theme>.rust.png. The app, and the
session host it starts, are stopped after each shot.
"""
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
SOCK = "nodus-term-parity.sock"
# The stand-ins are ES modules without .mjs: they need Node 22 or newer, which
# the seed links into the world as <work>/bin/node.
NODE = os.environ.get("PARITY_NODE") or next(
    (p for p in ["/opt/homebrew/bin/node", "/usr/local/bin/node"] if os.path.exists(p)), "node"
)
AWAY = {"cmd": "mouse", "kind": "move", "x": 720, "y": 26}


def T(action, **kw):
    return {"cmd": "terminal", "action": action, **kw}


def storefront():
    """The harness's storefront(): sessions live in every status, then the board."""
    return [
        T("board", project="@storefront"),
        T("liven", project="@storefront", sessions=["@sf-skeleton", "@sf-tax", "@sf-cart", "@sf-terminal"]),
        ("node", "status", "sf-skeleton", "working"),
        ("node", "status", "sf-skeleton", "working", "sf-a1"),
        ("node", "status", "sf-skeleton", "working", "sf-a2"),
        ("node", "status", "sf-tax", "waiting"),
        ("node", "usage", "sf-cart"),
        {"cmd": "wait", "ms": 300},
        T("reload"),
    ]


SCENES = {
    "terminal-claude": storefront() + [T("open", session="@sf-cart"), {"cmd": "wait", "ms": 1200}],
    "terminal-shell": storefront() + [T("open", session="@sf-terminal"), {"cmd": "wait", "ms": 1200}],
    "terminal-stop-confirm": storefront()
    + [T("open", session="@sf-skeleton"), {"cmd": "wait", "ms": 1200}, "click-stop"],
    "transcript-preview": storefront() + [T("preview", session="@sf-arch-01"), {"cmd": "wait", "ms": 600}],
    "agent-viewer": storefront()
    + [T("agents", session="@sf-skeleton", agent="@@sf-a3"), {"cmd": "wait", "ms": 800}],
}


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd, answer)
    return answer


def resolve(value, ids):
    if isinstance(value, str) and value.startswith("@@"):
        return ids["agents"][value[2:]]
    if isinstance(value, str) and value.startswith("@"):
        key = value[1:]
        return ids["projects"].get(key) or ids["sessions"][key]
    if isinstance(value, list):
        return [resolve(v, ids) for v in value]
    if isinstance(value, dict):
        return {k: resolve(v, ids) for k, v in value.items()}
    return value


def stop_host(world):
    """Stops the session host the app started (it outlives the app)."""
    out = subprocess.run(["pgrep", "-f", "nodus host"], capture_output=True, text=True).stdout.split()
    for pid in out:
        try:
            env = subprocess.run(["ps", "eww", "-p", pid], capture_output=True, text=True).stdout
            if world in env:
                os.kill(int(pid), signal.SIGTERM)
        except (OSError, ValueError):
            pass


def run(scene, theme, opts):
    steps = SCENES[scene]
    world = opts["world"]
    subprocess.run([NODE, os.path.join(HERE, "seed.mjs"), opts["harness"], world, theme], check=True, capture_output=True)
    node = [NODE, os.path.join(HERE, "terminal.mjs"), opts["harness"], world]
    ids = json.loads(subprocess.run(node + ["ids"], check=True, capture_output=True, text=True).stdout)
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
            if isinstance(step, tuple):
                r = subprocess.run(node + list(step[1:]), capture_output=True, text=True)
                if r.returncode != 0:
                    print("  ! node", step, r.stderr.strip())
                continue
            if step == "click-stop":
                # The Stop button: left of Archive in the header's actions.
                drive(f, {"cmd": "mouse", "kind": "click", "x": 1194, "y": 77})
                drive(f, AWAY)
                continue
            drive(f, resolve(step, ids))
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
        stop_host(world)


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(REPO, "migration", "parity"),
        "golden": os.path.join(REPO, "migration", "parity", "golden"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-term-parity"),
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
