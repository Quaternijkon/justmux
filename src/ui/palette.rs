//! TrueColor theme presets, RGB gradient interpolation, and pill styling.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;

/// Embedded 48×24 Unicode Braille emblem displayed in spacious Home Studio modes.
pub const HYDRA_48X24: &str = include_str!("hydra_48x24.txt");

/// Number of discrete 5° rotation frames across a full 360° turn.
pub const HYDRA_ROTATION_STEPS: usize = 72;

/// Radius squared (`40.5^2`) separating the rotating inner Hydra core from the
/// stationary outer double ring across the `r = 40..41` zero-dot moat.
const INNER_SPIN_RADIUS_SQ: f64 = 40.5 * 40.5;

/// Radius squared (`42.0^2`) bridging the `r = 40.0..41.5` dark moat into the
/// stationary thick inner ring (`r = 41.5..45.5`) for [`EmblemMode::InvertedSpin`],
/// while preserving the stationary outer dark groove (`r = 45.5..46.5`) and thin
/// outer ring (`r = 46.5..48.0`) verbatim.
const INVERTED_RING_BRIDGE_RADIUS_SQ: f64 = 42.0 * 42.0;

static HYDRA_BITMAP: std::sync::OnceLock<[[u8; 96]; 96]> = std::sync::OnceLock::new();
static HYDRA_INNER_BITMAP: std::sync::OnceLock<[[u8; 96]; 96]> = std::sync::OnceLock::new();
static HYDRA_INVERTED_OUTER_BITMAP: std::sync::OnceLock<[[u8; 96]; 96]> =
    std::sync::OnceLock::new();

/// Animation & rendering style for the Home Studio Hydra medallion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmblemMode {
    Static,
    #[default]
    Spin,
    InvertedSpin,
}

impl EmblemMode {
    /// Canonical configuration value for this emblem mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Spin => "spin",
            Self::InvertedSpin => "inverted-spin",
        }
    }

    /// Whether the event loop should advance rotation frames over time.
    pub const fn should_animate(self) -> bool {
        matches!(self, Self::Spin | Self::InvertedSpin)
    }

    /// Cycle forward across all 3 styles (`Static -> Spin -> InvertedSpin -> Static`).
    pub const fn next(self) -> Self {
        match self {
            Self::Static => Self::Spin,
            Self::Spin => Self::InvertedSpin,
            Self::InvertedSpin => Self::Static,
        }
    }

    /// Cycle backward across all 3 styles.
    pub const fn prev(self) -> Self {
        match self {
            Self::Static => Self::InvertedSpin,
            Self::InvertedSpin => Self::Spin,
            Self::Spin => Self::Static,
        }
    }

    /// Parse an emblem mode name or legacy boolean alias case-insensitively.
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "static" | "off" | "none" | "false" | "0" | "no" => Some(Self::Static),
            "spin" | "dynamic" | "cutout" | "normal" | "on" | "true" | "1" | "yes" => {
                Some(Self::Spin)
            }
            "inverted-spin" | "inverted_spin" | "inverted" | "invert" | "reverse"
            | "reverse-spin" | "reverse_spin" => Some(Self::InvertedSpin),
            _ => None,
        }
    }
}

const BRAILLE_DOTS: [(usize, usize, u8); 8] = [
    (0, 0, 0x01),
    (0, 1, 0x02),
    (0, 2, 0x04),
    (0, 3, 0x40),
    (1, 0, 0x08),
    (1, 1, 0x10),
    (1, 2, 0x20),
    (1, 3, 0x80),
];

pub(crate) type Rgb = (u8, u8, u8);
pub(crate) type GradientStops = (Rgb, Rgb, Rgb);

/// Built-in visual presets pairing TrueColor palettes with a classic 16-color ANSI fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreset {
    #[default]
    Classic,
    AntigravityAurora,
    CodexEmerald,
    ClaudeWarmth,
    CyberMatrix,
}

impl ThemePreset {
    /// Ordered list of all presets shown in the interactive theme switcher.
    pub const ALL: &'static [ThemePreset] = &[
        Self::AntigravityAurora,
        Self::CodexEmerald,
        Self::ClaudeWarmth,
        Self::CyberMatrix,
        Self::Classic,
    ];

    /// Human-readable display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::AntigravityAurora => "Antigravity Aurora",
            Self::CodexEmerald => "Codex Emerald",
            Self::ClaudeWarmth => "Claude Warmth",
            Self::CyberMatrix => "Cyber Matrix",
        }
    }

    /// Short kebab-case identifier suitable for `JUSTMUX_THEME` and compact pills.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::AntigravityAurora => "aurora",
            Self::CodexEmerald => "codex",
            Self::ClaudeWarmth => "claude",
            Self::CyberMatrix => "matrix",
        }
    }

    /// Creative tribute origin label for this palette.
    pub const fn origin(self) -> &'static str {
        match self {
            Self::AntigravityAurora => "Google Antigravity",
            Self::CodexEmerald => "OpenAI Codex",
            Self::ClaudeWarmth => "Claude Code",
            Self::CyberMatrix => "Phosphor Neon",
            Self::Classic => "Terminal ANSI",
        }
    }

    /// Short description shown in theme pickers and previews.
    pub const fn tagline(self) -> &'static str {
        match self {
            Self::AntigravityAurora => "Weightless sky-cyan, indigo & violet nebula",
            Self::CodexEmerald => "Crisp obsidian, emerald & icy mint precision",
            Self::ClaudeWarmth => "Warm terracotta, amber & cream studio glow",
            Self::CyberMatrix => "Phosphor lime, electric cyan & terminal neon",
            Self::Classic => "Portable 16-color terminal ANSI compatibility",
        }
    }

    /// Compact icon badge for headers and status pills.
    pub const fn badge_icon(self, ascii: bool) -> &'static str {
        if ascii {
            match self {
                Self::AntigravityAurora => "*",
                Self::CodexEmerald => ">_",
                Self::ClaudeWarmth => "+",
                Self::CyberMatrix => "#",
                Self::Classic => "[]",
            }
        } else {
            match self {
                Self::AntigravityAurora => "✦",
                Self::CodexEmerald => ">_",
                Self::ClaudeWarmth => "✻",
                Self::CyberMatrix => "◈",
                Self::Classic => "◫",
            }
        }
    }

    /// Advance to the next preset in [`ThemePreset::ALL`].
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Match a preset by slug, display name, origin, or known alias (case-insensitive).
    pub fn from_name(raw: &str) -> Option<Self> {
        let normalized: String = raw
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect();
        if normalized.is_empty() {
            return None;
        }
        match normalized.as_str() {
            "aurora" | "antigravity" | "antigravityaurora" | "googleantigravity" | "agy" => {
                Some(Self::AntigravityAurora)
            }
            "codex" | "emerald" | "codexemerald" | "openaicodex" | "openai" => {
                Some(Self::CodexEmerald)
            }
            "claude" | "warmth" | "claudewarmth" | "claudecode" | "anthropic" | "terracotta" => {
                Some(Self::ClaudeWarmth)
            }
            "matrix" | "cyber" | "cybermatrix" | "phosphor" | "phosphorneon" | "neon" => {
                Some(Self::CyberMatrix)
            }
            "classic" | "ansi" | "terminal" | "terminalansi" | "default" => Some(Self::Classic),
            _ => None,
        }
    }

    pub(crate) const fn rgb_stops(self) -> GradientStops {
        match self {
            Self::AntigravityAurora => ((103, 212, 255), (129, 140, 248), (192, 132, 252)),
            Self::CodexEmerald => ((16, 185, 129), (45, 212, 191), (110, 231, 183)),
            Self::ClaudeWarmth => ((217, 119, 87), (245, 158, 11), (244, 208, 168)),
            Self::CyberMatrix => ((57, 255, 136), (0, 238, 255), (163, 255, 71)),
            Self::Classic => ((0, 200, 220), (80, 160, 255), (200, 120, 255)),
        }
    }
}

