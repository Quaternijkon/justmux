//! Typed adapter over the tmux CLI.
//!
//! This is the only module that spawns `tmux`. The rest of the application
//! talks to the [`Tmux`] trait so it can be driven by a fake in tests.
//!
//! Rules that every implementation detail here must respect:
//!
//! * arguments are always passed as separate argv entries — never through a
//!   shell, never by string concatenation;
//! * objects are targeted by tmux ID (`$1`, `@2`, `%3`), never by name;
//! * data is read with explicit `-F` formats, never by scraping
//!   human-readable output;
//! * no socket flag is passed unless the user asked for one, so tmux's own
//!   `$TMUX` / default-socket selection applies;
//! * nothing here changes tmux options, key bindings or other clients unless
//!   explicitly requested by the user, and `attach-session -d` is never used.

pub mod client;
pub mod error;
pub mod model;
pub mod parser;

use std::path::{Path, PathBuf};
use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub use client::{TmuxClient, sanitize_captured_lines, sanitize_captured_sgr_lines};
pub use error::{TmuxError, TmuxResult};
pub use model::{
    ClientInfo, Context, InsideContext, LayoutPreset, Pane, PaneDimension, PaneId, PaneTargetSize,
    PasteBuffer, ResizeDirection, Session, SessionId, SplitDirection, Target, TmuxVersion, Window,
    WindowId,
};

/// Which tmux server to talk to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SocketSpec {
    /// Let tmux decide (`$TMUX` when inside tmux, otherwise its default socket).
    #[default]
    Default,
    /// `tmux -L <name>`.
    Name(String),
    /// `tmux -S <path>`.
    Path(PathBuf),
}

fn unsupported(command: &str) -> TmuxError {
    TmuxError::CommandFailed {
        command: command.to_string(),
        message: "operation is not implemented by this tmux backend".into(),
    }
}

/// Everything JustMux needs from tmux.
///
/// Conventions:
///
/// * "no server running" is **not** an error for read operations:
///   [`Tmux::list_sessions`] and friends return an empty list and
///   [`Tmux::server_running`] returns `false`;
/// * operations on an object that has disappeared return the matching
///   `*NotFound` error so the UI can say so and refresh;
/// * only [`Tmux::new_session`] may start a tmux server.
pub trait Tmux {
    /// `tmux -V`. [`TmuxError::NotInstalled`] if the binary cannot be found.
    fn version(&self) -> TmuxResult<TmuxVersion>;

    /// Whether a server is reachable on the selected socket.
    fn server_running(&self) -> TmuxResult<bool>;

    /// Where JustMux runs relative to the managed server. Never fails:
    /// stale or unresolvable `$TMUX` context becomes [`Context::Unavailable`]
    /// so entering a session cannot accidentally create a nested client.
    fn context(&self) -> Context;

    // --- sessions -------------------------------------------------------

    fn list_sessions(&self) -> TmuxResult<Vec<Session>>;

    fn has_session(&self, id: &SessionId) -> TmuxResult<bool>;

    /// Create a detached session. Validates `name` (see
    /// [`validate_session_name`]) and that `cwd` is an existing directory
    /// before running tmux. Starts the server if necessary.
    fn new_session(&self, name: &str, cwd: &Path) -> TmuxResult<Session>;

    fn rename_session(&self, id: &SessionId, new_name: &str) -> TmuxResult<()>;

    fn kill_session(&self, id: &SessionId) -> TmuxResult<()>;

    // --- windows --------------------------------------------------------

    /// Windows of one session, or of every session when `session` is `None`.
    fn list_windows(&self, session: Option<&SessionId>) -> TmuxResult<Vec<Window>>;

    /// Create a window in `session` without changing any client's view.
    fn new_window(
        &self,
        session: &SessionId,
        name: Option<&str>,
        cwd: Option<&Path>,
    ) -> TmuxResult<Window>;

    fn rename_window(&self, id: &WindowId, new_name: &str) -> TmuxResult<()>;

    fn kill_window(&self, id: &WindowId) -> TmuxResult<()>;

    /// Move `source` window into `target_session` (`move-window -d -s <src> -t <sess>:`).
    fn move_window(&self, _source: &WindowId, _target_session: &SessionId) -> TmuxResult<()> {
        Err(unsupported("move-window"))
    }

