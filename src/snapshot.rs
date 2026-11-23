//! Zero-dependency workspace snapshot/template persistence, capture, and restore.
//!
//! Snapshots capture live tmux sessions, windows (including `layout` and
//! `synchronize-panes`), and panes (`cwd`, `title`, `active`, `command`) into
//! clean human-editable TOML files stored under:
//! 1. `$JUSTMUX_SNAPSHOT_DIR` (if set and non-empty)
//! 2. `$XDG_CONFIG_HOME/justmux/snapshots` (if set and absolute)
//! 3. `~/.config/justmux/snapshots`

use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::tmux::{
    LayoutPreset, PaneId, SessionId, SplitDirection, Target, Tmux, validate_session_name,
};

/// Environment variable overriding the snapshot storage directory.
pub const SNAPSHOT_DIR_ENV: &str = "JUSTMUX_SNAPSHOT_DIR";
const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const MAX_SNAPSHOT_NAME_LEN: usize = 64;

/// A saved multi-session (or single-session) tmux workspace template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub name: String,
    pub created_epoch: u64,
    pub sessions: Vec<SnapshotSession>,
}

/// A captured tmux session inside a [`WorkspaceSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSession {
    pub name: String,
    pub path: String,
    pub windows: Vec<SnapshotWindow>,
}

/// A captured tmux window inside a [`SnapshotSession`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotWindow {
    pub name: String,
    pub active: bool,
    pub layout: String,
    pub synchronized: bool,
    pub panes: Vec<SnapshotPane>,
}

/// A captured tmux pane inside a [`SnapshotWindow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPane {
    pub cwd: String,
    pub title: String,
    pub active: bool,
    pub command: String,
}

/// Lightweight listing entry returned by [`list_snapshots`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub name: String,
    pub path: PathBuf,
    pub created_epoch: u64,
    pub session_count: usize,
    pub window_count: usize,
    pub pane_count: usize,
}

/// Summary returned by [`restore_snapshot`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub sessions_created: usize,
    pub sessions_reused: usize,
    pub windows_created: usize,
    pub panes_created: usize,
    pub warnings: Vec<String>,
}

impl WorkspaceSnapshot {
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn window_count(&self) -> usize {
        self.sessions.iter().map(|s| s.windows.len()).sum()
    }

    pub fn pane_count(&self) -> usize {
        self.sessions
            .iter()
            .flat_map(|s| s.windows.iter())
            .map(|w| w.panes.len())
            .sum()
    }

    /// Serialize this snapshot into clean, zero-dependency TOML.
    pub fn to_toml(&self) -> String {
        to_toml(self)
    }

    /// Parse a [`WorkspaceSnapshot`] from TOML text.
    pub fn from_toml(input: &str) -> Result<Self, String> {
        from_toml(input)
    }

    /// Format a Unix epoch timestamp as `YYYY-MM-DD HH:MM UTC`.
    pub fn format_epoch_utc(epoch: u64) -> String {
        format_utc_datetime(epoch)
    }

    /// Render a structured plain-text tree preview suitable for the `/snapshot` UI.
    pub fn format_tree_preview(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "{}  ({})",
            self.name,
            Self::format_epoch_utc(self.created_epoch)
        ));
        if self.sessions.is_empty() {
            lines.push("  (no sessions)".to_owned());
            return lines;
        }
        for session in &self.sessions {
            let sess_dir = if session.path.is_empty() {
                String::new()
            } else {
                format!("  {}", session.path)
            };
            lines.push(format!("$ {}{sess_dir}", session.name));
            for window in &session.windows {
                let mut flags = Vec::new();
                if window.active {
                    flags.push("*");
                }
                if window.synchronized {
                    flags.push("SYNC");
                }
                let badge = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(" "))
                };
                lines.push(format!(
                    "  @ {}{badge} ({} panes)",
                    window.name,
                    window.panes.len()
                ));
                for (p_idx, pane) in window.panes.iter().enumerate() {
                    let star = if pane.active { "*" } else { " " };
                    let cmd = if pane.command.is_empty() {
                        "shell"
                    } else {
                        pane.command.as_str()
                    };
                    let title = if pane.title.is_empty() {
                        String::new()
                    } else {
                        format!(" \"{}\"", pane.title)
                    };
                    lines.push(format!("    {star}%{p_idx} {cmd}{title}  {}", pane.cwd));
                }
            }
        }
        lines
    }
}

/// Validate a snapshot name before using it as a filename stem.
pub fn validate_snapshot_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("snapshot name cannot be empty".to_owned());
    }
    if trimmed != name {
        return Err("snapshot name cannot start or end with whitespace".to_owned());
    }
    if name.chars().count() > MAX_SNAPSHOT_NAME_LEN {
        return Err(format!(
            "snapshot name is too long (max {MAX_SNAPSHOT_NAME_LEN} characters)"
        ));
    }
    if name.starts_with('.') || name.contains("..") {
        return Err("snapshot name cannot start with `.` or contain `..`".to_owned());
    }
    if let Some(bad) = name.chars().find(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(format!(
            "snapshot name cannot contain invalid character `{bad}`"
        ));
    }
    Ok(())
}

