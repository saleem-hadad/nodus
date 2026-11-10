#!/usr/bin/env python3
"""Starts a packaged Nodus and drives it, the way CI checks a build no one has
clicked through yet: the window opens, a terminal card's shell runs in the
session host, the host answers on its socket or pipe, the app quits cleanly,
and each step is screenshotted in light and dark.

    python3 packaging/smoke.py [--open] <Nodus.app/Contents/MacOS/Nodus or Nodus.exe> [screenshot folder]

--open (macOS) starts the bundle through LaunchServices with
`open -n --env NODUS_USER_DATA=… <bundle>`, as 0.9.4's swap script reopens
the app it put in place (UPD-063, REL-042), instead of running it directly.

The Rust port of 0.9.4's scripts/smoke.mjs (REL-032…REL-035). The app is driven
through its test driver (decision D11, NODUS_AUTOMATION; see
crates/nodus-ui/src/automation.rs), which replaces the DevTools protocol. The
driver has no `sessions.write` or output events yet, so the shell is typed
into through the session host instead, as a second app would. Where the
driver isn't built (Windows, so far), the smoke test checks what it can
without it: the window appears and the app quits when asked.

It uses its own userData folder, so real boards are never touched, and stops
the session host and the app it started, whatever happens. Standard library
only.
"""

import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

WINDOWS = sys.platform == "win32"

# The step labels are UTF-8 (▸); Windows' console code page would refuse them.
for stream in (sys.stdout, sys.stderr):
    stream.reconfigure(encoding="utf-8")


def step(text):
    print(f"\n▸ {text}", flush=True)


def until(what, fn, timeout=60.0):
    end = time.monotonic() + timeout
    while True:
        try:
            value = fn()
            if value:
                return value
        except Exception:  # not yet
            pass
        if time.monotonic() > end:
            raise RuntimeError(f"Timed out waiting for {what}")
        time.sleep(0.25)


class Lines:
    """A newline-delimited JSON connection: a Unix socket or a Windows pipe.

    Read only when asked, never from a thread: a synchronous pipe handle
    makes a write wait for a read in progress on it.
    """

    def __init__(self, path):
        self.pending = b""
        self.closed = False
        if WINDOWS:
            self.pipe = open(path, "r+b", buffering=0)
            self.sock = None
        else:
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.sock.connect(path)
            self.pipe = None

    def send(self, message):
        data = (json.dumps(message) + "\n").encode()
        if self.pipe:
            self.pipe.write(data)
        else:
            self.sock.sendall(data)

    def _chunk(self, timeout):
        """What has arrived, waiting up to `timeout` for something; b"" once closed."""
        if not WINDOWS:
            self.sock.settimeout(max(timeout, 0.001))
            try:
                return self.sock.recv(65536)
            except socket.timeout:
                return None
        import ctypes
        import msvcrt
        from ctypes import wintypes

        handle = msvcrt.get_osfhandle(self.pipe.fileno())
        available = wintypes.DWORD()
        end = time.monotonic() + timeout
        while True:
            if not ctypes.windll.kernel32.PeekNamedPipe(
                wintypes.HANDLE(handle), None, 0, None, ctypes.byref(available), None
            ):
                return b""  # the other end closed it
            if available.value:
                return self.pipe.read(available.value)
            if time.monotonic() > end:
                return None
            time.sleep(0.05)

    def receive(self, timeout):
        end = time.monotonic() + timeout
        while b"\n" not in self.pending:
            if self.closed:
                raise RuntimeError("the connection closed")
            chunk = self._chunk(end - time.monotonic())
            if chunk is None:
                raise TimeoutError("no answer in time")
            if not chunk:
                self.closed = True
            self.pending += chunk or b""
        line, self.pending = self.pending.split(b"\n", 1)
        return json.loads(line) if line.strip() else self.receive(max(0.001, end - time.monotonic()))

    def close(self):
        try:
            if self.pipe:
                self.pipe.close()
            else:
                self.sock.close()
        except OSError:
            pass


class Driver:
    """The app's test driver: one command per line, one answer per command."""

    def __init__(self, path):
        self.conn = Lines(path)

    def __call__(self, cmd, timeout=60.0, **args):
        self.conn.send({"cmd": cmd, **args})
        answer = self.conn.receive(timeout)
        if not answer.get("ok"):
            raise RuntimeError(f"{cmd} {json.dumps(args)}: {answer.get('error')}")
        return answer.get("result")

    def core(self, method, *args):
        return self("core", method=method, args=list(args))