/// Interface display language across JustMux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    En,
    Zh,
}

impl Language {
    /// All supported interface languages in cycle order.
    pub const ALL: &'static [Self] = &[Self::En, Self::Zh];

    /// Canonical TOML / CLI code (`"en"` or `"zh"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }

    /// Human-readable language name respecting `--ascii`.
    pub const fn display_name(self, ascii: bool) -> &'static str {
        match self {
            Self::En => "English",
            Self::Zh => {
                if ascii {
                    "Chinese"
                } else {
                    "中文"
                }
            }
        }
    }

    /// Returns `true` when Simplified Chinese is active.
    pub const fn is_zh(self) -> bool {
        matches!(self, Self::Zh)
    }

    /// Toggle between English (`En`) and Simplified Chinese (`Zh`).
    pub const fn toggle(self) -> Self {
        match self {
            Self::En => Self::Zh,
            Self::Zh => Self::En,
        }
    }

    /// Parse a language specifier or alias from config / environment.
    pub fn from_name(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let lower = trimmed.to_ascii_lowercase();
        match lower.as_str() {
            "en" | "eng" | "english" | "en-us" | "en_us" | "en-gb" => Some(Self::En),
            "zh" | "zh-cn" | "zh_cn" | "zh-hans" | "cn" | "chinese" => Some(Self::Zh),
            _ if matches!(trimmed, "中文" | "简体中文" | "汉语") => Some(Self::Zh),
            _ => None,
        }
    }
}

/// Active composer interaction mode cycled via `Shift+Tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComposerMode {
    #[default]
    CommandJump,
    LiveRadar,
    QuickLaunch,
}

impl ComposerMode {
    /// Cycle through `CommandJump -> LiveRadar -> QuickLaunch -> CommandJump`.
    pub const fn next(self) -> Self {
        match self {
            Self::CommandJump => Self::LiveRadar,
            Self::LiveRadar => Self::QuickLaunch,
            Self::QuickLaunch => Self::CommandJump,
        }
    }

    /// Compact title badge rendered on the composer frame in English.
    pub const fn label(self, ascii: bool) -> &'static str {
        self.label_for(ascii, Language::En)
    }

    /// Compact localized title badge rendered on the composer frame.
    pub const fn label_for(self, ascii: bool, lang: Language) -> &'static str {
        match (lang, ascii) {
            (Language::Zh, true) => match self {
                Self::CommandJump => "> 命令与 @跳转",
                Self::LiveRadar => "# 实时雷达",
                Self::QuickLaunch => "! 快速启动",
            },
            (Language::Zh, false) => match self {
                Self::CommandJump => "▸ 命令与 @跳转",
                Self::LiveRadar => "◈ 实时雷达",
                Self::QuickLaunch => "⚡ 快速启动",
            },
            (Language::En, true) => match self {
                Self::CommandJump => "> Command & @Jump",
                Self::LiveRadar => "# Live Radar",
                Self::QuickLaunch => "! Quick Launch",
            },
            (Language::En, false) => match self {
                Self::CommandJump => "▸ Command & @Jump",
                Self::LiveRadar => "◈ Live Radar",
                Self::QuickLaunch => "⚡ Quick Launch",
            },
        }
    }

    /// Concise contextual input hint in English (never containing `"Type / for commands"`).
    pub const fn placeholder(self, ascii: bool) -> &'static str {
        self.placeholder_for(ascii, Language::En)
    }

    /// Concise localized contextual input hint (never containing `"Type / for commands"`).
    pub const fn placeholder_for(self, ascii: bool, lang: Language) -> &'static str {
        match (lang, ascii) {
            (Language::Zh, true) => match self {
                Self::CommandJump => " /命令 | @跳转 | 1-9 直达 | Down 菜单 | ? 帮助",
                Self::LiveRadar => " Up/Down 浏览会话 | Enter 附着 | Shift+Tab 模式",
                Self::QuickLaunch => " 输入工作区名 (Enter = 当前目录) | Shift+Tab 模式",
            },
            (Language::Zh, false) => match self {
                Self::CommandJump => " /命令 · @跳转 · 1-9 直达 · ↓ 菜单 · ? 帮助",
                Self::LiveRadar => " ↑↓ 浏览会话 · Enter 附着 · Shift+Tab 模式",
                Self::QuickLaunch => " 输入工作区名 (Enter = 当前目录) · Shift+Tab 模式",
            },
            (Language::En, true) => match self {
                Self::CommandJump => " /cmd | @jump | 1-9 attach | Down menu | ? help",
                Self::LiveRadar => " Up/Down browse sessions | Enter attach | Shift+Tab mode",
                Self::QuickLaunch => " workspace name (Enter = cwd) | Shift+Tab mode",
            },
            (Language::En, false) => match self {
                Self::CommandJump => " /cmd · @jump · 1-9 attach · ↓ menu · ? help",
                Self::LiveRadar => " ↑↓ browse sessions · Enter attach · Shift+Tab mode",
                Self::QuickLaunch => " workspace name (Enter = cwd) · Shift+Tab mode",
            },
        }
    }

    /// Frame border accent color associated with this mode.
    pub const fn border_color(self, theme: Theme) -> Color {
        match self {
            Self::CommandJump => theme.accent,
            Self::LiveRadar => theme.secondary,
            Self::QuickLaunch => theme.warning,
        }
    }
}

/// Theme-reactive 3-line pixel mascot with automatic pure-ASCII fallback.
pub const fn mascot_lines(theme: Theme) -> [&'static str; 3] {
    if theme.ascii {
        return match theme.preset {
            ThemePreset::ClaudeWarmth => [".---.", "|* *|", "`---'"],
            ThemePreset::AntigravityAurora => [" /*\\ ", "/ * \\", "+---+"],
            ThemePreset::CyberMatrix => ["+===+", "|## |", "+===+"],
            ThemePreset::CodexEmerald | ThemePreset::Classic => ["+---+", "|>_ |", "+---+"],
        };
    }
    match theme.preset {
        ThemePreset::Classic => ["+---+", "|>_ |", "+---+"],
        ThemePreset::ClaudeWarmth => ["▗▛▀▜▖", "▐ ●● ▌", "▝▙▄▟▘"],
        ThemePreset::CodexEmerald => ["┏━━━┓", "┃>_ ┃", "┗━━━┛"],
        ThemePreset::AntigravityAurora => [" /✦\\ ", "/ ◈ \\", "╰───╯"],
        ThemePreset::CyberMatrix => ["╔═══╗", "║◈◈ ║", "╚═══╝"],
    }
}

/// Active visual theme controlling colors, TrueColor gradients, and ASCII fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub error: Color,
    pub selected: Color,
    pub no_color: bool,
    pub ascii: bool,
    pub language: Language,
    pub preset: ThemePreset,
    pub secondary: Color,
    pub tertiary: Color,
    pub success: Color,
    pub warning: Color,
    pub surface: Color,
    pub truecolor: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            accent: Color::Cyan,
            text: Color::Reset,
            muted: Color::DarkGray,
            error: Color::Red,
            selected: Color::Cyan,
            no_color: false,
            ascii: false,
            language: Language::En,
            preset: ThemePreset::Classic,
            secondary: Color::LightBlue,
            tertiary: Color::Magenta,
            success: Color::Green,
            warning: Color::Yellow,
            surface: Color::Reset,
            truecolor: false,
        }
    }
}