/// Resolve the snapshot directory:
/// 1) `$JUSTMUX_SNAPSHOT_DIR` (if set and non-empty)
/// 2) `$XDG_CONFIG_HOME/justmux/snapshots` (if set and absolute)
/// 3) `~/.config/justmux/snapshots`
pub fn resolve_snapshot_dir() -> Option<PathBuf> {
    let env_override = env::var_os(SNAPSHOT_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let home_dir = env::home_dir();
    resolve_snapshot_dir_with(
        env_override.as_deref(),
        xdg_config_home.as_deref(),
        home_dir.as_deref(),
    )
}

fn resolve_snapshot_dir_with(
    env_override: Option<&Path>,
    xdg_config_home: Option<&Path>,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = env_override.filter(|p| !p.as_os_str().is_empty()) {
        return Some(dir.to_path_buf());
    }
    if let Some(xdg) = xdg_config_home.filter(|p| p.is_absolute()) {
        return Some(xdg.join("justmux").join("snapshots"));
    }
    home_dir.map(|home| home.join(".config").join("justmux").join("snapshots"))
}

/// Compute the `.toml` file path for a validated snapshot name inside `dir`.
pub fn snapshot_file_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_snapshot_name(name)?;
    Ok(dir.join(format!("{name}.toml")))
}

/// Serialize a [`WorkspaceSnapshot`] into clean, round-trippable TOML.
pub fn to_toml(snapshot: &WorkspaceSnapshot) -> String {
    let mut out = String::new();
    out.push_str("# JustMux workspace snapshot\n");
    out.push_str(&format!("version = {SNAPSHOT_SCHEMA_VERSION}\n"));
    out.push_str(&format!("name = {}\n", quote_toml_string(&snapshot.name)));
    out.push_str(&format!("created_epoch = {}\n", snapshot.created_epoch));

    for session in &snapshot.sessions {
        out.push_str("\n[[sessions]]\n");
        out.push_str(&format!("name = {}\n", quote_toml_string(&session.name)));
        out.push_str(&format!("path = {}\n", quote_toml_string(&session.path)));

        for window in &session.windows {
            out.push_str("\n[[sessions.windows]]\n");
            out.push_str(&format!("name = {}\n", quote_toml_string(&window.name)));
            out.push_str(&format!("active = {}\n", window.active));
            out.push_str(&format!("layout = {}\n", quote_toml_string(&window.layout)));
            out.push_str(&format!("synchronized = {}\n", window.synchronized));

            for pane in &window.panes {
                out.push_str("\n[[sessions.windows.panes]]\n");
                out.push_str(&format!("cwd = {}\n", quote_toml_string(&pane.cwd)));
                out.push_str(&format!("title = {}\n", quote_toml_string(&pane.title)));
                out.push_str(&format!("active = {}\n", pane.active));
                out.push_str(&format!("command = {}\n", quote_toml_string(&pane.command)));
            }
        }
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseSection {
    Root,
    Session,
    Window,
    Pane,
}

/// Parse a [`WorkspaceSnapshot`] from TOML text produced by [`to_toml`].
pub fn from_toml(input: &str) -> Result<WorkspaceSnapshot, String> {
    let mut section = ParseSection::Root;
    let mut name: Option<String> = None;
    let mut created_epoch: u64 = 0;
    let mut sessions: Vec<SnapshotSession> = Vec::new();

    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = if idx == 0 {
            raw_line.strip_prefix('\u{feff}').unwrap_or(raw_line)
        } else {
            raw_line
        };
        let stripped = strip_toml_comment(line);
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
            let table = trimmed[2..trimmed.len() - 2].trim();
            match table {
                "sessions" => {
                    sessions.push(SnapshotSession {
                        name: String::new(),
                        path: String::new(),
                        windows: Vec::new(),
                    });
                    section = ParseSection::Session;
                }
                "sessions.windows" => {
                    let Some(sess) = sessions.last_mut() else {
                        return Err(format!(
                            "line {line_no}: `[[sessions.windows]]` appeared before `[[sessions]]`"
                        ));
                    };
                    sess.windows.push(SnapshotWindow {
                        name: String::new(),
                        active: false,
                        layout: String::new(),
                        synchronized: false,
                        panes: Vec::new(),
                    });
                    section = ParseSection::Window;
                }
                "sessions.windows.panes" => {
                    let Some(sess) = sessions.last_mut() else {
                        return Err(format!(
                            "line {line_no}: `[[sessions.windows.panes]]` appeared before `[[sessions]]`"
                        ));
                    };
                    let Some(win) = sess.windows.last_mut() else {
                        return Err(format!(
                            "line {line_no}: `[[sessions.windows.panes]]` appeared before `[[sessions.windows]]`"
                        ));
                    };
                    win.panes.push(SnapshotPane {
                        cwd: String::new(),
                        title: String::new(),
                        active: false,
                        command: String::new(),
                    });
                    section = ParseSection::Pane;
                }
                other => {
                    return Err(format!(
                        "line {line_no}: unknown snapshot table `[[{other}]]`"
                    ));
                }
            }
            continue;
        }

        if trimmed.starts_with('[') {
            return Err(format!(
                "line {line_no}: unexpected section header `{trimmed}`"
            ));
        }

        let Some((raw_key, raw_val)) = trimmed.split_once('=') else {
            return Err(format!(
                "line {line_no}: expected `key = value`, found `{trimmed}`"
            ));
        };
        let key = raw_key.trim();
        let val = raw_val.trim();
        if key.is_empty() || val.is_empty() {
            return Err(format!("line {line_no}: malformed assignment `{trimmed}`"));
        }

        match section {
            ParseSection::Root => match key {
                "version" => {
                    let v = parse_toml_u64(val, key, line_no)?;
                    if v != u64::from(SNAPSHOT_SCHEMA_VERSION) {
                        return Err(format!(
                            "line {line_no}: unsupported snapshot version `{v}`"
                        ));
                    }
                }
                "name" => name = Some(parse_toml_string(val, line_no)?),
                "created_epoch" => created_epoch = parse_toml_u64(val, key, line_no)?,
                _ => {
                    return Err(format!("line {line_no}: unknown root key `{key}`"));
                }
            },
            ParseSection::Session => {
                let Some(sess) = sessions.last_mut() else {
                    return Err(format!("line {line_no}: missing session block"));
                };
                match key {
                    "name" => sess.name = parse_toml_string(val, line_no)?,
                    "path" => sess.path = parse_toml_string(val, line_no)?,
                    _ => {
                        return Err(format!("line {line_no}: unknown session key `{key}`"));
                    }
                }
            }
            ParseSection::Window => {
                let Some(win) = sessions.last_mut().and_then(|s| s.windows.last_mut()) else {
                    return Err(format!("line {line_no}: missing window block"));
                };
                match key {
                    "name" => win.name = parse_toml_string(val, line_no)?,
                    "active" => win.active = parse_toml_bool(val, key, line_no)?,
                    "layout" => win.layout = parse_toml_string(val, line_no)?,
                    "synchronized" => win.synchronized = parse_toml_bool(val, key, line_no)?,
                    _ => {
                        return Err(format!("line {line_no}: unknown window key `{key}`"));
                    }
                }
            }
            ParseSection::Pane => {
                let Some(pane) = sessions
                    .last_mut()
                    .and_then(|s| s.windows.last_mut())
                    .and_then(|w| w.panes.last_mut())
                else {
                    return Err(format!("line {line_no}: missing pane block"));
                };
                match key {
                    "cwd" => pane.cwd = parse_toml_string(val, line_no)?,
                    "title" => pane.title = parse_toml_string(val, line_no)?,
                    "active" => pane.active = parse_toml_bool(val, key, line_no)?,
                    "command" => pane.command = parse_toml_string(val, line_no)?,
                    _ => {
                        return Err(format!("line {line_no}: unknown pane key `{key}`"));
                    }
                }
            }
        }
    }

    let snapshot_name =
        name.ok_or_else(|| "snapshot file is missing required `name`".to_owned())?;
    if snapshot_name.trim().is_empty() {
        return Err("snapshot `name` cannot be empty".to_owned());
    }

    Ok(WorkspaceSnapshot {
        name: snapshot_name,
        created_epoch,
        sessions,
    })
}