def host_socket(user_data):
    """Where the session host listens for this userData (HOST-001, REL-033)."""
    folder = os.path.join(user_data, "host")
    digest = hashlib.sha1(folder.encode()).hexdigest()[:12]
    if WINDOWS:
        return rf"\\.\pipe\nodus-{digest}"
    path = os.path.join(folder, "host.sock")
    if len(path.encode()) > 100:
        tmp = (os.environ.get("TMPDIR") or os.environ.get("TMP") or os.environ.get("TEMP") or "/tmp").rstrip("/") or "/"
        path = os.path.join(tmp, f"nodus-{digest}.sock")
    return path


def host_welcome(user_data):
    """Says hello as protocol 0, which the host never counts as an app, and returns its welcome."""
    conn = Lines(host_socket(user_data))
    try:
        conn.send({"type": "hello", "protocol": 0})
        return conn.receive(10)
    finally:
        conn.close()


def windows_window(pid):
    """Whether the process has a visible top-level window."""
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    found = []
    proc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def each(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            found.append(hwnd)
        return True

    user32.EnumWindows(proc(each), 0)
    return bool(found)


def windows_screen(path):
    """The whole screen, through PowerShell, as scripts/smoke.mjs took it."""
    file = os.path.abspath(path).replace("'", "''")
    script = (
        "$ProgressPreference = 'SilentlyContinue'; Add-Type -AssemblyName System.Windows.Forms, System.Drawing; "
        "$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; "
        "$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height; "
        "[System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); "
        f"$bmp.Save('{file}')"
    )
    subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], check=False)


class Opened:
    """An app LaunchServices started: not our child, so known by its pid alone."""

    def __init__(self, pid):
        self.pid = pid
        self.returncode = None

    def poll(self):
        if self.returncode is None:
            try:
                os.kill(self.pid, 0)
            except ProcessLookupError:
                self.returncode = 0  # gone; LaunchServices keeps the exit code
        return self.returncode

    def wait(self, timeout):
        until("Nodus to exit", lambda: self.poll() is not None, timeout)
        return self.returncode

    def kill(self):
        try:
            os.kill(self.pid, 9)
        except ProcessLookupError:
            pass

    terminate = kill


def open_bundle(exe, env, log_path):
    """`open -n --env … <bundle>`, then the pid of the process running exactly `exe`."""
    bundle = exe[: exe.rindex("/Contents/MacOS/")]
    args = ["/usr/bin/open", "-n", "--stdout", log_path, "--stderr", log_path]
    for name in ("NODUS_USER_DATA", "NODUS_AUTOMATION"):
        args += ["--env", f"{name}={env[name]}"]
    subprocess.run(args + [bundle], check=True)

    def started():
        listing = subprocess.run(["/bin/ps", "-axo", "pid=,args="], capture_output=True, text=True).stdout
        for line in listing.splitlines():
            pid, _, command = line.strip().partition(" ")
            if command.strip() == exe:
                return int(pid)
        return None

    return Opened(until("the app process", started, 30))


