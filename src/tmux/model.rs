//! Plain data types describing tmux objects.
//!
//! Everything is addressed by tmux's own stable IDs (`$3`, `@7`, `%12`) rather
//! than by name, so same-named or prefix-matching sessions can never be
//! confused with each other.

use std::fmt;

macro_rules! tmux_id {
    ($(#[$meta:meta])* $name:ident, $sigil:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// The sigil tmux uses for this kind of ID.
            pub const SIGIL: char = $sigil;

            /// Parse a raw tmux ID such as `$3`. Returns `None` unless the
            /// value is the sigil followed by one or more ASCII digits.
            pub fn new(raw: impl Into<String>) -> Option<Self> {
                let raw = raw.into();
                let digits = raw.strip_prefix($sigil)?;
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                Some(Self(raw))
            }

            /// The ID exactly as tmux prints and accepts it (e.g. `$3`).
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

tmux_id!(
    /// A tmux session ID, e.g. `$3`.
    SessionId,
    '$'
);
tmux_id!(
    /// A tmux window ID, e.g. `@7`.
    WindowId,
    '@'
);
tmux_id!(
    /// A tmux pane ID, e.g. `%12`.
    PaneId,
    '%'
);

/// Something a client can be attached or switched to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Session(SessionId),
    Window(WindowId),
    Pane(PaneId),
}

impl Target {
    /// The value to pass to tmux's `-t`.
    pub fn as_str(&self) -> &str {
        match self {
            Target::Session(id) => id.as_str(),
            Target::Window(id) => id.as_str(),
            Target::Pane(id) => id.as_str(),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    /// Number of windows.
    pub windows: usize,
    /// Number of clients currently attached.
    pub attached: usize,
    /// Creation time, seconds since the Unix epoch.
    pub created: u64,
    /// Last time a client attached, if ever (seconds since the Unix epoch).
    pub last_attached: Option<u64>,
    /// Last activity time, seconds since the Unix epoch.
    pub activity: u64,
    /// The session's working directory.
    pub path: String,
}

impl Session {
    pub fn is_attached(&self) -> bool {
        self.attached > 0
    }

    /// Timestamp used for most-recently-used ordering.
    pub fn recency(&self) -> u64 {
        self.last_attached.unwrap_or(0).max(self.activity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub session_id: SessionId,
    pub session_name: String,
    pub index: u32,
    pub name: String,
    /// Whether this is the session's current window.
    pub active: bool,
    /// Number of panes.
    pub panes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub id: PaneId,
    pub window_id: WindowId,
    pub window_index: u32,
    pub window_name: String,
    pub session_id: SessionId,
    pub session_name: String,
    pub index: u32,
    /// Whether this is the window's active pane.
    pub active: bool,
    /// Foreground command as reported by tmux (`pane_current_command`).
    pub current_command: String,
    pub current_path: String,
    pub title: String,
    /// PID of the pane's first process.
    pub pid: Option<u32>,
    /// The pane's process has exited (`remain-on-exit`).
    pub dead: bool,
    /// Whether something other than an idle shell appears to be running.
    /// Closing a busy pane needs a stronger confirmation.
    pub busy: bool,
}

/// How to split a pane. Named after the resulting layout because tmux's own
/// `-h`/`-v` naming is famously easy to get backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitDirection {
    /// New pane to the right of the current one (`split-window -h`).
    SideBySide,
    /// New pane below the current one (`split-window -v`).
    Stacked,
}

/// A parsed `tmux -V`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxVersion {
    /// The raw string after `tmux `, e.g. `3.3a` or `next-3.5`.
    pub raw: String,
    /// `(major, minor)` when the version is numeric; `None` for e.g. `master`.
    pub number: Option<(u32, u32)>,
}

impl TmuxVersion {
    /// `display-popup` exists from tmux 3.2. Unparseable development builds
    /// (`master`) are assumed to be recent.
    pub fn supports_popup(&self) -> bool {
        match self.number {
            Some(v) => v >= (3, 2),
            None => true,
        }
    }
}

impl fmt::Display for TmuxVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Where JustMux is running relative to the tmux server it manages. This
/// decides how "go to session" is carried out and must never be guessed in a
/// way that could nest tmux inside tmux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// A plain terminal (or a stale `$TMUX`): entering a session replaces the
    /// `jm` process with `tmux attach-session`.
    Outside,
    /// Inside a client of the managed server — a pane's shell or a
    /// `display-popup`: entering a session is `tmux switch-client`.
    Inside(InsideContext),
    /// Inside a *different* tmux server than the one being managed (e.g.
    /// `jm -L other` typed in a tmux pane). Sessions can be managed but not
    /// entered, because attaching would nest tmux.
    ForeignServer,
    /// `$TMUX` was present but the launching pane could not be resolved to a
    /// live client. Entering a session must be refused to avoid nesting.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InsideContext {
    /// The session the current client is showing.
    pub session_id: Option<SessionId>,
    /// The current client's active window.
    pub window_id: Option<WindowId>,
    /// The current client's active pane (for a popup: the pane underneath).
    pub pane_id: Option<PaneId>,
    /// Best-effort: running in a `display-popup` rather than in a pane.
    pub popup: bool,
    /// The exact tmux client attached to the launching pane.
    pub client_name: Option<String>,
}

impl Context {
    pub fn is_inside(&self) -> bool {
        matches!(self, Context::Inside(_))
    }

    /// The session the user is currently looking at, if any.
    pub fn current_session(&self) -> Option<&SessionId> {
        match self {
            Context::Inside(inside) => inside.session_id.as_ref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_require_sigil_and_digits() {
        assert_eq!(
            SessionId::new("$12").map(|id| id.to_string()),
            Some("$12".into())
        );
        assert!(SessionId::new("12").is_none());
        assert!(SessionId::new("$").is_none());
        assert!(SessionId::new("$1a").is_none());
        assert!(SessionId::new("@1").is_none());
        assert!(WindowId::new("@0").is_some());
        assert!(PaneId::new("%42").is_some());
    }

    #[test]
    fn popup_support_starts_at_3_2() {
        let v = |raw: &str, number| TmuxVersion {
            raw: raw.into(),
            number,
        };
        assert!(!v("3.1c", Some((3, 1))).supports_popup());
        assert!(v("3.2", Some((3, 2))).supports_popup());
        assert!(v("4.0", Some((4, 0))).supports_popup());
        assert!(v("master", None).supports_popup());
    }
}
