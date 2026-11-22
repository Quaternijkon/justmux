#!/usr/bin/env python3
"""Offline npm update-picker PTY checks; no real npm, network or tmux is used.

Run from any directory: python3 scripts/test-npm-update-pty.py
--entry PATH copies that installed npm entry's package into a temporary global
prefix before testing it, so neither an installation nor user caches are edited.
The real entry, updater and launcher run; native execution and npm are fixtures.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("justmux_update_pty_support", ROOT / "scripts/pty_smoke.py")
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


class Fixture:
    def __init__(self, directory, source_entry, node):
        self.directory = directory
        self.node = node
        source_entry = source_entry.resolve(strict=True)
        source_root = source_entry.parent.parent
        manifest = json.loads((source_root / "package.json").read_text())
        self.name, self.version = manifest["name"], manifest["version"]
        self.latest = "999.0.0"
        self.prefix = directory / "global npm prefix with spaces"
        self.package = self.prefix / "lib/node_modules" / self.name
        shutil.copytree(source_root, self.package, ignore=shutil.ignore_patterns("node_modules", "__pycache__"))
        self.entry = self.package / source_entry.relative_to(source_root)
        runtime = json.loads(subprocess.check_output(
            [node, "-p", "JSON.stringify({platform:process.platform,arch:process.arch,execve:typeof process.execve})"],
            text=True, timeout=5))
        require(runtime["execve"] == "function", "Node.js with process.execve is required (22.15+)")
        require((runtime["platform"], runtime["arch"]) in (("darwin", "arm64"), ("linux", "x64")),
                "Update PTY checks require a supported npm platform: macOS arm64 or Linux x64")
        native_name = f"{self.name}-{runtime['platform']}-{runtime['arch']}"
        native = self.prefix / "lib/node_modules" / native_name
        (native / "bin").mkdir(parents=True)
        (native / "package.json").write_text(json.dumps({"name": native_name, "version": self.version}))
        self.write_executable(native / "bin/jm", """
import json, os, sys, termios
attrs = termios.tcgetattr(0)
record = {"pid": os.getpid(), "argv": sys.argv[1:],
          "canonical": bool(attrs[3] & termios.ICANON), "echo": bool(attrs[3] & termios.ECHO)}
with open(os.environ["JUSTMUX_TEST_NATIVE_LOG"], "w") as stream:
    json.dump(record, stream)
print("NATIVE_FIXTURE", flush=True)
""")
        self.tools = directory / "fake tools"
        self.tools.mkdir()
        self.write_executable(self.tools / "npm", """
import json, os, sys, termios
attrs = termios.tcgetattr(0)
record = {"argv": sys.argv[1:], "canonical": bool(attrs[3] & termios.ICANON),
          "echo": bool(attrs[3] & termios.ECHO)}
with open(os.environ["JUSTMUX_TEST_NPM_LOG"], "w") as stream:
    json.dump(record, stream)
print("NPM_FIXTURE", flush=True)
sys.exit(int(os.environ.get("JUSTMUX_TEST_NPM_STATUS", "0")))
""")
        self.preload = directory / "deny-network.cjs"
        self.preload.write_text("""'use strict';