impl Theme {
    /// Detect terminal color/unicode capabilities and apply `JUSTMUX_THEME`.
    pub fn from_env() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some()
            || std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"));
        let ascii = !locale_supports_unicode();
        let requested = std::env::var("JUSTMUX_THEME")
            .ok()
            .and_then(|value| ThemePreset::from_name(&value));
        let preset = if no_color {
            requested.unwrap_or(ThemePreset::Classic)
        } else {
            requested.unwrap_or(ThemePreset::AntigravityAurora)
        };
        Self {
            no_color,
            ascii,
            ..Self::default()
        }
        .with_preset(preset)
    }

    /// Return a copy of this theme updated to the requested [`ThemePreset`],
    /// preserving terminal capability flags.
    pub fn with_preset(self, preset: ThemePreset) -> Self {
        let base = match preset {
            ThemePreset::Classic => Self::default(),
            ThemePreset::AntigravityAurora => {
                let (a, b, c) = preset.rgb_stops();
                Self {
                    preset,
                    accent: Color::Rgb(a.0, a.1, a.2),
                    secondary: Color::Rgb(b.0, b.1, b.2),
                    tertiary: Color::Rgb(c.0, c.1, c.2),
                    success: Color::Rgb(74, 222, 160),
                    warning: Color::Rgb(251, 191, 36),
                    error: Color::Rgb(248, 113, 113),
                    selected: Color::Rgb(a.0, a.1, a.2),
                    text: Color::Reset,
                    muted: Color::Rgb(138, 148, 174),
                    surface: Color::Rgb(24, 29, 44),
                    truecolor: true,
                    no_color: false,
                    ascii: false,
                    language: Language::En,
                }
            }
            ThemePreset::CodexEmerald => {
                let (a, b, c) = preset.rgb_stops();
                Self {
                    preset,
                    accent: Color::Rgb(a.0, a.1, a.2),
                    secondary: Color::Rgb(b.0, b.1, b.2),
                    tertiary: Color::Rgb(c.0, c.1, c.2),
                    success: Color::Rgb(52, 211, 153),
                    warning: Color::Rgb(250, 204, 21),
                    error: Color::Rgb(251, 113, 133),
                    selected: Color::Rgb(a.0, a.1, a.2),
                    text: Color::Reset,
                    muted: Color::Rgb(132, 148, 145),
                    surface: Color::Rgb(18, 32, 29),
                    truecolor: true,
                    no_color: false,
                    ascii: false,
                    language: Language::En,
                }
            }
            ThemePreset::ClaudeWarmth => {
                let (a, b, c) = preset.rgb_stops();
                Self {
                    preset,
                    accent: Color::Rgb(a.0, a.1, a.2),
                    secondary: Color::Rgb(b.0, b.1, b.2),
                    tertiary: Color::Rgb(c.0, c.1, c.2),
                    success: Color::Rgb(110, 196, 138),
                    warning: Color::Rgb(245, 158, 11),
                    error: Color::Rgb(239, 98, 98),
                    selected: Color::Rgb(a.0, a.1, a.2),
                    text: Color::Reset,
                    muted: Color::Rgb(168, 152, 142),
                    surface: Color::Rgb(38, 28, 24),
                    truecolor: true,
                    no_color: false,
                    ascii: false,
                    language: Language::En,
                }
            }
            ThemePreset::CyberMatrix => {
                let (a, b, c) = preset.rgb_stops();
                Self {
                    preset,
                    accent: Color::Rgb(a.0, a.1, a.2),
                    secondary: Color::Rgb(b.0, b.1, b.2),
                    tertiary: Color::Rgb(c.0, c.1, c.2),
                    success: Color::Rgb(57, 255, 136),
                    warning: Color::Rgb(255, 214, 10),
                    error: Color::Rgb(255, 77, 109),
                    selected: Color::Rgb(a.0, a.1, a.2),
                    text: Color::Reset,
                    muted: Color::Rgb(116, 154, 134),
                    surface: Color::Rgb(14, 28, 22),
                    truecolor: true,
                    no_color: false,
                    ascii: false,
                    language: Language::En,
                }
            }
        };
        Self {
            no_color: self.no_color,
            ascii: self.ascii,
            language: self.language,
            truecolor: if self.no_color { false } else { base.truecolor },
            ..base
        }
    }

    /// Select localized copy based on `self.language` (`En` or `Zh`).
    pub const fn tr<'a>(self, en: &'a str, zh: &'a str) -> &'a str {
        match self.language {
            Language::En => en,
            Language::Zh => zh,
        }
    }

    pub fn selected_style(self) -> Style {
        self.style(self.text)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    }

    pub fn marker(self) -> &'static str {
        if self.ascii { ">" } else { "›" }
    }

    pub fn adapt(self, value: &str) -> String {
        if self.ascii {
            value
                .replace("↑↓", "Up/Down")
                .replace("←→", "Left/Right")
                .replace("↑", "Up")
                .replace("↓", "Down")
                .replace("←", "<-")
                .replace("→", "->")
                .replace("◀", "<")
                .replace("▶", ">")
                .replace("·", "|")
                .replace("›", ">")
                .replace("…", "...")
                .replace("✦", "*")
                .replace("✻", "*")
                .replace("◈", "#")
                .replace("◫", "[]")
                .replace("●", "*")
                .replace("○", "o")
                .replace("★", "*")
                .replace("⚙", "*")
                .replace("▸", ">")
                .replace(['⚡', '⚠'], "!")
                .replace("📺", "[TV]")
                .replace("⌂", "~")
                .replace("◇", "*")
                .replace(['─', '━', '═'], "-")
                .replace(['—', '–'], "-")
                .replace(['│', '┃', '║', '▐', '▌'], "|")
                .replace(
                    [
                        '╭', '╮', '╰', '╯', '├', '┤', '┬', '┴', '┼', '┏', '┓', '┗', '┛', '╔', '╗',
                        '╚', '╝',
                    ],
                    "+",
                )
                .replace(['█', '▛', '▀', '▜', '▙', '▄', '▟'], "#")
                .replace(['▗', '▖', '▝', '▘'], ".")
                .replace("•", "-")
                .replace("×", "x")
                .replace("✓", "*")
                .replace("∞", "inf")
                .replace("🎯", "*")
                .replace("🔥", "*")
        } else {
            value.to_owned()
        }
    }

    pub fn style(self, c: Color) -> Style {
        if self.no_color {
            Style::default()
        } else {
            Style::default().fg(c)
        }
    }

    pub(crate) fn pill_bg(self) -> Option<Color> {
        (self.truecolor && !self.no_color && self.surface != Color::Reset).then_some(self.surface)
    }

    /// Deep, crisp dark ink rendered atop the light pastel keycap fill.
    pub fn keycap_fg(self) -> Color {
        if !self.truecolor {
            return Color::Black;
        }
        match self.preset {
            ThemePreset::AntigravityAurora => Color::Rgb(15, 23, 42),
            ThemePreset::CodexEmerald => Color::Rgb(6, 38, 30),
            ThemePreset::ClaudeWarmth => Color::Rgb(41, 24, 15),
            ThemePreset::CyberMatrix => Color::Rgb(5, 32, 18),
            ThemePreset::Classic => Color::Black,
        }
    }

    /// Light theme-harmonized keycap fill behind highlighted keyboard shortcuts.
    pub fn keycap_bg(self) -> Option<Color> {
        if self.no_color {
            return None;
        }
        if !self.truecolor {
            return Some(Color::LightCyan);
        }
        match self.preset {
            ThemePreset::AntigravityAurora => Some(Color::Rgb(186, 230, 253)),
            ThemePreset::CodexEmerald => Some(Color::Rgb(167, 243, 208)),
            ThemePreset::ClaudeWarmth => Some(Color::Rgb(253, 230, 196)),
            ThemePreset::CyberMatrix => Some(Color::Rgb(187, 247, 208)),
            ThemePreset::Classic => Some(Color::LightCyan),
        }
    }

    /// High-contrast style for real keyboard key tokens (`Tab`, `1-9`, `Enter`, `Shift+Tab`, etc.).
    pub fn keycap_style(self) -> Style {
        if self.no_color {
            return Style::default().add_modifier(Modifier::BOLD);
        }
        let mut style = Style::default()
            .fg(self.keycap_fg())
            .add_modifier(Modifier::BOLD);
        if let Some(bg) = self.keycap_bg() {
            style = style.bg(bg);
        }
        style
    }
}

