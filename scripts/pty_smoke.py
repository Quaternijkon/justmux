#!/usr/bin/env python3
"""Bounded, real PTY regression checks; every tmux command uses a private socket."""
from __future__ import annotations
import argparse
import codecs
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
import unicodedata
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


class Screen:
    """Small ANSI screen oracle for CPR and the direct-jm preservation checks.

    This is test-only terminal emulation: the application still uses the real
    PTY. We answer actual cursor reports just as a terminal emulator would.
    """
    def __init__(self, rows=30, cols=100):
        self.rows, self.cols = rows, cols
        self.row = self.col = 0
        self.cells = [[" "] * cols for _ in range(rows)]
        self.history = []
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.reports = 0

    def newline(self):
        self.row += 1
        if self.row >= self.rows:
            self.history.append("".join(self.cells.pop(0)))
            self.cells.append([" "] * self.cols)
            self.row = self.rows - 1

    def feed(self, chunk):
        self.pending += self.decoder.decode(chunk)
        replies = []
        i = 0
        while i < len(self.pending):
            char = self.pending[i]
            if char == "\x1b":
                if i + 1 >= len(self.pending):
                    break
                if self.pending[i + 1] == "[":
                    end = i + 2
                    while end < len(self.pending) and not ("@" <= self.pending[end] <= "~"):
                        end += 1
                    if end >= len(self.pending):
                        break
                    value, command = self.pending[i + 2:end], self.pending[end]
                    private = value.startswith("?")
                    parts = value.lstrip("?").split(";")
                    numbers = [int(part) if part.isdecimal() else 0 for part in parts]
                    n = numbers[0] or 1
                    if command in "Hf":
                        self.row = min(self.rows - 1, n - 1)
                        self.col = min(self.cols - 1, (numbers[1] or 1) - 1 if len(numbers) > 1 else 0)
                    elif command == "A": self.row = max(0, self.row - n)
                    elif command == "B": self.row = min(self.rows - 1, self.row + n)
                    elif command == "C": self.col = min(self.cols - 1, self.col + n)
                    elif command == "D": self.col = max(0, self.col - n)
                    elif command == "G": self.col = min(self.cols - 1, n - 1)
                    elif command == "d": self.row = min(self.rows - 1, n - 1)
                    elif command == "E": self.row, self.col = min(self.rows - 1, self.row + n), 0
                    elif command == "F": self.row, self.col = max(0, self.row - n), 0
                    elif command == "J":
                        mode = numbers[0]
                        if mode in (2, 3): self.cells = [[" "] * self.cols for _ in range(self.rows)]
                        elif mode == 0:
                            self.cells[self.row][self.col:] = [" "] * (self.cols - self.col)
                            for row in range(self.row + 1, self.rows): self.cells[row] = [" "] * self.cols
                    elif command == "K":
                        mode = numbers[0]
                        start, end_col = (0, self.cols) if mode == 2 else ((0, self.col + 1) if mode == 1 else (self.col, self.cols))
                        self.cells[self.row][start:end_col] = [" "] * (end_col - start)
                    elif command == "n" and numbers[0] == 6 and not private:
                        replies.append(f"\x1b[{self.row + 1};{min(self.col, self.cols - 1) + 1}R".encode())
                        self.reports += 1
                    i = end + 1
                    continue
                if self.pending[i + 1] in "]P":
                    end = i + 2
                    while end < len(self.pending) and self.pending[end] != "\x07" and self.pending[end:end + 2] != "\x1b\\": end += 1
                    if end >= len(self.pending): break
                    i = end + (1 if self.pending[end] == "\x07" else 2)
                    continue
                i += 2
                continue
            if char == "\r": self.col = 0
            elif char == "\n": self.newline()
            elif char == "\b": self.col = max(0, self.col - 1)
            elif char == "\t": self.col = min(self.cols - 1, (self.col // 8 + 1) * 8)
            elif ord(char) >= 32 and not unicodedata.combining(char):
                width = 2 if unicodedata.east_asian_width(char) in "WF" else 1
                if self.col + width > self.cols:
                    self.col = 0
                    self.newline()
                self.cells[self.row][self.col] = char
                self.col += width
            i += 1
        self.pending = self.pending[i:]
        return replies

    def text(self):
        return "\n".join(self.history + ["".join(row) for row in self.cells])


class PTY:
    def __init__(self, argv, prefix=b""):
        # Synchronise before exec so baseline termios and size cannot race jm.
        ready_r, ready_w = os.pipe()
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.close(ready_w)
            os.read(ready_r, 1)
            os.close(ready_r)
            if prefix:
                os.write(1, prefix)
            os.execvpe(argv[0], argv, clean_env())
        os.close(ready_r)
        self.original_pid = self.pid
        self.baseline = termios.tcgetattr(self.fd)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        os.set_blocking(self.fd, False)
        self.sel = selectors.DefaultSelector()
        self.sel.register(self.fd, selectors.EVENT_READ)
        self.out = bytearray()
        self.screen = Screen()
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
                        for reply in self.screen.feed(chunk):
                            os.write(self.fd, reply)
                        RECORDS.append([round(time.monotonic() - START, 4), "o", chunk.decode("utf-8", "replace")])
                except (BlockingIOError, OSError):
                    pass
        return bytes(self.out)

    def send(self, text):
        for char in text:
            os.write(self.fd, char.encode())
            time.sleep(.015)

    def resize(self, cols, rows):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.screen.cols, self.screen.rows = cols, rows
        self.screen.cells = [(row + [" "] * cols)[:cols] for row in self.screen.cells[:rows]]
        self.screen.cells.extend([[" "] * cols for _ in range(rows - len(self.screen.cells))])
        self.screen.row = min(self.screen.row, rows - 1)
        self.screen.col = min(self.screen.col, cols - 1)

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

    def assert_restored(self, fullscreen=False, native_tmux=False):
        require(termios.tcgetattr(self.fd) == self.baseline, "terminal termios was not fully restored")
        require(b"\x1b[?25h" in self.out, "cursor Show sequence missing")
        if fullscreen or native_tmux:
            require(b"\x1b[?1049l" in self.out, "LeaveAlternateScreen sequence missing")
        else:
            self.assert_native_screen()

    def assert_native_screen(self):
        for forbidden in (b"\x1b[?1049h", b"\x1b[?1049l", b"\x1b[2J", b"\x1b[3J",
                          b"\x1b[?1000h", b"\x1b[?1002h", b"\x1b[?1003h", b"\x1b[?1006h"):
            require(forbidden not in self.out, f"inline output changed native terminal mode: {forbidden!r}")

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


def inline_terminal(sock):
    prefix = b"JM_NATIVE_HISTORY_1\r\nJM_NATIVE_HISTORY_2\r\nJM_NATIVE_PROMPT\r\n"
    command = (shlex.quote(str(JM)) + " -L " + shlex.quote(sock.name)
               + '; status=$?; printf "JM_SHELL_RETURN:%s\\n" "$status"')
    with PTY(["/bin/sh", "-c", command], prefix=prefix) as p:
        p.text("JustMux")
        require(p.screen.reports >= 1, "inline startup never queried the native cursor")
        require("JM_NATIVE_HISTORY_1" in p.screen.text(), "inline startup overwrote existing terminal history")
        title_rows = [i for i, row in enumerate(p.screen.cells) if "JustMux" in "".join(row)]
        require(title_rows and min(title_rows) >= 3, "inline UI was not anchored below the original prompt")
        p.assert_native_screen()
        before = len(p.out)
        p.read(.25)
        require(len(p.out) == before, "idle inline UI continuously writes terminal output")
        p.send("/")
        p.read(.2)
        before = len(p.out)
        os.write(p.fd, b"\x1b[B" * 10)
        p.read(.2)
        navigation_bytes = len(p.out) - before
        require("/quit" in p.screen.text(), "inline navigation did not make the selected command visible")
        reports = p.screen.reports
        p.resize(80, 28)
        p.until(lambda: p.screen.reports > reports, "inline resize did not query the resized cursor")
        p.read(.15)
        require("JM_NATIVE_HISTORY_1" in p.screen.text(), "inline resize overwrote existing terminal history")
        p.assert_native_screen()
        p.send("\x1b")
        p.read(.15)
        # The home footer is the last visible UI row before exit.
        footer_rows = [i for i, row in enumerate(p.screen.cells) if "Type / for commands" in "".join(row)]
        require(footer_rows, "inline footer did not return after cancel")
        ui_bottom = max(footer_rows)
        p.send("\x1b")
        p.text("JM_SHELL_RETURN:0")
        p.wait_exit()
        p.assert_restored()
        shell_rows = [i for i, row in enumerate(p.screen.cells) if "JM_SHELL_RETURN:0" in "".join(row)]
        require(shell_rows and min(shell_rows) > ui_bottom, "shell resumed over the inline UI instead of below it")
    print(f"PASS inline: native history + non-top cursor + resize + shell placement; idle 0 bytes, 10-key burst {navigation_bytes} bytes")


def fullscreen(sock):
    with PTY([str(JM), "--fullscreen", "-L", sock.name]) as p:
        p.text("JustMux")
        require(b"\x1b[?1049h" in p.out, "--fullscreen did not enter alternate screen")
        p.send("\x1b")
        p.wait_exit()
        require(os.waitstatus_to_exitcode(p.status) == 0, "fullscreen quit failed")
        p.assert_restored(fullscreen=True)
    print("PASS --fullscreen: explicit alternate screen entry/exit and termios restored")


def inline_bottom(sock):
    prefix = b"JM_DEEP_HISTORY\r\n" + b"".join(
        f"JM_BEFORE_LINE_{index:02d}\r\n".encode() for index in range(40))
    command = (shlex.quote(str(JM)) + " -L " + shlex.quote(sock.name)
               + '; status=$?; printf "JM_BOTTOM_RETURN:%s\\n" "$status"')
    with PTY(["/bin/sh", "-c", command], prefix=prefix) as p:
        p.text("JustMux")
        require("JM_DEEP_HISTORY" in "\n".join(p.screen.history), "bottom startup lost the native scrollback sentinel")
        title_rows = [i for i, row in enumerate(p.screen.cells) if "JustMux" in "".join(row)]
        require(title_rows and min(title_rows) == 14, "bottom startup did not reserve a 16-row inline viewport")
        p.assert_native_screen()
        for cols, rows in ((70, 10), (100, 30)):
            reports = p.screen.reports
            p.resize(cols, rows)
            p.until(lambda: p.screen.reports > reports, f"resize to {cols}x{rows} did not reposition the inline viewport")
            p.read(.15)
            require("JM_DEEP_HISTORY" in p.screen.text(), "inline shrink/grow erased native scrollback")
            require("JustMux" in "\n".join("".join(row) for row in p.screen.cells), "inline title disappeared after shrink/grow")
            p.assert_native_screen()
        p.send("\x1b")
        p.text("JM_BOTTOM_RETURN:0")
        p.wait_exit()
        p.assert_restored()
        rows = ["".join(row) for row in p.screen.cells]
        footer = next((i for i, row in enumerate(rows) if "Type / for commands" in row), None)
        shell = next((i for i, row in enumerate(rows) if "JM_BOTTOM_RETURN:0" in row), None)
        require(footer is not None and shell is not None and shell > footer, "shell did not resume below the resized inline UI")
        require("JM_DEEP_HISTORY" in p.screen.text(), "exiting inline lost the preexisting scrollback")
    print("PASS bottom inline: 16-row viewport, native scrollback, 10-row shrink/grow and shell placement")


def external(sock):
    sock.run("new-session", "-d", "-s", "orion", "/bin/sh")
    sock.run("new-session", "-d", "-s", "lyra", "/bin/sh")
    shared = sock.run("new-window", "-d", "-P", "-F", "#{window_id}", "-t", "orion:1", "-n", "workspace", "/bin/sh")
    sock.run("link-window", "-d", "-s", shared, "-t", "lyra:3")
    with PTY([str(JM), "-L", sock.name]) as p:
        p.text("JustMux")
        p.send("/resume\r")
        p.until(lambda: any("orion" in row and "lyra" in row for row in p.screen.text().splitlines()),
                "session headers were not displayed side by side")
        p.assert_native_screen()
        pid = p.pid
        p.send("lyra")
        os.write(p.fd, b"\x1b[B")
        p.until(lambda: any("3 workspace" in row and ("›" in row or ">" in row)
                            for row in p.screen.text().splitlines()), "linked window was not selected")
        p.send("\r")
        p.until(lambda: pid in sock.clients(), "jm PID did not become native tmux client")
        require(sock.clients()[pid][1] == "lyra", "linked window attached to its other session")
        require(f"{shared} 1" in sock.run("list-windows", "-t", "=lyra", "-F", "#{window_id} #{window_active}").splitlines(),
                "tmux client did not open the selected window")
        command = subprocess.run(["ps", "-p", str(pid), "-o", "comm="], text=True, capture_output=True, timeout=5).stdout
        require("tmux" in command and "jm" not in Path(command.strip()).name, f"exec evidence failed: {command!r}")
        # An external sentinel proves native input reached the pane, not echoed UI text.
        marker = f"@jm-smoke-{uuid.uuid4().hex[:8]}"
        p.send(f"tmux set-option -g {marker} native\r")
        p.until(lambda: sock.run("show-option", "-gv", marker, check=False) == "native", "native pane command did not execute")
        p.send("\x02d")
        p.wait_exit()
        require(os.waitstatus_to_exitcode(p.status) == 0, "native detach failed")
        p.assert_restored(native_tmux=True)
    print("PASS external /resume: column layout, linked window in chosen session, exec and terminal restoration")


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


def detach(sock):
    sock.run("new-session", "-d", "-s", "detach-source", "/bin/sh")
    sock.run("new-session", "-d", "-s", "detach-other", "/bin/sleep", "300")
    sock.run("link-window", "-s", "detach-source:0", "-t", "detach-other:1")
    sock.run("select-window", "-t", "detach-other:1")
    source_pid = sock.run("list-panes", "-t", "=detach-source", "-F", "#{pane_pid}")
    attach = sock.argv("attach-session", "-t", "=detach-source")
    shell = shlex.join(attach) + '; printf "JM_DETACH_SHELL:%s\\n" "$?"'
    other = sock.argv("attach-session", "-t", "=detach-other")
    with PTY(["/bin/sh", "-c", shell]) as first, PTY(other) as second:
        first.until(lambda: second.pid in sock.clients() and len(sock.clients()) == 2,
                    "the two clients did not attach to the shared window")
        second_name = sock.clients()[second.pid][0]
        record = shlex.join(sock.argv("set-option", "-g", "@jm-detach-result"))
        # PENDIN is a transient input redisplay flag that the OS can clear.
        attributes = shlex.join([sys.executable, "-c", "import termios; attrs = termios.tcgetattr(0); "
                                 "attrs[3] &= ~getattr(termios, 'PENDIN', 0); print(attrs)"])
        launch = (f'saved=$({attributes}); {shlex.quote(str(JM))}; result=$?; '
                  f'if [ "$saved" = "$({attributes})" ]; then {record} "$result:restored"; fi\r')
        os.write(first.fd, launch.encode())
        first.until(lambda: "Type / for commands" in first.screen.text(), "pane jm did not render")
        first.send("/exit\r")
        first.until(lambda: "Cannot identify this tmux client" in first.screen.text(),
                    "/exit did not report the ambiguous clients")
        require(len(sock.clients()) == 2, "ambiguous /exit detached a client")

        sock.run("select-window", "-t", "detach-other:0")
        first.send("/exit")
        first.read(.15)
        os.write(first.fd, b"\r")
        first.text("JM_DETACH_SHELL:0")
        first.wait_exit()
        first.assert_restored(native_tmux=True)
        second.until(lambda: sock.run("show-option", "-gv", "@jm-detach-result", check=False) == "0:restored",
                     "jm did not return with the original pane termios")
        require(sock.clients() == {second.pid: [second_name, "detach-other"]}, "wrong client detached")
        require(sock.run("list-panes", "-t", "=detach-source", "-F", "#{pane_pid}") == source_pid,
                "detach did not preserve the original pane process")
        second.send("\x02d")
        second.wait_exit()
    print("PASS pane /exit: refuses a shared window, detaches the current client, and preserves the pane")

    with PTY([str(JM), "-L", sock.name]) as outside:
        outside.text("JustMux")
        outside.send("/exit\r")
        outside.wait_exit()
        require(os.waitstatus_to_exitcode(outside.status) == 0, "/exit from outside tmux did not quit")
        outside.assert_restored()
    print("PASS standalone /exit: quits JustMux and restores the terminal")


def interrupt(sock, method, fullscreen=False):
    args = [str(JM), "-L", sock.name]
    if fullscreen:
        args.append("--fullscreen")
    with PTY(args) as p:
        p.text("JustMux")
        require(not (termios.tcgetattr(p.fd)[3] & termios.ICANON), "jm did not enter raw mode")
        if method == "Ctrl+C":
            p.send("\x03")
        else:
            os.kill(p.pid, signal.SIGTERM)
        p.wait_exit()
        p.assert_restored(fullscreen=fullscreen)
        expected = 0 if method == "Ctrl+C" else 128 + signal.SIGTERM
        require(os.waitstatus_to_exitcode(p.status) == expected, f"unexpected {method} status: {p.status}")
    screen = "with alternate screen restored" if fullscreen else "without alternate screen"
    print(f"PASS {method}{' --fullscreen' if fullscreen else ''}: full termios and cursor restored {screen}")


def popup(sock):
    sock.run("new-session", "-d", "-s", "popup-source", "/bin/sleep", "300")
    sock.run("new-session", "-d", "-s", "popup-target", "/bin/sh")
    work = sock.run("new-window", "-d", "-P", "-F", "#{window_id}", "-t", "popup-target:1", "-n", "popup-work", "/bin/sh")
    source_pid = int(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_pid}"))
    with PTY(sock.argv("attach-session", "-t", "=popup-source")) as first, PTY(sock.argv("attach-session", "-t", "=popup-source")) as second:
        first.until(lambda: len(sock.clients()) == 2, "two native clients did not attach")
        # Exercise the exact recommended config syntax, adding only test socket,
        # binary path, and a completion sentinel after jm returns.
        printed = subprocess.run([str(JM), "--print-keybind"], env=clean_env(),
                                 capture_output=True, text=True, timeout=5, check=True).stdout
        binding = next(line for line in printed.splitlines() if line.startswith("bind-key "))
        require("run-shell" in binding, "keybind must expand launching client in run-shell")
        for action in ("cancel", "switch", "detach"):
            if action == "detach":
                sock.run("switch-client", "-c", sock.clients()[first.pid][0], "-t", "=popup-source")
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
            elif action == "switch":
                first.send("/resume\r")
                first.until(lambda: b"popup-target" in first.out[before:], "popup picker did not render")
                first.send("popup-work\r")
            else:
                first.send("/exit")
                # Drain redraws before tmux writes its terminal restore codes.
                first.read(.15)
                os.write(first.fd, b"\r")

            if action == "detach":
                first.wait_exit()
                require(os.waitstatus_to_exitcode(first.status) == 0, "popup /exit did not detach normally")
                first.assert_restored(native_tmux=True)
            else:
                first.until(lambda: sock.run("show-option", "-gv", marker, check=False) == "done", f"popup did not close after {action}")
            clients = sock.clients()
            if action == "detach":
                require(first.original_pid not in clients, "popup /exit left its client attached")
            else:
                require(clients[first.pid][1] == ("popup-source" if action == "cancel" else "popup-target"), f"wrong target after {action}: {clients}")
                if action == "switch":
                    require(f"{work} 1" in sock.run("list-windows", "-t", "=popup-target", "-F", "#{window_id} #{window_active}").splitlines(),
                            "popup /resume did not select the requested window")
            require(clients[second.pid][1] == "popup-source", "second client was unexpectedly switched")
            require(int(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_pid}")) == source_pid, "source process PID changed")
            os.kill(source_pid, 0)
            require(sock.run("list-panes", "-t", "=popup-source", "-F", "#{pane_current_command}") == "sleep", "source foreground sleep was interrupted")
        second.send("\x02d"); second.wait_exit()
    print("PASS popup cancel, /resume, and /exit: source sleep PID survives; second client remains attached")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cast", default=str(ROOT / "assets/demo.cast"))
    args = parser.parse_args()
    require(JM.exists(), "build first: cargo build --release")
    sock = Socket()
    try:
        inline_terminal(sock)
        inline_bottom(sock)
        fullscreen(sock)
        external(sock)
        create(sock)
        detach(sock)
        interrupt(sock, "Ctrl+C")
        interrupt(sock, "SIGTERM")
        interrupt(sock, "SIGTERM", fullscreen=True)
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