    /// Swap two windows' positions (`swap-window -d -s <src> -t <dst>`).
    fn swap_window(&self, _source: &WindowId, _target: &WindowId) -> TmuxResult<()> {
        Err(unsupported("swap-window"))
    }

    /// Link `source` window into `target_session` (`link-window -d -s <src> -t <sess>:`).
    fn link_window(&self, _source: &WindowId, _target_session: &SessionId) -> TmuxResult<()> {
        Err(unsupported("link-window"))
    }

    /// Apply a built-in layout preset to `window` (`select-layout -t <win> <preset>`).
    fn select_layout(&self, window: &WindowId, layout: LayoutPreset) -> TmuxResult<()> {
        self.select_layout_raw(window, layout.as_tmux_str())
    }

    /// Apply a raw tmux layout string (`#{window_layout}`) to `window`.
    fn select_layout_raw(&self, _window: &WindowId, _layout: &str) -> TmuxResult<()> {
        Err(unsupported("select-layout"))
    }

    /// Enable or disable `synchronize-panes` on `window`.
    fn set_synchronize_panes(&self, _window: &WindowId, _enabled: bool) -> TmuxResult<()> {
        Err(unsupported("set-window-option"))
    }

    /// Enable or disable `monitor-activity` on `window`.
    fn set_monitor_activity(&self, _window: &WindowId, _enabled: bool) -> TmuxResult<()> {
        Err(unsupported("set-window-option"))
    }

    /// Configure `monitor-silence` timeout in seconds on `window` (`0` disables it).
    fn set_monitor_silence(&self, _window: &WindowId, _seconds: u64) -> TmuxResult<()> {
        Err(unsupported("set-window-option"))
    }

    // --- panes ----------------------------------------------------------

    /// Panes of one window, or of every window when `window` is `None`.
    fn list_panes(&self, window: Option<&WindowId>) -> TmuxResult<Vec<Pane>>;

    /// Split `target`, returning the new pane. The new pane starts in the
    /// target pane's current directory.
    fn split_pane(&self, target: &PaneId, direction: SplitDirection) -> TmuxResult<Pane>;

    /// Split `target`, optionally starting the new pane in an explicit `cwd`
    /// (falling back to the target pane's current directory when `None`).
    fn split_pane_in(
        &self,
        target: &PaneId,
        direction: SplitDirection,
        _cwd: Option<&Path>,
    ) -> TmuxResult<Pane> {
        self.split_pane(target, direction)
    }

    /// Move `source` pane into a split alongside or below `target` (`tmux join-pane -d -h|-v`).
    fn join_pane(
        &self,
        source: &PaneId,
        target: &PaneId,
        direction: SplitDirection,
    ) -> TmuxResult<()>;

    /// Move `source` pane into a split adjacent to `target` (`tmux join-pane -d [-b] -h|-v`).
    ///
    /// When `before` is `true`, `source` is docked to the left of (or above) `target`;
    /// when `false`, `source` is docked to the right of (or below) `target`.
    fn join_pane_ordered(
        &self,
        source: &PaneId,
        target: &PaneId,
        direction: SplitDirection,
        _before: bool,
    ) -> TmuxResult<()> {
        self.join_pane(source, target, direction)
    }

    fn kill_pane(&self, id: &PaneId) -> TmuxResult<()>;

    /// Toggle zoom on `id` (`resize-pane -Z -t <id>`).
    fn toggle_zoom_pane(&self, _id: &PaneId) -> TmuxResult<()> {
        Err(unsupported("resize-pane"))
    }

    /// Break pane `id` out into its own detached window (`break-pane -d -P`).
    fn break_pane(&self, _id: &PaneId, _name: Option<&str>) -> TmuxResult<Window> {
        Err(unsupported("break-pane"))
    }

    /// Swap two panes without changing focus (`swap-pane -d -s <src> -t <dst>`).
    fn swap_pane(&self, _source: &PaneId, _target: &PaneId) -> TmuxResult<()> {
        Err(unsupported("swap-pane"))
    }

    /// Resize pane `id` incrementally (`resize-pane -t <id> -L/-R/-U/-D <amount>`).
    fn resize_pane(
        &self,
        _id: &PaneId,
        _direction: ResizeDirection,
        _amount: u16,
    ) -> TmuxResult<()> {
        Err(unsupported("resize-pane"))
    }