/// Linearly interpolate between two 24-bit RGB stops at `index / max_index`.
pub fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), index: usize, max_index: usize) -> Color {
    if max_index == 0 || index == 0 {
        return Color::Rgb(a.0, a.1, a.2);
    }
    if index >= max_index {
        return Color::Rgb(b.0, b.1, b.2);
    }
    let t = index as f64 / max_index as f64;
    let lerp = |start: u8, end: u8| -> u8 {
        let start = f64::from(start);
        let end = f64::from(end);
        (start + (end - start) * t).round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

/// Render `text` across a smooth 3-stop RGB gradient (`accent -> secondary -> tertiary`)
/// when TrueColor is active, falling back cleanly to `theme.style(theme.accent)`.
pub fn gradient_spans(text: &str, theme: Theme, bold: bool) -> Vec<Span<'static>> {
    let text = theme.adapt(text);
    if text.is_empty() {
        return Vec::new();
    }
    if !theme.truecolor || theme.no_color {
        let mut style = theme.style(theme.accent);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        return vec![Span::styled(text, style)];
    }
    let (default_a, default_b, default_c) = theme.preset.rgb_stops();
    let stop_a = rgb_of(theme.accent, default_a);
    let stop_b = rgb_of(theme.secondary, default_b);
    let stop_c = rgb_of(theme.tertiary, default_c);
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    if graphemes.len() <= 1 {
        let mut style = theme.style(Color::Rgb(stop_a.0, stop_a.1, stop_a.2));
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        return vec![Span::styled(text, style)];
    }
    let max_index = graphemes.len() - 1;
    graphemes
        .into_iter()
        .enumerate()
        .map(|(index, grapheme)| {
            let scaled = index * 2;
            let color = if scaled <= max_index {
                lerp_rgb(stop_a, stop_b, scaled, max_index)
            } else {
                lerp_rgb(stop_b, stop_c, scaled - max_index, max_index)
            };
            let mut style = theme.style(color);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            Span::styled(grapheme.to_owned(), style)
        })
        .collect()
}

/// Render a pill badge span that respects `theme.no_color` (returning uncolored `Span::raw`).
pub fn pill_span(
    label: impl Into<String>,
    fg: Color,
    bg: Option<Color>,
    theme: Theme,
    bold: bool,
) -> Span<'static> {
    let label = theme.adapt(&label.into());
    if theme.no_color {
        return Span::raw(label);
    }
    let mut style = theme.style(fg);
    if let Some(bg_color) = bg.filter(|&c| c != Color::Reset) {
        style = style.bg(bg_color);
    }
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Span::styled(label, style)
}

fn is_key_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn boundary_left_ok(s: &str, idx: usize) -> bool {
    s[..idx]
        .chars()
        .next_back()
        .is_none_or(|ch| !is_key_word_char(ch))
}

fn boundary_right_ok(s: &str, end: usize) -> bool {
    s[end..]
        .chars()
        .next()
        .is_none_or(|ch| !is_key_word_char(ch))
}

fn match_key_token(s: &str, idx: usize) -> Option<usize> {
    let rest = &s[idx..];
    for arrow in ["↑↓", "←→"] {
        if rest.starts_with(arrow) {
            return Some(arrow.len());
        }
    }
    if rest.starts_with('↑') && !rest.starts_with("↑ +") {
        return Some('↑'.len_utf8());
    }
    for arrow in ['↓', '←', '→'] {
        if rest.starts_with(arrow) {
            return Some(arrow.len_utf8());
        }
    }

    if boundary_left_ok(s, idx) {
        if rest.starts_with("Shift+Tab") && boundary_right_ok(s, idx + 9) {
            return Some(9);
        }
        if let Some(after_ctrl) = rest.strip_prefix("Ctrl+") {
            let bytes = after_ctrl.as_bytes();
            if bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b'/'
                && bytes[2].is_ascii_alphabetic()
                && boundary_right_ok(s, idx + 8)
            {
                return Some(8);
            }
            if !bytes.is_empty() && bytes[0].is_ascii_alphabetic() && boundary_right_ok(s, idx + 6)
            {
                return Some(6);
            }
        }
        if rest.starts_with("Alt")
            && (rest[3..].starts_with("/Shift") || rest[3..].starts_with('+'))
        {
            return Some(3);
        }
        if rest.starts_with("Shift")
            && (s[..idx].ends_with("Alt/") || rest[5..].starts_with('+'))
            && boundary_right_ok(s, idx + 5)
        {
            return Some(5);
        }

        const NAMED_AND_DIGIT_KEYS: [&str; 17] = [
            "PageDown", "PageUp", "Arrows", "Enter", "Space", "Right", "PgUp", "PgDn", "Left",
            "Down", "Tab", "Esc", "Up", "1-9", "1-6", "1-5", "1~9",
        ];
        for tok in NAMED_AND_DIGIT_KEYS {
            if rest.starts_with(tok) && boundary_right_ok(s, idx + tok.len()) {
                return Some(tok.len());
            }
        }

        if rest.starts_with("s Save") || rest.starts_with("s 保存") {
            return Some(1);
        }
        if rest.starts_with('t')
            && (s[..idx].ends_with("Enter/")
                || s[..idx].ends_with("Enter or ")
                || s[..idx].ends_with("Enter 或 "))
            && boundary_right_ok(s, idx + 1)
        {
            return Some(1);
        }
        if rest.starts_with("? Help") || rest.starts_with("? help") || rest.starts_with("? 帮助")
        {
            return Some(1);
        }
        if rest.starts_with('+')
            && (s[..idx].ends_with("· ") || s[..idx].ends_with("| ") || s[..idx].ends_with('('))
            && (rest[1..].starts_with(" new") || rest[1..].starts_with(" 新建"))
        {
            return Some(1);
        }
    }

    if rest.starts_with('/')
        && (idx == 0 || s[..idx].ends_with(' '))
        && (rest[1..].starts_with(" for commands")
            || rest[1..].starts_with(" Commands")
            || rest[1..].starts_with(" 打开命令")
            || rest[1..].starts_with(" 命令")
            || (s[..idx].ends_with("type ")
                && (rest[1..].starts_with('!') || rest[1..].starts_with(" or "))))
    {
        return Some(1);
    }

    None
}

/// Split `text` into spans that apply `theme.keycap_style()` to real keyboard key names
/// while leaving surrounding words and delimiters in `base_style` (width-preserving).
pub fn styled_key_hint_spans(text: &str, base_style: Style, theme: Theme) -> Vec<Span<'static>> {
    let adapted = theme.adapt(text);
    if adapted.is_empty() {
        return Vec::new();
    }
    let key_style = theme.keycap_style();
    let mut spans = Vec::new();
    let mut plain_start = 0;
    let mut idx = 0;
    while idx < adapted.len() {
        if let Some(len) = match_key_token(&adapted, idx) {
            if idx > plain_start {
                spans.push(Span::styled(
                    adapted[plain_start..idx].to_owned(),
                    base_style,
                ));
            }
            spans.push(Span::styled(adapted[idx..idx + len].to_owned(), key_style));
            idx += len;
            plain_start = idx;
        } else {
            let step = adapted[idx..].chars().next().map_or(1, char::len_utf8);
            idx += step;
        }
    }
    if plain_start < adapted.len() {
        spans.push(Span::styled(adapted[plain_start..].to_owned(), base_style));
    }
    spans
}

