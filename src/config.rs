//! Unified TOML configuration resolution, zero-dependency parsing, precedence, and persistence.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::{
    terminal::{INLINE_HEIGHT, ScreenMode},
    ui::{ComposerMode, EmblemMode, Language, Theme, ThemePreset},
};

pub const DEFAULT_CONFIG_REL_PATH: &str = ".config/justmux/config.toml";
const MIN_INLINE_HEIGHT: u16 = 8;
const MAX_INLINE_HEIGHT: u16 = 40;

/// Effective JustMux configuration resolved from defaults, config file, env vars, and CLI flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub screen_mode: ScreenMode,
    pub theme: ThemePreset,
    pub composer_mode: ComposerMode,
    pub live_preview: bool,
    pub live_stream: bool,
    pub emblem_mode: EmblemMode,
    pub mouse: bool,
    pub inline_height: u16,
    pub no_color: bool,
    pub ascii: bool,
    pub language: Language,
    pub tmux_popup: bool,
    pub tmux_mouse: bool,
    pub tmux_history_limit: u32,
    pub tmux_base_index_one: bool,
    pub tmux_renumber_windows: bool,
    pub tmux_zero_escape_time: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            screen_mode: ScreenMode::Fullscreen,
            theme: ThemePreset::AntigravityAurora,
            composer_mode: ComposerMode::CommandJump,
            live_preview: true,
            live_stream: true,
            emblem_mode: EmblemMode::Spin,
            mouse: true,
            inline_height: INLINE_HEIGHT,
            no_color: false,
            ascii: false,
            language: Language::En,
            tmux_popup: false,
            tmux_mouse: false,
            tmux_history_limit: 0,
            tmux_base_index_one: false,
            tmux_renumber_windows: false,
            tmux_zero_escape_time: false,
        }
    }
}

impl Config {
    /// Build the effective runtime [`Theme`], downgrading default Aurora to Classic under `no_color`.
    pub fn effective_theme(&self) -> Theme {
        Theme {
            no_color: self.no_color,
            ascii: self.ascii,
            language: self.language,
            ..Theme::default()
        }
        .with_preset(
            if self.no_color && self.theme == ThemePreset::AntigravityAurora {
                ThemePreset::Classic
            } else {
                self.theme
            },
        )
    }

    /// Apply any specified values from a parsed [`PartialConfig`].
    pub fn apply(&mut self, partial: &PartialConfig) {
        if let Some(screen_mode) = partial.screen_mode {
            self.screen_mode = screen_mode;
        }
        if let Some(theme) = partial.theme {
            self.theme = theme;
        }
        if let Some(composer_mode) = partial.composer_mode {
            self.composer_mode = composer_mode;
        }
        if let Some(live_preview) = partial.live_preview {
            self.live_preview = live_preview;
        }
        if let Some(live_stream) = partial.live_stream {
            self.live_stream = live_stream;
        }
        if let Some(emblem_mode) = partial.emblem_mode {
            self.emblem_mode = emblem_mode;
        }
        if let Some(mouse) = partial.mouse {
            self.mouse = mouse;
        }
        if let Some(inline_height) = partial.inline_height {
            self.inline_height = inline_height.clamp(MIN_INLINE_HEIGHT, MAX_INLINE_HEIGHT);
        }
        if let Some(no_color) = partial.no_color {
            self.no_color = no_color;
        }
        if let Some(ascii) = partial.ascii {
            self.ascii = ascii;
        }
        if let Some(language) = partial.language {
            self.language = language;
        }
        if let Some(tmux_popup) = partial.tmux_popup {
            self.tmux_popup = tmux_popup;
        }
        if let Some(tmux_mouse) = partial.tmux_mouse {
            self.tmux_mouse = tmux_mouse;
        }
        if let Some(tmux_history_limit) = partial.tmux_history_limit {
            self.tmux_history_limit = tmux_history_limit;
        }
        if let Some(tmux_base_index_one) = partial.tmux_base_index_one {
            self.tmux_base_index_one = tmux_base_index_one;
        }
        if let Some(tmux_renumber_windows) = partial.tmux_renumber_windows {
            self.tmux_renumber_windows = tmux_renumber_windows;
        }
        if let Some(tmux_zero_escape_time) = partial.tmux_zero_escape_time {
            self.tmux_zero_escape_time = tmux_zero_escape_time;
        }
    }

    /// Extract the opt-in `~/.tmux.conf` comfort settings managed by JustMux.
    pub fn tmux_comfort_options(&self) -> crate::tmux_conf::TmuxComfortOptions {
        crate::tmux_conf::TmuxComfortOptions {
            mouse: self.tmux_mouse,
            history_limit: self.tmux_history_limit,
            base_index_one: self.tmux_base_index_one,
            renumber_windows: self.tmux_renumber_windows,
            zero_escape_time: self.tmux_zero_escape_time,
        }
    }

    /// Update the opt-in `~/.tmux.conf` comfort fields from [`crate::tmux_conf::TmuxComfortOptions`].
    pub fn set_tmux_comfort_options(&mut self, opts: crate::tmux_conf::TmuxComfortOptions) {
        self.tmux_mouse = opts.mouse;
        self.tmux_history_limit = opts.history_limit;
        self.tmux_base_index_one = opts.base_index_one;
        self.tmux_renumber_windows = opts.renumber_windows;
        self.tmux_zero_escape_time = opts.zero_escape_time;
    }

    /// Serialize this configuration as clean, round-trippable TOML.
    pub fn to_toml(&self) -> String {
        let inline_height = self
            .inline_height
            .clamp(MIN_INLINE_HEIGHT, MAX_INLINE_HEIGHT);
        format!(
            "# JustMux configuration\n\n\
             [ui]\n\
             theme = \"{}\"\n\
             composer_mode = \"{}\"\n\
             live_preview = {}\n\
             live_stream = {}\n\
             emblem_mode = \"{}\"\n\
             no_color = {}\n\
             ascii = {}\n\
             language = \"{}\"\n\n\
             [terminal]\n\
             screen_mode = \"{}\"\n\
             mouse = {}\n\
             inline_height = {}\n\n\
             [tmux]\n\
             tmux_popup = {}\n\
             tmux_mouse = {}\n\
             tmux_history_limit = {}\n\
             tmux_base_index_one = {}\n\
             tmux_renumber_windows = {}\n\
             tmux_zero_escape_time = {}\n",
            self.theme.slug(),
            composer_mode_slug(self.composer_mode),
            self.live_preview,
            self.live_stream,
            self.emblem_mode.as_str(),
            self.no_color,
            self.ascii,
            self.language.as_str(),
            self.screen_mode.as_str(),
            self.mouse,
            inline_height,
            self.tmux_popup,
            self.tmux_mouse,
            self.tmux_history_limit,
            self.tmux_base_index_one,
            self.tmux_renumber_windows,
            self.tmux_zero_escape_time,
        )
    }

    /// Create parent directories if needed and persist [`Config::to_toml`] to `path`.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|err| {
                ConfigError::new(format!(
                    "could not create config directory `{}`: {err}",
                    parent.display()
                ))
            })?;
        }
        fs::write(path, self.to_toml()).map_err(|err| {
            ConfigError::new(format!(
                "could not write config file `{}`: {err}",
                path.display()
            ))
        })
    }
}

