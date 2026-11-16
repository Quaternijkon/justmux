#!/usr/bin/env python3
"""Bounded, real PTY regression checks; every tmux command uses a private socket."""
from __future__ import annotations
import argparse
import fcntl
import json
import os
import pty
import selectors
import shlex
import signal
import struct
import subprocess
import sys
import termios
import time
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
JM = ROOT / "target/release/jm"
RECORDS = []
START = time.monotonic()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def clean_env():
    env = os.environ.copy()
    for name in ("TMUX", "TMUX_PANE", "JUSTMUX_CLIENT", "JUSTMUX_PANE", "JUSTMUX_POPUP", "TMUX_POPUP"):
        env.pop(name, None)
    env["TERM"] = "xterm-256color"
    return env


class PTY:
    def __init__(self, argv):
        # Synchronise before exec so baseline termios and size cannot race jm.
        ready_r, ready_w = os.pipe()
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.close(ready_w)
            os.read(ready_r, 1)
            os.close(ready_r)
            os.execvpe(argv[0], argv, clean_env())
        os.close(ready_r)
        self.original_pid = self.pid
        self.baseline = termios.tcgetattr(self.fd)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        os.set_blocking(self.fd, False)
        self.sel = selectors.DefaultSelector()
        self.sel.register(self.fd, selectors.EVENT_READ)
        self.out = bytearray()
        self.status = None
        os.write(ready_w, b"1")
        os.close(ready_w)

    def read(self, seconds=.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            for _ in self.sel.select(max(0, deadline - time.monotonic())):
                try:
                    chunk = os.read(self.fd, 65536)
                    if chunk:
                        self.out.extend(chunk)
                        RECORDS.append([round(time.monotonic() - START, 4), "o", chunk.decode("utf-8", "replace")])
                except (BlockingIOError, OSError):
                    pass
        return bytes(self.out)

    def send(self, text):
        for char in text:
            os.write(self.fd, char.encode())
            time.sleep(.015)

    def until(self, predicate, message, timeout=6):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            self.read(.08)
        raise RuntimeError(message)

    def text(self, value, timeout=6):
        self.until(lambda: value.encode() in self.out, f"terminal output missing {value!r}", timeout)

    def wait_exit(self, timeout=6):
        def exited():
            if self.pid < 0:
                return True
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.pid, self.status = -1, status
                return True
            return False
        self.until(exited, "PTY child did not exit", timeout)
        self.read(.1)

    def assert_restored(self):
        require(termios.tcgetattr(self.fd) == self.baseline, "terminal termios was not fully restored")
        require(b"\x1b[?25h" in self.out, "cursor Show sequence missing")
        require(b"\x1b[?1049l" in self.out, "LeaveAlternateScreen sequence missing")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        try:
            if self.pid > 0:
                try:
                    os.kill(self.pid, signal.SIGTERM)
                    self.wait_exit(2)
                except (ProcessLookupError, RuntimeError):
                    if self.pid > 0:
                        try:
                            os.kill(self.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        # SIGKILL guarantees termination; still use a deadline.
                        self.wait_exit(2)
        finally:
            self.sel.close()
            os.close(self.fd)


class Socket:
    def __init__(self):
        self.name = f"jm-smoke-{os.getpid()}-{uuid.uuid4().hex[:8]}"

    def argv(self, *args):
        return ["tmux", "-L", self.name, "-f", "/dev/null", *args]

    def run(self, *args, check=True):
        result = subprocess.run(self.argv(*args), env=clean_env(), text=True,
                                capture_output=True, timeout=5)
        require(not check or result.returncode == 0, result.stderr.strip() or "tmux failed")
        return result.stdout.strip()

    def cleanup(self):
        self.run("kill-server", check=False)

    def clients(self):
        output = self.run("list-clients", "-F", "#{client_pid}\t#{client_name}\t#{client_session}")
        return {int(row.split("\t")[0]): row.split("\t")[1:] for row in output.splitlines() if row}


def external(sock):
    sock.run("new-session", "-d", "-s", "orion", "/bin/sh")
    with PTY([str(JM), "-L", sock.name]) as p:
        p.text("JustMux")
        p.send("/resume\r")
        p.text("orion")
        pid = p.pid
        p.send("\r")
        p.until(lambda: pid in sock.clients(), "jm PID did not become native tmux client")
        command = subprocess.run(["ps", "-p", str(pid), "-o", "comm="], text=True, capture_output=True, timeout=5).stdout
        require("tmux" in command and "jm" not in Path(command.strip()).name, f"exec evidence failed: {command!r}")
        # An external sentinel proves native input reached the pane, not echoed UI text.
        marker = f"@jm-smoke-{uuid.uuid4().hex[:8]}"
        p.send(f"tmux set-option -g {marker} native\r")
        p.until(lambda: sock.run("show-option", "-gv", marker, check=False) == "native", "native pane command did not execute")
        p.send("\x02d")
        p.wait_exit()
        require(os.waitstatus_to_exitcode(p.status) == 0, "native detach failed")
        p.assert_restored()
    print("PASS external /resume: original jm PID became tmux; native input + detach + termios restored")


def create(sock):
    with PTY([str(JM), "-L", sock.name]) as p:
        p.text("JustMux")
        p.send("/new\r")
        p.read(.2)
        p.send("smoke-new\r")
        p.read(.2)
        p.send(str(ROOT) + "\r")
        p.until(lambda: "smoke-new" in sock.run("list-sessions", "-F", "#{session_name}", check=False), "/new did not create native session")
        path = sock.run("display-message", "-p", "-t", "=smoke-new", "#{session_path}")
        require(Path(path).resolve() == ROOT, "new session working directory differs")
        p.send("\x1b"); p.read(.15); p.send("\x1b"); p.read(.15)
        p.send("\x15/quit\r")
        p.wait_exit()
        p.assert_restored()
    require("smoke-new" in sock.run("list-sessions", "-F", "#{session_name}"), "session did not outlive jm")
    print("PASS /new: native session and cwd verified; /quit restores terminal; session survives")


def interrupt(sock, method):
    with PTY([str(JM), "-L", sock.name]) as p:
        p.text("JustMux")
        require(not (termios.tcgetattr(p.fd)[3] & termios.ICANON), "jm did not enter raw mode")
        if method == "Ctrl+C":
            p.send("\x03")
        else:
            os.kill(p.pid, signal.SIGTERM)
        p.wait_exit()
        p.assert_restored()
        expected = 0 if method == "Ctrl+C" else 128 + signal.SIGTERM
        require(os.waitstatus_to_exitcode(p.status) == expected, f"unexpected {method} status: {p.status}")
    print(f"PASS {method}: full termios, cursor and alternate screen restored")


def popup(sock):
    sock.run("new-session", "-d", "-s", "popup-source", "/bin/sleep", "300")
    sock.run("new-session", "-d", "-s", "popup-target", "/bin/sh")
    source_pid = int(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_pid}"))
    with PTY(sock.argv("attach-session", "-t", "=popup-source")) as first, PTY(sock.argv("attach-session", "-t", "=popup-source")) as second:
        first.until(lambda: len(sock.clients()) == 2, "two native clients did not attach")
        # Exercise the exact recommended config syntax, adding only test socket,
        # binary path, and a completion sentinel after jm returns.
        printed = subprocess.run([str(JM), "--print-keybind"], env=clean_env(),
                                 capture_output=True, text=True, timeout=5, check=True).stdout
        binding = next(line for line in printed.splitlines() if line.startswith("bind-key "))
        require("run-shell" in binding, "keybind must expand launching client in run-shell")
        for action in ("cancel", "switch"):
            before = len(first.out)
            marker = "@jm-popup-" + action
            config = binding.replace("tmux display-popup", "tmux -L " + shlex.quote(sock.name) + " display-popup")
            tail = shlex.quote(str(JM)) + " -L " + shlex.quote(sock.name) + "; tmux -L " + shlex.quote(sock.name) + " set-option -g " + marker + " done"
            config = config.replace(' jm"', ' ' + tail + '"')
            with tempfile.NamedTemporaryFile(mode="w", suffix=".tmux.conf", delete=False) as file:
                file.write(config + "\n")
                config_path = file.name
            try:
                sock.run("source-file", config_path)
            finally:
                os.unlink(config_path)
            first.send("\x02j")
            first.until(lambda: b"JustMux" in first.out[before:], "popup jm did not render")
            if action == "cancel":
                first.send("\x1b")
            else:
                first.send("/resume\r")
                first.until(lambda: b"popup-target" in first.out[before:], "popup picker did not render")
                first.send("popup-target\r")
            first.until(lambda: sock.run("show-option", "-gv", marker, check=False).strip() == "done", f"popup did not close after {action}")
            clients = sock.clients()
            require(clients[first.pid][1] == ("popup-source" if action == "cancel" else "popup-target"), f"wrong target after {action}: {clients}")
            require(clients[second.pid][1] == "popup-source", "second client was unexpectedly switched")
            require(int(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_pid}")) == source_pid, "source process PID changed")
            os.kill(source_pid, 0)
            require(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_current_command}") == "sleep", "source foreground sleep was interrupted")
        first.send("\x02d"); first.wait_exit()
        second.send("\x02d"); second.wait_exit()
    print("PASS popup cancel + /resume switch: source sleep PID survives; second client unchanged")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cast", default=str(ROOT / "assets/demo.cast"))
    args = parser.parse_args()
    require(JM.exists(), "build first: cargo build --release")
    sock = Socket()
    try:
        external(sock)
        create(sock)
        interrupt(sock, "Ctrl+C")
        interrupt(sock, "SIGTERM")
        popup(sock)
    finally:
        sock.cleanup()
    header = {"version": 2, "width": 100, "height": 30, "timestamp": int(time.time()), "env": {"TERM": "xterm-256color"}}
    with open(args.cast, "w", encoding="utf-8") as file:
        file.write(json.dumps(header) + "\n")
        for record in RECORDS:
            file.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(f"Real PTY capture: {args.cast}")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"PTY smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
