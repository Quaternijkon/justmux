//! JustMux — a modern slash-command interface for tmux.
//!
//! JustMux is a thin, on-demand front end for the user's *existing* tmux
//! server. It never emulates a terminal, never runs a daemon, and never keeps
//! state of its own: everything shown comes from `tmux` and everything done is
//! a native `tmux` command.
//!
//! Layering (dependencies point downwards only):
//!
//! ```text
//! cli        argument parsing, `doctor`, keybind helpers, process entry point
//! app        state machine: composer + view stack, turns key events into tmux calls
//! ui         tmux-agnostic widgets (composer, command popup, picker, dialogs, theme)
//! commands   slash-command registry and parser
//! config     TOML configuration loading, validation, precedence, and persistence
//! fuzzy      fuzzy matching / ranking
//! tmux       typed adapter over the tmux CLI (the only module that spawns tmux)
//! terminal   inline/fullscreen lifecycle, restoration, and exec hand-off to tmux
//! tributes   celebration cards, micro-tributes, and session badges
//! ```

pub mod app;
pub mod cli;
pub mod commands;
pub mod config;
pub mod fuzzy;
pub mod terminal;
pub mod tmux;
pub mod tributes;
pub mod ui;
