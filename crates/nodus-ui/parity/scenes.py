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
SOCK = "nodus-parity.sock"  # relative: macOS caps socket paths at 104 bytes


def M(kind, x, y, **kw):
    return {"cmd": "mouse", "kind": kind, "x": x, "y": y, **kw}


AWAY = M("move", 720, 26)  # the title bar's empty middle
STOREFRONT = (104, 220)
OPEN_STOREFRONT = [M("click", *STOREFRONT), {"cmd": "key", "keystroke": "cmd-o"}, {"cmd": "wait", "ms": 300}]

# storefront opens at x 60, y 96, zoom 0.8 under the 52px title bar: flow -> window.
def AT(x, y):
    return (60 + x * 0.8, 52 + 96 + y * 0.8)


def OPEN(name):
    return [{"cmd": "open", "project": name}, {"cmd": "wait", "ms": 1000}]


def sid(key):
    """The seed's session id for a key (`sessionId` in seed/world.mjs)."""
    h = hashlib.sha1(f"nodus-parity:session:{key}".encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}"


def STATUS(key, status, live=True):
    return {"cmd": "emit", "event": "status", "payload": {"sessionId": sid(key), "status": status, "live": live}}


# storefront's sessions as `storefront()` in scenes/helpers.mjs starts them.
STOREFRONT_LIVE = OPEN("storefront") + [
    STATUS("sf-skeleton", "working"), STATUS("sf-tax", "waiting"),
    STATUS("sf-cart", "done"), STATUS("sf-terminal", "done")]


EMPTY = AT(1150, 690)  # the empty board below the terminal card
STACK = AT(172, 200)  # the archive stack's top card
PAD = AT(172, 546)  # the pad's top sheet
SF_CART = AT(1216, 148)  # the middle of sf-cart's card
NOTE = AT(1508, 148)  # the middle of the yellow note
SIGNED = AT(1520, 408)  # the middle of the signed blue note

