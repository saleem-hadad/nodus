#!/usr/bin/env python3
"""Renders the floating capture panel's parity scenes with the Rust app.

usage: capture_scenes.py [scene-prefix ...] [--theme light|dark]
                         [--harness <migration/parity>] [--golden <dir>]
                         [--world <dir>] [--bin <nodus binary>] [--node <node 22+>]

The same scenes as migration/parity/scenes/capture.mjs, driven through the
test driver (crates/nodus-ui/src/automation.rs, capture/automation.rs): the
panel opens through File › Capture…'s path, reads nothing from the screen or
the clipboard (the driver's stand-ins answer instead, as the harness's stubs
do), and its own window is shot with its see-through margin, as
<golden>/<scene>.<theme>.rust.png. Compare with compare.mjs. Needs the
harness's pngjs on NODE_PATH for the screenshot thumbnail, and Node 22 or
newer: the claude stand-in it seeds runs on the node that seeded it.
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
SOCK = "nodus-capture-parity.sock"
TEXT = "Fix the coupon field so it trims spaces before checking the code"
COPIED = "TypeError: Cannot read properties of undefined (reading ‘trim’) at applyCoupon (coupon.ts:42)"
P = {"window": "capture"}


def wait(ms):
    return {"cmd": "wait", "ms": ms}


def show():
    return [{"cmd": "capture", "action": "show"}, wait(700)]


def typed(text):
    return {"cmd": "type", "text": text, **P}


def press(control):
    return {"cmd": "capture", "action": "press", "control": control}


def stub(name, value):
    return {"cmd": "capture", "stub": name, "value": value}


def context(ctx):
    return {"cmd": "emit", "event": "captureContext", "payload": ctx}


def mouse(kind, x, y):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **P}


# scene: (seed spec, steps given the ids)
SCENES = {
    "capture-empty": ({"noCaptures": True}, lambda ids: show()),
    "capture-recents": ({}, lambda ids: [stub("statuses", {ids["sfTax"]: "waiting"})] + show() + [wait(600)]),
    "capture-guessing": (
        {"noCaptures": True, "responses": {"delayMs": 120000}},
        lambda ids: show() + [typed(TEXT), wait(1500)],
    ),
    "capture-guess": ({"noCaptures": True}, lambda ids: show() + [typed(TEXT), wait(3500)]),
    "capture-kind-menu": (
        {"noCaptures": True},
        lambda ids: show() + [typed(TEXT), wait(3500), mouse("click", 150, 64), wait(500)],
    ),
    "capture-attachments": (
        {"noCaptures": True, "thumb": True},
        lambda ids: [
            stub("screenshot", {"kind": "image", "path": "/tmp/nodus-parity-shot.png", "thumb": ids["thumb"]}),
            stub("files", [{"kind": "file", "path": "/tmp/coupon-spec.pdf", "name": "coupon-spec.pdf", "size": 248000}]),
            stub("clipboard", {"kind": "text", "text": COPIED}),
        ]
        + show()
        + [
            context({"tab": {"kind": "tab", "url": "https://shop.example.com/checkout?coupon=SPRING",
                             "title": "Checkout – Example Shop", "app": "Safari"}}),
            context({"clipboard": {"kind": "text", "text": COPIED}}),
            wait(300),
            press("screenshot"),
            wait(300),
            press("files"),
            wait(300),
            press("offer"),
            wait(300),
            typed("The coupon field breaks on this page"),
            wait(3500),
            mouse("move", 2, 2),
        ],
    ),
    "capture-receipt-note": (
        {"noCaptures": True, "guessNote": True},
        lambda ids: show() + [typed("Remember to ask Sam about the coupon limits"), wait(3500),
                              {"cmd": "key", "keystroke": "enter", **P}, wait(200)],
    ),
    "capture-receipt-session": (
        {"noCaptures": True},
        lambda ids: [stub("submit", {"kind": "session", "projectId": ids["storefront"], "projectName": "storefront",
                                     "title": TEXT, "sessionId": ids["capNew"]})]
        + show()
        + [typed(TEXT), wait(3500), {"cmd": "key", "keystroke": "enter", **P}, wait(200)],
    ),
    "capture-permission": (
        {"noCaptures": True},
        lambda ids: show() + [context({"shortcut": "needs-permission"}), wait(300)],
    ),
    # Settings' Floating capture rows (spec 08 UI-131…UI-135), shot in the
    # board window: Accessibility not allowed yet, then allowed.
    "settings": ({"capture": True}, lambda ids: [{"cmd": "menu", "command": "settings"}, wait(400)]),
    "settings-ready": (
        {"capture": True},
        lambda ids: [
            stub("trusted", True),
            # Capture looks again every 3 s while it isn't allowed (CAP-037).
            wait(3500),
            {"cmd": "emit", "event": "updateStatus",
             "payload": {"state": "downloading", "version": "0.9.5", "percent": 42, "current": "0.9.4"}},
            {"cmd": "menu", "command": "settings"},
            wait(400),
            {"cmd": "scroll", "x": 720, "y": 500, "dx": 0, "dy": -3000},
            wait(300),
        ],
    ),
    "capture-screen-permission": (
        {"noCaptures": True},
        lambda ids: [stub("screenshot", "needs-permission")] + show() + [press("screenshot"), wait(300),
                                                                         mouse("move", 2, 2)],
    ),
}


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd.get("cmd"), answer)
    return answer


def run(scene, theme, opts):
    spec, steps = SCENES[scene]
    spec = dict(spec)
    seeded = subprocess.run(
        [opts["node"], os.path.join(HERE, "capture_seed.mjs"), opts["harness"], opts["world"], theme, json.dumps(spec)],
        check=True, capture_output=True, text=True,
    )
    ids = json.loads(seeded.stdout.strip().splitlines()[-1])
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
        drive(f, {"cmd": "idle"})
        for step in steps(ids):
            drive(f, step)
            drive(f, wait(80))
        # The receipt goes 2.6 s after it comes, so it's taken straight away.
        if not scene.startswith("capture-receipt"):
            drive(f, {"cmd": "idle"})
        out = os.path.join(opts["golden"], f"{scene}.{theme}.rust.png")
        where = {} if scene.startswith("settings") else P
        drive(f, {"cmd": "screenshot", "path": out, **where})
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
        "golden": os.path.join(REPO, "migration", "parity", "golden"),
        "world": os.path.join(tempfile.gettempdir(), "nodus-capture-parity"),
        "bin": os.path.join(REPO, "target", "debug", "nodus"),
        "node": "node",
    }
    themes = ["light", "dark"]
    for flag in ["--harness", "--golden", "--world", "--bin", "--theme", "--node"]:
        if flag in args:
            i = args.index(flag)
            value = os.path.abspath(args[i + 1]) if flag not in ("--theme", "--node") else args[i + 1]
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