def main():
    argv = sys.argv[1:]
    launch_services = "--open" in argv
    argv = [a for a in argv if a != "--open"]
    if not argv:
        sys.exit("usage: python3 packaging/smoke.py [--open] <app executable> [screenshot folder]")
    exe = os.path.abspath(argv[0])
    out = os.path.abspath(argv[1] if len(argv) > 1 else "smoke")
    os.makedirs(out, exist_ok=True)
    root = tempfile.mkdtemp(prefix="nodus-smoke-")
    user_data = os.path.join(root, "userData")
    repo = os.path.join(root, "demo")
    os.makedirs(repo)
    # A short path: Unix sockets top out at 104 bytes on macOS.
    driver_path = os.path.join(root, "driver.sock")
    if not WINDOWS and len(driver_path.encode()) > 100:
        driver_path = os.path.join("/tmp", f"nodus-smoke-{os.getpid()}.sock")
    if WINDOWS:
        driver_path = rf"\\.\pipe\nodus-smoke-{os.getpid()}"

    step(f"Starting {exe}")
    env = {**os.environ, "NODUS_USER_DATA": user_data, "NODUS_AUTOMATION": driver_path}
    log = open(os.path.join(out, "app.log"), "wb")
    if launch_services:
        app = open_bundle(exe, env, os.path.join(out, "app.log"))
    else:
        app = subprocess.Popen([exe], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    failed = False
    driver = None
    try:
        step("Waiting for the window")

        def connected():
            if app.poll() is not None:
                raise SystemExit(f"Nodus exited with {app.returncode}")
            return Driver(driver_path)

        try:
            driver = until("the test driver", connected, 30 if WINDOWS else 60)
        except RuntimeError:
            if not WINDOWS:
                raise
        if driver is None:
            print("  the test driver isn't built for Windows yet: checking the window only")
            until("the window", lambda: windows_window(app.pid), 60)
            print("  the window is up")
            time.sleep(3)
            windows_screen(os.path.join(out, "1-screen.png"))
            if app.poll() is not None:
                raise RuntimeError(f"Nodus exited with {app.returncode}")
        else:
            info = driver("window")
            print(f"  window {info}")
            driver("idle")
            driver("screenshot", path=os.path.join(out, "1-projects-light.png"))

            step("Adding a project")
            project = driver.core("projects.add", repo)
            if not project or not project.get("id"):
                raise RuntimeError(f"projects.add answered {project!r}")
            # The grid reads the projects again, as scripts/smoke.mjs's reload made it.
            driver("emit", event="projectsChanged")
            driver("idle")

            step("Starting a terminal card")
            card = driver.core("sessions.create", project["id"], {"x": 80, "y": 80, "kind": "terminal", "expanded": True})
            session_id = card["sessionId"]
            print(f"  card {session_id}, live {card.get('live')}")

            step("Asking the session host for its sessions")
            welcome = until(
                "the host to list the terminal",
                lambda: (w := host_welcome(user_data)) and any(s["sessionId"] == session_id for s in w["sessions"]) and w,
                30,
            )
            print(f"  {host_socket(user_data)}: protocol {welcome['protocol']}, {len(welcome['sessions'])} session(s)")
            # On Windows the app only trusts a host whose welcome has the key in its private folder.
            if WINDOWS:
                with open(os.path.join(user_data, "host", "host.key"), encoding="utf-8") as f:
                    if welcome.get("key") != f.read():
                        raise RuntimeError("The host's welcome doesn't carry the key in host.key")

            step("Typing into the terminal")
            # The driver can't type into a session yet, so this says hello on
            # the app's protocol, as a second window would, and types there.
            shell = Lines(host_socket(user_data))
            try:
                shell.send({"type": "hello", "protocol": welcome["protocol"]})
                said = ""
                end = time.monotonic() + 30
                while True:
                    message = shell.receive(max(0.1, end - time.monotonic()))
                    if message.get("type") == "welcome":
                        said += "".join(s.get("buffer", "") for s in message["sessions"] if s["sessionId"] == session_id)
                    elif message.get("type") == "data" and message.get("sessionId") == session_id:
                        said += message["data"]
                    if said:
                        break
                # Only the shell's output has the sum; the typed line shows the sum's parts.
                total = 'echo ("nodus-" + (6 * 7))' if WINDOWS else "echo nodus-$((6 * 7))"
                shell.send({"type": "write", "sessionId": session_id, "data": total + "\r"})
                while "nodus-42" not in said:
                    message = shell.receive(max(0.1, end - time.monotonic()))
                    if message.get("type") == "data" and message.get("sessionId") == session_id:
                        said += message["data"]
                    if time.monotonic() > end:
                        raise RuntimeError("Timed out waiting for the shell to answer")
                print("  the shell answered")
            finally:
                shell.close()

            # REL-032: the project's board with its terminal card, then dark.
            step("The board")
            driver("open", project=project["name"])
            driver("idle")
            driver("screenshot", path=os.path.join(out, "2-board-light.png"))

            step("Dark mode")
            driver.core("settings.update", {"theme": "dark"})
            driver("idle")
            driver("screenshot", path=os.path.join(out, "3-board-dark.png"))
            driver.core("settings.update", {"theme": "light"})

        step("Quitting")
        if driver is not None:
            # Quit as the menu does: the app goes, its sessions stay in the host.
            driver("quit")
            driver.conn.close()
            driver = None
            code = app.wait(30)
            print(f"  Nodus exited with {code}")
            if code != 0:
                raise RuntimeError(f"Nodus exited with {code}")
        else:
            app.terminate()
            app.wait(30)
        print("\n✓ Smoke test passed")
    except BaseException as err:  # noqa: BLE001 — every failure ends the same way
        failed = True
        print(f"\n✗ Smoke test failed: {err}", file=sys.stderr)
        if driver is not None and app.poll() is None:
            try:
                driver("screenshot", path=os.path.join(out, "failure.png"), timeout=10)
            except Exception:
                pass
    finally:
        if driver is not None:
            driver.conn.close()
        # Stops the host and its sessions, then the app.
        try:
            conn = Lines(host_socket(user_data))
            conn.send({"type": "hello", "protocol": 0})
            conn.send({"type": "shutdown"})
            time.sleep(1)
            conn.close()
        except Exception:
            pass  # no host
        if app.poll() is None:
            app.kill()
            app.wait(10)
        log.close()
        host_log = os.path.join(user_data, "host", "host.log")
        if os.path.exists(host_log):
            shutil.copyfile(host_log, os.path.join(out, "host.log"))
        if not WINDOWS and os.path.exists(driver_path):
            os.unlink(driver_path)
        shutil.rmtree(root, ignore_errors=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
