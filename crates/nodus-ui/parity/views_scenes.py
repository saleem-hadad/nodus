#!/usr/bin/env python3
"""Renders the views' parity scenes (spec 04) with the Rust app, beside the goldens.

usage: views_scenes.py [scene-prefix ...] [--theme light|dark]
                       [--harness <dir, default harness/>] [--golden <dir>] [--world <dir>] [--bin <nodus>]

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


# Not a 0.9.4 golden: a page that calls every method of window.nodus and
# writes what came back, to check the relay end to end in a live window.
RELAY_PAGE = """<!doctype html><html><head><style>
body{padding:8px 12px;font:12px -apple-system,sans-serif}pre{white-space:pre-wrap;margin:44px 0 0}
#b{position:fixed;left:10px;top:6px;width:180px;height:30px}
</style></head><body><button id="b">start a session</button><pre id="out"></pre><script>
var out=document.getElementById('out');
function log(k,v){out.textContent+=k+': '+v+'\\n';}
function tryit(k,p){p.then(function(r){log(k,JSON.stringify(r))},function(e){log(k,'ERR '+e.message)});}
tryit('notes',nodus.notes().then(function(n){return n.length}));
tryit('views',nodus.views().then(function(v){return v.map(function(x){return x.title})}));
tryit('view',nodus.view(nodus.viewId).then(function(v){return v.title}));
tryit('sessions',nodus.sessions().then(function(s){return s.length}));
tryit('git',nodus.git().then(function(g){return g&&g.branch}));
tryit('setState',nodus.setState({n:1}));
tryit('openUrl',nodus.openUrl('ftp://x'));
tryit('startNoClick',nodus.startSession('hi'));
tryit('openSession',nodus.openSession(42));
nodus.on('board',function(){log('event','board')});
nodus.on('git',function(g){log('event','git '+(g?g.branch:g))});
nodus.on('state',function(s){log('event','state '+JSON.stringify(s))});
document.getElementById('b').onclick=function(){tryit('startClick',nodus.startSession('hello from a view',{title:'From the view'}))};
</script></body></html>"""


def relay_setup(world):
    """Docs build status gets the relay page, and may start sessions."""
    path = os.path.join(world, "userData", "state.json")
    state = json.load(open(path))
    for board in state["boards"].values():
        for view in board.get("views", {}).values():
            if view["title"] == "Docs build status":
                view["startsSessions"] = True
                with open(os.path.join(world, "userData", "views", view["id"], "index.html"), "w") as page:
                    page.write(RELAY_PAGE)
    json.dump(state, open(path, "w"))


WHEEL_PAGE = """<!doctype html><html><body><p>wheel</p><script>
setTimeout(function(){parent.postMessage({nodus:1,wheel:{dx:0,dy:200,mode:0,x:10,y:10}},'*')},500);
</script></body></html>"""


def page_setup(html):
    def setup(world):
        path = os.path.join(world, "userData", "state.json")
        state = json.load(open(path))
        for board in state["boards"].values():
            for view in board.get("views", {}).values():
                if view["title"] == "Docs build status":
                    with open(os.path.join(world, "userData", "views", view["id"], "index.html"), "w") as page:
                        page.write(html)
    return setup


def zoom_setup(zoom, x, y):
    """docs-site opens at another zoom."""
    def setup(world):
        path = os.path.join(world, "userData", "state.json")
        state = json.load(open(path))
        for board in state["boards"].values():
            if any(v["title"] == "Docs build status" for v in board.get("views", {}).values()):
                board["viewport"] = {"x": x, "y": y, "zoom": zoom}
        json.dump(state, open(path, "w"))
    return setup


def ink_setup(world):
    """Ink across Docs build status (720, 0, 480 × 360 units): a pen line, an
    arrow and a label, half on the page and half off it."""
    path = os.path.join(world, "userData", "state.json")
    state = json.load(open(path))
    for board in state["boards"].values():
        if any(v["title"] == "Docs build status" for v in board.get("views", {}).values()):
            board["ink"] = [
                {"id": "ink-pen", "kind": "pen", "color": "red", "size": 1.6,
                 "points": [640, 200, 700, 215, 780, 236, 860, 248, 940, 245, 1010, 230, 1060, 214]},
                {"id": "ink-arrow", "kind": "arrow", "color": "blue", "size": 1.6, "points": [880, 420, 1080, 140]},
                {"id": "ink-label", "kind": "text", "color": "graphite", "size": 1.6, "points": [660, 300],
                 "text": "Why did this one fail?"},
            ]
    json.dump(state, open(path, "w"))


SETUP = {"try-relay": relay_setup, "try-wheel": page_setup(WHEEL_PAGE), "try-ink": ink_setup,
         "try-ink-select": ink_setup,
         "try-zoom-in": zoom_setup(1.6, -1100, 40), "try-zoom-out": zoom_setup(0.35, 120, 160)}
# PLAT-009: the page's scheme is standard (a host, a path) and secure (a
# secure context, crypto.subtle), as Electron registered it. The page keeps
# what it sees with setState: <userData>/views/<id>/runtime.json.
SCHEME_PAGE = """<!doctype html><html><body><script>
nodus.setState({secure: isSecureContext, subtle: typeof (crypto && crypto.subtle), protocol: location.protocol,
  host: location.host, path: location.pathname, origin: self.origin, href: location.href});
