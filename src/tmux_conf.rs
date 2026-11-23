//! Managed `~/.tmux.conf` popup keybinding and comfort setting helpers.
//!
//! All reads/writes are strictly opt-in and only modify clearly delimited
//! JustMux blocks (`MANAGED_BLOCK_START` .. `MANAGED_BLOCK_END` for the popup
//! keybind, and `COMFORT_BLOCK_START` .. `COMFORT_BLOCK_END` for beginner-friendly
//! tmux ergonomic defaults), preserving every other line of the user's tmux
//! configuration verbatim.

use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

pub const MANAGED_BLOCK_START: &str = "# >>> justmux managed popup keybind >>>";
pub const MANAGED_BLOCK_END: &str = "# <<< justmux managed popup keybind <<<";
pub const COMFORT_BLOCK_START: &str = "# >>> justmux managed tmux comfort >>>";
pub const COMFORT_BLOCK_END: &str = "# <<< justmux managed tmux comfort <<<";
pub const POPUP_KEY: &str = "j";
pub const TMUX_CONF_ENV: &str = "JUSTMUX_TMUX_CONF";
pub const HISTORY_LIMIT_PRESETS: &[u32] = &[0, 10_000, 50_000, 100_000];

/// Opt-in beginner-friendly tmux ergonomic settings managed inside
/// [`COMFORT_BLOCK_START`] .. [`COMFORT_BLOCK_END`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TmuxComfortOptions {
    /// Enable mouse scroll, click-to-focus, and drag-to-resize (`set -g mouse on`).
    pub mouse: bool,
    /// Scrollback buffer size (`0` = unmanaged/off; positive emits `set -g history-limit <N>`).
    pub history_limit: u32,
    /// Start window and pane numbering at 1 (`set -g base-index 1` and `set -gw pane-base-index 1`).
    pub base_index_one: bool,
    /// Automatically renumber windows when one closes (`set -g renumber-windows on`).
    pub renumber_windows: bool,
    /// Eliminate Esc key delay for editors like Vim/Neovim (`set -sg escape-time 0`).
    pub zero_escape_time: bool,
}

/// Scope of a live tmux option assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxOptionScope {
    /// `set-option -g <option> <value>` (`set -g`)
    Global,
    /// `set-window-option -g <option> <value>` (`set -gw`)
    GlobalWindow,
    /// `set-option -sg <option> <value>` (`set -sg`)
    Server,
}

/// One tmux option assignment ready to be applied live to a running tmux server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxOptionCommand {
    pub scope: TmuxOptionScope,
    pub option: &'static str,
    pub value: String,
}

impl TmuxComfortOptions {
    /// Report whether any comfort option is currently enabled.
    pub fn is_any_enabled(&self) -> bool {
        self.mouse
            || self.history_limit > 0
            || self.base_index_one
            || self.renumber_windows
            || self.zero_escape_time
    }

    /// Count how many comfort options are currently enabled (0..=5).
    pub fn enabled_count(&self) -> usize {
        usize::from(self.mouse)
            + usize::from(self.history_limit > 0)
            + usize::from(self.base_index_one)
            + usize::from(self.renumber_windows)
            + usize::from(self.zero_escape_time)
    }

    /// Cycle through standard `history-limit` presets (`0 -> 10_000 -> 50_000 -> 100_000 -> 0`).
    pub fn next_history_limit_preset(current: u32) -> u32 {
        for &preset in HISTORY_LIMIT_PRESETS {
            if preset > current {
                return preset;
            }
        }
        0
    }

    /// Build the delimited JustMux-managed comfort block for `tmux.conf`, or
    /// `None` when every comfort option is disabled/unmanaged.
    pub fn managed_comfort_block(&self) -> Option<String> {
        let mut lines = Vec::new();
        if self.mouse {
            lines.push("set -g mouse on".to_owned());
        }
        if self.history_limit > 0 {
            lines.push(format!("set -g history-limit {}", self.history_limit));
        }
        if self.base_index_one {
            lines.push("set -g base-index 1".to_owned());
            lines.push("set -gw pane-base-index 1".to_owned());
        }
        if self.renumber_windows {
            lines.push("set -g renumber-windows on".to_owned());
        }
        if self.zero_escape_time {
            lines.push("set -sg escape-time 0".to_owned());
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "{COMFORT_BLOCK_START}\n{}\n{COMFORT_BLOCK_END}",
            lines.join("\n")
        ))
    }

    /// Return the live tmux option commands corresponding to all enabled comfort options.
    pub fn live_tmux_commands(&self) -> Vec<TmuxOptionCommand> {
        let mut cmds = Vec::new();
        if self.mouse {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "mouse",
                value: "on".to_owned(),
            });
        }
        if self.history_limit > 0 {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "history-limit",
                value: self.history_limit.to_string(),
            });
        }
        if self.base_index_one {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "base-index",
                value: "1".to_owned(),
            });
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::GlobalWindow,
                option: "pane-base-index",
                value: "1".to_owned(),
            });
        }
        if self.renumber_windows {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "renumber-windows",
                value: "on".to_owned(),
            });
        }
        if self.zero_escape_time {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Server,
                option: "escape-time",
                value: "0".to_owned(),
            });
        }
        cmds
    }

    /// Return only the live tmux option commands needed to transition from `previous`
    /// to `self` (setting newly enabled/updated options and restoring tmux built-in
    /// defaults for options that were explicitly turned off).
    pub fn live_tmux_transition_commands(&self, previous: &Self) -> Vec<TmuxOptionCommand> {
        let mut cmds = Vec::new();
        if self.mouse != previous.mouse {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "mouse",
                value: if self.mouse { "on" } else { "off" }.to_owned(),
            });
        }
        if self.history_limit != previous.history_limit {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "history-limit",
                value: if self.history_limit > 0 {
                    self.history_limit.to_string()
                } else {
                    "2000".to_owned()
                },
            });
        }
        if self.base_index_one != previous.base_index_one {
            let val = if self.base_index_one { "1" } else { "0" };
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "base-index",
                value: val.to_owned(),
            });
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::GlobalWindow,
                option: "pane-base-index",
                value: val.to_owned(),
            });
        }
        if self.renumber_windows != previous.renumber_windows {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "renumber-windows",
                value: if self.renumber_windows { "on" } else { "off" }.to_owned(),
            });
        }
        if self.zero_escape_time != previous.zero_escape_time {
            cmds.push(TmuxOptionCommand {
                scope: TmuxOptionScope::Server,
                option: "escape-time",
                value: if self.zero_escape_time { "0" } else { "500" }.to_owned(),
            });
        }
        cmds
    }
}

