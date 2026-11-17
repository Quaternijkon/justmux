# JustMux

**Stop memorizing tmux commands. Just type `/`.**

JustMux is a small native interface to tmux, with slash commands for sessions,
windows, and panes. Both installed commands launch the same application:

```sh
npm install --global justmux@latest
jm
justmux
```

Run the same npm install command to update to the latest release.

The npm package selects a precompiled native binary for macOS or Linux on x64
or arm64. No Rust compiler, installation script, or download on first launch is
required. Keep npm optional dependencies enabled so the matching binary can be
installed. Node.js 22.15.0 or newer is required (Node.js 24 LTS is recommended) to replace the launcher process
with the native executable; JustMux does not leave a Node process supervising tmux.

Install tmux through your system's package manager. tmux is a runtime dependency;
it is not bundled by this npm package. Popup integration requires tmux 3.2+.

Project documentation: https://github.com/Quaternijkon/justmux
