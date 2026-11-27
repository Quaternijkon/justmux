# JustMux

**Stop memorizing tmux commands. Just type `/`.**

JustMux is a small native interface to tmux, with slash commands for sessions,
windows, and panes. Both installed commands launch the same application:

```sh
npm install --global justmux@latest
jm
justmux
```

Run `jm update` (or `justmux update`) to check npm and update a global
installation directly, or `jm update --check` to check without installing. You
can also re-run the `npm install` command above.

Global installations also check npm at most once a day when launched
interactively without a command, including `--fullscreen` and `--inline`. A newer version opens
a compact TUI picker with `Update`, `Skip`, and `Skip this version`, in that
order. Use Up/Down and Enter, or press 1, 2, or 3. `Update` is initially selected
and installs that exact version, then exits; run `jm` again afterward. `Skip`
(also Esc) continues this launch. `Skip this version` suppresses reminders for
that version, including on later days; a different release can prompt again.
Ctrl+C cancels startup. The picker restores terminal input and the cursor
before continuing. Checks have a three-second timeout and offline failures do
not block use. Set `JUSTMUX_NO_UPDATE_CHECK=1` to disable them. Help, version,
direct commands, CI, and piped runs skip checks.

The npm package selects a precompiled native binary for macOS ARM64 or Linux
x64. No Rust compiler or download on first launch is required. A local-only
`postinstall` step verifies that `jm` and `justmux` are linked into `<prefix>/bin`
and reachable on your `PATH`; when the global npm `bin` directory is not on
`PATH`, it creates symlinks in a writable standard `PATH` directory (such as
`/usr/local/bin` or `~/.local/bin`) without overwriting unrelated commands.
Set `JUSTMUX_NO_POSTINSTALL_LINK=1` to skip this check, or install with
`--ignore-scripts` when `<prefix>/bin` is already on `PATH`.

Keep npm optional dependencies enabled so the matching binary can be installed.
Node.js 22.15.0 or newer is required (Node.js 24 LTS is recommended) to replace
the launcher process with the native executable; JustMux does not leave a Node
process supervising tmux.

Alternatively, install the standalone native binary without Node.js:

```sh
curl -fsSL https://raw.githubusercontent.com/Quaternijkon/justmux/main/install.sh | sh
```

Install tmux through your system's package manager. tmux is a runtime dependency;
it is not bundled by this npm package. Popup integration requires tmux 3.2+.

Project documentation: https://github.com/Quaternijkon/justmux
