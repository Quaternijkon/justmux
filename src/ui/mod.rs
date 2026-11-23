//! Small, tmux-agnostic Ratatui widgets used by JustMux.

mod controller;
pub use controller::*;
mod desktop;
pub use desktop::*;
mod palette;
pub use palette::*;
mod split;
pub use split::*;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::commands::{CommandKind, MERGED_COMMANDS, registry};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}
impl TextInput {
    pub fn new(v: impl Into<String>) -> Self {
        let value = v.into();
        let cursor = value.graphemes(true).count();
        Self { value, cursor }
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn text(&self) -> &str {
        &self.value
    }
    pub fn set_value(&mut self, v: impl Into<String>) {
        self.value = v.into();
        self.cursor = self.value.graphemes(true).count();
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// Insert a single-line paste and leave the cursor on a grapheme boundary.
    pub fn insert_str(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        let byte = self.byte_offset(self.cursor);
        self.value.insert_str(byte, &clean);
        let caret = byte + clean.len();
        self.cursor = self
            .value
            .grapheme_indices(true)
            .take_while(|(i, _)| *i < caret)
            .count();
    }
    fn byte_offset(&self, cursor: usize) -> usize {
        self.value
            .grapheme_indices(true)
            .nth(cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }
    fn delete_range(&mut self, start: usize, end: usize) {
        let range = self.byte_offset(start)..self.byte_offset(end);
        self.value.replace_range(range, "");
        self.cursor = start.min(self.value.graphemes(true).count());
    }
    fn previous_word(&self) -> usize {
        let gs: Vec<&str> = self.value.graphemes(true).collect();
        let mut i = self.cursor;
        while i > 0 && gs[i - 1].chars().all(char::is_whitespace) {
            i -= 1;
        }
        while i > 0 && !gs[i - 1].chars().all(char::is_whitespace) {
            i -= 1;
        }
        i
    }
    fn next_word(&self) -> usize {
        let gs: Vec<&str> = self.value.graphemes(true).collect();
        let mut i = self.cursor;
        while i < gs.len() && !gs[i].chars().all(char::is_whitespace) {
            i += 1;
        }
        while i < gs.len() && gs[i].chars().all(char::is_whitespace) {
            i += 1;
        }
        i
    }
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let count = self.value.graphemes(true).count();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl || alt {
            match key.code {
                KeyCode::Char('a') if ctrl => self.cursor = 0,
                KeyCode::Char('e') if ctrl => self.cursor = count,
                KeyCode::Char('u') if ctrl => self.delete_range(0, self.cursor),
                KeyCode::Char('k') if ctrl => self.delete_range(self.cursor, count),
                KeyCode::Char('w') if ctrl => self.delete_range(self.previous_word(), self.cursor),
                KeyCode::Backspace if alt => self.delete_range(self.previous_word(), self.cursor),
                KeyCode::Char('b') if alt => self.cursor = self.previous_word(),
                KeyCode::Left if ctrl || alt => self.cursor = self.previous_word(),
                KeyCode::Char('f') if alt => self.cursor = self.next_word(),
                KeyCode::Right if ctrl || alt => self.cursor = self.next_word(),
                KeyCode::Char('d') if alt => self.delete_range(self.cursor, self.next_word()),
                _ => return false,
            }
            return true;
        }
        match key.code {
            KeyCode::Char(c) if !c.is_control() => self.insert_str(&c.to_string()),
            KeyCode::Backspace if self.cursor > 0 => {
                self.delete_range(self.cursor - 1, self.cursor)
            }
            KeyCode::Delete if self.cursor < count => {
                self.delete_range(self.cursor, self.cursor + 1)
            }
            KeyCode::Left if self.cursor > 0 => self.cursor -= 1,
            KeyCode::Right if self.cursor < count => self.cursor += 1,
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = count,
            _ => return false,
        }
        true
    }
    pub fn rendered(&self, focused: bool, theme: Theme) -> Line<'static> {
        self.rendered_width(focused, theme, u16::MAX)
    }
    /// Keep the caret visible without cutting a wide or combined grapheme.
    pub fn rendered_width(&self, focused: bool, theme: Theme, width: u16) -> Line<'static> {
        let width = usize::from(width);
        if width == 0 {
            return Line::default();
        }
        let gs: Vec<&str> = self.value.graphemes(true).collect();
        let mut start = 0;
        let caret_width = gs
            .get(self.cursor)
            .map_or(1, |g| g.width().max(1))
            .min(width);
        while start < self.cursor
            && gs[start..self.cursor]
                .iter()
                .map(|g| g.width())
                .sum::<usize>()
                + caret_width
                > width
        {
            start += 1;
        }
        let mut spans = Vec::new();
        let mut used = 0;
        for (i, g) in gs.iter().enumerate().skip(start) {
            let cells = g.width();
            if used + cells > width {
                break;
            }
            spans.push(if focused && i == self.cursor {
                Span::styled((*g).to_owned(), theme.selected_style())
            } else {
                Span::raw((*g).to_owned())
            });
            used += cells;
        }
        if focused && self.cursor == gs.len() && used < width {
            spans.push(Span::styled(" ", theme.selected_style()));
        }
        Line::from(spans)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandItem<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub alias: Option<&'a str>,
    pub indices: &'a [usize],
}

/// Split a label into contiguous styled runs at grapheme boundaries.
pub(crate) fn highlighted_spans<'a>(
    text: &'a str,
    indices: &[usize],
    style: Style,
) -> Vec<Span<'a>> {
    highlighted_spans_with_match(
        text,
        indices,
        style,
        style.add_modifier(Modifier::UNDERLINED),
    )
}

fn highlighted_spans_with_match<'a>(
    text: &'a str,
    indices: &[usize],
    style: Style,
    match_style: Style,
) -> Vec<Span<'a>> {
    if indices.is_empty() {
        return vec![Span::styled(text, style)];
    }
    let mut spans = Vec::new();
    let mut start = 0;
    let mut matched = indices.binary_search(&0).is_ok();
    for (index, (byte, _)) in text.grapheme_indices(true).enumerate().skip(1) {
        let next = indices.binary_search(&index).is_ok();
        if next != matched {
            spans.push(Span::styled(
                &text[start..byte],
                if matched { match_style } else { style },
            ));
            start = byte;
            matched = next;
        }
    }
    spans.push(Span::styled(
        &text[start..],
        if matched { match_style } else { style },
    ));
    spans
}

/// Persistent bottom composer state. The app owns command execution; this
/// widget only keeps the editable text and exposes it for rendering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    input: TextInput,
}
impl Composer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn input(&self) -> &TextInput {
        &self.input
    }
    pub fn input_mut(&mut self) -> &mut TextInput {
        &mut self.input
    }
    pub fn text(&self) -> &str {
        self.input.value()
    }
    pub fn clear(&mut self) {
        self.input.set_value("");
    }
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.input.handle_key(key)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HomeDetails<'a> {
    pub directory: Option<&'a str>,
    pub context: &'a str,
    pub session: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Optional live tmux telemetry & subtle badge shown on the home dashboard.
#[derive(Debug, Clone, Copy, Default)]
pub struct HomeTelemetry<'a> {
    pub sessions: Option<usize>,
    pub windows: Option<usize>,
    pub attached: Option<usize>,
    pub clients: Option<usize>,
    pub hint_badge: Option<&'a str>,
}

/// Live tmux session summary displayed in the home studio cards and radar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSessionEntry {
    pub name: String,
    pub windows: usize,
    pub attached: bool,
    pub current: bool,
    pub active_window: String,
    pub badge: Option<&'static str>,
}

/// Captured live terminal buffer preview shown inside studio cards and pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanePreview<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub lines: &'a [String],
    pub scroll: usize,
    /// Optional search query highlighted across visible preview lines.
    pub search_query: Option<&'a str>,
    /// True while the user is actively editing the inline preview search bar.
    pub search_active: bool,
    /// Total matched occurrences across the captured buffer.
    pub match_count: usize,
    /// 1-based index of the currently focused match (`0` if none).
    pub current_match: usize,
    /// Multi-pane indicator `(current_1based, total_panes)` when window has > 1 pane.
    pub pane_index: Option<(usize, usize)>,
    /// True when the active pane is zoomed (`resize-pane -Z`).
    pub zoomed: bool,
    /// True when `synchronize-panes` is enabled on the window.
    pub synchronized: bool,
    /// True when `pipe-pane` live logging is active on the pane.
    pub piping: bool,
    /// Render clickable `[f Search]`, `[y Copy]`, `[Log]`, `[Pipe]` pills & window controls on card chrome.
    pub show_actions: bool,
    /// True while Live Interactive Preview (PIP direct keystroke forwarding) is active.
    pub pip_active: bool,
    /// Currently hovered preview action pill / window control button.
    pub hovered_action: Option<PreviewAction>,
    /// True when mouse hovers the right-edge VS Code scrollbar.
    pub scrollbar_hovered: bool,
    /// True while mouse drags the right-edge VS Code scrollbar thumb.
    pub scrollbar_dragging: bool,
    /// 0-based line indices into `lines` containing search matches (rendered as amber scrollbar ticks).
    pub search_match_lines: &'a [usize],
    /// 0-based line indices into `lines` containing errors/warnings (rendered as red scrollbar ticks).
    pub error_lines: &'a [usize],
    /// 0-indexed `(visible_row, char_col)` drag-selection range relative to `content_rect`.
    pub selection_range: Option<((usize, usize), (usize, usize))>,
    /// Optional VS Code edge dock drop-zone overlay highlighted over the preview card.
    pub dock_highlight: Option<DockZone>,
}

/// `@target` completion candidate rendered by the mention popup drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionItem<'a> {
    pub token: &'a str,
    pub kind_badge: &'a str,
    pub detail: &'a str,
    pub indices: &'a [usize],
}

/// Rich interactive dashboard state for [`render_home_studio`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HomeDashboardState<'a> {
    pub mode: ComposerMode,
    pub sessions: &'a [HomeSessionEntry],
    pub mentions: &'a [MentionItem<'a>],
    pub mention_selected: usize,
    pub radar_selected: usize,
    pub preview: Option<PanePreview<'a>>,
    pub tip_step: usize,
    pub emblem_step: usize,
}

/// Target scope toggled via `Tab` during Home matrix rename/kill actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetScope {
    #[default]
    Window,
    Session,
}

impl TargetScope {
    /// Switch between window scope and session scope.
    pub const fn toggle(self) -> Self {
        match self {
            Self::Window => Self::Session,
            Self::Session => Self::Window,
        }
    }
}

/// Interactive command action driving the unified Home Session×Window Matrix composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatrixAction {
    #[default]
    Open,
    Rename(TargetScope),
    Kill(TargetScope),
    Split,
    Window,
    Inspect,
}

impl MatrixAction {
    /// Cycle across the four clickable Home Matrix workspace tabs:
    /// `Open -> Rename(scope) -> Kill(scope) -> Split -> Open`.
    pub const fn cycle_workspace_tab(self, session_selected: bool) -> Self {
        let scope = if session_selected {
            TargetScope::Session
        } else {
            TargetScope::Window
        };
        match self {
            Self::Open | Self::Window | Self::Inspect => Self::Rename(scope),
            Self::Rename(_) => Self::Kill(scope),
            Self::Kill(_) => Self::Split,
            Self::Split => Self::Open,
        }
    }

    /// Border highlight color paired with this Home matrix action.
    pub const fn border_color(self, theme: Theme) -> Color {
        match self {
            Self::Open => theme.accent,
            Self::Rename(_) => theme.warning,
            Self::Kill(_) => theme.error,
            Self::Split | Self::Inspect => theme.secondary,
            Self::Window => theme.success,
        }
    }

    /// Top-left composer border title shown in [`ComposerMode::CommandJump`].
    pub const fn composer_title_for(self, ascii: bool, lang: Language) -> &'static str {
        let zh = lang.is_zh() && !ascii;
        match self {
            Self::Open => {
                if ascii {
                    "> Resume | Search: session or window"
                } else if zh {
                    "▸ 恢复连接 · 搜索会话或窗口"
                } else {
                    "▸ Resume · Search: session or window"
                }
            }
            Self::Rename(TargetScope::Session) => {
                if ascii {
                    "> Rename session [Down: window]"
                } else if zh {
                    "✎ 重命名会话 · ↓ 选窗口 · Enter 确认"
                } else {
                    "✎ Rename session · ↓ window · Enter rename"
                }
            }
            Self::Rename(TargetScope::Window) => {
                if ascii {
                    "> Rename window [Up: session]"
                } else if zh {
                    "✎ 重命名窗口 · ↑ 选会话 · Enter 确认"
                } else {
                    "✎ Rename window · ↑ session · Enter rename"
                }
            }
            Self::Kill(TargetScope::Window) => {
                if ascii {
                    "! Kill window [Up: session]"
                } else if zh {
                    "✕ 关闭窗口 · ↑ 选会话 · Enter 确认"
                } else {
                    "✕ Kill window · ↑ session · Enter confirm"
                }
            }
            Self::Kill(TargetScope::Session) => {
                if ascii {
                    "! Kill session [Down: window]"
                } else if zh {
                    "✕ 关闭会话 · ↓ 选窗口 · Enter 确认"
                } else {
                    "✕ Kill session · ↓ window · Enter confirm"
                }
            }
            Self::Split => {
                if ascii {
                    "# Split window pane (Enter layout)"
                } else if zh {
                    "◫ 拆分窗格 · Enter 选择布局"
                } else {
                    "◫ Split window pane · Enter layout"
                }
            }
            Self::Window => {
                if ascii {
                    "+ Window (+ new | Enter actions)"
                } else if zh {
                    "⊞ 窗口管理 · + 新建 · Enter 操作"
                } else {
                    "⊞ Window · + new · Enter actions"
                }
            }
            Self::Inspect => {
                if ascii {
                    "# Sessions & Windows (Enter actions)"
                } else if zh {
                    "◈ 会话与窗口列表 · Enter 操作"
                } else {
                    "◈ Sessions & Windows · Enter actions"
                }
            }
        }
    }

    /// Top-left composer border title shown in [`ComposerMode::CommandJump`].
    pub const fn composer_title(self, ascii: bool) -> &'static str {
        self.composer_title_for(ascii, Language::En)
    }

    /// Compact pill badge displayed on the Home card header for non-default actions.
    pub const fn mode_pill_for(self, lang: Language) -> Option<&'static str> {
        let zh = lang.is_zh();
        match self {
            Self::Open => None,
            Self::Rename(TargetScope::Session) => Some(if zh {
                "重命名会话"
            } else {
                "RENAME SESSION"
            }),
            Self::Rename(TargetScope::Window) => Some(if zh {
                "重命名窗口"
            } else {
                "RENAME WINDOW"
            }),
            Self::Kill(TargetScope::Window) => {
                Some(if zh { "关闭窗口" } else { "KILL WINDOW" })
            }
            Self::Kill(TargetScope::Session) => {
                Some(if zh { "关闭会话" } else { "KILL SESSION" })
            }
            Self::Split => Some(if zh { "拆分窗格" } else { "SPLIT" }),
            Self::Window => Some(if zh { "窗口" } else { "WINDOW" }),
            Self::Inspect => Some(if zh { "查看" } else { "INSPECT" }),
        }
    }

    /// Compact pill badge displayed on the Home card header for non-default actions.
    pub const fn mode_pill(self) -> Option<&'static str> {
        self.mode_pill_for(Language::En)
    }
}

/// Interactive state for the unified non-columnar [`render_workspace_home`] chrome.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspaceHomeSpec<'a> {
    pub details: HomeDetails<'a>,
    pub telemetry: HomeTelemetry<'a>,
    pub action: MatrixAction,
    pub composer_mode: ComposerMode,
    pub sessions: &'a [HomeSessionEntry],
    pub mentions: &'a [MentionItem<'a>],
    pub mention_selected: usize,
    pub radar_selected: usize,
    pub preview: Option<PanePreview<'a>>,
    pub live_preview: bool,
    pub inside_tmux: bool,
    pub emblem_step: usize,
    pub emblem_mode: EmblemMode,
    pub popup: &'a [CommandItem<'a>],
    pub popup_selected: usize,
    pub menubar: DesktopMenubarState<'a>,
    /// Adjustable upper-preview split percentage (`25..=75`; `0` uses responsive default).
    pub home_split_ratio: u16,
    pub home_sash_hovered: bool,
    pub home_sash_dragging: bool,
}

/// Computed sub-regions produced by [`render_workspace_home`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceHomeLayout {
    pub matrix_body: Rect,
    pub preview_area: Option<Rect>,
    pub tab_hits: [(Rect, MatrixAction); 4],
    pub quick_action_hits: Vec<(Rect, HomeQuickAction)>,
    pub preview_action_hits: Vec<(Rect, PreviewAction)>,
    pub hydra_rendered: bool,
    pub menubar_hits: Vec<(Rect, MenubarItem)>,
    pub sash_hits: Vec<(Rect, SashId)>,
    pub preview_layout: PreviewCardLayout,
}

/// Report whether [`render_home_studio`] renders the Hydra emblem for `area` and `mode`.
pub fn home_renders_hydra(area: Rect, mode: ComposerMode) -> bool {
    if area.width < 36 || area.height < 12 || mode == ComposerMode::LiveRadar {
        return false;
    }
    let welcome_w = area.width;
    let welcome_h = area.height.saturating_sub(4);
    let raw_inner_w = welcome_w.saturating_sub(2);
    let raw_inner_h = welcome_h.saturating_sub(2);
    let inner_w = if raw_inner_w >= 6 {
        raw_inner_w - 2
    } else {
        raw_inner_w
    };
    let inner_h = raw_inner_h;
    let mode_a = welcome_w >= 104
        && raw_inner_h >= 16
        && ((raw_inner_h >= 24 && raw_inner_w >= 116) || (raw_inner_h >= 16 && raw_inner_w >= 102));
    let mode_b = welcome_w >= 76 && inner_h >= 14 && inner_w.saturating_sub(36) >= 24;
    mode_a || mode_b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceBodyPlan {
    LiveRadar,
    SplitUpperLower {
        upper_band: Rect,
        rule_area: Rect,
        matrix_body: Rect,
        hydra_box: Option<Rect>,
    },
    CompactWithPreview {
        preview_box: Rect,
        matrix_body: Rect,
    },
    MatrixOnly {
        matrix_body: Rect,
    },
}

fn padded_card_inner(raw_inner: Rect) -> Rect {
    if raw_inner.width >= 6 {
        Rect {
            x: raw_inner.x + 1,
            width: raw_inner.width - 2,
            ..raw_inner
        }
    } else {
        raw_inner
    }
}

fn workspace_card_inner_area(area: Rect) -> Option<Rect> {
    if area.width < 36 || area.height < 12 {
        return None;
    }
    let body = if area.height >= 17 {
        let [_, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        rest
    } else {
        area
    };
    let [card_area, _, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(body);
    let raw_inner = Block::default().borders(Borders::ALL).inner(card_area);
    if raw_inner.width == 0 || raw_inner.height == 0 {
        return None;
    }
    Some(padded_card_inner(raw_inner))
}

pub(super) fn split_lower_band_for_hydra(lower_band: Rect) -> (Rect, Option<Rect>) {
    if lower_band.width >= 60 {
        let max_hydra_w = lower_band.width.saturating_sub(44).min(32);
        if let Some((hydra_w, _)) = hydra_target_dimensions(max_hydra_w, lower_band.height) {
            let [left_band, _gap, hydra_box] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(2),
                Constraint::Length(hydra_w),
            ])
            .areas(lower_band);
            return (left_band, Some(hydra_box));
        }
    }
    (lower_band, None)
}

pub(super) fn render_hydra_emblem_if_some(
    frame: &mut Frame<'_>,
    hydra_box: Option<Rect>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> bool {
    if let Some(hbox) = hydra_box
        && let Some((_, hydra_lines)) =
            hydra_emblem_mode_frame(hbox.width, hbox.height, emblem_step, emblem_mode, theme)
    {
        render_hydra_emblem_area(frame, hbox, hydra_lines);
        return true;
    }
    false
}

pub(super) fn render_horizontal_divider_line(frame: &mut Frame<'_>, rule_area: Rect, theme: Theme) {
    if rule_area.width == 0 || rule_area.height == 0 {
        return;
    }
    let rule_ch = if theme.ascii { "-" } else { "─" };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            rule_ch.repeat(usize::from(rule_area.width)),
            theme.style(theme.muted),
        ))),
        rule_area,
    );
}

fn plan_workspace_card_body(
    card_inner: Rect,
    composer_mode: ComposerMode,
    live_preview: bool,
    has_preview: bool,
    home_split_ratio: u16,
) -> WorkspaceBodyPlan {
    if composer_mode == ComposerMode::LiveRadar {
        return WorkspaceBodyPlan::LiveRadar;
    }
    if card_inner.height >= 15 {
        let [upper_band, rule_area, lower_band] = if live_preview {
            if home_split_ratio != 0 {
                let avail = card_inner.height.saturating_sub(1);
                let pct = home_split_ratio.clamp(25, 75);
                let min_upper = 5_u16.min(avail.saturating_sub(5));
                let max_upper = avail.saturating_sub(5).max(min_upper);
                let upper_h = (((u32::from(avail) * u32::from(pct) + 50) / 100) as u16)
                    .clamp(min_upper, max_upper);
                let lower_h = avail.saturating_sub(upper_h);
                Layout::vertical([
                    Constraint::Length(upper_h),
                    Constraint::Length(1),
                    Constraint::Length(lower_h),
                ])
                .areas(card_inner)
            } else {
                let lower_h = ((card_inner.height * 2) / 5).clamp(8, 16);
                Layout::vertical([
                    Constraint::Min(6),
                    Constraint::Length(1),
                    Constraint::Length(lower_h),
                ])
                .areas(card_inner)
            }
        } else {
            Layout::vertical([
                Constraint::Length(4),
                Constraint::Length(1),
                Constraint::Min(8),
            ])
            .areas(card_inner)
        };
        let (matrix_body, hydra_box) = split_lower_band_for_hydra(lower_band);
        return WorkspaceBodyPlan::SplitUpperLower {
            upper_band,
            rule_area,
            matrix_body,
            hydra_box,
        };
    }
    if live_preview && has_preview && card_inner.height >= 9 && card_inner.width >= 60 {
        let [preview_box, matrix_body] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(5)]).areas(card_inner);
        return WorkspaceBodyPlan::CompactWithPreview {
            preview_box,
            matrix_body,
        };
    }
    WorkspaceBodyPlan::MatrixOnly {
        matrix_body: card_inner,
    }
}

/// Report whether [`render_workspace_home`] renders the rotating Hydra emblem for `area` and
/// `composer_mode`.
pub fn workspace_home_renders_hydra(area: Rect, composer_mode: ComposerMode) -> bool {
    let Some(card_inner) = workspace_card_inner_area(area) else {
        return false;
    };
    matches!(
        plan_workspace_card_body(card_inner, composer_mode, false, false, 0),
        WorkspaceBodyPlan::SplitUpperLower {
            hydra_box: Some(_),
            ..
        }
    )
}

struct WorkspaceOuterSpec<'a> {
    details: HomeDetails<'a>,
    telemetry: HomeTelemetry<'a>,
    border_color: Color,
    mode_pill: Option<(&'a str, Color)>,
    menubar: DesktopMenubarState<'a>,
}

struct WorkspaceOuterAreas {
    card_inner: Option<Rect>,
    composer: Rect,
    footer: Rect,
    menubar_hits: Vec<(Rect, MenubarItem)>,
}

fn render_workspace_outer_frame(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: WorkspaceOuterSpec<'_>,
    theme: Theme,
) -> WorkspaceOuterAreas {
    let (menubar_hits, body_area) = if area.height >= 17 {
        let [bar_area, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let hits = render_desktop_menubar(frame, bar_area, spec.menubar, theme);
        (hits, rest)
    } else {
        (Vec::new(), area)
    };
    let [card_area, composer, footer] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(body_area);

    let max_title_width = usize::from(card_area.width.saturating_sub(3));
    let mut title_spans = build_home_card_title_prefix(
        card_area.width,
        max_title_width,
        spec.telemetry,
        true,
        theme,
    );
    if let Some((pill_label, pill_color)) = spec.mode_pill {
        append_pill_to_title(
            &mut title_spans,
            max_title_width,
            pill_label,
            pill_color,
            theme,
        );
    }
    append_workspace_card_meta(
        &mut title_spans,
        max_title_width,
        spec.details,
        spec.telemetry,
        theme,
    );
    if title_spans.iter().map(Span::width).sum::<usize>() < max_title_width {
        title_spans.push(Span::raw(" "));
    }

    let card_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(spec.border_color))
        .title(Line::from(title_spans));
    let raw_inner = card_block.inner(card_area);
    frame.render_widget(card_block, card_area);

    let card_inner =
        (raw_inner.width > 0 && raw_inner.height > 0).then(|| padded_card_inner(raw_inner));
    WorkspaceOuterAreas {
        card_inner,
        composer,
        footer,
        menubar_hits,
    }
}

type ActionTabsLayout = ([(Rect, MatrixAction); 4], Vec<(Rect, HomeQuickAction)>);
type MatrixWithTabsLayout = (
    Rect,
    [(Rect, MatrixAction); 4],
    Vec<(Rect, HomeQuickAction)>,
);

fn render_workspace_action_tabs(
    frame: &mut Frame<'_>,
    tabs_area: Rect,
    active_action: MatrixAction,
    inside_tmux: bool,
    theme: Theme,
) -> ActionTabsLayout {
    let mut tab_hits = [(Rect::default(), MatrixAction::Open); 4];
    let mut quick_hits = Vec::new();
    if tabs_area.width == 0 || tabs_area.height == 0 {
        return (tab_hits, quick_hits);
    }
    let scope = match active_action {
        MatrixAction::Rename(s) | MatrixAction::Kill(s) => s,
        _ => TargetScope::Window,
    };
    let zh = theme.language.is_zh() && !theme.ascii;
    let tabs = [
        (
            if zh {
                "[ ▸ 恢复 ]"
            } else {
                "[ ▸ Resume ]"
            },
            if zh { "[▸ 恢复]" } else { "[> Resume]" },
            MatrixAction::Open,
            matches!(
                active_action,
                MatrixAction::Open | MatrixAction::Window | MatrixAction::Inspect
            ),
        ),
        (
            if zh {
                "[ ✎ 改名 ]"
            } else {
                "[ ✎ Rename ]"
            },
            if zh { "[改名]" } else { "[Rename]" },
            MatrixAction::Rename(scope),
            matches!(active_action, MatrixAction::Rename(_)),
        ),
        (
            if zh { "[ ✕ 关闭 ]" } else { "[ ✕ Kill ]" },
            if zh { "[关闭]" } else { "[Kill]" },
            MatrixAction::Kill(scope),
            matches!(active_action, MatrixAction::Kill(_)),
        ),
        (
            if zh {
                "[ ◫ 分屏 ]"
            } else {
                "[ ◫ Split ]"
            },
            if zh { "[分屏]" } else { "[Split]" },
            MatrixAction::Split,
            matches!(active_action, MatrixAction::Split),
        ),
    ];

    let use_compact_labels = theme.ascii || tabs_area.width < 48;
    let mut spans = Vec::new();
    let mut x = tabs_area.x;
    let max_x = tabs_area.right();
    for (idx, (unicode_label, ascii_label, tab_action, is_active)) in tabs.into_iter().enumerate() {
        let label = if use_compact_labels {
            ascii_label
        } else {
            unicode_label
        };
        if idx > 0 {
            if x >= max_x {
                break;
            }
            spans.push(Span::raw(" "));
            x += 1;
        }
        let style = if is_active {
            theme
                .style(tab_action.border_color(theme))
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            theme.style(theme.muted)
        };
        let avail = usize::from(max_x.saturating_sub(x));
        if avail == 0 {
            break;
        }
        let shown = truncate_to_width(label, avail);
        let w = shown.width() as u16;
        if w == 0 {
            break;
        }
        tab_hits[idx] = (
            Rect {
                x,
                y: tabs_area.y,
                width: w,
                height: 1,
            },
            tab_action,
        );
        spans.push(Span::styled(shown, style));
        x += w;
    }
    let hint = if theme.ascii { "  Tab" } else { "  Tab ⇄" };
    if x + (hint.width() as u16) <= max_x {
        spans.extend(styled_key_hint_spans(hint, theme.style(theme.muted), theme));
        x = x.saturating_add(hint.width() as u16);
    }

    // Right-aligned zero-memorization quick pills (prioritizing `[ ⏏ Detach ]` when inside tmux).
    let rem = usize::from(max_x.saturating_sub(x));
    if rem >= 10 {
        let bg = theme.pill_bg();
        let detach_full = if theme.ascii {
            "[Detach]"
        } else if zh {
            "[ ⏏ 分离 ]"
        } else {
            "[ ⏏ Detach ]"
        };
        let detach_short = if theme.ascii {
            "[Det]"
        } else if zh {
            "[⏏ 分离]"
        } else {
            "[⏏ Det]"
        };
        let panes_lbl = if theme.ascii {
            "[Panes]"
        } else if zh {
            "[ ⊞ 窗格 ]"
        } else {
            "[ ⊞ Panes ]"
        };
        let layout_lbl = if theme.ascii {
            "[Layout]"
        } else if zh {
            "[ ▦ 布局 ]"
        } else {
            "[ ▦ Layout ]"
        };

        let mut chosen: Vec<(HomeQuickAction, &str, Color, bool)> = Vec::new();
        let mut used_right = 0_usize;
        let max_right = rem.saturating_sub(2);
        if inside_tmux {
            if detach_full.width() <= max_right {
                chosen.push((HomeQuickAction::Detach, detach_full, theme.warning, true));
                used_right += detach_full.width();
            } else if detach_short.width() <= max_right {
                chosen.push((HomeQuickAction::Detach, detach_short, theme.warning, true));
                used_right += detach_short.width();
            }
        }
        for (act, lbl) in [
            (HomeQuickAction::Layout, layout_lbl),
            (HomeQuickAction::Panes, panes_lbl),
        ] {
            let need = lbl.width() + usize::from(!chosen.is_empty());
            if used_right + need <= max_right {
                chosen.insert(0, (act, lbl, theme.secondary, false));
                used_right += need;
            }
        }

        if !chosen.is_empty() {
            let pad = rem.saturating_sub(used_right);
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
                x = x.saturating_add(u16::try_from(pad).unwrap_or(0));
            }
            for (idx, (act, lbl, color, bold)) in chosen.into_iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::raw(" "));
                    x = x.saturating_add(1);
                }
                let w = u16::try_from(lbl.width()).unwrap_or(0);
                spans.push(pill_span(lbl, color, bg, theme, bold));
                quick_hits.push((
                    Rect {
                        x,
                        y: tabs_area.y,
                        width: w,
                        height: 1,
                    },
                    act,
                ));
                x = x.saturating_add(w);
            }
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
    (tab_hits, quick_hits)
}

fn render_matrix_with_tabs(
    frame: &mut Frame<'_>,
    matrix_body: Rect,
    mode: ComposerMode,
    action: MatrixAction,
    inside_tmux: bool,
    theme: Theme,
    mut render_matrix: impl FnMut(&mut Frame<'_>, Rect),
) -> MatrixWithTabsLayout {
    if mode == ComposerMode::CommandJump && matrix_body.height >= 4 {
        let [tabs_area, inner_matrix_body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(matrix_body);
        let (tab_hits, quick_hits) =
            render_workspace_action_tabs(frame, tabs_area, action, inside_tmux, theme);
        render_matrix(frame, inner_matrix_body);
        (inner_matrix_body, tab_hits, quick_hits)
    } else {
        render_matrix(frame, matrix_body);
        (
            matrix_body,
            [(Rect::default(), MatrixAction::Open); 4],
            Vec::new(),
        )
    }
}

/// Render the unified Home workspace with an upper Window Preview / hint band,
/// lower Action Tab Bar + Session×Window Matrix + Hydra emblem, action composer, footer hints,
/// and upward `/` / `@` drawers.
pub fn render_workspace_home(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    spec: WorkspaceHomeSpec<'_>,
    theme: Theme,
    mut render_matrix: impl FnMut(&mut Frame<'_>, Rect),
) -> WorkspaceHomeLayout {
    if area.width == 0 || area.height == 0 {
        return WorkspaceHomeLayout::default();
    }
    let active_border = if spec.composer_mode == ComposerMode::CommandJump {
        spec.action.border_color(theme)
    } else {
        spec.composer_mode.border_color(theme)
    };

    if area.width < 36 || area.height < 12 {
        let header_h = u16::from(area.height >= 7);
        let [header_area, raw_matrix_body, composer, footer] = Layout::vertical([
            Constraint::Length(header_h),
            Constraint::Min(0),
            Constraint::Length(if area.height > 4 { 2 } else { 1 }),
            Constraint::Length(u16::from(area.height > 2)),
        ])
        .areas(area);
        if header_area.height > 0 {
            let max_title_width = usize::from(header_area.width);
            let mut title_spans = build_home_card_title_prefix(
                header_area.width,
                max_title_width,
                spec.telemetry,
                false,
                theme,
            );
            if let Some(pill_label) = spec.action.mode_pill_for(theme.language) {
                append_pill_to_title(
                    &mut title_spans,
                    max_title_width,
                    pill_label,
                    spec.action.border_color(theme),
                    theme,
                );
            }
            frame.render_widget(Paragraph::new(Line::from(title_spans)), header_area);
        }
        let (matrix_body, tab_hits, quick_action_hits) = render_matrix_with_tabs(
            frame,
            raw_matrix_body,
            spec.composer_mode,
            spec.action,
            spec.inside_tmux,
            theme,
            &mut render_matrix,
        );
        render_composer_line(
            frame,
            composer,
            input,
            spec.composer_mode,
            active_border,
            composer.width >= 44,
            theme,
        );
        render_workspace_footer(frame, footer, spec.details.status, theme);
        render_home_drawers(
            frame,
            composer,
            HomeDrawerData {
                popup: spec.popup,
                popup_selected: spec.popup_selected,
                mentions: spec.mentions,
                mention_selected: spec.mention_selected,
            },
            theme,
        );
        return WorkspaceHomeLayout {
            matrix_body,
            preview_area: None,
            tab_hits,
            quick_action_hits,
            preview_action_hits: Vec::new(),
            hydra_rendered: false,
            menubar_hits: Vec::new(),
            sash_hits: Vec::new(),
            preview_layout: PreviewCardLayout::default(),
        };
    }

    let menubar_state = DesktopMenubarState {
        inside_tmux: spec.menubar.inside_tmux || spec.inside_tmux,
        ..spec.menubar
    };
    let outer = render_workspace_outer_frame(
        frame,
        area,
        WorkspaceOuterSpec {
            details: spec.details,
            telemetry: spec.telemetry,
            border_color: active_border,
            mode_pill: spec
                .action
                .mode_pill_for(theme.language)
                .map(|label| (label, spec.action.border_color(theme))),
            menubar: menubar_state,
        },
        theme,
    );
    let menubar_hits = outer.menubar_hits;

    let layout = if let Some(card_inner) = outer.card_inner {
        match plan_workspace_card_body(
            card_inner,
            spec.composer_mode,
            spec.live_preview,
            spec.preview.is_some(),
            spec.home_split_ratio,
        ) {
            WorkspaceBodyPlan::LiveRadar => {
                let radar_preview = render_live_radar(
                    frame,
                    card_inner,
                    HomeDashboardState {
                        mode: spec.composer_mode,
                        sessions: spec.sessions,
                        radar_selected: spec.radar_selected,
                        preview: spec.preview,
                        ..HomeDashboardState::default()
                    },
                    theme,
                );
                let preview_action_hits = radar_preview.action_hits.clone();
                WorkspaceHomeLayout {
                    matrix_body: Rect::default(),
                    preview_area: if spec.live_preview {
                        radar_preview.area
                    } else {
                        None
                    },
                    tab_hits: [(Rect::default(), MatrixAction::Open); 4],
                    quick_action_hits: Vec::new(),
                    preview_action_hits,
                    hydra_rendered: false,
                    menubar_hits,
                    sash_hits: Vec::new(),
                    preview_layout: radar_preview,
                }
            }
            WorkspaceBodyPlan::SplitUpperLower {
                upper_band,
                rule_area,
                matrix_body: raw_matrix_body,
                hydra_box,
            } => {
                let preview_layout = if spec.live_preview {
                    render_workspace_preview_card(frame, upper_band, spec.preview, theme)
                } else {
                    render_workspace_action_hints(frame, upper_band, spec.action, theme);
                    PreviewCardLayout::default()
                };
                let mut sash_hits = Vec::new();
                if spec.live_preview {
                    let total_h = upper_band
                        .height
                        .saturating_add(raw_matrix_body.height)
                        .max(1);
                    let ratio_pct = if spec.home_split_ratio != 0 {
                        spec.home_split_ratio.clamp(25, 75)
                    } else {
                        ((u32::from(upper_band.height) * 100 + u32::from(total_h) / 2)
                            / u32::from(total_h)) as u16
                    };
                    render_sash_divider(
                        frame,
                        rule_area,
                        false,
                        ratio_pct,
                        spec.home_sash_hovered,
                        spec.home_sash_dragging,
                        theme,
                    );
                    sash_hits.push((rule_area, SashId::HomePreviewMatrix));
                } else {
                    render_horizontal_divider_line(frame, rule_area, theme);
                }
                let (matrix_body, tab_hits, quick_action_hits) = render_matrix_with_tabs(
                    frame,
                    raw_matrix_body,
                    spec.composer_mode,
                    spec.action,
                    spec.inside_tmux,
                    theme,
                    &mut render_matrix,
                );
                let hydra_rendered = render_hydra_emblem_if_some(
                    frame,
                    hydra_box,
                    spec.emblem_step,
                    spec.emblem_mode,
                    theme,
                );
                let preview_action_hits = preview_layout.action_hits.clone();
                WorkspaceHomeLayout {
                    matrix_body,
                    preview_area: spec.live_preview.then_some(upper_band),
                    tab_hits,
                    quick_action_hits,
                    preview_action_hits,
                    hydra_rendered,
                    menubar_hits,
                    sash_hits,
                    preview_layout,
                }
            }
            WorkspaceBodyPlan::CompactWithPreview {
                preview_box,
                matrix_body: raw_matrix_body,
            } => {
                let preview_layout =
                    render_workspace_preview_card(frame, preview_box, spec.preview, theme);
                let (matrix_body, tab_hits, quick_action_hits) = render_matrix_with_tabs(
                    frame,
                    raw_matrix_body,
                    spec.composer_mode,
                    spec.action,
                    spec.inside_tmux,
                    theme,
                    &mut render_matrix,
                );
                let preview_action_hits = preview_layout.action_hits.clone();
                WorkspaceHomeLayout {
                    matrix_body,
                    preview_area: Some(preview_box),
                    tab_hits,
                    quick_action_hits,
                    preview_action_hits,
                    hydra_rendered: false,
                    menubar_hits,
                    sash_hits: Vec::new(),
                    preview_layout,
                }
            }
            WorkspaceBodyPlan::MatrixOnly {
                matrix_body: raw_matrix_body,
            } => {
                let (matrix_body, tab_hits, quick_action_hits) = render_matrix_with_tabs(
                    frame,
                    raw_matrix_body,
                    spec.composer_mode,
                    spec.action,
                    spec.inside_tmux,
                    theme,
                    &mut render_matrix,
                );
                WorkspaceHomeLayout {
                    matrix_body,
                    preview_area: None,
                    tab_hits,
                    quick_action_hits,
                    preview_action_hits: Vec::new(),
                    hydra_rendered: false,
                    menubar_hits,
                    sash_hits: Vec::new(),
                    preview_layout: PreviewCardLayout::default(),
                }
            }
        }
    } else {
        WorkspaceHomeLayout {
            menubar_hits,
            ..WorkspaceHomeLayout::default()
        }
    };

    let zh = theme.language.is_zh() && !theme.ascii;
    let left_title = if spec.composer_mode == ComposerMode::CommandJump {
        spec.action.composer_title_for(theme.ascii, theme.language)
    } else {
        spec.composer_mode.label_for(theme.ascii, theme.language)
    };
    let composer_avail = usize::from(outer.composer.width);
    let right_hint = if outer.composer.width >= 58 {
        if spec.action != MatrixAction::Open {
            let full = if theme.ascii {
                " Tab Next | Esc Resume "
            } else if zh {
                " Tab 下一项 · Esc 恢复 "
            } else {
                " Tab Next · Esc Resume "
            };
            let compact = if zh { " Esc 恢复 " } else { " Esc Resume " };
            if left_title.width() + full.width() + 4 <= composer_avail {
                Some(full)
            } else if left_title.width() + compact.width() + 4 <= composer_avail {
                Some(compact)
            } else {
                None
            }
        } else {
            let full = if theme.ascii {
                " Tab Switch | / Commands | Shift+Tab Mode "
            } else if zh {
                " Tab 切换 · / 命令 · Shift+Tab 模式 "
            } else {
                " Tab Switch · / Commands · Shift+Tab Mode "
            };
            let compact = if theme.ascii {
                " Tab Switch | / Commands "
            } else if zh {
                " Tab 切换 · / 命令 "
            } else {
                " Tab Switch · / Commands "
            };
            let mini = if zh { " Tab 切换 " } else { " Tab Switch " };
            if left_title.width() + full.width() + 4 <= composer_avail {
                Some(full)
            } else if left_title.width() + compact.width() + 4 <= composer_avail {
                Some(compact)
            } else if left_title.width() + mini.width() + 4 <= composer_avail {
                Some(mini)
            } else {
                None
            }
        }
    } else {
        None
    };

    render_composer_card(
        frame,
        outer.composer,
        input,
        ComposerChrome {
            mode: spec.composer_mode,
            border_color: active_border,
            left_title,
            right_title: right_hint,
        },
        theme,
    );
    render_workspace_footer(frame, outer.footer, spec.details.status, theme);

    let drawer_anchor = Rect {
        x: outer.composer.x,
        y: outer.composer.y + 1,
        width: outer.composer.width,
        height: 1,
    };
    render_home_drawers(
        frame,
        drawer_anchor,
        HomeDrawerData {
            popup: spec.popup,
            popup_selected: spec.popup_selected,
            mentions: spec.mentions,
            mention_selected: spec.mention_selected,
        },
        theme,
    );

    layout
}

struct ComposerChrome<'a> {
    mode: ComposerMode,
    border_color: Color,
    left_title: &'a str,
    right_title: Option<&'a str>,
}

struct HomeDrawerData<'a> {
    popup: &'a [CommandItem<'a>],
    popup_selected: usize,
    mentions: &'a [MentionItem<'a>],
    mention_selected: usize,
}

fn render_bordered_composer_box(
    frame: &mut Frame<'_>,
    composer: Rect,
    border_color: Color,
    left_title: &str,
    right_title: Option<&str>,
    theme: Theme,
) -> Rect {
    let mut composer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(border_color))
        .title_top(Line::from(styled_key_hint_spans(
            &format!(" {left_title} "),
            theme.style(border_color).add_modifier(Modifier::BOLD),
            theme,
        )));
    if let Some(right) = right_title {
        composer_block = composer_block.title_top(
            styled_key_hint_line(right, theme.style(theme.muted), theme).right_aligned(),
        );
    }
    let input_area = composer_block.inner(composer);
    frame.render_widget(composer_block, composer);
    input_area
}

fn render_composer_card(
    frame: &mut Frame<'_>,
    composer: Rect,
    input: &TextInput,
    chrome: ComposerChrome<'_>,
    theme: Theme,
) {
    let input_area = render_bordered_composer_box(
        frame,
        composer,
        chrome.border_color,
        chrome.left_title,
        chrome.right_title,
        theme,
    );
    render_composer_line(
        frame,
        input_area,
        input,
        chrome.mode,
        chrome.border_color,
        input_area.width >= 40,
        theme,
    );
}

fn render_home_drawers(
    frame: &mut Frame<'_>,
    anchor: Rect,
    data: HomeDrawerData<'_>,
    theme: Theme,
) {
    if !data.popup.is_empty() {
        render_command_popup(frame, anchor, data.popup, data.popup_selected, theme);
    } else if !data.mentions.is_empty() {
        render_mention_popup(frame, anchor, data.mentions, data.mention_selected, theme);
    }
}

/// Result geometry and interaction metadata returned by [`render_workspace_subview`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceSubviewLayout {
    pub list_rect: Option<Rect>,
    pub preview_area: Option<Rect>,
    pub hydra_rendered: bool,
    pub confirm_scroll: u16,
    pub split_slot_hits: Vec<(Rect, usize)>,
    pub split_toolbar_hits: Vec<(Rect, SplitToolbarButton)>,
    pub preview_action_hits: Vec<(Rect, PreviewAction)>,
    pub controller_hits: Vec<(Rect, ControllerAction)>,
    pub menubar_hits: Vec<(Rect, MenubarItem)>,
    pub sash_hits: Vec<(Rect, SashId)>,
    pub expose_tile_hits: Vec<ExposeTileHit>,
    pub preview_layout: PreviewCardLayout,
}

/// Interactive view payload rendered inside the unified [`render_workspace_subview`] chrome.
#[derive(Debug, Clone, Default)]
pub enum SubviewContent<'a> {
    Config {
        path_label: &'a str,
        items: &'a [ConfigItemView<'a>],
        selected: usize,
        status_note: Option<&'a str>,
    },
    Theme {
        presets: &'a [ThemePreset],
        selected: usize,
        active: ThemePreset,
    },
    #[default]
    Help,
    Picker {
        title: &'a str,
        query: &'a TextInput,
        rows: &'a [RichPickerRow],
        selected: usize,
        empty: &'a str,
        preview: Option<PanePreview<'a>>,
    },
    TargetPicker {
        title: &'a str,
        action_banner: &'a str,
        query: &'a TextInput,
        rows: &'a [RichPickerRow],
        selected: usize,
        empty: &'a str,
        preview: Option<PanePreview<'a>>,
    },
    Split(SplitStudioView<'a>),
    LayoutStudio(LayoutStudioView<'a>),
    Expose(ExposeView<'a>),
    Clients(ClientsView<'a>),
    Buffers(BuffersView<'a>),
    Snapshots(SnapshotsView<'a>),
    Form {
        title: &'a str,
        label: &'a str,
        input: &'a TextInput,
    },
    DualForm {
        title: &'a str,
        first_label: &'a str,
        first_input: &'a TextInput,
        second_label: &'a str,
        second_input: &'a TextInput,
        active_field: usize,
    },
    Confirm {
        title: &'a str,
        target_summary: &'a str,
        message: &'a str,
        input: &'a TextInput,
        scroll: u16,
    },
    Actions {
        title: &'a str,
        target_label: &'a str,
        actions: &'a [&'a str],
        selected: usize,
    },
    Created {
        target_label: &'a str,
    },
    Tribute(TributeModal<'a>),
}

/// Specification for rendering any active non-Home subview inside unified JustMux workspace chrome.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSubviewSpec<'a> {
    pub details: HomeDetails<'a>,
    pub telemetry: HomeTelemetry<'a>,
    pub emblem_step: usize,
    pub emblem_mode: EmblemMode,
    pub footer_hint: &'a str,
    pub content: SubviewContent<'a>,
    pub menubar: DesktopMenubarState<'a>,
}

impl SubviewContent<'_> {
    fn chrome_badge_and_color(&self, theme: Theme) -> (&str, Color) {
        match self {
            Self::Config { .. } => (theme.tr("CONFIG", "设置"), theme.accent),
            Self::Theme { .. } => (theme.tr("THEME", "主题"), theme.accent),
            Self::Help => (theme.tr("HELP", "帮助"), theme.accent),
            Self::Picker { title, .. } => {
                let badge = if title.eq_ignore_ascii_case("Panes") || *title == "窗格列表" {
                    theme.tr("PANES", "窗格")
                } else {
                    theme.tr("PICKER", "选择")
                };
                (badge, theme.accent)
            }
            Self::TargetPicker { .. } => (theme.tr("TARGET", "选择目标"), theme.warning),
            Self::Split { .. } => (theme.tr("SPLIT", "分屏"), theme.secondary),
            Self::LayoutStudio(_) => (theme.tr("LAYOUT", "布局"), theme.secondary),
            Self::Expose(_) => (theme.tr("EXPOSE", "全景墙"), theme.accent),
            Self::Clients(_) => (theme.tr("CLIENTS", "客户端"), theme.accent),
            Self::Buffers(_) => (theme.tr("BUFFERS", "缓冲区"), theme.secondary),
            Self::Snapshots(_) => (theme.tr("SNAPSHOTS", "快照"), theme.accent),
            Self::Form { title, .. } | Self::DualForm { title, .. } => {
                if *title == "Rename" || *title == "重命名" {
                    (theme.tr("RENAME", "重命名"), theme.warning)
                } else if *title == "New window" || *title == "新建窗口" {
                    (theme.tr("NEW WINDOW", "新建窗口"), theme.success)
                } else if *title == "New session" || *title == "新建会话" {
                    (theme.tr("NEW SESSION", "新建会话"), theme.accent)
                } else {
                    (theme.tr("INPUT", "输入"), theme.accent)
                }
            }
            Self::Confirm { .. } => (theme.tr("CONFIRM KILL", "确认关闭"), theme.error),
            Self::Actions { .. } => (theme.tr("ACTIONS", "操作"), theme.secondary),
            Self::Created { .. } => (theme.tr("CREATED", "已创建"), theme.success),
            Self::Tribute(modal) => (modal.badge.trim(), theme.accent),
        }
    }

    fn default_shortcut_hint_for(&self, theme: Theme) -> &str {
        let zh = theme.language.is_zh() && !theme.ascii;
        match self {
            Self::Config { .. } => {
                if zh {
                    "↑↓ 选择 · ←→ / Enter 修改 · s 保存到 config.toml · Esc 关闭"
                } else {
                    "↑↓ Select · ←→/Enter Change · s Save to config.toml · Esc Close"
                }
            }
            Self::Theme { .. } => {
                if zh {
                    "↑↓ 实时预览 · Enter 应用 · s 保存到 config.toml · Esc 还原"
                } else {
                    "↑↓ Live Preview · Enter Apply · s Save to config.toml · Esc Revert"
                }
            }
            Self::Help => {
                if zh {
                    "PgUp / PgDn 滚动预览 · Tab 切换操作 · Esc 返回"
                } else {
                    "PgUp / PgDn Scroll Preview · Tab Switch Actions · Esc Back"
                }
            }
            Self::Picker { .. } => {
                if zh {
                    "↑↓ 选择 · Enter 操作 · PgUp / PgDn 滚动预览 · Esc 返回"
                } else {
                    "↑↓ Select · Enter Actions · PgUp/PgDn Scroll Preview · Esc Back"
                }
            }
            Self::TargetPicker { .. } => {
                if zh {
                    "输入过滤 · ↑↓ 选择目标 · Enter 确认执行 · Esc 取消"
                } else {
                    "Type to filter · ↑↓ Select Target · Enter Confirm · Esc Cancel"
                }
            }
            Self::Split(..) => {
                if zh {
                    "1-4/Tab 选窗格 · h/v 分屏 · t 翻转 · x 移除 · r 重置 · ↑↓ 选窗口 · Space 交换 · Enter 进入 · Esc 返回"
                } else {
                    "1-4/Tab Pane · h/v Split · t Flip · x Del · r Reset · ↑↓ Window · Space Swap · Enter Launch · Esc Back"
                }
            }
            Self::LayoutStudio(_) => {
                if zh {
                    "1-5 应用布局 · ←→ ↑↓ 调节尺寸 · z 放大 · s 同步输入 · Esc 返回"
                } else {
                    "1-5 Preset · ←→ ↑↓ Resize · z Zoom · s Sync Panes · Esc Back"
                }
            }
            Self::Expose(_) => {
                if zh {
                    "↑↓←→ 选择窗口 · Enter 附着 · i 直控输入 · n/p 翻页 · 拖拽边缘分屏 · Esc 返回"
                } else {
                    "↑↓←→ Select · Enter Attach · i Live Input · n/p Page · Drag Edge to Dock · Esc Back"
                }
            }
            Self::Clients(_) => {
                if zh {
                    "↑↓ 选择客户端 · d 踢出其他端恢复全屏 · x 分离选中 · K 关闭服务 · Esc 返回"
                } else {
                    "↑↓ Select · d Detach Others · x Detach Selected · K Kill Server · Esc Back"
                }
            }
            Self::Buffers(_) => {
                if zh {
                    "↑↓ 选择缓冲区 · Enter 粘贴 · y 复制 · n 新建 · x 删除 · PgUp/PgDn 预览 · Esc 返回"
                } else {
                    "↑↓ Select · Enter Paste · y Copy · n New · x Delete · PgUp/PgDn Preview · Esc Back"
                }
            }
            Self::Snapshots(_) => {
                if zh {
                    "↑↓ 选择快照 · n 保存当前工作区 · Enter 恢复 · x 删除 · Esc 返回"
                } else {
                    "↑↓ Select · n Save Workspace · Enter Restore · x Delete · Esc Back"
                }
            }
            Self::Form { .. } => {
                if zh {
                    "输入内容 · Enter 提交 · Esc 返回"
                } else {
                    "Type value · Enter Submit · Esc Back"
                }
            }
            Self::DualForm { .. } => {
                if zh {
                    "Tab 切换名称/目录 · Enter 一步创建 · Esc 返回"
                } else {
                    "Tab Switch Name/Directory · Enter Submit · Esc Back"
                }
            }
            Self::Confirm { .. } => {
                if zh {
                    "↑↓ 滚动详情 · 输入 yes 后按 Enter 关闭 · Esc 返回"
                } else {
                    "↑↓ Review · Type yes + Enter to kill · Esc Back"
                }
            }
            Self::Actions { .. } => {
                if zh {
                    "↑↓ 选择操作 · Enter 执行 · Esc 返回"
                } else {
                    "↑↓ Choose Action · Enter Execute · Esc Back"
                }
            }
            Self::Created { .. } => {
                if zh {
                    "Enter 进入原生 tmux · Esc 返回继续管理"
                } else {
                    "Enter to enter native tmux · Esc to continue managing"
                }
            }
            Self::Tribute(modal) => modal.footer_hint,
        }
    }
}

/// Render an active JustMux subview wrapped inside the unified workspace chrome on standard
/// terminals (`area.width >= 36 && area.height >= 12`), falling back gracefully on tiny terminals.
pub fn render_workspace_subview(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: &WorkspaceSubviewSpec<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width == 0 || area.height == 0 {
        return WorkspaceSubviewLayout::default();
    }

    if area.width < 36 || area.height < 12 {
        return render_compact_subview_fallback(frame, area, spec, theme);
    }

    let (pill_label, border_color) = spec.content.chrome_badge_and_color(theme);
    let outer = render_workspace_outer_frame(
        frame,
        area,
        WorkspaceOuterSpec {
            details: spec.details,
            telemetry: spec.telemetry,
            border_color,
            mode_pill: Some((pill_label, border_color)),
            menubar: spec.menubar,
        },
        theme,
    );

    let mut layout = if let Some(card_inner) = outer.card_inner {
        render_subview_card_body(frame, card_inner, spec, border_color, theme)
    } else {
        WorkspaceSubviewLayout::default()
    };
    layout.menubar_hits = outer.menubar_hits;

    render_subview_bottom_composer(
        frame,
        outer.composer,
        outer.footer,
        spec,
        border_color,
        theme,
    );
    layout
}

fn render_compact_subview_fallback(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: &WorkspaceSubviewSpec<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let mut layout = WorkspaceSubviewLayout::default();
    match &spec.content {
        SubviewContent::Config {
            path_label,
            items,
            selected,
            status_note,
        } => render_config_modal(
            frame,
            area,
            path_label,
            items,
            *selected,
            *status_note,
            theme,
        ),
        SubviewContent::Theme {
            presets,
            selected,
            active,
        } => render_theme_picker(frame, area, presets, *selected, *active, theme),
        SubviewContent::Help => render_help(frame, area, theme),
        SubviewContent::Picker {
            title,
            query,
            rows,
            selected,
            empty,
            preview,
        } => render_rich_picker_with_preview(
            frame,
            area,
            title,
            query,
            rows,
            *selected,
            empty,
            spec.footer_hint,
            *preview,
            theme,
        ),
        SubviewContent::TargetPicker {
            title,
            action_banner,
            query,
            rows,
            selected,
            empty,
            preview,
        } => {
            layout = render_compact_target_picker(
                frame,
                area,
                title,
                action_banner,
                query,
                rows,
                *selected,
                empty,
                spec.footer_hint,
                *preview,
                theme,
            );
        }
        SubviewContent::Split(studio) => {
            layout = render_compact_split_studio(frame, area, studio, theme)
        }
        SubviewContent::LayoutStudio(view) => {
            layout = render_compact_layout_studio(frame, area, view, theme);
        }
        SubviewContent::Expose(view) => {
            layout = render_compact_expose(frame, area, view, theme);
        }
        SubviewContent::Clients(view) => {
            layout = render_compact_clients(frame, area, view, theme);
        }
        SubviewContent::Buffers(view) => {
            layout = render_compact_buffers(frame, area, view, theme);
        }
        SubviewContent::Snapshots(view) => {
            layout = render_compact_snapshots(frame, area, view, theme);
        }
        SubviewContent::Form {
            title,
            label,
            input,
        } => render_form(frame, area, title, label, input, spec.footer_hint, theme),
        SubviewContent::DualForm {
            title,
            first_label,
            first_input,
            second_label,
            second_input,
            active_field,
        } => render_compact_dual_form(
            frame,
            area,
            title,
            first_label,
            first_input,
            second_label,
            second_input,
            *active_field,
            theme,
        ),
        SubviewContent::Confirm {
            title,
            target_summary,
            message,
            input,
            scroll,
        } => {
            layout.confirm_scroll = render_confirm_card(
                frame,
                area,
                title,
                target_summary,
                message,
                input,
                spec.footer_hint,
                theme,
                *scroll,
            );
        }
        SubviewContent::Actions {
            title,
            target_label,
            actions,
            selected,
        } => {
            layout.list_rect = render_actions_list_panel(
                frame,
                area,
                title,
                target_label,
                actions,
                *selected,
                theme,
            );
        }
        SubviewContent::Created { target_label } => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "Created successfully",
                        theme.style(theme.success).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(single_line(target_label)),
                    styled_key_hint_line(
                        "Enter to enter native tmux · Esc to continue managing",
                        theme.style(theme.text),
                        theme,
                    ),
                ]),
                area,
            );
        }
        SubviewContent::Tribute(modal) => render_tribute_modal(frame, area, modal.clone(), theme),
    }
    layout
}

pub(super) fn split_subview_bands(
    card_inner: Rect,
    upper_min: u16,
    balanced: bool,
) -> (Rect, Rect, Rect) {
    let lower_h = if card_inner.height >= 20 {
        (card_inner.height / 2).clamp(10, 16)
    } else if balanced {
        card_inner.height.saturating_sub(upper_min + 1).clamp(8, 12)
    } else {
        (card_inner.height / 2).clamp(8, 14)
    };
    let [upper_band, rule_area, lower_band] = Layout::vertical([
        Constraint::Min(upper_min),
        Constraint::Length(1),
        Constraint::Length(lower_h),
    ])
    .areas(card_inner);
    (upper_band, rule_area, lower_band)
}

fn render_subview_card_body(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    spec: &WorkspaceSubviewSpec<'_>,
    border_color: Color,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    match &spec.content {
        SubviewContent::Config {
            path_label,
            items,
            selected,
            status_note,
        } => render_subview_config(
            frame,
            card_inner,
            path_label,
            items,
            *selected,
            *status_note,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Theme {
            presets,
            selected,
            active,
        } => render_subview_theme(
            frame,
            card_inner,
            presets,
            *selected,
            *active,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Help => {
            render_subview_help(frame, card_inner, spec.emblem_step, spec.emblem_mode, theme)
        }
        SubviewContent::Picker {
            title,
            rows,
            selected,
            empty,
            preview,
            ..
        } => render_subview_picker(
            frame,
            card_inner,
            title,
            rows,
            *selected,
            empty,
            *preview,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::TargetPicker {
            title,
            action_banner,
            rows,
            selected,
            empty,
            preview,
            ..
        } => render_subview_target_picker(
            frame,
            card_inner,
            title,
            action_banner,
            rows,
            *selected,
            empty,
            *preview,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Split(studio) => render_subview_split(
            frame,
            card_inner,
            studio,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::LayoutStudio(view) => render_subview_layout_studio(
            frame,
            card_inner,
            view,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Expose(view) => render_subview_expose(
            frame,
            card_inner,
            view,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Clients(view) => render_subview_clients(
            frame,
            card_inner,
            view,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Buffers(view) => render_subview_buffers(frame, card_inner, view, theme),
        SubviewContent::Snapshots(view) => render_subview_snapshots(frame, card_inner, view, theme),
        SubviewContent::Form {
            title,
            label,
            input,
        } => render_subview_form(
            frame,
            card_inner,
            title,
            label,
            input,
            border_color,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::DualForm {
            title,
            first_label,
            first_input,
            second_label,
            second_input,
            active_field,
        } => render_subview_dual_form(
            frame,
            card_inner,
            title,
            first_label,
            first_input,
            second_label,
            second_input,
            *active_field,
            border_color,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Confirm {
            title: _,
            target_summary,
            message,
            scroll,
            ..
        } => render_subview_confirm(
            frame,
            card_inner,
            target_summary,
            message,
            *scroll,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Actions {
            title,
            target_label,
            actions,
            selected,
        } => render_subview_actions(
            frame,
            card_inner,
            title,
            target_label,
            actions,
            *selected,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Created { target_label } => render_subview_created(
            frame,
            card_inner,
            target_label,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
        SubviewContent::Tribute(modal) => render_subview_tribute(
            frame,
            card_inner,
            modal,
            spec.emblem_step,
            spec.emblem_mode,
            theme,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_subview_config(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    path_label: &str,
    items: &[ConfigItemView<'_>],
    selected: usize,
    status_note: Option<&str>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let render_header = |frame: &mut Frame<'_>, area: Rect| {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let muted = theme.style(theme.muted);
        let mut lines = vec![Line::from(Span::styled(
            theme.adapt(&format!(
                "⚙ {}",
                theme.tr("JustMux Configuration", "JustMux 设置与参数")
            )),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        ))];
        let path_prefix = theme.tr("Config: ", "配置: ");
        let path_marker = if theme.ascii { "..." } else { "…" };
        let trimmed = directory_tail(
            &single_line(path_label),
            usize::from(area.width).saturating_sub(path_prefix.width()),
            path_marker,
        );
        lines.push(Line::from(vec![
            Span::styled(path_prefix, muted),
            Span::styled(trimmed, theme.style(theme.secondary)),
        ]));
        if area.height >= 4
            && let Some(item) = items.get(selected.min(items.len().saturating_sub(1)))
            && !item.description.is_empty()
        {
            lines.push(Line::from(vec![
                Span::styled(format!("{}: ", item.key), theme.style(theme.accent)),
                Span::styled(
                    truncate_to_width(
                        &theme.adapt(item.description),
                        usize::from(area.width).saturating_sub(item.key.width() + 2),
                    ),
                    muted,
                ),
            ]));
        }
        if let Some(note) = status_note
            && usize::from(area.height) > lines.len()
        {
            lines.push(Line::from(Span::styled(
                truncate_to_width(&theme.adapt(note), usize::from(area.width)),
                status_style(note, theme),
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    };

    let render_list = |frame: &mut Frame<'_>, area: Rect| {
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new(theme.tr("No settings available", "暂无可用配置"))
                    .style(theme.style(theme.muted)),
                area,
            );
            return;
        }
        let clamped = selected.min(items.len() - 1);
        let list_items = build_config_list_items(items, clamped, area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(list_items), area, &mut state);
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 4, true);
        if upper_band.height >= 6 {
            let active_item = items.get(selected.min(items.len().saturating_sub(1)));
            render_config_inspector_card(
                frame,
                upper_band,
                path_label,
                active_item,
                status_note,
                emblem_step,
                theme,
            );
        } else {
            render_header(frame, upper_band);
        }
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        render_list(frame, left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            list_rect: Some(left_band),
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let [header_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(card_inner);
    render_header(frame, header_area);
    render_list(frame, list_area);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) struct ConfigGuideSpec {
    pub(super) title: &'static str,
    pub(super) summary: String,
    pub(super) traits: [String; 2],
    pub(super) keys_hint: &'static str,
    pub(super) art: Vec<Line<'static>>,
    pub(super) compact_art: Vec<Line<'static>>,
}

fn render_config_inspector_card(
    frame: &mut Frame<'_>,
    area: Rect,
    path_label: &str,
    active_item: Option<&ConfigItemView<'_>>,
    status_note: Option<&str>,
    emblem_step: usize,
    theme: Theme,
) {
    let key_label = active_item.map_or("config", |item| item.key);
    let val_label = active_item.map_or("", |item| item.value.as_str());
    let description = active_item.map_or("", |item| item.description);

    let heading_label = theme.tr("JustMux Configuration", "JustMux 设置与参数");
    let title_text = theme.adapt(&format!(" ⚙ {heading_label} · {key_label} "));
    let mut left_spans = vec![Span::styled(
        title_text,
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if !val_label.is_empty() {
        let bg = theme.pill_bg();
        let pill = pill_span(
            format!(" {} ", single_line(val_label)),
            theme.secondary,
            bg,
            theme,
            true,
        );
        let used: usize = left_spans.iter().map(Span::width).sum();
        if used + pill.width() + 3 <= usize::from(area.width) {
            left_spans.push(pill);
            left_spans.push(Span::raw(" "));
        }
    }

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.secondary))
        .title(Line::from(left_spans.clone()));

    let clean_path = single_line(path_label);
    let left_used: usize = left_spans.iter().map(Span::width).sum();
    let right_budget = usize::from(area.width).saturating_sub(left_used + 4);
    if right_budget >= 12 && !clean_path.is_empty() {
        let path_prefix = theme.tr(" Config: ", " 配置文件: ");
        let path_marker = if theme.ascii { "..." } else { "…" };
        let trimmed = directory_tail(
            &clean_path,
            right_budget.saturating_sub(path_prefix.width() + 1),
            path_marker,
        );
        block = block.title_top(
            Line::from(vec![
                Span::styled(path_prefix, theme.style(theme.muted)),
                Span::styled(format!("{trimmed} "), theme.style(theme.secondary)),
            ])
            .right_aligned(),
        );
    }

    let raw_inner = block.inner(area);
    frame.render_widget(block, area);
    if raw_inner.width == 0 || raw_inner.height == 0 {
        return;
    }

    let inner = if raw_inner.width >= 8 {
        Rect {
            x: raw_inner.x + 1,
            width: raw_inner.width - 2,
            ..raw_inner
        }
    } else {
        raw_inner
    };

    let show_status = status_note.is_some() && inner.height >= 2;
    let [status_area, content_area] = Layout::vertical([
        Constraint::Length(u16::from(show_status)),
        Constraint::Min(0),
    ])
    .areas(inner);

    if let Some(note) = status_note
        && status_area.height > 0
    {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(&theme.adapt(note), usize::from(status_area.width)),
                status_style(note, theme),
            ))),
            status_area,
        );
    }

    if content_area.width == 0 || content_area.height == 0 {
        return;
    }

    if inner.width >= 72 {
        let left_w = ((content_area.width.saturating_sub(3)) / 2).clamp(32, 62);
        let [doc_area, div_area, art_area] = Layout::horizontal([
            Constraint::Length(left_w),
            Constraint::Length(3),
            Constraint::Fill(1),
        ])
        .areas(content_area);
        let guide = build_config_guide(
            key_label,
            val_label,
            description,
            emblem_step,
            art_area.width,
            art_area.height,
            theme,
        );
        let doc_lines =
            render_config_doc_lines(&guide, val_label, doc_area.width, doc_area.height, theme);
        frame.render_widget(Paragraph::new(doc_lines), doc_area);

        let div_glyph = if theme.ascii { "|" } else { "│" };
        let div_style = theme.style(theme.muted);
        let div_lines: Vec<Line<'static>> = (0..div_area.height)
            .map(|_| Line::from(vec![Span::raw(" "), Span::styled(div_glyph, div_style)]))
            .collect();
        frame.render_widget(Paragraph::new(div_lines), div_area);

        let (top_pad, padded_art) = center_art_block(guide.art, art_area.width, art_area.height);
        let target_art = Rect {
            y: art_area.y.saturating_add(top_pad),
            height: art_area.height.saturating_sub(top_pad),
            ..art_area
        };
        frame.render_widget(Paragraph::new(padded_art), target_art);
    } else if content_area.height >= 7 {
        let guide = build_config_guide(
            key_label,
            val_label,
            description,
            emblem_step,
            content_area.width,
            3,
            theme,
        );
        if guide.compact_art.is_empty() {
            let doc_lines = render_config_doc_lines(
                &guide,
                val_label,
                content_area.width,
                content_area.height,
                theme,
            );
            frame.render_widget(Paragraph::new(doc_lines), content_area);
        } else {
            let art_h = (guide.compact_art.len() as u16).min(3);
            let [doc_area, art_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(art_h)])
                    .areas(content_area);
            let doc_lines =
                render_config_doc_lines(&guide, val_label, doc_area.width, doc_area.height, theme);
            frame.render_widget(Paragraph::new(doc_lines), doc_area);
            frame.render_widget(Paragraph::new(guide.compact_art), art_area);
        }
    } else {
        let guide = build_config_guide(
            key_label,
            val_label,
            description,
            emblem_step,
            content_area.width,
            3,
            theme,
        );
        let doc_lines = render_config_doc_lines(
            &guide,
            val_label,
            content_area.width,
            content_area.height,
            theme,
        );
        frame.render_widget(Paragraph::new(doc_lines), content_area);
    }
}

fn center_art_block(
    lines: Vec<Line<'static>>,
    avail_w: u16,
    avail_h: u16,
) -> (u16, Vec<Line<'static>>) {
    let block_w = lines.iter().map(Line::width).max().unwrap_or(0);
    let left_pad = usize::from(avail_w).saturating_sub(block_w) / 2;
    let top_pad = usize::from(avail_h).saturating_sub(lines.len()) / 2;
    let padded_lines = lines
        .into_iter()
        .map(|mut line| {
            if left_pad > 0 {
                line.spans.insert(0, Span::raw(" ".repeat(left_pad)));
            }
            line
        })
        .collect();
    (u16::try_from(top_pad).unwrap_or(0), padded_lines)
}

fn render_config_doc_lines(
    guide: &ConfigGuideSpec,
    val_label: &str,
    width: u16,
    height: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let w = usize::from(width);
    let h = usize::from(height);
    let accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let text_style = theme.style(theme.text);
    let muted = theme.style(theme.muted);
    let trait_prefix = theme.adapt("▸ ");

    let mut lines = Vec::with_capacity(h);
    let mut head_spans = vec![Span::styled(
        truncate_to_width(&theme.adapt(guide.title), w),
        accent,
    )];
    if !val_label.is_empty() {
        let val_badge = format!(" [{}]", single_line(val_label));
        if theme.adapt(guide.title).width() + val_badge.width() <= w {
            head_spans.push(Span::styled(
                val_badge,
                theme.style(theme.secondary).add_modifier(Modifier::BOLD),
            ));
        }
    }
    lines.push(Line::from(head_spans));

    if h >= 7 {
        let rule_ch = if theme.ascii { "-" } else { "─" };
        lines.push(Line::from(Span::styled(rule_ch.repeat(w.min(48)), muted)));
    }

    let traits_count = guide.traits.iter().filter(|t| !t.is_empty()).count();
    let max_desc_lines = if h >= 9 {
        3
    } else if h >= 6 {
        2
    } else {
        1
    };
    let footer_reserved = traits_count + 1;
    for wrapped in wrap_description(&theme.adapt(&guide.summary), width)
        .into_iter()
        .take(max_desc_lines)
    {
        if lines.len() + footer_reserved >= h {
            break;
        }
        lines.push(styled_key_hint_line(&wrapped, text_style, theme));
    }

    if h >= 8 && lines.len() + traits_count + 2 <= h {
        lines.push(Line::default());
    }

    for tr in &guide.traits {
        if lines.len() + 1 >= h {
            break;
        }
        if tr.is_empty() {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(trait_prefix.clone(), theme.style(theme.secondary)),
            Span::styled(
                truncate_to_width(&theme.adapt(tr), w.saturating_sub(trait_prefix.width())),
                muted,
            ),
        ]));
    }

    while lines.len() + 1 < h.min(9) {
        lines.push(Line::default());
    }

    if lines.len() < h {
        lines.push(styled_key_hint_line(
            &truncate_to_width(&theme.adapt(guide.keys_hint), w),
            muted,
            theme,
        ));
    }

    lines
}

fn build_config_guide(
    key: &str,
    value: &str,
    description: &str,
    emblem_step: usize,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> ConfigGuideSpec {
    let zh = theme.language.is_zh() && !theme.ascii;
    let fallback_summary = |default_text: &str| {
        if description.is_empty() {
            default_text.to_owned()
        } else {
            description.to_owned()
        }
    };

    match key {
        "screen_mode" => {
            let is_inline = value.trim().eq_ignore_ascii_case("inline");
            let art = config_art_screen_mode(is_inline, art_w, art_h, theme);
            let compact = config_art_screen_mode(is_inline, 38, 5, theme);
            let compact_art = vec![compact[1].clone(), compact[3].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "视口呈现模式"
                } else {
                    "Viewport Presentation"
                },
                summary: fallback_summary(if zh {
                    "全屏独占终端备用画布；内嵌固定在命令行下方并保留 Shell 输出历史"
                } else {
                    "Fullscreen owns the alt-screen; Inline anchors above your shell scrollback"
                }),
                traits: if zh {
                    [
                        "fullscreen: 备用屏幕，支持鼠标点击与滚轮".into(),
                        "inline: 保留终端历史记录（重启生效）".into(),
                    ]
                } else {
                    [
                        "fullscreen: Alt-screen, mouse click & wheel scroll".into(),
                        "inline: Keeps shell scrollback (restart required)".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 切换 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space switch · s Save config · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "theme" => {
            let preset = ThemePreset::from_name(value).unwrap_or(theme.preset);
            let art = config_art_theme(preset, art_w, art_h, theme);
            let compact = config_art_theme(preset, 38, 5, theme);
            let compact_art = vec![compact[0].clone(), compact[1].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "真彩配色与角色徽标"
                } else {
                    "TrueColor Palette & Mascot"
                },
                summary: fallback_summary(if zh {
                    "即时切换 JustMux 全局主题渐变色板、状态徽标与顶部图标样式"
                } else {
                    "Applies instant accent ramps, badges, and studio artwork across JustMux"
                }),
                traits: if zh {
                    [
                        format!("当前预览: {} — {}", preset.name(), preset.origin()),
                        "可选: aurora/codex/claude/matrix/classic".into(),
                    ]
                } else {
                    [
                        format!("Preview: {} — {}", preset.name(), preset.origin()),
                        "Presets: aurora · codex · claude · matrix · classic".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 轮换 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space cycle preset · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "composer_mode" => {
            let art = config_art_composer_mode(value, art_w, art_h, theme);
            let compact = config_art_composer_mode(value, 38, 5, theme);
            let compact_art = vec![compact[0].clone(), compact[2].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "主页启动交互模式"
                } else {
                    "Startup Home Composer Lens"
                },
                summary: fallback_summary(if zh {
                    "选择启动时默认激活的主页工作区模式（主页按 Shift+Tab 可实时切换）"
                } else {
                    "Chooses which Home workflow wakes on launch (Shift+Tab switches live)"
                }),
                traits: if zh {
                    [
                        "command: /命令与@跳转 · radar: 实时会话树".into(),
                        "launch: 输入目录一键创建并附着 tmux 会话".into(),
                    ]
                } else {
                    [
                        "command: /cmd & @jump · radar: Live session tree".into(),
                        "launch: Instant directory -> tmux session spawn".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 切换模式 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space cycle mode · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "live_preview" => {
            let enabled = !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            );
            let art = config_art_live_preview(enabled, art_w, art_h, theme);
            let compact = config_art_live_preview(enabled, 38, 5, theme);
            let compact_art = vec![compact[1].clone(), compact[2].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "窗格终端实时预览"
                } else {
                    "Live Terminal Capture Preview"
                },
                summary: fallback_summary(if zh {
                    "带 ANSI 真彩色流式捕获选中窗格终端输出，支持 PgUp/PgDn 回看"
                } else {
                    "Streams live tmux pane output with ANSI color & PgUp/PgDn scrollback"
                }),
                traits: if zh {
                    [
                        "true: 在主页与 /panes 上方实时展示终端内容".into(),
                        "false: 轻量提示卡片，零额外捕获性能开销".into(),
                    ]
                } else {
                    [
                        "true: Live upper preview card on Home & /panes".into(),
                        "false: Lightweight hints card, zero capture cost".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 开关 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space toggle · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "mouse" => {
            let enabled = !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            );
            let art = config_art_mouse(enabled, art_w, art_h, theme);
            let compact = config_art_mouse(enabled, 38, 5, theme);
            let compact_art = vec![compact[0].clone(), compact[1].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "鼠标点击与滚轮捕获"
                } else {
                    "Mouse Capture & Wheel Scroll"
                },
                summary: fallback_summary(if zh {
                    "在全屏模式下启用标签栏点击、矩阵选择与窗格预览滚轮翻页"
                } else {
                    "Enables tab clicks, matrix selection, and preview wheel scrolling"
                }),
                traits: if zh {
                    [
                        "true: 支持点击标签/列表及滚轮查看终端历史".into(),
                        "false: 纯键盘交互，保留终端原生文本划选".into(),
                    ]
                } else {
                    [
                        "true: Clickable tabs, matrix & preview wheel".into(),
                        "false: Keyboard-only; keeps native text select".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 开关(重启生效) · s 保存"
                } else {
                    "←→ / Space toggle (restart applies) · s Save"
                },
                art,
                compact_art,
            }
        }
        "emblem_mode" | "emblem_spin" => {
            let mode = EmblemMode::from_name(value).unwrap_or(EmblemMode::Spin);
            let art = config_art_emblem_mode(mode, emblem_step, art_w, art_h, theme);
            let compact_art = vec![config_art_emblem_compact_row(mode, theme)];
            ConfigGuideSpec {
                title: if zh {
                    "中心点阵徽标动画"
                } else {
                    "Hydra Center Medallion Mode"
                },
                summary: fallback_summary(if zh {
                    "控制九头蛇中心点阵内核静态、正转或反相旋转，外侧双环始终保持稳固"
                } else {
                    "Rotates or inverts the inner Hydra core while the outer double ring holds"
                }),
                traits: if zh {
                    [
                        "static (0°) · spin (5°) · inverted-spin".into(),
                        "全部 4 种徽标尺寸均稳固渲染外侧双护盾环".into(),
                    ]
                } else {
                    [
                        if theme.ascii {
                            "static 0deg | spin 5deg | inverted-spin (hollow)".into()
                        } else {
                            "static (0°) · spin (5°) · inverted-spin (hollow)".into()
                        },
                        "Double outer ring holds steady on all 4 scales".into(),
                    ]
                },
                keys_hint: if zh {
                    match mode {
                        EmblemMode::Static => "←→ / Space 切换 3 种风格 (1/3) · s 保存",
                        EmblemMode::Spin => "←→ / Space 切换 3 种风格 (2/3) · s 保存",
                        EmblemMode::InvertedSpin => "←→ / Space 切换 3 种风格 (3/3) · s 保存",
                    }
                } else {
                    match mode {
                        EmblemMode::Static => "←→ / Space cycle 3 styles (1/3) · s Save",
                        EmblemMode::Spin => "←→ / Space cycle 3 styles (2/3) · s Save",
                        EmblemMode::InvertedSpin => "←→ / Space cycle 3 styles (3/3) · s Save",
                    }
                },
                art,
                compact_art,
            }
        }
        "inline_height" => {
            let rows = value.trim().parse::<u16>().unwrap_or(16).clamp(8, 40);
            let art = config_art_inline_height(rows, art_w, art_h, theme);
            let compact = config_art_inline_height(rows, 38, 5, theme);
            let compact_art = vec![compact[2].clone(), compact[3].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "内嵌抽屉预留行数"
                } else {
                    "Inline Drawer Viewport Rows"
                },
                summary: fallback_summary(if zh {
                    "设置 inline 模式下在命令行提示符下方固定预留的终端抽屉高度"
                } else {
                    "Sets how many terminal rows inline mode reserves below your shell prompt"
                }),
                traits: if zh {
                    [
                        format!("当前配置: {rows} 行 (范围 8..=40，重启生效)"),
                        "16+ 展示 16×8 徽标 · 24+ 解锁 32×16 大图".into(),
                    ]
                } else {
                    [
                        format!("Configured: {rows} rows (8..=40, restart applies)"),
                        "16+ unlocks 16×8 Hydra; 24+ shows full 32×16 art".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ 步进 8..=40 · Enter 轮换 · s 保存"
                } else {
                    "←→ step 8..=40 · Enter cycle presets · s Save"
                },
                art,
                compact_art,
            }
        }
        "ascii" => {
            let active_ascii = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            );
            let art = config_art_ascii(active_ascii, art_w, art_h, theme);
            let compact = config_art_ascii(active_ascii, 38, 5, theme);
            let compact_art = vec![compact[1].clone(), compact[2].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "纯 ASCII 兼容模式"
                } else {
                    "ASCII Compatibility Mode"
                },
                summary: fallback_summary(if zh {
                    "将 Unicode 圆角边框、指示符号与盲文点阵替换为纯粹 7 位 + - | * 字符"
                } else {
                    "Replaces Unicode rounded frames, dots, and Braille with crisp + - | * ASCII"
                }),
                traits: if zh {
                    [
                        "false: 圆角线框、图标徽章与 2×4 点阵徽标".into(),
                        "true: 纯 7-bit ASCII，适配串口与老旧终端".into(),
                    ]
                } else {
                    [
                        "false: Rounded frames, icons & subdot Braille art".into(),
                        "true: Pure 7-bit ASCII for serial & legacy fonts".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 实时切换 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space toggle live · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "no_color" => {
            let active_nc = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            );
            let art = config_art_no_color(active_nc, art_w, art_h, theme);
            let compact = config_art_no_color(active_nc, 38, 5, theme);
            let compact_art = vec![compact[0].clone(), compact[1].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "NO_COLOR 单色模式"
                } else {
                    "Monochrome NO_COLOR Output"
                },
                summary: fallback_summary(if zh {
                    "遵循 NO_COLOR 规范清除全界面 ANSI 前景/背景着色，保持纯单色结构"
                } else {
                    "Strips foreground/background ANSI color attributes across the entire TUI"
                }),
                traits: if zh {
                    [
                        "false: 启用完整真彩渐变、胶囊标签与高亮".into(),
                        "true: 全部复位为 Color::Reset (无颜色单色)".into(),
                    ]
                } else {
                    [
                        "false: Full TrueColor ramps, pills & colored logs".into(),
                        "true: Monochrome Color::Reset (NO_COLOR standard)".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 实时切换 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space toggle live · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "language" | "lang" => {
            let active_lang = Language::from_name(value).unwrap_or(theme.language);
            let art = config_art_language(active_lang, art_w, art_h, theme);
            let compact = config_art_language(active_lang, 38, 5, theme);
            let compact_art = vec![compact[1].clone(), compact[2].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "界面显示语言 (en / zh)"
                } else {
                    "Display Language (en / zh)"
                },
                summary: fallback_summary(if zh {
                    "一键切换 JustMux 全部菜单、快捷键提示、设置图鉴、弹窗与帮助文案"
                } else {
                    "Switches all JustMux menus, keycap hints, config guides, and help copy"
                }),
                traits: if zh {
                    [
                        "en: 默认英文界面（不受系统 $LANG 自动切换）".into(),
                        "zh: 简体中文界面（含中文导航、设置与帮助）".into(),
                    ]
                } else {
                    [
                        "en: English default (ignores system $LANG)".into(),
                        if theme.ascii {
                            "zh: Simplified Chinese UI".into()
                        } else {
                            "zh: Simplified Chinese UI (简体中文)".into()
                        },
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 实时切换 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space toggle live · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        "tmux_popup" | "tmux_keybind" | "popup_keybind" => {
            let enabled = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            );
            let art = config_art_tmux_popup(enabled, art_w, art_h, theme);
            let compact = config_art_tmux_popup(enabled, 38, 5, theme);
            let compact_art = vec![compact[1].clone(), compact[2].clone()];
            ConfigGuideSpec {
                title: if zh {
                    "tmux 全局浮窗唤出快捷键"
                } else {
                    "tmux Global Popup Shortcut"
                },
                summary: fallback_summary(if zh {
                    "开启后自动写入 ~/.tmux.conf 托管块并热加载 Prefix+j，在终端或 agy 内都能随时唤出 jm"
                } else {
                    "Writes a managed Ctrl+b j popup binding to ~/.tmux.conf and reloads tmux for shell & agy"
                }),
                traits: if zh {
                    [
                        "true: 自动写入 ~/.tmux.conf 托管块并热加载生效".into(),
                        "false: 无痕移除托管块，保留你的全部自有配置".into(),
                    ]
                } else {
                    [
                        "true: Writes ~/.tmux.conf managed block + live bind".into(),
                        "false: Cleanly removes managed block, keeps your file".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 立即生效 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space apply now · s Save · Esc Back"
                },
                art,
                compact_art,
            }
        }
        _ => {
            if let Some(guide) =
                build_tmux_comfort_config_guide(key, value, description, art_w, art_h, theme)
            {
                return guide;
            }
            let art = config_art_generic(key, value, theme);
            ConfigGuideSpec {
                title: if zh {
                    "配置参数说明"
                } else {
                    "Configuration Setting"
                },
                summary: fallback_summary(if zh {
                    "调整参数值并按 s 保存写入 config.toml"
                } else {
                    "Adjust setting value and press s to persist config.toml"
                }),
                traits: if zh {
                    [
                        format!("键值: {key} = {value}"),
                        "在 /config 视图中实时交互更新".into(),
                    ]
                } else {
                    [
                        format!("Key: {key} = {value}"),
                        "Interactive live update in /config studio".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Enter 修改 · s 保存 · Esc 返回"
                } else {
                    "←→ / Enter change · s Save · Esc Back"
                },
                compact_art: vec![art[1].clone()],
                art,
            }
        }
    }
}

fn pad_line_to_width(mut line: Line<'static>, width: usize) -> Line<'static> {
    let used = line.width();
    if used < width {
        line.spans.push(Span::raw(" ".repeat(width - used)));
    }
    line
}

pub(super) fn build_art_box(
    total_w: usize,
    title: Line<'static>,
    body: Vec<Line<'static>>,
    bottom: Option<Line<'static>>,
    border_style: Style,
    ascii: bool,
) -> Vec<Line<'static>> {
    let (tl, tr, bl, br, hz, vt) = if ascii {
        ("+", "+", "+", "+", "-", "|")
    } else {
        ("╭", "╮", "╰", "╯", "─", "│")
    };
    let inner_w = total_w.saturating_sub(2);
    let make_edge = |left_c: &str, right_c: &str, cap: Option<Line<'static>>| -> Line<'static> {
        let mut spans = vec![Span::styled(left_c.to_owned(), border_style)];
        if let Some(cap_line) = cap
            && !cap_line.spans.is_empty()
            && cap_line.width() + 2 <= inner_w
        {
            let cap_w = cap_line.width();
            spans.push(Span::styled(hz.to_owned(), border_style));
            spans.extend(cap_line.spans);
            let rest = inner_w.saturating_sub(1 + cap_w);
            if rest > 0 {
                spans.push(Span::styled(hz.repeat(rest), border_style));
            }
        } else {
            spans.push(Span::styled(hz.repeat(inner_w), border_style));
        }
        spans.push(Span::styled(right_c.to_owned(), border_style));
        Line::from(spans)
    };

    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(make_edge(
        tl,
        tr,
        (!title.spans.is_empty()).then_some(title),
    ));
    for row in body {
        let padded = pad_line_to_width(row, inner_w);
        let mut spans = vec![Span::styled(vt.to_owned(), border_style)];
        spans.extend(padded.spans);
        spans.push(Span::styled(vt.to_owned(), border_style));
        out.push(Line::from(spans));
    }
    out.push(make_edge(bl, br, bottom));
    out
}

fn zip_art_columns(
    left: Vec<Line<'static>>,
    left_w: usize,
    gap: usize,
    right: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    let rows = left.len().max(right.len());
    let gap_str = " ".repeat(gap);
    (0..rows)
        .map(|i| {
            let mut spans = Vec::new();
            let l_line = left.get(i).cloned().unwrap_or_default();
            spans.extend(pad_line_to_width(l_line, left_w).spans);
            if gap > 0 {
                spans.push(Span::raw(gap_str.clone()));
            }
            if let Some(r_line) = right.get(i) {
                spans.extend(r_line.spans.clone());
            }
            Line::from(spans)
        })
        .collect()
}

fn spectrum_bar_spans(
    cols: usize,
    theme: Theme,
    glyph_uni: &str,
    glyph_asc: &str,
) -> Vec<Span<'static>> {
    if cols == 0 {
        return Vec::new();
    }
    if theme.no_color {
        let ch = if theme.ascii { "#" } else { glyph_uni };
        return vec![Span::raw(ch.repeat(cols))];
    }
    if !theme.truecolor {
        let ch = if theme.ascii { glyph_asc } else { glyph_uni };
        let third = (cols / 3).max(1);
        let rem = cols.saturating_sub(third * 2);
        return vec![
            Span::styled(
                ch.repeat(third),
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(ch.repeat(third), theme.style(theme.secondary)),
            Span::styled(ch.repeat(rem), theme.style(theme.tertiary)),
        ];
    }
    let ch = if theme.ascii { glyph_asc } else { glyph_uni };
    let (sa, sb, sc) = theme.preset.rgb_stops();
    let max_idx = cols.saturating_sub(1).max(1);
    (0..cols)
        .map(|idx| {
            let scaled = idx * 2;
            let color = if scaled <= max_idx {
                lerp_rgb(sa, sb, scaled, max_idx)
            } else {
                lerp_rgb(sb, sc, scaled - max_idx, max_idx)
            };
            Span::styled(ch.to_owned(), theme.style(color))
        })
        .collect()
}

fn config_art_screen_mode(
    is_inline: bool,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let active_b = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let active_t = theme.style(theme.text).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let full_b = if !is_inline { active_b } else { dim };
    let full_t = if !is_inline { active_t } else { dim };
    let inl_b = if is_inline { active_b } else { dim };
    let inl_t = if is_inline { active_t } else { dim };
    let zh = theme.language.is_zh() && !theme.ascii;

    if art_h >= 8 && art_w >= 48 {
        let dots = |active: bool| -> Vec<Span<'static>> {
            if theme.ascii {
                vec![Span::styled("o o o", if active { active_t } else { dim })]
            } else if active && !theme.no_color {
                vec![
                    Span::styled("●", theme.style(theme.error)),
                    Span::raw(" "),
                    Span::styled("●", theme.style(theme.warning)),
                    Span::raw(" "),
                    Span::styled("●", theme.style(theme.success)),
                ]
            } else {
                vec![Span::styled("● ● ●", dim)]
            }
        };
        let mut full_title_spans = dots(!is_inline);
        full_title_spans.push(Span::styled(
            if zh {
                if !is_inline {
                    " *全屏模式* "
                } else {
                    "  全屏模式  "
                }
            } else if !is_inline {
                " *FULLSCREEN*"
            } else {
                "  FULLSCREEN "
            },
            full_b,
        ));
        let full_bot = Line::from(Span::styled(
            if zh {
                if !is_inline {
                    "● 100% 画布"
                } else {
                    "○ 备用屏幕"
                }
            } else if !is_inline {
                if theme.ascii {
                    "* 100% Canvas"
                } else {
                    "● 100% Canvas"
                }
            } else if theme.ascii {
                "o Alt-Screen"
            } else {
                "○ Alt-Screen"
            },
            full_b,
        ));
        let mini_inner_b = if !is_inline {
            theme.style(theme.secondary)
        } else {
            dim
        };
        let (mtl, mtr, mbl, mbr, mhz, mvt) = if theme.ascii {
            ("+", "+", "+", "+", "---------------", "|")
        } else {
            ("╭", "╮", "╰", "╯", "───────────────", "│")
        };
        let full_body = vec![
            Line::from(vec![
                Span::styled(
                    theme.adapt(" ◫ JustMux "),
                    if !is_inline { active_b } else { dim },
                ),
                Span::styled(
                    theme.adapt(if zh { " 2会 · 3窗" } else { "  2s · 3w" }),
                    dim,
                ),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{mtl}{mhz}{mtr}"), mini_inner_b),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(mvt, mini_inner_b),
                Span::styled(
                    if theme.ascii {
                        " $ cargo ok    "
                    } else {
                        " ✓ cargo 151ok "
                    },
                    if !is_inline {
                        theme.style(theme.success)
                    } else {
                        dim
                    },
                ),
                Span::styled(mvt, mini_inner_b),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{mbl}{mhz}{mbr}"), mini_inner_b),
            ]),
            Line::from(vec![
                Span::styled(
                    theme.adapt(if zh {
                        " ▸ dev  1窗   "
                    } else {
                        " ▸ dev  1w    "
                    }),
                    full_t,
                ),
                Span::styled(
                    if theme.ascii { "+##+" } else { "⠰⣿⣷⣄" },
                    if !is_inline {
                        theme.style(theme.tertiary)
                    } else {
                        dim
                    },
                ),
            ]),
            Line::from(Span::styled(
                theme.adapt(if zh {
                    " › /命令 · 支持鼠标"
                } else {
                    " › /cmd · mouse"
                }),
                dim,
            )),
        ];
        let left_box = build_art_box(
            23,
            Line::from(full_title_spans),
            full_body,
            Some(full_bot),
            full_b,
            theme.ascii,
        );

        let mut inl_title_spans = dots(is_inline);
        inl_title_spans.push(Span::styled(
            if zh {
                if is_inline {
                    " *内嵌模式* "
                } else {
                    "  内嵌模式  "
                }
            } else if is_inline {
                "  *INLINE*   "
            } else {
                "   INLINE    "
            },
            inl_b,
        ));
        let inl_bot = Line::from(Span::styled(
            if zh {
                if is_inline {
                    "● 底部抽屉"
                } else {
                    "○ 底部抽屉"
                }
            } else if is_inline {
                if theme.ascii {
                    "* Docked Drawer"
                } else {
                    "● Docked Drawer"
                }
            } else if theme.ascii {
                "o Docked Drawer"
            } else {
                "○ Docked Drawer"
            },
            inl_b,
        ));
        let dock_hz = if theme.ascii {
            "---------------------"
        } else {
            "─────────────────────"
        };
        let inl_body = vec![
            Line::from(Span::styled(" $ git status -sb", dim)),
            Line::from(Span::styled("  M src/ui/mod.rs", dim)),
            Line::from(Span::styled(" $ jm --inline", dim)),
            Line::from(Span::styled(dock_hz, inl_b)),
            Line::from(Span::styled(theme.adapt(" ▸ dev   0* zsh"), inl_t)),
            Line::from(Span::styled(
                theme.adapt(if zh {
                    " › /命令 · 保留历史"
                } else {
                    " › /cmd · history"
                }),
                dim,
            )),
        ];
        let right_box = build_art_box(
            23,
            Line::from(inl_title_spans),
            inl_body,
            Some(inl_bot),
            inl_b,
            theme.ascii,
        );
        return zip_art_columns(left_box, 23, 2, right_box);
    }

    let (tl, tr, bl, br, hz, vt, badge_f, badge_i) = if theme.ascii {
        (
            "+",
            "+",
            "+",
            "+",
            "--------------",
            "|",
            if !is_inline {
                "*FULLSCREEN"
            } else {
                " FULLSCREEN"
            },
            if is_inline {
                "*INLINE    "
            } else {
                " INLINE    "
            },
        )
    } else if zh {
        (
            "╭",
            "╮",
            "╰",
            "╯",
            "──────────────",
            "│",
            if !is_inline {
                "●全屏模式  "
            } else {
                "○全屏模式  "
            },
            if is_inline {
                "●内嵌模式  "
            } else {
                "○内嵌模式  "
            },
        )
    } else {
        (
            "╭",
            "╮",
            "╰",
            "╯",
            "──────────────",
            "│",
            if !is_inline {
                "●FULLSCREEN"
            } else {
                "○FULLSCREEN"
            },
            if is_inline {
                "●INLINE    "
            } else {
                "○INLINE    "
            },
        )
    };
    vec![
        Line::from(vec![
            Span::styled(format!("{tl}{hz}{tr}"), full_b),
            Span::raw("  "),
            Span::styled(format!("{tl}{hz}{tr}"), inl_b),
        ]),
        Line::from(vec![
            Span::styled(vt, full_b),
            Span::styled(format!(" {badge_f}  "), full_t),
            Span::styled(vt, full_b),
            Span::raw("  "),
            Span::styled(vt, inl_b),
            Span::styled(" $ ls /repos  ", dim),
            Span::styled(vt, inl_b),
        ]),
        Line::from(vec![
            Span::styled(vt, full_b),
            Span::styled(
                if zh {
                    " 100% 工作台  "
                } else {
                    " 100% studio  "
                },
                full_t,
            ),
            Span::styled(vt, full_b),
            Span::raw("  "),
            Span::styled(vt, inl_b),
            Span::styled(hz, inl_b),
            Span::styled(vt, inl_b),
        ]),
        Line::from(vec![
            Span::styled(vt, full_b),
            Span::styled(
                if zh {
                    " 备用屏+鼠标  "
                } else {
                    " alt + mouse  "
                },
                full_t,
            ),
            Span::styled(vt, full_b),
            Span::raw("  "),
            Span::styled(vt, inl_b),
            Span::styled(format!(" {badge_i}  "), inl_t),
            Span::styled(vt, inl_b),
        ]),
        Line::from(vec![
            Span::styled(format!("{bl}{hz}{br}"), full_b),
            Span::raw("  "),
            Span::styled(format!("{bl}{hz}{br}"), inl_b),
        ]),
    ]
}

fn config_art_theme(
    preset: ThemePreset,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let preview_theme = theme.with_preset(preset);
    let mascot = mascot_lines(preview_theme);
    let mascot_style = preview_theme
        .style(preview_theme.accent)
        .add_modifier(Modifier::BOLD);
    let swatch = if theme.ascii { "####" } else { "████" };
    let zh = theme.language.is_zh() && !theme.ascii;
    let header = theme.adapt(&format!(
        "{} {} · {}",
        preset.badge_icon(theme.ascii),
        preset.name(),
        preset.origin()
    ));

    if art_h >= 8 && art_w >= 48 {
        let mut head_line = gradient_spans(&header, preview_theme, true);
        let badge_mode = if theme.no_color {
            "NO_COLOR"
        } else if preview_theme.truecolor {
            if zh { "24位真彩" } else { "24-bit RGB" }
        } else {
            "ANSI 16"
        };
        let badge = pill_span(
            format!(" {badge_mode} "),
            preview_theme.secondary,
            preview_theme.pill_bg(),
            preview_theme,
            false,
        );
        if Line::from(head_line.clone()).width() + 2 + badge.width() <= 48 {
            head_line.push(Span::raw("  "));
            head_line.push(badge);
        }

        let mascot_body: Vec<Line<'static>> = mascot
            .into_iter()
            .map(|row| {
                let mw = row.width();
                let lp = 7usize.saturating_sub(mw) / 2;
                let rp = 7usize.saturating_sub(lp + mw);
                Line::from(Span::styled(
                    format!("{}{row}{}", " ".repeat(lp), " ".repeat(rp)),
                    mascot_style,
                ))
            })
            .collect();
        let mascot_box = build_art_box(
            9,
            Line::from(Span::styled(
                if zh { "图标" } else { "Bot" },
                preview_theme.style(preview_theme.muted),
            )),
            mascot_body,
            None,
            preview_theme.style(preview_theme.accent),
            theme.ascii,
        );

        let mut spec_spans = vec![Span::styled(
            if zh { "色谱     " } else { "Spectrum " },
            theme.style(theme.muted),
        )];
        spec_spans.extend(spectrum_bar_spans(28, preview_theme, "█", "#"));

        let mini_sw = if theme.ascii { "### " } else { "███ " };
        let swatch_row = Line::from(vec![
            Span::styled(mini_sw, preview_theme.style(preview_theme.accent)),
            Span::raw("  "),
            Span::styled(mini_sw, preview_theme.style(preview_theme.secondary)),
            Span::raw("  "),
            Span::styled(mini_sw, preview_theme.style(preview_theme.tertiary)),
            Span::raw("  "),
            Span::styled(mini_sw, preview_theme.style(preview_theme.success)),
            Span::raw("  "),
            Span::styled(mini_sw, preview_theme.style(preview_theme.warning)),
            Span::raw("  "),
            Span::styled(mini_sw, preview_theme.style(preview_theme.error)),
        ]);
        let label_row = Line::from(Span::styled(
            if zh {
                "主色  辅色  三色  成功  警告  错误"
            } else {
                "ACC   SEC   TRT   OK    WRN   ERR"
            },
            theme.style(theme.muted),
        ));

        let (sa, _, sc) = preset.rgb_stops();
        let raw_hex = if theme.no_color {
            format!(
                "theme = \"{}\" · {}",
                preset.slug(),
                if zh { "单色模式" } else { "mono mode" }
            )
        } else if preview_theme.truecolor {
            format!(
                "#{:02X}{:02X}{:02X} -> #{:02X}{:02X}{:02X} · theme=\"{}\"",
                sa.0,
                sa.1,
                sa.2,
                sc.0,
                sc.1,
                sc.2,
                preset.slug()
            )
        } else {
            format!(
                "{} · theme = \"{}\"",
                if zh {
                    "ANSI 16 回退"
                } else {
                    "ANSI 16 fallback"
                },
                preset.slug()
            )
        };
        let hex_or_mode = theme.adapt(&raw_hex);
        let mut hex_spans =
            gradient_spans(&truncate_to_width(&hex_or_mode, 37), preview_theme, false);
        if preview_theme.no_color {
            hex_spans = vec![Span::raw(truncate_to_width(&hex_or_mode, 37))];
        }

        let live_badge_row = Line::from(vec![
            pill_span(
                if zh {
                    " ★ 当前 "
                } else if theme.ascii {
                    " * current "
                } else {
                    " ★ current "
                },
                preview_theme.accent,
                preview_theme.pill_bg(),
                preview_theme,
                true,
            ),
            Span::raw(" "),
            pill_span(
                if zh {
                    " ⚡ 运行 "
                } else if theme.ascii {
                    " ! busy "
                } else {
                    " ⚡ busy "
                },
                preview_theme.warning,
                preview_theme.pill_bg(),
                preview_theme,
                false,
            ),
            Span::raw(" "),
            Span::styled(
                if zh {
                    " › 已选 "
                } else if theme.ascii {
                    " * selected "
                } else {
                    " › selected "
                },
                preview_theme.selected_style(),
            ),
        ]);

        let right_col = vec![
            Line::from(spec_spans),
            swatch_row,
            label_row,
            Line::from(hex_spans),
            live_badge_row,
        ];
        let body_grid = zip_art_columns(mascot_box, 9, 2, right_col);

        let mut dot_spans = Vec::new();
        for (idx, p) in ThemePreset::ALL.iter().enumerate() {
            if idx > 0 {
                dot_spans.push(Span::raw("  "));
            }
            let active = *p == preset;
            let dot = if active {
                if theme.ascii { "*" } else { "●" }
            } else if theme.ascii {
                "o"
            } else {
                "○"
            };
            dot_spans.push(Span::styled(
                format!("{dot} {}", p.slug()),
                if active {
                    preview_theme
                        .style(preview_theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.style(theme.muted)
                },
            ));
        }

        let mut out = Vec::with_capacity(8);
        out.push(pad_line_to_width(Line::from(head_line), 48));
        out.push(Line::default());
        out.extend(body_grid);
        out.push(Line::from(dot_spans));
        return out;
    }

    let mut gradient_line = gradient_spans(
        &format!("theme = \"{}\"", preset.slug()),
        preview_theme,
        true,
    );
    gradient_line.push(Span::styled(
        theme.adapt(if zh {
            "  ·  实时调色板"
        } else {
            "  ·  live palette"
        }),
        theme.style(theme.muted),
    ));

    vec![
        Line::from(Span::styled(truncate_to_width(&header, 38), mascot_style)),
        Line::from(vec![
            Span::styled(mascot[0], mascot_style),
            Span::raw("  "),
            Span::styled(swatch, preview_theme.style(preview_theme.accent)),
            Span::raw(" "),
            Span::styled(swatch, preview_theme.style(preview_theme.secondary)),
            Span::raw(" "),
            Span::styled(swatch, preview_theme.style(preview_theme.tertiary)),
            Span::raw(" "),
            Span::styled(swatch, preview_theme.style(preview_theme.success)),
            Span::raw(" "),
            Span::styled(swatch, preview_theme.style(preview_theme.warning)),
        ]),
        Line::from(vec![
            Span::styled(mascot[1], mascot_style),
            Span::raw("  "),
            Span::styled(
                if zh {
                    "主色 辅色 三色 成功 警告"
                } else {
                    "acc  sec  trt  ok   wrn "
                },
                theme.style(theme.muted),
            ),
        ]),
        Line::from(vec![
            Span::styled(mascot[2], mascot_style),
            Span::raw("  "),
            Span::styled(
                truncate_to_width(
                    &format!(
                        "{}{}",
                        if zh { "模式: " } else { "mode: " },
                        if theme.no_color {
                            if zh { "无色单色" } else { "no-color" }
                        } else if preview_theme.truecolor {
                            if zh {
                                "24位真彩色"
                            } else {
                                "24-bit TrueColor"
                            }
                        } else if zh {
                            "ANSI 16色"
                        } else {
                            "ANSI 16-color"
                        }
                    ),
                    31,
                ),
                theme.style(theme.text),
            ),
        ]),
        Line::from(gradient_line),
    ]
}

fn config_art_composer_mode(
    value: &str,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let norm = value.trim().to_ascii_lowercase();
    let mode = if norm.contains("radar") {
        ComposerMode::LiveRadar
    } else if norm.contains("launch") || norm.contains("quick") {
        ComposerMode::QuickLaunch
    } else {
        ComposerMode::CommandJump
    };
    let zh = theme.language.is_zh() && !theme.ascii;
    let bg = theme.pill_bg();
    let pill = |label: &str, m: ComposerMode| {
        if m == mode {
            pill_span(format!(" {label} "), m.border_color(theme), bg, theme, true)
        } else {
            Span::styled(format!(" {label} "), theme.style(theme.muted))
        }
    };
    let border = theme
        .style(mode.border_color(theme))
        .add_modifier(Modifier::BOLD);

    let (lbl_cmd, lbl_rad, lbl_lch) = if zh {
        ("命令跳转", "实时雷达", "快速启动")
    } else {
        ("command", "radar", "launch")
    };

    if art_h >= 8 && art_w >= 48 {
        let shift_tab_hint = if theme.ascii {
            "       Shift+Tab <->"
        } else if zh {
            "   Shift+Tab ⇄"
        } else {
            "         Shift+Tab ⇄"
        };
        let mut top_spans = vec![
            pill(lbl_cmd, ComposerMode::CommandJump),
            Span::raw(" "),
            pill(lbl_rad, ComposerMode::LiveRadar),
            Span::raw(" "),
            pill(lbl_lch, ComposerMode::QuickLaunch),
        ];
        top_spans.extend(styled_key_hint_spans(
            shift_tab_hint,
            theme.style(theme.muted),
            theme,
        ));
        let top_bar = Line::from(top_spans);
        let dim = theme.style(theme.muted);
        let text_st = theme.style(theme.text);
        let sep_hz = if theme.ascii { "-" } else { "─" }.repeat(46);
        let (box_title, body_lines) = match mode {
            ComposerMode::CommandJump => (
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ▸ 命令面板 & @直达 "
                    } else {
                        " ▸ Command & @Jump "
                    }),
                    border,
                )),
                vec![
                    Line::from(vec![
                        Span::styled(" /resume  ", theme.style(theme.accent)),
                        Span::styled(
                            if zh {
                                "附着或切换会话               核心"
                            } else {
                                "Attach or switch session     CORE"
                            },
                            dim,
                        ),
                    ]),
                    Line::from(Span::styled(
                        theme.adapt(if zh {
                            " ▸/split   2x2 可视化分屏工作台      窗格"
                        } else {
                            " ▸/split   2x2 visual Split Studio  PANE "
                        }),
                        theme.selected_style(),
                    )),
                    Line::from(vec![
                        Span::styled(" @dev-api ", theme.style(theme.secondary)),
                        Span::styled(
                            if zh {
                                "直接跳转到活动窗格           @标签"
                            } else {
                                "Jump directly to live pane   @TAG"
                            },
                            dim,
                        ),
                    ]),
                    Line::from(Span::styled(sep_hz, dim)),
                    Line::from({
                        let mut spans = vec![
                            Span::styled(theme.adapt(" › "), border),
                            Span::styled("/sp_", text_st.add_modifier(Modifier::BOLD)),
                        ];
                        spans.extend(styled_key_hint_spans(
                            if zh {
                                "                     Enter 执行"
                            } else {
                                "                    Enter Execute"
                            },
                            dim,
                            theme,
                        ));
                        spans
                    }),
                ],
            ),
            ComposerMode::LiveRadar => (
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ◈ 实时雷达 · 3 个会话 "
                    } else {
                        " ◈ Live Radar · 3 Sessions "
                    }),
                    border,
                )),
                vec![
                    Line::from(vec![
                        Span::styled(
                            theme.adapt(if zh {
                                " ★ dev-api  3窗 ● "
                            } else {
                                " ★ dev-api  3w ● "
                            }),
                            theme.selected_style(),
                        ),
                        Span::styled(
                            if theme.ascii {
                                " | $ cargo watch"
                            } else {
                                " │ $ cargo watch"
                            },
                            text_st,
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            theme.adapt(if zh {
                                " ▸ web-app  2窗 ● "
                            } else {
                                " ▸ web-app  2w ● "
                            }),
                            text_st,
                        ),
                        Span::styled(
                            if theme.ascii {
                                " | * 151 tests ok"
                            } else if zh {
                                " │ ✓ 151 通过"
                            } else {
                                " │ ✓ 151 tests ok"
                            },
                            theme.style(theme.success),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            theme.adapt(if zh {
                                "   worker   1窗 ○ "
                            } else {
                                "   worker   1w ○ "
                            }),
                            dim,
                        ),
                        Span::styled(
                            if theme.ascii {
                                " | idle shell"
                            } else if zh {
                                " │ 空闲终端"
                            } else {
                                " │ idle shell"
                            },
                            dim,
                        ),
                    ]),
                    Line::from(Span::styled(sep_hz, dim)),
                    styled_key_hint_line(
                        if theme.ascii {
                            " Up/Down Browse | Enter Instant Attach"
                        } else if zh {
                            " ↑↓ 浏览会话 · Enter 立即附着"
                        } else {
                            " ↑↓ Browse sessions · Enter Instant Attach"
                        },
                        dim,
                        theme,
                    ),
                ],
            ),
            ComposerMode::QuickLaunch => (
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ⚡ 快速启动 · 即时工作区 "
                    } else {
                        " ⚡ Quick Launch · Instant Workspace "
                    }),
                    border,
                )),
                vec![
                    Line::from(vec![
                        Span::styled(
                            theme.adapt(if zh { " ⌂ 目录   " } else { " ⌂ CWD    " }),
                            border,
                        ),
                        Span::styled("~/project/justmux", text_st),
                    ]),
                    Line::from(Span::styled(
                        if zh {
                            " 目标     新建或附着同名 tmux 会话"
                        } else {
                            " Target   New or existing tmux session"
                        },
                        dim,
                    )),
                    Line::from(Span::styled(sep_hz, dim)),
                    Line::from({
                        let mut spans = vec![
                            Span::styled(theme.adapt(" › "), border),
                            Span::styled("my-service_", text_st.add_modifier(Modifier::BOLD)),
                        ];
                        spans.extend(styled_key_hint_spans(
                            if zh {
                                "        Enter -> 启动"
                            } else {
                                "        Enter -> Launch"
                            },
                            dim,
                            theme,
                        ));
                        spans
                    }),
                    styled_key_hint_line(
                        if zh {
                            " 留空直接按 Enter 默认使用当前文件夹名"
                        } else {
                            " Empty Enter auto-names from folder basename"
                        },
                        dim,
                        theme,
                    ),
                ],
            ),
        };
        let mut out = vec![pad_line_to_width(top_bar, 48)];
        out.extend(build_art_box(
            48,
            box_title,
            body_lines,
            None,
            border,
            theme.ascii,
        ));
        return out;
    }

    let (tl, tr, bl, br, hz, vt) = if theme.ascii {
        (
            "+",
            "+",
            "+",
            "+",
            "----------------------------------",
            "|",
        )
    } else {
        (
            "╭",
            "╮",
            "╰",
            "╯",
            "──────────────────────────────────",
            "│",
        )
    };
    let (line_a, line_b) = match mode {
        ComposerMode::CommandJump => (
            if zh {
                " /split @dev 1-9 附着 ? 帮助"
            } else {
                " /split  @dev  1-9 attach  ? help "
            },
            if zh {
                " 输入 / 打开命令面板"
            } else {
                " Type / for slash command drawer  "
            },
        ),
        ComposerMode::LiveRadar => (
            if theme.ascii {
                " > dev-api   3 win | active pane  "
            } else if zh {
                " > dev-api 3窗 · 实时窗格"
            } else {
                " > dev-api   3 win · active pane  "
            },
            if zh {
                " 方向键浏览全部活跃会话"
            } else {
                " Arrow-browse every live session  "
            },
        ),
        ComposerMode::QuickLaunch => (
            if zh {
                " > my-service Enter -> 创建"
            } else {
                " > my-service   Enter -> workspace"
            },
            if zh {
                " 在当前目录启动新 tmux 会话"
            } else {
                " Launch new tmux session in cwd   "
            },
        ),
    };
    let row_a = pad_line_to_width(
        styled_key_hint_line(
            line_a,
            theme.style(theme.text).add_modifier(Modifier::BOLD),
            theme,
        ),
        34,
    );
    let row_b = pad_line_to_width(
        Line::from(Span::styled(theme.adapt(line_b), theme.style(theme.muted))),
        34,
    );
    vec![
        Line::from(vec![
            pill(lbl_cmd, ComposerMode::CommandJump),
            Span::raw(" "),
            pill(lbl_rad, ComposerMode::LiveRadar),
            Span::raw(" "),
            pill(lbl_lch, ComposerMode::QuickLaunch),
        ]),
        Line::from(Span::styled(format!("{tl}{hz}{tr}"), border)),
        Line::from({
            let mut spans = vec![Span::styled(vt, border)];
            spans.extend(row_a.spans);
            spans.push(Span::styled(vt, border));
            spans
        }),
        Line::from({
            let mut spans = vec![Span::styled(vt, border)];
            spans.extend(row_b.spans);
            spans.push(Span::styled(vt, border));
            spans
        }),
        Line::from(Span::styled(format!("{bl}{hz}{br}"), border)),
    ]
}

fn config_art_live_preview(
    enabled: bool,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let active_b = theme.style(theme.secondary).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let right_b = if enabled { active_b } else { dim };
    let zh = theme.language.is_zh() && !theme.ascii;

    if art_h >= 8 && art_w >= 48 {
        let sess_box = build_art_box(
            15,
            Line::from(Span::styled(
                if zh { " 会话列表 " } else { " Sessions " },
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            )),
            vec![
                Line::from(Span::styled(
                    theme.adapt(" ★ dev-api 2w"),
                    theme.style(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    theme.adapt(" ▸ 0* cargo  "),
                    theme.selected_style(),
                )),
                Line::from(Span::styled("   1  logs   ", dim)),
                Line::from(Span::styled(
                    theme.adapt(" ● web     1w"),
                    theme.style(theme.text),
                )),
                Line::from(Span::styled(theme.adapt(" ○ worker  1w"), dim)),
                Line::from(Span::styled(
                    if zh {
                        " + 新建窗口"
                    } else {
                        " + New window"
                    },
                    dim,
                )),
            ],
            None,
            active_b,
            theme.ascii,
        );

        let track_lit = if theme.ascii { "#" } else { "█" };
        let track_bar = if theme.ascii { "|" } else { "┃" };
        let row_with_track = |content: Line<'static>, track: &str, tst: Style| -> Line<'static> {
            let mut spans = pad_line_to_width(content, 29).spans;
            spans.push(Span::styled(track.to_owned(), tst));
            Line::from(spans)
        };
        let prev_box = if enabled {
            let mut rgb_line = vec![Span::raw(" ")];
            rgb_line.extend(spectrum_bar_spans(10, theme, "█", "#"));
            rgb_line.push(Span::styled(
                if zh {
                    " ANSI/24位真彩"
                } else {
                    " ANSI/TrueColor"
                },
                theme.style(theme.text),
            ));

            build_art_box(
                32,
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ◈ 实时预览 · dev:0 "
                    } else {
                        " ◈ Live Preview · dev:0 "
                    }),
                    active_b,
                )),
                vec![
                    row_with_track(
                        Line::from(Span::styled(" $ cargo test --all", theme.style(theme.text))),
                        track_lit,
                        theme.style(theme.accent),
                    ),
                    row_with_track(
                        Line::from(Span::styled(
                            "   Compiling justmux v0.2.0",
                            theme.style(theme.secondary),
                        )),
                        track_bar,
                        dim,
                    ),
                    row_with_track(
                        Line::from(Span::styled(
                            "   Finished test in 0.42s",
                            theme.style(theme.success),
                        )),
                        track_bar,
                        dim,
                    ),
                    row_with_track(
                        Line::from(Span::styled(
                            theme.adapt(if zh {
                                " ✓ 通过: 151 项 · 0 失败"
                            } else {
                                " ✓ ok: 151 passed · 0 failed"
                            }),
                            theme.style(theme.success).add_modifier(Modifier::BOLD),
                        )),
                        track_bar,
                        dim,
                    ),
                    row_with_track(Line::from(rgb_line), track_bar, dim),
                    styled_key_hint_line(
                        if theme.ascii {
                            " ^v PgUp/PgDn | Wheel scroll"
                        } else if zh {
                            " ↕ PgUp/PgDn · 滚轮滚动"
                        } else {
                            " ↕ PgUp/PgDn · Wheel scroll"
                        },
                        dim,
                        theme,
                    ),
                ],
                Some(Line::from(Span::styled(
                    if theme.ascii {
                        " ^v 48L Scrollback "
                    } else if zh {
                        " ↕ 48行 历史回看 "
                    } else {
                        " ↕ 48L Scrollback "
                    },
                    theme.style(theme.accent),
                ))),
                right_b,
                theme.ascii,
            )
        } else {
            build_art_box(
                32,
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ✦ 操作提示 (已关闭) "
                    } else {
                        " ✦ Action Hints (Off) "
                    }),
                    dim,
                )),
                vec![
                    Line::from(Span::styled(
                        if zh {
                            " 已暂停实时终端捕获"
                        } else {
                            " Live capture paused"
                        },
                        theme.style(theme.text),
                    )),
                    Line::from(Span::styled(
                        theme.adapt(if zh {
                            " ○ 零 capture-pane 性能开销"
                        } else {
                            " ○ Zero capture-pane overhead"
                        }),
                        dim,
                    )),
                    Line::from(Span::styled(
                        theme.adapt(if zh {
                            " ▸ 极低 SSH 远程带宽占用"
                        } else {
                            " ▸ Minimal SSH bandwidth draw"
                        }),
                        dim,
                    )),
                    Line::from(Span::styled(
                        theme.adapt(if zh {
                            " ▸ 全屏与内嵌模式均适用"
                        } else {
                            " ▸ Fullscreen & Inline ready"
                        }),
                        dim,
                    )),
                    Line::default(),
                    styled_key_hint_line(
                        &theme.adapt(if zh {
                            " Space -> 开启实时预览"
                        } else {
                            " Space -> Enable Live Preview"
                        }),
                        theme.style(theme.accent),
                        theme,
                    ),
                ],
                Some(Line::from(Span::styled(
                    if zh {
                        " 零捕获开销 "
                    } else {
                        " zero capture "
                    },
                    dim,
                ))),
                right_b,
                theme.ascii,
            )
        };
        return zip_art_columns(sess_box, 15, 1, prev_box);
    }

    let (tl, tr, lbox_top, lbox_bot, vt) = if theme.ascii {
        ("+", "+", "+----------+", "+----------+", "|")
    } else {
        ("╭", "╮", "╭──────────╮", "╰──────────╯", "│")
    };
    let (r_head, r_line1, r_line2) = if enabled {
        (
            if theme.ascii {
                "-[] Live Preview-----"
            } else if zh {
                "─◈ 实时预览──────────"
            } else {
                "─◈ Live Preview──────"
            },
            " cargo test --all    ",
            if zh {
                " 42 项测试全部通过"
            } else {
                " ok: 42 tests pass   "
            },
        )
    } else {
        (
            if theme.ascii {
                "-* Action Hints------"
            } else if zh {
                "─✦ 操作提示──────────"
            } else {
                "─✦ Action Hints──────"
            },
            if zh {
                " 预览捕获已暂停"
            } else {
                " preview paused      "
            },
            if zh {
                " 零窗格捕获开销"
            } else {
                " zero pane capture   "
            },
        )
    };
    let line1_style = if enabled {
        theme.style(theme.text)
    } else {
        dim
    };
    let line2_style = if enabled {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        dim
    };
    let r_row1 = pad_line_to_width(Line::from(Span::styled(r_line1, line1_style)), 21);
    let r_row2 = pad_line_to_width(Line::from(Span::styled(r_line2, line2_style)), 21);
    let r_row3 = pad_line_to_width(
        styled_key_hint_line(
            if enabled {
                if theme.ascii {
                    " | ANSI + PgUp/PgDn"
                } else if zh {
                    " ↕ 真彩 + PgUp/PgDn"
                } else {
                    " ↕ ANSI + PgUp/PgDn"
                }
            } else if zh {
                " 捕获已关闭"
            } else {
                " capture disabled"
            },
            dim,
            theme,
        ),
        21,
    );
    let sess_head = pad_line_to_width(
        Line::from(Span::styled(
            if zh { " 会话列表 " } else { " Sessions " },
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )),
        10,
    );
    vec![
        Line::from(vec![
            Span::styled(lbox_top, active_b),
            Span::raw(" "),
            Span::styled(format!("{tl}{r_head}{tr}"), right_b),
        ]),
        Line::from({
            let mut spans = vec![Span::styled(vt, active_b)];
            spans.extend(sess_head.spans);
            spans.push(Span::styled(vt, active_b));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(vt, right_b));
            spans.extend(r_row1.spans);
            spans.push(Span::styled(vt, right_b));
            spans
        }),
        Line::from({
            let mut spans = vec![
                Span::styled(vt, active_b),
                Span::styled(theme.adapt(" ▸ dev    "), theme.style(theme.text)),
                Span::styled(vt, active_b),
                Span::raw(" "),
                Span::styled(vt, right_b),
            ];
            spans.extend(r_row2.spans);
            spans.push(Span::styled(vt, right_b));
            spans
        }),
        Line::from({
            let mut spans = vec![
                Span::styled(vt, active_b),
                Span::styled("   api    ", dim),
                Span::styled(vt, active_b),
                Span::raw(" "),
                Span::styled(vt, right_b),
            ];
            spans.extend(r_row3.spans);
            spans.push(Span::styled(vt, right_b));
            spans
        }),
        Line::from(vec![
            Span::styled(lbox_bot, active_b),
            Span::raw(" "),
            Span::styled(
                if theme.ascii {
                    "+---------------------+"
                } else {
                    "╰─────────────────────╯"
                },
                right_b,
            ),
        ]),
    ]
}

fn config_art_mouse(enabled: bool, art_w: u16, art_h: u16, theme: Theme) -> Vec<Line<'static>> {
    let body_st = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let on_st = if enabled {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let dim = theme.style(theme.muted);
    let zh = theme.language.is_zh() && !theme.ascii;

    if art_h >= 8 && art_w >= 48 {
        let wheel_st = if enabled { on_st } else { dim };
        let mouse_col = if theme.ascii {
            vec![
                Line::from(Span::styled("   .-----.   ", body_st)),
                Line::from(Span::styled("  .| L|R |.  ", body_st)),
                Line::from(Span::styled("  ||--+--||  ", body_st)),
                Line::from(vec![
                    Span::styled("  ||  ", body_st),
                    Span::styled("#", wheel_st),
                    Span::styled("  ||  ", body_st),
                ]),
                Line::from(Span::styled("  |`--+--'|  ", body_st)),
                Line::from(Span::styled("  | *  JM |  ", body_st)),
                Line::from(Span::styled("  |       |  ", body_st)),
                Line::from(Span::styled("  `-------'  ", body_st)),
            ]
        } else {
            vec![
                Line::from(Span::styled("   ╭──┬──╮   ", body_st)),
                Line::from(Span::styled("  ╭┤L │R ├╮  ", body_st)),
                Line::from(Span::styled("  │├──┼──┤│  ", body_st)),
                Line::from(vec![
                    Span::styled("  ││  ", body_st),
                    Span::styled("█", wheel_st),
                    Span::styled("  ││  ", body_st),
                ]),
                Line::from(Span::styled("  │└──┴──┘│  ", body_st)),
                Line::from(Span::styled("  │ ◈  JM │  ", body_st)),
                Line::from(Span::styled("  │       │  ", body_st)),
                Line::from(Span::styled("  ╰───────╯  ", body_st)),
            ]
        };

        let bg = theme.pill_bg();
        let right_box = if enabled {
            build_art_box(
                33,
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ● 鼠标已开启 "
                    } else {
                        " ● MOUSE ENABLED "
                    }),
                    on_st,
                )),
                vec![
                    Line::from(vec![
                        Span::raw(" "),
                        pill_span(
                            theme.adapt(if zh { "▸恢复" } else { "▸Resume" }),
                            theme.accent,
                            bg,
                            theme,
                            true,
                        ),
                        Span::raw(" "),
                        pill_span(
                            if zh {
                                "✎改名"
                            } else if theme.ascii {
                                "*Rename"
                            } else {
                                "✎Rename"
                            },
                            theme.secondary,
                            bg,
                            theme,
                            false,
                        ),
                        Span::raw(" "),
                        pill_span(
                            if zh {
                                "✕关闭"
                            } else if theme.ascii {
                                "xKill"
                            } else {
                                "✕Kill"
                            },
                            theme.error,
                            bg,
                            theme,
                            false,
                        ),
                    ]),
                    Line::from(Span::styled(
                        if theme.ascii {
                            " +- ! Wheel Scroll Zone ----+"
                        } else if zh {
                            " ╭─↕ 滚轮独立滚动区────────╮"
                        } else {
                            " ╭─↕ Wheel Scroll Zone─────╮"
                        },
                        theme.style(theme.secondary),
                    )),
                    Line::from(Span::styled(
                        if theme.ascii {
                            " | Scrollback indep. view   |"
                        } else if zh {
                            " │ 独立滚动回看窗格历史    │"
                        } else {
                            " │ Scrollback indep. view  │"
                        },
                        theme.style(theme.text),
                    )),
                    Line::from(Span::styled(
                        theme.adapt(if zh {
                            " ▸ 单击选行 · 双击进入"
                        } else {
                            " ▸ Click Row = Focus · 2x=Go"
                        }),
                        theme.style(theme.text),
                    )),
                    styled_key_hint_line(
                        if zh {
                            " Alt/Shift + 拖拽 = 终端复制"
                        } else {
                            " Alt/Shift + Drag = TTY Copy"
                        },
                        dim,
                        theme,
                    ),
                    Line::from(Span::styled(
                        if zh {
                            " 仅全屏 (fullscreen) 下有效"
                        } else {
                            " Active in Fullscreen mode"
                        },
                        dim,
                    )),
                ],
                Some(Line::from(Span::styled(
                    if zh {
                        " 点击 + 滚轮 "
                    } else {
                        " click + wheel "
                    },
                    theme.style(theme.accent),
                ))),
                theme.style(theme.secondary),
                theme.ascii,
            )
        } else {
            build_art_box(
                33,
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ○ 鼠标已关闭 "
                    } else {
                        " ○ MOUSE DISABLED "
                    }),
                    dim,
                )),
                vec![
                    Line::from(Span::styled(
                        if zh {
                            " 纯键盘极速导航"
                        } else {
                            " Keyboard-only navigation"
                        },
                        theme.style(theme.text),
                    )),
                    Line::from(Span::styled(
                        if theme.ascii {
                            " +- Native Terminal Copy ---+"
                        } else if zh {
                            " ╭─ 终端原生文本选择 ──────╮"
                        } else {
                            " ╭─ Native Terminal Copy ──╮"
                        },
                        dim,
                    )),
                    Line::from(vec![
                        Span::styled(if theme.ascii { " | " } else { " │ " }, dim),
                        Span::styled(
                            if zh {
                                "鼠标拖拽直接选中复制    "
                            } else {
                                "Drag selects raw text   "
                            },
                            theme.selected_style(),
                        ),
                        Span::styled(if theme.ascii { "|" } else { "│" }, dim),
                    ]),
                    styled_key_hint_line(
                        if theme.ascii {
                            " > ^v Matrix | Tab | 1-9 Jump"
                        } else if zh {
                            " ▸ ↑↓ 矩阵 · Tab · 1-9 跳转"
                        } else {
                            " ▸ ↑↓ Matrix · Tab · 1-9 Jump"
                        },
                        theme.style(theme.text),
                        theme,
                    ),
                    Line::from(Span::styled(
                        if zh {
                            " 终端保留滚轮滚动历史输出"
                        } else {
                            " Terminal owns wheel scroll"
                        },
                        dim,
                    )),
                    styled_key_hint_line(
                        if zh {
                            " Space -> 开启鼠标支持"
                        } else {
                            " Space -> Enable Mouse"
                        },
                        dim,
                        theme,
                    ),
                ],
                Some(Line::from(Span::styled(
                    if zh {
                        " 重启后生效 "
                    } else {
                        " restart applies "
                    },
                    dim,
                ))),
                dim,
                theme.ascii,
            )
        };
        return zip_art_columns(mouse_col, 14, 1, right_box);
    }

    let art_rows = if theme.ascii {
        [" .---. ", "| L|R |", "|--+--|", "|  |  |", " `---' "]
    } else {
        [" ╭─┬─╮ ", " │L│R│ ", " ├─┴─┤ ", " │ ↑ │ ", " ╰───╯ "]
    };
    let notes = if enabled {
        [
            (
                theme.adapt(if zh {
                    "● 鼠标已开启"
                } else {
                    "● MOUSE ENABLED"
                }),
                on_st,
            ),
            (
                theme.adapt(if zh {
                    "▸ 点击标签与矩阵行"
                } else {
                    "▸ Click tabs & matrix rows"
                }),
                theme.style(theme.text),
            ),
            (
                theme.adapt(if zh {
                    "▸ 滚轮翻看窗格预览"
                } else {
                    "▸ Wheel scrolls pane preview"
                }),
                theme.style(theme.text),
            ),
            (
                theme.adapt(if zh {
                    "▸ 需配合 fullscreen 模式"
                } else {
                    "▸ Requires Fullscreen mode"
                }),
                dim,
            ),
            (
                theme.adapt(if zh {
                    "  按住 Shift 可原生复制"
                } else {
                    "  Hold Shift for terminal copy"
                }),
                dim,
            ),
        ]
    } else {
        [
            (
                theme.adapt(if zh {
                    "○ 鼠标已关闭"
                } else {
                    "○ MOUSE DISABLED"
                }),
                dim,
            ),
            (
                if zh {
                    "  纯键盘快速导航"
                } else {
                    "  Keyboard-only navigation"
                }
                .into(),
                dim,
            ),
            (
                if zh {
                    "  拖拽保留终端原生复制"
                } else {
                    "  Drag selects native text"
                }
                .into(),
                theme.style(theme.text),
            ),
            (
                if zh {
                    "  滚轮使用终端默认滚动"
                } else {
                    "  Wheel uses terminal scroll"
                }
                .into(),
                dim,
            ),
            (
                if zh {
                    "  全屏重启后应用更改"
                } else {
                    "  Restart applies Fullscreen"
                }
                .into(),
                dim,
            ),
        ]
    };
    art_rows
        .into_iter()
        .zip(notes)
        .map(|(glyph, (label, lstyle))| {
            Line::from(vec![
                Span::styled(glyph, body_st),
                Span::raw("  "),
                Span::styled(truncate_to_width(&label, 29), lstyle),
            ])
        })
        .collect()
}

fn config_art_emblem_mode(
    active_mode: EmblemMode,
    emblem_step: usize,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let modes = [
        EmblemMode::Static,
        EmblemMode::Spin,
        EmblemMode::InvertedSpin,
    ];
    let zh = theme.language.is_zh() && !theme.ascii;
    let step_for = |mode: EmblemMode| -> usize {
        if mode == EmblemMode::Static {
            0
        } else if emblem_step == 0 {
            6
        } else {
            emblem_step
        }
    };
    let short_caption = |mode: EmblemMode| -> &'static str {
        if zh {
            match mode {
                EmblemMode::Static => "静止",
                EmblemMode::Spin => "正转",
                EmblemMode::InvertedSpin => "反转",
            }
        } else {
            match mode {
                EmblemMode::Static => "static",
                EmblemMode::Spin => "spin",
                EmblemMode::InvertedSpin => "invert",
            }
        }
    };
    let bottom_note = |mode: EmblemMode, step: usize| -> String {
        let deg = (step % HYDRA_ROTATION_STEPS) * 5;
        let deg_unit = if theme.ascii { "deg" } else { "°" };
        if zh {
            match mode {
                EmblemMode::Static => format!("0{deg_unit} 固定"),
                EmblemMode::Spin => format!("{deg}{deg_unit} 正转"),
                EmblemMode::InvertedSpin => format!("{deg}{deg_unit} 反转"),
            }
        } else {
            match mode {
                EmblemMode::Static => format!("0{deg_unit} fixed"),
                EmblemMode::Spin => format!("{deg}{deg_unit} spin"),
                EmblemMode::InvertedSpin => format!("{deg}{deg_unit} invert"),
            }
        }
    };

    if (art_w >= 50 && art_h >= 9) || (art_w >= 38 && art_h >= 7) {
        let (thumb_cols, thumb_rows) = if art_w >= 50 && art_h >= 9 {
            (14_u16, 7_u16)
        } else {
            (10_u16, 5_u16)
        };
        let box_w = usize::from(thumb_cols) + 2;
        let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
        let dim = theme.style(theme.muted);

        let build_card = |mode: EmblemMode| -> Vec<Line<'static>> {
            let is_active = mode == active_mode;
            let step = step_for(mode);
            let cap_raw = short_caption(mode);
            let cap_text = if is_active {
                format!("*{cap_raw}*")
            } else {
                cap_raw.to_owned()
            };
            let title_st = if is_active { active_st } else { dim };
            let b_st = if is_active { active_st } else { dim };
            let body: Vec<Line<'static>> = if is_active {
                hydra_thumbnail_lines(thumb_cols, thumb_rows, step, mode, theme)
            } else {
                hydra_thumbnail_rows(thumb_cols, thumb_rows, step, mode, theme.ascii)
                    .into_iter()
                    .map(|row| Line::from(Span::styled(row, dim)))
                    .collect()
            };
            let bot = bottom_note(mode, step);
            let bot_opt = (bot.width() + 2 <= usize::from(thumb_cols))
                .then(|| Line::from(Span::styled(format!(" {bot} "), title_st)));
            build_art_box(
                box_w,
                Line::from(Span::styled(format!(" {cap_text} "), title_st)),
                body,
                bot_opt,
                b_st,
                theme.ascii,
            )
        };

        let c0 = build_card(modes[0]);
        let c1 = build_card(modes[1]);
        let c2 = build_card(modes[2]);
        let left_two = zip_art_columns(c0, box_w, 1, c1);
        return zip_art_columns(left_two, box_w * 2 + 1, 1, c2);
    }

    let (thumb_cols, thumb_rows) = (8_u16, 4_u16);
    let cell_w = usize::from(thumb_cols);
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let pad_raw = |raw: &str| -> String {
        let pad = cell_w.saturating_sub(raw.chars().count());
        format!("{raw}{}", " ".repeat(pad))
    };

    let render_col = |mode: EmblemMode, r: usize| -> Line<'static> {
        let step = step_for(mode);
        if mode == active_mode {
            hydra_thumbnail_lines(thumb_cols, thumb_rows, step, mode, theme)
                .get(r)
                .cloned()
                .unwrap_or_default()
        } else {
            let text = hydra_thumbnail_rows(thumb_cols, thumb_rows, step, mode, theme.ascii)
                .get(r)
                .cloned()
                .unwrap_or_default();
            Line::from(Span::styled(pad_raw(&text), dim))
        }
    };

    let mut out = Vec::with_capacity(usize::from(thumb_rows) + 1);
    for r in 0..usize::from(thumb_rows) {
        let mut spans = Vec::new();
        for (idx, &mode) in modes.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw("   "));
            }
            let line = pad_line_to_width(render_col(mode, r), cell_w);
            spans.extend(line.spans);
        }
        out.push(Line::from(spans));
    }

    let center_in_w = |text: &str, width: usize| -> String {
        let w = text.width();
        let lp = width.saturating_sub(w) / 2;
        let rp = width.saturating_sub(lp + w);
        format!("{}{text}{}", " ".repeat(lp), " ".repeat(rp))
    };
    let cap_spans: Vec<Span<'static>> = modes
        .into_iter()
        .enumerate()
        .flat_map(|(idx, mode)| {
            let is_active = mode == active_mode;
            let text = if is_active {
                format!("*{}*", short_caption(mode))
            } else {
                short_caption(mode).to_owned()
            };
            let st = if is_active { active_st } else { dim };
            let mut items = Vec::new();
            if idx > 0 {
                items.push(Span::raw("   "));
            }
            items.push(Span::styled(center_in_w(&text, cell_w), st));
            items
        })
        .collect();
    out.push(Line::from(cap_spans));
    out
}

fn emblem_mode_index(mode: EmblemMode) -> usize {
    match mode {
        EmblemMode::Static => 1,
        EmblemMode::Spin => 2,
        EmblemMode::InvertedSpin => 3,
    }
}

fn config_art_emblem_compact_row(active_mode: EmblemMode, theme: Theme) -> Line<'static> {
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let mode_label = |m: EmblemMode| -> &'static str {
        if theme.language.is_zh() && !theme.ascii {
            match m {
                EmblemMode::Static => "静止",
                EmblemMode::Spin => "正转",
                EmblemMode::InvertedSpin => "反转",
            }
        } else {
            m.as_str()
        }
    };
    Line::from(vec![
        Span::styled(format!("[{}]", mode_label(active_mode.prev())), dim),
        Span::raw(" "),
        Span::styled(format!("[*{}*]", mode_label(active_mode)), active_st),
        Span::raw(" "),
        Span::styled(format!("[{}]", mode_label(active_mode.next())), dim),
        Span::styled(format!(" ({}/3)", emblem_mode_index(active_mode)), dim),
    ])
}

fn config_art_inline_height(rows: u16, art_w: u16, art_h: u16, theme: Theme) -> Vec<Line<'static>> {
    let filled = if rows <= 8 {
        1
    } else {
        ((usize::from(rows - 8) * 14) / 32 + 1).min(15)
    };
    let border = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let text_st = theme.style(theme.text);
    let zh = theme.language.is_zh() && !theme.ascii;

    if art_h >= 8 && art_w >= 48 {
        let drawer_rows: usize = ((usize::from(rows - 8) * 4) / 32 + 2).clamp(2, 6);
        let shell_rows = 6usize.saturating_sub(drawer_rows);
        let shell_samples = [
            " $ ls -la",
            " $ git log -1 --oneline",
            "  306508d Release v0.2.0",
            " $ cargo check --all",
        ];
        let mut monitor_body = Vec::with_capacity(6);
        for line in shell_samples.iter().take(shell_rows) {
            monitor_body.push(Line::from(Span::styled(*line, dim)));
        }
        let div_lbl = format!(
            "{} {} ({rows:>2}{}) {}",
            if theme.ascii { "-" } else { "─" },
            if zh { "抽屉高" } else { "Inline Drawer" },
            if zh { "行" } else { "r" },
            if theme.ascii { "---" } else { "───" }
        );
        monitor_body.push(Line::from(Span::styled(div_lbl, border)));
        let tier_str = if zh {
            if rows >= 24 {
                " 32x16 徽标 + 预览"
            } else if rows >= 16 {
                " 16x8 徽标已解锁"
            } else {
                " 紧凑命令菜单模式"
            }
        } else if rows >= 24 {
            " Hydra 32x16 + Preview"
        } else if rows >= 16 {
            " Hydra 16x8 Unlocked"
        } else {
            " Compact Menu Mode"
        };
        if drawer_rows >= 3 {
            monitor_body.push(Line::from(Span::styled(
                theme.adapt(if zh {
                    " ★ dev-api  2窗 ●"
                } else {
                    " ★ dev-api  2w ●"
                }),
                theme.style(theme.secondary).add_modifier(Modifier::BOLD),
            )));
        }
        if drawer_rows >= 4 {
            monitor_body.push(Line::from(Span::styled(
                tier_str,
                theme.style(theme.success),
            )));
        }
        if drawer_rows >= 5 {
            monitor_body.push(Line::from(Span::styled(
                theme.adapt(if zh {
                    " ▸ 保留上方终端历史"
                } else {
                    " ▸ Live shell history"
                }),
                dim,
            )));
        }
        if drawer_rows >= 6 {
            monitor_body.push(Line::from(Span::styled(
                if zh {
                    " 24+ 行启用宽敞视图"
                } else {
                    " 24+ rows = spacious"
                },
                dim,
            )));
        }
        monitor_body.push(Line::from(Span::styled(
            theme.adapt(if zh {
                " › /命令 · 保留终端"
            } else {
                " › /cmd · shell kept"
            }),
            text_st,
        )));

        let monitor_box = build_art_box(
            24,
            Line::from(Span::styled(
                if zh {
                    " 终端纵向剖面图 "
                } else {
                    " Terminal Cross-Section "
                },
                text_st,
            )),
            monitor_body,
            Some(Line::from(Span::styled(
                if zh {
                    format!(" {rows} / 40 行 ")
                } else {
                    format!(" {rows} / 40 rows ")
                },
                theme.style(theme.accent),
            ))),
            dim,
            theme.ascii,
        );

        let bar_cols = 15_usize;
        let mut gauge_spans = vec![Span::styled("8 [", border)];
        let fill_all = spectrum_bar_spans(bar_cols, theme, "█", "#");
        let empty_ch = if theme.ascii { "-" } else { "─" };
        for sp in fill_all.into_iter().take(filled) {
            gauge_spans.push(sp);
        }
        if filled < bar_cols {
            gauge_spans.push(Span::styled(empty_ch.repeat(bar_cols - filled), dim));
        }
        gauge_spans.push(Span::styled("] 40", border));

        let check_line = |req: u16, label: &str| -> Line<'static> {
            let ok = rows >= req;
            let mark = if ok {
                if theme.ascii { "*" } else { "✓" }
            } else if theme.ascii {
                "o"
            } else {
                "○"
            };
            Line::from(vec![
                Span::styled(
                    format!(" {mark} "),
                    if ok {
                        theme.style(theme.success).add_modifier(Modifier::BOLD)
                    } else {
                        dim
                    },
                ),
                Span::styled(label.to_owned(), if ok { text_st } else { dim }),
            ])
        };

        let pct = (usize::from(rows) * 100) / 40;
        let right_col = vec![
            Line::from(Span::styled(
                if zh {
                    format!(" 预留高度: {rows:>2} / 40 行 ")
                } else {
                    format!(" Reserved rows: {rows:>2} / 40 ")
                },
                text_st.add_modifier(Modifier::BOLD),
            )),
            Line::from(gauge_spans),
            Line::from(Span::styled(
                theme.adapt(&if zh {
                    let tier = if rows >= 24 {
                        "宽敞"
                    } else if rows >= 16 {
                        "徽标"
                    } else {
                        "紧凑"
                    };
                    format!(" 比例: {pct:>3}% · 档位 {tier}")
                } else {
                    let tier = if rows >= 24 {
                        "Spacious"
                    } else if rows >= 16 {
                        "Hydra"
                    } else {
                        "Compact"
                    };
                    format!(" Scale: {pct:>3}% · tier {tier}")
                }),
                dim,
            )),
            check_line(
                8,
                if zh {
                    " 8+ 内嵌模式基线"
                } else {
                    " 8+ Inline baseline"
                },
            ),
            check_line(
                16,
                if zh {
                    "16+ 解锁 16x8 徽标"
                } else {
                    "16+ Hydra 16x8 emblem"
                },
            ),
            check_line(
                24,
                if zh {
                    "24+ 32x16 徽标+预览"
                } else {
                    "24+ Hydra 32x16 + Prev"
                },
            ),
            check_line(
                40,
                if zh {
                    "40  抽屉高度上限"
                } else {
                    "40  Max drawer height"
                },
            ),
            Line::from(Span::styled(
                if zh {
                    " 重启后按新高度生效"
                } else {
                    " Restart applies height"
                },
                dim,
            )),
        ];
        return zip_art_columns(monitor_box, 24, 1, right_col);
    }

    let (fill_ch, empty_ch) = if theme.ascii {
        ("#", "-")
    } else {
        ("█", "─")
    };
    let bar_fill = fill_ch.repeat(filled + 1);
    let bar_rest = empty_ch.repeat(16 - (filled + 1));
    let (tl, tr, bl, br, hz_top, vt) = if theme.ascii {
        ("+", "+", "+", "+", "---- Inline Drawer -----", "|")
    } else if zh {
        ("╭", "╮", "╰", "╯", "────── 内嵌抽屉 ──────", "│")
    } else {
        ("╭", "╮", "╰", "╯", "──── Inline Drawer ─────", "│")
    };
    let row_label = if zh {
        format!(" 预留高度: {rows:>2} / 40 行 ")
    } else {
        format!(" Reserved rows: {rows:>2} / 40 ")
    };
    let row_pad = " ".repeat(24usize.saturating_sub(row_label.width()));
    let tier_tag = if zh {
        if rows >= 24 {
            "32x16徽标"
        } else {
            "16x8 徽标"
        }
    } else if rows >= 24 {
        "Hydra 32x16"
    } else {
        "Hydra 16x8"
    };
    let bottom_note = if zh {
        format!(" 小:8  {tier_tag:^8}  大:40 ")
    } else {
        format!(" min:8 {tier_tag:^10} max:40 ")
    };
    let bot_pad =
        if theme.ascii { "-" } else { "─" }.repeat(24usize.saturating_sub(bottom_note.width()));
    vec![
        Line::from(Span::styled(
            if zh {
                " $ ls -la   # 保留终端历史"
            } else {
                " $ ls -la   # shell history kept"
            },
            dim,
        )),
        Line::from(Span::styled(format!(" {tl}{hz_top}{tr}"), border)),
        Line::from(vec![
            Span::styled(format!(" {vt}"), border),
            Span::styled(
                row_label,
                theme.style(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::raw(row_pad),
            Span::styled(vt, border),
        ]),
        Line::from(vec![
            Span::styled(format!(" {vt} 8 ["), border),
            Span::styled(bar_fill, theme.style(theme.accent)),
            Span::styled(bar_rest, dim),
            Span::styled("] 40 ", border),
            Span::styled(vt, border),
        ]),
        Line::from(Span::styled(
            format!(" {bl}{bottom_note}{bot_pad}{br}"),
            border,
        )),
    ]
}

fn config_art_ascii(
    active_ascii: bool,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let uni_st = if !active_ascii { active_st } else { dim };
    let asc_st = if active_ascii { active_st } else { dim };
    let zh = theme.language.is_zh();

    if art_h >= 8 && art_w >= 48 {
        let asc_hydra = hydra_thumbnail_rows(6, 3, 0, EmblemMode::Spin, true);
        let left_box = if theme.ascii {
            build_art_box(
                23,
                Line::from(Span::styled(
                    if !active_ascii {
                        " * Unicode * "
                    } else {
                        "   Unicode   "
                    },
                    uni_st,
                )),
                vec![
                    Line::from(Span::styled(" Rounded borders (UTF8)", uni_st)),
                    Line::from(Span::styled(" [Braille art emblem]", uni_st)),
                    Line::from(Span::styled(
                        format!(
                            "  {}   (hidden)",
                            asc_hydra.first().map_or("", String::as_str)
                        ),
                        dim,
                    )),
                    Line::from(Span::styled(" Rich icons & meters", dim)),
                    Line::from(Span::styled(" Set ascii=false to ", dim)),
                    Line::from(Span::styled(" render full UTF-8 UI", dim)),
                ],
                None,
                uni_st,
                true,
            )
        } else {
            let uni_hydra = hydra_thumbnail_lines(6, 3, 0, EmblemMode::Spin, theme);
            let row_with_hydra = |idx: usize, label: &str, st: Style| -> Line<'static> {
                let mut spans = vec![Span::raw(" ")];
                if let Some(hl) = uni_hydra.get(idx) {
                    spans.extend(pad_line_to_width(hl.clone(), 6).spans);
                } else {
                    spans.push(Span::raw("      "));
                }
                spans.push(Span::raw("  "));
                spans.push(Span::styled(label.to_owned(), st));
                Line::from(spans)
            };
            build_art_box(
                23,
                Line::from(Span::styled(
                    if !active_ascii {
                        " ● Unicode "
                    } else {
                        " ○ Unicode "
                    },
                    uni_st,
                )),
                vec![
                    row_with_hydra(
                        0,
                        if zh {
                            "╭─╮ 圆角框"
                        } else {
                            "╭─╮ Rounded "
                        },
                        uni_st,
                    ),
                    row_with_hydra(
                        1,
                        if zh {
                            "盲文点阵图"
                        } else {
                            "Braille art "
                        },
                        theme.style(theme.secondary),
                    ),
                    row_with_hydra(2, if zh { "2x4 亚像素" } else { "2x4 subdots " }, dim),
                    Line::from(Span::styled(" ◈ ✦ ★ ● ▸ ✓ ↕ ∞", theme.style(theme.text))),
                    Line::from(Span::styled(
                        if zh {
                            " ▁▂▃▄▅▆▇█ 图表刻度"
                        } else {
                            " ▁▂▃▄▅▆▇█ meters"
                        },
                        theme.style(theme.accent),
                    )),
                    Line::from(Span::styled(
                        if zh {
                            " 完整 UTF-8 与中文界面"
                        } else {
                            " Modern UTF-8 canvas"
                        },
                        dim,
                    )),
                ],
                None,
                uni_st,
                false,
            )
        };

        let right_title = if theme.ascii || !zh {
            if active_ascii {
                " * ASCII Only "
            } else {
                "   ASCII Only "
            }
        } else if active_ascii {
            " * 纯 ASCII "
        } else {
            "   纯 ASCII "
        };
        let right_body = if !theme.ascii && zh {
            vec![
                Line::from(Span::styled(
                    format!(
                        " {}  +-+ 简框",
                        asc_hydra.first().map_or("", String::as_str)
                    ),
                    asc_st,
                )),
                Line::from(Span::styled(
                    format!(
                        " {}  ASCII 徽标",
                        asc_hydra.get(1).map_or("", String::as_str)
                    ),
                    asc_st,
                )),
                Line::from(Span::styled(
                    format!(
                        " {}  7-bit 安全",
                        asc_hydra.get(2).map_or("", String::as_str)
                    ),
                    dim,
                )),
                Line::from(Span::styled(" > * # + - | <-> inf", asc_st)),
                Line::from(Span::styled(" .:-=+*#%@ 密度阶", dim)),
                Line::from(Span::styled(" 纯 7 位兼容老旧终端", dim)),
            ]
        } else {
            vec![
                Line::from(Span::styled(
                    format!(
                        " {}  +-+ Plain ",
                        asc_hydra.first().map_or("", String::as_str)
                    ),
                    asc_st,
                )),
                Line::from(Span::styled(
                    format!(
                        " {}  ASCII Hydra ",
                        asc_hydra.get(1).map_or("", String::as_str)
                    ),
                    asc_st,
                )),
                Line::from(Span::styled(
                    format!(
                        " {}  7-bit safe  ",
                        asc_hydra.get(2).map_or("", String::as_str)
                    ),
                    dim,
                )),
                Line::from(Span::styled(" > * # + - | <-> inf", asc_st)),
                Line::from(Span::styled(" .:-=+*#%@ density", dim)),
                Line::from(Span::styled(" Pure 7-bit TTY safe", dim)),
            ]
        };
        let right_box = build_art_box(
            23,
            Line::from(Span::styled(right_title, asc_st)),
            right_body,
            None,
            asc_st,
            true,
        );
        return zip_art_columns(left_box, 23, 2, right_box);
    }

    if theme.ascii {
        return vec![
            Line::from(vec![
                Span::styled("[Unicode glyphs] ", uni_st),
                Span::styled("+--------------+", asc_st),
            ]),
            Line::from(vec![
                Span::styled("  rounded box    ", uni_st),
                Span::styled("| * ASCII ONLY |", asc_st),
            ]),
            Line::from(vec![
                Span::styled("  Braille art    ", uni_st),
                Span::styled("| +-+ > * v oo |", asc_st),
            ]),
            Line::from(vec![
                Span::styled("  7-bit active   ", dim),
                Span::styled("| pure 7-bit   |", asc_st),
            ]),
            Line::from(vec![
                Span::styled("                 ", dim),
                Span::styled("+--------------+", asc_st),
            ]),
        ];
    }

    vec![
        Line::from(vec![
            Span::styled("╭──────────────╮", uni_st),
            Span::raw("  "),
            Span::styled("+--------------+", asc_st),
        ]),
        Line::from(vec![
            Span::styled("│", uni_st),
            Span::styled(
                if !active_ascii {
                    " ● Unicode    "
                } else {
                    " ○ Unicode    "
                },
                uni_st,
            ),
            Span::styled("│", uni_st),
            Span::raw("  "),
            Span::styled("|", asc_st),
            Span::styled(
                if zh {
                    if active_ascii {
                        " * 纯 ASCII   "
                    } else {
                        "   纯 ASCII   "
                    }
                } else if active_ascii {
                    " * ASCII Only "
                } else {
                    "   ASCII Only "
                },
                asc_st,
            ),
            Span::styled("|", asc_st),
        ]),
        Line::from(vec![
            Span::styled("│", uni_st),
            Span::styled(" ╭─╮ ◈ • ✓ ∞  ", theme.style(theme.secondary)),
            Span::styled("│", uni_st),
            Span::raw("  "),
            Span::styled("|", asc_st),
            Span::styled(" +-+ > * v oo ", asc_st),
            Span::styled("|", asc_st),
        ]),
        Line::from(vec![
            Span::styled("│", uni_st),
            Span::styled(
                if zh {
                    " 盲文点阵+中文"
                } else {
                    " Braille art  "
                },
                dim,
            ),
            Span::styled("│", uni_st),
            Span::raw("  "),
            Span::styled("|", asc_st),
            Span::styled(
                if zh {
                    " 7-bit 兼容   "
                } else {
                    " pure 7-bit   "
                },
                dim,
            ),
            Span::styled("|", asc_st),
        ]),
        Line::from(vec![
            Span::styled("╰──────────────╯", uni_st),
            Span::raw("  "),
            Span::styled("+--------------+", asc_st),
        ]),
    ]
}

fn config_art_no_color(
    active_nc: bool,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let show_color = !active_nc && !theme.no_color;
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let zh = theme.language.is_zh() && !theme.ascii;
    let blk = if theme.ascii {
        "######"
    } else {
        "██████"
    };
    let plain_blk = if theme.ascii {
        "------"
    } else {
        "──────"
    };
    let (color_head, mono_head) = if show_color {
        if zh {
            ("[* 开启色彩 ]", "[  关闭单色 ]")
        } else {
            ("[* Color ON ]", "[  Mono OFF ]")
        }
    } else if zh {
        ("[  关闭色彩 ]", "[* 无色单色 ]")
    } else {
        ("[  Color OFF]", "[* NO_COLOR ]")
    };

    if art_h >= 8 && art_w >= 48 {
        let mut ramp_spans = vec![Span::raw(" ")];
        if show_color {
            ramp_spans.extend(spectrum_bar_spans(19, theme, "█", "#"));
        } else {
            ramp_spans.push(Span::styled(
                if theme.ascii { "-" } else { "─" }.repeat(19),
                dim,
            ));
        }

        let mini_sw = if theme.ascii { "###" } else { "███" };
        let color_b = if show_color { active_st } else { dim };
        let mono_b = if !show_color { active_st } else { dim };
        let left_box = build_art_box(
            23,
            Line::from(Span::styled(format!(" {color_head} "), color_b)),
            vec![
                Line::from(ramp_spans),
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        mini_sw,
                        if show_color {
                            theme.style(theme.accent)
                        } else {
                            dim
                        },
                    ),
                    Span::raw("  "),
                    Span::styled(
                        mini_sw,
                        if show_color {
                            theme.style(theme.secondary)
                        } else {
                            dim
                        },
                    ),
                    Span::raw("  "),
                    Span::styled(
                        mini_sw,
                        if show_color {
                            theme.style(theme.tertiary)
                        } else {
                            dim
                        },
                    ),
                    Span::styled(
                        if zh { "  色彩" } else { "  RGB" },
                        if show_color {
                            theme.style(theme.text)
                        } else {
                            dim
                        },
                    ),
                ]),
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        "[OK] ",
                        if show_color {
                            theme.style(theme.success).add_modifier(Modifier::BOLD)
                        } else {
                            dim
                        },
                    ),
                    Span::styled(
                        "[WARN] ",
                        if show_color {
                            theme.style(theme.warning).add_modifier(Modifier::BOLD)
                        } else {
                            dim
                        },
                    ),
                    Span::styled(
                        "[ERR]",
                        if show_color {
                            theme.style(theme.error).add_modifier(Modifier::BOLD)
                        } else {
                            dim
                        },
                    ),
                ]),
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        " ✓ 151 通过 · 0 错"
                    } else {
                        " ✓ 151 pass · 0 err"
                    }),
                    if show_color {
                        theme.style(theme.success)
                    } else {
                        dim
                    },
                )),
                Line::from(Span::styled(
                    if zh {
                        " 彩色徽章+ANSI 着色"
                    } else {
                        " Styled pills + ANSI"
                    },
                    dim,
                )),
                Line::from(Span::styled(
                    if show_color {
                        if theme.truecolor {
                            if zh {
                                " 24位真彩色 已开启"
                            } else {
                                " 24-bit TrueColor ON"
                            }
                        } else if zh {
                            " 16色 ANSI 已开启"
                        } else {
                            " 16-color ANSI ON"
                        }
                    } else if zh {
                        " 颜色属性已禁用"
                    } else {
                        " Color attributes OFF"
                    },
                    color_b,
                )),
            ],
            None,
            color_b,
            theme.ascii,
        );

        let mono_strip = if theme.ascii {
            " #####::::::------"
        } else {
            " ▓▓▓▓▓▒▒▒▒▒▒░░░░░░"
        };
        let right_box = build_art_box(
            23,
            Line::from(Span::styled(format!(" {mono_head} "), mono_b)),
            vec![
                Line::from(Span::raw(mono_strip)),
                Line::from(Span::raw(if zh {
                    " 主色 辅色 三色 重置"
                } else {
                    " ACC  SEC  TRT  Reset"
                })),
                Line::from(Span::raw(" [OK] [WARN] [ERR]")),
                Line::from(Span::raw(theme.adapt(if zh {
                    " * 151 通过 · 0 错"
                } else {
                    " * 151 pass · 0 err"
                }))),
                Line::from(Span::raw(if zh {
                    " 前景色/背景色 重置"
                } else {
                    " SGR fg/bg = Reset"
                })),
                Line::from(Span::raw(if zh {
                    " 纯单色安全兼容模式"
                } else {
                    " Monochrome safe mode"
                })),
            ],
            None,
            mono_b,
            theme.ascii,
        );
        return zip_art_columns(left_box, 23, 2, right_box);
    }

    vec![
        Line::from(vec![
            Span::styled(color_head, if show_color { active_st } else { dim }),
            Span::raw("    "),
            Span::styled(mono_head, if !show_color { active_st } else { dim }),
        ]),
        Line::from(vec![
            Span::styled(
                blk,
                if show_color {
                    theme.style(theme.accent)
                } else {
                    dim
                },
            ),
            Span::raw(" "),
            Span::styled(
                blk,
                if show_color {
                    theme.style(theme.secondary)
                } else {
                    dim
                },
            ),
            Span::raw(" "),
            Span::styled(
                blk,
                if show_color {
                    theme.style(theme.tertiary)
                } else {
                    dim
                },
            ),
            Span::raw("   "),
            Span::styled(plain_blk, dim),
        ]),
        Line::from(vec![Span::styled(
            if zh {
                "主色   辅色   三色    重置 fg/bg"
            } else {
                "acc    sec    trt     Reset fg/bg"
            },
            dim,
        )]),
        Line::from(vec![
            Span::styled(
                "[OK] ",
                if show_color {
                    theme.style(theme.success).add_modifier(Modifier::BOLD)
                } else {
                    active_st
                },
            ),
            Span::styled(
                "[WARN] ",
                if show_color {
                    theme.style(theme.warning).add_modifier(Modifier::BOLD)
                } else {
                    dim
                },
            ),
            Span::styled(
                "[ERR] ",
                if show_color {
                    theme.style(theme.error).add_modifier(Modifier::BOLD)
                } else {
                    dim
                },
            ),
            Span::styled(
                if show_color {
                    if zh { "全彩 + ANSI" } else { "RGB + ANSI" }
                } else if zh {
                    "纯单色模式"
                } else {
                    "Pure Mono "
                },
                theme.style(theme.text),
            ),
        ]),
        Line::from(Span::styled(
            if show_color {
                if zh {
                    "主题渐变与语义状态标签"
                } else {
                    "Theme gradients & semantic pills"
                }
            } else if zh {
                "已剥离全部 SGR 颜色属性"
            } else {
                "All SGR color attributes stripped"
            },
            dim,
        )),
    ]
}

fn config_art_language(
    active: Language,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let en_active = active == Language::En;
    let en_st = if en_active { active_st } else { dim };
    let zh_st = if !en_active { active_st } else { dim };
    let en_title = if theme.ascii {
        if en_active {
            " * English (en) * "
        } else {
            "   English (en)   "
        }
    } else if en_active {
        " ● English (en) "
    } else {
        " ○ English (en) "
    };
    let zh_title = if theme.ascii {
        if !en_active {
            " * Chinese (zh) * "
        } else {
            "   Chinese (zh)   "
        }
    } else if !en_active {
        " ● 中文 (zh) "
    } else {
        " ○ 中文 (zh) "
    };

    if art_h >= 8 && art_w >= 48 {
        let hint_line = |text: &str, active_card: bool| -> Line<'static> {
            let base = if active_card {
                theme.style(theme.text)
            } else {
                dim
            };
            styled_key_hint_line(text, base, theme)
        };
        let en_body = vec![
            Line::from(Span::styled(
                theme.adapt(" Resume · New · Kill"),
                if en_active {
                    theme.style(theme.text)
                } else {
                    dim
                },
            )),
            Line::from(Span::styled(theme.adapt(" Search: filter…"), dim)),
            hint_line(" Tab | / Commands", en_active),
            hint_line(" Ctrl+T Theme", en_active),
            Line::from(Span::styled(" Default UI language", dim)),
            Line::from(Span::styled(" language = \"en\"", en_st)),
        ];
        let zh_body = if theme.ascii {
            vec![
                Line::from(Span::styled(" HuiFu | MingMing", dim)),
                Line::from(Span::styled(" SouSuo: filter...", dim)),
                hint_line(" Tab Mode | / Cmd", !en_active),
                hint_line(" Ctrl+T Theme", !en_active),
                Line::from(Span::styled(" Set ascii=false UTF8", dim)),
                Line::from(Span::styled(" language = \"zh\"", zh_st)),
            ]
        } else {
            vec![
                Line::from(Span::styled(
                    " 恢复·重命名·关闭",
                    if !en_active {
                        theme.style(theme.text)
                    } else {
                        dim
                    },
                )),
                Line::from(Span::styled(" 搜索: 过滤会话…", dim)),
                hint_line(" Tab 切换 · / 命令", !en_active),
                hint_line(" Ctrl+T 主题", !en_active),
                Line::from(Span::styled(" 全部菜单与帮助", dim)),
                Line::from(Span::styled(" language = \"zh\"", zh_st)),
            ]
        };
        let left_box = build_art_box(
            23,
            Line::from(Span::styled(en_title, en_st)),
            en_body,
            None,
            en_st,
            theme.ascii,
        );
        let right_box = build_art_box(
            23,
            Line::from(Span::styled(zh_title, zh_st)),
            zh_body,
            None,
            zh_st,
            theme.ascii,
        );
        return zip_art_columns(left_box, 23, 2, right_box);
    }

    let left_box = build_art_box(
        18,
        Line::from(Span::styled(
            if en_active {
                if theme.ascii { "* en" } else { "● en" }
            } else if theme.ascii {
                "  en"
            } else {
                "○ en"
            },
            en_st,
        )),
        vec![
            Line::from(Span::styled(" English (en)", en_st)),
            Line::from(Span::styled(" Default", dim)),
        ],
        None,
        en_st,
        theme.ascii,
    );
    let right_box = build_art_box(
        18,
        Line::from(Span::styled(
            if !en_active {
                if theme.ascii { "* zh" } else { "● zh" }
            } else if theme.ascii {
                "  zh"
            } else {
                "○ zh"
            },
            zh_st,
        )),
        vec![
            Line::from(Span::styled(
                if theme.ascii {
                    " Chinese (zh)"
                } else {
                    " 中文 (zh)"
                },
                zh_st,
            )),
            Line::from(Span::styled(
                if theme.ascii {
                    " Simplified"
                } else {
                    " 简体中文"
                },
                dim,
            )),
        ],
        None,
        zh_st,
        theme.ascii,
    );
    zip_art_columns(left_box, 18, 2, right_box)
}

fn config_art_tmux_popup(
    enabled: bool,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let active_st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let ok_st = theme.style(theme.success).add_modifier(Modifier::BOLD);
    let dim = theme.style(theme.muted);
    let text_st = theme.style(theme.text);
    let zh = theme.language.is_zh() && !theme.ascii;

    if art_h >= 8 && art_w >= 48 {
        let popup_border = if enabled { active_st } else { dim };
        let popup_title = if enabled {
            if zh {
                " * JustMux 浮窗 (Ctrl+b j) * "
            } else {
                " * JustMux Popup (Ctrl+b j) * "
            }
        } else if zh {
            " o 按 Space 绑定 Ctrl+b j "
        } else {
            " o Press Space to bind Ctrl+b j "
        };
        let popup_line_0 = if enabled {
            if zh {
                "  ● ~/.tmux.conf 已绑定 [Ctrl+b j]"
            } else if theme.ascii {
                "  * ~/.tmux.conf  [Ctrl+b j]"
            } else {
                "  ● ~/.tmux.conf  [Ctrl+b j]"
            }
        } else if zh {
            "  ○ ~/.tmux.conf 尚未写入绑定"
        } else if theme.ascii {
            "  o ~/.tmux.conf  (not bound)"
        } else {
            "  ○ ~/.tmux.conf  (not bound)"
        };
        let popup_line_1 = if enabled {
            if zh {
                "  ▸ 终端与 agy 内直接浮窗切换"
            } else if theme.ascii {
                "  > Works in shell & agy"
            } else {
                "  ▸ Works in shell & agy"
            }
        } else if zh {
            "  ▸ 开启前不会改动 ~/.tmux.conf"
        } else if theme.ascii {
            "  > Keeps ~/.tmux.conf untouched"
        } else {
            "  ▸ Keeps ~/.tmux.conf untouched"
        };
        let inner_popup = build_art_box(
            40,
            Line::from(styled_key_hint_spans(popup_title, popup_border, theme)),
            vec![
                Line::from(styled_key_hint_spans(
                    popup_line_0,
                    if enabled { text_st } else { dim },
                    theme,
                )),
                Line::from(Span::styled(
                    theme.adapt(popup_line_1),
                    if enabled { ok_st } else { dim },
                )),
            ],
            None,
            popup_border,
            theme.ascii,
        );

        let mut outer_body = Vec::with_capacity(6);
        outer_body.push(Line::from(Span::styled(
            if zh {
                " $ agy / 终端  (当前位于 tmux 窗格)"
            } else {
                " $ agy / zsh  (inside tmux pane)"
            },
            dim,
        )));
        for mut row in inner_popup {
            row.spans.insert(0, Span::raw("   "));
            outer_body.push(row);
        }
        outer_body.push(Line::from(Span::styled(
            theme.adapt(if enabled {
                if zh {
                    " [0] agy*  1:zsh       ✓ 已热加载生效"
                } else if theme.ascii {
                    " [0] agy*  1:zsh      * live reloaded"
                } else {
                    " [0] agy*  1:zsh      ✓ live reloaded"
                }
            } else if zh {
                " [0] agy*  1:zsh         未绑定快捷键"
            } else {
                " [0] agy*  1:zsh      Prefix+j inactive"
            }),
            if enabled { ok_st } else { dim },
        )));

        let outer_title = if enabled {
            if zh {
                " * tmux 全局唤出已开启 * "
            } else {
                " * tmux + Ctrl+b j ON * "
            }
        } else if zh {
            " o tmux 浮窗未开启 "
        } else {
            " o tmux Popup OFF "
        };
        let outer_bottom = if enabled {
            if zh {
                " ~/.tmux.conf 已同步 "
            } else {
                " ~/.tmux.conf synced "
            }
        } else if zh {
            " ~/.tmux.conf 未修改 "
        } else {
            " ~/.tmux.conf clean "
        };
        return build_art_box(
            48,
            Line::from(styled_key_hint_spans(
                outer_title,
                if enabled { active_st } else { dim },
                theme,
            )),
            outer_body,
            Some(Line::from(Span::styled(outer_bottom, dim))),
            if enabled {
                theme.style(theme.secondary)
            } else {
                dim
            },
            theme.ascii,
        );
    }

    let head = if enabled {
        if zh {
            " [* Ctrl+b j 已开启] ~/.tmux.conf"
        } else {
            " [* Ctrl+b j ON ] ~/.tmux.conf"
        }
    } else if zh {
        " [o Ctrl+b j 未开启] ~/.tmux.conf"
    } else {
        " [o Ctrl+b j OFF] ~/.tmux.conf"
    };
    let box_title = if enabled {
        if zh {
            " JustMux 浮窗 "
        } else {
            " JustMux Popup "
        }
    } else if zh {
        " 尚未绑定快捷键 "
    } else {
        " Not Bound Yet "
    };
    let box_mid = if enabled {
        if zh {
            " ▸ 终端 / agy 按 Ctrl+b j"
        } else {
            " > shell & agy -> Ctrl+b j"
        }
    } else if zh {
        " ○ 按 Space 写入 .tmux.conf"
    } else {
        " o Space -> write .tmux.conf"
    };
    let boxed = build_art_box(
        34,
        Line::from(Span::styled(
            box_title,
            if enabled { active_st } else { dim },
        )),
        vec![Line::from(styled_key_hint_spans(
            box_mid,
            if enabled { text_st } else { dim },
            theme,
        ))],
        None,
        if enabled { active_st } else { dim },
        theme.ascii,
    );
    let foot = if enabled {
        if zh {
            " * 热加载生效 (终端 / agy 均可)"
        } else {
            " * live reload active (shell & agy)"
        }
    } else if zh {
        " o 按 Space 启用全局浮窗快捷键"
    } else {
        " o toggle with Space to enable"
    };

    let mut lines = Vec::with_capacity(5);
    lines.push(Line::from(styled_key_hint_spans(
        head,
        if enabled { active_st } else { dim },
        theme,
    )));
    for mut row in boxed {
        row.spans.insert(0, Span::raw(" "));
        lines.push(row);
    }
    lines.push(Line::from(Span::styled(
        foot,
        if enabled { ok_st } else { dim },
    )));
    lines
}

fn config_art_generic(key: &str, value: &str, theme: Theme) -> Vec<Line<'static>> {
    let b = theme.style(theme.secondary);
    let (tl, tr, bl, br, hz, vt) = if theme.ascii {
        ("+", "+", "+", "+", "----------------------------", "|")
    } else {
        ("╭", "╮", "╰", "╯", "────────────────────────────", "│")
    };
    let line = truncate_to_width(&format!(" {key} = {value}"), 28);
    let pad = " ".repeat(28usize.saturating_sub(line.width()));
    vec![
        Line::from(Span::styled(format!("{tl}{hz}{tr}"), b)),
        Line::from(vec![
            Span::styled(vt, b),
            Span::styled(line, theme.style(theme.accent).add_modifier(Modifier::BOLD)),
            Span::raw(pad),
            Span::styled(vt, b),
        ]),
        Line::from(Span::styled(format!("{bl}{hz}{br}"), b)),
    ]
}

#[allow(clippy::too_many_arguments)]
fn render_subview_theme(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    presets: &[ThemePreset],
    selected: usize,
    active: ThemePreset,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let clamped = if presets.is_empty() {
        0
    } else {
        selected.min(presets.len() - 1)
    };
    let preview_preset = presets.get(clamped).copied().unwrap_or(active);

    let render_upper = |frame: &mut Frame<'_>, area: Rect| {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [title_area, sample_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let bg = theme.pill_bg();
        let mut title_line = Line::from(gradient_spans(
            theme.tr("Theme palette", "主题调色板"),
            theme,
            true,
        ));
        let active_pill = pill_span(
            format!(" {} ", active.name()),
            theme.accent,
            bg,
            theme,
            false,
        );
        if title_line.width() + 2 + active_pill.width() <= usize::from(title_area.width) {
            title_line.spans.push(Span::raw("  "));
            title_line.spans.push(active_pill);
        }
        frame.render_widget(Paragraph::new(title_line), title_area);
        if sample_area.height > 0 {
            render_theme_live_preview(frame, sample_area, preview_preset, theme);
        }
    };

    let render_list = |frame: &mut Frame<'_>, area: Rect| {
        if presets.is_empty() {
            frame.render_widget(
                Paragraph::new(theme.tr("No themes available", "暂无可用主题"))
                    .style(theme.style(theme.muted)),
                area,
            );
            return;
        }
        let list_items = build_theme_list_items(presets, clamped, active, area.width, false, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(list_items), area, &mut state);
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 4, true);
        render_upper(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        render_list(frame, left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            list_rect: Some(left_band),
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let [upper_band, rule_area, list_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(card_inner);
    render_upper(frame, upper_band);
    render_horizontal_divider_line(frame, rule_area, theme);
    render_list(frame, list_area);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        ..WorkspaceSubviewLayout::default()
    }
}

fn render_subview_help(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let cmd_style = theme.style(theme.success).add_modifier(Modifier::BOLD);
    let sec_style = theme.style(theme.secondary).add_modifier(Modifier::BOLD);
    let accent_style = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let muted = theme.style(theme.muted);
    let text_style = theme.style(theme.text);
    let zh = theme.language.is_zh() && !theme.ascii;

    let command_rows = if zh {
        [
            (
                "/resume",
                "打开工作区矩阵连接会话",
                "/pane",
                "检查、缩放、拆出并关闭窗格",
            ),
            (
                "/new",
                "新建并启动 tmux 会话",
                "/split",
                "2x2 可视化分屏工作台",
            ),
            (
                "/window",
                "管理、移动与链接窗口",
                "/layout",
                "视觉布局预设与实时调大小",
            ),
            (
                "/rename",
                "重命名当前会话或窗口",
                "/expose",
                "实时多窗格全景任务墙",
            ),
            (
                "/kill",
                "关闭指定会话或窗口",
                "/buffers",
                "浏览、复制与粘贴缓冲区",
            ),
            (
                "/snapshot",
                "保存与恢复工作区快照模板",
                "/popup",
                "弹出临时浮动终端 (3.2+)",
            ),
            (
                "/list",
                "会话与窗口全局概览",
                "/clients",
                "管理客户端与修复虚线框",
            ),
            (
                "/exit",
                "仅分离当前 tmux 客户端",
                "/theme",
                "实时预览并切换主题配色",
            ),
            (
                "/kill-server",
                "停止整个 tmux 服务",
                "/config",
                "设置与保存 config.toml",
            ),
            ("/quit", "退出 JustMux", "/help", "命令与快捷键速查指南"),
        ]
    } else {
        [
            (
                "/resume",
                "open window in matrix",
                "/pane",
                "inspect, zoom, break, close",
            ),
            (
                "/new",
                "create tmux session",
                "/split",
                "2x2 visual Split Studio",
            ),
            (
                "/window",
                "manage, move & link windows",
                "/layout",
                "layout presets & pane resize",
            ),
            (
                "/rename",
                "rename session or window",
                "/expose",
                "live Mission Control wall",
            ),
            (
                "/kill",
                "kill session or window",
                "/buffers",
                "browse, copy & paste buffers",
            ),
            (
                "/snapshot",
                "save & restore workspaces",
                "/popup",
                "floating scratchpad (3.2+)",
            ),
            (
                "/list",
                "session & window overview",
                "/clients",
                "clients & fix dotted border",
            ),
            (
                "/exit",
                "detach current tmux client",
                "/theme",
                "live-preview color palettes",
            ),
            (
                "/kill-server",
                "stop entire tmux server",
                "/config",
                "settings & save config.toml",
            ),
            (
                "/quit",
                "quit JustMux",
                "/help",
                "commands & shortcuts guide",
            ),
        ]
    };

    let build_command_lines = |width: u16| -> Vec<Line<'static>> {
        let icon = if theme.ascii { "[] " } else { "◫ " };
        let mut header = vec![Span::styled(icon, accent_style)];
        header.extend(gradient_spans(
            if zh {
                "JustMux 命令大全"
            } else {
                "JustMux commands"
            },
            theme,
            true,
        ));
        header.push(Span::styled(
            theme.adapt(if zh {
                "  ·  工作区与工作台命令分组"
            } else {
                "  ·  grouped workspace & studio"
            }),
            muted,
        ));
        let mut lines = vec![Line::from(header)];
        if width >= 72 {
            let ws_label = if zh {
                "工作区与退出"
            } else {
                "Workspace & Exit"
            };
            let st_label = if zh {
                "工作台与系统"
            } else {
                "Studio & System"
            };
            let left_col_w = 39_usize;
            let ws_pad = left_col_w.saturating_sub(ws_label.width());
            lines.push(Line::from(vec![
                Span::styled(format!("{ws_label}{}", " ".repeat(ws_pad)), sec_style),
                Span::styled(st_label, sec_style),
            ]));
            for (lc, ld, rc, rd) in command_rows {
                let lpad = 10_usize.saturating_sub(lc.width()).max(1);
                let mid = left_col_w
                    .saturating_sub(lc.width() + lpad + ld.width())
                    .max(2);
                let rpad = 10_usize.saturating_sub(rc.width()).max(1);
                lines.push(Line::from(vec![
                    Span::styled(lc, cmd_style),
                    Span::raw(" ".repeat(lpad)),
                    Span::styled(ld, text_style),
                    Span::raw(" ".repeat(mid)),
                    Span::styled(rc, cmd_style),
                    Span::raw(" ".repeat(rpad)),
                    Span::styled(rd, text_style),
                ]));
            }
        } else {
            for (lc, ld, rc, _) in command_rows {
                let pad = 10_usize.saturating_sub(lc.width()).max(1);
                lines.push(Line::from(vec![
                    Span::styled(lc, cmd_style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(
                        truncate_to_width(
                            &format!("{ld} · {rc}"),
                            usize::from(width).saturating_sub(10),
                        ),
                        text_style,
                    ),
                ]));
            }
        }
        lines.push(Line::from(vec![
            Span::styled(if zh { "致敬彩蛋  " } else { "Tributes  " }, muted),
            Span::styled(
                theme.adapt("/codex · /claude · /antigravity · /vibe"),
                theme.style(theme.accent),
            ),
        ]));
        lines
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = if card_inner.height >= 22 {
            let lower_h = card_inner.height.saturating_sub(14).clamp(8, 12);
            let [u, r, l] = Layout::vertical([
                Constraint::Min(13),
                Constraint::Length(1),
                Constraint::Length(lower_h),
            ])
            .areas(card_inner);
            (u, r, l)
        } else {
            split_subview_bands(card_inner, 10, true)
        };
        frame.render_widget(
            Paragraph::new(build_command_lines(upper_band.width)),
            upper_band,
        );
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let nav_rows = if zh {
            [
                (
                    "Tab        ",
                    "切换 恢复/改名/关闭/分屏 标签 (或 会话/窗口)",
                ),
                ("PgUp / PgDn", "滚动右侧窗格预览历史 (或使用鼠标滚轮)"),
                ("f / y / [] ", "预览搜索 · 复制输出 · 切换同窗口多窗格"),
                ("↑↓ / ←→    ", "在会话列与窗口行之间上下左右导航"),
                ("1-9        ", "快速附着或跳转到对应编号的工作区"),
                ("Shift+Tab  ", "切换 命令跳转 · 实时雷达 · 快速启动 模式"),
                ("Ctrl+T     ", "轮换真彩主题 · Esc 返回上级 / 退出"),
            ]
        } else {
            [
                (
                    "Tab        ",
                    "cycle Resume/Rename/Kill/Split tabs (or Session/Window)",
                ),
                (
                    "PgUp / PgDn",
                    "scroll Window Preview history (or Mouse Wheel)",
                ),
                (
                    "f / y / [] ",
                    "search preview · copy preview · switch panes in window",
                ),
                ("↑↓ / ←→    ", "navigate windows & session columns"),
                ("1-9        ", "quick-attach or jump to numbered workspace"),
                (
                    "Shift+Tab  ",
                    "cycle Command & @Jump · Live Radar · Quick Launch",
                ),
                (
                    "Ctrl+T     ",
                    "cycle TrueColor themes · Esc backs up / quits",
                ),
            ]
        };
        let mut nav_lines = vec![Line::from(Span::styled(
            theme.adapt(if zh {
                "◈ 工作区快捷键与导航"
            } else {
                "◈ Workspace Shortcuts & Navigation"
            }),
            sec_style,
        ))];
        for (k, desc) in nav_rows {
            if nav_lines.len() >= usize::from(left_band.height) {
                break;
            }
            let adapted_k = theme.adapt(k.trim_end());
            let pad = 11_usize.saturating_sub(adapted_k.width()) + 1;
            let avail = usize::from(left_band.width).saturating_sub(adapted_k.width() + pad);
            let mut row_spans = styled_key_hint_spans(&adapted_k, muted, theme);
            row_spans.push(Span::raw(" ".repeat(pad)));
            row_spans.extend(styled_key_hint_spans(
                &truncate_to_width(&theme.adapt(desc), avail),
                text_style,
                theme,
            ));
            nav_lines.push(Line::from(row_spans));
        }
        frame.render_widget(Paragraph::new(nav_lines), left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let mut lines = build_command_lines(card_inner.width);
    if usize::from(card_inner.height) > lines.len() {
        let mut key_spans = vec![Span::styled(
            if zh { "快捷键    " } else { "Keys      " },
            muted,
        )];
        key_spans.extend(styled_key_hint_spans(
            &truncate_to_width(
                &theme.adapt(if zh {
                    "Tab 切换操作 · f 搜索 · y 复制 · PgUp / PgDn 滚动预览 · Esc 返回"
                } else {
                    "Tab actions · f search · y copy · PgUp / PgDn preview scroll · Esc back"
                }),
                usize::from(card_inner.width).saturating_sub(10),
            ),
            muted,
            theme,
        ));
        lines.push(Line::from(key_spans));
    }
    frame.render_widget(Paragraph::new(lines), card_inner);
    WorkspaceSubviewLayout::default()
}

#[allow(clippy::too_many_arguments)]
fn render_subview_picker(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    title: &str,
    rows: &[RichPickerRow],
    selected: usize,
    empty: &str,
    preview: Option<PanePreview<'_>>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let render_list = |frame: &mut Frame<'_>, area: Rect| {
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(theme.adapt(empty)).style(theme.style(theme.muted)),
                area,
            );
            return;
        }
        let clamped = selected.min(rows.len() - 1);
        let items = build_rich_picker_list_items(rows, clamped, area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), area, &mut state);
    };

    let render_header_line = |frame: &mut Frame<'_>, area: Rect| {
        let counter = if rows.is_empty() {
            " [0]".to_owned()
        } else {
            format!(" [{}/{}]", selected.min(rows.len() - 1) + 1, rows.len())
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    theme.adapt(title),
                    theme.style(theme.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(counter, theme.style(theme.muted)),
            ])),
            area,
        );
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 5, true);
        let preview_layout = if let Some(card) = preview {
            render_pane_preview_card_interactive(frame, upper_band, card, theme)
        } else {
            let [h_area, hint_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(upper_band);
            render_header_line(frame, h_area);
            if hint_area.height > 0 {
                frame.render_widget(
                    Paragraph::new(theme.adapt(theme.tr(
                        "Live preview off · use Filter bar below to narrow",
                        "实时预览已关闭 · 请在下方过滤栏输入关键字筛选",
                    )))
                    .style(theme.style(theme.muted)),
                    hint_area,
                );
            }
            PreviewCardLayout::default()
        };
        let preview_area = preview_layout.area;
        let preview_action_hits = preview_layout.action_hits.clone();
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let [count_area, list_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(left_band);
        render_header_line(frame, count_area);
        render_list(frame, list_area);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            list_rect: Some(list_area),
            preview_area,
            hydra_rendered,
            preview_action_hits,
            preview_layout,
            ..WorkspaceSubviewLayout::default()
        };
    }

    if let Some(card) = preview
        && card_inner.height >= 10
        && card_inner.width >= 60
    {
        let [preview_box, list_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(card_inner);
        let pl = render_pane_preview_card_interactive(frame, preview_box, card, theme);
        render_list(frame, list_area);
        let preview_action_hits = pl.action_hits.clone();
        return WorkspaceSubviewLayout {
            list_rect: Some(list_area),
            preview_area: pl.area,
            preview_action_hits,
            preview_layout: pl,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let [head, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(card_inner);
    render_header_line(frame, head);
    render_list(frame, list_area);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        ..WorkspaceSubviewLayout::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn render_subview_form(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    title: &str,
    label: &str,
    input: &TextInput,
    border_color: Color,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    let render_upper = |frame: &mut Frame<'_>, area: Rect| {
        let mut lines = vec![Line::from(Span::styled(
            theme.adapt(title),
            theme.style(border_color).add_modifier(Modifier::BOLD),
        ))];
        lines.push(Line::default());
        for wrapped in wrap_description(&theme.adapt(label), area.width) {
            if usize::from(area.height) <= lines.len() + 1 {
                break;
            }
            lines.push(Line::from(Span::styled(wrapped, theme.style(theme.text))));
        }
        if usize::from(area.height) > lines.len() {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", theme.marker()), theme.style(border_color)),
                Span::styled(
                    truncate_to_width(
                        &single_line(input.value()),
                        usize::from(area.width).saturating_sub(2),
                    ),
                    theme.style(theme.accent).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), area);
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 5, false);
        render_upper(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let tips = Paragraph::new(vec![
            Line::from(Span::styled(
                theme.adapt(if zh {
                    "◈ 输入控制说明"
                } else {
                    "◈ Input Controls"
                }),
                theme.style(theme.secondary).add_modifier(Modifier::BOLD),
            )),
            styled_key_hint_line(
                if zh {
                    "在下方输入框键入内容后按 Enter 提交"
                } else {
                    "Type value in the bar below and press Enter to submit"
                },
                theme.style(theme.text),
                theme,
            ),
            styled_key_hint_line(
                if zh {
                    "编辑快捷键: Ctrl+A/E 移动 · Ctrl+W/U 清空 · Esc 取消"
                } else {
                    "Emacs editing: Ctrl+A/E move · Ctrl+W/U clear · Esc cancel"
                },
                theme.style(theme.muted),
                theme,
            ),
        ]);
        frame.render_widget(tips, left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    render_upper(frame, card_inner);
    WorkspaceSubviewLayout::default()
}

#[allow(clippy::too_many_arguments)]
fn render_subview_confirm(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    target_summary: &str,
    message: &str,
    scroll: u16,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    let render_upper = |frame: &mut Frame<'_>, area: Rect| -> u16 {
        let [banner_area, diff_area, msg_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(area);
        let warn_icon = if theme.ascii { "!" } else { "⚠" };
        let sep = if theme.ascii { " | " } else { " · " };
        let banner = Line::from(vec![
            Span::styled(
                format!(
                    "{warn_icon} {}",
                    if zh {
                        "需要危险操作确认"
                    } else {
                        "Approval Required"
                    }
                ),
                theme.style(theme.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled(sep, theme.style(theme.muted)),
            pill_span(
                if zh {
                    "[环境: 原生 tmux]"
                } else {
                    "[sandbox: native-tmux]"
                },
                theme.secondary,
                theme.pill_bg(),
                theme,
                false,
            ),
        ]);
        frame.render_widget(Paragraph::new(banner), banner_area);
        let diff_text = format!(
            "- {} {}",
            if zh { "关闭" } else { "kill" },
            single_line(target_summary)
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(&theme.adapt(&diff_text), usize::from(diff_area.width)),
                theme.style(theme.error).add_modifier(Modifier::BOLD),
            ))),
            diff_area,
        );
        let lines = wrap_description(&theme.adapt(message), msg_area.width);
        let max_scroll = lines.len().saturating_sub(usize::from(msg_area.height));
        let clamped = scroll.min(u16::try_from(max_scroll).unwrap_or(u16::MAX));
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .style(theme.style(theme.text))
                .scroll((clamped, 0)),
            msg_area,
        );
        clamped
    };

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 6, false);
        let confirm_scroll = render_upper(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        "! 二次确认保护"
                    } else {
                        "! Confirmation Guardrail"
                    }),
                    theme.style(theme.error).add_modifier(Modifier::BOLD),
                )),
                styled_key_hint_line(
                    if zh {
                        "请在下方确认栏输入 yes 后按 Enter 执行"
                    } else {
                        "Type yes in the confirmation prompt below and press Enter"
                    },
                    theme.style(theme.text),
                    theme,
                ),
                styled_key_hint_line(
                    if zh {
                        "↑↓ 滚动详情 · Esc 取消且不会修改 tmux"
                    } else {
                        "↑↓ Scroll message · Esc cancels without changing tmux"
                    },
                    theme.style(theme.muted),
                    theme,
                ),
            ]),
            left_band,
        );
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            confirm_scroll,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let confirm_scroll = render_upper(frame, card_inner);
    WorkspaceSubviewLayout {
        confirm_scroll,
        ..WorkspaceSubviewLayout::default()
    }
}

fn render_actions_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    target_label: &str,
    actions: &[&str],
    selected: usize,
    theme: Theme,
) -> Option<Rect> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let [title_area, target_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(area.height >= actions.len() as u16 + 3)),
        Constraint::Min(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(theme.adapt(title))
            .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        title_area,
    );
    if target_area.height > 0 {
        frame.render_widget(
            Paragraph::new(single_line(target_label)).style(theme.style(theme.muted)),
            target_area,
        );
    }
    let list_items: Vec<ListItem<'_>> = actions
        .iter()
        .enumerate()
        .map(|(idx, label)| {
            let is_sel = idx == selected;
            let style = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            ListItem::new(Line::from(Span::styled(
                format!("{marker} {}", theme.adapt(label)),
                style,
            )))
        })
        .collect();
    let mut state = ListState::default();
    if !actions.is_empty() {
        state.select(Some(selected.min(actions.len() - 1)));
    }
    frame.render_stateful_widget(List::new(list_items), list_area, &mut state);
    Some(list_area)
}

#[allow(clippy::too_many_arguments)]
fn render_subview_actions(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    title: &str,
    target_label: &str,
    actions: &[&str],
    selected: usize,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 4, true);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    theme.adapt(title),
                    theme.style(theme.secondary).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    single_line(target_label),
                    theme.style(theme.accent).add_modifier(Modifier::BOLD),
                )),
                styled_key_hint_line(
                    if zh {
                        "在下方选择要执行的操作 · Enter 执行 · Esc 返回"
                    } else {
                        "Choose an operation below · Enter Execute · Esc Back"
                    },
                    theme.style(theme.muted),
                    theme,
                ),
            ]),
            upper_band,
        );
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let list_items: Vec<ListItem<'_>> = actions
            .iter()
            .enumerate()
            .map(|(idx, label)| {
                let is_sel = idx == selected;
                let style = if is_sel {
                    theme.selected_style()
                } else {
                    theme.style(theme.text)
                };
                let marker = if is_sel { theme.marker() } else { " " };
                ListItem::new(Line::from(Span::styled(
                    format!("{marker} {}", theme.adapt(label)),
                    style,
                )))
            })
            .collect();
        let mut state = ListState::default();
        if !actions.is_empty() {
            state.select(Some(selected.min(actions.len() - 1)));
        }
        frame.render_stateful_widget(List::new(list_items), left_band, &mut state);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            list_rect: Some(left_band),
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let list_rect = render_actions_list_panel(
        frame,
        card_inner,
        title,
        target_label,
        actions,
        selected,
        theme,
    );
    WorkspaceSubviewLayout {
        list_rect,
        ..WorkspaceSubviewLayout::default()
    }
}

fn render_subview_created(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    target_label: &str,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    let lines = vec![
        Line::from(Span::styled(
            if theme.ascii {
                "* Created successfully"
            } else if zh {
                "✓ 创建成功"
            } else {
                "✓ Created successfully"
            },
            theme.style(theme.success).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            single_line(target_label),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        styled_key_hint_line(
            if zh {
                "按 Enter 立即进入原生 tmux · Esc 返回继续管理"
            } else {
                "Enter to enter native tmux · Esc to continue managing"
            },
            theme.style(theme.text),
            theme,
        ),
    ];
    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 5, false);
        frame.render_widget(Paragraph::new(lines), upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    theme.adapt(if zh {
                        "◈ 工作区已就绪"
                    } else {
                        "◈ Workspace Ready"
                    }),
                    theme.style(theme.secondary).add_modifier(Modifier::BOLD),
                )),
                styled_key_hint_line(
                    if zh {
                        "按 Enter 立即附着进入，或按 Esc 继续编排管理"
                    } else {
                        "Press Enter to attach immediately, or Esc to keep orchestrating"
                    },
                    theme.style(theme.muted),
                    theme,
                ),
            ]),
            left_band,
        );
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }
    frame.render_widget(Paragraph::new(lines), card_inner);
    WorkspaceSubviewLayout::default()
}

fn render_subview_tribute(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    modal: &TributeModal<'_>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let render_upper_card = |frame: &mut Frame<'_>, area: Rect| {
        let mut lines = Vec::new();
        let mut title_line = Line::from(gradient_spans(modal.title, theme, true));
        let subtitle = theme.adapt(modal.subtitle);
        let inline_subtitle = !subtitle.is_empty()
            && title_line.width() + 2 + subtitle.width() <= usize::from(area.width);
        if inline_subtitle {
            title_line.spans.push(Span::raw("  "));
            title_line
                .spans
                .push(Span::styled(subtitle.clone(), theme.style(theme.muted)));
        }
        lines.push(title_line);
        if !inline_subtitle && !subtitle.is_empty() {
            lines.push(Line::from(Span::styled(
                truncate_to_width(&subtitle, usize::from(area.width)),
                theme.style(theme.muted),
            )));
        }
        if !modal.telemetry.is_empty() {
            lines.push(Line::from(Span::styled(
                truncate_to_width(&theme.adapt(modal.telemetry), usize::from(area.width)),
                theme.style(theme.secondary),
            )));
        }
        if usize::from(area.height) > lines.len() + modal.art.len() {
            lines.push(Line::default());
        }
        for art_line in modal.art {
            if lines.len() >= usize::from(area.height) {
                break;
            }
            lines.push(Line::from(Span::styled(
                theme.adapt(art_line),
                theme.style(theme.accent),
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    };

    if card_inner.height >= 9 {
        let desired_upper = u16::try_from(modal.art.len() + 3)
            .unwrap_or(8)
            .max(4)
            .min(card_inner.height.saturating_sub(3));
        let [upper_band, rule_area, lower_band] = Layout::vertical([
            Constraint::Length(desired_upper),
            Constraint::Length(1),
            Constraint::Min(2),
        ])
        .areas(card_inner);
        render_upper_card(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let bullet = if theme.ascii { "- " } else { "• " };
        let reserve_quip = usize::from(!modal.status_quip.is_empty());
        let mut lower_lines = Vec::new();
        for highlight in modal.highlights {
            if lower_lines.len() + reserve_quip >= usize::from(left_band.height) {
                break;
            }
            let avail = usize::from(left_band.width).saturating_sub(bullet.width());
            lower_lines.push(Line::from(vec![
                Span::styled(bullet, theme.style(theme.accent)),
                Span::styled(
                    truncate_to_width(&theme.adapt(highlight), avail),
                    theme.style(theme.text),
                ),
            ]));
        }
        if !modal.status_quip.is_empty() && lower_lines.len() < usize::from(left_band.height) {
            if lower_lines.len() + 2 <= usize::from(left_band.height) {
                lower_lines.push(Line::default());
            }
            lower_lines.push(Line::from(Span::styled(
                truncate_to_width(
                    &theme.adapt(modal.status_quip),
                    usize::from(left_band.width),
                ),
                theme.style(theme.success).add_modifier(Modifier::BOLD),
            )));
        }
        frame.render_widget(Paragraph::new(lower_lines), left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            ..WorkspaceSubviewLayout::default()
        };
    }

    render_upper_card(frame, card_inner);
    WorkspaceSubviewLayout::default()
}

fn render_subview_bottom_composer(
    frame: &mut Frame<'_>,
    composer: Rect,
    footer: Rect,
    spec: &WorkspaceSubviewSpec<'_>,
    border_color: Color,
    theme: Theme,
) {
    let zh = theme.language.is_zh() && !theme.ascii;
    let right_hint =
        (composer.width >= 48).then_some(if zh { " Esc 返回 " } else { " Esc Back " });
    let default_hint = spec.content.default_shortcut_hint_for(theme);
    let active_hint = if spec.footer_hint.is_empty() {
        default_hint
    } else {
        spec.footer_hint
    };

    match &spec.content {
        SubviewContent::Picker { title, query, .. }
        | SubviewContent::TargetPicker { title, query, .. } => {
            let filter_suffix = if zh { "过滤" } else { "Filter" };
            let left_title = format!(
                "{} {} · {filter_suffix}",
                if theme.ascii { ">" } else { "▸" },
                theme.adapt(title)
            );
            let picker_right = if composer.width >= 68 {
                Some(if theme.ascii {
                    " Up/Down Select | PgUp/PgDn Preview | Esc Back "
                } else if zh {
                    " ↑↓ 选择 · PgUp/PgDn 预览 · Esc 返回 "
                } else {
                    " ↑↓ Select · PgUp/PgDn Preview · Esc Back "
                })
            } else {
                right_hint
            };
            let input_area = render_bordered_composer_box(
                frame,
                composer,
                border_color,
                &left_title,
                picker_right,
                theme,
            );
            let prefix = if zh { "过滤: " } else { "Filter: " };
            let mut line = query.rendered_width(
                true,
                theme,
                input_area.width.saturating_sub(prefix.width() as u16),
            );
            if query.value().is_empty() && input_area.width >= 32 {
                let ghost = theme.adapt(if zh {
                    "输入关键字筛选…"
                } else {
                    "type to narrow…"
                });
                if prefix.width() + line.width() + ghost.width() <= usize::from(input_area.width) {
                    line.spans
                        .push(Span::styled(ghost, theme.style(theme.muted)));
                }
            }
            line.spans
                .insert(0, Span::styled(prefix, theme.style(theme.muted)));
            frame.render_widget(Paragraph::new(line), input_area);
        }
        SubviewContent::Form { title, input, .. } => {
            let left_title = format!(
                "{} {}",
                if theme.ascii { ">" } else { "▸" },
                theme.adapt(title)
            );
            let form_right = (composer.width >= 52).then_some(if theme.ascii {
                " Enter Submit | Esc Back "
            } else if zh {
                " Enter 提交 · Esc 返回 "
            } else {
                " Enter Submit · Esc Back "
            });
            let input_area = render_bordered_composer_box(
                frame,
                composer,
                border_color,
                &left_title,
                form_right,
                theme,
            );
            render_composer_line(
                frame,
                input_area,
                input,
                ComposerMode::CommandJump,
                border_color,
                false,
                theme,
            );
        }
        SubviewContent::DualForm {
            title,
            first_label,
            first_input,
            second_label,
            second_input,
            active_field,
        } => {
            let (field_label, active_input) = if *active_field == 0 {
                (*first_label, *first_input)
            } else {
                (*second_label, *second_input)
            };
            let left_title = format!(
                "{} {} · {}",
                if theme.ascii { ">" } else { "▸" },
                theme.adapt(title),
                theme.adapt(field_label)
            );
            let form_right = (composer.width >= 56).then_some(if theme.ascii {
                " Tab Next | Enter Submit | Esc Back "
            } else if zh {
                " Tab 切换栏位 · Enter 提交 · Esc 返回 "
            } else {
                " Tab Next · Enter Submit · Esc Back "
            });
            let input_area = render_bordered_composer_box(
                frame,
                composer,
                border_color,
                &left_title,
                form_right,
                theme,
            );
            render_composer_line(
                frame,
                input_area,
                active_input,
                ComposerMode::CommandJump,
                border_color,
                false,
                theme,
            );
        }
        SubviewContent::Confirm { title, input, .. } => {
            let left_title = format!("! {}", theme.adapt(title));
            let confirm_right = (composer.width >= 54).then_some(if theme.ascii {
                " Type yes + Enter | Esc Back "
            } else if zh {
                " 输入 yes + Enter · Esc 返回 "
            } else {
                " Type yes + Enter · Esc Back "
            });
            let input_area = render_bordered_composer_box(
                frame,
                composer,
                border_color,
                &left_title,
                confirm_right,
                theme,
            );
            let marker = format!("{} ", theme.marker());
            let mut line = input.rendered_width(
                true,
                theme,
                input_area.width.saturating_sub(marker.width() as u16),
            );
            if input.value().is_empty() && input_area.width >= 30 {
                let ghost = if zh {
                    "输入 yes 以确认"
                } else {
                    "type yes to confirm"
                };
                if marker.width() + line.width() + ghost.width() <= usize::from(input_area.width) {
                    line.spans
                        .push(Span::styled(ghost, theme.style(theme.muted)));
                }
            }
            line.spans.insert(
                0,
                Span::styled(
                    marker,
                    theme.style(theme.error).add_modifier(Modifier::BOLD),
                ),
            );
            frame.render_widget(Paragraph::new(line), input_area);
        }
        _ => {
            let left_title = if theme.ascii {
                "* Shortcuts"
            } else if zh {
                "✦ 快捷键"
            } else {
                "✦ Shortcuts"
            };
            let input_area = render_bordered_composer_box(
                frame,
                composer,
                border_color,
                left_title,
                right_hint,
                theme,
            );
            frame.render_widget(
                Paragraph::new(styled_key_hint_line(
                    &truncate_to_width(&theme.adapt(active_hint), usize::from(input_area.width)),
                    theme.style(theme.text),
                    theme,
                )),
                input_area,
            );
        }
    }

    let variants: [&str; 3] = if zh {
        [
            "Esc 返回 · Ctrl+T 主题 · Ctrl+C 退出",
            "Esc 返回 · ? 帮助",
            "Esc 返回",
        ]
    } else {
        [
            "Esc Back · Ctrl+T Theme · Ctrl+C Quit",
            "Esc Back · ? Help",
            "Esc Back",
        ]
    };
    render_footer_variants(frame, footer, spec.details.status, &variants, theme);
}

fn build_home_card_title_prefix(
    card_width: u16,
    max_title_width: usize,
    telemetry: HomeTelemetry<'_>,
    leading_pad: bool,
    theme: Theme,
) -> Vec<Span<'static>> {
    let bold_accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let muted = theme.style(theme.muted);
    let brand = match (leading_pad, theme.ascii) {
        (true, true) => " [] JustMux",
        (true, false) => " ◫ JustMux",
        (false, true) => "[] JustMux",
        (false, false) => "◫ JustMux",
    };
    let mut title_spans = vec![Span::styled(brand, bold_accent)];
    let version = format!(" (v{})", env!("CARGO_PKG_VERSION"));
    let used_width = |spans: &[Span<'_>]| spans.iter().map(Span::width).sum::<usize>();
    if used_width(&title_spans) + version.width() <= max_title_width {
        title_spans.push(Span::styled(version, muted));
    }
    if card_width >= 54 {
        let bg = theme.pill_bg();
        if let Some(badge) = telemetry.hint_badge.filter(|b| !b.is_empty()) {
            let pill = pill_span(format!(" {} ", badge), theme.accent, bg, theme, false);
            if used_width(&title_spans) + 2 + pill.width() <= max_title_width {
                title_spans.push(Span::raw("  "));
                title_spans.push(pill);
            }
        } else if theme.preset != ThemePreset::Classic {
            let badge = format!(
                " {} {} ",
                theme.preset.badge_icon(theme.ascii),
                theme.preset.slug()
            );
            let pill = pill_span(badge, theme.secondary, bg, theme, false);
            if used_width(&title_spans) + 2 + pill.width() <= max_title_width {
                title_spans.push(Span::raw("  "));
                title_spans.push(pill);
            }
        }
    }
    title_spans
}

fn append_pill_to_title(
    title_spans: &mut Vec<Span<'static>>,
    max_title_width: usize,
    pill_label: &str,
    pill_color: Color,
    theme: Theme,
) {
    let pill = pill_span(
        format!(" {pill_label} "),
        pill_color,
        theme.pill_bg(),
        theme,
        true,
    );
    let used: usize = title_spans.iter().map(Span::width).sum();
    let gap = if used + 2 + pill.width() <= max_title_width {
        2
    } else {
        1
    };
    if used + gap + pill.width() <= max_title_width {
        title_spans.push(Span::raw(" ".repeat(gap)));
        title_spans.push(pill);
    }
}

fn append_workspace_card_meta(
    spans: &mut Vec<Span<'static>>,
    max_title_width: usize,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    theme: Theme,
) {
    let used: usize = spans.iter().map(Span::width).sum();
    let Some(rem) = max_title_width.checked_sub(used + 2) else {
        return;
    };
    if rem < 4 {
        return;
    }

    let muted = theme.style(theme.muted);
    let sep = if theme.ascii { " | " } else { " · " };
    let tele_part = format_telemetry(telemetry, theme);
    let dir_clean = details
        .directory
        .filter(|dir| !dir.is_empty())
        .map(single_line);

    if let Some(dir) = dir_clean {
        let dir_label = if theme.ascii { "dir " } else { "⌂ " };
        let marker = if theme.ascii { "..." } else { "…" };
        let tele_extra = tele_part
            .as_ref()
            .map_or(0, |summary| sep.width() + summary.width());
        let avail_dir = rem.saturating_sub(dir_label.width() + tele_extra).min(26);
        if avail_dir >= 6 {
            let tail = directory_tail(&dir, avail_dir, marker);
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("{dir_label}{tail}"), muted));
            if let Some(summary) = tele_part {
                spans.push(Span::styled(sep, muted));
                spans.push(Span::styled(summary, theme.style(theme.secondary)));
            }
            return;
        }
        let solo_dir = rem.saturating_sub(dir_label.width()).min(26);
        if tele_part.is_none() && solo_dir >= 6 {
            let tail = directory_tail(&dir, solo_dir, marker);
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("{dir_label}{tail}"), muted));
            return;
        }
    }

    if let Some(summary) = tele_part.filter(|summary| summary.width() <= rem) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(summary, theme.style(theme.secondary)));
    } else if !details.context.is_empty() {
        let ctx = format!("tmux {}", single_line(details.context));
        if ctx.width() <= rem {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(ctx, muted));
        }
    }
}

fn render_workspace_preview_card(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: Option<PanePreview<'_>>,
    theme: Theme,
) -> PreviewCardLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    let subtitle = preview.map_or_else(
        || {
            if zh {
                "暂无活跃会话"
            } else {
                "no active session"
            }
            .to_owned()
        },
        |p| {
            if p.subtitle.is_empty() {
                p.title.to_owned()
            } else if p.title.is_empty() {
                p.subtitle.to_owned()
            } else {
                format!("{} · {}", p.title, p.subtitle)
            }
        },
    );
    let base = preview.unwrap_or_default();
    render_pane_preview_card_interactive(
        frame,
        area,
        PanePreview {
            title: if zh {
                "📺 窗口预览"
            } else {
                "📺 Window Preview"
            },
            subtitle: &subtitle,
            show_actions: preview.is_some() && base.show_actions,
            ..base
        },
        theme,
    )
}

fn wrap_tokens(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for token in text.split(' ').filter(|token| !token.is_empty()) {
        if current.is_empty() {
            current.push_str(token);
        } else if current.width() + 1 + token.width() <= max_width {
            current.push(' ');
            current.push_str(token);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(token);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn render_workspace_action_hints(
    frame: &mut Frame<'_>,
    area: Rect,
    action: MatrixAction,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let accent = theme
        .style(action.border_color(theme))
        .add_modifier(Modifier::BOLD);
    let secondary = theme.style(theme.secondary).add_modifier(Modifier::BOLD);
    let muted = theme.style(theme.muted);
    let text = theme.style(theme.text);
    let mut lines = Vec::new();
    if area.height >= 4 {
        lines.push(Line::from(Span::styled(
            truncate_to_width(
                &theme.adapt(if zh {
                    "◈ 工作区操作与工作台"
                } else {
                    "◈ Workspace Actions & Studio"
                }),
                usize::from(area.width),
            ),
            secondary,
        )));
    }
    let hint_rows = if zh {
        [
            (
                "切换窗口操作:   ",
                theme.adapt("/resume (打开) · /rename · /kill · /split · /window"),
                text,
            ),
            (
                "控制器与工作台: ",
                theme.adapt("/layout · /clients · /buffers · /snapshot · /config · /help"),
                muted,
            ),
            (
                "焦点与作用域:   ",
                theme.adapt("←→ 会话 · ↑↓ 窗口 · Tab 作用域 · Enter 执行"),
                muted,
            ),
        ]
    } else {
        [
            (
                "Switch window action: ",
                theme.adapt("/resume (open) · /rename · /kill · /split · /window"),
                text,
            ),
            (
                "Controller & studio:  ",
                theme.adapt("/layout · /clients · /buffers · /snapshot · /config · /help"),
                muted,
            ),
            (
                "Focus & scope:        ",
                theme.adapt("←→ Session · ↑↓ Window · Tab Scope · Enter Act"),
                muted,
            ),
        ]
    };
    let avail_width = usize::from(area.width);
    for (prefix, rest, style) in hint_rows {
        if lines.len() >= usize::from(area.height) {
            break;
        }
        if prefix.width() + rest.width() <= avail_width {
            let mut spans = vec![Span::styled(prefix, accent)];
            spans.extend(styled_key_hint_spans(&rest, style, theme));
            lines.push(Line::from(spans));
        } else if avail_width > prefix.width() + 10 {
            let indent = " ".repeat(prefix.width());
            let wrapped = wrap_tokens(&rest, avail_width - prefix.width());
            for (idx, part) in wrapped.into_iter().enumerate() {
                if lines.len() >= usize::from(area.height) {
                    break;
                }
                if idx == 0 {
                    let mut spans = vec![Span::styled(prefix, accent)];
                    spans.extend(styled_key_hint_spans(&part, style, theme));
                    lines.push(Line::from(spans));
                } else {
                    lines.push(styled_key_hint_line(
                        &format!("{indent}{part}"),
                        style,
                        theme,
                    ));
                }
            }
        } else {
            let full = format!("{prefix}{rest}");
            for part in wrap_tokens(&full, avail_width) {
                if lines.len() >= usize::from(area.height) {
                    break;
                }
                lines.push(styled_key_hint_line(
                    &truncate_to_width(&part, avail_width),
                    style,
                    theme,
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_workspace_footer(
    frame: &mut Frame<'_>,
    footer: Rect,
    status: Option<&str>,
    theme: Theme,
) {
    let zh = theme.language.is_zh() && !theme.ascii;
    let variants: [&str; 4] = if zh {
        [
            "←→ 会话 · ↑↓ 窗口 · Enter 执行 · 输入 / 打开命令 · ? 帮助",
            "←→ 会话 · ↑↓ 窗口 · 输入 / 打开命令 · ? 帮助",
            "输入 / 打开命令 · ? 帮助",
            "输入 / 打开命令",
        ]
    } else {
        [
            "←→ Session · ↑↓ Window · Enter Act · Type / for commands · ? Help",
            "←→ Session · ↑↓ Window · Type / for commands · ? Help",
            "Type / for commands · ? Help",
            "Type / for commands",
        ]
    };
    render_footer_variants(frame, footer, status, &variants, theme);
}

pub fn render_home(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    popup: &[CommandItem<'_>],
    selected: usize,
    details: HomeDetails<'_>,
    theme: Theme,
) {
    render_home_dashboard(
        frame,
        area,
        input,
        popup,
        selected,
        details,
        HomeTelemetry::default(),
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn render_home_dashboard(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    popup: &[CommandItem<'_>],
    selected: usize,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    theme: Theme,
) {
    render_home_studio(
        frame,
        area,
        input,
        popup,
        selected,
        details,
        telemetry,
        HomeDashboardState::default(),
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn render_home_studio(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    popup: &[CommandItem<'_>],
    selected: usize,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    studio: HomeDashboardState<'_>,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.width < 36 || area.height < 12 {
        let [welcome, composer, footer] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(if area.height > 4 { 2 } else { 1 }),
            Constraint::Length(u16::from(area.height > 2)),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(compact_welcome_lines(
                welcome,
                details,
                telemetry,
                popup.is_empty() && studio.mentions.is_empty(),
                theme,
            )),
            welcome,
        );
        render_composer_line(
            frame,
            composer,
            input,
            studio.mode,
            theme.accent,
            composer.width >= 44,
            theme,
        );
        render_home_footer(frame, footer, details.status, theme);
        render_home_drawers(
            frame,
            composer,
            HomeDrawerData {
                popup,
                popup_selected: selected,
                mentions: studio.mentions,
                mention_selected: studio.mention_selected,
            },
            theme,
        );
        return;
    }

    let [welcome, composer, footer] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);

    render_studio_welcome_card(
        frame,
        welcome,
        details,
        telemetry,
        studio,
        popup.is_empty(),
        area.width >= 80 && area.height >= 22,
        area.width >= 80 && area.height >= 24,
        theme,
    );

    let border_color = studio.mode.border_color(theme);
    render_composer_card(
        frame,
        composer,
        input,
        ComposerChrome {
            mode: studio.mode,
            border_color,
            left_title: studio.mode.label_for(theme.ascii, theme.language),
            right_title: (composer.width >= 48)
                .then_some(theme.tr(" Shift+Tab Mode ", " Shift+Tab 切换模式 ")),
        },
        theme,
    );

    render_home_footer(frame, footer, details.status, theme);

    let drawer_anchor = Rect {
        x: composer.x,
        y: composer.y + 1,
        width: composer.width,
        height: 1,
    };
    render_home_drawers(
        frame,
        drawer_anchor,
        HomeDrawerData {
            popup,
            popup_selected: selected,
            mentions: studio.mentions,
            mention_selected: studio.mention_selected,
        },
        theme,
    );
}

fn render_composer_line(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    mode: ComposerMode,
    marker_color: Color,
    show_placeholder: bool,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let marker = if area.width > 2 {
        format!("{} ", theme.marker())
    } else {
        String::new()
    };
    let avail = area.width.saturating_sub(marker.width() as u16);
    let mut line = input.rendered_width(true, theme, avail);
    if input.value().is_empty() && show_placeholder {
        let remaining = usize::from(area.width).saturating_sub(marker.width() + line.width());
        if remaining >= 8 {
            let placeholder =
                truncate_to_width(mode.placeholder_for(theme.ascii, theme.language), remaining);
            if !placeholder.is_empty() {
                line.spans.extend(styled_key_hint_spans(
                    &placeholder,
                    theme.style(theme.muted),
                    theme,
                ));
            }
        }
    }
    line.spans.insert(
        0,
        Span::styled(
            marker,
            theme.style(marker_color).add_modifier(Modifier::BOLD),
        ),
    );
    frame.render_widget(Paragraph::new(line), area);
}

#[allow(clippy::too_many_arguments)]
fn render_studio_welcome_card(
    frame: &mut Frame<'_>,
    welcome: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    studio: HomeDashboardState<'_>,
    show_commands: bool,
    spacious_list: bool,
    spacious_preview: bool,
    theme: Theme,
) {
    let accent = theme.style(theme.accent);
    let max_title_width = usize::from(welcome.width.saturating_sub(3));
    let mut title_spans =
        build_home_card_title_prefix(welcome.width, max_title_width, telemetry, true, theme);
    if title_spans.iter().map(Span::width).sum::<usize>() < max_title_width {
        title_spans.push(Span::raw(" "));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(accent)
        .title(Line::from(title_spans));
    let raw_inner = block.inner(welcome);
    frame.render_widget(block, welcome);
    if raw_inner.width == 0 || raw_inner.height == 0 {
        return;
    }
    let inner = if raw_inner.width >= 6 {
        Rect {
            x: raw_inner.x + 1,
            width: raw_inner.width - 2,
            ..raw_inner
        }
    } else {
        raw_inner
    };

    if studio.mode == ComposerMode::LiveRadar {
        render_live_radar(frame, inner, studio, theme);
        return;
    }

    // Mode A — 3-Column Fullscreen Hero Studio
    if welcome.width >= 104 && raw_inner.height >= 16 {
        let hero_target = if raw_inner.height >= 24 && raw_inner.width >= 116 {
            Some((48_u16, 24_u16))
        } else if raw_inner.height >= 16 && raw_inner.width >= 102 {
            Some((32_u16, 16_u16))
        } else {
            None
        };
        if let Some((target_w, target_h)) = hero_target
            && let Some((hydra_w, hydra_lines)) =
                hydra_emblem_frame(target_w, target_h, studio.emblem_step, theme)
        {
            let usable = if raw_inner.width >= hydra_w + 72 {
                Rect {
                    x: raw_inner.x + 1,
                    width: raw_inner.width - 2,
                    ..raw_inner
                }
            } else {
                raw_inner
            };
            let rem = usable.width.saturating_sub(hydra_w + 2);
            let max_mid = rem.saturating_sub(33);
            let mid_w = (((u32::from(rem) * 51) / 100) as u16).clamp(35, max_mid.max(35));
            let [hydra_area, div1, mid_area, div2, right_area] = Layout::horizontal([
                Constraint::Length(hydra_w),
                Constraint::Length(1),
                Constraint::Length(mid_w),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .areas(usable);
            render_hydra_emblem_area(frame, hydra_area, hydra_lines);
            render_vertical_divider(frame, div1, theme);
            let mid_inner = Rect {
                x: mid_area.x + 1,
                width: mid_area.width.saturating_sub(1),
                ..mid_area
            };
            render_studio_mid(
                frame,
                mid_inner,
                details,
                telemetry,
                show_commands,
                spacious_list,
                theme,
            );
            render_vertical_divider(frame, div2, theme);
            let right_inner = Rect {
                x: right_area.x + 1,
                width: right_area.width.saturating_sub(1),
                ..right_area
            };
            render_studio_right(
                frame,
                right_inner,
                studio,
                spacious_list,
                spacious_preview,
                theme,
            );
            return;
        }
    }

    // Mode B — 2-Column Hydra Studio
    if welcome.width >= 76 && inner.height >= 14 {
        let max_hydra_w = inner.width.saturating_sub(36);
        if let Some((hydra_w, hydra_lines)) =
            hydra_emblem_frame(max_hydra_w, inner.height, studio.emblem_step, theme)
        {
            let [hydra_area, divider, right_area] = Layout::horizontal([
                Constraint::Length(hydra_w),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .areas(inner);
            render_hydra_emblem_area(frame, hydra_area, hydra_lines);
            render_vertical_divider(frame, divider, theme);
            let right_inner = Rect {
                x: right_area.x + 1,
                width: right_area.width.saturating_sub(1),
                ..right_area
            };
            render_studio_combined_right(
                frame,
                right_inner,
                details,
                telemetry,
                studio,
                show_commands,
                spacious_list,
                theme,
            );
            return;
        }
    }

    // Mode C — Compact / Inline & Narrow Fallbacks
    if welcome.width >= 76 && inner.width >= 60 {
        let [left, divider, right] = Layout::horizontal([
            Constraint::Min(42),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inner);
        render_studio_left(
            frame,
            left,
            details,
            telemetry,
            show_commands,
            spacious_list,
            theme,
        );
        render_vertical_divider(frame, divider, theme);
        let right_inner = if right.width >= 10 {
            Rect {
                x: right.x + 1,
                width: right.width - 1,
                ..right
            }
        } else {
            right
        };
        render_studio_right(
            frame,
            right_inner,
            studio,
            spacious_list,
            spacious_preview,
            theme,
        );
    } else {
        let lines = welcome_info_lines(inner, details, telemetry, show_commands, theme);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn render_hydra_emblem_area(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    if area.width == 0 || area.height == 0 || lines.is_empty() {
        return;
    }
    let top_pad = usize::from(area.height).saturating_sub(lines.len()) / 2;
    let mut padded = Vec::with_capacity(top_pad + lines.len());
    padded.resize_with(top_pad, Line::default);
    padded.extend(lines);
    frame.render_widget(Paragraph::new(padded), area);
}

fn render_vertical_divider(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let div_glyph = if theme.ascii { "|" } else { "│" };
    let muted = theme.style(theme.muted);
    let div_lines: Vec<Line<'static>> = (0..area.height)
        .map(|_| Line::from(Span::styled(div_glyph, muted)))
        .collect();
    frame.render_widget(Paragraph::new(div_lines), area);
}

fn append_horizontal_rule(lines: &mut Vec<Line<'static>>, width: u16, theme: Theme) {
    let rule_len = usize::from(width).saturating_sub(1);
    if rule_len == 0 {
        return;
    }
    let rule_ch = if theme.ascii { "-" } else { "─" };
    lines.push(Line::from(Span::styled(
        rule_ch.repeat(rule_len),
        theme.style(theme.muted),
    )));
}

fn context_row_spans(
    avail: usize,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    theme: Theme,
) -> [Option<Vec<Span<'static>>>; 3] {
    let muted = theme.style(theme.muted);
    let bold_accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);

    let dir_line = details.directory.map(|directory| {
        let label = if theme.ascii { "dir " } else { "⌂ " };
        let width = avail.saturating_sub(label.width());
        let directory = single_line(directory);
        let marker = if theme.ascii { "..." } else { "…" };
        vec![
            Span::styled(label, muted),
            Span::raw(directory_tail(&directory, width, marker)),
        ]
    });

    let ctx_line = (!details.context.is_empty()).then(|| {
        let mut spans = vec![
            Span::styled("tmux ", muted),
            Span::raw(details.context.to_owned()),
        ];
        if let Some(session) = details.session.filter(|name| !name.is_empty()) {
            spans.push(Span::styled(if theme.ascii { " | " } else { " · " }, muted));
            spans.push(Span::styled(single_line(session), bold_accent));
        }
        spans
    });

    let tele_line = if let Some(summary) = format_telemetry(telemetry, theme) {
        Some(vec![Span::styled(
            truncate_to_width(&summary, avail),
            theme.style(theme.secondary),
        )])
    } else {
        Some(vec![Span::styled(
            truncate_to_width(
                &theme.adapt(&format!(
                    "{} · {}",
                    theme.preset.name(),
                    theme.preset.origin()
                )),
                avail,
            ),
            muted,
        )])
    };

    [dir_line, ctx_line, tele_line]
}

fn build_studio_context_lines(
    avail: usize,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    theme: Theme,
) -> Vec<Line<'static>> {
    context_row_spans(avail, details, telemetry, theme)
        .into_iter()
        .flatten()
        .map(Line::from)
        .collect()
}

fn render_studio_mid(
    frame: &mut Frame<'_>,
    area: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    show_commands: bool,
    spacious: bool,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let mut lines = build_studio_context_lines(usize::from(area.width), details, telemetry, theme);
    if show_commands {
        if usize::from(area.height) >= lines.len() + 7 {
            append_horizontal_rule(&mut lines, area.width, theme);
        }
        if usize::from(area.height) >= lines.len() + 6 {
            lines.push(Line::from(Span::styled(
                if zh {
                    "试试以下命令"
                } else {
                    "Try a command"
                },
                theme.style(theme.muted),
            )));
        }
        append_quick_commands(&mut lines, area, "", spacious, theme);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

#[allow(clippy::too_many_arguments)]
fn render_studio_combined_right(
    frame: &mut Frame<'_>,
    area: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    studio: HomeDashboardState<'_>,
    show_commands: bool,
    spacious_list: bool,
    theme: Theme,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let mut lines = build_studio_context_lines(usize::from(area.width), details, telemetry, theme);
    if usize::from(area.height) >= lines.len() + 4 {
        append_horizontal_rule(&mut lines, area.width, theme);
    }
    let max_workspaces = if area.height >= 22 { 5 } else { 3 };
    append_workspace_rows(
        &mut lines,
        area,
        if zh {
            "◈ 活跃工作区"
        } else {
            "◈ Live Workspaces"
        },
        max_workspaces,
        studio.sessions,
        theme,
    );
    if show_commands && spacious_list && lines.len() + 8 <= usize::from(area.height) {
        append_horizontal_rule(&mut lines, area.width, theme);
        lines.push(Line::from(Span::styled(
            if zh {
                "试试以下命令"
            } else {
                "Try a command"
            },
            theme.style(theme.muted),
        )));
        append_quick_commands(&mut lines, area, "", spacious_list, theme);
    }
    if usize::from(area.height) > lines.len() + 2 {
        lines.push(Line::default());
        append_pro_tip_lines(&mut lines, area.width, area.height, studio.tip_step, theme);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_studio_left(
    frame: &mut Frame<'_>,
    area: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    show_commands: bool,
    spacious: bool,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let muted = theme.style(theme.muted);
    let mascot = mascot_lines(theme);
    let mascot_colors = [theme.accent, theme.secondary, theme.tertiary];
    let mascot_w = mascot.iter().map(|line| line.width()).max().unwrap_or(5);
    let info_avail = usize::from(area.width).saturating_sub(mascot_w + 2);

    let info_rows = context_row_spans(info_avail, details, telemetry, theme);
    let mut lines = Vec::new();
    for (idx, m_line) in mascot.into_iter().enumerate() {
        let pad = " ".repeat(mascot_w.saturating_sub(m_line.width()) + 2);
        let m_style = if idx < 2 {
            theme.style(mascot_colors[idx]).add_modifier(Modifier::BOLD)
        } else {
            theme.style(mascot_colors[idx])
        };
        let mut spans = vec![Span::styled(m_line, m_style), Span::raw(pad)];
        if let Some(extra) = info_rows.get(idx).and_then(|row| row.as_ref()) {
            spans.extend(extra.iter().cloned());
        }
        lines.push(Line::from(spans));
    }

    if show_commands {
        if usize::from(area.height) >= lines.len() + 7 {
            append_horizontal_rule(&mut lines, area.width, theme);
        }
        if usize::from(area.height) >= lines.len() + 6 {
            lines.push(Line::from(Span::styled(
                if zh {
                    "试试以下命令"
                } else {
                    "Try a command"
                },
                muted,
            )));
        }
        append_quick_commands(&mut lines, area, "", spacious, theme);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn append_workspace_rows(
    lines: &mut Vec<Line<'static>>,
    area: Rect,
    header_label: &str,
    max_workspaces: usize,
    sessions: &[HomeSessionEntry],
    theme: Theme,
) {
    let zh = theme.language.is_zh() && !theme.ascii;
    let muted = theme.style(theme.muted);
    let accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let header = theme.adapt(header_label);
    lines.push(styled_key_hint_line(
        &truncate_to_width(&header, usize::from(area.width)),
        accent,
        theme,
    ));

    if sessions.is_empty() {
        let launch_label = if theme.ascii {
            " ! Launch workspace in cwd"
        } else if zh {
            " ⚡ 在当前目录启动工作区"
        } else {
            " ⚡ Launch workspace in cwd"
        };
        lines.push(Line::from(vec![
            Span::styled("[Enter]", theme.keycap_style()),
            Span::styled(
                truncate_to_width(launch_label, usize::from(area.width).saturating_sub(7)),
                theme.style(theme.text),
            ),
        ]));
        if usize::from(area.height) >= lines.len() + 3 {
            lines.push(Line::from(Span::styled(
                truncate_to_width(
                    if zh {
                        "  或输入 /new <名称>"
                    } else {
                        "  or type /new <name>"
                    },
                    usize::from(area.width),
                ),
                muted,
            )));
        }
        return;
    }

    for (idx, session) in sessions.iter().take(max_workspaces).enumerate() {
        if lines.len() >= usize::from(area.height) {
            break;
        }
        let key_badge = format!("[{}]", idx + 1);
        let dot = if session.attached {
            if theme.ascii { "* " } else { "● " }
        } else if theme.ascii {
            "o "
        } else {
            "○ "
        };
        let dot_style = if session.attached {
            theme.style(theme.success)
        } else {
            muted
        };
        let name_style = if session.current {
            theme.style(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            theme.style(theme.text).add_modifier(Modifier::BOLD)
        };
        let badge_str = session
            .badge
            .map(|b| theme.adapt(&format!(" {b}")))
            .unwrap_or_default();
        let win_unit = if zh { "窗" } else { "w" };
        let meta = if session.active_window.is_empty() {
            format!("  {}{win_unit}", session.windows)
        } else {
            theme.adapt(&format!(
                "  {}{win_unit} · {}",
                session.windows, session.active_window
            ))
        };
        let fixed = key_badge.width() + 1 + dot.width() + badge_str.width();
        let avail_for_name = usize::from(area.width).saturating_sub(fixed);
        let name = truncate_to_width(&single_line(&session.name), avail_for_name);
        let rem = avail_for_name.saturating_sub(name.width());
        let mut spans = vec![
            Span::styled(key_badge, theme.keycap_style()),
            Span::raw(" "),
            Span::styled(dot, dot_style),
            Span::styled(name, name_style),
        ];
        if rem >= 4 {
            spans.push(Span::styled(truncate_to_width(&meta, rem), muted));
        }
        if !badge_str.is_empty() {
            spans.push(Span::styled(badge_str, theme.style(theme.secondary)));
        }
        lines.push(Line::from(spans));
    }
}

fn render_studio_right(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: HomeDashboardState<'_>,
    spacious_list: bool,
    spacious_preview: bool,
    theme: Theme,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let max_workspaces = if spacious_list { 6 } else { 4 };
    let header_label = if zh {
        if spacious_list {
            "◈ 活跃工作区 (1-6)"
        } else {
            "◈ 活跃工作区 (1-5)"
        }
    } else if spacious_list {
        "◈ Live Workspaces (1-6)"
    } else {
        "◈ Live Workspaces (1-5)"
    };
    let mut lines = Vec::new();
    append_workspace_rows(
        &mut lines,
        area,
        header_label,
        max_workspaces,
        studio.sessions,
        theme,
    );

    let preview_card = spacious_preview
        .then_some(studio.preview)
        .flatten()
        .filter(|_| area.height >= 14 && area.width >= 20);
    if let Some(preview) = preview_card {
        let workspace_rows = (lines.len() as u16).min(area.height.saturating_sub(8));
        let [top_area, gap_area, preview_area, tip_area] = Layout::vertical([
            Constraint::Length(workspace_rows),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .areas(area);
        let _ = gap_area;
        frame.render_widget(Paragraph::new(lines), top_area);
        let subtitle = if preview.subtitle.is_empty() {
            preview.title.to_owned()
        } else if preview.title.is_empty() {
            preview.subtitle.to_owned()
        } else {
            format!("{} · {}", preview.title, preview.subtitle)
        };
        render_pane_preview_card(
            frame,
            preview_area,
            PanePreview {
                title: if zh {
                    "📺 当前工作区预览"
                } else {
                    "📺 Active Workspace Preview"
                },
                subtitle: &subtitle,
                ..preview
            },
            theme,
        );
        render_pro_tip_block(frame, tip_area, studio.tip_step, theme);
        return;
    }

    if usize::from(area.height) > lines.len() + 2 {
        lines.push(Line::default());
        append_pro_tip_lines(&mut lines, area.width, area.height, studio.tip_step, theme);
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_pro_tip_block(frame: &mut Frame<'_>, area: Rect, tip_step: usize, theme: Theme) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    append_pro_tip_lines(&mut lines, area.width, area.height, tip_step, theme);
    frame.render_widget(Paragraph::new(lines), area);
}

fn append_pro_tip_lines(
    lines: &mut Vec<Line<'static>>,
    width: u16,
    max_height: u16,
    tip_step: usize,
    theme: Theme,
) {
    let zh = theme.language.is_zh() && !theme.ascii;
    let muted = theme.style(theme.muted);
    lines.push(Line::from(Span::styled(
        theme.adapt(if zh {
            "✻ 使用技巧"
        } else {
            "✻ Pro Tip"
        }),
        theme.style(theme.secondary).add_modifier(Modifier::BOLD),
    )));
    let tips = if zh {
        [
            "输入 @会话名 可直接跳转到任意会话或窗口",
            "输入框为空时按 1-5 可一键附着到对应工作区",
            "按 Shift+Tab 在 命令跳转、实时雷达 与 快速启动 间切换",
            "按 Ctrl+T 轮换 5 套真彩主题 · /theme 实时预览",
            "输入框为空时按 ↓ 可浏览所有斜杠命令",
            "输入 /codex、/claude、/antigravity 或 /vibe 解锁彩蛋卡片",
            "/split 打开 2x2 可视化分屏工作台并预览实时终端输出",
        ]
    } else {
        [
            "Type @session to jump directly to any session or window",
            "Press 1-5 on an empty bar to attach straight to a workspace",
            "Shift+Tab cycles Command, Live Radar & Quick Launch modes",
            "Ctrl+T cycles 5 TrueColor themes · /theme previews live",
            "Press ↓ on an empty bar to browse all slash commands",
            "Try /codex, /claude, /antigravity, or /vibe for tributes",
            "/split opens the 2x2 visual Split Studio with live pane previews",
        ]
    };
    let tip = theme.adapt(tips[tip_step % tips.len()]);
    let remaining_rows = usize::from(max_height).saturating_sub(lines.len());
    for wrapped in wrap_description(&tip, width)
        .into_iter()
        .take(remaining_rows)
    {
        lines.push(styled_key_hint_line(&wrapped, muted, theme));
    }
}

fn render_live_radar(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: HomeDashboardState<'_>,
    theme: Theme,
) -> PreviewCardLayout {
    if area.width == 0 || area.height == 0 {
        return PreviewCardLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let (left, right) = if area.width >= 54 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
            .areas(area);
        (l, Some(r))
    } else {
        (area, None)
    };

    let muted = theme.style(theme.muted);
    let header_title = theme.adapt(&format!(
        "◈ {} ({})",
        if zh { "实时雷达" } else { "Live Radar" },
        studio.sessions.len()
    ));
    let mut left_lines = vec![Line::from(Span::styled(
        truncate_to_width(&header_title, usize::from(left.width)),
        theme.style(theme.secondary).add_modifier(Modifier::BOLD),
    ))];

    if studio.sessions.is_empty() {
        left_lines.push(Line::from(Span::styled(
            truncate_to_width(
                if zh {
                    "  暂无活跃 tmux 会话"
                } else {
                    "  No active tmux sessions"
                },
                usize::from(left.width),
            ),
            muted,
        )));
        left_lines.push(styled_key_hint_line(
            &truncate_to_width(
                if zh {
                    "  按 Enter 立即启动或输入 /new"
                } else {
                    "  Press Enter to launch or /new"
                },
                usize::from(left.width),
            ),
            muted,
            theme,
        ));
    } else {
        let visible_rows = usize::from(left.height.saturating_sub(1)).max(1);
        let selected = studio.radar_selected.min(studio.sessions.len() - 1);
        let offset = (selected + 1).saturating_sub(visible_rows);
        for (idx, session) in studio
            .sessions
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
        {
            let is_sel = idx == selected;
            let row_style = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            let dot = if session.attached {
                if theme.ascii { "*" } else { "●" }
            } else if theme.ascii {
                "o"
            } else {
                "○"
            };
            let dot_style = if is_sel {
                row_style
            } else if session.attached {
                theme.style(theme.success)
            } else {
                muted
            };
            let win_tag = if zh {
                format!(" {}窗", session.windows)
            } else {
                format!(" {}w", session.windows)
            };
            let avail = usize::from(left.width).saturating_sub(4 + win_tag.width());
            let name = truncate_to_width(&single_line(&session.name), avail);
            left_lines.push(Line::from(vec![
                Span::styled(format!("{marker} "), row_style),
                Span::styled(format!("{dot} "), dot_style),
                Span::styled(name, row_style),
                Span::styled(win_tag, if is_sel { row_style } else { muted }),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(left_lines), left);

    if let Some(right_area) = right {
        let has_preview = studio.preview.is_some();
        let preview = studio.preview.unwrap_or(PanePreview {
            title: if zh {
                "实时终端预览"
            } else {
                "Live Terminal Preview"
            },
            subtitle: if zh { "空闲" } else { "idle" },
            lines: &[],
            ..PanePreview::default()
        });
        return render_pane_preview_card_interactive(
            frame,
            right_area,
            PanePreview {
                show_actions: has_preview && preview.show_actions,
                ..preview
            },
            theme,
        );
    }
    PreviewCardLayout::default()
}

fn compact_welcome_lines(
    area: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    show_commands: bool,
    theme: Theme,
) -> Vec<Line<'static>> {
    let accent = theme.style(theme.accent);
    let muted = theme.style(theme.muted);
    let bold_accent = accent.add_modifier(Modifier::BOLD);

    let mut title = Line::from(vec![Span::styled(
        if theme.ascii {
            "[] JustMux"
        } else {
            "◫ JustMux"
        },
        bold_accent,
    )]);
    let version = format!(" (v{})", env!("CARGO_PKG_VERSION"));
    if title.width() + version.width() <= usize::from(area.width) {
        title.spans.push(Span::styled(version, muted));
    }
    if area.width >= 54 {
        let bg = theme.pill_bg();
        if let Some(badge) = telemetry.hint_badge.filter(|b| !b.is_empty()) {
            let pill = pill_span(format!(" {} ", badge), theme.accent, bg, theme, false);
            if title.width() + 2 + pill.width() <= usize::from(area.width) {
                title.spans.push(Span::raw("  "));
                title.spans.push(pill);
            }
        } else if theme.preset != ThemePreset::Classic {
            let badge = format!(
                " {} {} ",
                theme.preset.badge_icon(theme.ascii),
                theme.preset.slug()
            );
            let pill = pill_span(badge, theme.secondary, bg, theme, false);
            if title.width() + 2 + pill.width() <= usize::from(area.width) {
                title.spans.push(Span::raw("  "));
                title.spans.push(pill);
            }
        }
    }

    let mut lines = vec![title];
    lines.extend(welcome_info_lines(
        area,
        details,
        telemetry,
        show_commands,
        theme,
    ));
    lines
}

fn welcome_info_lines(
    area: Rect,
    details: HomeDetails<'_>,
    telemetry: HomeTelemetry<'_>,
    show_commands: bool,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let muted = theme.style(theme.muted);
    let bold_accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    if let Some(directory) = details.directory {
        let label = if theme.ascii { "  dir " } else { "  ⌂ " };
        let width = usize::from(area.width).saturating_sub(label.width());
        let directory = single_line(directory);
        let marker = if theme.ascii { "..." } else { "…" };
        lines.push(Line::from(vec![
            Span::styled(label, muted),
            Span::raw(directory_tail(&directory, width, marker)),
        ]));
    }

    let telemetry_summary = format_telemetry(telemetry, theme);
    if !details.context.is_empty() {
        let mut spans = vec![
            Span::styled("  tmux ", muted),
            Span::raw(details.context.to_owned()),
        ];
        if let Some(session) = details.session.filter(|name| !name.is_empty()) {
            spans.push(Span::styled(if theme.ascii { " | " } else { " · " }, muted));
            spans.push(Span::styled(single_line(session), bold_accent));
        }
        let mut line = Line::from(spans);
        if let Some(summary) = telemetry_summary.as_deref() {
            let sep = if theme.ascii { " | " } else { "  │  " };
            if line.width() + sep.width() + summary.width() <= usize::from(area.width) {
                line.spans.push(Span::styled(sep, muted));
                line.spans.push(Span::styled(
                    summary.to_owned(),
                    theme.style(theme.secondary),
                ));
            }
        }
        lines.push(line);
    } else if let Some(summary) = telemetry_summary {
        lines.push(Line::from(vec![
            Span::styled("  live ", muted),
            Span::styled(summary, theme.style(theme.secondary)),
        ]));
    }

    if show_commands {
        if usize::from(area.height) >= lines.len() + 3 {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                if zh {
                    "试试以下命令"
                } else {
                    "Try a command"
                },
                muted,
            )));
        }
        append_quick_commands(&mut lines, area, "  ", false, theme);
    }
    lines
}

fn append_quick_commands(
    lines: &mut Vec<Line<'static>>,
    area: Rect,
    prefix: &str,
    spacious: bool,
    theme: Theme,
) {
    let muted = theme.style(theme.muted);
    let compact_start: &[CommandKind] = &[
        CommandKind::Resume,
        CommandKind::New,
        CommandKind::List,
        CommandKind::Window,
        CommandKind::Help,
    ];
    let spacious_start: &[CommandKind] = &[
        CommandKind::Resume,
        CommandKind::New,
        CommandKind::List,
        CommandKind::Window,
        CommandKind::Pane,
        CommandKind::Split,
        CommandKind::Theme,
        CommandKind::Help,
    ];
    let quick_start = if spacious {
        spacious_start
    } else {
        compact_start
    };
    let command_style = theme.style(theme.success).add_modifier(Modifier::BOLD);
    for command in quick_start.iter().filter_map(|&kind| {
        registry()
            .iter()
            .chain(MERGED_COMMANDS.iter())
            .find(|command| command.kind == kind)
    }) {
        if lines.len() >= usize::from(area.height) {
            break;
        }
        let cmd = format!("{prefix}/{}", command.name);
        let mut spans = vec![Span::styled(cmd, command_style)];
        if area.width >= 34 {
            let pad = " ".repeat(9_usize.saturating_sub(command.name.width()));
            let used = prefix.width() + 1 + command.name.width() + pad.width();
            let desc_avail = usize::from(area.width).saturating_sub(used);
            if desc_avail > 0 {
                let desc = crate::commands::description_for(command.kind, theme.language);
                spans.push(Span::raw(pad));
                spans.push(Span::styled(
                    truncate_to_width(&theme.adapt(desc), desc_avail),
                    muted,
                ));
            }
        }
        lines.push(Line::from(spans));
    }
}

fn render_home_footer(frame: &mut Frame<'_>, footer: Rect, status: Option<&str>, theme: Theme) {
    let zh = theme.language.is_zh() && !theme.ascii;
    let variants: [&str; 4] = if zh {
        [
            "输入 / 打开命令 · ↓ 菜单 · Ctrl+T 主题 · ? 帮助",
            "输入 / 打开命令 · ↓ 菜单 · Ctrl+T 主题",
            "输入 / 打开命令 · ? 帮助",
            "输入 / 打开命令",
        ]
    } else {
        [
            "Type / for commands · ↓ Menu · Ctrl+T Theme · ? Help",
            "Type / for commands · ↓ Menu · Ctrl+T Theme",
            "Type / for commands · ? Help",
            "Type / for commands",
        ]
    };
    render_footer_variants(frame, footer, status, &variants, theme);
}

fn render_footer_variants(
    frame: &mut Frame<'_>,
    footer: Rect,
    status: Option<&str>,
    variants: &[&str],
    theme: Theme,
) {
    if footer.width == 0 || footer.height == 0 {
        return;
    }
    if let Some(status) = status {
        let style = status_style(status, theme);
        frame.render_widget(Paragraph::new(theme.adapt(status)).style(style), footer);
        return;
    }
    let width = usize::from(footer.width);
    let adapted: Vec<String> = variants.iter().map(|hint| theme.adapt(hint)).collect();
    let fallback = theme.tr("Type / for commands", "输入 / 打开命令");
    let text = adapted
        .iter()
        .find(|hint| hint.width() <= width)
        .cloned()
        .unwrap_or_else(|| {
            truncate_to_width(adapted.last().map_or(fallback, String::as_str), width)
        });
    frame.render_widget(
        Paragraph::new(styled_key_hint_line(&text, theme.style(theme.muted), theme)),
        footer,
    );
}

pub fn status_style(status: &str, theme: Theme) -> Style {
    let trimmed = status.trim_start();
    if trimmed.starts_with("Renamed")
        || trimmed.starts_with("Refreshed")
        || trimmed.starts_with("Pane split")
        || trimmed.starts_with("Session created")
        || trimmed.starts_with("Window created")
        || trimmed.starts_with("Saved")
        || trimmed.starts_with("已重命名")
        || trimmed.starts_with("已刷新")
        || trimmed.starts_with("窗格已拆分")
        || trimmed.starts_with("会话已创建")
        || trimmed.starts_with("窗口已创建")
        || trimmed.starts_with("已保存")
        || trimmed.starts_with("已关闭")
    {
        theme.style(theme.success)
    } else if trimmed.starts_with('✦')
        || trimmed.starts_with('✻')
        || trimmed.starts_with('◈')
        || trimmed.starts_with('🎯')
        || trimmed.starts_with('🔥')
        || trimmed.starts_with(">_")
        || trimmed.starts_with("Cancelled")
        || trimmed.starts_with("已取消")
        || trimmed.starts_with("Reset")
        || trimmed.starts_with("Flipped")
        || trimmed.starts_with("分屏画布")
        || trimmed.starts_with("已翻转")
    {
        theme.style(theme.accent)
    } else {
        theme.style(theme.error)
    }
}

pub(super) fn truncate_to_width(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_owned();
    }
    let mut used = 0;
    let mut end = 0;
    for (byte, grapheme) in text.grapheme_indices(true) {
        let w = grapheme.width();
        if used + w > max_width {
            break;
        }
        used += w;
        end = byte + grapheme.len();
    }
    text[..end].to_owned()
}

fn format_telemetry(telemetry: HomeTelemetry<'_>, theme: Theme) -> Option<String> {
    let (sessions, windows) = (telemetry.sessions, telemetry.windows);
    if sessions.is_none()
        && windows.is_none()
        && telemetry.attached.is_none()
        && telemetry.clients.is_none()
    {
        return None;
    }
    let sep = if theme.ascii { " | " } else { " · " };
    let mut parts = Vec::new();
    if let Some(s) = sessions {
        parts.push(format!("{s}{}", theme.tr("s", "会话")));
    }
    if let Some(w) = windows {
        parts.push(format!("{w}{}", theme.tr("w", "窗")));
    }
    if let Some(a) = telemetry.attached.filter(|&a| a > 0) {
        let dot = if theme.ascii { "a" } else { "●" };
        parts.push(format!("{a}{dot}"));
    }
    if let Some(c) = telemetry.clients.filter(|&c| c > 1) {
        if theme.ascii {
            parts.push(format!("{c}c"));
        } else {
            parts.push(format!("👥 {c}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(sep))
}

pub(super) fn single_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn directory_tail(path: &str, width: usize, marker: &str) -> String {
    if path.width() <= width {
        return path.to_owned();
    }
    let Some(mut remaining) = width.checked_sub(marker.width()) else {
        return String::new();
    };
    let mut start = path.len();
    for (index, grapheme) in path.grapheme_indices(true).rev() {
        let Some(next) = remaining.checked_sub(grapheme.width()) else {
            break;
        };
        remaining = next;
        start = index;
    }
    format!("{marker}{}", &path[start..])
}

fn command_group_badge(name: &str, theme: Theme) -> (&'static str, Color) {
    match name {
        "resume" | "new" | "window" | "rename" | "kill" | "snapshot" | "list" => {
            (theme.tr("workspace", "工作区"), theme.secondary)
        }
        "pane" | "split" | "layout" | "expose" | "buffers" | "popup" | "theme" | "config" => {
            (theme.tr("studio", "工作台"), theme.tertiary)
        }
        "kill-server" => (theme.tr("danger", "危险"), theme.error),
        "clients" | "help" | "exit" | "quit" => (theme.tr("system", "系统"), theme.muted),
        _ => (theme.tr("studio", "工作台"), theme.tertiary),
    }
}

pub fn render_command_popup(
    frame: &mut Frame<'_>,
    anchor: Rect,
    items: &[CommandItem<'_>],
    selected: usize,
    theme: Theme,
) {
    // Inline viewports can start below existing shell output. Measure the
    // available rows relative to that viewport, rather than the screen top.
    let available = anchor.y.saturating_sub(frame.area().y);
    let h = (items.len() as u16 + 2).min(available);
    if h == 0 {
        return;
    }
    let area = Rect {
        x: anchor.x,
        y: anchor.y.saturating_sub(h),
        width: anchor.width.min(80),
        height: h,
    };
    frame.render_widget(Clear, area);
    let label_width = |item: &CommandItem<'_>| {
        1 + item.name.len() + item.alias.map_or(0, |alias| 2 + alias.len())
    };
    let name_column = items
        .iter()
        .map(label_width)
        .max()
        .map_or(13, |width| 13.max(width + 2));
    let show_groups = area.width >= 68;
    let inner_width = usize::from(area.width);
    let bg = theme.pill_bg();
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let is_selected = i == selected;
            let style = if is_selected {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let match_style = if is_selected {
                style.add_modifier(Modifier::UNDERLINED)
            } else {
                theme
                    .style(theme.accent)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            };
            let mut spans = vec![Span::styled("/", style)];
            if let Some(alias) = c.alias {
                spans.push(Span::styled(c.name, style));
                spans.push(Span::styled(" /", style));
                spans.extend(highlighted_spans_with_match(
                    alias,
                    c.indices,
                    style,
                    match_style,
                ));
            } else {
                spans.extend(highlighted_spans_with_match(
                    c.name,
                    c.indices,
                    style,
                    match_style,
                ));
            }
            spans.push(Span::styled(
                " ".repeat(name_column.saturating_sub(label_width(c))),
                style,
            ));
            let desc_style = if is_selected {
                theme.style(theme.text)
            } else {
                theme.style(theme.muted)
            };
            if show_groups {
                let (group_label, group_color) = command_group_badge(c.name, theme);
                let badge = format!(" {group_label} ");
                let avail = inner_width.saturating_sub(name_column);
                if avail > badge.width() + 2 {
                    let desc =
                        truncate_to_width(&theme.adapt(c.description), avail - badge.width() - 1);
                    let gap = avail.saturating_sub(desc.width() + badge.width());
                    spans.push(Span::styled(desc, desc_style));
                    spans.push(Span::raw(" ".repeat(gap)));
                    spans.push(pill_span(badge, group_color, bg, theme, false));
                } else {
                    spans.push(Span::styled(
                        truncate_to_width(&theme.adapt(c.description), avail),
                        desc_style,
                    ));
                }
            } else {
                spans.push(Span::styled(theme.adapt(c.description), desc_style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(selected.min(items.len() - 1)));
    }
    let mut block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.style(theme.muted));
    if area.width >= 36 && !theme.ascii {
        let mut title = vec![
            Span::styled("─ ", theme.style(theme.muted)),
            Span::styled(
                theme.tr("✦ Commands", "✦ 命令面板"),
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ];
        title.extend(styled_key_hint_spans(
            " · ↑↓ Tab Enter ",
            theme.style(theme.muted),
            theme,
        ));
        block = block.title(Line::from(title));
    }
    frame.render_stateful_widget(List::new(rows).block(block), area, &mut state);
}

/// Upward-docking `@target` jump drawer matching the command popup chrome.
pub fn render_mention_popup(
    frame: &mut Frame<'_>,
    anchor: Rect,
    items: &[MentionItem<'_>],
    selected: usize,
    theme: Theme,
) {
    if items.is_empty() {
        return;
    }
    let available = anchor.y.saturating_sub(frame.area().y);
    let h = (items.len() as u16 + 2).min(available);
    if h == 0 {
        return;
    }
    let area = Rect {
        x: anchor.x,
        y: anchor.y.saturating_sub(h),
        width: anchor.width.min(80),
        height: h,
    };
    frame.render_widget(Clear, area);
    let token_column = items
        .iter()
        .map(|item| item.token.width())
        .max()
        .map_or(14, |width| 14.max(width + 2));
    let bg = theme.pill_bg();
    let rows: Vec<ListItem<'_>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == selected;
            let style = if is_selected {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let match_style = if is_selected {
                style.add_modifier(Modifier::UNDERLINED)
            } else {
                theme
                    .style(theme.accent)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            };
            let mut spans =
                highlighted_spans_with_match(item.token, item.indices, style, match_style)
                    .into_iter()
                    .map(|span| Span::styled(theme.adapt(&span.content), span.style))
                    .collect::<Vec<_>>();
            spans.push(Span::styled(
                " ".repeat(token_column.saturating_sub(item.token.width())),
                style,
            ));
            spans.push(pill_span(
                format!(" {} ", item.kind_badge),
                theme.secondary,
                bg,
                theme,
                is_selected,
            ));
            if !item.detail.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    theme.adapt(item.detail),
                    if is_selected {
                        theme.style(theme.text)
                    } else {
                        theme.style(theme.muted)
                    },
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(selected.min(items.len() - 1)));
    let mut block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.style(theme.muted));
    if area.width >= 36 && !theme.ascii {
        let mut title = vec![
            Span::styled("─ ", theme.style(theme.muted)),
            Span::styled(
                theme.tr("@ Targets", "@ 跳转目标"),
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ];
        title.extend(styled_key_hint_spans(
            " · ↑↓ Tab Enter ",
            theme.style(theme.muted),
            theme,
        ));
        block = block.title(Line::from(title));
    }
    frame.render_stateful_widget(List::new(rows).block(block), area, &mut state);
}

fn ansi_basic_color(index: u8) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

fn ansi_bright_color(index: u8) -> Color {
    match index {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

fn parse_sgr_u8(raw: &str) -> Option<u8> {
    raw.parse::<u16>().ok().and_then(|v| u8::try_from(v).ok())
}

fn parse_colon_extended_color(parts: &[&str]) -> Option<Color> {
    match parts.get(1).copied() {
        Some("5") => parts
            .get(2)
            .copied()
            .and_then(parse_sgr_u8)
            .map(Color::Indexed),
        Some("2") => {
            let rgb = if parts.len() >= 6 {
                &parts[3..6]
            } else if parts.len() == 5 {
                &parts[2..5]
            } else {
                return None;
            };
            let r = parse_sgr_u8(rgb[0])?;
            let g = parse_sgr_u8(rgb[1])?;
            let b = parse_sgr_u8(rgb[2])?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn apply_sgr_colon_token(token: &str, style: &mut Style) {
    let parts: Vec<&str> = token.split(':').collect();
    match parts.first().copied() {
        Some("4") => match parts.get(1).copied() {
            Some("0") => *style = style.remove_modifier(Modifier::UNDERLINED),
            Some("1" | "2" | "3" | "4" | "5") => {
                *style = style.add_modifier(Modifier::UNDERLINED);
            }
            _ => {}
        },
        Some("38") => {
            if let Some(color) = parse_colon_extended_color(&parts) {
                *style = style.fg(color);
            }
        }
        Some("48") => {
            if let Some(color) = parse_colon_extended_color(&parts) {
                *style = style.bg(color);
            }
        }
        Some("58") => {}
        _ => {}
    }
}

fn apply_sgr_params(params: &str, style: &mut Style, base_fg: Color) {
    if params.is_empty() {
        *style = Style::default().fg(base_fg);
        return;
    }
    if !params
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b';' | b':'))
    {
        return;
    }
    let tokens: Vec<&str> = params.split(';').collect();
    let mut idx = 0;
    while idx < tokens.len() {
        let token = tokens[idx];
        if token.contains(':') {
            apply_sgr_colon_token(token, style);
            idx += 1;
            continue;
        }
        let code = if token.is_empty() {
            0
        } else if let Ok(v) = token.parse::<u16>() {
            v
        } else {
            idx += 1;
            continue;
        };
        match code {
            0 => *style = Style::default().fg(base_fg),
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            5 | 6 => *style = style.add_modifier(Modifier::SLOW_BLINK),
            7 => *style = style.add_modifier(Modifier::REVERSED),
            8 => *style = style.add_modifier(Modifier::HIDDEN),
            9 => *style = style.add_modifier(Modifier::CROSSED_OUT),
            21 | 22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            25 => *style = style.remove_modifier(Modifier::SLOW_BLINK),
            27 => *style = style.remove_modifier(Modifier::REVERSED),
            28 => *style = style.remove_modifier(Modifier::HIDDEN),
            29 => *style = style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => *style = style.fg(ansi_basic_color((code - 30) as u8)),
            39 => *style = style.fg(base_fg),
            40..=47 => *style = style.bg(ansi_basic_color((code - 40) as u8)),
            49 => *style = style.bg(Color::Reset),
            90..=97 => *style = style.fg(ansi_bright_color((code - 90) as u8)),
            100..=107 => *style = style.bg(ansi_bright_color((code - 100) as u8)),
            38 | 48 => {
                let is_fg = code == 38;
                match tokens.get(idx + 1).copied() {
                    Some("5") => {
                        if let Some(color) = tokens
                            .get(idx + 2)
                            .copied()
                            .and_then(parse_sgr_u8)
                            .map(Color::Indexed)
                        {
                            *style = if is_fg {
                                style.fg(color)
                            } else {
                                style.bg(color)
                            };
                        }
                        idx = (idx + 3).min(tokens.len());
                        continue;
                    }
                    Some("2") => {
                        if let (Some(r), Some(g), Some(b)) = (
                            tokens.get(idx + 2).copied().and_then(parse_sgr_u8),
                            tokens.get(idx + 3).copied().and_then(parse_sgr_u8),
                            tokens.get(idx + 4).copied().and_then(parse_sgr_u8),
                        ) {
                            let color = Color::Rgb(r, g, b);
                            *style = if is_fg {
                                style.fg(color)
                            } else {
                                style.bg(color)
                            };
                        }
                        idx = (idx + 5).min(tokens.len());
                        continue;
                    }
                    _ => {}
                }
            }
            58 => match tokens.get(idx + 1).copied() {
                Some("5") => {
                    idx = (idx + 3).min(tokens.len());
                    continue;
                }
                Some("2") => {
                    idx = (idx + 5).min(tokens.len());
                    continue;
                }
                _ => {}
            },
            59 => {}
            _ => {}
        }
        idx += 1;
    }
}

fn push_preview_run(
    spans: &mut Vec<Span<'static>>,
    raw_run: &mut String,
    used_width: &mut usize,
    max_width: usize,
    style: Style,
    theme: Theme,
) -> bool {
    if raw_run.is_empty() {
        return false;
    }
    let adapted = theme.adapt(raw_run);
    raw_run.clear();
    let mut out = String::new();
    let mut truncated = false;
    for grapheme in adapted.graphemes(true) {
        let gw = grapheme.width();
        if *used_width + gw > max_width {
            truncated = true;
            break;
        }
        *used_width += gw;
        out.push_str(grapheme);
    }
    if !out.is_empty() {
        let span_style = if theme.no_color {
            Style::default()
        } else {
            style
        };
        spans.push(Span::styled(out, span_style));
    }
    truncated
}

pub(super) fn parse_ansi_preview_line(raw: &str, max_width: usize, theme: Theme) -> Line<'static> {
    if max_width == 0 {
        return Line::default();
    }
    let base_fg = if theme.no_color {
        Color::Reset
    } else {
        theme.text
    };
    let mut current_style = Style::default().fg(base_fg);
    let mut spans = Vec::new();
    let mut run_buf = String::new();
    let mut used_width = 0_usize;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if push_preview_run(
                &mut spans,
                &mut run_buf,
                &mut used_width,
                max_width,
                current_style,
                theme,
            ) {
                return Line::from(spans);
            }
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    let mut final_ch = None;
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii() && (0x20..=0x3f).contains(&(next as u8)) {
                            params.push(next);
                            chars.next();
                        } else if next.is_ascii() && (0x40..=0x7e).contains(&(next as u8)) {
                            final_ch = Some(next);
                            chars.next();
                            break;
                        } else {
                            break;
                        }
                    }
                    if final_ch == Some('m') {
                        apply_sgr_params(&params, &mut current_style, base_fg);
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' || next == '\u{009c}' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('(' | ')' | '*' | '+' | '-' | '.' | '/' | '#') => {
                    chars.next();
                    if let Some(&final_ch) = chars.peek()
                        && final_ch.is_ascii()
                        && (0x30..=0x7e).contains(&(final_ch as u8))
                    {
                        chars.next();
                    }
                }
                Some(next) if next.is_ascii() && (0x20..=0x7e).contains(&(next as u8)) => {
                    chars.next();
                }
                _ => {}
            }
        } else if ch == '\t' {
            run_buf.push_str("  ");
        } else if !ch.is_control() {
            run_buf.push(ch);
        }
    }

    push_preview_run(
        &mut spans,
        &mut run_buf,
        &mut used_width,
        max_width,
        current_style,
        theme,
    );
    Line::from(spans)
}

/// Rounded card rendering captured terminal output lines cleanly without wrapping distortion.
pub fn render_pane_preview_card(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: PanePreview<'_>,
    theme: Theme,
) {
    let _ = render_pane_preview_card_inner(frame, area, preview, theme);
}

/// Interactive preview card returning geometry and clickable hit-boxes for pane cycling,
/// search, copy, log export, and live `pipe-pane` toggle.
pub fn render_pane_preview_card_interactive(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: PanePreview<'_>,
    theme: Theme,
) -> PreviewCardLayout {
    render_pane_preview_card_inner(frame, area, preview, theme)
}

#[allow(clippy::too_many_arguments)]
pub fn render_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    query: &TextInput,
    rows: &[String],
    selected: usize,
    empty: &str,
    footer: &str,
    theme: Theme,
) {
    let c = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(theme.adapt(title))
            .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        c[0],
    );
    frame.render_widget(
        Paragraph::new(query.rendered_width(true, theme, c[1].width)),
        c[1],
    );
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.adapt(empty)).style(theme.style(theme.muted)),
            c[2],
        );
    } else {
        let it = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if i == selected {
                    theme.selected_style()
                } else {
                    theme.style(theme.text)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{} {}", if i == selected { theme.marker() } else { " " }, r),
                    style,
                )))
            })
            .collect::<Vec<_>>();
        let mut st = ListState::default();
        st.select(Some(selected.min(rows.len() - 1)));
        frame.render_stateful_widget(List::new(it), c[2], &mut st);
    }
    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            footer,
            theme.style(theme.muted),
            theme,
        )),
        c[3],
    );
}

/// Structured picker row with fuzzy highlight positions, secondary metadata, and status badge.
#[derive(Debug, Clone)]
pub struct RichPickerRow {
    pub primary: String,
    pub primary_indices: Vec<usize>,
    pub secondary: String,
    pub badge: Option<(String, Color)>,
    pub is_action: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn render_rich_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    query: &TextInput,
    rows: &[RichPickerRow],
    selected: usize,
    empty: &str,
    footer: &str,
    theme: Theme,
) {
    render_rich_picker_with_preview(
        frame, area, title, query, rows, selected, empty, footer, None, theme,
    );
}

pub(super) fn build_rich_picker_list_items(
    rows: &[RichPickerRow],
    selected: usize,
    list_width: u16,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let max_primary = rows
        .iter()
        .filter(|row| !row.is_action)
        .map(|row| row.primary.width())
        .max()
        .unwrap_or(0)
        .min(usize::from(list_width) / 2);
    let bg = theme.pill_bg();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let is_sel = i == selected;
            let base_style = if is_sel {
                theme.selected_style()
            } else if row.is_action {
                theme.style(theme.secondary)
            } else {
                theme.style(theme.text)
            };
            let match_style = if is_sel {
                base_style.add_modifier(Modifier::UNDERLINED)
            } else {
                theme
                    .style(theme.accent)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            let mut spans = vec![Span::styled(format!("{marker} "), base_style)];
            spans.extend(
                highlighted_spans_with_match(
                    &row.primary,
                    &row.primary_indices,
                    base_style,
                    match_style,
                )
                .into_iter()
                .map(|span| Span::styled(theme.adapt(&span.content), span.style)),
            );
            if !row.is_action && (!row.secondary.is_empty() || row.badge.is_some()) {
                let pad = max_primary.saturating_sub(row.primary.width()) + 2;
                spans.push(Span::raw(" ".repeat(pad)));
            } else if !row.secondary.is_empty() || row.badge.is_some() {
                spans.push(Span::raw("  "));
            }
            if let Some((badge_text, badge_color)) = &row.badge {
                spans.push(pill_span(
                    format!(" {badge_text} "),
                    *badge_color,
                    bg,
                    theme,
                    is_sel,
                ));
                if !row.secondary.is_empty() {
                    spans.push(Span::raw("  "));
                }
            }
            if !row.secondary.is_empty() {
                spans.push(Span::styled(
                    theme.adapt(&row.secondary),
                    theme.style(theme.muted),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

/// Rich picker with an optional right-side live terminal preview card on wide screens (`>= 68`).
#[allow(clippy::too_many_arguments)]
pub fn render_rich_picker_with_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    query: &TextInput,
    rows: &[RichPickerRow],
    selected: usize,
    empty: &str,
    footer: &str,
    preview: Option<PanePreview<'_>>,
    theme: Theme,
) {
    let [title_area, search_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let title_text = theme.adapt(title);
    let counter = if rows.is_empty() {
        " [0]".to_owned()
    } else {
        format!(" [{}/{}]", selected.min(rows.len() - 1) + 1, rows.len())
    };
    let mut title_line = Line::from(vec![Span::styled(
        title_text,
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )]);
    if title_line.width() + counter.width() <= usize::from(title_area.width) {
        title_line
            .spans
            .push(Span::styled(counter, theme.style(theme.muted)));
    }
    frame.render_widget(Paragraph::new(title_line), title_area);

    let prompt = if search_area.width < 14 {
        "? "
    } else {
        theme.tr("Filter: ", "过滤: ")
    };
    let mut query_line = query.rendered_width(
        true,
        theme,
        search_area.width.saturating_sub(prompt.width() as u16),
    );
    if query.value().is_empty() && search_area.width >= 32 {
        let ghost = theme.adapt(theme.tr("type to narrow…", "输入关键词筛选…"));
        if prompt.width() + query_line.width() + ghost.width() <= usize::from(search_area.width) {
            query_line
                .spans
                .push(Span::styled(ghost, theme.style(theme.muted)));
        }
    }
    query_line
        .spans
        .insert(0, Span::styled(prompt, theme.style(theme.muted)));
    frame.render_widget(Paragraph::new(query_line), search_area);

    let (list_area, preview_area) = match preview {
        Some(card) if area.width >= 68 && body_area.height >= 4 => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
                    .areas(body_area);
            (l, Some((r, card)))
        }
        _ => (body_area, None),
    };

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.adapt(empty)).style(theme.style(theme.muted)),
            list_area,
        );
    } else {
        let items = build_rich_picker_list_items(rows, selected, list_area.width, theme);
        let mut state = ListState::default();
        state.select(Some(selected.min(rows.len() - 1)));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
    }

    if let Some((preview_rect, card)) = preview_area {
        render_pane_preview_card(frame, preview_rect, card, theme);
    }

    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            footer,
            theme.style(theme.muted),
            theme,
        )),
        footer_area,
    );
}

fn build_theme_list_items(
    presets: &[ThemePreset],
    clamped_selected: usize,
    active: ThemePreset,
    width: u16,
    show_tagline: bool,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    presets
        .iter()
        .enumerate()
        .map(|(i, &preset)| {
            let is_sel = i == clamped_selected;
            let is_active = preset == active;
            let candidate = theme.with_preset(preset);
            let marker = if is_sel { theme.marker() } else { " " };
            let active_mark = if is_active {
                if theme.ascii { "* " } else { "● " }
            } else {
                "  "
            };
            let name_style = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text).add_modifier(Modifier::BOLD)
            };
            let mut spans = vec![
                Span::styled(format!("{marker} "), name_style),
                Span::styled(
                    active_mark,
                    if is_active {
                        theme.style(theme.success).add_modifier(Modifier::BOLD)
                    } else {
                        theme.style(theme.muted)
                    },
                ),
            ];
            if theme.ascii {
                spans.push(Span::styled("[", theme.style(candidate.accent)));
                spans.push(Span::styled("#", theme.style(candidate.secondary)));
                spans.push(Span::styled("] ", theme.style(candidate.tertiary)));
            } else {
                spans.push(Span::styled("█", theme.style(candidate.accent)));
                spans.push(Span::styled("█", theme.style(candidate.secondary)));
                spans.push(Span::styled("█ ", theme.style(candidate.tertiary)));
            }
            let padded_name = format!("{:<19}", preset.name());
            spans.push(Span::styled(padded_name, name_style));
            if width >= 52 {
                spans.push(Span::raw(" "));
                spans.push(pill_span(
                    format!(" {} ", preset.origin()),
                    candidate.accent,
                    candidate.pill_bg(),
                    theme,
                    false,
                ));
            }
            if show_tagline && width >= 82 {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    theme.adapt(preset.tagline()),
                    theme.style(theme.muted),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

fn render_theme_live_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    preview_preset: ThemePreset,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let preview_theme = theme.with_preset(preview_preset);
    let mut preview_lines = Vec::new();
    if area.height >= 3 {
        preview_lines.push(Line::default());
    }
    let mut sample = vec![Span::styled(
        format!("{} ", preview_preset.badge_icon(theme.ascii)),
        preview_theme
            .style(preview_theme.accent)
            .add_modifier(Modifier::BOLD),
    )];
    sample.extend(gradient_spans(
        theme.tr("Preview", "实时预览"),
        preview_theme,
        true,
    ));
    sample.push(Span::styled(
        theme.adapt(&format!(" · {}", preview_preset.tagline())),
        preview_theme.style(preview_theme.muted),
    ));
    preview_lines.push(Line::from(sample));
    preview_lines.push(Line::from(vec![
        Span::styled(
            "  /resume  ",
            preview_theme
                .style(preview_theme.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            theme.adapt(theme.tr("session grid  ·  ", "会话矩阵  ·  ")),
            preview_theme.style(preview_theme.muted),
        ),
        pill_span(
            format!(" {} ", preview_preset.origin()),
            preview_theme.secondary,
            preview_theme.pill_bg(),
            preview_theme,
            false,
        ),
    ]));
    frame.render_widget(Paragraph::new(preview_lines), area);
}

/// Live-preview palette switcher showing RGB swatch blocks, origin badges, active state, and taglines.
pub fn render_theme_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    presets: &[ThemePreset],
    selected: usize,
    active: ThemePreset,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let show_preview = area.height >= 10 && !presets.is_empty();
    let [title_area, list_area, preview_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(if show_preview { 3 } else { 0 }),
        Constraint::Length(u16::from(area.height >= 3)),
    ])
    .areas(area);

    let bg = theme.pill_bg();
    let mut title_line = Line::from(gradient_spans(
        theme.tr("Theme palette", "主题调色板"),
        theme,
        true,
    ));
    let active_pill = pill_span(
        format!(" {} ", active.name()),
        theme.accent,
        bg,
        theme,
        false,
    );
    if title_line.width() + 2 + active_pill.width() <= usize::from(title_area.width) {
        title_line.spans.push(Span::raw("  "));
        title_line.spans.push(active_pill);
    }
    frame.render_widget(Paragraph::new(title_line), title_area);

    if presets.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.tr("No themes available", "暂无可用主题"))
                .style(theme.style(theme.muted)),
            list_area,
        );
    } else {
        let clamped_selected = selected.min(presets.len() - 1);
        let items = build_theme_list_items(
            presets,
            clamped_selected,
            active,
            area.width,
            !show_preview,
            theme,
        );
        let mut state = ListState::default();
        state.select(Some(clamped_selected));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);

        if show_preview {
            render_theme_live_preview(frame, preview_area, presets[clamped_selected], theme);
        }
    }

    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            theme.tr(
                "↑↓ Live Preview · Enter Apply · s Save to config.toml · Esc Revert",
                "↑↓ 实时预览 · Enter 应用 · s 保存到 config.toml · Esc 还原",
            ),
            theme.style(theme.muted),
            theme,
        )),
        footer_area,
    );
}

/// Interactive row entry rendered inside [`render_config_modal`].
#[derive(Debug, Clone)]
pub struct ConfigItemView<'a> {
    pub key: &'a str,
    pub value: String,
    pub description: &'a str,
}

fn build_config_list_items(
    items: &[ConfigItemView<'_>],
    clamped_selected: usize,
    list_width: u16,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let muted = theme.style(theme.muted);
    let key_col = items
        .iter()
        .map(|item| item.key.width())
        .max()
        .unwrap_or(10)
        .clamp(10, 22);
    let bg = theme.pill_bg();
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == clamped_selected;
            let marker = if is_sel { theme.marker() } else { " " };
            let key_style = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text).add_modifier(Modifier::BOLD)
            };
            let padded_key = format!("{:<width$}", item.key, width = key_col);
            let val_fg = if is_sel {
                theme.accent
            } else {
                theme.secondary
            };
            let pill = pill_span(format!(" {} ", item.value), val_fg, bg, theme, is_sel);
            let pill_w = pill.width();
            let mut spans = vec![
                Span::styled(format!("{marker} "), key_style),
                Span::styled(padded_key, key_style),
                Span::raw("  "),
                pill,
            ];
            let used = 2 + key_col + 2 + pill_w;
            let desc_avail = usize::from(list_width).saturating_sub(used + 2);
            if desc_avail >= 4 && !item.description.is_empty() {
                spans.push(Span::raw("  "));
                spans.extend(styled_key_hint_spans(
                    &truncate_to_width(&theme.adapt(item.description), desc_avail),
                    muted,
                    theme,
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

/// Interactive `/config` settings studio modal displaying active file path, toggles, and save hint.
#[allow(clippy::too_many_arguments)]
pub fn render_config_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    path_label: &str,
    items: &[ConfigItemView<'_>],
    selected: usize,
    status_note: Option<&str>,
    theme: Theme,
) {
    if area.width < 12 || area.height < 6 {
        return;
    }
    let w = area.width.min(90);
    let h = area.height.min(18);
    let card_area = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, card_area);

    let title_text = theme.adapt(theme.tr(" ⚙ JustMux Configuration ", " ⚙ JustMux 设置与参数 "));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.accent))
        .title(Line::from(Span::styled(
            title_text,
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )));
    let raw_inner = block.inner(card_area);
    frame.render_widget(block, card_area);
    if raw_inner.width == 0 || raw_inner.height < 3 {
        return;
    }
    let inner = if raw_inner.width >= 8 {
        Rect {
            x: raw_inner.x + 1,
            width: raw_inner.width - 2,
            ..raw_inner
        }
    } else {
        raw_inner
    };

    let show_spacer = inner.height >= 8;
    let show_status = status_note.is_some() && inner.height >= 6;
    let [path_area, _spacer, list_area, status_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(show_spacer)),
        Constraint::Min(1),
        Constraint::Length(u16::from(show_status)),
        Constraint::Length(1),
    ])
    .areas(inner);

    let muted = theme.style(theme.muted);
    let path_prefix = theme.tr("Config: ", "配置文件: ");
    let path_marker = if theme.ascii { "..." } else { "…" };
    let clean_path = single_line(path_label);
    let trimmed_path = directory_tail(
        &clean_path,
        usize::from(path_area.width).saturating_sub(path_prefix.width()),
        path_marker,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(path_prefix, muted),
            Span::styled(trimmed_path, theme.style(theme.secondary)),
        ])),
        path_area,
    );

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.tr("No settings available", "暂无设置项")).style(muted),
            list_area,
        );
    } else {
        let clamped_selected = selected.min(items.len() - 1);
        let list_items = build_config_list_items(items, clamped_selected, list_area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped_selected));
        frame.render_stateful_widget(List::new(list_items), list_area, &mut state);
    }

    if show_status && let Some(note) = status_note {
        frame.render_widget(
            Paragraph::new(truncate_to_width(
                &theme.adapt(note),
                usize::from(status_area.width),
            ))
            .style(status_style(note, theme)),
            status_area,
        );
    }

    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            &truncate_to_width(
                &theme.adapt(theme.tr(
                    "↑↓ Select · ←→/Enter Change · s Save to config.toml · Esc Close",
                    "↑↓ 选择 · ←→/Enter 切换 · s 保存到 config.toml · Esc 关闭",
                )),
                usize::from(footer_area.width),
            ),
            muted,
            theme,
        )),
        footer_area,
    );
}

/// Content model for a tribute Easter-egg modal card.
#[derive(Debug, Clone)]
pub struct TributeModal<'a> {
    pub badge: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub telemetry: &'a str,
    pub art: &'a [&'a str],
    pub highlights: &'a [&'a str],
    pub status_quip: &'a str,
    pub footer_hint: &'a str,
}

/// Render a rounded tribute card celebrating classic AI TUI interfaces.
pub fn render_tribute_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    modal: TributeModal<'_>,
    theme: Theme,
) {
    if area.width < 6 || area.height < 4 {
        return;
    }
    let w = area.width.min(86);
    let h = area.height.min(18);
    let card_area = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, card_area);

    let border_type = if theme.ascii {
        BorderType::Plain
    } else {
        BorderType::Rounded
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(theme.style(theme.accent))
        .title(Line::from(vec![
            Span::raw(" "),
            pill_span(modal.badge, theme.accent, theme.pill_bg(), theme, true),
            Span::raw(" "),
        ]));
    let inner = block.inner(card_area);
    frame.render_widget(block, card_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let compact = area.height <= 12;
    let footer_rows = u16::from(inner.height >= 2 && !modal.footer_hint.is_empty());
    let [body_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(footer_rows)]).areas(inner);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut title_line = Line::from(gradient_spans(modal.title, theme, true));
    let subtitle = theme.adapt(modal.subtitle);
    if !subtitle.is_empty()
        && title_line.width() + 2 + subtitle.width() <= usize::from(body_area.width)
    {
        title_line.spans.push(Span::raw("  "));
        title_line
            .spans
            .push(Span::styled(subtitle.clone(), theme.style(theme.muted)));
    }
    lines.push(title_line);

    if !subtitle.is_empty()
        && lines
            .first()
            .is_some_and(|line| !line.spans.iter().any(|s| s.content == subtitle))
        && usize::from(body_area.height) > lines.len() + 2
    {
        lines.push(Line::from(Span::styled(subtitle, theme.style(theme.muted))));
    }

    if !modal.telemetry.is_empty() {
        lines.push(Line::from(Span::styled(
            theme.adapt(modal.telemetry),
            theme.style(theme.secondary),
        )));
    }

    let max_art = if compact {
        if body_area.height >= 8 { 1 } else { 0 }
    } else {
        modal.art.len()
    };
    if max_art > 0 && !modal.art.is_empty() {
        if !compact && body_area.height >= 12 {
            lines.push(Line::default());
        }
        for art_line in modal.art.iter().take(max_art) {
            lines.push(Line::from(Span::styled(
                theme.adapt(art_line),
                theme.style(theme.accent),
            )));
        }
    }

    if !compact && body_area.height >= 10 {
        lines.push(Line::default());
    }
    let bullet = if theme.ascii { "  - " } else { "  • " };
    let reserved_quip = usize::from(!modal.status_quip.is_empty());
    for highlight in modal.highlights {
        if lines.len() + reserved_quip >= usize::from(body_area.height) {
            break;
        }
        lines.push(Line::from(vec![
            Span::styled(bullet, theme.style(theme.accent)),
            Span::styled(theme.adapt(highlight), theme.style(theme.text)),
        ]));
    }

    if !modal.status_quip.is_empty() && lines.len() < usize::from(body_area.height) {
        if !compact && lines.len() + 2 <= usize::from(body_area.height) {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            theme.adapt(modal.status_quip),
            theme.style(theme.success).add_modifier(Modifier::BOLD),
        )));
    }

    frame.render_widget(Paragraph::new(lines), body_area);
    if footer_rows > 0 {
        frame.render_widget(
            Paragraph::new(styled_key_hint_line(
                modal.footer_hint,
                theme.style(theme.muted),
                theme,
            )),
            footer_area,
        );
    }
}

pub fn render_form(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    label: &str,
    input: &TextInput,
    footer: &str,
    theme: Theme,
) {
    render_form_scrolled(frame, area, title, label, input, footer, theme, 0);
}

/// A wrapped description scrolls independently of the pinned input/footer.
/// Returns the clamped offset so callers can retain it between redraws.
#[allow(clippy::too_many_arguments)]
pub fn render_form_scrolled(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    label: &str,
    input: &TextInput,
    footer: &str,
    theme: Theme,
    scroll: u16,
) -> u16 {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(theme.adapt(title))
            .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        chunks[0],
    );
    let lines = wrap_description(&theme.adapt(label), chunks[1].width);
    let max_scroll = lines.len().saturating_sub(usize::from(chunks[1].height));
    let description = Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>());
    let scroll = scroll.min(u16::try_from(max_scroll).unwrap_or(u16::MAX));
    frame.render_widget(description.scroll((scroll, 0)), chunks[1]);
    frame.render_widget(
        Paragraph::new(input.rendered_width(true, theme, chunks[2].width)),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            footer,
            theme.style(theme.muted),
            theme,
        )),
        chunks[3],
    );
    scroll
}

/// Codex-style rounded approval card with diff summary, scrollable warning body, and pinned input.
#[allow(clippy::too_many_arguments)]
pub fn render_confirm_card(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    target_summary: &str,
    message: &str,
    input: &TextInput,
    footer: &str,
    theme: Theme,
    scroll: u16,
) -> u16 {
    if area.width < 44 || area.height < 9 {
        return render_form_scrolled(frame, area, title, message, input, footer, theme, scroll);
    }
    let w = area.width.min(82);
    let h = area.height.min(13);
    let card_area = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, card_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.error))
        .title(Line::from(Span::styled(
            format!(" {} ", theme.adapt(title)),
            theme.style(theme.error).add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(card_area);
    frame.render_widget(block, card_area);
    if inner.width == 0 || inner.height < 5 {
        return 0;
    }

    let [banner_area, diff_area, msg_area, input_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let warn_icon = if theme.ascii { "!" } else { "⚠" };
    let sep = if theme.ascii { " | " } else { " · " };
    let banner = Line::from(vec![
        Span::styled(
            format!(
                "{warn_icon} {}",
                theme.tr("Approval Required", "需要危险操作确认")
            ),
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        ),
        Span::styled(sep, theme.style(theme.muted)),
        pill_span(
            theme.tr("[sandbox: native-tmux]", "[沙箱: native-tmux]"),
            theme.secondary,
            theme.pill_bg(),
            theme,
            false,
        ),
    ]);
    frame.render_widget(Paragraph::new(banner), banner_area);

    let diff_text = format!(
        "- {} {}",
        theme.tr("kill", "关闭"),
        single_line(target_summary)
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(&theme.adapt(&diff_text), usize::from(diff_area.width)),
            theme.style(theme.error).add_modifier(Modifier::BOLD),
        ))),
        diff_area,
    );

    let lines = wrap_description(&theme.adapt(message), msg_area.width);
    let max_scroll = lines.len().saturating_sub(usize::from(msg_area.height));
    let scroll = scroll.min(u16::try_from(max_scroll).unwrap_or(u16::MAX));
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(theme.style(theme.text))
            .scroll((scroll, 0)),
        msg_area,
    );

    let marker = format!("{} ", theme.marker());
    let mut input_line = input.rendered_width(
        true,
        theme,
        input_area.width.saturating_sub(marker.width() as u16),
    );
    input_line.spans.insert(
        0,
        Span::styled(
            marker,
            theme.style(theme.error).add_modifier(Modifier::BOLD),
        ),
    );
    frame.render_widget(Paragraph::new(input_line), input_area);

    frame.render_widget(
        Paragraph::new(styled_key_hint_line(
            footer,
            theme.style(theme.muted),
            theme,
        )),
        footer_area,
    );
    scroll
}

pub(super) fn wrap_description(text: &str, width: u16) -> Vec<String> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_word_bounds() {
            if line.width() + word.width() > width && !line.is_empty() {
                lines.push(line.trim_end().to_owned());
                line.clear();
            }
            if word.trim().is_empty() && line.is_empty() {
                continue;
            }
            for grapheme in word.graphemes(true) {
                if line.width() + grapheme.width() > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                line.push_str(grapheme);
            }
        }
        lines.push(line);
    }
    lines
}

pub fn render_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    message: &str,
    yes: bool,
    theme: Theme,
) {
    let w = area.width.min(70);
    let h = 7;
    let r = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, r);
    frame.render_widget(
        Block::default()
            .title(theme.adapt(title))
            .borders(Borders::ALL)
            .border_type(if theme.ascii {
                BorderType::Plain
            } else {
                BorderType::Rounded
            })
            .style(theme.style(theme.error)),
        r,
    );
    let inner = Rect {
        x: r.x + 2,
        y: r.y + 1,
        width: w.saturating_sub(4),
        height: h.saturating_sub(2),
    };
    let choices = if yes {
        Line::from(vec![
            Span::styled(
                theme.tr("[ Yes ]", "[ 是 ]"),
                theme.style(theme.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(theme.tr("    No", "    否"), theme.style(theme.muted)),
        ])
    } else {
        Line::from(vec![
            Span::styled(theme.tr("  Yes    ", "  是    "), theme.style(theme.muted)),
            Span::styled(
                theme.tr("[ No ]", "[ 否 ]"),
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(theme.adapt(message)),
            Line::from(""),
            choices,
        ])
        .wrap(Wrap { trim: true })
        .style(theme.style(theme.text)),
        inner,
    );
}

pub fn render_help(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cmd_style = theme.style(theme.success).add_modifier(Modifier::BOLD);
    let sec_style = theme.style(theme.secondary).add_modifier(Modifier::BOLD);
    let muted = theme.style(theme.muted);
    let text_style = theme.style(theme.text);

    if area.width >= 68 {
        let mut lines = Vec::new();
        let icon = if theme.ascii { "[] " } else { "◫ " };
        let mut header = vec![Span::styled(
            icon,
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )];
        header.extend(gradient_spans(
            theme.tr("JustMux commands", "JustMux 命令"),
            theme,
            true,
        ));
        header.push(Span::styled(
            theme.adapt(theme.tr("  ·  zero-memorization tmux", "  ·  免记忆 tmux")),
            muted,
        ));
        lines.push(Line::from(header));
        if area.height >= 17 {
            lines.push(Line::default());
        }
        let left_head = theme.tr("Workspace & Exit", "工作区与会话操作");
        let left_col_w = 37_usize;
        let left_head_pad = " ".repeat(left_col_w.saturating_sub(left_head.width()));
        lines.push(Line::from(vec![
            Span::styled(format!("{left_head}{left_head_pad}"), sec_style),
            Span::styled(theme.tr("Studio & System", "工作台与系统"), sec_style),
        ]));
        let wide_pairs_full: [(&str, &str, &str, &str); 10] = [
            (
                "/resume",
                theme.tr("session & window grid", "会话与窗口网格"),
                "/pane",
                theme.tr("zoom, break, swap & close", "缩放、拆出、互换与关闭"),
            ),
            (
                "/new",
                theme.tr("create a tmux session", "新建 tmux 会话"),
                "/split",
                theme.tr("2x2 visual Split Studio", "2x2 可视化分屏工作台"),
            ),
            (
                "/window",
                theme.tr("move, link & manage windows", "移动、链接与管理窗口"),
                "/layout",
                theme.tr("presets & live pane resize", "布局预设与实时调大小"),
            ),
            (
                "/rename",
                theme.tr("rename session or window", "重命名会话或窗口"),
                "/expose",
                theme.tr("live Mission Control wall", "实时多窗格全景任务墙"),
            ),
            (
                "/kill",
                theme.tr("kill session or window", "关闭会话或窗口"),
                "/buffers",
                theme.tr("browse, copy & paste buffers", "浏览复制粘贴缓冲区"),
            ),
            (
                "/snapshot",
                theme.tr("save & restore workspaces", "保存与恢复工作区快照"),
                "/popup",
                theme.tr("floating scratchpad (3.2+)", "浮动临时终端 (3.2+)"),
            ),
            (
                "/list",
                theme.tr("browse sessions & actions", "浏览会话与操作"),
                "/clients",
                theme.tr("clients & fix dotted border", "客户端与修复虚线框"),
            ),
            (
                "/exit",
                theme.tr("detach tmux client", "分离 tmux 客户端"),
                "/theme",
                theme.tr("switch TrueColor palettes", "切换真彩色主题"),
            ),
            (
                "/kill-server",
                theme.tr("stop entire tmux server", "停止整个 tmux 服务"),
                "/config",
                theme.tr("settings & save config.toml", "设置与保存 config.toml"),
            ),
            (
                "/quit",
                theme.tr("quit JustMux", "退出 JustMux"),
                "/help",
                theme.tr("show this guide", "显示此帮助指南"),
            ),
        ];
        let wide_pairs_tight: [(&str, &str, &str, &str); 8] = [
            wide_pairs_full[0],
            wide_pairs_full[1],
            wide_pairs_full[2],
            wide_pairs_full[3],
            wide_pairs_full[4],
            wide_pairs_full[5],
            (
                "/list",
                theme.tr("browse sessions (/exit)", "浏览会话 (/exit 分离)"),
                "/clients",
                theme.tr("clients · /theme · /config", "客户端 · /theme · /config"),
            ),
            wide_pairs_full[9],
        ];
        let pairs: &[(&str, &str, &str, &str)] = if area.height >= 15 {
            &wide_pairs_full
        } else {
            &wide_pairs_tight
        };
        for &(l_cmd, l_desc, r_cmd, r_desc) in pairs {
            let l_desc = theme.adapt(l_desc);
            let r_desc = theme.adapt(r_desc);
            let left_pad = 10_usize.saturating_sub(l_cmd.width()).max(1);
            let right_col_pad = left_col_w
                .saturating_sub(l_cmd.width() + left_pad + l_desc.width())
                .max(2);
            let right_pad = 10_usize.saturating_sub(r_cmd.width()).max(1);
            lines.push(Line::from(vec![
                Span::styled(l_cmd, cmd_style),
                Span::raw(" ".repeat(left_pad)),
                Span::styled(l_desc, text_style),
                Span::raw(" ".repeat(right_col_pad)),
                Span::styled(r_cmd, cmd_style),
                Span::raw(" ".repeat(right_pad)),
                Span::styled(r_desc, text_style),
            ]));
        }
        if area.height >= 16 {
            lines.push(Line::default());
        }
        lines.push(Line::from(vec![
            Span::styled(theme.tr("Tributes  ", "致敬彩蛋  "), muted),
            Span::styled(
                theme.adapt("/codex · /claude · /antigravity · /vibe"),
                theme.style(theme.accent),
            ),
        ]));
        if area.height >= 13 {
            let mut preview_spans = vec![Span::styled(theme.tr("Preview   ", "实时预览  "), muted)];
            preview_spans.extend(styled_key_hint_spans(
                theme.tr(
                    "PgUp / PgDn · f search · y copy · [/] panes · Wheel scroll",
                    "PgUp / PgDn · f 搜索 · y 复制 · [/] 切窗格 · 滚轮滚动",
                ),
                muted,
                theme,
            ));
            lines.push(Line::from(preview_spans));
        }
        lines.push(styled_key_hint_line(
            theme.tr(
                "Esc back · ↑↓ navigate · Tab complete · Enter select · Ctrl+T theme · Ctrl+C quit",
                "Esc 返回 · ↑↓ 导航 · Tab 补全 · Enter 选择 · Ctrl+T 主题 · Ctrl+C 退出",
            ),
            muted,
            theme,
        ));
        frame.render_widget(Paragraph::new(lines), area);
    } else {
        let mut lines = vec![Line::from(gradient_spans(
            theme.tr("JustMux commands", "JustMux 命令"),
            theme,
            true,
        ))];
        let entries = [
            // Workspace
            (
                "/resume",
                theme.tr("session & window grid", "会话与窗口网格"),
            ),
            ("/new", theme.tr("create a session", "新建会话")),
            ("/window", theme.tr("manage windows", "管理窗口")),
            ("/rename", theme.tr("rename a session", "重命名会话")),
            ("/kill", theme.tr("terminate a session", "终止会话")),
            (
                "/snapshot",
                theme.tr("save/restore workspace", "工作区快照"),
            ),
            (
                "/list",
                theme.tr("browse sessions & actions", "浏览会话与操作"),
            ),
            // Studio
            ("/pane", theme.tr("manage panes", "管理窗格")),
            (
                "/split",
                theme.tr("2x2 visual Split Studio", "2x2 可视化分屏工作台"),
            ),
            ("/layout", theme.tr("layout & pane resize", "布局与调大小")),
            (
                "/expose",
                theme.tr("live Mission Control wall", "实时全景任务墙"),
            ),
            (
                "/buffers",
                theme.tr("paste buffer manager", "粘贴缓冲区管理"),
            ),
            ("/popup", theme.tr("floating scratchpad", "浮动临时终端")),
            ("/theme", theme.tr("switch theme palette", "切换主题调色板")),
            (
                "/config",
                theme.tr("settings & save config.toml", "设置并保存 config.toml"),
            ),
            // System
            (
                "/clients",
                theme.tr("clients & dotted border", "客户端与虚线框"),
            ),
            ("/help", theme.tr("show this help", "显示此帮助")),
            ("/exit", theme.tr("detach from tmux", "分离 tmux")),
            (
                "/kill-server",
                theme.tr("stop entire tmux server", "停止整个 tmux 服务"),
            ),
            ("/quit", theme.tr("quit JustMux", "退出 JustMux")),
        ];
        for (cmd, desc) in entries {
            let pad = 13_usize.saturating_sub(cmd.width()).max(1);
            lines.push(Line::from(vec![
                Span::styled(cmd, cmd_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(theme.adapt(desc), text_style),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(theme.tr("Also: ", "彩蛋: "), muted),
            Span::styled(
                theme.adapt("/codex · /claude · /antigravity · /vibe"),
                theme.style(theme.accent),
            ),
        ]));
        let mut preview_spans = vec![Span::styled(theme.tr("Preview: ", "预览: "), muted)];
        preview_spans.extend(styled_key_hint_spans(
            theme.tr(
                "PgUp / PgDn · f search · y copy · [/] panes",
                "PgUp / PgDn · f 搜索 · y 复制 · [/] 切窗格",
            ),
            muted,
            theme,
        ));
        lines.push(Line::from(preview_spans));
        lines.push(styled_key_hint_line(
            theme.tr(
                "Esc back · ↑↓ navigate · Enter select · Ctrl+C quit",
                "Esc 返回 · ↑↓ 导航 · Enter 选择 · Ctrl+C 退出",
            ),
            muted,
            theme,
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn unicode_editing() {
        let mut i = TextInput::new("你好");
        i.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        i.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(i.value(), "好");
    }

    #[test]
    fn insert() {
        let mut i = TextInput::default();
        i.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(i.value(), "/");
    }

    #[test]
    fn combining_mark_does_not_break_cursor() {
        let mut i = TextInput::new("e");
        i.handle_key(KeyEvent::new(KeyCode::Char('\u{301}'), KeyModifiers::NONE));
        assert_eq!(i.value(), "e\u{301}");
        assert_eq!(i.cursor(), 1);
        i.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(i.value(), "");
    }

    #[test]
    fn emacs_editing_shortcuts() {
        let mut i = TextInput::new("one two");
        i.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.value(), "one ");
        i.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(i.value(), "");
    }

    #[test]
    fn narrow_input_keeps_cursor_visible() {
        let i = TextInput::new("a长long");
        let line = i.rendered_width(true, Theme::default(), 4);
        assert_eq!(line.width(), 4);
        assert!(
            line.spans
                .last()
                .is_some_and(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        );
    }

    #[test]
    fn command_popup_renders_on_test_backend() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::new("/res"),
                    &[CommandItem {
                        name: "resume",
                        description: "Resume session",
                        alias: None,
                        indices: &[0, 1, 2],
                    }],
                    0,
                    HomeDetails::default(),
                    Theme::default(),
                );
            })
            .expect("draw");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("/resume"));
        assert!(rendered.contains("/res"));
    }

    #[test]
    fn theme_default_and_presets_preserve_invariants() {
        let def = Theme::default();
        assert_eq!(def.preset, ThemePreset::Classic);
        assert!(!def.truecolor);
        assert_eq!(def.accent, Color::Cyan);
        assert_eq!(def.success, Color::Green);
        assert_eq!(def.text, Color::Reset);
        assert_eq!(def.muted, Color::DarkGray);
        assert_eq!(def.error, Color::Red);
        assert_eq!(def.selected, Color::Cyan);
        assert!(!def.no_color);
        assert!(!def.ascii);

        assert_eq!(ThemePreset::ALL.len(), 5);
        for &preset in ThemePreset::ALL {
            assert_eq!(ThemePreset::from_name(preset.slug()), Some(preset));
            assert_eq!(ThemePreset::from_name(preset.name()), Some(preset));
            assert_eq!(ThemePreset::from_name(preset.origin()), Some(preset));
            assert!(!preset.tagline().is_empty());
            assert!(preset.badge_icon(true).is_ascii());
        }
        assert_eq!(
            ThemePreset::from_name("agy"),
            Some(ThemePreset::AntigravityAurora)
        );
        assert_eq!(
            ThemePreset::from_name("EMERALD"),
            Some(ThemePreset::CodexEmerald)
        );
        assert_eq!(
            ThemePreset::from_name("warmth"),
            Some(ThemePreset::ClaudeWarmth)
        );
        assert_eq!(
            ThemePreset::from_name("cyber"),
            Some(ThemePreset::CyberMatrix)
        );
        assert_eq!(ThemePreset::from_name("unknown-theme"), None);

        let mut p = ThemePreset::ALL[0];
        for _ in 0..ThemePreset::ALL.len() {
            p = p.next();
        }
        assert_eq!(p, ThemePreset::ALL[0]);
    }

    #[test]
    fn lerp_gradient_and_pill_respect_truecolor_and_no_color() {
        assert_eq!(
            lerp_rgb((10, 20, 30), (110, 220, 230), 0, 10),
            Color::Rgb(10, 20, 30)
        );
        assert_eq!(
            lerp_rgb((10, 20, 30), (110, 220, 230), 10, 10),
            Color::Rgb(110, 220, 230)
        );
        assert_eq!(
            lerp_rgb((0, 100, 200), (100, 200, 0), 5, 10),
            Color::Rgb(50, 150, 100)
        );

        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        assert!(aurora.truecolor);
        let spans = gradient_spans("JustMux", aurora, true);
        assert_eq!(spans.len(), 7);
        assert!(
            spans
                .iter()
                .all(|s| matches!(s.style.fg, Some(Color::Rgb(_, _, _))))
        );

        let no_color = Theme {
            no_color: true,
            ..aurora
        };
        assert!(!no_color.with_preset(ThemePreset::CodexEmerald).truecolor);
        let fallback = gradient_spans("JustMux", no_color, true);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].style.fg, None);
        assert_eq!(
            pill_span(
                "aurora",
                aurora.accent,
                Some(aurora.surface),
                no_color,
                true
            ),
            Span::raw("aurora")
        );
    }

    #[test]
    fn home_dashboard_status_colors_and_composer_placeholder() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).expect("terminal");
        terminal
            .draw(|frame| {
                render_home_dashboard(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/repo"),
                        context: "popup",
                        session: Some("work"),
                        status: Some("✦ Antigravity lifted off"),
                    },
                    HomeTelemetry {
                        sessions: Some(3),
                        windows: Some(8),
                        attached: Some(1),
                        clients: None,
                        hint_badge: Some("✦ aurora"),
                    },
                    aurora,
                );
            })
            .expect("draw");
        let output = terminal.backend().to_string();
        assert!(output.contains("JustMux"));
        assert!(output.contains("3s · 8w · 1●"));
        assert!(output.contains("@jump"));
        assert!(output.contains("↓ menu"));
        assert!(output.contains("✦ Antigravity lifted off"));
        assert!(!output.contains("Type / for commands"));
        let buffer = terminal.backend().buffer();
        let bottom_y = 15;
        assert_eq!(buffer[(0, bottom_y)].fg, aurora.accent);
        assert_ne!(buffer[(0, bottom_y)].fg, aurora.error);
    }

    #[test]
    fn theme_picker_rich_picker_and_tribute_modal_render_cleanly() {
        let theme = Theme::default().with_preset(ThemePreset::CodexEmerald);
        let mut terminal = Terminal::new(TestBackend::new(72, 16)).expect("terminal");

        terminal
            .draw(|frame| {
                render_theme_picker(
                    frame,
                    frame.area(),
                    ThemePreset::ALL,
                    1,
                    ThemePreset::AntigravityAurora,
                    theme,
                )
            })
            .expect("theme picker");
        let theme_text = terminal.backend().to_string();
        assert!(theme_text.contains("███"));
        assert!(theme_text.contains("OpenAI Codex"));

        terminal
            .draw(|frame| {
                render_rich_picker(
                    frame,
                    frame.area(),
                    "Sessions",
                    &TextInput::default(),
                    &[
                        RichPickerRow {
                            primary: "orion".into(),
                            primary_indices: vec![0, 1],
                            secondary: "3 windows".into(),
                            badge: Some(("attached".into(), theme.success)),
                            is_action: false,
                        },
                        RichPickerRow {
                            primary: "+ New window".into(),
                            primary_indices: vec![],
                            secondary: String::new(),
                            badge: None,
                            is_action: true,
                        },
                    ],
                    0,
                    "Empty",
                    "Enter Select",
                    theme,
                )
            })
            .expect("rich picker");
        let picker_text = terminal.backend().to_string();
        assert!(picker_text.contains("[1/2]"));
        assert!(picker_text.contains("Filter:"));
        assert!(picker_text.contains("attached"));

        terminal
            .draw(|frame| {
                render_tribute_modal(
                    frame,
                    frame.area(),
                    TributeModal {
                        badge: "✦ ANTIGRAVITY",
                        title: "Google DeepMind Antigravity",
                        subtitle: "Agentic Velocity",
                        telemetry: "gravity: 0.00g · mode: plan->swarm",
                        art: &["  /\\  ", " /✦ \\ ", "/____\\"],
                        highlights: &["Zero-gravity panes", "Swarm choreography"],
                        status_quip: "✦ Lifting off...",
                        footer_hint: "Enter Apply · Esc Back",
                    },
                    theme,
                )
            })
            .expect("tribute modal");
        let modal_text = terminal.backend().to_string();
        assert!(modal_text.contains("╭"));
        assert!(modal_text.contains("ANTIGRAVITY"));
        assert!(modal_text.contains("Lifting off"));

        let ascii_no_color = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        terminal
            .draw(|frame| {
                render_theme_picker(
                    frame,
                    frame.area(),
                    ThemePreset::ALL,
                    1,
                    ThemePreset::AntigravityAurora,
                    ascii_no_color,
                )
            })
            .expect("ascii theme picker");
        assert!(terminal.backend().to_string().is_ascii());
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset)
        );
    }

    #[test]
    fn command_popup_uses_accent_on_unselected_matches_and_preserves_underlines() {
        let theme = Theme::default().with_preset(ThemePreset::ClaudeWarmth);
        let mut terminal = Terminal::new(TestBackend::new(56, 12)).expect("terminal");
        terminal
            .draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::new("/r"),
                    &[
                        CommandItem {
                            name: "resume",
                            description: "Open window",
                            alias: Some("r"),
                            indices: &[0],
                        },
                        CommandItem {
                            name: "rename",
                            description: "Rename session",
                            alias: None,
                            indices: &[0],
                        },
                    ],
                    0,
                    HomeDetails::default(),
                    theme,
                );
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        let underlined: Vec<_> = buf
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::UNDERLINED))
            .collect();
        assert_eq!(
            underlined.iter().map(|c| c.symbol()).collect::<String>(),
            "rr"
        );
        assert_eq!(underlined[1].fg, theme.accent);
        assert!(terminal.backend().to_string().contains("✦ Commands"));
    }

    #[test]
    fn tribute_modal_compacts_at_height_12_and_help_renders_all_sections() {
        let theme = Theme::default().with_preset(ThemePreset::CyberMatrix);
        let mut compact = Terminal::new(TestBackend::new(64, 10)).expect("compact terminal");
        compact
            .draw(|frame| {
                render_tribute_modal(
                    frame,
                    frame.area(),
                    TributeModal {
                        badge: "◈ TRIBUTE VAULT",
                        title: "Hall of Fame",
                        subtitle: "TUI Titans",
                        telemetry: "palettes: 5 · secrets: active",
                        art: &["line1", "line2", "line3"],
                        highlights: &["Codex CLI", "Claude Code", "Antigravity"],
                        status_quip: "◈ Matrix synced",
                        footer_hint: "Enter Apply · Esc Back",
                    },
                    theme,
                )
            })
            .expect("compact modal");
        let text = compact.backend().to_string();
        assert!(text.contains("Hall of Fame"));
        assert!(text.contains("palettes: 5"));
        assert!(text.contains("Matrix synced"));
        assert!(text.contains("Esc Back"));

        let mut wide_help = Terminal::new(TestBackend::new(80, 13)).expect("wide help");
        wide_help
            .draw(|frame| render_help(frame, frame.area(), theme))
            .expect("wide help draw");
        let help_text = wide_help.backend().to_string();
        assert!(help_text.contains("/codex · /claude · /antigravity · /vibe"));
        assert!(help_text.contains("/split"));

        let ascii_theme = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        wide_help
            .draw(|frame| render_help(frame, frame.area(), ascii_theme))
            .expect("ascii help draw");
        assert!(wide_help.backend().to_string().is_ascii());
    }

    #[test]
    fn footer_truncation_and_positive_vs_error_status_colors() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        for width in [72_u16, 46, 30, 20] {
            let mut term = Terminal::new(TestBackend::new(width, 6)).expect("term");
            term.draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails::default(),
                    theme,
                );
            })
            .expect("draw");
            assert!(
                term.backend().to_string().contains("Type / for commands"),
                "width {width} dropped Type / for commands"
            );
        }

        let mut term = Terminal::new(TestBackend::new(60, 6)).expect("term");
        for (msg, expected_fg) in [
            ("Session created.", theme.success),
            ("Renamed", theme.success),
            ("Cancelled. Nothing was killed.", theme.accent),
            ("unknown command: /wat", theme.error),
        ] {
            term.draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        status: Some(msg),
                        ..HomeDetails::default()
                    },
                    theme,
                );
            })
            .expect("draw status");
            assert_eq!(term.backend().buffer()[(0, 5)].fg, expected_fg);
        }
        for ok_accent in [
            "Reset 2x2 split layout",
            "Flipped split layout",
            "分屏画布已重置",
            "已翻转分屏方向",
        ] {
            assert_eq!(status_style(ok_accent, theme).fg, Some(theme.accent));
        }
        assert_eq!(
            status_style("unknown command: /wat", theme).fg,
            Some(theme.error)
        );
    }

    #[test]
    fn composer_mode_mascot_and_adapt_preserve_ascii_and_invariants() {
        assert_eq!(ComposerMode::CommandJump.next(), ComposerMode::LiveRadar);
        assert_eq!(ComposerMode::LiveRadar.next(), ComposerMode::QuickLaunch);
        assert_eq!(ComposerMode::QuickLaunch.next(), ComposerMode::CommandJump);

        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        assert_eq!(
            ComposerMode::CommandJump.border_color(aurora),
            aurora.accent
        );
        assert_eq!(
            ComposerMode::LiveRadar.border_color(aurora),
            aurora.secondary
        );
        assert_eq!(
            ComposerMode::QuickLaunch.border_color(aurora),
            aurora.warning
        );

        let ascii_theme = Theme {
            ascii: true,
            no_color: true,
            ..aurora
        };
        for mode in [
            ComposerMode::CommandJump,
            ComposerMode::LiveRadar,
            ComposerMode::QuickLaunch,
        ] {
            for ascii in [false, true] {
                assert!(!mode.placeholder(ascii).contains("Type / for commands"));
                assert!(!mode.label(ascii).is_empty());
            }
            assert!(mode.label(true).is_ascii());
            assert!(mode.placeholder(true).is_ascii());
            assert!(ascii_theme.adapt(mode.label(false)).is_ascii());
            assert!(ascii_theme.adapt(mode.placeholder(false)).is_ascii());
        }

        for &preset in ThemePreset::ALL {
            let unicode_theme = Theme::default().with_preset(preset);
            let lines = mascot_lines(unicode_theme);
            assert_eq!(lines.len(), 3);
            for line in lines {
                assert!(
                    ascii_theme.adapt(line).is_ascii(),
                    "failed ascii adapt: {line}"
                );
            }
            let ascii_preset = Theme {
                ascii: true,
                ..unicode_theme
            };
            for line in mascot_lines(ascii_preset) {
                assert!(line.is_ascii());
            }
        }
        for line in mascot_lines(Theme::default()) {
            assert!(line.is_ascii());
        }
    }

    #[test]
    fn home_studio_split_radar_mentions_preview_and_confirm_card_render_cleanly() {
        let claude = Theme::default().with_preset(ThemePreset::ClaudeWarmth);
        let sessions = [
            HomeSessionEntry {
                name: "claude-core".into(),
                windows: 3,
                attached: true,
                current: true,
                active_window: "editor".into(),
                badge: Some("✻"),
            },
            HomeSessionEntry {
                name: "codex-api".into(),
                windows: 2,
                attached: false,
                current: false,
                active_window: "tests".into(),
                badge: Some(">_"),
            },
        ];
        let preview_lines = vec![
            "$ cargo test --all".into(),
            "test result: ok. 94 passed; 0 failed".into(),
        ];

        // 100x16 Two-column Claude-Code split card + workspaces + mascot + Pro Tip
        let mut studio_term = Terminal::new(TestBackend::new(100, 16)).expect("studio term");
        studio_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/project/justmux"),
                        context: "popup",
                        session: Some("claude-core"),
                        status: None,
                    },
                    HomeTelemetry {
                        sessions: Some(2),
                        windows: Some(5),
                        attached: Some(1),
                        clients: None,
                        hint_badge: Some("✻ claude"),
                    },
                    HomeDashboardState {
                        mode: ComposerMode::CommandJump,
                        sessions: &sessions,
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: None,
                        tip_step: 2,
                        emblem_step: 0,
                    },
                    claude,
                );
            })
            .expect("draw studio");
        let studio_out = studio_term.backend().to_string();
        assert!(studio_out.contains("◫ JustMux"));
        assert!(studio_out.contains("▗▛▀▜▖"));
        assert!(studio_out.contains("Live Workspaces (1-5)"));
        assert!(studio_out.contains("claude-core"));
        assert!(studio_out.contains("Pro Tip"));
        assert!(studio_out.contains("Shift+Tab Mode"));
        assert!(studio_out.contains("/help"));

        // LiveRadar mode with right-hand terminal preview card
        studio_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails::default(),
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        mode: ComposerMode::LiveRadar,
                        sessions: &sessions,
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 1,
                        preview: Some(PanePreview {
                            title: "codex-api",
                            subtitle: "tests",
                            lines: &preview_lines,
                            ..PanePreview::default()
                        }),
                        tip_step: 0,
                        emblem_step: 0,
                    },
                    claude,
                );
            })
            .expect("draw radar");
        let radar_out = studio_term.backend().to_string();
        assert!(radar_out.contains("Live Radar (2)"));
        assert!(radar_out.contains("cargo test --all"));
        assert!(radar_out.contains("94 passed"));

        // @ mention upward drawer
        studio_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::new("@cod"),
                    &[],
                    0,
                    HomeDetails::default(),
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        mode: ComposerMode::CommandJump,
                        sessions: &sessions,
                        mentions: &[MentionItem {
                            token: "@codex-api",
                            kind_badge: "session",
                            detail: "2w · tests",
                            indices: &[1, 2, 3],
                        }],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: None,
                        tip_step: 0,
                        emblem_step: 0,
                    },
                    claude,
                );
            })
            .expect("draw mentions");
        let mention_out = studio_term.backend().to_string();
        assert!(mention_out.contains("@ Targets"));
        assert!(mention_out.contains("@codex-api"));
        assert!(mention_out.contains("session"));

        // ASCII + no_color studio verification
        let ascii_no_color = Theme {
            ascii: true,
            no_color: true,
            ..claude
        };
        studio_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/repo"),
                        context: "standalone",
                        session: None,
                        status: None,
                    },
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        mode: ComposerMode::QuickLaunch,
                        sessions: &[],
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: None,
                        tip_step: 1,
                        emblem_step: 0,
                    },
                    ascii_no_color,
                );
            })
            .expect("draw ascii studio");
        let ascii_studio = studio_term.backend().to_string();
        assert!(!ascii_studio.contains('╭'));
        assert!(
            ascii_studio
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "unexpected non-ASCII in ascii studio:\n{ascii_studio}"
        );
        assert!(
            studio_term
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );

        // render_rich_picker_with_preview & render_confirm_card
        let mut wide = Terminal::new(TestBackend::new(80, 14)).expect("wide");
        wide.draw(|frame| {
            render_rich_picker_with_preview(
                frame,
                frame.area(),
                "Panes",
                &TextInput::default(),
                &[RichPickerRow {
                    primary: "codex-api:0.0 zsh".into(),
                    primary_indices: vec![0, 1],
                    secondary: "%1 · ~/repo".into(),
                    badge: Some(("active".into(), claude.accent)),
                    is_action: false,
                }],
                0,
                "Empty",
                "Enter Select",
                Some(PanePreview {
                    title: "codex-api:0.0",
                    subtitle: "%1",
                    lines: &preview_lines,
                    ..PanePreview::default()
                }),
                claude,
            );
        })
        .expect("picker with preview");
        let picker_preview = wide.backend().to_string();
        assert!(picker_preview.contains("Filter:"));
        assert!(picker_preview.contains("94 passed"));

        let mut scroll = 0;
        wide.draw(|frame| {
            scroll = render_confirm_card(
                frame,
                frame.area(),
                "Confirm irreversible action",
                "codex-api (2 windows)",
                "Permanently kill session? All running tasks will terminate.",
                &TextInput::new("yes"),
                "Esc Back",
                claude,
                0,
            );
        })
        .expect("confirm card");
        let confirm_out = wide.backend().to_string();
        assert!(confirm_out.contains("Approval Required"));
        assert!(confirm_out.contains("[sandbox: native-tmux]"));
        assert!(confirm_out.contains("- kill codex-api (2 windows)"));
        assert_eq!(scroll, 0);

        // Narrow confirm_card falls back to render_form_scrolled
        let mut narrow = Terminal::new(TestBackend::new(24, 6)).expect("narrow");
        narrow
            .draw(|frame| {
                let s = render_confirm_card(
                    frame,
                    frame.area(),
                    "Confirm",
                    "work",
                    "Kill work session? Very long description to wrap and scroll cleanly across multiple rows.",
                    &TextInput::new("yes"),
                    "Esc Back",
                    ascii_no_color,
                    u16::MAX,
                );
                assert!(s > 0);
            })
            .expect("narrow confirm fallback");
        assert!(narrow.backend().to_string().is_ascii());
    }

    #[test]
    fn spacious_fullscreen_studio_and_config_modal_render_cleanly() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let sessions: Vec<HomeSessionEntry> = (1..=6)
            .map(|idx| HomeSessionEntry {
                name: format!("ws-{idx}"),
                windows: idx,
                attached: idx % 2 == 1,
                current: idx == 1,
                active_window: format!("win-{idx}"),
                badge: (idx == 1).then_some("✦"),
            })
            .collect();
        let preview_lines = vec![
            "$ cargo test --all".into(),
            "test result: ok. 107 passed".into(),
        ];

        let mut spacious = Terminal::new(TestBackend::new(120, 32)).expect("spacious");
        spacious
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/project/justmux"),
                        context: "standalone",
                        session: None,
                        status: None,
                    },
                    HomeTelemetry {
                        sessions: Some(6),
                        windows: Some(21),
                        attached: Some(3),
                        clients: None,
                        hint_badge: Some("✦ aurora"),
                    },
                    HomeDashboardState {
                        mode: ComposerMode::CommandJump,
                        sessions: &sessions,
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: Some(PanePreview {
                            title: "ws-1",
                            subtitle: "win-1",
                            lines: &preview_lines,
                            ..PanePreview::default()
                        }),
                        tip_step: 0,
                        emblem_step: 0,
                    },
                    theme,
                );
            })
            .expect("draw spacious");
        let studio_out = spacious.backend().to_string();
        assert!(studio_out.contains("⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀"));
        for cmd in [
            "/resume", "/new", "/list", "/window", "/pane", "/split", "/theme", "/help",
        ] {
            assert!(studio_out.contains(cmd), "missing {cmd} in:\n{studio_out}");
        }
        assert!(studio_out.contains("Create a tmux session"));
        assert!(studio_out.contains("Live Workspaces (1-6)"));
        assert!(studio_out.contains("[6]"));
        assert!(studio_out.contains("ws-6"));
        assert!(studio_out.contains("📺 Active Workspace Preview"));
        assert!(studio_out.contains("107 passed"));
        assert!(
            spacious
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.symbol() == "/" && cell.fg == theme.success)
        );

        // Spacious 120x32 with ascii = true, no_color = true
        let spacious_ascii_no_color = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        spacious
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/project/justmux"),
                        context: "standalone",
                        session: None,
                        status: None,
                    },
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        mode: ComposerMode::CommandJump,
                        sessions: &sessions,
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: Some(PanePreview {
                            title: "ws-1",
                            subtitle: "win-1",
                            lines: &preview_lines,
                            ..PanePreview::default()
                        }),
                        tip_step: 0,
                        emblem_step: 0,
                    },
                    spacious_ascii_no_color,
                );
            })
            .expect("draw spacious ascii");
        let spacious_ascii_out = spacious.backend().to_string();
        assert!(!spacious_ascii_out.contains('╭'));
        assert!(
            spacious_ascii_out
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "spacious ascii not ASCII:\n{spacious_ascii_out}"
        );
        assert!(
            spacious
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );

        // Mode B: 100x30 2-column Hydra rendering
        let mut two_col = Terminal::new(TestBackend::new(100, 30)).expect("100x30");
        two_col
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails {
                        directory: Some("~/project/justmux"),
                        context: "standalone",
                        session: None,
                        status: None,
                    },
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        mode: ComposerMode::CommandJump,
                        sessions: &sessions,
                        mentions: &[],
                        mention_selected: 0,
                        radar_selected: 0,
                        preview: None,
                        tip_step: 1,
                        emblem_step: 0,
                    },
                    theme,
                );
            })
            .expect("draw 100x30");
        let two_col_out = two_col.backend().to_string();
        assert!(two_col_out.contains("⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀"));
        assert!(two_col_out.contains("Live Workspaces"));
        assert!(two_col_out.contains("Pro Tip"));

        // Interactive /config modal in Unicode + TrueColor
        let items = [
            ConfigItemView {
                key: "screen_mode",
                value: "fullscreen".into(),
                description: "Default display mode (fullscreen or inline)",
            },
            ConfigItemView {
                key: "theme",
                value: "aurora".into(),
                description: "Active TrueColor or ANSI theme preset",
            },
            ConfigItemView {
                key: "mouse",
                value: "false".into(),
                description: "Enable mouse scroll/click in fullscreen mode",
            },
        ];
        let mut cfg_term = Terminal::new(TestBackend::new(88, 16)).expect("cfg term");
        cfg_term
            .draw(|frame| {
                render_config_modal(
                    frame,
                    frame.area(),
                    "/Users/demo/.config/justmux/config.toml",
                    &items,
                    1,
                    Some("Saved settings to ~/.config/justmux/config.toml"),
                    theme,
                );
            })
            .expect("draw config");
        let cfg_out = cfg_term.backend().to_string();
        assert!(cfg_out.contains("⚙ JustMux Configuration"));
        assert!(cfg_out.contains("config.toml"));
        assert!(cfg_out.contains("screen_mode"));
        assert!(cfg_out.contains("aurora"));
        assert!(cfg_out.contains("Saved settings"));
        assert!(cfg_out.contains("s Save to config.toml"));

        // ASCII + no_color config modal and help check
        let ascii_no_color = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        cfg_term
            .draw(|frame| {
                render_config_modal(
                    frame,
                    frame.area(),
                    "/Users/demo/.config/justmux/config.toml",
                    &items,
                    0,
                    None,
                    ascii_no_color,
                );
            })
            .expect("draw ascii config");
        let ascii_cfg_out = cfg_term.backend().to_string();
        assert!(!ascii_cfg_out.contains('╭'));
        assert!(
            ascii_cfg_out
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "config modal not ASCII:\n{ascii_cfg_out}"
        );
        assert!(
            cfg_term
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );

        let mut help_term = Terminal::new(TestBackend::new(80, 14)).expect("help term");
        help_term
            .draw(|frame| render_help(frame, frame.area(), theme))
            .expect("help");
        assert!(help_term.backend().to_string().contains("/config"));
    }

    #[test]
    fn hydra_emblem_scales_and_respects_ascii_no_color_and_truecolor() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let (w48, lines48) = hydra_emblem_lines(48, 24, aurora).expect("48x24");
        assert_eq!(w48, 48);
        assert_eq!(lines48.len(), 24);
        let expected_48: Vec<&str> = HYDRA_48X24.lines().take(24).collect();
        for (line, expected) in lines48.iter().zip(expected_48.iter()) {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(&text, expected);
            assert_eq!(line.spans.len(), 1);
            assert!(matches!(line.spans[0].style.fg, Some(Color::Rgb(_, _, _))));
        }
        assert!(
            lines48
                .iter()
                .any(|l| l.spans[0].content.contains("⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀"))
        );

        let (w32, lines32) = hydra_emblem_lines(32, 16, aurora).expect("32x16");
        assert_eq!(w32, 32);
        assert_eq!(lines32.len(), 16);
        assert!(lines32.iter().all(|l| l.width() <= 32));

        let (w24, lines24) = hydra_emblem_lines(24, 12, aurora).expect("24x12");
        assert_eq!(w24, 24);
        assert_eq!(lines24.len(), 12);
        assert!(lines24.iter().all(|l| l.width() <= 24));

        let (w16, lines16) = hydra_emblem_lines(16, 8, aurora).expect("16x8");
        assert_eq!(w16, 16);
        assert_eq!(lines16.len(), 8);
        assert!(lines16.iter().all(|l| l.width() <= 16));

        assert!(hydra_emblem_lines(15, 8, aurora).is_none());
        assert!(hydra_emblem_lines(16, 7, aurora).is_none());
        assert!(hydra_emblem_lines(48, 7, aurora).is_none());

        let ascii_theme = Theme {
            ascii: true,
            ..aurora
        };
        for (mw, mh) in [(48, 24), (32, 16), (24, 12), (16, 8)] {
            let (_, ascii_lines) = hydra_emblem_lines(mw, mh, ascii_theme).expect("ascii scale");
            assert!(
                ascii_lines.iter().all(|l| l.spans[0].content.is_ascii()
                    && l.spans[0].content.len() <= usize::from(mw))
            );
            assert!(
                ascii_lines
                    .iter()
                    .any(|l| l.spans[0].content.contains('#') || l.spans[0].content.contains('+'))
            );
        }

        let no_color = Theme {
            no_color: true,
            ..aurora
        };
        let (_, nc_lines) = hydra_emblem_lines(48, 24, no_color).expect("no_color");
        for line in nc_lines {
            assert_eq!(line.spans.len(), 1);
            assert_eq!(line.spans[0].style.fg, None);
            assert_eq!(line.spans[0].style.bg, None);
            assert!(line.spans[0].style.add_modifier.is_empty());
        }

        let classic = Theme::default();
        let (_, classic_lines) = hydra_emblem_lines(24, 12, classic).expect("classic");
        assert_eq!(classic_lines[0].spans[0].style.fg, Some(classic.accent));
        assert!(
            classic_lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(classic_lines[5].spans[0].style.fg, Some(classic.secondary));
        assert_eq!(classic_lines[11].spans[0].style.fg, Some(classic.tertiary));
    }

    #[test]
    fn hydra_emblem_rotation_and_home_renders_hydra_invariants() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let extract_text = |lines: &[Line<'static>]| -> Vec<String> {
            lines
                .iter()
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };

        let (w0, frame0) = hydra_emblem_frame(48, 24, 0, aurora).expect("frame 0");
        assert_eq!(w0, 48);
        let rows0 = extract_text(&frame0);
        assert_eq!(
            rows0,
            HYDRA_48X24
                .lines()
                .take(24)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );

        let (w_wrap, frame_wrap) =
            hydra_emblem_frame(48, 24, HYDRA_ROTATION_STEPS, aurora).expect("wrapped frame");
        assert_eq!(w_wrap, 48);
        assert_eq!(extract_text(&frame_wrap), rows0);

        for step in [0_usize, 9, 18, 27] {
            let (w_rot, rot) = hydra_emblem_frame(48, 24, step, aurora).expect("rotated frame");
            assert_eq!(w_rot, 48);
            assert_eq!(rot.len(), 24);
            assert!(rot.iter().all(|l| l.width() <= 48));
            let rows = extract_text(&rot);
            assert_eq!(rows[0], "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀");
            assert_eq!(rows[23], "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠙⠓⠶⠿⣭⣭⣭⣿⣿⣭⣭⣭⠿⠶⠚⠋⠉");
            if step > 0 {
                assert_ne!(
                    rows[6..18],
                    rows0[6..18],
                    "interior rows 6..18 at step {step} should differ from frame 0"
                );
            }
        }

        let (w16_0, lines16_0) = hydra_emblem_frame(16, 8, 0, aurora).expect("16x8 step 0");
        let (w16_9, lines16_9) = hydra_emblem_frame(16, 8, 9, aurora).expect("16x8 step 9");
        assert_eq!(w16_0, 16);
        assert_eq!(w16_9, 16);
        assert_eq!(lines16_9.len(), 8);
        assert!(lines16_9.iter().all(|l| l.width() <= 16));
        assert_ne!(extract_text(&lines16_9), extract_text(&lines16_0));

        let ascii_theme = Theme {
            ascii: true,
            ..aurora
        };
        for step in [0_usize, 9, 18] {
            let (_, rot_ascii) =
                hydra_emblem_frame(48, 24, step, ascii_theme).expect("ascii rotated");
            assert_eq!(rot_ascii.len(), 24);
            assert!(
                rot_ascii
                    .iter()
                    .all(|l| l.spans[0].content.is_ascii() && l.spans[0].content.len() <= 48)
            );
        }

        let no_color = Theme {
            no_color: true,
            ..aurora
        };
        for step in [0_usize, 9, 18] {
            let (_, rot_nc) = hydra_emblem_frame(32, 16, step, no_color).expect("no_color rotated");
            assert_eq!(rot_nc.len(), 16);
            assert!(rot_nc.iter().all(|l| l.spans.len() == 1
                && l.spans[0].style.fg.is_none()
                && l.spans[0].style.bg.is_none()
                && l.spans[0].style.add_modifier.is_empty()));
        }

        assert!(home_renders_hydra(
            Rect::new(0, 0, 120, 32),
            ComposerMode::CommandJump
        ));
        assert!(home_renders_hydra(
            Rect::new(0, 0, 100, 30),
            ComposerMode::CommandJump
        ));
        assert!(home_renders_hydra(
            Rect::new(0, 0, 120, 32),
            ComposerMode::QuickLaunch
        ));
        assert!(!home_renders_hydra(
            Rect::new(0, 0, 100, 16),
            ComposerMode::CommandJump
        ));
        assert!(!home_renders_hydra(
            Rect::new(0, 0, 72, 30),
            ComposerMode::CommandJump
        ));
        assert!(!home_renders_hydra(
            Rect::new(0, 0, 120, 32),
            ComposerMode::LiveRadar
        ));

        // End-to-end render_home_studio keeps stationary outer ring while rotating interior
        let mut static_term = Terminal::new(TestBackend::new(120, 32)).expect("static term");
        let mut rotated_term = Terminal::new(TestBackend::new(120, 32)).expect("rotated term");
        static_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails::default(),
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        emblem_step: 0,
                        ..HomeDashboardState::default()
                    },
                    aurora,
                );
            })
            .expect("draw static studio");
        rotated_term
            .draw(|frame| {
                render_home_studio(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    &[],
                    0,
                    HomeDetails::default(),
                    HomeTelemetry::default(),
                    HomeDashboardState {
                        emblem_step: 9,
                        ..HomeDashboardState::default()
                    },
                    aurora,
                );
            })
            .expect("draw rotated studio");
        let static_str = static_term.backend().to_string();
        let rotated_str = rotated_term.backend().to_string();
        assert!(rotated_str.contains("⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀"));
        assert!(rotated_str.contains("⠉⠙⠓⠶⠿⣭⣭⣭⣿⣿⣭⣭⣭⠿⠶⠚⠋⠉"));
        assert_ne!(rotated_str, static_str);
    }

    #[test]
    fn workspace_home_layout_modes_and_ascii_no_color_invariants() {
        let codex = Theme::default().with_preset(ThemePreset::CodexEmerald);

        assert_eq!(TargetScope::Window.toggle(), TargetScope::Session);
        assert_eq!(TargetScope::Session.toggle(), TargetScope::Window);
        assert_eq!(
            MatrixAction::Open.cycle_workspace_tab(false),
            MatrixAction::Rename(TargetScope::Window)
        );
        assert_eq!(
            MatrixAction::Rename(TargetScope::Window).cycle_workspace_tab(true),
            MatrixAction::Kill(TargetScope::Session)
        );
        assert_eq!(
            MatrixAction::Kill(TargetScope::Session).cycle_workspace_tab(false),
            MatrixAction::Split
        );
        assert_eq!(
            MatrixAction::Split.cycle_workspace_tab(true),
            MatrixAction::Open
        );
        assert_eq!(
            MatrixAction::Inspect.cycle_workspace_tab(true),
            MatrixAction::Rename(TargetScope::Session)
        );
        assert_eq!(MatrixAction::Open.border_color(codex), codex.accent);
        assert_eq!(
            MatrixAction::Rename(TargetScope::Window).border_color(codex),
            codex.warning
        );
        assert_eq!(
            MatrixAction::Kill(TargetScope::Session).border_color(codex),
            codex.error
        );
        assert_eq!(MatrixAction::Split.border_color(codex), codex.secondary);
        assert_eq!(MatrixAction::Inspect.border_color(codex), codex.secondary);
        assert_eq!(MatrixAction::Window.border_color(codex), codex.success);

        assert!(MatrixAction::Open.composer_title(false).contains("Search:"));
        assert!(MatrixAction::Open.composer_title(true).contains("Search:"));
        assert!(
            MatrixAction::Rename(TargetScope::Session)
                .composer_title(false)
                .contains("Rename session")
        );
        assert!(
            !MatrixAction::Rename(TargetScope::Session)
                .composer_title(false)
                .contains("Search:")
        );
        assert_eq!(MatrixAction::Open.mode_pill(), None);
        assert_eq!(
            MatrixAction::Rename(TargetScope::Session).mode_pill(),
            Some("RENAME SESSION")
        );
        assert_eq!(
            MatrixAction::Kill(TargetScope::Session).mode_pill(),
            Some("KILL SESSION")
        );
        assert_eq!(MatrixAction::Split.mode_pill(), Some("SPLIT"));
        assert_eq!(MatrixAction::Window.mode_pill(), Some("WINDOW"));
        assert_eq!(MatrixAction::Inspect.mode_pill(), Some("INSPECT"));

        let preview_lines = vec!["cargo test --lib ui".to_owned(), "24 passed".to_owned()];
        let details = HomeDetails {
            directory: Some("/Users/demo/justmux"),
            context: "default",
            session: Some("work"),
            status: None,
        };
        let telemetry = HomeTelemetry {
            sessions: Some(3),
            windows: Some(8),
            attached: Some(1),
            clients: None,
            hint_badge: None,
        };
        let preview = Some(PanePreview {
            title: "work",
            subtitle: "1:zsh",
            lines: &preview_lines,
            ..PanePreview::default()
        });

        // 1. 120x32 spacious fullscreen: upper ~60% Window Preview + lower Action Tabs, Matrix & 16x8 Hydra.
        let mut wide = Terminal::new(TestBackend::new(120, 32)).expect("wide term");
        let mut wide_layout = WorkspaceHomeLayout::default();
        wide.draw(|frame| {
            wide_layout = render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                WorkspaceHomeSpec {
                    details,
                    telemetry,
                    action: MatrixAction::Open,
                    composer_mode: ComposerMode::CommandJump,
                    preview,
                    live_preview: true,
                    emblem_step: 9,
                    ..WorkspaceHomeSpec::default()
                },
                codex,
                |f, body| {
                    f.render_widget(
                        Paragraph::new("MATRIX_FULL_ROW_SENTINEL | col2 | col3"),
                        body,
                    );
                },
            );
        })
        .expect("draw 120x32");
        assert!(wide_layout.hydra_rendered);
        assert!(wide_layout.preview_area.is_some());
        assert!(workspace_home_renders_hydra(
            Rect::new(0, 0, 120, 32),
            ComposerMode::CommandJump
        ));
        assert_eq!(wide_layout.matrix_body.width, 98);
        assert_eq!(wide_layout.matrix_body.height, 9);
        assert!(
            wide_layout
                .tab_hits
                .iter()
                .all(|(r, _)| r.width > 0 && r.height == 1)
        );
        assert_eq!(wide_layout.tab_hits[0].1, MatrixAction::Open);
        assert_eq!(wide_layout.tab_hits[3].1, MatrixAction::Split);
        let wide_str = wide.backend().to_string();
        assert!(wide_str.contains(">_ codex"));
        assert!(wide_str.contains("[ ▸ Resume ]"));
        assert!(wide_str.contains("[ ✎ Rename ]"));
        assert!(wide_str.contains("[ ✕ Kill ]"));
        assert!(wide_str.contains("[ ◫ Split ]"));
        assert!(wide_str.contains("Tab ⇄"));
        assert!(wide_str.contains("▸ Resume · Search: session or window"));
        assert!(wide_str.contains("Tab Switch · / Commands · Shift+Tab Mode"));
        assert!(wide_str.contains("MATRIX_FULL_ROW_SENTINEL"));
        assert!(wide_str.contains("Window Preview"));
        assert!(wide_str.contains("cargo test --lib ui"));
        assert!(wide_str.contains("Type / for commands"));
        let wide_lines: Vec<&str> = wide_str.lines().collect();
        let preview_y = wide_lines
            .iter()
            .position(|l| l.contains("Window Preview"))
            .expect("preview row");
        let matrix_start_y = usize::from(wide_layout.matrix_body.y);
        assert!(
            preview_y < matrix_start_y,
            "Window Preview (row {preview_y}) must appear above matrix_body (row {matrix_start_y})"
        );
        assert!(
            wide_layout.matrix_body.y.saturating_sub(2) > wide_layout.matrix_body.height,
            "upper Window Preview height ({}) should exceed lower matrix_body height ({})",
            wide_layout.matrix_body.y.saturating_sub(2),
            wide_layout.matrix_body.height
        );
        let matrix_right_col = usize::from(wide_layout.matrix_body.right());
        let hydra_rows_in_lower_right = wide_lines[matrix_start_y..]
            .iter()
            .filter(|line| {
                line.chars()
                    .skip(matrix_right_col)
                    .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
            })
            .count();
        assert!(
            hydra_rows_in_lower_right >= 8,
            "expected 16x8 Hydra rendered in lower-right corner (right of col {matrix_right_col}), found {hydra_rows_in_lower_right} rows"
        );
        assert!(
            !wide_lines[..matrix_start_y].iter().any(|line| line
                .chars()
                .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))),
            "Hydra must NOT render non-blank Braille dots above matrix_body.y"
        );

        // 2. 100x30 with live_preview = false: compact 4-row upper hints + large Matrix & 32x16 Hydra.
        let mut np = Terminal::new(TestBackend::new(100, 30)).expect("100x30 term");
        let mut np_layout = WorkspaceHomeLayout::default();
        np.draw(|frame| {
            np_layout = render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                WorkspaceHomeSpec {
                    details,
                    telemetry,
                    action: MatrixAction::Rename(TargetScope::Session),
                    composer_mode: ComposerMode::CommandJump,
                    preview,
                    live_preview: false,
                    emblem_step: 18,
                    ..WorkspaceHomeSpec::default()
                },
                codex,
                |f, body| {
                    f.render_widget(Paragraph::new("MATRIX_100X30"), body);
                },
            );
        })
        .expect("draw 100x30");
        assert!(np_layout.hydra_rendered);
        assert!(np_layout.preview_area.is_none());
        assert!(workspace_home_renders_hydra(
            Rect::new(0, 0, 100, 30),
            ComposerMode::CommandJump
        ));
        assert_eq!(np_layout.matrix_body.width, 62);
        let np_str = np.backend().to_string();
        assert!(!np_str.contains("Window Preview"));
        assert!(!np_str.contains("Active Workspace Preview"));
        assert!(np_str.contains("MATRIX_100X30"));
        assert!(np_str.contains("RENAME SESSION"));
        assert!(np_str.contains("✎ Rename session · ↓ window · Enter rename"));
        assert!(np_str.contains("Tab Next · Esc Resume"));
        assert!(np_str.contains("Switch window action: /resume (open)"));
        assert!(np_str.contains("/rename"));
        assert!(np_str.contains("/kill"));
        assert!(np_str.contains("/split"));
        assert!(np_str.contains("/window"));
        assert!(np_str.contains("/help"));
        let np_lines: Vec<&str> = np_str.lines().collect();
        let np_hydra_rows = np_lines[usize::from(np_layout.matrix_body.y)..]
            .iter()
            .filter(|line| {
                line.chars()
                    .skip(usize::from(np_layout.matrix_body.right()))
                    .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
            })
            .count();
        assert!(np_hydra_rows >= 16);

        // 3. 100x16 inline: no Hydra, Window Preview on top and matrix below when live_preview = true.
        let mut inline = Terminal::new(TestBackend::new(100, 16)).expect("inline term");
        let mut inline_layout = WorkspaceHomeLayout::default();
        inline
            .draw(|frame| {
                inline_layout = render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec {
                        details,
                        telemetry,
                        preview,
                        live_preview: true,
                        emblem_step: 9,
                        ..WorkspaceHomeSpec::default()
                    },
                    codex,
                    |f, body| {
                        f.render_widget(Paragraph::new("MATRIX_INLINE_100X16"), body);
                    },
                );
            })
            .expect("draw 100x16");
        assert!(!inline_layout.hydra_rendered);
        assert!(inline_layout.preview_area.is_some());
        assert!(!workspace_home_renders_hydra(
            Rect::new(0, 0, 100, 16),
            ComposerMode::CommandJump
        ));
        let inline_str = inline.backend().to_string();
        assert!(inline_str.contains("MATRIX_INLINE_100X16"));
        assert!(inline_str.contains("Window Preview"));
        assert!(inline_str.contains("Type / for commands"));
        let inline_preview_y = inline_str
            .lines()
            .position(|l| l.contains("Window Preview"))
            .expect("inline preview row");
        assert!(
            inline_preview_y < usize::from(inline_layout.matrix_body.y),
            "inline Window Preview (row {inline_preview_y}) must appear above matrix_body (row {})",
            inline_layout.matrix_body.y
        );

        // 4. Threshold viewports (80x22 hits 16x8 Hydra, LiveRadar & tiny skip Hydra).
        assert!(workspace_home_renders_hydra(
            Rect::new(0, 0, 80, 22),
            ComposerMode::CommandJump
        ));
        assert!(!workspace_home_renders_hydra(
            Rect::new(0, 0, 120, 32),
            ComposerMode::LiveRadar
        ));
        assert!(!workspace_home_renders_hydra(
            Rect::new(0, 0, 34, 10),
            ComposerMode::CommandJump
        ));

        let mut tiny = Terminal::new(TestBackend::new(34, 10)).expect("tiny term");
        let mut tiny_layout = WorkspaceHomeLayout::default();
        tiny.draw(|frame| {
            tiny_layout = render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                WorkspaceHomeSpec::default(),
                Theme::default(),
                |f, body| {
                    f.render_widget(Paragraph::new("TINY_MATRIX"), body);
                },
            );
        })
        .expect("draw tiny");
        assert!(!tiny_layout.hydra_rendered);
        let tiny_out = tiny.backend().to_string();
        assert!(tiny_out.contains("◫ JustMux"));
        assert!(tiny_out.contains("TINY_MATRIX"));
        assert!(tiny_out.contains("Type / for commands"));

        // Compact inline 70x10 (inline_bottom shrink size) keeps contiguous title + matrix + footer.
        let mut compact70 = Terminal::new(TestBackend::new(70, 10)).expect("70x10 term");
        compact70
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec::default(),
                    Theme::default(),
                    |f, body| {
                        f.render_widget(Paragraph::new("COMPACT_70X10"), body);
                    },
                );
            })
            .expect("draw 70x10");
        let compact70_out = compact70.backend().to_string();
        assert!(compact70_out.contains("◫ JustMux"));
        assert!(compact70_out.contains("COMPACT_70X10"));
        assert!(compact70_out.contains("Type / for commands"));

        // Inline(6) (height < 7) omits header row so matrix + popup + composer + footer keep space.
        let mut inline6 = Terminal::new(TestBackend::new(40, 6)).expect("40x6 term");
        inline6
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec::default(),
                    Theme::default(),
                    |f, body| {
                        assert_eq!(body.height, 3);
                        f.render_widget(Paragraph::new("INLINE_6_BODY"), body);
                    },
                );
            })
            .expect("draw 40x6");
        let inline6_out = inline6.backend().to_string();
        assert!(!inline6_out.contains("JustMux"));
        assert!(inline6_out.contains("INLINE_6_BODY"));

        // 5. ASCII + no_color invariant at 120x32 with Kill action & rotated Hydra.
        let ascii_no_color = Theme {
            ascii: true,
            no_color: true,
            ..codex
        };
        let mut ascii_term = Terminal::new(TestBackend::new(120, 32)).expect("ascii term");
        let mut ascii_layout = WorkspaceHomeLayout::default();
        ascii_term
            .draw(|frame| {
                ascii_layout = render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec {
                        details,
                        telemetry,
                        action: MatrixAction::Kill(TargetScope::Session),
                        composer_mode: ComposerMode::CommandJump,
                        preview,
                        live_preview: true,
                        emblem_step: 9,
                        ..WorkspaceHomeSpec::default()
                    },
                    ascii_no_color,
                    |f, body| {
                        f.render_widget(Paragraph::new("ASCII_MATRIX_ROW"), body);
                    },
                );
            })
            .expect("draw ascii workspace");
        assert!(ascii_layout.hydra_rendered);
        let ascii_out = ascii_term.backend().to_string();
        assert!(ascii_out.contains("[] JustMux"));
        assert!(ascii_out.contains(">_ codex"));
        assert!(ascii_out.contains("[> Resume] [Rename] [Kill] [Split]"));
        assert!(ascii_out.contains("KILL SESSION"));
        assert!(ascii_out.contains("! Kill session [Down: window]"));
        assert!(ascii_out.contains("Tab Next | Esc Resume"));
        assert!(ascii_out.contains("Type / for commands"));
        assert!(
            ascii_out
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "workspace home ASCII output contained non-ASCII:\n{ascii_out}"
        );
        assert!(
            ascii_term
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset),
            "no_color must emit only Color::Reset cells"
        );
    }

    #[test]
    fn pane_preview_card_scrollback_clamping_badges_and_ascii_invariants() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let lines: Vec<String> = (1..=30).map(|i| format!("L{i:02}")).collect();
        let mut term = Terminal::new(TestBackend::new(60, 10)).expect("preview term");

        // scroll = 0: bottom 8 lines (L23..L30) + muted line-count hint.
        term.draw(|frame| {
            render_pane_preview_card(
                frame,
                frame.area(),
                PanePreview {
                    title: "📺 Window Preview",
                    subtitle: "main · 1:zsh",
                    lines: &lines,
                    scroll: 0,
                    ..PanePreview::default()
                },
                theme,
            );
        })
        .expect("draw scroll 0");
        let out0 = term.backend().to_string();
        assert!(out0.contains("L23") && out0.contains("L30"));
        assert!(!out0.contains("L22"));
        assert!(out0.contains("↕ 30L · PgUp/Wheel"));

        // scroll = 8: middle slice L15..L22 + accent offset badge.
        term.draw(|frame| {
            render_pane_preview_card(
                frame,
                frame.area(),
                PanePreview {
                    title: "📺 Window Preview",
                    subtitle: "main · 1:zsh",
                    lines: &lines,
                    scroll: 8,
                    ..PanePreview::default()
                },
                theme,
            );
        })
        .expect("draw scroll 8");
        let out8 = term.backend().to_string();
        assert!(out8.contains("L15") && out8.contains("L22"));
        assert!(!out8.contains("L14") && !out8.contains("L23"));
        assert!(out8.contains("↑ +8/22 · PgUp/PgDn"));

        // scroll = 999: clamps to max_scroll = 22 (L01..L08 + ↑ +22/22).
        term.draw(|frame| {
            render_pane_preview_card(
                frame,
                frame.area(),
                PanePreview {
                    title: "📺 Window Preview",
                    subtitle: "main · 1:zsh",
                    lines: &lines,
                    scroll: 999,
                    ..PanePreview::default()
                },
                theme,
            );
        })
        .expect("draw scroll clamped");
        let out_max = term.backend().to_string();
        assert!(out_max.contains("L01") && out_max.contains("L08"));
        assert!(!out_max.contains("L09"));
        assert!(out_max.contains("↑ +22/22 · PgUp/PgDn"));

        // ascii = true, no_color = true: pure ASCII badges and Color::Reset everywhere.
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        for (scroll, expected_badge) in [(0, "[30L PgUp/Wheel]"), (8, "[+8/22 PgUp/PgDn]")] {
            term.draw(|frame| {
                render_pane_preview_card(
                    frame,
                    frame.area(),
                    PanePreview {
                        title: "📺 Window Preview",
                        subtitle: "main",
                        lines: &lines,
                        scroll,
                        ..PanePreview::default()
                    },
                    ascii_nc,
                );
            })
            .expect("draw ascii preview");
            let ascii_out = term.backend().to_string();
            assert!(ascii_out.contains(expected_badge), "{ascii_out}");
            assert!(
                ascii_out
                    .chars()
                    .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
                "non-ASCII in preview:\n{ascii_out}"
            );
            assert!(
                term.backend()
                    .buffer()
                    .content()
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
            );
        }

        // Narrow card (34x10) uses compact badge fallback cleanly.
        let mut narrow = Terminal::new(TestBackend::new(34, 10)).expect("narrow preview");
        narrow
            .draw(|frame| {
                render_pane_preview_card(
                    frame,
                    frame.area(),
                    PanePreview {
                        title: "📺 Window Preview",
                        subtitle: "work",
                        lines: &lines,
                        scroll: 8,
                        ..PanePreview::default()
                    },
                    theme,
                );
            })
            .expect("draw narrow scrolled");
        let narrow_out = narrow.backend().to_string();
        assert!(narrow_out.contains("↑ +8"));
        assert!(!narrow_out.contains("PgUp/PgDn"));

        // render_help at 80x13 documents PgUp / PgDn preview scrolling.
        let mut help = Terminal::new(TestBackend::new(80, 13)).expect("help term");
        help.draw(|frame| render_help(frame, frame.area(), theme))
            .expect("help draw");
        assert!(help.backend().to_string().contains("PgUp / PgDn"));
    }

    #[test]
    fn pane_preview_card_parses_ansi_256_truecolor_and_respects_no_color_and_width() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let lines = vec![
            "\x1b[1;32mANSI16\x1b[0m \x1b[38;5;208mC256\x1b[0m".to_owned(),
            "\x1b[38;2;120;180;255;48;2;10;20;30mRGB\x1b[0m \x1b[38:2::255:100:50mCOLON\x1b[0m"
                .to_owned(),
            "\x1b[58;2;255;0;0;32mULSKIP\x1b[0m \x1b[4:3mUON\x1b[4:0mOFF".to_owned(),
            "\x1b]0;win-title\x07\x1b[2J\tTAB".to_owned(),
        ];
        let mut term = Terminal::new(TestBackend::new(60, 8)).expect("ansi preview term");
        term.draw(|frame| {
            render_pane_preview_card(
                frame,
                frame.area(),
                PanePreview {
                    title: "📺 Window Preview",
                    subtitle: "main · 1:zsh",
                    lines: &lines,
                    scroll: 0,
                    ..PanePreview::default()
                },
                theme,
            );
        })
        .expect("draw ansi preview");
        let buf = term.backend().buffer();
        let text = term.backend().to_string();
        assert!(text.contains("ANSI16 C256"));
        assert!(text.contains("RGB COLON"));
        assert!(text.contains("ULSKIP UONOFF"));
        assert!(text.contains("  TAB"));
        assert!(!text.contains("win-title"));

        let cell_for = |row: u16, needle: char| {
            (1..59_u16)
                .map(|x| &buf[(x, row)])
                .find(|cell| cell.symbol() == needle.encode_utf8(&mut [0; 4]))
                .expect("expected char in row")
        };

        let a16 = cell_for(1, 'A');
        assert_eq!(a16.fg, Color::Green);
        assert!(a16.modifier.contains(Modifier::BOLD));

        let c256 = cell_for(1, 'C');
        assert_eq!(c256.fg, Color::Indexed(208));

        let rgb = cell_for(2, 'R');
        assert_eq!(rgb.fg, Color::Rgb(120, 180, 255));
        assert_eq!(rgb.bg, Color::Rgb(10, 20, 30));

        let colon = cell_for(2, 'O');
        assert_eq!(colon.fg, Color::Rgb(255, 100, 50));

        let ul = cell_for(3, 'U');
        assert_eq!(ul.fg, Color::Green);
        let uon = cell_for(3, 'N');
        assert!(uon.modifier.contains(Modifier::UNDERLINED));
        let uoff = cell_for(3, 'F');
        assert!(!uoff.modifier.contains(Modifier::UNDERLINED));

        // Wide CJK grapheme truncation preserves styling and never cuts a wide glyph in half.
        let cjk_lines = vec!["ab\x1b[32m你好世界\x1b[0mcd".to_owned()];
        let mut narrow = Terminal::new(TestBackend::new(10, 4)).expect("narrow cjk term");
        narrow
            .draw(|frame| {
                render_pane_preview_card(
                    frame,
                    frame.area(),
                    PanePreview {
                        title: "📺 Preview",
                        subtitle: "",
                        lines: &cjk_lines,
                        scroll: 0,
                        ..PanePreview::default()
                    },
                    theme,
                );
            })
            .expect("draw cjk preview");
        let narrow_text = narrow.backend().to_string();
        assert!(narrow_text.contains("ab你好世"));
        assert!(!narrow_text.contains('界'));
        assert!(!narrow_text.contains("cd"));
        let cjk_buf = narrow.backend().buffer();
        assert_eq!(cjk_buf[(3, 1)].fg, Color::Green);

        // no_color = true + ascii = true renders pure ASCII with Color::Reset everywhere.
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        let ascii_src = vec!["\x1b[38;2;120;180;255mhello · → world\x1b[0m".to_owned()];
        term.draw(|frame| {
            render_pane_preview_card(
                frame,
                frame.area(),
                PanePreview {
                    title: "📺 Window Preview",
                    subtitle: "main · 1:zsh",
                    lines: &ascii_src,
                    scroll: 0,
                    ..PanePreview::default()
                },
                ascii_nc,
            );
        })
        .expect("draw ascii no_color ansi preview");
        let out_ascii = term.backend().to_string();
        assert!(out_ascii.contains("hello | -> world"));
        assert!(
            out_ascii
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII output:\n{out_ascii}"
        );
        assert!(
            term.backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
    }

    #[test]
    fn command_popup_renders_right_aligned_group_badges_without_underlines() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("popup term");
        let popup = [
            CommandItem {
                name: "resume",
                description: "Open tmux window",
                alias: Some("r"),
                indices: &[0],
            },
            CommandItem {
                name: "config",
                description: "Settings panel",
                alias: None,
                indices: &[0],
            },
            CommandItem {
                name: "quit",
                description: "Quit JustMux",
                alias: Some("q"),
                indices: &[0],
            },
        ];
        terminal
            .draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::new("/"),
                    &popup,
                    1,
                    HomeDetails::default(),
                    theme,
                );
            })
            .expect("draw grouped popup");
        let text = terminal.backend().to_string();
        assert!(text.contains("workspace"));
        assert!(text.contains("studio"));
        assert!(text.contains("system"));

        let buf = terminal.backend().buffer();
        let area_rows = buf.area.height;
        // Group badges occupy the rightmost columns of popup rows and NEVER carry UNDERLINED.
        for y in 0..area_rows {
            let row_str: String = (0..80_u16).map(|x| buf[(x, y)].symbol()).collect();
            for badge in ["workspace", "studio", "system"] {
                if let Some(byte_col) = row_str.find(badge) {
                    let cell_col = row_str[..byte_col].chars().count();
                    assert!(
                        cell_col >= 60,
                        "expected right-aligned '{badge}' at col {cell_col} on row {y}: {row_str}"
                    );
                    for x in cell_col..(cell_col + badge.len()) {
                        assert!(
                            !buf[(x as u16, y)].modifier.contains(Modifier::UNDERLINED),
                            "group badge cell ({x}, {y}) must never be underlined"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn workspace_subview_chrome_renders_config_theme_help_confirm_and_tribute_ascii() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let details = HomeDetails {
            directory: Some("/Users/demo/justmux"),
            context: "popup",
            session: Some("work"),
            status: None,
        };
        let telemetry = HomeTelemetry {
            sessions: Some(3),
            windows: Some(8),
            attached: Some(1),
            clients: None,
            hint_badge: None,
        };

        // 1. Config inside 120x32 workspace chrome (all 9 items + status + Hydra).
        let cfg_items = [
            ConfigItemView {
                key: "screen_mode",
                value: "fullscreen".into(),
                description: "Default screen layout",
            },
            ConfigItemView {
                key: "theme",
                value: "aurora".into(),
                description: "Color theme preset",
            },
            ConfigItemView {
                key: "live_preview",
                value: "true".into(),
                description: "Show Window Preview",
            },
            ConfigItemView {
                key: "mouse",
                value: "true".into(),
                description: "Enable mouse navigation",
            },
            ConfigItemView {
                key: "emblem_spin",
                value: "true".into(),
                description: "Animate inner Hydra emblem",
            },
            ConfigItemView {
                key: "inline_height",
                value: "16".into(),
                description: "Inline viewport rows",
            },
            ConfigItemView {
                key: "ascii",
                value: "false".into(),
                description: "ASCII fallback glyphs",
            },
            ConfigItemView {
                key: "no_color",
                value: "false".into(),
                description: "Disable ANSI color codes",
            },
            ConfigItemView {
                key: "preview_lines",
                value: "400".into(),
                description: "Scrollback capture depth",
            },
        ];
        let mut wide = Terminal::new(TestBackend::new(120, 32)).expect("wide subview");
        let cfg_layout = wide
            .draw(|frame| {
                let l = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 9,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &cfg_items,
                            selected: 4,
                            status_note: Some("Saved config to config.toml"),
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
                assert!(l.hydra_rendered);
                assert!(l.list_rect.is_some());
            })
            .expect("draw config subview");
        let _ = cfg_layout;
        let cfg_out = wide.backend().to_string();
        assert!(cfg_out.contains("CONFIG"));
        assert!(cfg_out.contains("JustMux Configuration"));
        assert!(cfg_out.contains("screen_mode"));
        assert!(cfg_out.contains("no_color"));
        assert!(cfg_out.contains("emblem_spin"));
        assert!(cfg_out.contains("Saved config to config.toml"));
        assert!(cfg_out.contains("s Save to config.toml"));

        // 2. Theme subview in 120x32.
        wide.draw(|frame| {
            let l = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 0,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Theme {
                        presets: ThemePreset::ALL,
                        selected: 1,
                        active: ThemePreset::AntigravityAurora,
                    },
                    menubar: DesktopMenubarState::default(),
                },
                theme,
            );
            assert!(l.hydra_rendered);
            assert!(l.list_rect.is_some());
        })
        .expect("draw theme subview");
        let theme_out = wide.backend().to_string();
        assert!(theme_out.contains("THEME"));
        assert!(theme_out.contains("Theme palette"));
        assert!(theme_out.contains("Aurora"));
        assert!(theme_out.contains("Google Antigravity"));
        assert!(theme_out.contains("OpenAI Codex"));
        assert!(theme_out.contains("███"));

        // 3. Help subview in 120x32.
        wide.draw(|frame| {
            let l = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 9,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Help,
                    menubar: DesktopMenubarState::default(),
                },
                theme,
            );
            assert!(l.hydra_rendered);
        })
        .expect("draw help subview");
        let help_out = wide.backend().to_string();
        for needle in [
            "HELP",
            "/resume",
            "/rename",
            "/kill",
            "/split",
            "/pane",
            "/config",
            "/theme",
            "/codex · /claude · /antigravity · /vibe",
            "PgUp / PgDn",
            "Tab",
        ] {
            assert!(
                help_out.contains(needle),
                "missing '{needle}' in:\n{help_out}"
            );
        }

        // 4. Confirm + Picker + Split + Form + Created subviews in 120x32.
        let confirm_input = TextInput::new("yes");
        wide.draw(|frame| {
            let l = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 0,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Confirm {
                        title: "Confirm irreversible action",
                        target_summary: "work (2 windows)",
                        message: "Permanently terminate session?",
                        input: &confirm_input,
                        scroll: 0,
                    },
                    menubar: DesktopMenubarState::default(),
                },
                theme,
            );
            assert_eq!(l.confirm_scroll, 0);
        })
        .expect("draw confirm subview");
        let confirm_out = wide.backend().to_string();
        assert!(confirm_out.contains("CONFIRM KILL"));
        assert!(confirm_out.contains("Approval Required"));
        assert!(confirm_out.contains("[sandbox: native-tmux]"));
        assert!(confirm_out.contains("- kill work (2 windows)"));

        let preview_lines = vec!["pane line 1".to_owned(), "pane line 2".to_owned()];
        wide.draw(|frame| {
            let l = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 0,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Picker {
                        title: "Panes",
                        query: &TextInput::default(),
                        rows: &[RichPickerRow {
                            primary: "work:1.0 zsh".into(),
                            primary_indices: vec![],
                            secondary: "%1".into(),
                            badge: Some(("active".into(), theme.success)),
                            is_action: false,
                        }],
                        selected: 0,
                        empty: "Empty",
                        preview: Some(PanePreview {
                            title: "work",
                            subtitle: "1:zsh",
                            lines: &preview_lines,
                            scroll: 0,
                            ..PanePreview::default()
                        }),
                    },
                    menubar: DesktopMenubarState::default(),
                },
                theme,
            );
            assert!(l.preview_area.is_some());
            assert!(l.list_rect.is_some());
            assert!(l.hydra_rendered);
        })
        .expect("draw picker subview");
        let picker_out = wide.backend().to_string();
        assert!(picker_out.contains("PANES"));
        assert!(picker_out.contains("Filter:"));
        assert!(picker_out.contains("pane line 1"));

        // 5. Tribute subview at 90x20 with ASCII + no_color fits full ASCII art and zero colors!
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            ..theme
        };
        let tribute = TributeModal {
            badge: ">_ CODEX CLI",
            title: "OpenAI Codex CLI — Terminal Craft",
            subtitle: "Bottom-anchored composer and inline diffs",
            telemetry: "sandbox: native-tmux · approval: on-confirm",
            art: &[
                "+------------------------+",
                "| >_ codex . diff & run  |",
                "+-----------[on-confirm]-+",
            ],
            highlights: &[
                "Bottom-anchored slash composer",
                "Deliberate on-confirm guardrails",
            ],
            status_quip: ">_ Token budget: 0 tokens spent",
            footer_hint: "Enter Apply · Space Remix · Esc Back",
        };
        let mut trib90 = Terminal::new(TestBackend::new(90, 20)).expect("90x20 tribute");
        trib90
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: tribute.footer_hint,
                        content: SubviewContent::Tribute(tribute.clone()),
                        menubar: DesktopMenubarState::default(),
                    },
                    ascii_nc,
                );
            })
            .expect("draw 90x20 ascii tribute");
        let trib_out = trib90.backend().to_string();
        assert!(trib_out.contains(">_ CODEX CLI"));
        assert!(trib_out.contains("+------------------------+"));
        assert!(trib_out.contains("| >_ codex . diff & run  |"));
        assert!(trib_out.contains("+-----------[on-confirm]-+"));
        assert!(trib_out.contains("Bottom-anchored slash composer"));
        assert!(trib_out.contains("Token budget: 0 tokens spent"));
        assert!(
            trib_out
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in 90x20 ASCII tribute subview:\n{trib_out}"
        );
        assert!(
            trib90
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
    }

    #[test]
    fn config_subview_inspector_renders_tailored_art_and_guides_for_all_ten_keys() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let details = HomeDetails {
            directory: Some("project/justmux"),
            context: "git main",
            session: Some("work"),
            status: Some("Ready"),
        };
        let telemetry = HomeTelemetry::default();
        let items = vec![
            ConfigItemView {
                key: "screen_mode",
                value: "inline".into(),
                description: "Alternate fullscreen or native inline scrollback (restart)",
            },
            ConfigItemView {
                key: "theme",
                value: "emerald".into(),
                description: "Color palette preset (aurora, codex, claude, matrix, classic)",
            },
            ConfigItemView {
                key: "composer_mode",
                value: "radar".into(),
                description: "Default home studio mode (Shift+Tab)",
            },
            ConfigItemView {
                key: "live_preview",
                value: "true".into(),
                description: "Capture live tmux pane previews",
            },
            ConfigItemView {
                key: "mouse",
                value: "true".into(),
                description: "Mouse wheel & click support in Fullscreen (restart)",
            },
            ConfigItemView {
                key: "emblem_mode",
                value: "inverted-spin".into(),
                description: "Hydra center medallion animation mode",
            },
            ConfigItemView {
                key: "inline_height",
                value: "24".into(),
                description: "Reserved viewport rows for inline mode 8..=40 (restart)",
            },
            ConfigItemView {
                key: "ascii",
                value: "false".into(),
                description: "Pure ASCII box borders and markers",
            },
            ConfigItemView {
                key: "no_color",
                value: "false".into(),
                description: "Disable ANSI color attributes",
            },
            ConfigItemView {
                key: "language",
                value: "en".into(),
                description: "Interface language (en = English, zh = Chinese)",
            },
            ConfigItemView {
                key: "tmux_popup",
                value: "true".into(),
                description: "Bind Ctrl+b j in ~/.tmux.conf to pop up jm in shell or agy",
            },
        ];

        let expected_specs: [(&str, &str, &str, &[&str]); 11] = [
            (
                "screen_mode",
                "Viewport Presentation",
                "FULLSCREEN",
                &["INLINE"],
            ),
            (
                "theme",
                "TrueColor Palette & Mascot",
                "Spectrum",
                &["Codex Emerald", "OpenAI Codex"],
            ),
            (
                "composer_mode",
                "Startup Home Composer Lens",
                "Instant Attach",
                &["command", "radar", "launch"],
            ),
            (
                "live_preview",
                "Live Terminal Capture Preview",
                "cargo test",
                &[],
            ),
            (
                "mouse",
                "Mouse Capture & Wheel Scroll",
                "MOUSE ENABLED",
                &[],
            ),
            (
                "emblem_mode",
                "Hydra Center Medallion Mode",
                "*invert*",
                &["static", "spin"],
            ),
            (
                "inline_height",
                "Inline Drawer Viewport Rows",
                "Reserved rows: 24 / 40",
                &[],
            ),
            ("ascii", "ASCII Compatibility Mode", "Braille art", &[]),
            ("no_color", "Monochrome NO_COLOR Output", "Color ON", &[]),
            (
                "language",
                "Display Language (en / zh)",
                "English (en)",
                &["中文 (zh)"],
            ),
            (
                "tmux_popup",
                "tmux Global Popup Shortcut",
                "JustMux Popup",
                &["Ctrl+b", "~/.tmux.conf"],
            ),
        ];

        let find_cell_col = |buf: &ratatui::buffer::Buffer, needle: &str| -> Option<u16> {
            (0..buf.area.height).find_map(|y| {
                let (row, byte_to_col) = buffer_row_logical(buf, y);
                row.find(needle)
                    .and_then(|byte_idx| byte_to_col.get(byte_idx).copied())
            })
        };

        let mut wide = Terminal::new(TestBackend::new(118, 34)).expect("wide test terminal");
        for (idx, (key, doc_title, art_marker, extra_markers)) in expected_specs.iter().enumerate()
        {
            let mut layout = WorkspaceSubviewLayout::default();
            wide.draw(|frame| {
                layout = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 9,
                        emblem_mode: EmblemMode::InvertedSpin,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &items,
                            selected: idx,
                            status_note: Some("Takes effect on next jm launch (press s to save)"),
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
            })
            .expect("draw config inspector");
            assert!(layout.hydra_rendered, "hydra missing on key {key}");
            let buf = wide.backend().buffer();
            let doc_col = find_cell_col(buf, doc_title)
                .unwrap_or_else(|| panic!("missing doc title {doc_title:?} for key {key}"));
            let art_col = find_cell_col(buf, art_marker)
                .unwrap_or_else(|| panic!("missing art marker {art_marker:?} for key {key}"));
            assert!(
                doc_col < 59,
                "expected text guide {doc_title:?} on left half (<59) for {key}, got col {doc_col}"
            );
            assert!(
                art_col >= 59,
                "expected graphic art {art_marker:?} on right half (>=59) for {key}, got col {art_col}"
            );

            let out = wide.backend().to_string();
            assert!(
                out.contains("╭") && out.contains("╯"),
                "missing rounded box on {key}"
            );
            assert!(
                out.contains(&format!("JustMux Configuration · {key}")),
                "missing title on {key}:\n{out}"
            );
            assert!(
                out.contains("Config:") && out.contains("config.toml"),
                "missing path on {key}"
            );
            assert!(
                out.contains("Takes effect on next jm launch"),
                "missing status note on {key}"
            );
            for needle in *extra_markers {
                assert!(
                    out.contains(needle),
                    "missing {needle:?} for key {key}:\n{out}"
                );
            }
        }

        // Chinese inspector layout: left half explains in Chinese, right half shows bilingual diagram.
        let mut items_zh = items.clone();
        items_zh[9].value = "zh".into();
        let theme_zh = Theme {
            language: Language::Zh,
            ..theme
        };
        wide.draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 9,
                    emblem_mode: EmblemMode::InvertedSpin,
                    footer_hint: "",
                    content: SubviewContent::Config {
                        path_label: "/Users/demo/.config/justmux/config.toml",
                        items: &items_zh,
                        selected: 9,
                        status_note: None,
                    },
                    menubar: DesktopMenubarState::default(),
                },
                theme_zh,
            );
        })
        .expect("draw zh config inspector");
        let zh_buf = wide.backend().buffer();
        let zh_doc_col = find_cell_col(zh_buf, "界面显示语言 (en / zh)")
            .expect("missing Chinese doc title for language");
        let zh_art_col =
            find_cell_col(zh_buf, "中文 (zh)").expect("missing Chinese art marker for language");
        assert!(
            zh_doc_col < 59,
            "expected Chinese guide on left half (<59), got col {zh_doc_col}"
        );
        assert!(
            zh_art_col >= 59,
            "expected bilingual diagram on right half (>=59), got col {zh_art_col}"
        );
        let zh_out = wide.backend().to_string();
        assert!(
            zh_out.contains("JustMux 设置与参数 · language"),
            "missing Chinese config header:\n{zh_out}"
        );
        assert!(
            zh_out.contains("配置文件:") && zh_out.contains("config.toml"),
            "missing Chinese config path label:\n{zh_out}"
        );

        // Narrow terminal (< 72 inner cols) falls back to full-width doc + compact strip.
        let mut narrow = Terminal::new(TestBackend::new(66, 30)).expect("narrow test terminal");
        narrow
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 6,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &items,
                            selected: 5,
                            status_note: None,
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
            })
            .expect("draw narrow config inspector");
        let narrow_out = narrow.backend().to_string();
        assert!(narrow_out.contains("Hydra Center Medallion Mode"));
        assert!(narrow_out.contains("[*inverted-spin*]"));
        assert!(narrow_out.contains("(3/3)"));

        // Verify all 3 Braille modes side-by-side captions, guides, absence of Quadrant/Sextant mosaic blocks, and ASCII safety.
        for (mode, short_tag, idx_label) in [
            (EmblemMode::Static, "*static*", "(1/3)"),
            (EmblemMode::Spin, "*spin*", "(2/3)"),
            (EmblemMode::InvertedSpin, "*invert*", "(3/3)"),
        ] {
            let mut mode_items = items.clone();
            mode_items[5].value = mode.as_str().to_owned();
            wide.draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 9,
                        emblem_mode: mode,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &mode_items,
                            selected: 5,
                            status_note: None,
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
            })
            .expect("draw emblem mode wide");
            let mode_out = wide.backend().to_string();
            assert!(mode_out.contains(short_tag), "{mode_out}");
            assert!(
                mode_out.contains("static (0°) · spin (5°) · inverted-spin (hollow)"),
                "{mode_out}"
            );
            assert!(
                mode_out.contains(&format!("cycle 3 styles {idx_label}")),
                "{mode_out}"
            );
            assert!(
                !mode_out.chars().any(|c| {
                    ('\u{1FB00}'..='\u{1FB3B}').contains(&c)
                        || matches!(c, '▘' | '▝' | '▖' | '▗' | '▚' | '▞' | '▛' | '▜' | '▙' | '▟')
                }),
                "unexpected legacy mosaic glyph in {mode:?}:\n{mode_out}"
            );

            narrow
                .draw(|frame| {
                    render_workspace_subview(
                        frame,
                        frame.area(),
                        &WorkspaceSubviewSpec {
                            details,
                            telemetry,
                            emblem_step: 6,
                            emblem_mode: mode,
                            footer_hint: "",
                            content: SubviewContent::Config {
                                path_label: "/Users/demo/.config/justmux/config.toml",
                                items: &mode_items,
                                selected: 5,
                                status_note: None,
                            },
                            menubar: DesktopMenubarState::default(),
                        },
                        theme,
                    );
                })
                .expect("draw emblem mode narrow");
            let narrow_mode_out = narrow.backend().to_string();
            assert!(narrow_mode_out.contains(&format!("[*{}*]", mode.as_str())));
            assert!(narrow_mode_out.contains(idx_label), "{narrow_mode_out}");

            let ascii_nc = Theme {
                ascii: true,
                no_color: true,
                truecolor: false,
                ..Theme::default()
            };
            let mut asc = Terminal::new(TestBackend::new(118, 34)).expect("ascii emblem term");
            asc.draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 9,
                        emblem_mode: mode,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &mode_items,
                            selected: 5,
                            status_note: None,
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    ascii_nc,
                );
            })
            .expect("draw emblem ascii");
            let asc_out = asc.backend().to_string();
            assert!(asc_out.contains(short_tag), "{asc_out}");
            assert!(
                asc_out.contains("static 0deg | spin 5deg | inverted-spin (hollow)"),
                "{asc_out}"
            );
            assert!(
                asc_out.contains(&format!("cycle 3 styles {idx_label}")),
                "{asc_out}"
            );
            assert!(
                asc_out
                    .chars()
                    .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
                "non-ASCII output on {mode:?}:\n{asc_out}"
            );
            assert!(
                asc.backend()
                    .buffer()
                    .content()
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
            );
        }

        // Short terminal height (upper_band < 6) degrades gracefully without panic.
        let mut short = Terminal::new(TestBackend::new(100, 22)).expect("short test terminal");
        short
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Static,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &items,
                            selected: 0,
                            status_note: None,
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
            })
            .expect("draw short config subview");
        assert!(
            short
                .backend()
                .to_string()
                .contains("JustMux Configuration")
        );

        // Pure ASCII + NO_COLOR invariant across all 10 keys.
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..Theme::default()
        };
        let mut asc = Terminal::new(TestBackend::new(118, 34)).expect("ascii terminal");
        for idx in 0..items.len() {
            asc.draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 9,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/Users/demo/.config/justmux/config.toml",
                            items: &items,
                            selected: idx,
                            status_note: Some("Saved config to config.toml"),
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    ascii_nc,
                );
            })
            .expect("draw ascii config inspector");
            let ascii_out = asc.backend().to_string();
            assert!(
                ascii_out
                    .chars()
                    .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
                "non-ASCII output on config key {}:\n{ascii_out}",
                items[idx].key
            );
            assert!(
                asc.backend()
                    .buffer()
                    .content()
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset),
                "unexpected color attribute on config key {}",
                items[idx].key
            );
        }
    }

    #[test]
    fn workspace_specs_wire_emblem_mode_for_home_and_subviews() {
        let theme = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let details = HomeDetails::default();
        let telemetry = HomeTelemetry::default();
        let input = TextInput::default();

        let render_home_mode = |mode: EmblemMode, step: usize| -> (bool, String) {
            let mut term = Terminal::new(TestBackend::new(120, 34)).expect("home term");
            let mut layout = WorkspaceHomeLayout::default();
            term.draw(|frame| {
                layout = render_workspace_home(
                    frame,
                    frame.area(),
                    &input,
                    WorkspaceHomeSpec {
                        details,
                        telemetry,
                        emblem_step: step,
                        emblem_mode: mode,
                        ..WorkspaceHomeSpec::default()
                    },
                    theme,
                    |_, _| {},
                );
            })
            .expect("draw home");
            (layout.hydra_rendered, term.backend().to_string())
        };

        let (r_s0, out_s0) = render_home_mode(EmblemMode::Static, 0);
        let (r_s9, out_s9) = render_home_mode(EmblemMode::Static, 9);
        let (r_p0, out_p0) = render_home_mode(EmblemMode::Spin, 0);
        let (r_p9, out_p9) = render_home_mode(EmblemMode::Spin, 9);
        let (r_i0, out_i0) = render_home_mode(EmblemMode::InvertedSpin, 0);
        let (r_i9, out_i9) = render_home_mode(EmblemMode::InvertedSpin, 9);

        assert!(r_s0 && r_s9 && r_p0 && r_p9 && r_i0 && r_i9);
        assert_eq!(out_s0, out_s9, "Home Static mode must not animate");
        assert_eq!(out_s0, out_p0, "Home Static matches Spin at step 0");
        assert_ne!(out_p0, out_p9, "Home Spin must animate across steps");
        assert_ne!(out_p0, out_i0, "Home InvertedSpin differs from Spin");
        assert_ne!(
            out_i0, out_i9,
            "Home InvertedSpin must animate across steps"
        );

        let render_help_mode = |mode: EmblemMode, step: usize| -> (bool, String) {
            let mut term = Terminal::new(TestBackend::new(120, 34)).expect("subview term");
            let mut layout = WorkspaceSubviewLayout::default();
            term.draw(|frame| {
                layout = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: step,
                        emblem_mode: mode,
                        footer_hint: "",
                        content: SubviewContent::Help,
                        menubar: DesktopMenubarState::default(),
                    },
                    theme,
                );
            })
            .expect("draw help");
            (layout.hydra_rendered, term.backend().to_string())
        };

        let (hs0, hout_s0) = render_help_mode(EmblemMode::Static, 0);
        let (hs9, hout_s9) = render_help_mode(EmblemMode::Static, 9);
        let (hp0, hout_p0) = render_help_mode(EmblemMode::Spin, 0);
        let (hp9, hout_p9) = render_help_mode(EmblemMode::Spin, 9);
        let (hi0, hout_i0) = render_help_mode(EmblemMode::InvertedSpin, 0);
        let (hi9, hout_i9) = render_help_mode(EmblemMode::InvertedSpin, 9);
        assert!(hs0 && hs9 && hp0 && hp9 && hi0 && hi9);
        assert_eq!(hout_s0, hout_s9);
        assert_eq!(hout_s0, hout_p0);
        assert_ne!(hout_p0, hout_p9);
        assert_ne!(hout_s0, hout_i0);
        assert_ne!(hout_i0, hout_i9);
    }

    pub(super) fn buffer_row_logical(buf: &ratatui::buffer::Buffer, y: u16) -> (String, Vec<u16>) {
        let mut row = String::new();
        let mut byte_to_col = Vec::new();
        let mut x = 0_u16;
        while x < buf.area.width {
            let sym = buf[(x, y)].symbol();
            for _ in 0..sym.len() {
                byte_to_col.push(x);
            }
            row.push_str(sym);
            let step = sym.width().max(1);
            x = x.saturating_add(u16::try_from(step).unwrap_or(1));
        }
        (row, byte_to_col)
    }

    pub(super) fn buffer_logical_text(buf: &ratatui::buffer::Buffer) -> String {
        (0..buf.area.height)
            .map(|y| buffer_row_logical(buf, y).0)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn keycap_hints_highlight_real_keys_distinct_from_body_across_home_config_and_help() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let key_fg = aurora.keycap_fg();
        let key_bg = aurora.keycap_bg().expect("aurora keycap bg");
        assert_eq!(key_bg, Color::Rgb(186, 230, 253));
        assert_eq!(key_fg, Color::Rgb(15, 23, 42));
        let details = HomeDetails {
            directory: Some("project/justmux"),
            context: "git main",
            session: Some("dev"),
            status: None,
        };
        let telemetry = HomeTelemetry::default();
        let input = TextInput::default();
        let preview_lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();

        let find_cells =
            |buf: &ratatui::buffer::Buffer, needle: &str| -> Vec<ratatui::buffer::Cell> {
                for y in 0..buf.area.height {
                    let (row, byte_to_col) = buffer_row_logical(buf, y);
                    if let Some(byte_idx) = row.find(needle) {
                        let start_col = byte_to_col[byte_idx];
                        let end_col = byte_to_col[byte_idx + needle.len() - 1];
                        return (start_col..=end_col).map(|x| buf[(x, y)].clone()).collect();
                    }
                }
                panic!(
                    "missing {needle:?} in buffer:\n{}",
                    buffer_logical_text(buf)
                );
            };

        let assert_keycap = |buf: &ratatui::buffer::Buffer, needle: &str| {
            let cells = find_cells(buf, needle);
            for c in cells {
                assert_eq!(c.fg, key_fg, "key {needle:?} cell {:?} fg", c.symbol());
                assert_eq!(c.bg, key_bg, "key {needle:?} cell {:?} bg", c.symbol());
                assert!(
                    c.modifier.contains(Modifier::BOLD),
                    "key {needle:?} cell {:?} not bold",
                    c.symbol()
                );
            }
        };

        let assert_plain_fg = |buf: &ratatui::buffer::Buffer, needle: &str, expected_fg: Color| {
            let cells = find_cells(buf, needle);
            for c in cells {
                assert_eq!(
                    c.fg,
                    expected_fg,
                    "plain {needle:?} cell {:?} fg",
                    c.symbol()
                );
                assert_eq!(
                    c.bg,
                    Color::Reset,
                    "plain {needle:?} cell {:?} bg",
                    c.symbol()
                );
            }
        };

        let mut home = Terminal::new(TestBackend::new(120, 32)).expect("home term");
        home.draw(|frame| {
            render_workspace_home(
                frame,
                frame.area(),
                &input,
                WorkspaceHomeSpec {
                    details,
                    telemetry,
                    live_preview: true,
                    preview: Some(PanePreview {
                        title: "dev",
                        subtitle: "editor",
                        lines: &preview_lines,
                        ..PanePreview::default()
                    }),
                    ..WorkspaceHomeSpec::default()
                },
                aurora,
                |_, _| {},
            );
        })
        .expect("draw home");
        let hbuf = home.backend().buffer();
        for key in ["Tab", "1-9", "Shift+Tab", "PgUp", "Enter"] {
            assert_keycap(hbuf, key);
        }
        let q_help = find_cells(hbuf, "? help");
        assert_eq!(q_help[0].fg, key_fg);
        assert_eq!(q_help[0].bg, key_bg);
        assert_eq!(q_help[2].fg, aurora.muted);
        assert_eq!(q_help[2].bg, Color::Reset);
        assert_plain_fg(hbuf, "Switch", aurora.muted);
        assert_plain_fg(hbuf, "attach", aurora.muted);
        assert_plain_fg(hbuf, "Session", aurora.muted);

        let mut help = Terminal::new(TestBackend::new(120, 32)).expect("help term");
        help.draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 0,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Help,
                    menubar: DesktopMenubarState::default(),
                },
                aurora,
            );
        })
        .expect("draw help");
        let help_buf = help.backend().buffer();
        for key in ["Tab", "PgUp", "PgDn", "1-9", "Shift+Tab", "Ctrl+T", "Esc"] {
            assert_keycap(help_buf, key);
        }
        assert_plain_fg(help_buf, "/resume", aurora.success);

        let cfg_items = vec![ConfigItemView {
            key: "screen_mode",
            value: "fullscreen".into(),
            description: "Alternate fullscreen or native inline scrollback (restart)",
        }];
        let mut cfg = Terminal::new(TestBackend::new(120, 32)).expect("cfg term");
        cfg.draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_step: 0,
                    emblem_mode: EmblemMode::Spin,
                    footer_hint: "",
                    content: SubviewContent::Config {
                        path_label: "/tmp/config.toml",
                        items: &cfg_items,
                        selected: 0,
                        status_note: None,
                    },
                    menubar: DesktopMenubarState::default(),
                },
                aurora,
            );
        })
        .expect("draw config");
        let cfg_buf = cfg.backend().buffer();
        for key in ["←→", "Space", "Esc"] {
            assert_keycap(cfg_buf, key);
        }
        let s_save = find_cells(cfg_buf, "s Save");
        assert_eq!(s_save[0].fg, key_fg);
        assert_eq!(s_save[0].bg, key_bg);
        assert_eq!(s_save[2].fg, aurora.muted);
        assert_eq!(s_save[2].bg, Color::Reset);

        // Verify Chinese UI copy still highlights real keycaps with theme-matched light fills.
        let aurora_zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let mut home_zh = Terminal::new(TestBackend::new(120, 32)).expect("home zh term");
        home_zh
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &input,
                    WorkspaceHomeSpec {
                        details,
                        telemetry,
                        live_preview: true,
                        preview: Some(PanePreview {
                            title: "dev",
                            subtitle: "editor",
                            lines: &preview_lines,
                            ..PanePreview::default()
                        }),
                        ..WorkspaceHomeSpec::default()
                    },
                    aurora_zh,
                    |_, _| {},
                );
            })
            .expect("draw zh home");
        let hz_buf = home_zh.backend().buffer();
        for key in ["Tab", "1-9", "Shift+Tab", "PgUp", "Enter"] {
            assert_keycap(hz_buf, key);
        }
        let q_help_zh = find_cells(hz_buf, "? 帮助");
        assert_eq!(q_help_zh[0].fg, key_fg);
        assert_eq!(q_help_zh[0].bg, key_bg);
        assert_eq!(q_help_zh[2].fg, aurora_zh.muted);
        assert_eq!(q_help_zh[2].bg, Color::Reset);
        let hz_text = buffer_logical_text(hz_buf);
        assert!(hz_text.contains("恢复连接") && hz_text.contains("直达"));

        let mut help_zh = Terminal::new(TestBackend::new(120, 32)).expect("help zh term");
        help_zh
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Help,
                        menubar: DesktopMenubarState::default(),
                    },
                    aurora_zh,
                );
            })
            .expect("draw zh help");
        let help_zh_buf = help_zh.backend().buffer();
        for key in ["Tab", "PgUp", "PgDn", "1-9", "Shift+Tab", "Ctrl+T", "Esc"] {
            assert_keycap(help_zh_buf, key);
        }
        assert_plain_fg(help_zh_buf, "/resume", aurora_zh.success);
        let help_zh_text = buffer_logical_text(help_zh_buf);
        assert!(help_zh_text.contains("打开工作区矩阵连接会话"));
        assert!(help_zh_text.contains("命令与快捷键速查指南"));

        let mut standalone_help_zh =
            Terminal::new(TestBackend::new(80, 20)).expect("standalone help zh term");
        standalone_help_zh
            .draw(|frame| render_help(frame, frame.area(), aurora_zh))
            .expect("draw standalone help zh");
        let shz_buf = standalone_help_zh.backend().buffer();
        for key in ["PgUp", "PgDn", "Esc", "Tab", "Enter", "Ctrl+T"] {
            assert_keycap(shz_buf, key);
        }
        assert!(buffer_logical_text(shz_buf).contains("会话操作"));

        let cfg_zh_items = vec![ConfigItemView {
            key: "language",
            value: "zh".into(),
            description: "界面显示语言 (en = 英文, zh = 中文)",
        }];
        let mut cfg_zh = Terminal::new(TestBackend::new(120, 32)).expect("cfg zh term");
        cfg_zh
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Config {
                            path_label: "/tmp/config.toml",
                            items: &cfg_zh_items,
                            selected: 0,
                            status_note: None,
                        },
                        menubar: DesktopMenubarState::default(),
                    },
                    aurora_zh,
                );
            })
            .expect("draw zh config");
        let cfg_zh_buf = cfg_zh.backend().buffer();
        for key in ["←→", "Space", "Esc"] {
            assert_keycap(cfg_zh_buf, key);
        }
        let s_save_zh = find_cells(cfg_zh_buf, "s 保存");
        assert_eq!(s_save_zh[0].fg, key_fg);
        assert_eq!(s_save_zh[0].bg, key_bg);
        assert_eq!(s_save_zh[2].fg, aurora_zh.muted);
        assert_eq!(s_save_zh[2].bg, Color::Reset);

        let no_color_ascii = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let mut nc_home = Terminal::new(TestBackend::new(120, 32)).expect("nc home term");
        nc_home
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &input,
                    WorkspaceHomeSpec {
                        details,
                        telemetry,
                        ..WorkspaceHomeSpec::default()
                    },
                    no_color_ascii,
                    |_, _| {},
                );
            })
            .expect("draw nc home");
        let nc_text = nc_home.backend().to_string();
        assert!(
            nc_text
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in no_color_ascii home:\n{nc_text}"
        );
        let nc_buf = nc_home.backend().buffer();
        assert!(
            nc_buf
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
        for key in ["Tab", "1-9", "Shift+Tab"] {
            assert!(
                find_cells(nc_buf, key)
                    .iter()
                    .all(|c| c.modifier.contains(Modifier::BOLD))
            );
        }

        let mut nc_help = Terminal::new(TestBackend::new(120, 32)).expect("nc help term");
        nc_help
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Help,
                        menubar: DesktopMenubarState::default(),
                    },
                    no_color_ascii,
                );
            })
            .expect("draw nc help");
        let nch_text = nc_help.backend().to_string();
        assert!(
            nch_text
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in no_color_ascii help:\n{nch_text}"
        );
        let nch_buf = nc_help.backend().buffer();
        assert!(
            nch_buf
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
        assert!(
            find_cells(nch_buf, "Esc")
                .iter()
                .all(|c| c.modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn command_group_badges_and_help_views_follow_contiguous_grouped_order() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };

        for name in [
            "resume", "new", "window", "rename", "kill", "snapshot", "list",
        ] {
            assert_eq!(command_group_badge(name, aurora).0, "workspace");
            assert_eq!(command_group_badge(name, zh).0, "工作区");
        }
        for name in [
            "pane", "split", "layout", "expose", "buffers", "popup", "theme", "config",
        ] {
            assert_eq!(command_group_badge(name, aurora).0, "studio");
            assert_eq!(command_group_badge(name, zh).0, "工作台");
        }
        assert_eq!(command_group_badge("kill-server", aurora).0, "danger");
        assert_eq!(command_group_badge("kill-server", zh).0, "危险");
        for name in ["clients", "help", "exit", "quit"] {
            assert_eq!(command_group_badge(name, aurora).0, "system");
            assert_eq!(command_group_badge(name, zh).0, "系统");
        }

        // Verify contiguous grouped left/right columns in render_subview_help.
        let mut sub_term = Terminal::new(TestBackend::new(120, 32)).expect("subview help term");
        sub_term
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Help,
                        ..WorkspaceSubviewSpec::default()
                    },
                    aurora,
                );
            })
            .expect("draw subview help");
        let sub_text = buffer_logical_text(sub_term.backend().buffer());
        let expected_left = [
            "/resume",
            "/new",
            "/window",
            "/rename",
            "/kill",
            "/snapshot",
            "/list",
            "/exit",
            "/kill-server",
            "/quit",
        ];
        let expected_right = [
            "/pane", "/split", "/layout", "/expose", "/buffers", "/popup", "/clients", "/theme",
            "/config", "/help",
        ];
        for pair in expected_left.windows(2) {
            let a = sub_text.find(pair[0]).expect(pair[0]);
            let b = sub_text.find(pair[1]).expect(pair[1]);
            assert!(
                a < b,
                "expected {} before {} in:\n{sub_text}",
                pair[0],
                pair[1]
            );
        }
        for pair in expected_right.windows(2) {
            let a = sub_text.find(pair[0]).expect(pair[0]);
            let b = sub_text.find(pair[1]).expect(pair[1]);
            assert!(
                a < b,
                "expected {} before {} in:\n{sub_text}",
                pair[0],
                pair[1]
            );
        }

        // Verify compact render_help follows exact palette grouping + ASCII/NO_COLOR invariants.
        let mut compact = Terminal::new(TestBackend::new(60, 26)).expect("compact help");
        compact
            .draw(|frame| render_help(frame, frame.area(), ascii_nc))
            .expect("draw compact ascii help");
        let compact_text = compact.backend().to_string();
        assert!(
            compact_text.is_ascii(),
            "non-ASCII compact help:\n{compact_text}"
        );
        assert!(
            compact
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
        let compact_order = [
            "/resume",
            "/new",
            "/window",
            "/rename",
            "/kill",
            "/snapshot",
            "/list",
            "/pane",
            "/split",
            "/layout",
            "/expose",
            "/buffers",
            "/popup",
            "/theme",
            "/config",
            "/clients",
            "/help",
            "/exit",
            "/kill-server",
            "/quit",
        ];
        for pair in compact_order.windows(2) {
            let a = compact_text.find(pair[0]).expect(pair[0]);
            let b = compact_text.find(pair[1]).expect(pair[1]);
            assert!(
                a < b,
                "expected {} before {} in compact help:\n{compact_text}",
                pair[0],
                pair[1]
            );
        }
    }
}