/// Error returned while reading, validating, or updating a `tmux.conf` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxConfError {
    message: String,
}

impl TmuxConfError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TmuxConfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TmuxConfError {}

fn sanitize_fallback_exe(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if !matches!(name, "jm" | "justmux") {
        return None;
    }
    let text = path.to_str()?;
    if text.is_empty() || text.contains('\'') || text.chars().any(char::is_control) {
        return None;
    }
    Some(text.to_owned())
}

fn escape_posix_double_quotes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Detect the current executable path when running as `jm` or `justmux`.
///
/// Test runner binaries and paths containing single quotes or control characters
/// are ignored so generated `tmux.conf` commands stay safe and portable.
pub fn detect_fallback_exe() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    sanitize_fallback_exe(&exe).map(PathBuf::from)
}

/// Build the inner `run-shell` command string for the JustMux popup binding.
pub fn popup_run_shell_command(fallback_exe: Option<&Path>) -> String {
    if let Some(clean) = fallback_exe.and_then(sanitize_fallback_exe) {
        let escaped = escape_posix_double_quotes(&clean);
        format!(
            r#"JM=$(command -v jm 2>/dev/null || printf "%s" "{escaped}"); tmux -S #{{q:socket_path}} display-popup -E -w 80% -h 70% -c #{{q:client_name}} -d #{{q:pane_current_path}} "env JUSTMUX_CLIENT=#{{q:client_name}} JUSTMUX_PANE=#{{q:pane_id}} JUSTMUX_POPUP=1 \"$JM\"""#
        )
    } else {
        r#"tmux -S #{q:socket_path} display-popup -E -w 80% -h 70% -c #{q:client_name} -d #{q:pane_current_path} "env JUSTMUX_CLIENT=#{q:client_name} JUSTMUX_PANE=#{q:pane_id} JUSTMUX_POPUP=1 jm""#
            .to_owned()
    }
}

/// Build the single `bind-key` configuration line written into `tmux.conf`.
pub fn managed_bind_line(fallback_exe: Option<&Path>) -> String {
    format!(
        "bind-key {POPUP_KEY} run-shell '{}'",
        popup_run_shell_command(fallback_exe)
    )
}

/// Build the delimited JustMux-managed `tmux.conf` snippet.
pub fn managed_block(fallback_exe: Option<&Path>) -> String {
    format!(
        "{MANAGED_BLOCK_START}\n{}\n{MANAGED_BLOCK_END}",
        managed_bind_line(fallback_exe)
    )
}

/// Resolve the active `tmux.conf` path:
/// 1) `JUSTMUX_TMUX_CONF` if set and non-empty (`/dev/null` disables)
/// 2) `~/.tmux.conf` if it already exists
/// 3) `$XDG_CONFIG_HOME/tmux/tmux.conf` (if absolute and exists)
/// 4) `~/.config/tmux/tmux.conf` if it already exists
/// 5) default `~/.tmux.conf`
pub fn resolve_tmux_conf_path() -> Option<PathBuf> {
    let env_override = env::var_os(TMUX_CONF_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let home_dir = env::home_dir();
    resolve_tmux_conf_path_with(
        env_override.as_deref(),
        xdg_config_home.as_deref(),
        home_dir.as_deref(),
    )
}

fn resolve_tmux_conf_path_with(
    env_override: Option<&Path>,
    xdg_config_home: Option<&Path>,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = env_override.filter(|p| !p.as_os_str().is_empty()) {
        if path == Path::new("/dev/null") {
            return None;
        }
        return Some(path.to_path_buf());
    }
    if let Some(home) = home_dir {
        let dot_tmux_conf = home.join(".tmux.conf");
        if dot_tmux_conf.exists() {
            return Some(dot_tmux_conf);
        }
    }
    if let Some(xdg) = xdg_config_home.filter(|p| p.is_absolute()) {
        let xdg_conf = xdg.join("tmux").join("tmux.conf");
        if xdg_conf.exists() {
            return Some(xdg_conf);
        }
    }
    if let Some(home) = home_dir {
        let xdg_default = home.join(".config").join("tmux").join("tmux.conf");
        if xdg_default.exists() {
            return Some(xdg_default);
        }
        return Some(home.join(".tmux.conf"));
    }
    None
}

fn scan_delimited_blocks(
    lines: &[&str],
    start_marker: &str,
    end_marker: &str,
) -> Result<Vec<(usize, usize)>, TmuxConfError> {
    let mut blocks = Vec::new();
    let mut open_start: Option<usize> = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == start_marker {
            if open_start.is_some() {
                return Err(TmuxConfError::new(format!(
                    "nested `{start_marker}` marker on line {}",
                    idx + 1
                )));
            }
            open_start = Some(idx);
        } else if trimmed == end_marker {
            let Some(start) = open_start.take() else {
                return Err(TmuxConfError::new(format!(
                    "unexpected `{end_marker}` marker on line {}",
                    idx + 1
                )));
            };
            blocks.push((start, idx));
        }
    }

    if let Some(start) = open_start {
        return Err(TmuxConfError::new(format!(
            "unclosed `{start_marker}` marker on line {}",
            start + 1
        )));
    }

    Ok(blocks)
}