/// Persist `snapshot` into `<dir>/<name>.toml`, creating `dir` if needed.
pub fn save_snapshot(dir: &Path, snapshot: &WorkspaceSnapshot) -> Result<PathBuf, String> {
    let path = snapshot_file_path(dir, &snapshot.name)?;
    fs::create_dir_all(dir).map_err(|err| {
        format!(
            "could not create snapshot directory `{}`: {err}",
            dir.display()
        )
    })?;
    fs::write(&path, to_toml(snapshot))
        .map_err(|err| format!("could not write snapshot `{}`: {err}", path.display()))?;
    Ok(path)
}

/// Load a named snapshot from `<dir>/<name>.toml`.
pub fn load_snapshot(dir: &Path, name: &str) -> Result<WorkspaceSnapshot, String> {
    let path = snapshot_file_path(dir, name)?;
    load_snapshot_file(&path)
}

/// Load a snapshot from an explicit `.toml` file path.
pub fn load_snapshot_file(path: &Path) -> Result<WorkspaceSnapshot, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("could not read snapshot `{}`: {err}", path.display()))?;
    from_toml(&content).map_err(|err| format!("{}: {err}", path.display()))
}

/// List all valid snapshots in `dir`, newest first (ties broken by name).
/// Missing directories cleanly return `Ok(Vec::new())` without creating them.
pub fn list_snapshots(dir: &Path) -> Result<Vec<SnapshotSummary>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(format!(
                "could not read snapshot directory `{}`: {err}",
                dir.display()
            ));
        }
    };

    let mut summaries = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") || !path.is_file() {
            continue;
        }
        let Ok(snap) = load_snapshot_file(&path) else {
            continue;
        };
        summaries.push(SnapshotSummary {
            session_count: snap.session_count(),
            window_count: snap.window_count(),
            pane_count: snap.pane_count(),
            name: snap.name,
            path,
            created_epoch: snap.created_epoch,
        });
    }

    summaries.sort_by(|a, b| {
        b.created_epoch
            .cmp(&a.created_epoch)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(summaries)
}

/// Delete `<dir>/<name>.toml` if it exists. Returns `Ok(true)` if removed,
/// `Ok(false)` if not found.
pub fn delete_snapshot(dir: &Path, name: &str) -> Result<bool, String> {
    let path = snapshot_file_path(dir, name)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!(
            "could not delete snapshot `{}`: {err}",
            path.display()
        )),
    }
}