    /// Resize pane `id` by window percentage (`resize-pane -t <id> [-x <n>%] [-y <n>%]`).
    fn resize_pane_percent(
        &self,
        id: &PaneId,
        width_pct: Option<u8>,
        height_pct: Option<u8>,
    ) -> TmuxResult<()> {
        self.resize_pane_to(id, PaneTargetSize::percent(width_pct, height_pct))
    }

    /// Resize pane `id` to an explicit width and/or height in percent or cells.
    fn resize_pane_to(&self, _id: &PaneId, _size: PaneTargetSize) -> TmuxResult<()> {
        Err(unsupported("resize-pane"))
    }

    /// Respawn the command running in pane `id` (`respawn-pane [-k] -t <id>`).
    fn respawn_pane(&self, _id: &PaneId, _kill_existing: bool) -> TmuxResult<()> {
        Err(unsupported("respawn-pane"))
    }

    /// Clear scrollback history of pane `id` (`clear-history -t <id>`).
    fn clear_pane_history(&self, _id: &PaneId) -> TmuxResult<()> {
        Err(unsupported("clear-history"))
    }

    /// Send keys to pane `id` (`send-keys -t <id> [-l] -- <keys...>`).
    fn send_keys(&self, _id: &PaneId, _keys: &[&str], _literal: bool) -> TmuxResult<()> {
        Err(unsupported("send-keys"))
    }

    /// Translate and forward a single interactive terminal [`KeyEvent`] into
    /// pane `id` for Live PIP type-through. Key release events and unsupported
    /// modifier-only keys are ignored cleanly (`Ok(())`).
    fn send_key_event(&self, id: &PaneId, key: KeyEvent) -> TmuxResult<()> {
        let Some((keys, literal)) = translate_key_event(key) else {
            return Ok(());
        };
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.send_keys(id, &refs, literal)
    }

