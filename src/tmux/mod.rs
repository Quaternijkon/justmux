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
//! * nothing here changes tmux options, key bindings or other clients, and
//!   `attach-session -d` is never used.

pub mod client;
pub mod error;
pub mod model;
pub mod parser;

use std::path::{Path, PathBuf};
use std::process::Command;

pub use client::TmuxClient;
pub use error::{TmuxError, TmuxResult};
pub use model::{
    Context, InsideContext, Pane, PaneId, Session, SessionId, SplitDirection, Target, TmuxVersion,
    Window, WindowId,
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

    // --- panes ----------------------------------------------------------

    /// Panes of one window, or of every window when `window` is `None`.
    fn list_panes(&self, window: Option<&WindowId>) -> TmuxResult<Vec<Pane>>;

    /// Split `target`, returning the new pane. The new pane starts in the
    /// target pane's current directory.
    fn split_pane(&self, target: &PaneId, direction: SplitDirection) -> TmuxResult<Pane>;

    fn kill_pane(&self, id: &PaneId) -> TmuxResult<()>;

    // --- going somewhere --------------------------------------------------

    /// Make `target` the current window/pane of its session (no-op for a
    /// session target). Used before attaching from outside tmux.
    fn select(&self, target: &Target) -> TmuxResult<()>;

    /// `switch-client` the *current* client to `target`. Only meaningful when
    /// [`Tmux::context`] is [`Context::Inside`].
    fn switch_client(&self, target: &Target) -> TmuxResult<()>;

    /// The `tmux attach-session` command to `exec` after the TUI has been
    /// torn down. Must not be spawned as a child by JustMux.
    fn attach_command(&self, target: &Target) -> Command;
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
}