/// Capture live tmux state into a [`WorkspaceSnapshot`].
///
/// When `session_filter` is `Some(id)`, only that session is captured; otherwise
/// every session on the server is captured.
pub fn capture_snapshot(
    tmux: &dyn Tmux,
    name: &str,
    session_filter: Option<&SessionId>,
) -> Result<WorkspaceSnapshot, String> {
    validate_snapshot_name(name)?;

    let mut sessions = tmux
        .list_sessions()
        .map_err(|err| format!("failed to list tmux sessions: {err}"))?;
    if let Some(filter_id) = session_filter {
        sessions.retain(|s| &s.id == filter_id);
        if sessions.is_empty() {
            return Err(format!("session `{filter_id}` was not found"));
        }
    }
    if sessions.is_empty() {
        return Err("no tmux sessions are running to snapshot".to_owned());
    }

    sessions.sort_by(|a, b| a.name.cmp(&b.name));

    let mut snap_sessions = Vec::with_capacity(sessions.len());
    for session in sessions {
        let mut windows = tmux
            .list_windows(Some(&session.id))
            .map_err(|err| format!("failed to list windows for `{}`: {err}", session.name))?;
        windows.sort_by_key(|w| w.index);

        let mut snap_windows = Vec::with_capacity(windows.len());
        for window in windows {
            let mut panes = tmux
                .list_panes(Some(&window.id))
                .map_err(|err| format!("failed to list panes for `{}`: {err}", window.name))?;
            panes.sort_by_key(|p| p.index);

            let snap_panes = panes
                .into_iter()
                .map(|p| SnapshotPane {
                    cwd: p.current_path,
                    title: p.title,
                    active: p.active,
                    command: p.current_command,
                })
                .collect();

            snap_windows.push(SnapshotWindow {
                name: window.name,
                active: window.active,
                layout: window.layout,
                synchronized: window.synchronized,
                panes: snap_panes,
            });
        }

        snap_sessions.push(SnapshotSession {
            name: session.name,
            path: session.path,
            windows: snap_windows,
        });
    }

    let created_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(WorkspaceSnapshot {
        name: name.to_owned(),
        created_epoch,
        sessions: snap_sessions,
    })
}