    /// Safely paste multi-line or arbitrary text into pane `id` for Live PIP.
    ///
    /// The default implementation forwards lines through [`Tmux::send_keys`] so
    /// test fakes that override `send_keys` work automatically; [`TmuxClient`]
    /// uses an ephemeral named buffer (`load-buffer` + `paste-buffer -d -p`) so
    /// bracketed paste works without disturbing the user's paste buffers.
    fn send_paste(&self, id: &PaneId, text: &str) -> TmuxResult<()> {
        if text.is_empty() {
            return Ok(());
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut parts = normalized.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                self.send_keys(id, &[part], true)?;
            }
            if parts.peek().is_some() {
                self.send_keys(id, &["Enter"], false)?;
            }
        }
        Ok(())
    }

    /// Set pane `id`'s title (`select-pane -t <id> -T <title>`).
    fn rename_pane_title(&self, _id: &PaneId, _title: &str) -> TmuxResult<()> {
        Err(unsupported("select-pane"))
    }

    /// Start or stop streaming pane `id`'s output to a log file (`pipe-pane`).
    /// Passing `Some(path)` starts appending output; `None` stops piping.
    fn toggle_pipe_pane(&self, _id: &PaneId, _log_path: Option<&Path>) -> TmuxResult<()> {
        Err(unsupported("pipe-pane"))
    }

    /// Capture scrollback history plus visible pane lines of `target`
    /// (session's active pane, window's active pane, or pane) with SGR
    /// color/attribute escapes (`\x1b[...m`) preserved and unsafe cursor,
    /// OSC, and control sequences stripped, trimmed to the last `max_lines`
    /// lines.
    fn capture_pane(&self, _target: &Target, _max_lines: usize) -> TmuxResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// Capture scrollback history plus visible pane lines of `target` as plain
    /// sanitized text (no SGR escape codes) for clipboard copy, search, or log
    /// export.
    fn capture_pane_plain(&self, target: &Target, max_lines: usize) -> TmuxResult<Vec<String>> {
        let lines = self.capture_pane(target, max_lines)?;
        Ok(sanitize_captured_lines(&lines.join("\n"), max_lines))
    }

    // --- paste buffers ----------------------------------------------------

    /// List tmux paste buffers. Returns an empty list if no server is running.
    fn list_buffers(&self) -> TmuxResult<Vec<PasteBuffer>> {
        Ok(Vec::new())
    }

    /// Read the full contents of a named tmux paste buffer (`show-buffer -b <name>`).
    fn show_buffer(&self, _name: &str) -> TmuxResult<String> {
        Err(unsupported("show-buffer"))
    }

    /// Push a new tmux paste buffer containing `content` (`load-buffer -`).
    fn set_buffer(&self, _content: &str) -> TmuxResult<()> {
        Err(unsupported("load-buffer"))
    }

    /// Paste buffer `name` into `target` pane without deleting the buffer.
    fn paste_buffer(&self, _name: &str, _target: &PaneId) -> TmuxResult<()> {
        Err(unsupported("paste-buffer"))
    }

    /// Delete paste buffer `name` (`delete-buffer -b <name>`).
    fn delete_buffer(&self, _name: &str) -> TmuxResult<()> {
        Err(unsupported("delete-buffer"))
    }

    // --- clients & going somewhere ----------------------------------------

    /// Prepare a target for attach from outside tmux. A window target is
    /// validated here and selected by `attach-session` during handoff.
    fn select(&self, target: &Target) -> TmuxResult<()>;

    /// `switch-client` the *current* client to `target`. Only meaningful when
    /// [`Tmux::context`] is [`Context::Inside`].
    fn switch_client(&self, target: &Target) -> TmuxResult<()>;

    /// Detach one client identified by client name.
    fn detach_client(&self, client: &str) -> TmuxResult<()>;

    /// List attached clients on the server. Returns empty if no server runs.
    fn list_clients(&self) -> TmuxResult<Vec<ClientInfo>> {
        Ok(Vec::new())
    }

    /// Detach clients attached to `session` (or all sessions if `None`), keeping
    /// `keep_client` (or the current hosting client when `keep_client` is `None`
    /// and [`Tmux::context`] is [`Context::Inside`]). Returns the number of
    /// detached clients.
    fn detach_other_clients(
        &self,
        _session: Option<&SessionId>,
        _keep_client: Option<&str>,
    ) -> TmuxResult<usize> {
        Ok(0)
    }

    /// The `tmux attach-session` command to `exec` after the TUI has been
    /// torn down. Must not be spawned as a child by JustMux.
    fn attach_command(&self, target: &Target) -> Command;

    /// The read-only `tmux attach-session -r` command to `exec` after teardown.
    fn attach_command_readonly(&self, target: &Target) -> Command {
        self.attach_command(target)
    }

    // --- server, popup, config & options ----------------------------------

    /// Reload a tmux configuration file on the running server (`source-file`).
    fn source_file(&self, _path: &Path) -> TmuxResult<()> {
        Err(unsupported("source-file"))
    }

    /// Stop the entire tmux server (`kill-server`).
    fn kill_server(&self) -> TmuxResult<()> {
        Err(unsupported("kill-server"))
    }

    /// Open a floating scratchpad shell popup on the current client (tmux 3.2+).
    fn display_popup_shell(&self, _cwd: Option<&Path>) -> TmuxResult<()> {
        Err(unsupported("display-popup"))
    }

    /// Set a global session option (`set-option -g <option> <value>`).
    fn set_global_option(&self, _option: &str, _value: &str) -> TmuxResult<()> {
        Err(unsupported("set-option"))
    }

    /// Set a global window option (`set-window-option -g <option> <value>`).
    fn set_global_window_option(&self, _option: &str, _value: &str) -> TmuxResult<()> {
        Err(unsupported("set-window-option"))
    }

    /// Set a global server option (`set-option -sg <option> <value>`).
    fn set_server_option(&self, _option: &str, _value: &str) -> TmuxResult<()> {
        Err(unsupported("set-option"))
    }

    // --- opt-in keybind sync ----------------------------------------------

    /// Bind the prefix popup key (`j`) on the running tmux server if reachable.
    /// Returns `Ok(true)` if bound on a live server, `Ok(false)` if no server is running.
    fn bind_popup_key(&self, _run_shell_command: &str) -> TmuxResult<bool> {
        Ok(false)
    }

    /// Unbind the prefix popup key (`j`) on the running tmux server if it was bound
    /// for JustMux (`JUSTMUX_POPUP=1`). Never removes user-customized `j` bindings.
    fn unbind_popup_key(&self) -> TmuxResult<bool> {
        Ok(false)
    }
}