fn apply_delimited_block(
    content: &str,
    start_marker: &str,
    end_marker: &str,
    block: &str,
) -> Result<String, TmuxConfError> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let blocks = scan_delimited_blocks(&lines, start_marker, end_marker)?;

    if blocks.is_empty() {
        if content.is_empty() {
            return Ok(format!("{block}\n"));
        }
        let mut out = String::with_capacity(content.len() + block.len() + 3);
        out.push_str(content);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if lines
            .last()
            .is_some_and(|line| !line.trim_end_matches(['\r', '\n']).is_empty())
        {
            out.push('\n');
        }
        out.push_str(block);
        out.push('\n');
        return Ok(out);
    }

    let mut out = String::with_capacity(content.len() + block.len() + 2);
    let mut idx = 0;
    while idx < lines.len() {
        if idx == blocks[0].0 {
            out.push_str(block);
            out.push('\n');
            idx = blocks[0].1 + 1;
            continue;
        }
        if let Some(&(_, dup_end)) = blocks.iter().skip(1).find(|&&(start, _)| start == idx) {
            idx = dup_end + 1;
            continue;
        }
        out.push_str(lines[idx]);
        idx += 1;
    }
    Ok(out)
}

fn remove_delimited_blocks(
    content: &str,
    start_marker: &str,
    end_marker: &str,
) -> Result<(String, bool), TmuxConfError> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let blocks = scan_delimited_blocks(&lines, start_marker, end_marker)?;
    if blocks.is_empty() {
        return Ok((content.to_owned(), false));
    }

    let mut skip = vec![false; lines.len()];
    for &(start, end) in &blocks {
        for flag in &mut skip[start..=end] {
            *flag = true;
        }
        if start > 0
            && !skip[start - 1]
            && lines[start - 1].trim_end_matches(['\r', '\n']).is_empty()
        {
            skip[start - 1] = true;
        } else if start == 0
            && end + 1 < lines.len()
            && !skip[end + 1]
            && lines[end + 1].trim_end_matches(['\r', '\n']).is_empty()
        {
            skip[end + 1] = true;
        }
    }

    let mut out = String::with_capacity(content.len());
    for (idx, line) in lines.iter().enumerate() {
        if !skip[idx] {
            out.push_str(line);
        }
    }
    if out.is_empty() {
        return Ok((out, true));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok((out, true))
}

/// Report whether `content` contains a valid JustMux managed popup keybind block.
pub fn has_managed_block(content: &str) -> Result<bool, TmuxConfError> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let blocks = scan_delimited_blocks(&lines, MANAGED_BLOCK_START, MANAGED_BLOCK_END)?;
    Ok(blocks.iter().any(|&(start, end)| {
        lines[start + 1..end].iter().any(|line| {
            let trimmed = line.trim_end_matches(['\r', '\n']).trim();
            trimmed.starts_with("bind-key ") || trimmed.starts_with("bind ")
        })
    }))
}

/// Insert or update the JustMux managed popup keybind block inside `content`.
pub fn apply_managed_block(
    content: &str,
    fallback_exe: Option<&Path>,
) -> Result<String, TmuxConfError> {
    let block = managed_block(fallback_exe);
    apply_delimited_block(content, MANAGED_BLOCK_START, MANAGED_BLOCK_END, &block)
}

/// Remove every JustMux managed popup keybind block from `content`.
pub fn remove_managed_block(content: &str) -> Result<(String, bool), TmuxConfError> {
    remove_delimited_blocks(content, MANAGED_BLOCK_START, MANAGED_BLOCK_END)
}