</script></body></html>"""
SETUP["try-scheme"] = page_setup(SCHEME_PAGE)
SCENES["try-scheme"] = [{"cmd": "open", "project": "docs-site"}, {"cmd": "wait", "ms": 3000}]
SIZE_PAGE = """<!doctype html><html><body style="margin:0;font:20px -apple-system"><div id="o"></div><script>
function show(){document.getElementById('o').textContent=innerWidth+'x'+innerHeight+' dpr '+devicePixelRatio+' '+document.documentElement.clientWidth;}
show();addEventListener('resize',show);
</script></body></html>"""
SETUP["try-size"] = page_setup(SIZE_PAGE)
SCENES["try-size"] = DOCS() + [AWAY]
# Ink over a live page (INKUI-025): run with VIEWS_LIVE=1 VIEWS_SCREENCAPTURE=<png>
# to see the page and its overlay; the page stays live and takes the pointer.
SCENES["try-ink"] = DOCS() + [{"cmd": "wait", "ms": 1500}, {"cmd": "views"}, {"cmd": "hit", "x": 560, "y": 250}]
# Run with VIEWS_LIVE=1: another element's tooltip over a live page (the
# note's ⤢ beside Docs build status) puts the page behind its snapshot.
SCENES["try-tooltip"] = DOCS() + [{"cmd": "wait", "ms": 1500}, M("move", 452, 134), {"cmd": "wait", "ms": 1500},
                                  {"cmd": "views"}]
# The pen line selected where it's off the page (x 640 units): its outline
# crosses the page too.
SCENES["try-ink-select"] = DOCS() + [{"cmd": "wait", "ms": 1500}, M("click", *AT(641, 200)),
                                     {"cmd": "wait", "ms": 300}, {"cmd": "views"}]
SCENES["try-zoom-in"] = DOCS() + [AWAY]
SCENES["try-zoom-out"] = DOCS() + [AWAY]
# A scroll the page has no use for pans the board (VIEW-047): the board moves up 200px.
SCENES["try-wheel"] = DOCS() + [{"cmd": "wait", "ms": 1500}, {"cmd": "views"}]
# The page takes the keyboard on a click, and gives it back on a click on the board (VIEW-051).
SCENES["try-keyboard"] = DOCS() + [{"cmd": "native-click", "x": 560, "y": 250}, {"cmd": "wait", "ms": 400},
                                   {"cmd": "views"}, {"cmd": "native-click", "x": 1100, "y": 720},
                                   {"cmd": "wait", "ms": 400}, {"cmd": "views"}]
SCENES["try-relay"] = DOCS() + [{"cmd": "wait", "ms": 1500}, {"cmd": "views"},
                                {"cmd": "hit", "x": 560, "y": 160}, {"cmd": "hit", "x": 300, "y": 400},
                                {"cmd": "native-click", "x": 520, "y": 159}, {"cmd": "wait", "ms": 2500}]


# Interactions to look at (not 0.9.4 goldens).
HEAD = (600, TOOL_Y)  # Docs build status's title bar, clear of its tools
CORNER = AT(1200, 360)  # its bottom-right corner
SCENES["try-rename"] = DOCS() + [M("double", 530, TOOL_Y), {"cmd": "wait", "ms": 300},
                                 {"cmd": "key", "keystroke": "cmd-a"}, {"cmd": "type", "text": "Builds"},
                                 {"cmd": "key", "keystroke": "enter"}, {"cmd": "wait", "ms": 300}, AWAY]
SCENES["try-refresh"] = DOCS() + [M("move", *BUILD), {"cmd": "wait", "ms": 200}, M("click", *TOOL(4)),
                                  {"cmd": "wait", "ms": 2500}]
SCENES["try-resize"] = DOCS() + [M("click", *HEAD), {"cmd": "wait", "ms": 200}, M("down", *CORNER),
                                 M("drag", CORNER[0] + 40, CORNER[1] + 40), M("drag", CORNER[0] + 100, CORNER[1] + 100),
                                 {"cmd": "views"}, M("up", CORNER[0] + 100, CORNER[1] + 100), {"cmd": "wait", "ms": 800}]
SCENES["try-drag"] = DOCS() + [M("down", *HEAD), M("drag", HEAD[0] + 2, HEAD[1] + 2),
                               M("drag", HEAD[0] + 60, HEAD[1] + 300), {"cmd": "views"}, {"cmd": "hit", "x": 900, "y": 300},
                               M("up", HEAD[0] + 60, HEAD[1] + 300), {"cmd": "wait", "ms": 800}, {"cmd": "views"}]
# The New view box empty (its placeholder wraps), and its grip dragged 60px down.
NEW_VIEW = DOCS() + [M("right", 1100, 720), {"cmd": "wait", "ms": 300}, M("click", 1150, 787), {"cmd": "wait", "ms": 400}]
SCENES["try-new-empty"] = NEW_VIEW
SCENES["try-new-grip"] = NEW_VIEW + [M("down", 1001, 449), M("drag", 1001, 480), M("drag", 1001, 509),
                                     M("up", 1001, 509), {"cmd": "wait", "ms": 300}]
SCENES["try-create"] = DOCS() + [M("right", 1100, 720), {"cmd": "wait", "ms": 300}, M("click", 1150, 787),
                                 {"cmd": "wait", "ms": 400}, {"cmd": "type", "text": "Open issues labelled docs"},
                                 {"cmd": "key", "keystroke": "cmd-enter"}, {"cmd": "wait", "ms": 3000}]


def drive(f, cmd):
    f.write(json.dumps(cmd) + "\n")
    f.flush()
    answer = json.loads(f.readline() or "{}")
    if not answer.get("ok"):
        print("  !", cmd, answer)
    elif cmd.get("cmd") in ("views", "hit"):
        print(json.dumps(answer.get("result"), indent=1))
    return answer


def run(scene, theme, opts):
    steps = SCENES[scene]
    subprocess.run(["node", os.path.join(HERE, "seed.mjs"), opts["harness"], opts["world"], theme],
                   check=True, capture_output=True)
    world = opts["world"]
    if scene in SETUP:
        SETUP[scene](world)
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
        "harness": os.path.join(HERE, "harness"),
        "golden": os.path.join(HERE, "harness", "golden"),
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
