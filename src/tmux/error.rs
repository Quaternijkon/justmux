//! Structured tmux errors. `Display` output is shown to users verbatim, so
//! every message must be a complete, friendly sentence fragment — never a raw
//! debug dump.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("tmux is not installed or not on your PATH")]
    NotInstalled,

    #[error("no tmux server is running")]
    NoServer,

    #[error("that session no longer exists")]
    SessionNotFound,

    #[error("that window no longer exists")]
    WindowNotFound,

    #[error("that pane no longer exists")]
    PaneNotFound,

    #[error("a session named \"{0}\" already exists")]
    DuplicateSession(String),

    #[error("invalid name: {0}")]
    InvalidName(String),

    #[error("directory does not exist: {}", .0.display())]
    DirectoryNotFound(PathBuf),

    #[error("no tmux client is attached to this terminal")]
    NoCurrentClient,

    /// tmux ran but reported an error JustMux has no specific handling for.
    /// `message` is tmux's own stderr, trimmed.
    #[error("tmux {command} failed: {message}")]
    CommandFailed { command: String, message: String },

    #[error("could not understand tmux output: {0}")]
    Parse(String),

    #[error("could not run tmux: {0}")]
    Io(#[from] std::io::Error),
}

pub type TmuxResult<T> = Result<T, TmuxError>;