/// Restore a [`WorkspaceSnapshot`] onto `tmux`.
///
/// - Creates missing sessions and reuses existing sessions by name (skipping
///   windows that already exist by name in a reused session so repeated restores
///   stay idempotent).
/// - Creates windows and splits panes with each pane's saved `cwd`, falling back
///   gracefully to `$HOME` (or `.`) with a warning if a saved directory no longer
///   exists.
/// - Restores each window's saved layout (`select_layout_raw`), `synchronize-panes`,
///   pane titles (`rename_pane_title`), and active pane selection.
pub fn restore_snapshot(
    tmux: &dyn Tmux,
    snapshot: &WorkspaceSnapshot,
) -> Result<RestoreReport, String> {
    if snapshot.sessions.is_empty() {
        return Err("snapshot contains no sessions to restore".to_owned());
    }

    let fallback_dir = fallback_working_dir();
    let existing_sessions: HashMap<String, SessionId> = tmux
        .list_sessions()
        .map_err(|err| format!("failed to list tmux sessions: {err}"))?
        .into_iter()
        .map(|s| (s.name, s.id))
        .collect();

    let mut report = RestoreReport::default();

    for snap_session in &snapshot.sessions {
        if let Err(err) = validate_session_name(&snap_session.name) {
            report
                .warnings
                .push(format!("skipped session `{}`: {err}", snap_session.name));
            continue;
        }

        if let Some(existing_id) = existing_sessions.get(&snap_session.name) {
            report.sessions_reused += 1;
            let existing_windows = tmux.list_windows(Some(existing_id)).map_err(|err| {
                format!("failed to inspect `{}` windows: {err}", snap_session.name)
            })?;
            let existing_names: Vec<String> =
                existing_windows.into_iter().map(|w| w.name).collect();

            for snap_win in &snap_session.windows {
                if !snap_win.name.is_empty() && existing_names.iter().any(|n| n == &snap_win.name) {
                    report.warnings.push(format!(
                        "session `{}` already has window `{}`; skipped",
                        snap_session.name, snap_win.name
                    ));
                    continue;
                }
                restore_new_window(
                    tmux,
                    existing_id,
                    &snap_session.path,
                    snap_win,
                    &fallback_dir,
                    &mut report,
                )?;
            }
            continue;
        }

        // Brand-new session: pick initial cwd from first pane (or session path).
        let first_raw_cwd = snap_session
            .windows
            .first()
            .and_then(|w| w.panes.first())
            .map(|p| p.cwd.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(snap_session.path.as_str());
        let first_cwd = resolve_restore_cwd(first_raw_cwd, &fallback_dir, &mut report.warnings);

        let created_session = tmux
            .new_session(&snap_session.name, &first_cwd)
            .map_err(|err| format!("failed to create session `{}`: {err}", snap_session.name))?;
        report.sessions_created += 1;

        if snap_session.windows.is_empty() {
            continue;
        }

        // Configure the initial window created with the session.
        let initial_windows = tmux
            .list_windows(Some(&created_session.id))
            .map_err(|err| format!("failed to list initial window: {err}"))?;
        let Some(first_window) = initial_windows.into_iter().next() else {
            return Err(format!(
                "newly created session `{}` reported no initial window",
                snap_session.name
            ));
        };
        let first_snap_win = &snap_session.windows[0];
        if !first_snap_win.name.is_empty()
            && first_window.name != first_snap_win.name
            && let Err(err) = tmux.rename_window(&first_window.id, &first_snap_win.name)
        {
            report.warnings.push(format!(
                "could not rename initial window to `{}`: {err}",
                first_snap_win.name
            ));
        }
        report.windows_created += 1;

        let initial_panes = tmux
            .list_panes(Some(&first_window.id))
            .map_err(|err| format!("failed to list initial pane: {err}"))?;
        if let Some(first_pane) = initial_panes.into_iter().next() {
            report.panes_created += 1;
            populate_window_panes(
                tmux,
                &first_window.id,
                &first_pane.id,
                &snap_session.path,
                first_snap_win,
                &fallback_dir,
                &mut report,
            )?;
        }

        // Create remaining windows in this new session.
        for snap_win in snap_session.windows.iter().skip(1) {
            restore_new_window(
                tmux,
                &created_session.id,
                &snap_session.path,
                snap_win,
                &fallback_dir,
                &mut report,
            )?;
        }
    }

    Ok(report)
}

fn restore_new_window(
    tmux: &dyn Tmux,
    session_id: &SessionId,
    session_path: &str,
    snap_win: &SnapshotWindow,
    fallback_dir: &Path,
    report: &mut RestoreReport,
) -> Result<(), String> {
    let first_raw_cwd = snap_win
        .panes
        .first()
        .map(|p| p.cwd.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(session_path);
    let first_cwd = resolve_restore_cwd(first_raw_cwd, fallback_dir, &mut report.warnings);
    let win_name = (!snap_win.name.is_empty()).then_some(snap_win.name.as_str());

    let window = tmux
        .new_window(session_id, win_name, Some(&first_cwd))
        .map_err(|err| format!("failed to create window `{}`: {err}", snap_win.name))?;
    report.windows_created += 1;

    let panes = tmux
        .list_panes(Some(&window.id))
        .map_err(|err| format!("failed to list panes for window `{}`: {err}", snap_win.name))?;
    let Some(first_pane) = panes.into_iter().next() else {
        return Ok(());
    };
    report.panes_created += 1;

    populate_window_panes(
        tmux,
        &window.id,
        &first_pane.id,
        session_path,
        snap_win,
        fallback_dir,
        report,
    )
}

fn populate_window_panes(
    tmux: &dyn Tmux,
    window_id: &crate::tmux::WindowId,
    first_pane_id: &PaneId,
    session_path: &str,
    snap_win: &SnapshotWindow,
    fallback_dir: &Path,
    report: &mut RestoreReport,
) -> Result<(), String> {
    let mut active_pane_id: Option<PaneId> = None;

    if let Some(first_snap_pane) = snap_win.panes.first() {
        if !first_snap_pane.title.is_empty()
            && let Err(err) = tmux.rename_pane_title(first_pane_id, &first_snap_pane.title)
        {
            report.warnings.push(format!(
                "could not set title `{}` on pane {first_pane_id}: {err}",
                first_snap_pane.title
            ));
        }
        if first_snap_pane.active {
            active_pane_id = Some(first_pane_id.clone());
        }
    }

    let mut last_pane_id = first_pane_id.clone();
    for (idx, snap_pane) in snap_win.panes.iter().enumerate().skip(1) {
        let raw_cwd = if snap_pane.cwd.is_empty() {
            session_path
        } else {
            snap_pane.cwd.as_str()
        };
        let pane_cwd = resolve_restore_cwd(raw_cwd, fallback_dir, &mut report.warnings);
        let primary_dir = if idx % 2 == 1 {
            SplitDirection::SideBySide
        } else {
            SplitDirection::Stacked
        };
        let alt_dir = if primary_dir == SplitDirection::SideBySide {
            SplitDirection::Stacked
        } else {
            SplitDirection::SideBySide
        };

        // Try splitting; if tmux runs low on space in a narrow detached window,
        // rebalance to tiled and retry so multi-pane restores succeed reliably.
        let split_res = match tmux.split_pane_in(&last_pane_id, primary_dir, Some(&pane_cwd)) {
            Ok(p) => Ok(p),
            Err(_) => {
                let _ = tmux.select_layout(window_id, LayoutPreset::Tiled);
                tmux.split_pane_in(first_pane_id, alt_dir, Some(&pane_cwd))
            }
        };

        match split_res {
            Ok(pane) => {
                report.panes_created += 1;
                if !snap_pane.title.is_empty()
                    && let Err(err) = tmux.rename_pane_title(&pane.id, &snap_pane.title)
                {
                    report.warnings.push(format!(
                        "could not set title `{}` on pane {}: {err}",
                        snap_pane.title, pane.id
                    ));
                }
                if snap_pane.active {
                    active_pane_id = Some(pane.id.clone());
                }
                last_pane_id = pane.id;
            }
            Err(err) => {
                report.warnings.push(format!(
                    "could not create pane #{} in window `{}`: {err}",
                    idx + 1,
                    snap_win.name
                ));
            }
        }
    }

    if !snap_win.layout.is_empty()
        && snap_win.panes.len() > 1
        && let Err(err) = tmux.select_layout_raw(window_id, &snap_win.layout)
    {
        report.warnings.push(format!(
            "could not restore exact layout for window `{}` ({err}); applied tiled layout",
            snap_win.name
        ));
        let _ = tmux.select_layout(window_id, LayoutPreset::Tiled);
    }

    if snap_win.synchronized
        && let Err(err) = tmux.set_synchronize_panes(window_id, true)
    {
        report.warnings.push(format!(
            "could not enable synchronize-panes on window `{}`: {err}",
            snap_win.name
        ));
    }

    if let Some(pane_id) = active_pane_id {
        let _ = tmux.select(&Target::Pane(pane_id));
    }

    Ok(())
}

fn fallback_working_dir() -> PathBuf {
    if let Some(home) = env::home_dir().filter(|p| p.is_dir()) {
        return home;
    }
    if let Ok(cwd) = env::current_dir()
        && cwd.is_dir()
    {
        return cwd;
    }
    PathBuf::from(".")
}

fn resolve_restore_cwd(raw: &str, fallback_dir: &Path, warnings: &mut Vec<String>) -> PathBuf {
    if raw.is_empty() {
        return fallback_dir.to_path_buf();
    }
    let candidate = Path::new(raw);
    if candidate.is_dir() {
        candidate.to_path_buf()
    } else {
        warnings.push(format!(
            "directory `{raw}` does not exist; using `{}`",
            fallback_dir.display()
        ));
        fallback_dir.to_path_buf()
    }
}

fn quote_toml_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn parse_toml_string(raw: &str, line_no: usize) -> Result<String, String> {
    if let Some(rest) = raw.strip_prefix('"') {
        let mut out = String::with_capacity(rest.len());
        let mut chars = rest.char_indices();
        while let Some((idx, ch)) = chars.next() {
            match ch {
                '"' => {
                    if idx + ch.len_utf8() != rest.len() {
                        return Err(format!(
                            "line {line_no}: unexpected trailing characters after string"
                        ));
                    }
                    return Ok(out);
                }
                '\\' => {
                    let Some((_, esc)) = chars.next() else {
                        return Err(format!("line {line_no}: unterminated string literal"));
                    };
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'u' => out.push(parse_unicode_escape(&mut chars, 4, line_no)?),
                        'U' => out.push(parse_unicode_escape(&mut chars, 8, line_no)?),
                        _ => {
                            return Err(format!(
                                "line {line_no}: unsupported escape sequence `\\{esc}`"
                            ));
                        }
                    }
                }
                other => out.push(other),
            }
        }
        return Err(format!("line {line_no}: unterminated string literal"));
    }

    if let Some(rest) = raw.strip_prefix('\'') {
        let Some(end) = rest.find('\'') else {
            return Err(format!("line {line_no}: unterminated string literal"));
        };
        if end + 1 != rest.len() {
            return Err(format!(
                "line {line_no}: unexpected trailing characters after string"
            ));
        }
        return Ok(rest[..end].to_owned());
    }

    Err(format!(
        "line {line_no}: expected quoted string, found `{raw}`"
    ))
}

fn parse_unicode_escape(
    chars: &mut std::str::CharIndices<'_>,
    digits: usize,
    line_no: usize,
) -> Result<char, String> {
    let mut hex = String::with_capacity(digits);
    for _ in 0..digits {
        let Some((_, ch)) = chars.next() else {
            return Err(format!("line {line_no}: incomplete unicode escape"));
        };
        hex.push(ch);
    }
    let code = u32::from_str_radix(&hex, 16)
        .map_err(|_| format!("line {line_no}: invalid unicode escape `\\u{hex}`"))?;
    char::from_u32(code)
        .ok_or_else(|| format!("line {line_no}: invalid unicode scalar value `0x{hex}`"))
}

fn parse_toml_bool(raw: &str, key: &str, line_no: usize) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "line {line_no}: invalid boolean for `{key}`: `{raw}`"
        )),
    }
}

