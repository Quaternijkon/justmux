//! TrueColor theme presets, RGB gradient interpolation, and pill styling.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use unicode_segmentation::UnicodeSegmentation;

type Rgb = (u8, u8, u8);
type GradientStops = (Rgb, Rgb, Rgb);

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

    const fn rgb_stops(self) -> GradientStops {
        match self {
            Self::AntigravityAurora => ((103, 212, 255), (129, 140, 248), (192, 132, 252)),
            Self::CodexEmerald => ((16, 185, 129), (45, 212, 191), (110, 231, 183)),
            Self::ClaudeWarmth => ((217, 119, 87), (245, 158, 11), (244, 208, 168)),
            Self::CyberMatrix => ((57, 255, 136), (0, 238, 255), (163, 255, 71)),
            Self::Classic => ((0, 200, 220), (80, 160, 255), (200, 120, 255)),
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

    /// Compact title badge rendered on the composer frame.
    pub const fn label(self, ascii: bool) -> &'static str {
        if ascii {
            match self {
                Self::CommandJump => "> Command & @Jump",
                Self::LiveRadar => "# Live Radar",
                Self::QuickLaunch => "! Quick Launch",
            }
        } else {
            match self {
                Self::CommandJump => "▸ Command & @Jump",
                Self::LiveRadar => "◈ Live Radar",
                Self::QuickLaunch => "⚡ Quick Launch",
            }
        }
    }

    /// Concise contextual input hint (never containing `"Type / for commands"`).
    pub const fn placeholder(self, ascii: bool) -> &'static str {
        if ascii {
            match self {
                Self::CommandJump => " /cmd | @jump | 1-9 attach | Down menu | ? help",
                Self::LiveRadar => " Up/Down browse sessions | Enter attach | Shift+Tab mode",
                Self::QuickLaunch => " workspace name (Enter = cwd) | Shift+Tab mode",
            }
        } else {
            match self {
                Self::CommandJump => " /cmd · @jump · 1-9 attach · ↓ menu · ? help",
                Self::LiveRadar => " ↑↓ browse sessions · Enter attach · Shift+Tab mode",
                Self::QuickLaunch => " workspace name (Enter = cwd) · Shift+Tab mode",
            }
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
    /// Detect terminal color/unicode capabilities and apply `JUSTMUX_THEME` if set.
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
    /// preserving `no_color` and `ascii` terminal capabilities.
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
                }
            }
        };
        Self {
            no_color: self.no_color,
            ascii: self.ascii,
            truecolor: if self.no_color { false } else { base.truecolor },
            ..base
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

fn rgb_of(color: Color, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => fallback,
    }
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