/// Build a single [`Line`] with real keyboard key names styled using `theme.keycap_style()`.
pub fn styled_key_hint_line(text: &str, base_style: Style, theme: Theme) -> Line<'static> {
    Line::from(styled_key_hint_spans(text, base_style, theme))
}

fn rgb_of(color: Color, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => fallback,
    }
}

fn decode_hydra_bitmap() -> [[u8; 96]; 96] {
    let mut bitmap = [[0_u8; 96]; 96];
    for (row, line) in HYDRA_48X24.lines().take(24).enumerate() {
        for (col, ch) in line.chars().take(48).enumerate() {
            let code = u32::from(ch);
            if !(0x2800..=0x28FF).contains(&code) {
                continue;
            }
            let mask = (code - 0x2800) as u8;
            for (dx, dy, bit) in BRAILLE_DOTS {
                bitmap[row * 4 + dy][col * 2 + dx] = u8::from((mask & bit) != 0);
            }
        }
    }
    bitmap
}

fn hydra_bitmap() -> &'static [[u8; 96]; 96] {
    HYDRA_BITMAP.get_or_init(decode_hydra_bitmap)
}

fn decode_hydra_inner_bitmap() -> [[u8; 96]; 96] {
    let mut bitmap = *hydra_bitmap();
    for (ty, row) in bitmap.iter_mut().enumerate() {
        let dy = ty as f64 - 47.5;
        for (tx, dot) in row.iter_mut().enumerate() {
            let dx = tx as f64 - 47.5;
            if dx * dx + dy * dy >= INNER_SPIN_RADIUS_SQ {
                *dot = 0;
            }
        }
    }
    bitmap
}

fn hydra_inner_bitmap() -> &'static [[u8; 96]; 96] {
    HYDRA_INNER_BITMAP.get_or_init(decode_hydra_inner_bitmap)
}

fn decode_inverted_outer_bitmap() -> [[u8; 96]; 96] {
    let mut bitmap = *hydra_bitmap();
    for (ty, row) in bitmap.iter_mut().enumerate() {
        let dy = ty as f64 - 47.5;
        for (tx, dot) in row.iter_mut().enumerate() {
            let dx = tx as f64 - 47.5;
            if dx * dx + dy * dy < INVERTED_RING_BRIDGE_RADIUS_SQ {
                *dot = 1;
            }
        }
    }
    bitmap
}

fn hydra_inverted_outer_bitmap() -> &'static [[u8; 96]; 96] {
    HYDRA_INVERTED_OUTER_BITMAP.get_or_init(decode_inverted_outer_bitmap)
}

fn sample_subdot(bitmap: &[[u8; 96]; 96], tx: usize, ty: usize, tw: usize, th: usize) -> bool {
    let x0 = (tx * 96) / tw;
    let x1 = (((tx + 1) * 96) / tw).max(x0 + 1).min(96);
    let y0 = (ty * 96) / th;
    let y1 = (((ty + 1) * 96) / th).max(y0 + 1).min(96);
    let mut ones = 0_usize;
    let total = (x1 - x0) * (y1 - y0);
    for row in &bitmap[y0..y1] {
        for &dot in &row[x0..x1] {
            ones += usize::from(dot);
        }
    }
    ones * 2 >= total
}

fn sample_inner_rotated_bilinear(
    bitmap: &[[u8; 96]; 96],
    rx: f64,
    ry: f64,
    sin_t: f64,
    cos_t: f64,
) -> bool {
    let ux = 47.5 + rx * cos_t + ry * sin_t;
    let uy = 47.5 - rx * sin_t + ry * cos_t;
    let ix = ux.floor() as isize;
    let iy = uy.floor() as isize;
    let fx = ux - ix as f64;
    let fy = uy - iy as f64;
    let sample = |x: isize, y: isize| -> f64 {
        if !(0..96).contains(&x) || !(0..96).contains(&y) {
            return 0.0;
        }
        if (x as f64 - 47.5).powi(2) + (y as f64 - 47.5).powi(2) >= INNER_SPIN_RADIUS_SQ {
            return 0.0;
        }
        f64::from(bitmap[y as usize][x as usize])
    };
    let val = sample(ix, iy) * (1.0 - fx) * (1.0 - fy)
        + sample(ix + 1, iy) * fx * (1.0 - fy)
        + sample(ix, iy + 1) * (1.0 - fx) * fy
        + sample(ix + 1, iy + 1) * fx * fy;
    val >= 0.45
}

fn sample_hydra_subdot(
    tx: usize,
    ty: usize,
    tw: usize,
    th: usize,
    inverted: bool,
    rot: Option<(f64, f64)>,
) -> bool {
    let px = (tx as f64 + 0.5) * (96.0 / tw as f64) - 0.5;
    let py = (ty as f64 + 0.5) * (96.0 / th as f64) - 0.5;
    let rx = px - 47.5;
    let ry = py - 47.5;
    if rx * rx + ry * ry >= INNER_SPIN_RADIUS_SQ {
        let outer = if inverted {
            hydra_inverted_outer_bitmap()
        } else {
            hydra_bitmap()
        };
        return if tw == 96 && th == 96 {
            outer[ty][tx] != 0
        } else {
            sample_subdot(outer, tx, ty, tw, th)
        };
    }
    let lit = match rot {
        Some((sin_t, cos_t)) => sample_inner_rotated_bilinear(hydra_bitmap(), rx, ry, sin_t, cos_t),
        None if tw == 96 && th == 96 => hydra_bitmap()[ty][tx] != 0,
        None => sample_subdot(hydra_inner_bitmap(), tx, ty, tw, th),
    };
    if inverted { !lit } else { lit }
}

pub(crate) const fn hydra_target_dimensions(max_width: u16, max_height: u16) -> Option<(u16, u16)> {
    if max_width >= 48 && max_height >= 24 {
        Some((48, 24))
    } else if max_width >= 32 && max_height >= 16 {
        Some((32, 16))
    } else if max_width >= 24 && max_height >= 12 {
        Some((24, 12))
    } else if max_width >= 16 && max_height >= 8 {
        Some((16, 8))
    } else {
        None
    }
}

fn encode_mask_char(mask: u8, ascii: bool) -> char {
    if ascii {
        match mask.count_ones() {
            0 => ' ',
            1..=2 => '.',
            3..=4 => ':',
            5..=6 => '+',
            _ => '#',
        }
    } else {
        char::from_u32(0x2800 + u32::from(mask)).unwrap_or(' ')
    }
}

fn encode_hydra_rows(
    cols: usize,
    row_count: usize,
    ascii: bool,
    is_lit: impl Fn(usize, usize) -> bool,
) -> Vec<String> {
    (0..row_count)
        .map(|cy| {
            let mut line = String::with_capacity(cols * if ascii { 1 } else { 3 });
            for cx in 0..cols {
                let mut mask = 0_u8;
                for (dx, dy, bit) in BRAILLE_DOTS {
                    if is_lit(cx * 2 + dx, cy * 4 + dy) {
                        mask |= bit;
                    }
                }
                line.push(encode_mask_char(mask, ascii));
            }
            if ascii {
                line.trim_end_matches(' ').to_owned()
            } else {
                line.trim_end_matches('\u{2800}').to_owned()
            }
        })
        .collect()
}