/// Partial configuration parsed from `config.toml`, leaving unset keys as `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialConfig {
    pub screen_mode: Option<ScreenMode>,
    pub theme: Option<ThemePreset>,
    pub composer_mode: Option<ComposerMode>,
    pub live_preview: Option<bool>,
    pub live_stream: Option<bool>,
    pub emblem_mode: Option<EmblemMode>,
    pub mouse: Option<bool>,
    pub inline_height: Option<u16>,
    pub no_color: Option<bool>,
    pub ascii: Option<bool>,
    pub language: Option<Language>,
    pub tmux_popup: Option<bool>,
    pub tmux_mouse: Option<bool>,
    pub tmux_history_limit: Option<u32>,
    pub tmux_base_index_one: Option<bool>,
    pub tmux_renumber_windows: Option<bool>,
    pub tmux_zero_escape_time: Option<bool>,
}

/// CLI arguments that take highest precedence over env vars and `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOverrides {
    pub config_path: Option<PathBuf>,
    pub screen_mode: Option<ScreenMode>,
    pub theme: Option<String>,
}

/// Result of resolving and loading JustMux configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig {
    pub config: Config,
    pub path: Option<PathBuf>,
    pub loaded_from_file: bool,
}

/// Line-aware configuration parse or I/O error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    line: Option<usize>,
    message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }

    pub fn at_line(line: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }

    pub fn line(&self) -> Option<usize> {
        self.line
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn with_path(self, path: &Path) -> Self {
        Self {
            line: self.line,
            message: format!("{}: {}", path.display(), self.message),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Clean, well-commented starter `config.toml` showcasing all options and built-in defaults.
pub fn starter_config_toml() -> &'static str {
    "# JustMux configuration file\n\
     # Default path: ~/.config/justmux/config.toml (or $XDG_CONFIG_HOME/justmux/config.toml)\n\
     # Precedence: CLI flags > environment variables > config.toml > defaults\n\n\
     [ui]\n\
     # Color theme preset: \"aurora\" | \"codex\" | \"claude\" | \"matrix\" | \"classic\"\n\
     theme = \"aurora\"\n\n\
     # Initial composer mode: \"command-jump\" | \"live-radar\" | \"quick-launch\"\n\
     composer_mode = \"command-jump\"\n\n\
     # Show live tmux pane previews in Fullscreen studio, Live Radar, /list, and /resume\n\
     live_preview = true\n\n\
     # Auto-refresh live previews & Mission Control tiles in Fullscreen mode (off in inline mode)\n\
     live_stream = true\n\n\
     # Home Hydra emblem mode: \"static\" | \"spin\" | \"inverted-spin\"\n\
     emblem_mode = \"spin\"\n\n\
     # Disable ANSI colors (also activated by NO_COLOR or TERM=dumb)\n\
     no_color = false\n\n\
     # Use pure ASCII box borders and symbols instead of Unicode\n\
     ascii = false\n\n\
     # Interface language: \"en\" (English) | \"zh\" (Chinese)\n\
     language = \"en\"\n\n\
     [terminal]\n\
     # Display mode: \"fullscreen\" (alternate screen) | \"inline\" (native scrollback)\n\
     screen_mode = \"fullscreen\"\n\n\
     # Enable mouse wheel & clicks in Fullscreen mode (disabled automatically before tmux attach)\n\
     mouse = true\n\n\
     # Reserved viewport height in rows for inline mode (8..=40)\n\
     inline_height = 16\n\n\
     [tmux]\n\
     # Manage a Ctrl+b j popup keybind in ~/.tmux.conf (toggled interactively in /config)\n\
     tmux_popup = false\n\n\
     # Opt-in ~/.tmux.conf comfort defaults (all off/unmanaged unless enabled):\n\
     # Enable tmux mouse scroll, click-to-focus, and drag-to-resize (`set -g mouse on`)\n\
     tmux_mouse = false\n\n\
     # Scrollback buffer lines (`0` = unmanaged; e.g. 10000, 50000, 100000 -> `set -g history-limit <N>`)\n\
     tmux_history_limit = 0\n\n\
     # Start window and pane numbering at 1 (`set -g base-index 1`, `set -gw pane-base-index 1`)\n\
     tmux_base_index_one = false\n\n\
     # Automatically renumber windows when one closes (`set -g renumber-windows on`)\n\
     tmux_renumber_windows = false\n\n\
     # Eliminate Esc key lag for Vim/Neovim (`set -sg escape-time 0`)\n\
     tmux_zero_escape_time = false\n"
}

/// Resolve the active config path following:
/// 1) `explicit` (`--config`)
/// 2) `JUSTMUX_CONFIG` (if set and non-empty)
/// 3) `$XDG_CONFIG_HOME/justmux/config.toml` (if set and absolute)
/// 4) `$HOME/.config/justmux/config.toml`
pub fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf> {
    resolve_config_path_with(explicit, &EnvSnapshot::from_process())
}

/// Load configuration with strict precedence:
/// `CLI overrides > Env vars > config.toml > Config::default()`.
pub fn load(overrides: &CliOverrides) -> Result<LoadedConfig, ConfigError> {
    load_with_env(overrides, &EnvSnapshot::from_process())
}

/// Parse a zero-dependency TOML subset (`[ui]`, `[general]`, `[display]`, `[terminal]`, `key = value`).
pub fn parse_config_str(input: &str) -> Result<PartialConfig, ConfigError> {
    let mut partial = PartialConfig::default();

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

        if trimmed.starts_with('[') {
            parse_section_header(trimmed, line_no)?;
            continue;
        }

        let Some((raw_key, raw_val)) = trimmed.split_once('=') else {
            return Err(ConfigError::at_line(
                line_no,
                format!("expected `key = value`, found `{trimmed}`"),
            ));
        };
        let key = raw_key.trim();
        let val = raw_val.trim();
        if key.is_empty() {
            return Err(ConfigError::at_line(line_no, "missing key before `=`"));
        }
        if val.is_empty() {
            return Err(ConfigError::at_line(
                line_no,
                format!("missing value for `{key}`"),
            ));
        }

        apply_kv(&mut partial, key, val, line_no)?;
    }

    Ok(partial)
}

#[derive(Debug, Clone, Default)]
struct EnvSnapshot {
    justmux_config: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    no_color_set: bool,
    term: Option<String>,
    lc_all: Option<String>,
    lc_ctype: Option<String>,
    lang: Option<String>,
    justmux_theme: Option<String>,
    justmux_screen_mode: Option<String>,
    justmux_emblem_spin: Option<String>,
    justmux_emblem_mode: Option<String>,
    justmux_language: Option<String>,
    justmux_lang: Option<String>,
    justmux_live_stream: Option<String>,
}

impl EnvSnapshot {
    fn from_process() -> Self {
        Self {
            justmux_config: std::env::var_os("JUSTMUX_CONFIG")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            home_dir: std::env::home_dir(),
            no_color_set: std::env::var_os("NO_COLOR").is_some(),
            term: std::env::var("TERM").ok(),
            lc_all: std::env::var("LC_ALL").ok(),
            lc_ctype: std::env::var("LC_CTYPE").ok(),
            lang: std::env::var("LANG").ok(),
            justmux_theme: std::env::var("JUSTMUX_THEME").ok(),
            justmux_screen_mode: std::env::var("JUSTMUX_SCREEN_MODE").ok(),
            justmux_emblem_spin: std::env::var("JUSTMUX_EMBLEM_SPIN").ok(),
            justmux_emblem_mode: std::env::var("JUSTMUX_EMBLEM_MODE").ok(),
            justmux_language: std::env::var("JUSTMUX_LANGUAGE").ok(),
            justmux_lang: std::env::var("JUSTMUX_LANG").ok(),
            justmux_live_stream: std::env::var("JUSTMUX_LIVE_STREAM").ok(),
        }
    }

    fn has_no_color(&self) -> bool {
        self.no_color_set
            || self
                .term
                .as_deref()
                .is_some_and(|term| term.trim().eq_ignore_ascii_case("dumb"))
    }

    fn has_ascii_locale(&self) -> bool {
        let locale = [
            self.lc_all.as_deref(),
            self.lc_ctype.as_deref(),
            self.lang.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty());

        locale.is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            !(lower.contains("utf-8") || lower.contains("utf8"))
        })
    }
}