/// Check a proposed session (or window) name before handing it to tmux.
///
/// tmux silently rewrites `.` and `:` to `_` and rejects empty names; we
/// refuse them up front so what the user typed is what they get.
pub fn validate_session_name(name: &str) -> TmuxResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(TmuxError::InvalidName("the name cannot be empty".into()));
    }
    if trimmed != name {
        return Err(TmuxError::InvalidName(
            "the name cannot start or end with whitespace".into(),
        ));
    }
    if let Some(bad) = name.chars().find(|c| matches!(c, '.' | ':')) {
        return Err(TmuxError::InvalidName(format!(
            "tmux does not allow \"{bad}\" in session names"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(TmuxError::InvalidName(
            "the name cannot contain control characters".into(),
        ));
    }
    Ok(())
}

/// Translate a crossterm [`KeyEvent`] into `(tmux_keys, literal)` suitable for
/// `tmux send-keys [-l] -- <keys...>`.
///
/// Returns `None` for key release events or non-transmittable keys.
pub fn translate_key_event(key: KeyEvent) -> Option<(Vec<String>, bool)> {
    if key.kind == KeyEventKind::Release {
        return None;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    if ctrl && alt {
        return match key.code {
            KeyCode::Char(' ') => Some((vec!["C-M-Space".into()], false)),
            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                Some((vec![format!("C-M-{}", c.to_ascii_lowercase())], false))
            }
            _ => None,
        };
    }

    if ctrl {
        return match key.code {
            KeyCode::Char(' ') | KeyCode::Char('@') => Some((vec!["C-Space".into()], false)),
            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                Some((vec![format!("C-{}", c.to_ascii_lowercase())], false))
            }
            KeyCode::Char(c @ ('[' | '\\' | ']' | '^' | '_')) => {
                Some((vec![format!("C-{c}")], false))
            }
            KeyCode::Left => Some((vec!["C-Left".into()], false)),
            KeyCode::Right => Some((vec!["C-Right".into()], false)),
            KeyCode::Up => Some((vec!["C-Up".into()], false)),
            KeyCode::Down => Some((vec!["C-Down".into()], false)),
            KeyCode::Home => Some((vec!["C-Home".into()], false)),
            KeyCode::End => Some((vec!["C-End".into()], false)),
            KeyCode::Backspace => Some((vec!["C-BSpace".into()], false)),
            KeyCode::Delete => Some((vec!["C-DC".into()], false)),
            KeyCode::PageUp => Some((vec!["C-PPage".into()], false)),
            KeyCode::PageDown => Some((vec!["C-NPage".into()], false)),
            _ => None,
        };
    }

    if alt {
        return match key.code {
            KeyCode::Char(' ') => Some((vec!["M-Space".into()], false)),
            KeyCode::Char(c) if c.is_ascii_graphic() => Some((vec![format!("M-{c}")], false)),
            KeyCode::Char(c) if !c.is_control() => Some((vec![c.to_string()], true)),
            KeyCode::Backspace => Some((vec!["M-BSpace".into()], false)),
            KeyCode::Delete => Some((vec!["M-DC".into()], false)),
            KeyCode::Enter => Some((vec!["M-Enter".into()], false)),
            KeyCode::Tab => Some((vec!["M-Tab".into()], false)),
            KeyCode::Up => Some((vec!["M-Up".into()], false)),
            KeyCode::Down => Some((vec!["M-Down".into()], false)),
            KeyCode::Left => Some((vec!["M-Left".into()], false)),
            KeyCode::Right => Some((vec!["M-Right".into()], false)),
            KeyCode::Home => Some((vec!["M-Home".into()], false)),
            KeyCode::End => Some((vec!["M-End".into()], false)),
            KeyCode::PageUp => Some((vec!["M-PPage".into()], false)),
            KeyCode::PageDown => Some((vec!["M-NPage".into()], false)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char(c) if !c.is_control() => Some((vec![c.to_string()], true)),
        KeyCode::Enter => Some((vec!["Enter".into()], false)),
        KeyCode::Tab => Some((vec!["Tab".into()], false)),
        KeyCode::BackTab => Some((vec!["BTab".into()], false)),
        KeyCode::Backspace => Some((vec!["BSpace".into()], false)),
        KeyCode::Delete => Some((vec!["DC".into()], false)),
        KeyCode::Up => Some((vec![if shift { "S-Up" } else { "Up" }.into()], false)),
        KeyCode::Down => Some((vec![if shift { "S-Down" } else { "Down" }.into()], false)),
        KeyCode::Left => Some((vec![if shift { "S-Left" } else { "Left" }.into()], false)),
        KeyCode::Right => Some((vec![if shift { "S-Right" } else { "Right" }.into()], false)),
        KeyCode::Home => Some((vec![if shift { "S-Home" } else { "Home" }.into()], false)),
        KeyCode::End => Some((vec![if shift { "S-End" } else { "End" }.into()], false)),
        KeyCode::PageUp => Some((vec![if shift { "S-PPage" } else { "PPage" }.into()], false)),
        KeyCode::PageDown => Some((vec![if shift { "S-NPage" } else { "NPage" }.into()], false)),
        KeyCode::Insert => Some((vec!["IC".into()], false)),
        KeyCode::Esc => Some((vec!["Escape".into()], false)),
        KeyCode::F(n) if (1..=12).contains(&n) => Some((vec![format!("F{n}")], false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_names_are_validated() {
        assert!(validate_session_name("orion").is_ok());
        assert!(validate_session_name("my project").is_ok());
        assert!(validate_session_name("项目-β").is_ok());
        for bad in ["", "   ", " lead", "trail ", "a.b", "a:b", "tab\there"] {
            assert!(
                matches!(validate_session_name(bad), Err(TmuxError::InvalidName(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn translate_key_event_covers_printable_cjk_specials_ctrl_alt_and_release() {
        let ev = |code, modifiers| KeyEvent::new(code, modifiers);

        // Printable ASCII (including `;` and `-`) and UTF-8/CJK use literal mode.
        assert_eq!(
            translate_key_event(ev(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some((vec!["a".into()], true))
        );
        assert_eq!(
            translate_key_event(ev(KeyCode::Char(';'), KeyModifiers::NONE)),
            Some((vec![";".into()], true))
        );
        assert_eq!(
            translate_key_event(ev(KeyCode::Char('中'), KeyModifiers::NONE)),
            Some((vec!["中".into()], true))
        );

        // Navigation, editing, function keys, and Escape.
        for (code, expected) in [
            (KeyCode::Enter, "Enter"),
            (KeyCode::Tab, "Tab"),
            (KeyCode::BackTab, "BTab"),
            (KeyCode::Backspace, "BSpace"),
            (KeyCode::Delete, "DC"),
            (KeyCode::Up, "Up"),
            (KeyCode::Down, "Down"),
            (KeyCode::Left, "Left"),
            (KeyCode::Right, "Right"),
            (KeyCode::Home, "Home"),
            (KeyCode::End, "End"),
            (KeyCode::PageUp, "PPage"),
            (KeyCode::PageDown, "NPage"),
            (KeyCode::Insert, "IC"),
            (KeyCode::Esc, "Escape"),
            (KeyCode::F(1), "F1"),
            (KeyCode::F(12), "F12"),
        ] {
            assert_eq!(
                translate_key_event(ev(code, KeyModifiers::NONE)),
                Some((vec![expected.into()], false)),
                "{code:?} -> {expected}"
            );
        }

        // Ctrl combinations normalize letter case and support C-Space.
        assert_eq!(
            translate_key_event(ev(KeyCode::Char('C'), KeyModifiers::CONTROL)),
            Some((vec!["C-c".into()], false))
        );
        assert_eq!(
            translate_key_event(ev(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some((vec!["C-d".into()], false))
        );
        assert_eq!(
            translate_key_event(ev(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            Some((vec!["C-Space".into()], false))
        );

        // Alt combinations.
        assert_eq!(
            translate_key_event(ev(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some((vec!["M-b".into()], false))
        );
        assert_eq!(
            translate_key_event(ev(KeyCode::Backspace, KeyModifiers::ALT)),
            Some((vec!["M-BSpace".into()], false))
        );

        // Key release events are ignored to prevent double-typing.
        let mut released = ev(KeyCode::Char('x'), KeyModifiers::NONE);
        released.kind = KeyEventKind::Release;
        assert_eq!(translate_key_event(released), None);
        assert_eq!(
            translate_key_event(ev(KeyCode::Null, KeyModifiers::NONE)),
            None
        );
    }
}