fn outer_ring_row_rgb(row_idx: usize, total_rows: usize, preset: ThemePreset) -> (u8, u8, u8) {
    let max_idx = total_rows.saturating_sub(1).max(1);
    let (stop_a, stop_b, stop_c) = preset.rgb_stops();
    let scaled = row_idx * 2;
    let color = if scaled <= max_idx {
        lerp_rgb(stop_a, stop_b, scaled, max_idx)
    } else {
        lerp_rgb(stop_b, stop_c, scaled - max_idx, max_idx)
    };
    rgb_of(color, stop_b)
}

fn outer_ring_row_style(row_idx: usize, total_rows: usize, theme: Theme) -> Style {
    if theme.no_color {
        Style::default()
    } else if theme.truecolor {
        let (r, g, b) = outer_ring_row_rgb(row_idx, total_rows, theme.preset);
        theme.style(Color::Rgb(r, g, b))
    } else if row_idx * 3 < total_rows {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else if row_idx * 3 < total_rows * 2 {
        theme.style(theme.secondary)
    } else {
        theme.style(theme.tertiary)
    }
}

fn style_monochrome_hydra_rows(rows: Vec<String>, theme: Theme) -> Vec<Line<'static>> {
    let total_rows = rows.len();
    rows.into_iter()
        .enumerate()
        .map(|(row_idx, row_text)| {
            if theme.no_color {
                Line::from(Span::raw(row_text))
            } else {
                Line::from(Span::styled(
                    row_text,
                    outer_ring_row_style(row_idx, total_rows, theme),
                ))
            }
        })
        .collect()
}

fn render_braille_hydra_rows(
    cols: usize,
    row_count: usize,
    angle_step: usize,
    mode: EmblemMode,
    ascii: bool,
) -> Vec<String> {
    let step = if mode.should_animate() {
        angle_step % HYDRA_ROTATION_STEPS
    } else {
        0
    };
    let rot = (step != 0).then(|| {
        let theta = (step as f64) * std::f64::consts::TAU / (HYDRA_ROTATION_STEPS as f64);
        theta.sin_cos()
    });
    let inverted = matches!(mode, EmblemMode::InvertedSpin);
    let tw = cols * 2;
    let th = row_count * 4;
    if step == 0 && !inverted && !ascii && (cols, row_count) == (48, 24) {
        return HYDRA_48X24.lines().take(24).map(str::to_owned).collect();
    }
    encode_hydra_rows(cols, row_count, ascii, |tx, ty| {
        sample_hydra_subdot(tx, ty, tw, th, inverted, rot)
    })
}

/// Render a compact fixed-grid Hydra medallion silhouette for `/config` option guides.
pub(crate) fn hydra_thumbnail_rows(
    cols: u16,
    row_count: u16,
    angle_step: usize,
    mode: EmblemMode,
    ascii: bool,
) -> Vec<String> {
    render_braille_hydra_rows(
        usize::from(cols.max(2)),
        usize::from(row_count.max(1)),
        angle_step,
        mode,
        ascii,
    )
}

/// Render a styled fixed-grid Hydra thumbnail matching the active theme gradient.
pub(crate) fn hydra_thumbnail_lines(
    cols: u16,
    row_count: u16,
    angle_step: usize,
    mode: EmblemMode,
    theme: Theme,
) -> Vec<Line<'static>> {
    let rows = hydra_thumbnail_rows(cols, row_count, angle_step, mode, theme.ascii);
    style_monochrome_hydra_rows(rows, theme)
}

/// Render the multi-scale, theme-reactive Hydra emblem in `mode` at `angle_step` (`5°` per step).
pub fn hydra_emblem_mode_frame(
    max_width: u16,
    max_height: u16,
    angle_step: usize,
    mode: EmblemMode,
    theme: Theme,
) -> Option<(u16, Vec<Line<'static>>)> {
    let (target_w, target_h) = hydra_target_dimensions(max_width, max_height)?;
    let rows = render_braille_hydra_rows(
        usize::from(target_w),
        usize::from(target_h),
        angle_step,
        mode,
        theme.ascii,
    );
    Some((target_w, style_monochrome_hydra_rows(rows, theme)))
}

/// Render the multi-scale, theme-reactive Hydra emblem at `angle_step` (`5°` per step).
pub fn hydra_emblem_frame(
    max_width: u16,
    max_height: u16,
    angle_step: usize,
    theme: Theme,
) -> Option<(u16, Vec<Line<'static>>)> {
    hydra_emblem_mode_frame(max_width, max_height, angle_step, EmblemMode::Spin, theme)
}

/// Render the multi-scale, theme-reactive Hydra emblem (`48×24`, `32×16`, `24×12`, or `16×8`).
pub fn hydra_emblem_lines(
    max_width: u16,
    max_height: u16,
    theme: Theme,
) -> Option<(u16, Vec<Line<'static>>)> {
    hydra_emblem_frame(max_width, max_height, 0, theme)
}