fn resolve_config_path_with(explicit: Option<&Path>, env: &EnvSnapshot) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(path) = env
        .justmux_config
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
    {
        return Some(path.clone());
    }
    if let Some(xdg) = env.xdg_config_home.as_ref().filter(|p| p.is_absolute()) {
        return Some(xdg.join("justmux").join("config.toml"));
    }
    env.home_dir
        .as_ref()
        .map(|home| home.join(DEFAULT_CONFIG_REL_PATH))
}

fn load_with_env(overrides: &CliOverrides, env: &EnvSnapshot) -> Result<LoadedConfig, ConfigError> {
    let cli_theme = match overrides.theme.as_deref() {
        Some(raw) => Some(ThemePreset::from_name(raw).ok_or_else(|| {
            ConfigError::new(format!(
                "unknown theme `{raw}`; expected aurora, codex, claude, matrix, or classic"
            ))
        })?),
        None => None,
    };

    let path = resolve_config_path_with(overrides.config_path.as_deref(), env);
    let explicit_cli = overrides.config_path.is_some();
    let explicit_env = !explicit_cli
        && env
            .justmux_config
            .as_ref()
            .is_some_and(|p| !p.as_os_str().is_empty());

    let mut loaded_from_file = false;
    let mut file_partial = PartialConfig::default();

    if let Some(ref cfg_path) = path {
        let is_dev_null_env = explicit_env && cfg_path == Path::new("/dev/null");
        if !is_dev_null_env {
            match fs::read_to_string(cfg_path) {
                Ok(contents) => {
                    file_partial =
                        parse_config_str(&contents).map_err(|err| err.with_path(cfg_path))?;
                    loaded_from_file = true;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    if explicit_cli || explicit_env {
                        return Err(ConfigError::new(format!(
                            "config file not found: `{}`",
                            cfg_path.display()
                        )));
                    }
                }
                Err(err) => {
                    return Err(ConfigError::new(format!(
                        "could not read config file `{}`: {err}",
                        cfg_path.display()
                    )));
                }
            }
        }
    }

    let mut config = Config::default();
    config.apply(&file_partial);

    if env.has_no_color() {
        config.no_color = true;
    }
    if env.has_ascii_locale() {
        config.ascii = true;
    }
    if let Some(raw_theme) = env
        .justmux_theme
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(preset) = ThemePreset::from_name(raw_theme)
    {
        config.theme = preset;
    }
    if let Some(raw_mode) = env
        .justmux_screen_mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(mode) = ScreenMode::from_name(raw_mode)
    {
        config.screen_mode = mode;
    }
    if let Some(raw_spin) = env
        .justmux_emblem_spin
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(mode) = EmblemMode::from_name(raw_spin)
    {
        config.emblem_mode = mode;
    }
    if let Some(raw_mode) = env
        .justmux_emblem_mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(mode) = EmblemMode::from_name(raw_mode)
    {
        config.emblem_mode = mode;
    }
    if let Some(raw_lang) = env
        .justmux_lang
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(language) = Language::from_name(raw_lang)
    {
        config.language = language;
    }
    if let Some(raw_lang) = env
        .justmux_language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(language) = Language::from_name(raw_lang)
    {
        config.language = language;
    }
    if let Some(raw_stream) = env
        .justmux_live_stream
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(enabled) = parse_env_bool(raw_stream)
    {
        config.live_stream = enabled;
    }

    if let Some(mode) = overrides.screen_mode {
        config.screen_mode = mode;
    }
    if let Some(preset) = cli_theme {
        config.theme = preset;
    }

    config.inline_height = config
        .inline_height
        .clamp(MIN_INLINE_HEIGHT, MAX_INLINE_HEIGHT);

    Ok(LoadedConfig {
        config,
        path,
        loaded_from_file,
    })
}

fn composer_mode_slug(mode: ComposerMode) -> &'static str {
    match mode {
        ComposerMode::CommandJump => "command-jump",
        ComposerMode::LiveRadar => "live-radar",
        ComposerMode::QuickLaunch => "quick-launch",
    }
}

fn parse_composer_mode(raw: &str) -> Option<ComposerMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "command" | "command-jump" | "command_jump" | "jump" => Some(ComposerMode::CommandJump),
        "radar" | "live-radar" | "live_radar" => Some(ComposerMode::LiveRadar),
        "quick" | "quick-launch" | "quick_launch" => Some(ComposerMode::QuickLaunch),
        _ => None,
    }
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

