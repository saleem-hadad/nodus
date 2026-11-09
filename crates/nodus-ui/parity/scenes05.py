#!/usr/bin/env python3
"""Renders spec 05's parity scenes (actions, git, voice) with the Rust app.

usage: scenes05.py [scene-prefix ...] [--theme light|dark]
                   [--harness <migration/parity>] [--out <dir>]
                   [--world <dir>] [--bin <nodus binary>]

As scenes.py does for the shell: for each scene and theme it seeds the parity
harness's synthetic world (with the scene's stand-in answers), starts `nodus`
on it with NODUS_AUTOMATION, drives it into the scene through the test driver
and saves <out>/<scene>.<theme>.rust.png, 1440x900 at 2x. Never touches the
real userData, never runs the real claude (the world's stand-in answers its
-p calls) and never records from the microphone.
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
SOCK = "nodus-parity05.sock"  # relative: macOS caps socket paths at 104 bytes


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


def W(ms):
    return {"cmd": "wait", "ms": ms}


def typed(text):
    keys = {" ": "space", "-": "minus", ".": "period", ",": "comma"}
    return [{"cmd": "key", "keystroke": keys.get(c, c)} for c in text]


AWAY = M("move", 720, 26)  # the title bar's empty middle
STOREFRONT = (104, 220)
OPEN_STOREFRONT = [W(600), M("click", *STOREFRONT), W(300), {"cmd": "key", "keystroke": "cmd-o"}, W(1200)]
# The actions rail's items, where 0.9.4 has them (x is the rail's middle).
RAIL_X = 1406
RUN_TESTS, OPEN_PR, PLUS = 380, 423.5, 572.5
SLIDE_OUT = [M("move", 1438, 450), W(200), M("move", 1437, 450), W(500)]
DESCRIBE = typed("Check the bundle size of each route against main")

# scene: (seed answers, steps)
SCENES = {
    "git-log": ({}, OPEN_STOREFRONT + [W(800), M("move", 380, 26), W(400), M("move", 381, 26), W(600)]),
    "git-diff": ({}, OPEN_STOREFRONT + [W(2000), M("click", 528, 26), W(1200), AWAY]),
    "actions-rail": ({}, OPEN_STOREFRONT + SLIDE_OUT + [M("move", RAIL_X, OPEN_PR), W(300)]),
    "actions-run": ({}, OPEN_STOREFRONT + SLIDE_OUT + [M("click", RAIL_X, RUN_TESTS), W(1500), M("move", 900, 600),
                                                     W(900)] + SLIDE_OUT + [M("move", RAIL_X, RUN_TESTS), W(900)]),
    "actions-new": ({}, OPEN_STOREFRONT + SLIDE_OUT + [M("click", RAIL_X, PLUS), W(400)] + DESCRIBE),
    "actions-drafting": ({"delayMs": 120000}, OPEN_STOREFRONT + SLIDE_OUT + [M("click", RAIL_X, PLUS), W(400)]
                         + DESCRIBE + [{"cmd": "key", "keystroke": "cmd-enter"}, W(400), AWAY]),
    "actions-review": ({}, OPEN_STOREFRONT + SLIDE_OUT + [M("click", RAIL_X, PLUS), W(400)] + DESCRIBE
                       + [{"cmd": "key", "keystroke": "cmd-enter"}, W(2500), AWAY]),
}


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd, answer)
    return answer


SEED = """
import path from 'node:path'
const [harness, work, theme, responses] = process.argv.slice(1)
const { seed } = await import(path.join(path.resolve(harness), 'seed', 'index.mjs'))
seed(work, { theme, responses: JSON.parse(responses) })
"""


def run(scene, theme, opts):
    responses, steps = SCENES[scene]
    subprocess.run(["node", "--input-type=module", "-e", SEED, opts["harness"], opts["world"], theme,
                    json.dumps(responses)], check=True, capture_output=True)
    world = opts["world"]
    env = dict(os.environ, NODUS_USER_DATA=os.path.join(world, "userData"), NODUS_AUTOMATION=SOCK,
               HOME=os.path.join(world, "home"), PATH=os.path.join(world, "bin") + ":/usr/bin:/bin:/usr/sbin",
               SHELL=os.path.join(world, "bin", "fakeshell"), PARITY_RESPONSES=os.path.join(world, "responses.json"),
               TZ="UTC", LANG="en_US.UTF-8")
    env.pop("XAI_API_KEY", None)
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
            if step.get("cmd") != "key":
                drive(f, W(120))
        drive(f, {"cmd": "idle"})
        out = os.path.join(opts["out"], f"{scene}.{theme}.rust.png")
        drive(f, {"cmd": "screenshot", "path": out})
        print("shot", out)
        try:
            f.write(json.dumps({"cmd": "quit"}) + "\n")
            f.flush()
        except OSError:
            pass
    finally:
        try:
            app.wait(timeout=8)
        except subprocess.TimeoutExpired:
            app.kill()


def main():
    args = sys.argv[1:]
    opts = {
        "harness": os.path.join(REPO, "migration", "parity"),
        "out": os.path.join(REPO, "migration", "parity", "golden"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-ui-parity05"),
        "bin": os.path.join(REPO, "target", "debug", "nodus"),
    }
    themes = ["light", "dark"]
    for flag in ["--harness", "--out", "--world", "--bin", "--theme"]:
        if flag in args:
            i = args.index(flag)
            value = os.path.abspath(args[i + 1]) if flag != "--theme" else args[i + 1]
            del args[i : i + 2]
            if flag == "--theme":
                themes = [value]
            else:
                opts[flag[2:]] = value
    os.makedirs(opts["world"], exist_ok=True)
    os.makedirs(opts["out"], exist_ok=True)
    os.chdir(opts["world"])
    names = [n for n in SCENES if not args or any(n.startswith(a) for a in args)]
    for name in names:
        for theme in themes:
            run(name, theme, opts)


main()