fn parse_toml_u64(raw: &str, key: &str, line_no: usize) -> Result<u64, String> {
    if raw.is_empty()
        || raw.starts_with('_')
        || raw.ends_with('_')
        || raw.contains("__")
        || !raw.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "line {line_no}: invalid unsigned integer for `{key}`: `{raw}`"
        ));
    }
    let compact: String = raw.chars().filter(|&c| c != '_').collect();
    compact
        .parse::<u64>()
        .map_err(|_| format!("line {line_no}: integer out of range for `{key}`: `{raw}`"))
}

fn format_utc_datetime(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::tmux::{SocketSpec, TmuxClient};

    fn sample_snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            name: "full-stack".to_owned(),
            created_epoch: 1_763_880_000,
            sessions: vec![
                SnapshotSession {
                    name: "backend".to_owned(),
                    path: r#"/Users/dev/my "app"\src"#.to_owned(),
                    windows: vec![
                        SnapshotWindow {
                            name: "editor".to_owned(),
                            active: true,
                            layout: "bb62,159x48,0,0{79x48,0,0,1,79x48,80,0,2}".to_owned(),
                            synchronized: false,
                            panes: vec![
                                SnapshotPane {
                                    cwd: r#"/Users/dev/my "app"\src"#.to_owned(),
                                    title: "nvim \"main\"\nline2".to_owned(),
                                    active: true,
                                    command: "nvim".to_owned(),
                                },
                                SnapshotPane {
                                    cwd: "/Users/dev/logs#tail".to_owned(),
                                    title: "logs\t#1".to_owned(),
                                    active: false,
                                    command: "tail".to_owned(),
                                },
                            ],
                        },
                        SnapshotWindow {
                            name: "cluster".to_owned(),
                            active: false,
                            layout: "even-horizontal".to_owned(),
                            synchronized: true,
                            panes: vec![SnapshotPane {
                                cwd: "/tmp".to_owned(),
                                title: "node-1".to_owned(),
                                active: true,
                                command: "zsh".to_owned(),
                            }],
                        },
                    ],
                },
                SnapshotSession {
                    name: "docs".to_owned(),
                    path: "/Users/dev/docs".to_owned(),
                    windows: vec![],
                },
            ],
        }
    }

    #[test]
    fn toml_round_trip_preserves_escapes_unicode_and_structure() {
        let original = sample_snapshot();
        let serialized = original.to_toml();
        assert!(serialized.contains("version = 1"));
        assert!(serialized.contains("[[sessions]]"));
        assert!(serialized.contains("[[sessions.windows]]"));
        assert!(serialized.contains("[[sessions.windows.panes]]"));

        let parsed = WorkspaceSnapshot::from_toml(&serialized).expect("reparse snapshot");
        assert_eq!(parsed, original);
        assert_eq!(parsed.session_count(), 2);
        assert_eq!(parsed.window_count(), 2);
        assert_eq!(parsed.pane_count(), 3);

        // Tree preview renders sessions, windows, sync badges, and panes cleanly.
        let preview = parsed.format_tree_preview();
        let joined = preview.join("\n");
        assert!(joined.contains("full-stack"), "{joined}");
        assert!(joined.contains("$ backend"), "{joined}");
        assert!(joined.contains("@ cluster [SYNC]"), "{joined}");
    }

    #[test]
    fn parser_rejects_malformed_snapshot_toml() {
        for bad in [
            "created_epoch = 10\n",
            "name = \"\"\n",
            "version = 99\nname = \"x\"\n",
            "name = \"x\"\n[[sessions.windows]]\nname = \"w\"\n",
            "name = \"x\"\n[[sessions]]\n[[sessions.windows.panes]]\n",
            "name = \"x\"\n[[unknown]]\n",
            "name = \"unterminated\n",
            "name = \"x\"\ncreated_epoch = -5\n",
        ] {
            assert!(from_toml(bad).is_err(), "expected error for {bad:?}");
        }
    }

    #[test]
    fn validate_name_and_storage_crud_in_tempdir() {
        for ok in ["dev", "my-workspace_1", "工作区 2", "v1.2"] {
            assert!(validate_snapshot_name(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "",
            "   ",
            " lead",
            "../escape",
            ".hidden",
            "a/b",
            "a\\b",
            "a:b",
            "bad\nname",
        ] {
            assert!(validate_snapshot_name(bad).is_err(), "{bad:?}");
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        assert_eq!(list_snapshots(&missing).expect("missing list"), Vec::new());
        assert!(!missing.exists(), "list_snapshots must not create dir");

        let mut older = sample_snapshot();
        older.name = "alpha".to_owned();
        older.created_epoch = 1_700_000_000;
        let mut newer = sample_snapshot();
        newer.name = "beta".to_owned();
        newer.created_epoch = 1_760_000_000;

        let p1 = save_snapshot(dir.path(), &older).expect("save alpha");
        let p2 = save_snapshot(dir.path(), &newer).expect("save beta");
        assert!(p1.exists() && p2.exists());

        // Stray non-toml or corrupt toml files are gracefully ignored by list_snapshots.
        fs::write(dir.path().join("notes.txt"), "hello").expect("write stray");
        fs::write(dir.path().join("broken.toml"), "not valid = = =\n").expect("write broken");

        let list = list_snapshots(dir.path()).expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "beta");
        assert_eq!(list[1].name, "alpha");
        assert_eq!(list[0].session_count, 2);
        assert_eq!(list[0].window_count, 2);
        assert_eq!(list[0].pane_count, 3);

        assert_eq!(load_snapshot(dir.path(), "alpha").expect("load"), older);
        assert!(delete_snapshot(dir.path(), "alpha").expect("delete alpha"));
        assert!(!delete_snapshot(dir.path(), "alpha").expect("delete again"));
        assert_eq!(
            list_snapshots(dir.path()).expect("list after delete").len(),
            1
        );
    }

    #[test]
    fn resolve_snapshot_dir_precedence() {
        let custom = PathBuf::from("/custom/snaps");
        let xdg = PathBuf::from("/xdg/config");
        let rel = PathBuf::from("rel/xdg");
        let home = PathBuf::from("/home/tester");

        assert_eq!(
            resolve_snapshot_dir_with(Some(&custom), Some(&xdg), Some(&home)),
            Some(custom)
        );
        assert_eq!(
            resolve_snapshot_dir_with(None, Some(&xdg), Some(&home)),
            Some(xdg.join("justmux/snapshots"))
        );
        assert_eq!(
            resolve_snapshot_dir_with(None, Some(&rel), Some(&home)),
            Some(home.join(".config/justmux/snapshots"))
        );
        assert_eq!(resolve_snapshot_dir_with(None, None, None), None);
    }

    struct PrivateTmuxServer {
        socket_name: String,
        client: TmuxClient,
    }

    impl PrivateTmuxServer {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let socket_name = format!("justmux-snap-test-{}-{n}", std::process::id());
            let client = TmuxClient::new(SocketSpec::Name(socket_name.clone()))
                .with_config_file(PathBuf::from("/dev/null"));
            Self {
                socket_name,
                client,
            }
        }
    }

    impl Drop for PrivateTmuxServer {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .args(["-L", &self.socket_name, "-f", "/dev/null", "kill-server"])
                .output();
        }
    }

    #[test]
    fn capture_save_and_restore_round_trip_on_private_tmux_socket() {
        let server = PrivateTmuxServer::new();
        if server.client.version().is_err() {
            return;
        }

        let work = tempfile::tempdir().expect("tempdir");
        let dir_a = work.path().join("dir_a");
        let dir_b = work.path().join("dir_b");
        let snap_dir = work.path().join("snapshots");
        fs::create_dir_all(&dir_a).expect("mkdir a");
        fs::create_dir_all(&dir_b).expect("mkdir b");

        let sess = server
            .client
            .new_session("snap-work", &dir_a)
            .expect("new session");
        let w2 = server
            .client
            .new_window(&sess.id, Some("pair"), Some(&dir_a))
            .expect("new window");
        let panes = server.client.list_panes(Some(&w2.id)).expect("list panes");
        let p0 = &panes[0].id;
        server
            .client
            .rename_pane_title(p0, "left-pane")
            .expect("title p0");
        let p1 = server
            .client
            .split_pane_in(p0, SplitDirection::SideBySide, Some(&dir_b))
            .expect("split pane in dir_b");
        server
            .client
            .rename_pane_title(&p1.id, "right-pane")
            .expect("title p1");
        server
            .client
            .set_synchronize_panes(&w2.id, true)
            .expect("sync on");

        let captured =
            capture_snapshot(&server.client, "my-dev", Some(&sess.id)).expect("capture snapshot");
        assert_eq!(captured.sessions.len(), 1);
        assert_eq!(captured.sessions[0].windows.len(), 2);
        let pair_win = captured.sessions[0]
            .windows
            .iter()
            .find(|w| w.name == "pair")
            .expect("pair window");
        assert!(pair_win.synchronized);
        assert!(!pair_win.layout.is_empty());
        assert_eq!(pair_win.panes.len(), 2);

        save_snapshot(&snap_dir, &captured).expect("save snapshot");
        let mut loaded = load_snapshot(&snap_dir, "my-dev").expect("load snapshot");

        // Kill the original session, remove one saved cwd to verify graceful fallback warning,
        // and restore onto the private server.
        server.client.kill_session(&sess.id).expect("kill original");
        let missing_dir = work.path().join("deleted_dir");
        loaded.sessions[0].windows[1].panes[1].cwd = missing_dir.display().to_string();

        let report = restore_snapshot(&server.client, &loaded).expect("restore snapshot");
        assert_eq!(report.sessions_created, 1);
        assert_eq!(report.sessions_reused, 0);
        assert_eq!(report.windows_created, 2);
        assert_eq!(report.panes_created, 3);
        assert!(
            report.warnings.iter().any(|w| w.contains("does not exist")),
            "expected missing cwd warning, got {:?}",
            report.warnings
        );

        let restored_sessions = server.client.list_sessions().expect("list restored");
        assert_eq!(restored_sessions.len(), 1);
        assert_eq!(restored_sessions[0].name, "snap-work");

        let restored_windows = server
            .client
            .list_windows(Some(&restored_sessions[0].id))
            .expect("list restored windows");
        assert_eq!(restored_windows.len(), 2);
        let restored_pair = restored_windows
            .iter()
            .find(|w| w.name == "pair")
            .expect("restored pair");
        assert!(restored_pair.synchronized);
        assert_eq!(restored_pair.panes, 2);

        // Restoring again reuses the existing session without duplicating windows.
        let second = restore_snapshot(&server.client, &loaded).expect("idempotent restore");
        assert_eq!(second.sessions_created, 0);
        assert_eq!(second.sessions_reused, 1);
        assert_eq!(second.windows_created, 0);
    }
}