/// Parse the JustMux managed comfort block from `content`.
///
/// Only directives inside [`COMFORT_BLOCK_START`] .. [`COMFORT_BLOCK_END`] are
/// inspected; user settings outside the managed block are left untouched.
pub fn parse_comfort_block(content: &str) -> Result<TmuxComfortOptions, TmuxConfError> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let blocks = scan_delimited_blocks(&lines, COMFORT_BLOCK_START, COMFORT_BLOCK_END)?;
    let mut opts = TmuxComfortOptions::default();

    for &(start, end) in &blocks {
        for line in &lines[start + 1..end] {
            let trimmed = line.trim_end_matches(['\r', '\n']).trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            match tokens.as_slice() {
                ["set" | "set-option", "-g", "mouse", "on"] => {
                    opts.mouse = true;
                }
                ["set" | "set-option", "-g", "history-limit", raw_val] => {
                    if let Ok(limit) = raw_val.parse::<u32>() {
                        opts.history_limit = limit;
                    }
                }
                ["set" | "set-option", "-g", "base-index", "1"]
                | ["set" | "set-option", "-gw", "pane-base-index", "1"]
                | ["setw" | "set-window-option", "-g", "pane-base-index", "1"] => {
                    opts.base_index_one = true;
                }
                ["set" | "set-option", "-g", "renumber-windows", "on"] => {
                    opts.renumber_windows = true;
                }
                ["set" | "set-option", "-sg", "escape-time", "0"]
                | ["set" | "set-option", "-s", "-g", "escape-time", "0"] => {
                    opts.zero_escape_time = true;
                }
                _ => {}
            }
        }
    }

    Ok(opts)
}

/// Report whether `content` contains a JustMux managed comfort block with at least
/// one enabled setting.
pub fn has_comfort_block(content: &str) -> Result<bool, TmuxConfError> {
    Ok(parse_comfort_block(content)?.is_any_enabled())
}

/// Insert, update, or remove the JustMux managed comfort block inside `content`.
///
/// When all options in `opts` are disabled, any existing comfort block is removed
/// cleanly. Returns `(updated_content, changed)`.
pub fn apply_comfort_block(
    content: &str,
    opts: &TmuxComfortOptions,
) -> Result<(String, bool), TmuxConfError> {
    let Some(block) = opts.managed_comfort_block() else {
        return remove_comfort_block(content);
    };
    let updated = apply_delimited_block(content, COMFORT_BLOCK_START, COMFORT_BLOCK_END, &block)?;
    let changed = updated != content;
    Ok((updated, changed))
}

/// Remove every JustMux managed comfort block from `content`.
pub fn remove_comfort_block(content: &str) -> Result<(String, bool), TmuxConfError> {
    remove_delimited_blocks(content, COMFORT_BLOCK_START, COMFORT_BLOCK_END)
}

/// Read the currently managed comfort options from `path`.
/// Missing files cleanly return `Ok(TmuxComfortOptions::default())`.
pub fn read_comfort_block(path: &Path) -> Result<TmuxComfortOptions, TmuxConfError> {
    match fs::read_to_string(path) {
        Ok(content) => parse_comfort_block(&content),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(TmuxComfortOptions::default()),
        Err(err) => Err(TmuxConfError::new(format!(
            "could not read `{}`: {err}",
            path.display()
        ))),
    }
}

/// Synchronize the JustMux managed comfort block in `path` with `opts`.
///
/// - When all options in `opts` are off/default, any existing comfort block is
///   removed cleanly, and missing files are **never** created (returns `Ok(false)`).
/// - When at least one option is enabled, the comfort block is inserted or updated
///   in place without touching the popup block or user lines.
/// - Returns `Ok(true)` if the file was created or modified, `Ok(false)` if unchanged.
pub fn sync_comfort_block(path: &Path, opts: &TmuxComfortOptions) -> Result<bool, TmuxConfError> {
    if !opts.is_any_enabled() {
        let existing = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(TmuxConfError::new(format!(
                    "could not read `{}`: {err}",
                    path.display()
                )));
            }
        };
        let (updated, changed) = remove_comfort_block(&existing)?;
        if changed {
            fs::write(path, updated).map_err(|err| {
                TmuxConfError::new(format!("could not write `{}`: {err}", path.display()))
            })?;
        }
        return Ok(changed);
    }

    let (existing, existed) = match fs::read_to_string(path) {
        Ok(content) => (content, true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => (String::new(), false),
        Err(err) => {
            return Err(TmuxConfError::new(format!(
                "could not read `{}`: {err}",
                path.display()
            )));
        }
    };
    let (updated, changed) = apply_comfort_block(&existing, opts)?;
    if !existed || changed {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|err| {
                TmuxConfError::new(format!(
                    "could not create directory `{}`: {err}",
                    parent.display()
                ))
            })?;
        }
        fs::write(path, updated).map_err(|err| {
            TmuxConfError::new(format!("could not write `{}`: {err}", path.display()))
        })?;
        return Ok(true);
    }
    Ok(false)
}

/// Check whether `path` currently contains a JustMux managed popup keybind block.
/// Missing files cleanly return `Ok(false)`.
pub fn is_installed(path: &Path) -> Result<bool, TmuxConfError> {
    match fs::read_to_string(path) {
        Ok(content) => has_managed_block(&content),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(TmuxConfError::new(format!(
            "could not read `{}`: {err}",
            path.display()
        ))),
    }
}

/// Install or refresh the JustMux managed popup keybind block in `path`.
pub fn install(path: &Path, fallback_exe: Option<&Path>) -> Result<(), TmuxConfError> {
    let existing = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(TmuxConfError::new(format!(
                "could not read `{}`: {err}",
                path.display()
            )));
        }
    };
    let updated = apply_managed_block(&existing, fallback_exe)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| {
            TmuxConfError::new(format!(
                "could not create directory `{}`: {err}",
                parent.display()
            ))
        })?;
    }
    fs::write(path, updated)
        .map_err(|err| TmuxConfError::new(format!("could not write `{}`: {err}", path.display())))
}