fn parse_section_header(trimmed: &str, line_no: usize) -> Result<(), ConfigError> {
    if !trimmed.ends_with(']') || trimmed.len() < 2 {
        return Err(ConfigError::at_line(
            line_no,
            format!("malformed section header `{trimmed}`"),
        ));
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    match inner {
        "ui" | "general" | "display" | "terminal" | "tmux" => Ok(()),
        "" => Err(ConfigError::at_line(line_no, "empty section header `[]`")),
        _ => Err(ConfigError::at_line(
            line_no,
            format!("unknown config section `[{inner}]`"),
        )),
    }
}

fn apply_kv(
    partial: &mut PartialConfig,
    key: &str,
    val: &str,
    line_no: usize,
) -> Result<(), ConfigError> {
    match key {
        "screen_mode" | "display_mode" => {
            let s = parse_toml_string(val, line_no)?;
            let mode = ScreenMode::from_name(&s).ok_or_else(|| {
                ConfigError::at_line(
                    line_no,
                    format!("invalid `{key}` value `{s}` (expected `fullscreen` or `inline`)"),
                )
            })?;
            partial.screen_mode = Some(mode);
        }
        "theme" | "palette" => {
            let s = parse_toml_string(val, line_no)?;
            let preset = ThemePreset::from_name(&s).ok_or_else(|| {
                ConfigError::at_line(
                    line_no,
                    format!(
                        "unknown `{key}` preset `{s}` (expected aurora, codex, claude, matrix, or classic)"
                    ),
                )
            })?;
            partial.theme = Some(preset);
        }
        "composer_mode" | "mode" => {
            let s = parse_toml_string(val, line_no)?;
            let mode = parse_composer_mode(&s).ok_or_else(|| {
                ConfigError::at_line(
                    line_no,
                    format!(
                        "invalid `{key}` value `{s}` (expected command-jump, live-radar, or quick-launch)"
                    ),
                )
            })?;
            partial.composer_mode = Some(mode);
        }
        "live_preview" | "preview" => {
            partial.live_preview = Some(parse_toml_bool(val, key, line_no)?);
        }
        "live_stream" | "auto_refresh" | "stream_preview" => {
            partial.live_stream = Some(parse_toml_bool(val, key, line_no)?);
        }
        "emblem_mode" | "hydra_mode" | "emblem_spin" | "hydra_spin" | "rotate_emblem"
        | "animate_emblem" => {
            if val.starts_with('"') || val.starts_with('\'') {
                let s = parse_toml_string(val, line_no)?;
                let mode = EmblemMode::from_name(&s).ok_or_else(|| {
                    ConfigError::at_line(
                        line_no,
                        format!(
                            "invalid `{key}` value `{s}` (expected \"static\", \"spin\", or \"inverted-spin\")"
                        ),
                    )
                })?;
                partial.emblem_mode = Some(mode);
            } else {
                let enabled = parse_toml_bool(val, key, line_no)?;
                partial.emblem_mode = Some(if enabled {
                    EmblemMode::Spin
                } else {
                    EmblemMode::Static
                });
            }
        }
        "mouse" => {
            partial.mouse = Some(parse_toml_bool(val, key, line_no)?);
        }
        "inline_height" => {
            partial.inline_height = Some(parse_toml_inline_height(val, line_no)?);
        }
        "no_color" => {
            partial.no_color = Some(parse_toml_bool(val, key, line_no)?);
        }
        "ascii" => {
            partial.ascii = Some(parse_toml_bool(val, key, line_no)?);
        }
        "language" | "lang" => {
            let parsed_str = parse_toml_string(val, line_no)?;
            partial.language = Some(Language::from_name(&parsed_str).ok_or_else(|| {
                ConfigError::at_line(
                    line_no,
                    format!("invalid language `{parsed_str}` (expected en or zh)"),
                )
            })?);
        }
        "tmux_popup" | "tmux_keybind" | "popup_keybind" => {
            partial.tmux_popup = Some(parse_toml_bool(val, key, line_no)?);
        }
        "tmux_mouse" => {
            partial.tmux_mouse = Some(parse_toml_bool(val, key, line_no)?);
        }
        "tmux_history_limit" => {
            partial.tmux_history_limit = Some(parse_toml_u32(val, key, line_no)?);
        }
        "tmux_base_index_one" => {
            partial.tmux_base_index_one = Some(parse_toml_bool(val, key, line_no)?);
        }
        "tmux_renumber_windows" => {
            partial.tmux_renumber_windows = Some(parse_toml_bool(val, key, line_no)?);
        }
        "tmux_zero_escape_time" => {
            partial.tmux_zero_escape_time = Some(parse_toml_bool(val, key, line_no)?);
        }
        _ => {
            return Err(ConfigError::at_line(
                line_no,
                format!("unknown config key `{key}`"),
            ));
        }
    }
    Ok(())
}

fn parse_toml_string(raw: &str, line_no: usize) -> Result<String, ConfigError> {
    if let Some(rest) = raw.strip_prefix('"') {
        let mut out = String::with_capacity(rest.len());
        let mut chars = rest.char_indices();
        while let Some((idx, ch)) = chars.next() {
            match ch {
                '"' => {
                    if idx + ch.len_utf8() != rest.len() {
                        return Err(ConfigError::at_line(
                            line_no,
                            "unexpected trailing characters after string literal",
                        ));
                    }
                    return Ok(out);
                }
                '\\' => {
                    let Some((_, esc)) = chars.next() else {
                        return Err(ConfigError::at_line(line_no, "unterminated string literal"));
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
                            return Err(ConfigError::at_line(
                                line_no,
                                format!("unsupported escape sequence `\\{esc}`"),
                            ));
                        }
                    }
                }
                other => out.push(other),
            }
        }
        return Err(ConfigError::at_line(line_no, "unterminated string literal"));
    }

    if let Some(rest) = raw.strip_prefix('\'') {
        let Some(end) = rest.find('\'') else {
            return Err(ConfigError::at_line(line_no, "unterminated string literal"));
        };
        if end + 1 != rest.len() {
            return Err(ConfigError::at_line(
                line_no,
                "unexpected trailing characters after string literal",
            ));
        }
        return Ok(rest[..end].to_owned());
    }

    Err(ConfigError::at_line(
        line_no,
        format!("expected quoted string, found `{raw}`"),
    ))
}

fn parse_unicode_escape(
    chars: &mut std::str::CharIndices<'_>,
    digits: usize,
    line_no: usize,
) -> Result<char, ConfigError> {
    let mut hex = String::with_capacity(digits);
    for _ in 0..digits {
        let Some((_, ch)) = chars.next() else {
            return Err(ConfigError::at_line(line_no, "incomplete unicode escape"));
        };
        hex.push(ch);
    }
    let code = u32::from_str_radix(&hex, 16)
        .map_err(|_| ConfigError::at_line(line_no, format!("invalid unicode escape `\\u{hex}`")))?;
    char::from_u32(code).ok_or_else(|| {
        ConfigError::at_line(line_no, format!("invalid unicode scalar value `0x{hex}`"))
    })
}

fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" => Some(true),
        "false" | "0" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn parse_toml_bool(raw: &str, key: &str, line_no: usize) -> Result<bool, ConfigError> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::at_line(
            line_no,
            format!("invalid boolean for `{key}`: `{raw}` (expected `true` or `false`)"),
        )),
    }
}

fn parse_toml_inline_height(raw: &str, line_no: usize) -> Result<u16, ConfigError> {
    if raw.is_empty() || raw.starts_with('_') || raw.ends_with('_') || raw.contains("__") {
        return Err(ConfigError::at_line(
            line_no,
            format!("invalid integer for `inline_height`: `{raw}` (expected 8..=40)"),
        ));
    }

    let (negative, digits_part) = if let Some(rest) = raw.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = raw.strip_prefix('+') {
        (false, rest)
    } else {
        (false, raw)
    };

    if digits_part.is_empty()
        || digits_part.starts_with('_')
        || digits_part.ends_with('_')
        || !digits_part.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return Err(ConfigError::at_line(
            line_no,
            format!("invalid integer for `inline_height`: `{raw}` (expected 8..=40)"),
        ));
    }

    let compact: String = digits_part.chars().filter(|&c| c != '_').collect();
    let magnitude = compact.parse::<i64>().map_err(|_| {
        ConfigError::at_line(
            line_no,
            format!("invalid integer for `inline_height`: `{raw}` (expected 8..=40)"),
        )
    })?;
    let signed = if negative { -magnitude } else { magnitude };

    if !(i64::from(MIN_INLINE_HEIGHT)..=i64::from(MAX_INLINE_HEIGHT)).contains(&signed) {
        return Err(ConfigError::at_line(
            line_no,
            format!("`inline_height` out of range: {signed} (expected 8..=40)"),
        ));
    }

    u16::try_from(signed).map_err(|_| {
        ConfigError::at_line(
            line_no,
            format!("`inline_height` out of range: {signed} (expected 8..=40)"),
        )
    })
}

