#!/usr/bin/env python3
"""Real PTY smoke checks for JustMux.

All tmux work uses an isolated socket.  The script is deliberately bounded: every
child has a deadline and is terminated/reaped in finally blocks.
"""
from __future__ import annotations
import argparse, os, pty, selectors, signal, subprocess, sys, time, json, uuid, re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
JM = ROOT / "target" / "release" / "jm"

class PTY:
    def __init__(self, argv, env, cols=100, rows=30):
        self.argv, self.env = argv, env
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execvpe(argv[0], argv, env)
        import fcntl
        fcntl.fcntl(self.fd, fcntl.F_SETFL, os.O_NONBLOCK)
        import termios, struct
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.sel = selectors.DefaultSelector(); self.sel.register(self.fd, selectors.EVENT_READ)
        self.out = bytearray(); self.started = time.monotonic()
    def read(self, seconds=0.2):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            for _, _ in self.sel.select(max(0, end-time.monotonic())):
                try: self.out.extend(os.read(self.fd, 65536))
                except (BlockingIOError, OSError): pass
        return bytes(self.out)
    def send(self, text):
        for ch in text:
            os.write(self.fd, ch.encode())
            time.sleep(0.015)
    def wait_for(self, needle, timeout=5):
        deadline = time.monotonic() + timeout
        n = needle.encode() if isinstance(needle, str) else needle
        while time.monotonic() < deadline:
            clean = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", bytes(self.out))
            if n in self.out or n in clean: return True
            self.read(.1)
        return False
    def wait_exit(self, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            got, _ = os.waitpid(self.pid, os.WNOHANG)
            if got:
                self.pid = -1
                return True
            self.read(.05)
        return False
    def terminate(self):
        if self.pid <= 0: return
        try: os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError: return
        deadline = time.monotonic()+2
        while time.monotonic()<deadline:
            got, _ = os.waitpid(self.pid, os.WNOHANG)
            if got: self.pid=-1; return
            self.read(.05)
        try: os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(self.pid, 0)
        except ChildProcessError: pass
        self.pid=-1
    def close(self):
        try: self.sel.close(); os.close(self.fd)
        except OSError: pass
    def __enter__(self): return self
    def __exit__(self, *_): self.terminate(); self.close()

class Socket:
    def __init__(self): self.name = f"jm-smoke-{os.getpid()}-{uuid.uuid4().hex[:8]}"
    def run(self, *args, check=True):
        env = os.environ.copy(); env.pop("TMUX", None); env.pop("TMUX_PANE", None)
        p = subprocess.run(["tmux", "-L", self.name, "-f", "/dev/null", *args], env=env,
                           text=True, capture_output=True, timeout=5)
        if check and p.returncode: raise RuntimeError(p.stderr.strip() or "tmux failed")
        return p
    def cleanup(self):
        try: self.run("kill-server", check=False)
        except Exception: pass

def env_for(sock):
    e=os.environ.copy(); e.pop("TMUX", None); e.pop("TMUX_PANE", None)
    e["TERM"]="xterm-256color"; e["JUSTMUX_SMOKE"]="1"; return e

def smoke_external(sock, cast):
    sock.run("new-session", "-d", "-s", "orion", "-c", str(ROOT))
    with PTY([str(JM), "-L", sock.name], env_for(sock)) as p:
        if not p.wait_for("JustMux", 5): raise RuntimeError("TUI did not start")
        p.send("/resume\r")
        if not p.wait_for("orion", 5):
            raise RuntimeError("/resume picker did not render")
        if not p.wait_for("orion", 5): raise RuntimeError("session was not listed")
        p.send("\r")
        p.read(.5)
        p.send("printf JM_NATIVE_ATTACH\\n\r"); p.read(.4); p.send("\x02d")
        if not p.wait_for("JM_NATIVE_ATTACH", 1):
            raise RuntimeError("native tmux command did not run")
        if not p.wait_exit(5):
            raise RuntimeError("native attach did not return after detach")
        cast.append(bytes(p.out))

def smoke_new_quit(sock, cast):
    with PTY([str(JM), "-L", sock.name], env_for(sock)) as p:
        p.read(.3); p.send("/new\r"); p.read(.2); p.send("smoke-new\r"); p.read(.2)
        p.send((str(ROOT) + "\r")); p.read(.4)
        p.send("\x1b\x1b"); p.read(.2)
        p.send("/quit\r")
        if not p.wait_exit(5):
            raise RuntimeError("/quit did not return from the home view")
        cast.append(bytes(p.out))

def smoke_popup_switch(sock, cast):
    sock.run("new-session", "-d", "-s", "popup-source", "-c", str(ROOT), "sleep", "30")
    sock.run("new-session", "-d", "-s", "popup-target", "-c", str(ROOT))
    env = env_for(sock)
    with PTY(["tmux", "-L", sock.name, "-f", "/dev/null", "attach-session", "-t", "popup-source"], env) as p:
        p.read(.8)
        client = sock.run("list-clients", "-F", "#{client_name}").stdout.strip()
        if not client:
            raise RuntimeError("private tmux client name was not reported")
        command = (
            "JUSTMUX_CLIENT=$(tmux display-message -p \"#{client_name}\") "
            "JUSTMUX_PANE=$(tmux display-message -p \"#{pane_id}\") "
            "JUSTMUX_POPUP=1 exec " + str(JM) + " -L " + sock.name
        )
        popup = subprocess.Popen(
            ["tmux", "-L", sock.name, "-f", "/dev/null", "display-popup", "-E", "-t", client, command],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            if not p.wait_for("JustMux", 5):
                raise RuntimeError("popup TUI did not start")
            p.send("/resume\r")
            if not p.wait_for("popup-target", 5):
                raise RuntimeError("popup resume picker did not render")
            p.send("\x1b[B")
            p.send("\r")
            deadline = time.monotonic() + 8
            while popup.poll() is None and time.monotonic() < deadline:
                p.read(.1)
            if popup.poll() is None:
                popup.kill(); popup.wait(timeout=2)
                raise RuntimeError("popup did not close after switch")
            if popup.returncode != 0:
                raise RuntimeError("display-popup returned a failure")
            current = sock.run("list-clients", "-F", "#{client_session}").stdout.strip()
            if current != "popup-target":
                raise RuntimeError(f"popup switched to unexpected session: {current!r}")
            pane_command = sock.run("list-panes", "-t", "popup-source", "-F", "#{pane_current_command}").stdout.strip()
            if pane_command not in {"sleep", "zsh", "bash", "sh"}:
                raise RuntimeError("the source pane did not remain alive")
            cast.append(bytes(p.out))
        finally:
            if popup.poll() is None:
                popup.kill(); popup.wait(timeout=2)

def main():
    ap=argparse.ArgumentParser(); ap.add_argument("--cast", default=str(ROOT/"assets"/"demo.cast")); args=ap.parse_args()
    if not JM.exists():
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    cast=[]; sock=Socket(); started=time.monotonic()
    try:
        smoke_external(sock, cast); sock.cleanup(); smoke_new_quit(sock, cast)
        sock.cleanup(); smoke_popup_switch(sock, cast)
        print("PTY smoke passed: external attach/detach, /new/create/quit, and popup switch")
    finally: sock.cleanup()
    header={"version":2,"width":100,"height":30,"timestamp":int(time.time()),"env":{"TERM":"xterm-256color","SHELL":os.environ.get("SHELL","/bin/zsh")}}
    with open(args.cast,"w",encoding="utf-8") as f:
        f.write(json.dumps(header,separators=(",",":"))+"\n")
        now=0.0
        for chunk in cast:
            if isinstance(chunk,int): continue
            text=chunk.decode("utf-8","replace")
            if text: now += 0.01; f.write(json.dumps([round(now,3),"o",text],ensure_ascii=False,separators=(",",":"))+"\n")
    return 0

if __name__ == "__main__":
    try: raise SystemExit(main())
    except Exception as e:
        print(f"pty smoke failed: {e}", file=sys.stderr); raise SystemExit(1)