/// Remove the JustMux managed popup keybind block from `path`, if present.
/// Missing files are never created and return `Ok(false)`.
pub fn uninstall(path: &Path) -> Result<bool, TmuxConfError> {
    let existing = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(TmuxConfError::new(format!(
                "could not read `{}`: {err}",
                path.display()
            )));
        }
    };
    let (updated, changed) = remove_managed_block(&existing)?;
    if changed {
        fs::write(path, updated).map_err(|err| {
            TmuxConfError::new(format!("could not write `{}`: {err}", path.display()))
        })?;
    }
    Ok(changed)
}

/// Apply a slice of [`TmuxOptionCommand`]s to a running tmux server via
/// `set_global_option`, `set_global_window_option`, and `set_server_option`.
///
/// Returns `Ok(0)` without error if no tmux server is currently running or
/// `commands` is empty; otherwise returns the number of options applied.
pub fn apply_comfort_commands_live(
    tmux: &dyn crate::tmux::Tmux,
    commands: &[TmuxOptionCommand],
) -> crate::tmux::TmuxResult<usize> {
    if commands.is_empty() || !tmux.server_running()? {
        return Ok(0);
    }
    let mut applied = 0;
    for cmd in commands {
        match cmd.scope {
            TmuxOptionScope::Global => tmux.set_global_option(cmd.option, &cmd.value)?,
            TmuxOptionScope::GlobalWindow => {
                tmux.set_global_window_option(cmd.option, &cmd.value)?;
            }
            TmuxOptionScope::Server => tmux.set_server_option(cmd.option, &cmd.value)?,
        }
        applied += 1;
    }
    Ok(applied)
}