# scene: (seed mutations, steps)
SCENES = {
    "board-empty": ([], OPEN("mobile-app") + [AWAY]),
    "board-populated": ([], STOREFRONT_LIVE + [AWAY]),
    "board-zoomed-out": (["zoomOut"], STOREFRONT_LIVE + [AWAY]),
    "board-zoomed-in": (["zoomIn"], STOREFRONT_LIVE + [AWAY]),
    "board-marquee": ([], STOREFRONT_LIVE + [
        M("down", *EMPTY), M("drag", EMPTY[0] - 150, EMPTY[1] - 100),
        M("drag", EMPTY[0] - 330, EMPTY[1] - 260)]),
    "board-menu-pane": ([], STOREFRONT_LIVE + [M("right", *EMPTY)]),
    "board-menu-archived": ([], STOREFRONT_LIVE + [M("right", *STACK)]),
    "board-selected": ([], STOREFRONT_LIVE + [
        M("click", 1030, 300, shift=True), M("click", 495, 560, shift=True), AWAY]),
    "board-section-naming": (["slowNaming"], STOREFRONT_LIVE + [
        M("click", 495, 560, shift=True), M("click", 776, 560, shift=True),
        {"cmd": "key", "keystroke": "cmd-g"}, {"cmd": "wait", "ms": 700}, AWAY]),
    "board-section-rename": ([], STOREFRONT_LIVE + [M("double", 700, 163), {"cmd": "wait", "ms": 300}, AWAY]),
    "board-toast": (["run"], OPEN("storefront") + [{"cmd": "wait", "ms": 400}, AWAY]),
    # 0.9.4 clicks the card's Archive button; selecting it and pressing ⌫ asks the same.
    "board-confirm-archive": ([], STOREFRONT_LIVE + [
        M("click", 497, 300, shift=True), {"cmd": "key", "keystroke": "backspace"}, AWAY]),
    # Not 0.9.4 goldens: interactions to look at.
    "try-drag-out": ([], STOREFRONT_LIVE + [
        M("down", 1030, 300), M("drag", 1031, 301), M("drag", 1080, 500), M("drag", 1130, 640),
        M("up", 1130, 640), {"cmd": "wait", "ms": 600}, AWAY]),
    "try-select-all": ([], STOREFRONT_LIVE + [{"cmd": "key", "keystroke": "cmd-a"}, AWAY]),
    "try-wait": ([], OPEN("storefront") + [{"cmd": "wait", "ms": 4000}, AWAY]),
    "try-pane-note": ([], STOREFRONT_LIVE + [M("right", *EMPTY), {"cmd": "wait", "ms": 200},
        M("click", 1020, 761), {"cmd": "wait", "ms": 600}, AWAY]),
    "board-menu-section": ([], STOREFRONT_LIVE + [M("right", 700, 163)]),
    "board-menu-card": ([], STOREFRONT_LIVE + [M("right", *SF_CART)]),
    "board-menu-note": ([], STOREFRONT_LIVE + [M("right", *NOTE)]),
    "board-note-editing": ([], STOREFRONT_LIVE + [M("double", *NOTE), {"cmd": "wait", "ms": 300}]),
    "board-note-selected": ([], STOREFRONT_LIVE + [M("click", *SIGNED), AWAY]),
    "board-card-hover": ([], STOREFRONT_LIVE + [M("move", *SF_CART), {"cmd": "wait", "ms": 300}]),
    "board-archive-hover": ([], STOREFRONT_LIVE + [M("move", *STACK), {"cmd": "wait", "ms": 300}]),
    "board-archive-open": ([], STOREFRONT_LIVE + [M("click", *STACK), {"cmd": "wait", "ms": 900}, AWAY]),
    "board-archive-search": ([], STOREFRONT_LIVE + [M("move", *STACK), {"cmd": "wait", "ms": 300},
                                                    {"cmd": "type", "text": "fix"}, {"cmd": "wait", "ms": 800}]),
    "board-pad-hover": ([], STOREFRONT_LIVE + [M("move", PAD[0] + 40, PAD[1] + 40), M("move", *PAD),
                                               {"cmd": "wait", "ms": 300}]),
    "board-deck-fanned": ([], STOREFRONT_LIVE + [M("move", *AT(544, 263)), {"cmd": "wait", "ms": 400}]),
    "board-expanded-stopped": ([], OPEN("infra") + [AWAY]),
    # Resume session on in-certs, Start shell on the terminal card (the
    # overlay buttons, centred in the cards' bodies), then in-certs' status
    # line as `app.usage('in-certs')` reports it.
    "board-expanded": ([], OPEN("infra") + [
        M("click", 577, 319), {"cmd": "wait", "ms": 1500},
        M("click", 1118, 319), {"cmd": "wait", "ms": 1500},
        {"cmd": "emit", "event": "usage", "payload": {"sessionId": sid("in-certs"), "usage": {
            "model": "Opus 5.5", "contextPercent": 42, "contextTokens": 84000, "contextSize": 200000,
            "costUsd": 1.84, "durationMs": 1394000, "at": 4102444800000}}},
        {"cmd": "wait", "ms": 600}, AWAY]),
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
    # 0.9.4's scene made `projects:add` fail; here the picker answers a file.
    "projects-toast": ([], [{"cmd": "stub", "name": "pickFolder", "path": "/etc/hosts"},
                            M("click", 1366, 26), {"cmd": "wait", "ms": 400}]),
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
            drive(f, {"cmd": "wait", "ms": 120})
        drive(f, {"cmd": "idle"})
        # A frame drawn now: macOS stops asking a covered window for frames,
        # so a transition (the archive panel's 460ms growth) may not have
        # been drawn to its end.
        drive(f, {"cmd": "terminal", "action": "draw"})
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
        # A session host a scene started outlives the app, as it's meant to:
        # stop it, so the next scene's world starts without one.
        subprocess.run(["pkill", "-f", os.path.join(world, "userData", "host")], check=False)


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