globalThis.fetch = async () => {
  require('node:fs').writeFileSync(process.env.JUSTMUX_TEST_NETWORK_LOG, 'forbidden fetch');
  throw new Error('Network disabled by the offline update PTY test');
};
""")

    @staticmethod
    def write_executable(path, body):
        path.write_text(f"#!{sys.executable}\n{body.lstrip()}")
        path.chmod(0o755)

    def start(self, label, npm_status=0):
        run = self.directory / label
        run.mkdir()
        cache = run / "cache/justmux/update.json"
        cache.parent.mkdir(parents=True)
        cache.write_text(json.dumps({"version": self.latest, "time": int(time.time() * 1000)}))
        env = os.environ.copy()
        for name in ("TMUX", "TMUX_PANE", "JUSTMUX_CLIENT", "JUSTMUX_PANE", "JUSTMUX_POPUP", "TMUX_POPUP",
                     "CI", "JUSTMUX_NO_UPDATE_CHECK", "NO_UPDATE_CHECK", "NODE_OPTIONS"):
            env.pop(name, None)
        env.update({
            "TERM": "xterm-256color",
            "XDG_CACHE_HOME": str(run / "cache"),
            "PATH": str(self.tools) + os.pathsep + env.get("PATH", ""),
            "JUSTMUX_TEST_NATIVE_LOG": str(run / "native.json"),
            "JUSTMUX_TEST_NPM_LOG": str(run / "npm.json"),
            "JUSTMUX_TEST_NETWORK_LOG": str(run / "network.txt"),
            "JUSTMUX_TEST_NPM_STATUS": str(npm_status),
        })
        support.clean_env = lambda: env.copy()
        terminal = support.PTY([self.node, "--require", str(self.preload), str(self.entry)])
        return terminal, run, cache

    def verify(self, terminal, run, cache, action, code=0):
        try:
            terminal.wait_exit()
        except RuntimeError as error:
            raise RuntimeError(f"{error}; transcript={bytes(terminal.out)!r}") from error
        require(os.waitstatus_to_exitcode(terminal.status) == code,
                f"unexpected exit status {terminal.status}: {bytes(terminal.out)!r}")
        terminal.assert_restored()
        require(not (run / "network.txt").exists(), "an update check attempted network access")
        native_file, npm_file = run / "native.json", run / "npm.json"
        require(native_file.exists() == (action in ("skip", "skip-version", "failed")),
                f"unexpected native handoff for {action}")
        require(npm_file.exists() == (action in ("update", "failed")), f"unexpected npm execution for {action}")
        if native_file.exists():
            native = json.loads(native_file.read_text())
            require(native["pid"] == terminal.original_pid, "Node supervised native fixture instead of exec replacement")
            require(native["argv"] == [], "update picker altered native arguments")
            require(native["canonical"] and native["echo"], "native executable inherited raw terminal mode")
        if npm_file.exists():
            npm = json.loads(npm_file.read_text())
            require(npm["argv"] == ["install", "--global", "--prefix", str(self.prefix),
                                     f"{self.name}@{self.latest}", "--include=optional"],
                    f"npm argv did not preserve exact version/prefix: {npm['argv']!r}")
            require(npm["canonical"] and npm["echo"], "npm inherited raw terminal mode")
        saved = json.loads(cache.read_text())
        require(saved.get("dismissedVersion") == (self.latest if action == "skip-version" else None),
                f"unexpected persistent dismissal for {action}: {saved}")

    def case(self, label, keys, action, code=0, signal_number=None, narrow=False, npm_status=0):
        terminal, run, cache = self.start(label, npm_status)
        with terminal:
            terminal.text("JustMux update available")
            require(b"\x1b[7m> 1. Update" in terminal.out, "Update was not highlighted initially")
            if narrow:
                terminal.resize(14, 4)
                os.kill(terminal.pid, signal.SIGWINCH)
                terminal.read(.1)
            if signal_number is not None:
                os.kill(terminal.pid, signal_number)
            else:
                os.write(terminal.fd, keys)
            self.verify(terminal, run, cache, action, code)
        print(f"PASS {label}", flush=True)

    def letters_case(self):
        terminal, run, cache = self.start("yes-no-letters-ignored")
        with terminal:
            terminal.text("JustMux update available")
            for text in (b"yes", b"no"):
                os.write(terminal.fd, text)
                terminal.read(.12)
                pid, _status = os.waitpid(terminal.pid, os.WNOHANG)
                require(pid == 0, "yes/no letters unexpectedly closed the update selector")
                require(not (run / "npm.json").exists(), "yes/no letters unexpectedly ran npm")
                require(not (run / "native.json").exists(), "yes/no letters unexpectedly resumed the application")
            os.write(terminal.fd, b"\x1b")
            self.verify(terminal, run, cache, "skip")
        print("PASS yes-no-letters-ignored", flush=True)

    def explicit_update_case(self, label, latest, expect_install):
        run = self.directory / label
        run.mkdir()
        cache = run / "cache/justmux/update.json"
        cache.parent.mkdir(parents=True)
        cache.write_text(
            json.dumps(
                {
                    "version": self.latest,
                    "dismissedVersion": self.latest,
                    "time": int(time.time() * 1000),
                }
            )
        )
        preload = run / "mock-fetch.cjs"
        preload.write_text(
            "'use strict';\n"
            "globalThis.fetch = async (url) => {\n"
            "  require('node:fs').writeFileSync(process.env.JUSTMUX_TEST_FETCH_LOG, String(url));\n"
            "  const body = JSON.stringify({ name: process.env.JUSTMUX_TEST_PKG, version: process.env.JUSTMUX_TEST_LATEST });\n"
            "  return new Response(body, {\n"
            "    status: 200,\n"
            "    headers: { 'content-type': 'application/json', 'content-length': String(Buffer.byteLength(body)) },\n"
            "  });\n"
            "};\n"
        )
        env = os.environ.copy()
        for name in (
            "TMUX",
            "TMUX_PANE",
            "JUSTMUX_CLIENT",
            "JUSTMUX_PANE",
            "JUSTMUX_POPUP",
            "TMUX_POPUP",
            "NODE_OPTIONS",
        ):
            env.pop(name, None)
        env.update({
            "TERM": "xterm-256color",
            "CI": "1",
            "JUSTMUX_NO_UPDATE_CHECK": "1",
            "XDG_CACHE_HOME": str(run / "cache"),
            "PATH": str(self.tools) + os.pathsep + env.get("PATH", ""),
            "JUSTMUX_TEST_NATIVE_LOG": str(run / "native.json"),
            "JUSTMUX_TEST_NPM_LOG": str(run / "npm.json"),
            "JUSTMUX_TEST_FETCH_LOG": str(run / "fetch.txt"),
            "JUSTMUX_TEST_PKG": self.name,
            "JUSTMUX_TEST_LATEST": latest,
            "JUSTMUX_TEST_NPM_STATUS": "0",
        })
        support.clean_env = lambda: env.copy()
        with support.PTY([self.node, "--require", str(preload), str(self.entry), "update"]) as terminal:
            terminal.wait_exit()
            require(
                os.waitstatus_to_exitcode(terminal.status) == 0,
                f"unexpected explicit update status {terminal.status}: {bytes(terminal.out)!r}",
            )
            require(
                support.termios.tcgetattr(terminal.fd) == terminal.baseline,
                "terminal termios changed during explicit update",
            )
            terminal.assert_native_screen()
            require(b"\x1b[?25l" not in terminal.out, "explicit update hid cursor")
            out_text = bytes(terminal.out).decode("utf-8", errors="replace")
            require(
                "JustMux update available" not in out_text,
                f"explicit update unexpectedly showed passive picker: {out_text!r}",
            )
            require((run / "fetch.txt").exists(), "explicit update bypassed fetch")
            require(not (run / "native.json").exists(), "explicit update launched native binary")
            if expect_install:
                require(f"Updating JustMux {self.version} -> {latest}" in out_text, out_text)
                require(f"JustMux updated to {latest}." in out_text, out_text)
                npm_file = run / "npm.json"
                require(npm_file.exists(), "explicit update did not run npm install")
                npm = json.loads(npm_file.read_text())
                require(
                    npm["argv"] == [
                        "install",
                        "--global",
                        "--prefix",
                        str(self.prefix),
                        f"{self.name}@{latest}",
                        "--include=optional",
                    ],
                    f"unexpected npm argv: {npm['argv']!r}",
                )
                saved = json.loads(cache.read_text())
                require("dismissedVersion" not in saved, f"explicit update kept dismissedVersion: {saved}")
            else:
                require(f"JustMux {self.version} is already up to date." in out_text, out_text)
                require(not (run / "npm.json").exists(), "up-to-date check unexpectedly ran npm install")
        print(f"PASS {label}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--entry", type=Path, default=ROOT / "npm/justmux/bin/justmux.cjs",
                        help="npm entry whose package will be copied to a private global prefix")
    parser.add_argument("--node", default=shutil.which("node"), help="Node.js executable (22.15+)")
    args = parser.parse_args()
    require(bool(args.node), "Node.js is not installed")
    with tempfile.TemporaryDirectory(prefix="justmux-npm-update-pty-") as temporary:
        fixture = Fixture(Path(temporary).resolve(), args.entry, args.node)
        fixture.case("default-enter-updates", b"\r", "update")
        fixture.case("arrow-enter-skips", b"\x1b[B\r", "skip")
        fixture.case("arrow-wrap-skips-version", b"\x1b[A\r", "skip-version")
        fixture.case("j-k-navigation", b"jjk\r", "skip")
        fixture.case("digit-1-updates", b"1", "update")
        fixture.case("digit-2-skips", b"2", "skip")
        fixture.case("digit-3-skips-version", b"3", "skip-version")
        fixture.case("escape-skips", b"\x1b", "skip")
        fixture.case("ctrl-d-skips", b"\x04", "skip")
        fixture.case("ctrl-c-restores", b"\x03", "exit", code=130)
        fixture.case("sigterm-restores", b"", "exit", code=143, signal_number=signal.SIGTERM)
        fixture.case("sighup-restores", b"", "exit", code=129, signal_number=signal.SIGHUP)
        fixture.case("sigquit-restores", b"", "exit", code=131, signal_number=signal.SIGQUIT)
        fixture.case("narrow-resize-still-selects", b"\x1b[A\r", "skip-version", narrow=True)
        fixture.case("failed-update-continues", b"1", "failed", npm_status=7)
        fixture.letters_case()
        fixture.explicit_update_case("explicit-update-installs-without-picker", fixture.latest, True)
        fixture.explicit_update_case("explicit-update-up-to-date", fixture.version, False)
    print("All 18 offline npm update PTY scenarios passed.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
