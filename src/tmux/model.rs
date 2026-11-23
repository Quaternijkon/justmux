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
    Window(SessionId, WindowId),
    Pane(PaneId),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(id) => f.write_str(id.as_str()),
            Self::Window(session, window) => write!(f, "{session}:{window}"),
            Self::Pane(id) => f.write_str(id.as_str()),
        }
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
    /// Whether the window's active pane is currently zoomed (`#{window_zoomed_flag}`).
    pub zoomed: bool,
    /// Whether `synchronize-panes` is enabled on this window (`#{pane_synchronized}`).
    pub synchronized: bool,
    /// Whether tmux flagged unseen activity on this window (`#{window_activity_flag}`).
    pub activity_flag: bool,
    /// Whether tmux flagged a bell on this window (`#{window_bell_flag}`).
    pub bell_flag: bool,
    /// Whether tmux flagged silence on this window (`#{window_silence_flag}`).
    pub silence_flag: bool,
    /// tmux layout descriptor (`#{window_layout}`), restorable via `select-layout`.
    pub layout: String,
    /// Whether `monitor-activity` is enabled (`#{?monitor-activity,1,0}`).
    pub monitor_activity: bool,
    /// `monitor-silence` interval in seconds (`0` means disabled, `#{monitor-silence}`).
    pub monitor_silence: u64,
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
    /// Pane width in terminal columns (`#{pane_width}`).
    pub width: u32,
    /// Pane height in terminal rows (`#{pane_height}`).
    pub height: u32,
    /// Pane left x-coordinate within its window (`#{pane_left}`).
    pub left: u32,
    /// Pane top y-coordinate within its window (`#{pane_top}`).
    pub top: u32,
    /// Whether the pane's window is currently zoomed (`#{window_zoomed_flag}`).
    pub zoomed: bool,
    /// Whether `synchronize-panes` is enabled on this pane (`#{pane_synchronized}`).
    pub synchronized: bool,
}

/// An attached tmux client reported by `list-clients`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    pub session_name: String,
    pub session_id: Option<SessionId>,
    pub width: u32,
    pub height: u32,
    pub tty: String,
    pub readonly: bool,
    pub control_mode: bool,
    pub activity: u64,
}

/// A tmux paste buffer reported by `list-buffers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteBuffer {
    pub name: String,
    pub size: usize,
    pub sample: String,
}

/// Built-in tmux window layout presets (`select-layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutPreset {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl LayoutPreset {
    pub const ALL: [Self; 5] = [
        Self::EvenHorizontal,
        Self::EvenVertical,
        Self::MainHorizontal,
        Self::MainVertical,
        Self::Tiled,
    ];

    pub const fn as_tmux_str(self) -> &'static str {
        match self {
            Self::EvenHorizontal => "even-horizontal",
            Self::EvenVertical => "even-vertical",
            Self::MainHorizontal => "main-horizontal",
            Self::MainVertical => "main-vertical",
            Self::Tiled => "tiled",
        }
    }

    pub const fn label_en(self) -> &'static str {
        match self {
            Self::EvenHorizontal => "Even Horizontal",
            Self::EvenVertical => "Even Vertical",
            Self::MainHorizontal => "Main Horizontal",
            Self::MainVertical => "Main Vertical",
            Self::Tiled => "Tiled",
        }
    }

    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::EvenHorizontal => "水平均分",
            Self::EvenVertical => "垂直均分",
            Self::MainHorizontal => "主上副下",
            Self::MainVertical => "主左副右",
            Self::Tiled => "网格平铺",
        }
    }

    pub const fn label(self, zh: bool) -> &'static str {
        if zh { self.label_zh() } else { self.label_en() }
    }
}

impl fmt::Display for LayoutPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_tmux_str())
    }
}

/// Direction for incremental `tmux resize-pane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeDirection {
    Left,
    Right,
    Up,
    Down,
}

impl ResizeDirection {
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Left => "-L",
            Self::Right => "-R",
            Self::Up => "-U",
            Self::Down => "-D",
        }
    }
}

/// Target pane dimension for `tmux resize-pane -x / -y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneDimension {
    /// Percentage of the window (`1..=100`).
    Percent(u8),
    /// Exact terminal cell count (`>= 1`).
    Cells(u16),
}

impl PaneDimension {
    pub fn to_tmux_arg(self) -> String {
        match self {
            Self::Percent(pct) => format!("{}%", pct.clamp(1, 100)),
            Self::Cells(cells) => cells.max(1).to_string(),
        }
    }
}

/// Target width and/or height for resizing a pane (`resize-pane -x/-y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PaneTargetSize {
    pub width: Option<PaneDimension>,
    pub height: Option<PaneDimension>,
}

impl PaneTargetSize {
    pub fn percent(width_pct: Option<u8>, height_pct: Option<u8>) -> Self {
        Self {
            width: width_pct.map(PaneDimension::Percent),
            height: height_pct.map(PaneDimension::Percent),
        }
    }

    pub fn cells(width: Option<u16>, height: Option<u16>) -> Self {
        Self {
            width: width.map(PaneDimension::Cells),
            height: height.map(PaneDimension::Cells),
        }
    }

    pub fn is_empty(self) -> bool {
        self.width.is_none() && self.height.is_none()
    }
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
    /// The attached client's session name, for display.
    pub session_name: Option<String>,
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

    #[test]
    fn layout_presets_and_resize_helpers_format_cleanly() {
        assert_eq!(LayoutPreset::ALL.len(), 5);
        assert_eq!(
            LayoutPreset::EvenHorizontal.as_tmux_str(),
            "even-horizontal"
        );
        assert_eq!(LayoutPreset::Tiled.to_string(), "tiled");
        assert_eq!(LayoutPreset::MainVertical.label(false), "Main Vertical");
        assert_eq!(LayoutPreset::MainVertical.label(true), "主左副右");

        assert_eq!(ResizeDirection::Left.flag(), "-L");
        assert_eq!(ResizeDirection::Right.flag(), "-R");
        assert_eq!(ResizeDirection::Up.flag(), "-U");
        assert_eq!(ResizeDirection::Down.flag(), "-D");

        assert_eq!(PaneDimension::Percent(0).to_tmux_arg(), "1%");
        assert_eq!(PaneDimension::Percent(70).to_tmux_arg(), "70%");
        assert_eq!(PaneDimension::Percent(150).to_tmux_arg(), "100%");
        assert_eq!(PaneDimension::Cells(0).to_tmux_arg(), "1");
        assert_eq!(PaneDimension::Cells(42).to_tmux_arg(), "42");

        assert!(PaneTargetSize::default().is_empty());
        let target = PaneTargetSize::percent(Some(70), None);
        assert!(!target.is_empty());
        assert_eq!(target.width, Some(PaneDimension::Percent(70)));
        assert_eq!(target.height, None);
    }
}