/// Apply all enabled comfort options in `opts` live to a running tmux server.
///
/// Returns `Ok(0)` if no server is running or no options are enabled.
pub fn apply_comfort_live(
    tmux: &dyn crate::tmux::Tmux,
    opts: &TmuxComfortOptions,
) -> crate::tmux::TmuxResult<usize> {
    let cmds = opts.live_tmux_commands();
    apply_comfort_commands_live(tmux, &cmds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_uninstall_round_trip_preserves_user_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join(".tmux.conf");

        // Missing file starts uninstalled and uninstall does not create it.
        assert!(!is_installed(&conf).expect("check missing"));
        assert!(!uninstall(&conf).expect("uninstall missing"));
        assert!(!conf.exists());

        // Install into non-existent file.
        let exe = PathBuf::from("/opt/homebrew/bin/jm");
        install(&conf, Some(&exe)).expect("install new file");
        assert!(is_installed(&conf).expect("installed"));
        let created = fs::read_to_string(&conf).expect("read created");
        assert!(created.starts_with(MANAGED_BLOCK_START));
        assert!(created.contains("JUSTMUX_POPUP=1"));
        assert!(created.contains("/opt/homebrew/bin/jm"));

        // Uninstall from file containing only the managed block leaves it empty.
        assert!(uninstall(&conf).expect("uninstall created"));
        assert!(!is_installed(&conf).expect("uninstalled"));
        assert_eq!(fs::read_to_string(&conf).expect("read empty"), "");

        // Existing user config without trailing newline keeps user settings intact.
        let original = "set -g mouse on\nset -g prefix C-a";
        fs::write(&conf, original).expect("seed user conf");
        install(&conf, Some(&exe)).expect("install over existing");
        let after_install = fs::read_to_string(&conf).expect("read after install");
        assert!(after_install.starts_with("set -g mouse on\nset -g prefix C-a\n\n"));
        assert!(after_install.contains(MANAGED_BLOCK_START));
        assert!(after_install.contains(MANAGED_BLOCK_END));

        // Re-installing updates in place without duplicating the block.
        let updated_exe = PathBuf::from("/usr/local/bin/justmux");
        install(&conf, Some(&updated_exe)).expect("re-install");
        let after_update = fs::read_to_string(&conf).expect("read after update");
        assert_eq!(after_update.matches(MANAGED_BLOCK_START).count(), 1);
        assert_eq!(after_update.matches(MANAGED_BLOCK_END).count(), 1);
        assert!(after_update.contains("/usr/local/bin/justmux"));
        assert!(!after_update.contains("/opt/homebrew/bin/jm"));

        // Uninstalling removes the managed block and separator blank line cleanly.
        assert!(uninstall(&conf).expect("uninstall existing"));
        assert_eq!(
            fs::read_to_string(&conf).expect("read restored"),
            "set -g mouse on\nset -g prefix C-a\n"
        );
        assert!(!uninstall(&conf).expect("second uninstall is no-op"));
    }

    #[test]
    fn handles_crlf_and_malformed_markers_safely() {
        let crlf = format!(
            "set -g mouse on\r\n\r\n{MANAGED_BLOCK_START}\r\n{}\r\n{MANAGED_BLOCK_END}\r\n",
            managed_bind_line(None)
        );
        assert!(has_managed_block(&crlf).expect("valid crlf"));
        let (removed, changed) = remove_managed_block(&crlf).expect("remove crlf");
        assert!(changed);
        assert_eq!(removed, "set -g mouse on\r\n");

        for malformed in [
            format!("{MANAGED_BLOCK_START}\nbind-key j run-shell 'echo hi'\n"),
            format!("{MANAGED_BLOCK_END}\n"),
            format!("{MANAGED_BLOCK_START}\n{MANAGED_BLOCK_START}\n{MANAGED_BLOCK_END}\n"),
        ] {
            assert!(has_managed_block(&malformed).is_err(), "{malformed:?}");
            assert!(
                apply_managed_block(&malformed, None).is_err(),
                "{malformed:?}"
            );
            assert!(remove_managed_block(&malformed).is_err(), "{malformed:?}");
        }

        // Block without bind-key line reports not installed yet can still be repaired/removed.
        let empty_block = format!("{MANAGED_BLOCK_START}\n# empty\n{MANAGED_BLOCK_END}\n");
        assert!(!has_managed_block(&empty_block).expect("valid empty block"));
        let repaired = apply_managed_block(&empty_block, None).expect("repair empty block");
        assert!(has_managed_block(&repaired).expect("repaired"));
    }

    #[test]
    fn resolve_tmux_conf_path_respects_env_and_xdg_precedence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let xdg = dir.path().join("xdg");
        fs::create_dir_all(&home).expect("mkdir home");
        fs::create_dir_all(&xdg).expect("mkdir xdg");

        // Default fallback when neither file exists yet -> ~/.tmux.conf.
        assert_eq!(
            resolve_tmux_conf_path_with(None, Some(&xdg), Some(&home)),
            Some(home.join(".tmux.conf"))
        );

        // ~/.config/tmux/tmux.conf is picked if it exists and ~/.tmux.conf does not.
        let home_xdg = home.join(".config/tmux/tmux.conf");
        fs::create_dir_all(home_xdg.parent().expect("parent")).expect("mkdir .config/tmux");
        fs::write(&home_xdg, "set -g mouse on\n").expect("write home_xdg");
        assert_eq!(
            resolve_tmux_conf_path_with(None, None, Some(&home)),
            Some(home_xdg.clone())
        );

        // Absolute $XDG_CONFIG_HOME/tmux/tmux.conf wins over ~/.config/tmux/tmux.conf when present.
        let xdg_conf = xdg.join("tmux/tmux.conf");
        fs::create_dir_all(xdg_conf.parent().expect("parent")).expect("mkdir xdg tmux");
        fs::write(&xdg_conf, "set -g mouse on\n").expect("write xdg_conf");
        assert_eq!(
            resolve_tmux_conf_path_with(None, Some(&xdg), Some(&home)),
            Some(xdg_conf)
        );

        // Existing ~/.tmux.conf wins over XDG tmux.conf (matching tmux lookup order).
        let dot_tmux = home.join(".tmux.conf");
        fs::write(&dot_tmux, "set -g mouse on\n").expect("write ~/.tmux.conf");
        assert_eq!(
            resolve_tmux_conf_path_with(None, Some(&xdg), Some(&home)),
            Some(dot_tmux)
        );

        // JUSTMUX_TMUX_CONF overrides everything; /dev/null disables path resolution.
        let custom = dir.path().join("custom.tmux.conf");
        assert_eq!(
            resolve_tmux_conf_path_with(Some(&custom), Some(&xdg), Some(&home)),
            Some(custom)
        );
        assert_eq!(
            resolve_tmux_conf_path_with(Some(Path::new("/dev/null")), Some(&xdg), Some(&home)),
            None
        );
    }

    #[test]
    fn popup_command_sanitizes_and_escapes_fallback_executable() {
        let plain = popup_run_shell_command(None);
        assert!(plain.contains("tmux -S #{q:socket_path} display-popup -E"));
        assert!(plain.contains("-d #{q:pane_current_path}"));
        assert!(plain.contains("JUSTMUX_POPUP=1 jm"));

        // Escapes spaces, $, and " inside POSIX double quotes.
        let special = PathBuf::from(r#"/opt/my "dev" $dir/jm"#);
        let cmd = popup_run_shell_command(Some(&special));
        assert!(cmd.contains(r#"\"dev\""#), "{cmd}");
        assert!(cmd.contains(r#"\$dir"#), "{cmd}");
        assert!(cmd.contains(r#"\"$JM\""#), "{cmd}");

        // Rejects non-jm binaries, single quotes, and control chars.
        assert_eq!(
            popup_run_shell_command(Some(Path::new("/tmp/cargo-test-123"))),
            plain
        );
        assert_eq!(
            popup_run_shell_command(Some(Path::new("/tmp/o'reilly/jm"))),
            plain
        );
        assert_eq!(
            popup_run_shell_command(Some(Path::new("/tmp/bad\npath/jm"))),
            plain
        );
    }

    #[test]
    fn comfort_block_sync_read_and_coexistence_with_popup_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join(".tmux.conf");

        // All-off comfort options never create a missing ~/.tmux.conf.
        let all_off = TmuxComfortOptions::default();
        assert!(!all_off.is_any_enabled());
        assert_eq!(all_off.enabled_count(), 0);
        assert_eq!(all_off.managed_comfort_block(), None);
        assert!(!sync_comfort_block(&conf, &all_off).expect("sync off missing"));
        assert!(
            !conf.exists(),
            "must never create missing file when all off"
        );
        assert_eq!(read_comfort_block(&conf).expect("read missing"), all_off);

        // Seed user config + popup block, then enable comfort options.
        fs::write(&conf, "set -g prefix C-a\n").expect("seed user conf");
        let exe = PathBuf::from("/opt/homebrew/bin/jm");
        install(&conf, Some(&exe)).expect("install popup");

        let all_on = TmuxComfortOptions {
            mouse: true,
            history_limit: 50_000,
            base_index_one: true,
            renumber_windows: true,
            zero_escape_time: true,
        };
        assert!(all_on.is_any_enabled());
        assert_eq!(all_on.enabled_count(), 5);
        assert!(sync_comfort_block(&conf, &all_on).expect("sync all on"));
        // Second identical sync is a clean no-op.
        assert!(!sync_comfort_block(&conf, &all_on).expect("sync idempotent"));

        let text = fs::read_to_string(&conf).expect("read combined conf");
        assert!(text.contains("set -g prefix C-a"));
        assert!(text.contains(MANAGED_BLOCK_START));
        assert!(text.contains(COMFORT_BLOCK_START));
        assert!(text.contains("set -g mouse on"));
        assert!(text.contains("set -g history-limit 50000"));
        assert!(text.contains("set -g base-index 1"));
        assert!(text.contains("set -gw pane-base-index 1"));
        assert!(text.contains("set -g renumber-windows on"));
        assert!(text.contains("set -sg escape-time 0"));
        assert!(is_installed(&conf).expect("popup still installed"));
        assert_eq!(read_comfort_block(&conf).expect("read back"), all_on);

        // Updating comfort options updates in place without duplicating or touching popup block.
        let partial = TmuxComfortOptions {
            mouse: true,
            history_limit: 100_000,
            base_index_one: false,
            renumber_windows: false,
            zero_escape_time: true,
        };
        assert!(sync_comfort_block(&conf, &partial).expect("sync partial"));
        let updated = fs::read_to_string(&conf).expect("read updated");
        assert_eq!(updated.matches(COMFORT_BLOCK_START).count(), 1);
        assert_eq!(updated.matches(MANAGED_BLOCK_START).count(), 1);
        assert_eq!(read_comfort_block(&conf).expect("read partial"), partial);

        // Uninstalling popup block preserves comfort block and user lines.
        assert!(uninstall(&conf).expect("uninstall popup"));
        assert!(!is_installed(&conf).expect("popup removed"));
        assert_eq!(
            read_comfort_block(&conf).expect("comfort preserved"),
            partial
        );

        // Turning all comfort options off removes the comfort block cleanly.
        assert!(sync_comfort_block(&conf, &all_off).expect("remove comfort"));
        assert!(!sync_comfort_block(&conf, &all_off).expect("remove comfort again"));
        assert_eq!(
            fs::read_to_string(&conf).expect("read final"),
            "set -g prefix C-a\n"
        );
    }

    #[test]
    fn comfort_block_ignores_user_lines_and_validates_markers() {
        // User's own `set -g mouse on` outside the comfort block is NOT treated as managed.
        let user_only = "set -g mouse on\nset -sg escape-time 0\n";
        assert_eq!(
            parse_comfort_block(user_only).expect("parse user only"),
            TmuxComfortOptions::default()
        );
        assert!(!has_comfort_block(user_only).expect("has comfort"));

        // CRLF comfort block parses and removes cleanly.
        let crlf = format!(
            "set -g prefix C-a\r\n\r\n{COMFORT_BLOCK_START}\r\nset -g mouse on\r\n{COMFORT_BLOCK_END}\r\n"
        );
        assert!(has_comfort_block(&crlf).expect("crlf comfort"));
        let (cleaned, changed) = remove_comfort_block(&crlf).expect("remove crlf");
        assert!(changed);
        assert_eq!(cleaned, "set -g prefix C-a\r\n");

        // Malformed markers fail loudly rather than corrupting the file.
        for malformed in [
            format!("{COMFORT_BLOCK_START}\nset -g mouse on\n"),
            format!("{COMFORT_BLOCK_END}\n"),
            format!("{COMFORT_BLOCK_START}\n{COMFORT_BLOCK_START}\n{COMFORT_BLOCK_END}\n"),
        ] {
            assert!(parse_comfort_block(&malformed).is_err(), "{malformed:?}");
            assert!(
                apply_comfort_block(&malformed, &TmuxComfortOptions::default()).is_err(),
                "{malformed:?}"
            );
        }
    }

    #[test]
    fn comfort_live_commands_transitions_and_presets() {
        assert_eq!(TmuxComfortOptions::next_history_limit_preset(0), 10_000);
        assert_eq!(
            TmuxComfortOptions::next_history_limit_preset(10_000),
            50_000
        );
        assert_eq!(
            TmuxComfortOptions::next_history_limit_preset(25_000),
            50_000
        );
        assert_eq!(
            TmuxComfortOptions::next_history_limit_preset(50_000),
            100_000
        );
        assert_eq!(TmuxComfortOptions::next_history_limit_preset(100_000), 0);

        let all_on = TmuxComfortOptions {
            mouse: true,
            history_limit: 50_000,
            base_index_one: true,
            renumber_windows: true,
            zero_escape_time: true,
        };
        let cmds = all_on.live_tmux_commands();
        assert_eq!(cmds.len(), 6);
        assert_eq!(
            cmds[0],
            TmuxOptionCommand {
                scope: TmuxOptionScope::Global,
                option: "mouse",
                value: "on".into(),
            }
        );
        assert_eq!(
            cmds[3],
            TmuxOptionCommand {
                scope: TmuxOptionScope::GlobalWindow,
                option: "pane-base-index",
                value: "1".into(),
            }
        );
        assert_eq!(
            cmds[5],
            TmuxOptionCommand {
                scope: TmuxOptionScope::Server,
                option: "escape-time",
                value: "0".into(),
            }
        );

        // Turning off mouse & zero_escape_time emits reset transition commands only for changed fields.
        let next = TmuxComfortOptions {
            mouse: false,
            zero_escape_time: false,
            ..all_on
        };
        let diff = next.live_tmux_transition_commands(&all_on);
        assert_eq!(
            diff,
            vec![
                TmuxOptionCommand {
                    scope: TmuxOptionScope::Global,
                    option: "mouse",
                    value: "off".into(),
                },
                TmuxOptionCommand {
                    scope: TmuxOptionScope::Server,
                    option: "escape-time",
                    value: "500".into(),
                },
            ]
        );
    }

    #[test]
    fn apply_comfort_live_dispatches_all_option_scopes_and_skips_when_offline() {
        use std::cell::RefCell;
        use std::process::Command;

        use crate::tmux::{
            Context, Pane, PaneId, Session, SessionId, SplitDirection, Target, Tmux, TmuxError,
            TmuxResult, TmuxVersion, Window, WindowId,
        };

        struct OptionStubTmux {
            running: bool,
            calls: RefCell<Vec<(TmuxOptionScope, String, String)>>,
        }

        impl Tmux for OptionStubTmux {
            fn version(&self) -> TmuxResult<TmuxVersion> {
                Ok(TmuxVersion {
                    raw: "3.4".into(),
                    number: Some((3, 4)),
                })
            }
            fn server_running(&self) -> TmuxResult<bool> {
                Ok(self.running)
            }
            fn context(&self) -> Context {
                Context::Outside
            }
            fn list_sessions(&self) -> TmuxResult<Vec<Session>> {
                Ok(Vec::new())
            }
            fn has_session(&self, _: &SessionId) -> TmuxResult<bool> {
                Ok(false)
            }
            fn new_session(&self, _: &str, _: &Path) -> TmuxResult<Session> {
                Err(TmuxError::NoServer)
            }
            fn rename_session(&self, _: &SessionId, _: &str) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn kill_session(&self, _: &SessionId) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn list_windows(&self, _: Option<&SessionId>) -> TmuxResult<Vec<Window>> {
                Ok(Vec::new())
            }
            fn new_window(
                &self,
                _: &SessionId,
                _: Option<&str>,
                _: Option<&Path>,
            ) -> TmuxResult<Window> {
                Err(TmuxError::NoServer)
            }
            fn rename_window(&self, _: &WindowId, _: &str) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn kill_window(&self, _: &WindowId) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn list_panes(&self, _: Option<&WindowId>) -> TmuxResult<Vec<Pane>> {
                Ok(Vec::new())
            }
            fn split_pane(&self, _: &PaneId, _: SplitDirection) -> TmuxResult<Pane> {
                Err(TmuxError::NoServer)
            }
            fn join_pane(&self, _: &PaneId, _: &PaneId, _: SplitDirection) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn kill_pane(&self, _: &PaneId) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn select(&self, _: &Target) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn switch_client(&self, _: &Target) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn detach_client(&self, _: &str) -> TmuxResult<()> {
                Err(TmuxError::NoServer)
            }
            fn attach_command(&self, _: &Target) -> Command {
                Command::new("tmux")
            }
            fn set_global_option(&self, option: &str, value: &str) -> TmuxResult<()> {
                self.calls.borrow_mut().push((
                    TmuxOptionScope::Global,
                    option.to_owned(),
                    value.to_owned(),
                ));
                Ok(())
            }
            fn set_global_window_option(&self, option: &str, value: &str) -> TmuxResult<()> {
                self.calls.borrow_mut().push((
                    TmuxOptionScope::GlobalWindow,
                    option.to_owned(),
                    value.to_owned(),
                ));
                Ok(())
            }
            fn set_server_option(&self, option: &str, value: &str) -> TmuxResult<()> {
                self.calls.borrow_mut().push((
                    TmuxOptionScope::Server,
                    option.to_owned(),
                    value.to_owned(),
                ));
                Ok(())
            }
        }

        let all_on = TmuxComfortOptions {
            mouse: true,
            history_limit: 50_000,
            base_index_one: true,
            renumber_windows: true,
            zero_escape_time: true,
        };

        let offline = OptionStubTmux {
            running: false,
            calls: RefCell::new(Vec::new()),
        };
        assert_eq!(apply_comfort_live(&offline, &all_on).expect("offline"), 0);
        assert!(offline.calls.borrow().is_empty());

        let online = OptionStubTmux {
            running: true,
            calls: RefCell::new(Vec::new()),
        };
        assert_eq!(apply_comfort_live(&online, &all_on).expect("online"), 6);
        assert_eq!(
            *online.calls.borrow(),
            vec![
                (TmuxOptionScope::Global, "mouse".into(), "on".into()),
                (
                    TmuxOptionScope::Global,
                    "history-limit".into(),
                    "50000".into(),
                ),
                (TmuxOptionScope::Global, "base-index".into(), "1".into()),
                (
                    TmuxOptionScope::GlobalWindow,
                    "pane-base-index".into(),
                    "1".into(),
                ),
                (
                    TmuxOptionScope::Global,
                    "renumber-windows".into(),
                    "on".into(),
                ),
                (TmuxOptionScope::Server, "escape-time".into(), "0".into()),
            ]
        );
    }
}