fn parse_toml_u32(raw: &str, key: &str, line_no: usize) -> Result<u32, ConfigError> {
    if raw.is_empty() || raw.starts_with('_') || raw.ends_with('_') || raw.contains("__") {
        return Err(ConfigError::at_line(
            line_no,
            format!("invalid integer for `{key}`: `{raw}` (expected non-negative integer)"),
        ));
    }

    let digits_part = if raw.starts_with('-') {
        return Err(ConfigError::at_line(
            line_no,
            format!("`{key}` cannot be negative: `{raw}`"),
        ));
    } else if let Some(rest) = raw.strip_prefix('+') {
        rest
    } else {
        raw
    };

    if digits_part.is_empty()
        || digits_part.starts_with('_')
        || digits_part.ends_with('_')
        || !digits_part.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return Err(ConfigError::at_line(
            line_no,
            format!("invalid integer for `{key}`: `{raw}` (expected non-negative integer)"),
        ));
    }

    let compact: String = digits_part.chars().filter(|&c| c != '_').collect();
    compact.parse::<u32>().map_err(|_| {
        ConfigError::at_line(
            line_no,
            format!("`{key}` out of range: `{raw}` (expected 0..={})", u32::MAX),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_starter_config_match_and_round_trip() {
        let def = Config::default();
        assert_eq!(def.screen_mode, ScreenMode::Fullscreen);
        assert_eq!(def.theme, ThemePreset::AntigravityAurora);
        assert_eq!(def.language, Language::En);
        assert_eq!(def.composer_mode, ComposerMode::CommandJump);
        assert!(def.live_preview);
        assert!(def.live_stream);
        assert_eq!(def.emblem_mode, EmblemMode::Spin);
        assert!(def.mouse);
        assert_eq!(def.inline_height, 16);
        assert!(!def.no_color);
        assert!(!def.ascii);
        assert!(!def.tmux_popup);
        assert!(!def.tmux_mouse);
        assert_eq!(def.tmux_history_limit, 0);
        assert!(!def.tmux_base_index_one);
        assert!(!def.tmux_renumber_windows);
        assert!(!def.tmux_zero_escape_time);

        let starter_partial = parse_config_str(starter_config_toml()).expect("parse starter toml");
        let mut from_starter = Config::default();
        from_starter.apply(&starter_partial);
        assert_eq!(from_starter, def);

        for (mode, language, tmux_popup) in [
            (EmblemMode::InvertedSpin, Language::Zh, true),
            (EmblemMode::Static, Language::En, false),
            (EmblemMode::Spin, Language::Zh, true),
        ] {
            let custom = Config {
                screen_mode: ScreenMode::Inline,
                theme: ThemePreset::CodexEmerald,
                language,
                composer_mode: ComposerMode::LiveRadar,
                live_preview: false,
                live_stream: false,
                emblem_mode: mode,
                mouse: false,
                inline_height: 24,
                no_color: true,
                ascii: true,
                tmux_popup,
                tmux_mouse: tmux_popup,
                tmux_history_limit: if tmux_popup { 50_000 } else { 0 },
                tmux_base_index_one: tmux_popup,
                tmux_renumber_windows: !tmux_popup,
                tmux_zero_escape_time: tmux_popup,
            };
            let toml = custom.to_toml();
            assert!(
                toml.contains(&format!("emblem_mode = \"{}\"", mode.as_str())),
                "to_toml missing emblem_mode = {:?}: {toml}",
                mode.as_str()
            );
            assert!(
                toml.contains(&format!("language = \"{}\"", language.as_str())),
                "to_toml missing language = {:?}: {toml}",
                language.as_str()
            );
            assert!(
                toml.contains(&format!("tmux_popup = {tmux_popup}")),
                "to_toml missing tmux_popup = {tmux_popup}: {toml}",
            );
            assert!(
                toml.contains(&format!(
                    "tmux_history_limit = {}",
                    custom.tmux_history_limit
                )),
                "to_toml missing tmux_history_limit: {toml}",
            );
            let reparsed = parse_config_str(&toml).expect("reparse to_toml");
            let mut round_tripped = Config::default();
            round_tripped.apply(&reparsed);
            assert_eq!(round_tripped, custom);
        }
    }

    #[test]
    fn parser_supports_sections_aliases_quotes_comments_and_integers() {
        let src = "\u{feff}# leading comment\n\
                   [ui]\n\
                   palette = 'claude' # inline comment with \"quotes\"\n\
                   language = \"zh\"\n\
                   mode = \"radar\"\n\
                   preview = false\n\
                   no_color = true\n\
                   ascii = true\n\
                   \n\
                   [terminal]\n\
                   display_mode = \"compact\" # alias for inline\n\
                   mouse = false\n\
                   inline_height = 2_8\n\
                   [tmux]\n\
                   tmux_popup = true\n\
                   [general]\n\
                   [display]\n";

        let parsed = parse_config_str(src).expect("valid config");
        assert_eq!(parsed.theme, Some(ThemePreset::ClaudeWarmth));
        assert_eq!(parsed.language, Some(Language::Zh));
        assert_eq!(parsed.composer_mode, Some(ComposerMode::LiveRadar));
        assert_eq!(parsed.live_preview, Some(false));
        assert_eq!(parsed.no_color, Some(true));
        assert_eq!(parsed.ascii, Some(true));
        assert_eq!(parsed.screen_mode, Some(ScreenMode::Inline));
        assert_eq!(parsed.mouse, Some(false));
        assert_eq!(parsed.inline_height, Some(28));
        assert_eq!(parsed.tmux_popup, Some(true));

        assert_eq!(
            parse_config_str("tmux_keybind = true\n")
                .expect("tmux_keybind alias")
                .tmux_popup,
            Some(true)
        );
        assert_eq!(
            parse_config_str("popup_keybind = false\n")
                .expect("popup_keybind alias")
                .tmux_popup,
            Some(false)
        );

        let lang_alias = parse_config_str("lang = 'zh-CN'\n").expect("lang alias");
        assert_eq!(lang_alias.language, Some(Language::Zh));
        let lang_en = parse_config_str("lang = \"English\"\n").expect("lang en");
        assert_eq!(lang_en.language, Some(Language::En));

        let quick = parse_config_str("mode = \"quick\"\n").expect("quick alias");
        assert_eq!(quick.composer_mode, Some(ComposerMode::QuickLaunch));
        let quick_full =
            parse_config_str("composer_mode = \"quick-launch\"\n").expect("quick-launch");
        assert_eq!(quick_full.composer_mode, Some(ComposerMode::QuickLaunch));
        let jump = parse_config_str("composer_mode = 'jump'\n").expect("jump alias");
        assert_eq!(jump.composer_mode, Some(ComposerMode::CommandJump));

        let hydra_spin = parse_config_str("hydra_spin = false\n").expect("hydra_spin alias");
        assert_eq!(hydra_spin.emblem_mode, Some(EmblemMode::Static));
        let rotate_emblem =
            parse_config_str("rotate_emblem = true\n").expect("rotate_emblem alias");
        assert_eq!(rotate_emblem.emblem_mode, Some(EmblemMode::Spin));
        let animate_emblem =
            parse_config_str("animate_emblem = false\n").expect("animate_emblem alias");
        assert_eq!(animate_emblem.emblem_mode, Some(EmblemMode::Static));
        let legacy_true = parse_config_str("emblem_spin = true\n").expect("emblem_spin true");
        assert_eq!(legacy_true.emblem_mode, Some(EmblemMode::Spin));
        let legacy_false = parse_config_str("emblem_spin = false\n").expect("emblem_spin false");
        assert_eq!(legacy_false.emblem_mode, Some(EmblemMode::Static));

        for (raw, expected) in [
            ("emblem_mode = \"spin\"\n", EmblemMode::Spin),
            ("emblem_mode = 'inverted-spin'\n", EmblemMode::InvertedSpin),
            ("emblem_mode = \"static\"\n", EmblemMode::Static),
            ("hydra_mode = \"inverted\"\n", EmblemMode::InvertedSpin),
            (
                "emblem_spin = \"inverted-spin\"\n",
                EmblemMode::InvertedSpin,
            ),
        ] {
            let parsed_mode = parse_config_str(raw).expect(raw);
            assert_eq!(parsed_mode.emblem_mode, Some(expected), "{raw}");
        }

        let min_h = parse_config_str("inline_height = 8\n").expect("min inline_height");
        assert_eq!(min_h.inline_height, Some(8));
        let max_h = parse_config_str("inline_height = 40\n").expect("max inline_height");
        assert_eq!(max_h.inline_height, Some(40));

        // `#` inside quotes is preserved as part of the string literal, not a comment.
        let hash_in_double = parse_config_str("screen_mode = \"inline#x\"\n")
            .expect_err("hash inside double quotes stays in value");
        assert!(hash_in_double.to_string().contains("inline#x"));
        let hash_in_single = parse_config_str("screen_mode = 'full#x'\n")
            .expect_err("hash inside single quotes stays in value");
        assert!(hash_in_single.to_string().contains("full#x"));
    }

    #[test]
    fn parser_reports_line_numbered_errors_for_invalid_inputs() {
        let cases = [
            ("\n[bogus]\n", 2),
            ("theme = \"aurora\"\nunknown_key = true\n", 2),
            ("not_a_kv_line\n", 1),
            ("theme = aurora\n", 1),
            ("theme = \"unclosed\n", 1),
            ("theme = 'unclosed\n", 1),
            ("theme = \"aurora\" trailing\n", 1),
            ("theme = \"unknown\"\n", 1),
            ("screen_mode = \"tiled\"\n", 1),
            ("composer_mode = \"wizard\"\n", 1),
            ("emblem_mode = \"wobble\"\n", 1),
            ("emblem_mode = \"quad-hydra\"\n", 1),
            ("emblem_mode = spin\n", 1),
            ("pixel_glyphs = \"auto\"\n", 1),
            ("mouse = \"yes\"\n", 1),
            ("tmux_popup = \"yes\"\n", 1),
            ("inline_height = 7\n", 1),
            ("inline_height = 41\n", 1),
            ("inline_height = -3\n", 1),
            ("inline_height = \"16\"\n", 1),
            ("[unterminated\n", 1),
        ];

        for (input, expected_line) in cases {
            let err = parse_config_str(input)
                .expect_err(&format!("expected parse failure for `{input:?}`"));
            assert_eq!(err.line(), Some(expected_line), "error was: {err}");
            assert!(
                err.to_string().contains(&format!("line {expected_line}:")),
                "Display missing line number: {err}"
            );
        }

        let bad_emblem = parse_config_str("emblem_mode = \"wobble\"\n").expect_err("invalid mode");
        let msg = bad_emblem.to_string();
        assert!(msg.contains("static"));
        assert!(msg.contains("spin"));
        assert!(msg.contains("inverted-spin"));
        assert!(!msg.contains("quad-hydra"));

        let removed_qh = parse_config_str("emblem_mode = \"quad-hydra\"\n").expect_err("removed");
        assert!(removed_qh.to_string().contains("inverted-spin"));

        let removed_pg = parse_config_str("pixel_glyphs = \"auto\"\n").expect_err("removed key");
        assert!(removed_pg.to_string().contains("unknown config key"));
    }

    #[test]
    fn resolve_config_path_follows_precedence_and_validates_xdg_absolute() {
        let explicit = PathBuf::from("/custom/explicit.toml");
        let jm_cfg = PathBuf::from("/env/justmux.toml");
        let abs_xdg = PathBuf::from("/xdg/config");
        let rel_xdg = PathBuf::from("relative/xdg");
        let home = PathBuf::from("/home/tester");

        let full_env = EnvSnapshot {
            justmux_config: Some(jm_cfg.clone()),
            xdg_config_home: Some(abs_xdg.clone()),
            home_dir: Some(home.clone()),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            resolve_config_path_with(Some(&explicit), &full_env),
            Some(explicit)
        );
        assert_eq!(resolve_config_path_with(None, &full_env), Some(jm_cfg));

        let xdg_env = EnvSnapshot {
            xdg_config_home: Some(abs_xdg.clone()),
            home_dir: Some(home.clone()),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            resolve_config_path_with(None, &xdg_env),
            Some(abs_xdg.join("justmux/config.toml"))
        );

        let rel_xdg_env = EnvSnapshot {
            xdg_config_home: Some(rel_xdg),
            home_dir: Some(home.clone()),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            resolve_config_path_with(None, &rel_xdg_env),
            Some(home.join(DEFAULT_CONFIG_REL_PATH))
        );

        let empty_env = EnvSnapshot::default();
        assert_eq!(resolve_config_path_with(None, &empty_env), None);

        // Empty JUSTMUX_CONFIG falls through to XDG / HOME.
        let empty_jm_cfg = EnvSnapshot {
            justmux_config: Some(PathBuf::from("")),
            xdg_config_home: Some(abs_xdg.clone()),
            home_dir: Some(home),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            resolve_config_path_with(None, &empty_jm_cfg),
            Some(abs_xdg.join("justmux/config.toml"))
        );
    }

    #[test]
    fn load_enforces_precedence_missing_file_rules_and_no_auto_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let default_path = home.join(DEFAULT_CONFIG_REL_PATH);

        // Missing default path -> silent defaults, loaded_from_file = false, never auto-writes.
        let env_default = EnvSnapshot {
            home_dir: Some(home.clone()),
            ..EnvSnapshot::default()
        };
        let loaded = load_with_env(&CliOverrides::default(), &env_default).expect("default load");
        assert!(!loaded.loaded_from_file);
        assert_eq!(loaded.path, Some(default_path.clone()));
        assert_eq!(loaded.config, Config::default());
        assert!(!default_path.exists(), "load must never auto-write files");

        // Missing explicit --config or non-/dev/null JUSTMUX_CONFIG -> error.
        let missing = dir.path().join("missing.toml");
        let err_cli = load_with_env(
            &CliOverrides {
                config_path: Some(missing.clone()),
                ..CliOverrides::default()
            },
            &env_default,
        )
        .expect_err("missing explicit --config should fail");
        assert!(err_cli.to_string().contains("not found"));

        let env_missing = EnvSnapshot {
            justmux_config: Some(missing),
            home_dir: Some(home.clone()),
            ..EnvSnapshot::default()
        };
        assert!(load_with_env(&CliOverrides::default(), &env_missing).is_err());

        // JUSTMUX_CONFIG=/dev/null succeeds cleanly without loading a file.
        let env_dev_null = EnvSnapshot {
            justmux_config: Some(PathBuf::from("/dev/null")),
            home_dir: Some(home),
            ..EnvSnapshot::default()
        };
        let dev_null_loaded =
            load_with_env(&CliOverrides::default(), &env_dev_null).expect("/dev/null allowed");
        assert!(!dev_null_loaded.loaded_from_file);
        assert_eq!(dev_null_loaded.config, Config::default());

        // Write a config file and verify CLI > Env > config.toml > Default precedence.
        let cfg_path = dir.path().join("custom.toml");
        let file_cfg = Config {
            screen_mode: ScreenMode::Inline,
            theme: ThemePreset::ClaudeWarmth,
            language: Language::Zh,
            composer_mode: ComposerMode::QuickLaunch,
            live_preview: false,
            live_stream: false,
            emblem_mode: EmblemMode::Static,
            mouse: false,
            inline_height: 22,
            no_color: false,
            ascii: false,
            tmux_popup: true,
            tmux_mouse: true,
            tmux_history_limit: 50_000,
            tmux_base_index_one: true,
            tmux_renumber_windows: true,
            tmux_zero_escape_time: true,
        };
        file_cfg.save(&cfg_path).expect("save config");
        assert!(cfg_path.exists());

        // File alone:
        let from_file = load_with_env(
            &CliOverrides {
                config_path: Some(cfg_path.clone()),
                ..CliOverrides::default()
            },
            &EnvSnapshot::default(),
        )
        .expect("load from saved file");
        assert!(from_file.loaded_from_file);
        assert_eq!(from_file.config, file_cfg);

        // Env overrides file:
        let env_over = EnvSnapshot {
            no_color_set: true,
            lc_all: Some("C".into()),
            justmux_theme: Some("codex".into()),
            justmux_language: Some("en".into()),
            justmux_screen_mode: Some("fullscreen".into()),
            justmux_emblem_spin: Some(" YES ".into()),
            ..EnvSnapshot::default()
        };
        let after_env = load_with_env(
            &CliOverrides {
                config_path: Some(cfg_path.clone()),
                ..CliOverrides::default()
            },
            &env_over,
        )
        .expect("env overrides file");
        assert_eq!(after_env.config.theme, ThemePreset::CodexEmerald);
        assert_eq!(after_env.config.language, Language::En);
        assert_eq!(after_env.config.screen_mode, ScreenMode::Fullscreen);
        assert_eq!(after_env.config.emblem_mode, EmblemMode::Spin);
        assert!(after_env.config.no_color);
        assert!(after_env.config.ascii);
        assert_eq!(after_env.config.inline_height, 22);
        assert!(!after_env.config.mouse);

        // System $LANG alone must NOT auto-switch the app language away from English default:
        let system_zh = load_with_env(
            &CliOverrides::default(),
            &EnvSnapshot {
                lang: Some("zh_CN.UTF-8".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("system locale must keep English default");
        assert_eq!(system_zh.config.language, Language::En);

        // JUSTMUX_LANGUAGE takes precedence over JUSTMUX_LANG:
        let lang_env = load_with_env(
            &CliOverrides::default(),
            &EnvSnapshot {
                justmux_lang: Some("zh".into()),
                justmux_language: Some("en".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("JUSTMUX_LANGUAGE overrides JUSTMUX_LANG");
        assert_eq!(lang_env.config.language, Language::En);
        let lang_fallback = load_with_env(
            &CliOverrides::default(),
            &EnvSnapshot {
                justmux_lang: Some("zh-CN".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("JUSTMUX_LANG sets Zh");
        assert_eq!(lang_fallback.config.language, Language::Zh);

        // JUSTMUX_EMBLEM_MODE takes precedence over JUSTMUX_EMBLEM_SPIN, and both support aliases:
        let both_env = load_with_env(
            &CliOverrides::default(),
            &EnvSnapshot {
                justmux_emblem_spin: Some("0".into()),
                justmux_emblem_mode: Some("inverted-spin".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("JUSTMUX_EMBLEM_MODE overrides JUSTMUX_EMBLEM_SPIN");
        assert_eq!(both_env.config.emblem_mode, EmblemMode::InvertedSpin);

        for (raw, expected) in [
            ("0", EmblemMode::Static),
            ("false", EmblemMode::Static),
            ("static", EmblemMode::Static),
            ("NO", EmblemMode::Static),
            ("off", EmblemMode::Static),
            ("1", EmblemMode::Spin),
            ("True", EmblemMode::Spin),
            ("spin", EmblemMode::Spin),
            ("on", EmblemMode::Spin),
            ("inverted", EmblemMode::InvertedSpin),
            ("inverted-spin", EmblemMode::InvertedSpin),
            ("quad-hydra", EmblemMode::Spin),
            ("quad-cutout", EmblemMode::Spin),
            ("quad-prism", EmblemMode::Spin),
            ("invalid", EmblemMode::Spin),
            ("", EmblemMode::Spin),
        ] {
            let spin_loaded = load_with_env(
                &CliOverrides::default(),
                &EnvSnapshot {
                    justmux_emblem_spin: Some(raw.into()),
                    ..EnvSnapshot::default()
                },
            )
            .expect("JUSTMUX_EMBLEM_SPIN parsing");
            assert_eq!(
                spin_loaded.config.emblem_mode, expected,
                "JUSTMUX_EMBLEM_SPIN={raw:?}"
            );

            let mode_loaded = load_with_env(
                &CliOverrides::default(),
                &EnvSnapshot {
                    justmux_emblem_mode: Some(raw.into()),
                    ..EnvSnapshot::default()
                },
            )
            .expect("JUSTMUX_EMBLEM_MODE parsing");
            assert_eq!(
                mode_loaded.config.emblem_mode, expected,
                "JUSTMUX_EMBLEM_MODE={raw:?}"
            );
        }

        // CLI overrides env + file:
        let after_cli = load_with_env(
            &CliOverrides {
                config_path: Some(cfg_path),
                screen_mode: Some(ScreenMode::Inline),
                theme: Some("matrix".into()),
            },
            &env_over,
        )
        .expect("cli overrides env");
        assert_eq!(after_cli.config.screen_mode, ScreenMode::Inline);
        assert_eq!(after_cli.config.theme, ThemePreset::CyberMatrix);

        // Invalid CLI --theme returns ConfigError:
        let bad_cli = load_with_env(
            &CliOverrides {
                theme: Some("nonexistent-palette".into()),
                ..CliOverrides::default()
            },
            &EnvSnapshot::default(),
        )
        .expect_err("invalid --theme must fail");
        assert!(bad_cli.to_string().contains("nonexistent-palette"));

        // Malformed default config file returns ConfigError instead of silently ignoring.
        let bad_home = dir.path().join("bad_home");
        let bad_default = bad_home.join(DEFAULT_CONFIG_REL_PATH);
        fs::create_dir_all(bad_default.parent().expect("parent")).expect("mkdir");
        fs::write(&bad_default, "theme = \"aurora\"\nbad_key = true\n").expect("write bad default");
        let bad_err = load_with_env(
            &CliOverrides::default(),
            &EnvSnapshot {
                home_dir: Some(bad_home),
                ..EnvSnapshot::default()
            },
        )
        .expect_err("malformed existing default config must error");
        assert_eq!(bad_err.line(), Some(2));
        assert!(bad_err.to_string().contains("line 2:"));
    }

    #[test]
    fn term_dumb_and_non_utf8_locale_activate_fallbacks_and_effective_theme() {
        let env = EnvSnapshot {
            term: Some("dumb".into()),
            lang: Some("en_US.ISO8859-1".into()),
            ..EnvSnapshot::default()
        };
        let loaded = load_with_env(&CliOverrides::default(), &env).expect("fallback env");
        assert!(loaded.config.no_color);
        assert!(loaded.config.ascii);
        let eff = loaded.config.effective_theme();
        assert!(eff.no_color);
        assert!(eff.ascii);
        assert!(!eff.truecolor);
        assert_eq!(eff.preset, ThemePreset::Classic);

        let explicit_codex = Config {
            theme: ThemePreset::CodexEmerald,
            no_color: true,
            ..Config::default()
        }
        .effective_theme();
        assert_eq!(explicit_codex.preset, ThemePreset::CodexEmerald);
        assert!(explicit_codex.no_color);
        assert!(!explicit_codex.truecolor);
    }

    #[test]
    fn save_creates_missing_parent_directories_and_writes_valid_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a/b/c/config.toml");
        let cfg = Config {
            theme: ThemePreset::CyberMatrix,
            inline_height: 99, // verify save clamps out-of-range manual struct edits
            ..Config::default()
        };
        cfg.save(&nested).expect("save nested");
        let content = fs::read_to_string(&nested).expect("read saved");
        let parsed = parse_config_str(&content).expect("parse saved");
        assert_eq!(parsed.theme, Some(ThemePreset::CyberMatrix));
        assert_eq!(parsed.inline_height, Some(40));
    }

    #[test]
    fn tmux_comfort_config_parses_underscores_errors_and_converts() {
        let toml = "\
            [tmux]\n\
            tmux_mouse = true\n\
            tmux_history_limit = 50_000\n\
            tmux_base_index_one = true\n\
            tmux_renumber_windows = true\n\
            tmux_zero_escape_time = true\n";
        let partial = parse_config_str(toml).expect("parse comfort toml");
        assert_eq!(partial.tmux_mouse, Some(true));
        assert_eq!(partial.tmux_history_limit, Some(50_000));
        assert_eq!(partial.tmux_base_index_one, Some(true));
        assert_eq!(partial.tmux_renumber_windows, Some(true));
        assert_eq!(partial.tmux_zero_escape_time, Some(true));

        let mut cfg = Config::default();
        cfg.apply(&partial);
        let opts = cfg.tmux_comfort_options();
        assert!(opts.is_any_enabled());
        assert!(opts.mouse);
        assert_eq!(opts.history_limit, 50_000);
        assert!(opts.base_index_one);
        assert!(opts.renumber_windows);
        assert!(opts.zero_escape_time);

        let mut reset_cfg = cfg.clone();
        reset_cfg.set_tmux_comfort_options(crate::tmux_conf::TmuxComfortOptions::default());
        assert!(!reset_cfg.tmux_comfort_options().is_any_enabled());
        assert_eq!(reset_cfg.tmux_history_limit, 0);

        for bad in [
            "tmux_mouse = \"on\"\n",
            "tmux_history_limit = -10\n",
            "tmux_history_limit = \"50000\"\n",
            "tmux_history_limit = 999999999999999\n",
            "tmux_base_index_one = 1\n",
            "tmux_renumber_windows = \"yes\"\n",
            "tmux_zero_escape_time = 0\n",
        ] {
            let err = parse_config_str(bad).expect_err(&format!("should reject {bad:?}"));
            assert_eq!(err.line(), Some(1), "error was: {err}");
        }
    }

    #[test]
    fn live_stream_parsing_aliases_env_precedence_and_round_trip() {
        assert!(Config::default().live_stream);

        // Canonical key and aliases under [ui].
        for (raw, expected) in [
            ("[ui]\nlive_stream = false\n", false),
            ("[ui]\nlive_stream = true\n", true),
            ("auto_refresh = false\n", false),
            ("auto_refresh = true\n", true),
            ("stream_preview = false\n", false),
            ("stream_preview = true\n", true),
        ] {
            let parsed = parse_config_str(raw).expect(raw);
            assert_eq!(parsed.live_stream, Some(expected), "{raw}");
        }

        // Invalid TOML value fails with line number.
        let err = parse_config_str("live_stream = \"on\"\n").expect_err("reject non-bool");
        assert_eq!(err.line(), Some(1));

        // TOML round-trip for both false and true.
        for enabled in [false, true] {
            let cfg = Config {
                live_stream: enabled,
                ..Config::default()
            };
            let toml = cfg.to_toml();
            assert!(
                toml.contains(&format!("live_stream = {enabled}")),
                "to_toml missing live_stream = {enabled}: {toml}"
            );
            let partial = parse_config_str(&toml).expect("reparse");
            let mut restored = Config::default();
            restored.apply(&partial);
            assert_eq!(restored, cfg);
        }

        // Env override parsing (`true`/`false`/`1`/`0`/`on`/`off`, case-insensitive & trimmed; invalid ignored).
        for (raw, expected) in [
            ("true", true),
            ("TRUE", true),
            ("1", true),
            ("on", true),
            (" On ", true),
            ("false", false),
            ("FALSE", false),
            ("0", false),
            ("off", false),
            (" Off ", false),
            ("invalid", true),
            ("", true),
        ] {
            let loaded = load_with_env(
                &CliOverrides::default(),
                &EnvSnapshot {
                    justmux_live_stream: Some(raw.into()),
                    ..EnvSnapshot::default()
                },
            )
            .expect("JUSTMUX_LIVE_STREAM parsing");
            assert_eq!(
                loaded.config.live_stream, expected,
                "JUSTMUX_LIVE_STREAM={raw:?}"
            );
        }

        // Env overrides config.toml in both directions.
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg_path = dir.path().join("stream.toml");
        Config {
            live_stream: false,
            ..Config::default()
        }
        .save(&cfg_path)
        .expect("save stream=false");

        let env_enables = load_with_env(
            &CliOverrides {
                config_path: Some(cfg_path.clone()),
                ..CliOverrides::default()
            },
            &EnvSnapshot {
                justmux_live_stream: Some("on".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("env overrides false -> true");
        assert!(env_enables.config.live_stream);

        Config {
            live_stream: true,
            ..Config::default()
        }
        .save(&cfg_path)
        .expect("save stream=true");

        let env_disables = load_with_env(
            &CliOverrides {
                config_path: Some(cfg_path),
                ..CliOverrides::default()
            },
            &EnvSnapshot {
                justmux_live_stream: Some("0".into()),
                ..EnvSnapshot::default()
            },
        )
        .expect("env overrides true -> false");
        assert!(!env_disables.config.live_stream);
    }
}