fn locale_supports_unicode() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .is_none_or(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_rows(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn emblem_mode_round_trip_and_cycling() {
        assert_eq!(EmblemMode::default(), EmblemMode::Spin);
        assert_eq!(EmblemMode::Static.as_str(), "static");
        assert_eq!(EmblemMode::Spin.as_str(), "spin");
        assert_eq!(EmblemMode::InvertedSpin.as_str(), "inverted-spin");

        assert!(!EmblemMode::Static.should_animate());
        assert!(EmblemMode::Spin.should_animate());
        assert!(EmblemMode::InvertedSpin.should_animate());

        assert_eq!(EmblemMode::Static.next(), EmblemMode::Spin);
        assert_eq!(EmblemMode::Spin.next(), EmblemMode::InvertedSpin);
        assert_eq!(EmblemMode::InvertedSpin.next(), EmblemMode::Static);

        assert_eq!(EmblemMode::Static.prev(), EmblemMode::InvertedSpin);
        assert_eq!(EmblemMode::InvertedSpin.prev(), EmblemMode::Spin);
        assert_eq!(EmblemMode::Spin.prev(), EmblemMode::Static);

        for (raw, expected) in [
            ("static", Some(EmblemMode::Static)),
            (" OFF ", Some(EmblemMode::Static)),
            ("false", Some(EmblemMode::Static)),
            ("0", Some(EmblemMode::Static)),
            ("spin", Some(EmblemMode::Spin)),
            ("Dynamic", Some(EmblemMode::Spin)),
            ("true", Some(EmblemMode::Spin)),
            ("inverted-spin", Some(EmblemMode::InvertedSpin)),
            ("INVERTED_SPIN", Some(EmblemMode::InvertedSpin)),
            ("reverse-spin", Some(EmblemMode::InvertedSpin)),
            ("quad-hydra", None),
            ("quad-cutout", None),
            ("quad-prism", None),
            ("pixel-static", None),
            ("pixel-spin", None),
            ("pixel-inverted-spin", None),
            ("shield-overlay", None),
            ("shield-cutout", None),
            ("bogus", None),
        ] {
            assert_eq!(
                EmblemMode::from_name(raw),
                expected,
                "from_name({raw:?}) mismatch"
            );
        }
    }

    #[test]
    fn hydra_emblem_mode_frame_handles_static_spin_inverted_and_ascii() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        for (mw, mh) in [(48_u16, 24_u16), (32, 16), (24, 12), (16, 8)] {
            let (ws0, static0) =
                hydra_emblem_mode_frame(mw, mh, 0, EmblemMode::Static, aurora).expect("static 0");
            let (ws9, static9) =
                hydra_emblem_mode_frame(mw, mh, 9, EmblemMode::Static, aurora).expect("static 9");
            let (wp0, spin0) =
                hydra_emblem_mode_frame(mw, mh, 0, EmblemMode::Spin, aurora).expect("spin 0");
            let (wp9, spin9) =
                hydra_emblem_mode_frame(mw, mh, 9, EmblemMode::Spin, aurora).expect("spin 9");
            let (wi0, inv0) = hydra_emblem_mode_frame(mw, mh, 0, EmblemMode::InvertedSpin, aurora)
                .expect("inverted 0");
            let (wi9, inv9) = hydra_emblem_mode_frame(mw, mh, 9, EmblemMode::InvertedSpin, aurora)
                .expect("inverted 9");

            assert_eq!((ws0, ws9, wp0, wp9, wi0, wi9), (mw, mw, mw, mw, mw, mw));
            let s0_rows = plain_rows(&static0);
            let s9_rows = plain_rows(&static9);
            let p0_rows = plain_rows(&spin0);
            let p9_rows = plain_rows(&spin9);
            let i0_rows = plain_rows(&inv0);
            let i9_rows = plain_rows(&inv9);

            assert_eq!(
                s0_rows, s9_rows,
                "Static mode must ignore angle_step at {mw}x{mh}"
            );
            assert_eq!(
                s0_rows, p0_rows,
                "Static matches Spin at step 0 ({mw}x{mh})"
            );
            assert_eq!(
                p0_rows[0], p9_rows[0],
                "Spin outer top ring stays stationary at {mw}x{mh}"
            );
            if mw >= 24 {
                assert_eq!(
                    p0_rows[0], i0_rows[0],
                    "InvertedSpin outer top ring matches normal at {mw}x{mh}"
                );
                assert_eq!(
                    i0_rows[0], i9_rows[0],
                    "InvertedSpin outer top ring stays stationary during rotation at {mw}x{mh}"
                );
            } else {
                assert_eq!(i0_rows.len(), usize::from(mh));
            }
            let mid = usize::from(mh) / 2;
            assert_ne!(
                p0_rows[mid], p9_rows[mid],
                "Spin inner core rotates at {mw}x{mh}"
            );
            assert_ne!(
                p0_rows[mid], i0_rows[mid],
                "InvertedSpin inner core differs from normal at {mw}x{mh}"
            );
            assert_ne!(
                i0_rows[mid], i9_rows[mid],
                "InvertedSpin inner cutout rotates at {mw}x{mh}"
            );
        }

        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..Theme::default().with_preset(ThemePreset::AntigravityAurora)
        };
        for mode in [
            EmblemMode::Static,
            EmblemMode::Spin,
            EmblemMode::InvertedSpin,
        ] {
            let (mw, l0) = hydra_emblem_mode_frame(48, 24, 0, mode, aurora).expect("mode 0");
            let (_, l9) = hydra_emblem_mode_frame(48, 24, 9, mode, aurora).expect("mode 9");
            assert_eq!((mw, l0.len(), l9.len()), (48, 24, 24));
            if mode.should_animate() {
                assert_ne!(
                    plain_rows(&l0),
                    plain_rows(&l9),
                    "{mode:?} must animate across steps"
                );
            } else {
                assert_eq!(plain_rows(&l0), plain_rows(&l9));
            }
            let (_, lines) = hydra_emblem_mode_frame(48, 24, 9, mode, ascii_nc).expect("ascii");
            assert_eq!(lines.len(), 24);
            for line in &lines {
                for span in &line.spans {
                    assert!(span.content.is_ascii());
                    assert_eq!(span.style.fg, None);
                    assert_eq!(span.style.bg, None);
                    assert_eq!(span.style.add_modifier, Modifier::empty());
                }
            }
        }
    }

    #[test]
    fn hydra_inverted_spin_preserves_size_and_rotates_only_inner_pattern() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);

        for (cols, rows) in [
            (48_usize, 24_usize),
            (32, 16),
            (24, 12),
            (16, 8),
            (10, 5),
            (8, 4),
        ] {
            let tw = cols * 2;
            let th = rows * 4;
            for step in [0_usize, 1, 9, 18, 27, 36] {
                let rot = (step != 0).then(|| {
                    let theta =
                        (step as f64) * std::f64::consts::TAU / (HYDRA_ROTATION_STEPS as f64);
                    theta.sin_cos()
                });
                for ty in 0..th {
                    let py = (ty as f64 + 0.5) * (96.0 / th as f64) - 0.5;
                    let ry = py - 47.5;
                    for tx in 0..tw {
                        let px = (tx as f64 + 0.5) * (96.0 / tw as f64) - 0.5;
                        let rx = px - 47.5;
                        if rx * rx + ry * ry < INNER_SPIN_RADIUS_SQ {
                            assert_eq!(
                                sample_hydra_subdot(tx, ty, tw, th, true, rot),
                                !sample_hydra_subdot(tx, ty, tw, th, false, rot),
                                "inner subdot ({tx},{ty}) at {cols}x{rows} step {step} must invert Spin exactly"
                            );
                        }
                    }
                }
            }

            for step in 0..HYDRA_ROTATION_STEPS {
                let rot = (step != 0).then(|| {
                    let theta =
                        (step as f64) * std::f64::consts::TAU / (HYDRA_ROTATION_STEPS as f64);
                    theta.sin_cos()
                });
                for ty in 0..th {
                    let py = (ty as f64 + 0.5) * (96.0 / th as f64) - 0.5;
                    let ry = py - 47.5;
                    for tx in 0..tw {
                        let px = (tx as f64 + 0.5) * (96.0 / tw as f64) - 0.5;
                        let rx = px - 47.5;
                        if rx * rx + ry * ry >= INNER_SPIN_RADIUS_SQ {
                            assert_eq!(
                                sample_hydra_subdot(tx, ty, tw, th, true, rot),
                                sample_hydra_subdot(tx, ty, tw, th, true, None),
                                "outer subdot ({tx},{ty}) at {cols}x{rows} step {step} moved!"
                            );
                        }
                    }
                }
            }
        }

        let canon = hydra_bitmap();
        let inv_outer = hydra_inverted_outer_bitmap();
        for ty in 0..96 {
            let dy = ty as f64 - 47.5;
            for tx in 0..96 {
                let dx = tx as f64 - 47.5;
                if dx * dx + dy * dy >= INVERTED_RING_BRIDGE_RADIUS_SQ {
                    assert_eq!(
                        inv_outer[ty][tx], canon[ty][tx],
                        "outer ring pixel ({tx},{ty}) altered"
                    );
                }
            }
        }

        let cell_diffs = |a: &[String], b: &[String]| -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            for (r, (la, lb)) in a.iter().zip(b.iter()).enumerate() {
                for c in 0..48 {
                    let ca = la.chars().nth(c).unwrap_or('\u{2800}');
                    let cb = lb.chars().nth(c).unwrap_or('\u{2800}');
                    if ca != cb {
                        out.push((r, c));
                    }
                }
            }
            out
        };

        let (_, s0_l) =
            hydra_emblem_mode_frame(48, 24, 0, EmblemMode::Spin, aurora).expect("spin 0");
        let (_, s9_l) =
            hydra_emblem_mode_frame(48, 24, 9, EmblemMode::Spin, aurora).expect("spin 9");
        let (_, s18_l) =
            hydra_emblem_mode_frame(48, 24, 18, EmblemMode::Spin, aurora).expect("spin 18");
        let (_, i0_l) =
            hydra_emblem_mode_frame(48, 24, 0, EmblemMode::InvertedSpin, aurora).expect("inv 0");
        let (_, i9_l) =
            hydra_emblem_mode_frame(48, 24, 9, EmblemMode::InvertedSpin, aurora).expect("inv 9");
        let (_, i18_l) =
            hydra_emblem_mode_frame(48, 24, 18, EmblemMode::InvertedSpin, aurora).expect("inv 18");

        let s0 = plain_rows(&s0_l);
        let s9 = plain_rows(&s9_l);
        let s18 = plain_rows(&s18_l);
        let i0 = plain_rows(&i0_l);
        let i9 = plain_rows(&i9_l);
        let i18 = plain_rows(&i18_l);

        assert_eq!(cell_diffs(&i0, &i9), cell_diffs(&s0, &s9));
        assert_eq!(cell_diffs(&i0, &i18), cell_diffs(&s0, &s18));

        for (mw, mh) in [(48_u16, 24_u16), (32, 16), (24, 12)] {
            let spin_0 = plain_rows(
                &hydra_emblem_mode_frame(mw, mh, 0, EmblemMode::Spin, aurora)
                    .expect("spin 0")
                    .1,
            );
            let inv_0 = plain_rows(
                &hydra_emblem_mode_frame(mw, mh, 0, EmblemMode::InvertedSpin, aurora)
                    .expect("inv 0")
                    .1,
            );
            let inv_9 = plain_rows(
                &hydra_emblem_mode_frame(mw, mh, 9, EmblemMode::InvertedSpin, aurora)
                    .expect("inv 9")
                    .1,
            );
            let inv_18 = plain_rows(
                &hydra_emblem_mode_frame(mw, mh, 18, EmblemMode::InvertedSpin, aurora)
                    .expect("inv 18")
                    .1,
            );
            let last = usize::from(mh) - 1;
            assert_eq!(inv_0[0], spin_0[0], "{mw}x{mh} row 0 step 0");
            assert_eq!(inv_9[0], spin_0[0], "{mw}x{mh} row 0 step 9");
            assert_eq!(inv_18[0], spin_0[0], "{mw}x{mh} row 0 step 18");
            assert_eq!(inv_0[last], spin_0[last], "{mw}x{mh} bottom row step 0");
            assert_eq!(inv_9[last], spin_0[last], "{mw}x{mh} bottom row step 9");
            assert_eq!(inv_18[last], spin_0[last], "{mw}x{mh} bottom row step 18");
        }
    }

    #[test]
    fn keycap_style_and_key_hint_tokenizer_highlight_real_keys_without_false_positives() {
        let presets = [
            ThemePreset::Classic,
            ThemePreset::AntigravityAurora,
            ThemePreset::CodexEmerald,
            ThemePreset::ClaudeWarmth,
            ThemePreset::CyberMatrix,
        ];
        for preset in presets {
            let theme = Theme::default().with_preset(preset);
            assert_ne!(
                theme.keycap_fg(),
                theme.accent,
                "keycap_fg must contrast with accent on {preset:?}"
            );
            assert!(
                theme.keycap_bg().is_some(),
                "light keycap fill must be set on {preset:?}"
            );
            assert_ne!(
                theme.keycap_bg(),
                Some(theme.surface),
                "keycap_bg must stand out from dark surface on {preset:?}"
            );
            assert_ne!(
                theme.keycap_bg(),
                Some(Color::Black),
                "keycap_bg must not use a black background on {preset:?}"
            );
            if let (Color::Rgb(fr, fg, fb), Some(Color::Rgb(br, bg, bb))) =
                (theme.keycap_fg(), theme.keycap_bg())
            {
                let fg_sum = u16::from(fr) + u16::from(fg) + u16::from(fb);
                let bg_sum = u16::from(br) + u16::from(bg) + u16::from(bb);
                assert!(
                    bg_sum > 540 && fg_sum < 150,
                    "expected light pastel fill + deep ink on {preset:?}, got fg={fg_sum} bg={bg_sum}"
                );
            }
            assert!(
                theme.keycap_style().add_modifier.contains(Modifier::BOLD),
                "keycap_style must be bold on {preset:?}"
            );
        }
        let classic = Theme::default().with_preset(ThemePreset::Classic);
        assert_eq!(classic.keycap_bg(), Some(Color::LightCyan));
        assert_eq!(classic.keycap_fg(), Color::Black);

        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let muted = aurora.style(aurora.muted);
        let key_style = aurora.keycap_style();

        let sample = "Tab Switch · / Commands · 1-9 attach · Shift+Tab Mode · s Save · Enter/t Apply · Ctrl+A/E move · ↑↓ · ←→ · PgUp/PgDn · ? Help · + new";
        let spans = styled_key_hint_spans(sample, muted, aurora);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, aurora.adapt(sample));

        let highlighted: Vec<&str> = spans
            .iter()
            .filter(|s| s.style == key_style)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            highlighted,
            vec![
                "Tab",
                "/",
                "1-9",
                "Shift+Tab",
                "s",
                "Enter",
                "t",
                "Ctrl+A/E",
                "↑↓",
                "←→",
                "PgUp",
                "PgDn",
                "?",
                "+",
            ]
        );

        let zh_sample = "Tab 切换 · / 打开命令 · 1-9 快速附着 · Shift+Tab 模式 · s 保存 · Enter 或 t 应用 · + 新建 · ? 帮助";
        let zh_spans = styled_key_hint_spans(zh_sample, muted, aurora);
        let zh_highlighted: Vec<&str> = zh_spans
            .iter()
            .filter(|s| s.style == key_style)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            zh_highlighted,
            vec!["Tab", "/", "1-9", "Shift+Tab", "s", "Enter", "t", "+", "?"]
        );

        let non_key_prose =
            "/resume · 8..=40 · Save · session · Alt-screen · backs up · on the right · ↑ +12/48";
        let plain_spans = styled_key_hint_spans(non_key_prose, muted, aurora);
        assert!(
            plain_spans.iter().all(|s| s.style == muted),
            "unexpected key highlight in prose: {plain_spans:?}"
        );

        let no_color_ascii = Theme {
            no_color: true,
            ascii: true,
            truecolor: false,
            ..aurora
        };
        let nc_spans = styled_key_hint_spans(
            "←→ / Space · ↑↓ · Ctrl+T",
            no_color_ascii.style(Color::Reset),
            no_color_ascii,
        );
        for span in &nc_spans {
            assert_eq!(span.style.fg, None);
            assert_eq!(span.style.bg, None);
        }
        let nc_joined: String = nc_spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(nc_joined, "Left/Right / Space | Up/Down | Ctrl+T");
        let nc_keys: Vec<&str> = nc_spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            nc_keys,
            vec!["Left", "Right", "Space", "Up", "Down", "Ctrl+T"]
        );
    }

    #[test]
    fn language_round_trip_toggle_and_preset_preservation() {
        assert_eq!(Language::default(), Language::En);
        assert_eq!(Language::ALL, [Language::En, Language::Zh]);
        assert_eq!(Language::En.toggle(), Language::Zh);
        assert_eq!(Language::Zh.toggle(), Language::En);
        assert_eq!(Language::from_name("en"), Some(Language::En));
        assert_eq!(Language::from_name("English"), Some(Language::En));
        assert_eq!(Language::from_name("zh"), Some(Language::Zh));
        assert_eq!(Language::from_name("zh-CN"), Some(Language::Zh));
        assert_eq!(Language::from_name("中文"), Some(Language::Zh));
        assert_eq!(Language::from_name("fr"), None);

        let mut theme = Theme::default();
        assert_eq!(theme.tr("Home", "主页"), "Home");
        theme.language = Language::Zh;
        assert_eq!(theme.tr("Home", "主页"), "主页");
        let switched = theme.with_preset(ThemePreset::ClaudeWarmth);
        assert_eq!(switched.language, Language::Zh);
        assert_eq!(switched.tr("Home", "主页"), "主页");
    }
}
