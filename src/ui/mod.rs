//! Small, tmux-agnostic Ratatui widgets used by JustMux.

mod palette;
pub use palette::*;

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

use crate::commands::{CommandKind, registry};

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
        if !popup.is_empty() {
            render_command_popup(frame, composer, popup, selected, theme);
        } else if !studio.mentions.is_empty() {
            render_mention_popup(
                frame,
                composer,
                studio.mentions,
                studio.mention_selected,
                theme,
            );
        }
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
    let mut composer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(border_color))
        .title_top(Line::from(Span::styled(
            format!(" {} ", studio.mode.label(theme.ascii)),
            theme.style(border_color).add_modifier(Modifier::BOLD),
        )));
    if composer.width >= 48 {
        composer_block = composer_block.title_top(
            Line::from(Span::styled(" Shift+Tab Mode ", theme.style(theme.muted))).right_aligned(),
        );
    }
    let input_area = composer_block.inner(composer);
    frame.render_widget(composer_block, composer);
    render_composer_line(
        frame,
        input_area,
        input,
        studio.mode,
        border_color,
        input_area.width >= 40,
        theme,
    );

    render_home_footer(frame, footer, details.status, theme);

    if !popup.is_empty() || !studio.mentions.is_empty() {
        let drawer_anchor = Rect {
            x: composer.x,
            y: composer.y + 1,
            width: composer.width,
            height: 1,
        };
        if !popup.is_empty() {
            render_command_popup(frame, drawer_anchor, popup, selected, theme);
        } else {
            render_mention_popup(
                frame,
                drawer_anchor,
                studio.mentions,
                studio.mention_selected,
                theme,
            );
        }
    }
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
            let placeholder = truncate_to_width(mode.placeholder(theme.ascii), remaining);
            if !placeholder.is_empty() {
                line.spans
                    .push(Span::styled(placeholder, theme.style(theme.muted)));
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
    let muted = theme.style(theme.muted);
    let bold_accent = accent.add_modifier(Modifier::BOLD);

    let mut title_spans = vec![Span::styled(
        if theme.ascii {
            " [] JustMux"
        } else {
            " ◫ JustMux"
        },
        bold_accent,
    )];
    let version = format!(" (v{})", env!("CARGO_PKG_VERSION"));
    let max_title_width = usize::from(welcome.width.saturating_sub(3));
    let used_width = |spans: &[Span<'_>]| spans.iter().map(Span::width).sum::<usize>();
    if used_width(&title_spans) + version.width() <= max_title_width {
        title_spans.push(Span::styled(version, muted));
    }
    if welcome.width >= 54 {
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
    if used_width(&title_spans) < max_title_width {
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
        let div_glyph = if theme.ascii { "|" } else { "│" };
        let div_lines: Vec<Line<'static>> = (0..divider.height)
            .map(|_| Line::from(Span::styled(div_glyph, muted)))
            .collect();
        frame.render_widget(Paragraph::new(div_lines), divider);
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
    let accent = theme.style(theme.accent);
    let bold_accent = accent.add_modifier(Modifier::BOLD);
    let muted = theme.style(theme.muted);
    let mascot = mascot_lines(theme);
    let mascot_colors = [theme.accent, theme.secondary, theme.tertiary];
    let mascot_w = mascot.iter().map(|line| line.width()).max().unwrap_or(5);
    let info_avail = usize::from(area.width).saturating_sub(mascot_w + 2);

    let dir_line = details.directory.map(|directory| {
        let label = if theme.ascii { "dir " } else { "⌂ " };
        let width = info_avail.saturating_sub(label.width());
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
            truncate_to_width(&summary, info_avail),
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
                info_avail,
            ),
            muted,
        )])
    };

    let info_rows = [dir_line, ctx_line, tele_line];
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
            let rule_len = usize::from(area.width).saturating_sub(1);
            let rule_ch = if theme.ascii { "-" } else { "─" };
            lines.push(Line::from(Span::styled(rule_ch.repeat(rule_len), muted)));
        }
        if usize::from(area.height) >= lines.len() + 6 {
            lines.push(Line::from(Span::styled("Try a command", muted)));
        }
        append_quick_commands(&mut lines, area, "", spacious, theme);
    }
    frame.render_widget(Paragraph::new(lines), area);
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
    let muted = theme.style(theme.muted);
    let accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let max_workspaces = if spacious_list { 6 } else { 4 };
    let header_label = if spacious_list {
        "◈ Live Workspaces (1-6)"
    } else {
        "◈ Live Workspaces (1-5)"
    };
    let header = theme.adapt(header_label);
    let mut lines = vec![Line::from(Span::styled(
        truncate_to_width(&header, usize::from(area.width)),
        accent,
    ))];

    if studio.sessions.is_empty() {
        let launch_label = if theme.ascii {
            " ! Launch workspace in cwd"
        } else {
            " ⚡ Launch workspace in cwd"
        };
        lines.push(Line::from(vec![
            Span::styled(
                "[Enter]",
                theme.style(theme.success).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_to_width(launch_label, usize::from(area.width).saturating_sub(7)),
                theme.style(theme.text),
            ),
        ]));
        if usize::from(area.height) >= 5 {
            lines.push(Line::from(Span::styled(
                truncate_to_width("  or type /new <name>", usize::from(area.width)),
                muted,
            )));
        }
    } else {
        for (idx, session) in studio.sessions.iter().take(max_workspaces).enumerate() {
            if lines.len() >= usize::from(area.height) {
                break;
            }
            let num = format!("[{}] ", idx + 1);
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
            let meta = if session.active_window.is_empty() {
                format!("  {}w", session.windows)
            } else {
                theme.adapt(&format!(
                    "  {}w · {}",
                    session.windows, session.active_window
                ))
            };
            let fixed = num.width() + dot.width() + badge_str.width();
            let avail_for_name = usize::from(area.width).saturating_sub(fixed);
            let name = truncate_to_width(&single_line(&session.name), avail_for_name);
            let rem = avail_for_name.saturating_sub(name.width());
            let mut spans = vec![
                Span::styled(num, accent),
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
                title: "📺 Active Workspace Preview",
                subtitle: &subtitle,
                lines: preview.lines,
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
    let muted = theme.style(theme.muted);
    lines.push(Line::from(Span::styled(
        theme.adapt("✻ Pro Tip"),
        theme.style(theme.secondary).add_modifier(Modifier::BOLD),
    )));
    let tips = [
        "Type @session to jump directly to any session or window",
        "Press 1-5 on an empty bar to attach straight to a workspace",
        "Shift+Tab cycles Command, Live Radar & Quick Launch modes",
        "Ctrl+T cycles 5 TrueColor themes · /theme previews live",
        "Press ↓ on an empty bar to browse all slash commands",
        "Try /codex, /claude, /antigravity, or /vibe for tributes",
        "/split shows live pane wireframes before splitting",
    ];
    let tip = theme.adapt(tips[tip_step % tips.len()]);
    let remaining_rows = usize::from(max_height).saturating_sub(lines.len());
    for wrapped in wrap_description(&tip, width)
        .into_iter()
        .take(remaining_rows)
    {
        lines.push(Line::from(Span::styled(wrapped, muted)));
    }
}

fn render_live_radar(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: HomeDashboardState<'_>,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (left, right) = if area.width >= 54 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
            .areas(area);
        (l, Some(r))
    } else {
        (area, None)
    };

    let muted = theme.style(theme.muted);
    let header_title = theme.adapt(&format!("◈ Live Radar ({})", studio.sessions.len()));
    let mut left_lines = vec![Line::from(Span::styled(
        truncate_to_width(&header_title, usize::from(left.width)),
        theme.style(theme.secondary).add_modifier(Modifier::BOLD),
    ))];

    if studio.sessions.is_empty() {
        left_lines.push(Line::from(Span::styled(
            truncate_to_width("  No active tmux sessions", usize::from(left.width)),
            muted,
        )));
        left_lines.push(Line::from(Span::styled(
            truncate_to_width("  Press Enter to launch or /new", usize::from(left.width)),
            muted,
        )));
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
            let win_tag = format!(" {}w", session.windows);
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
        let preview = studio.preview.unwrap_or(PanePreview {
            title: "Live Terminal Preview",
            subtitle: "idle",
            lines: &[],
        });
        render_pane_preview_card(frame, right_area, preview, theme);
    }
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
            lines.push(Line::from(Span::styled("Try a command", muted)));
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
    for command in quick_start
        .iter()
        .filter_map(|&kind| registry().iter().find(|command| command.kind == kind))
    {
        if lines.len() >= usize::from(area.height) {
            break;
        }
        let cmd = format!("{prefix}/{}", command.name);
        let mut spans = vec![Span::styled(cmd, command_style)];
        if area.width >= 34 {
            let pad = " ".repeat(9_usize.saturating_sub(command.name.len()));
            let used = prefix.width() + 1 + command.name.width() + pad.width();
            let desc_avail = usize::from(area.width).saturating_sub(used);
            if desc_avail > 0 {
                spans.push(Span::raw(pad));
                spans.push(Span::styled(
                    truncate_to_width(&theme.adapt(command.description), desc_avail),
                    muted,
                ));
            }
        }
        lines.push(Line::from(spans));
    }
}

fn render_home_footer(frame: &mut Frame<'_>, footer: Rect, status: Option<&str>, theme: Theme) {
    if footer.width == 0 || footer.height == 0 {
        return;
    }
    if let Some(status) = status {
        let style = status_style(status, theme);
        frame.render_widget(Paragraph::new(theme.adapt(status)).style(style), footer);
        return;
    }
    let width = usize::from(footer.width);
    let full = theme.adapt("Type / for commands · ↓ Menu · Ctrl+T Theme · ? Help");
    let medium = theme.adapt("Type / for commands · ↓ Menu · Ctrl+T Theme");
    let short = theme.adapt("Type / for commands · ? Help");
    let text = if width >= full.width() {
        full
    } else if width >= medium.width() {
        medium
    } else if width >= short.width() {
        short
    } else {
        "Type / for commands".to_owned()
    };
    frame.render_widget(Paragraph::new(text).style(theme.style(theme.muted)), footer);
}

fn status_style(status: &str, theme: Theme) -> Style {
    let trimmed = status.trim_start();
    if trimmed.starts_with("Renamed")
        || trimmed.starts_with("Refreshed")
        || trimmed.starts_with("Pane split")
        || trimmed.starts_with("Session created")
        || trimmed.starts_with("Window created")
        || trimmed.starts_with("Saved")
    {
        theme.style(theme.success)
    } else if trimmed.starts_with('✦')
        || trimmed.starts_with('✻')
        || trimmed.starts_with('◈')
        || trimmed.starts_with('🎯')
        || trimmed.starts_with('🔥')
        || trimmed.starts_with(">_")
        || trimmed.starts_with("Cancelled")
    {
        theme.style(theme.accent)
    } else {
        theme.style(theme.error)
    }
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
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
    if sessions.is_none() && windows.is_none() && telemetry.attached.is_none() {
        return None;
    }
    let sep = if theme.ascii { " | " } else { " · " };
    let mut parts = Vec::new();
    if let Some(s) = sessions {
        parts.push(format!("{s}s"));
    }
    if let Some(w) = windows {
        parts.push(format!("{w}w"));
    }
    if let Some(a) = telemetry.attached.filter(|&a| a > 0) {
        let dot = if theme.ascii { "a" } else { "●" };
        parts.push(format!("{a}{dot}"));
    }
    (!parts.is_empty()).then(|| parts.join(sep))
}

fn single_line(text: &str) -> String {
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
            spans.push(Span::styled(theme.adapt(c.description), desc_style));
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
        block = block.title(Line::from(vec![
            Span::styled("─ ", theme.style(theme.muted)),
            Span::styled(
                "✦ Commands",
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ↑↓ Tab Enter ", theme.style(theme.muted)),
        ]));
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
        block = block.title(Line::from(vec![
            Span::styled("─ ", theme.style(theme.muted)),
            Span::styled(
                "@ Targets",
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ↑↓ Tab Enter ", theme.style(theme.muted)),
        ]));
    }
    frame.render_stateful_widget(List::new(rows).block(block), area, &mut state);
}

/// Rounded card rendering captured terminal output lines cleanly without wrapping distortion.
pub fn render_pane_preview_card(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: PanePreview<'_>,
    theme: Theme,
) {
    if area.width < 6 || area.height < 3 {
        return;
    }
    let muted = theme.style(theme.muted);
    let adapted_title = theme.adapt(preview.title);
    let has_leading_icon = preview.title.starts_with('📺')
        || preview.title.starts_with('◈')
        || preview.title.starts_with("[]");
    let title_label = if has_leading_icon {
        format!(" {adapted_title}")
    } else if theme.ascii {
        format!(" [] {adapted_title}")
    } else {
        format!(" ◈ {adapted_title}")
    };
    let mut title_spans = vec![Span::styled(
        title_label,
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if !preview.subtitle.is_empty() {
        let sep = if theme.ascii { " | " } else { " · " };
        let sub = format!("{sep}{}", theme.adapt(preview.subtitle));
        let used: usize = title_spans.iter().map(Span::width).sum();
        if used + sub.width() < usize::from(area.width.saturating_sub(2)) {
            title_spans.push(Span::styled(sub, muted));
        }
    }
    title_spans.push(Span::raw(" "));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.secondary))
        .title(Line::from(title_spans));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if preview.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(" (empty terminal buffer)", usize::from(inner.width)),
                muted,
            ))),
            inner,
        );
        return;
    }
    let max_rows = usize::from(inner.height);
    let skip = preview.lines.len().saturating_sub(max_rows);
    let lines: Vec<Line<'static>> = preview
        .lines
        .iter()
        .skip(skip)
        .take(max_rows)
        .map(|raw| {
            let clean = theme.adapt(&single_line(raw));
            Line::from(Span::styled(
                truncate_to_width(&clean, usize::from(inner.width)),
                theme.style(theme.text),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
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
        Paragraph::new(theme.adapt(footer)).style(theme.style(theme.muted)),
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
        "Filter: "
    };
    let mut query_line = query.rendered_width(
        true,
        theme,
        search_area.width.saturating_sub(prompt.len() as u16),
    );
    if query.value().is_empty() && search_area.width >= 32 {
        let ghost = theme.adapt("type to narrow…");
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
        let max_primary = rows
            .iter()
            .filter(|row| !row.is_action)
            .map(|row| row.primary.width())
            .max()
            .unwrap_or(0)
            .min(usize::from(list_area.width) / 2);
        let bg = theme.pill_bg();
        let items: Vec<ListItem<'_>> = rows
            .iter()
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
                        format!(" {} ", badge_text),
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
            .collect();
        let mut state = ListState::default();
        state.select(Some(selected.min(rows.len() - 1)));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
    }

    if let Some((preview_rect, card)) = preview_area {
        render_pane_preview_card(frame, preview_rect, card, theme);
    }

    frame.render_widget(
        Paragraph::new(theme.adapt(footer)).style(theme.style(theme.muted)),
        footer_area,
    );
}

/// Interactive `/split` layout chooser with live GUI pane wireframe previews.
pub fn render_split_selector(
    frame: &mut Frame<'_>,
    area: Rect,
    target_label: &str,
    selected: usize,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let selected = selected.min(1);
    let [title_area, list_area, preview_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(u16::from(area.height >= 4)),
    ])
    .areas(area);

    let mut title_spans = vec![Span::styled(
        "Split pane",
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if !target_label.is_empty() {
        let label = format!("  {}", single_line(target_label));
        if "Split pane".width() + label.width() <= usize::from(title_area.width) {
            title_spans.push(Span::styled(theme.adapt(&label), theme.style(theme.muted)));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(title_spans)), title_area);

    let options = [
        "Side by side — new pane on the right (tmux -h)",
        "Stacked — new pane below (tmux -v)",
    ];
    let list_lines: Vec<Line<'static>> = options
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let is_sel = i == selected;
            let style = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            Line::from(Span::styled(
                format!("{marker} {}", theme.adapt(label)),
                style,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(list_lines), list_area);

    if preview_area.height >= 5 && preview_area.width >= 16 {
        let body = Rect {
            y: preview_area.y + 1,
            height: preview_area.height - 1,
            ..preview_area
        };
        if body.width >= 36 {
            let [left, right] =
                Layout::horizontal([Constraint::Length(17), Constraint::Min(17)]).areas(body);
            frame.render_widget(
                Paragraph::new(side_by_side_wireframe(selected == 0, theme)),
                left,
            );
            frame.render_widget(
                Paragraph::new(stacked_wireframe(selected == 1, theme)),
                right,
            );
        } else {
            let lines = if selected == 0 {
                side_by_side_wireframe(true, theme)
            } else {
                stacked_wireframe(true, theme)
            };
            frame.render_widget(Paragraph::new(lines), body);
        }
    }

    frame.render_widget(
        Paragraph::new(theme.adapt("↑↓ / ←→ / Tab Choose · Enter Split · Esc Back"))
            .style(theme.style(theme.muted)),
        footer_area,
    );
}

fn side_by_side_wireframe(active: bool, theme: Theme) -> Vec<Line<'static>> {
    let border = if active {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let new_style = if active {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let main_style = if active {
        theme.style(theme.text)
    } else {
        theme.style(theme.muted)
    };
    if theme.ascii {
        vec![
            Line::from(Span::styled("+------+------+", border)),
            Line::from(vec![
                Span::styled("| ", border),
                Span::styled("main", main_style),
                Span::styled(" | ", border),
                Span::styled("+new", new_style),
                Span::styled(" |", border),
            ]),
            Line::from(Span::styled("|      |      |", border)),
            Line::from(Span::styled("+------+------+", border)),
        ]
    } else {
        vec![
            Line::from(Span::styled("╭──────┬──────╮", border)),
            Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("main", main_style),
                Span::styled(" │ ", border),
                Span::styled("+new", new_style),
                Span::styled(" │", border),
            ]),
            Line::from(Span::styled("│      │      │", border)),
            Line::from(Span::styled("╰──────┴──────╯", border)),
        ]
    }
}

fn stacked_wireframe(active: bool, theme: Theme) -> Vec<Line<'static>> {
    let border = if active {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let new_style = if active {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let main_style = if active {
        theme.style(theme.text)
    } else {
        theme.style(theme.muted)
    };
    if theme.ascii {
        vec![
            Line::from(Span::styled("+-------------+", border)),
            Line::from(vec![
                Span::styled("| ", border),
                Span::styled("main        ", main_style),
                Span::styled("|", border),
            ]),
            Line::from(Span::styled("+-------------+", border)),
            Line::from(vec![
                Span::styled("| ", border),
                Span::styled("+new        ", new_style),
                Span::styled("|", border),
            ]),
            Line::from(Span::styled("+-------------+", border)),
        ]
    } else {
        vec![
            Line::from(Span::styled("╭─────────────╮", border)),
            Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("main        ", main_style),
                Span::styled("│", border),
            ]),
            Line::from(Span::styled("├─────────────┤", border)),
            Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("+new        ", new_style),
                Span::styled("│", border),
            ]),
            Line::from(Span::styled("╰─────────────╯", border)),
        ]
    }
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
    let mut title_line = Line::from(gradient_spans("Theme palette", theme, true));
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
            Paragraph::new("No themes available").style(theme.style(theme.muted)),
            list_area,
        );
    } else {
        let clamped_selected = selected.min(presets.len() - 1);
        let items: Vec<ListItem<'_>> = presets
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
                if area.width >= 52 {
                    spans.push(Span::raw(" "));
                    spans.push(pill_span(
                        format!(" {} ", preset.origin()),
                        candidate.accent,
                        candidate.pill_bg(),
                        theme,
                        false,
                    ));
                }
                if !show_preview && area.width >= 82 {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        theme.adapt(preset.tagline()),
                        theme.style(theme.muted),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let mut state = ListState::default();
        state.select(Some(clamped_selected));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);

        if show_preview {
            let preview_preset = presets[clamped_selected];
            let preview_theme = theme.with_preset(preview_preset);
            let mut preview_lines = vec![Line::default()];
            let mut sample = vec![Span::styled(
                format!("{} ", preview_preset.badge_icon(theme.ascii)),
                preview_theme
                    .style(preview_theme.accent)
                    .add_modifier(Modifier::BOLD),
            )];
            sample.extend(gradient_spans("Preview", preview_theme, true));
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
                    theme.adapt("session grid  ·  "),
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
            frame.render_widget(Paragraph::new(preview_lines), preview_area);
        }
    }

    frame.render_widget(
        Paragraph::new(
            theme.adapt("↑↓ Live Preview · Enter Apply · s Save to config.toml · Esc Revert"),
        )
        .style(theme.style(theme.muted)),
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

    let title_text = theme.adapt(" ⚙ JustMux Configuration ");
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
    let path_prefix = "Config: ";
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
            Paragraph::new("No settings available").style(muted),
            list_area,
        );
    } else {
        let clamped_selected = selected.min(items.len() - 1);
        let key_col = items
            .iter()
            .map(|item| item.key.width())
            .max()
            .unwrap_or(10)
            .clamp(10, 22);
        let bg = theme.pill_bg();
        let list_items: Vec<ListItem<'_>> = items
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
                let mut spans = vec![
                    Span::styled(format!("{marker} "), key_style),
                    Span::styled(padded_key, key_style),
                    Span::raw("  "),
                    pill,
                ];
                let used = 2 + key_col + 2 + spans.last().map_or(0, Span::width);
                let desc_avail = usize::from(list_area.width).saturating_sub(used + 2);
                if desc_avail >= 4 && !item.description.is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        truncate_to_width(&theme.adapt(item.description), desc_avail),
                        muted,
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
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
        Paragraph::new(truncate_to_width(
            &theme.adapt("↑↓ Select · ←→/Enter Change · s Save to config.toml · Esc Close"),
            usize::from(footer_area.width),
        ))
        .style(muted),
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
            Paragraph::new(theme.adapt(modal.footer_hint)).style(theme.style(theme.muted)),
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
        Paragraph::new(theme.adapt(footer)).style(theme.style(theme.muted)),
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
            format!("{warn_icon} Approval Required"),
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        ),
        Span::styled(sep, theme.style(theme.muted)),
        pill_span(
            "[sandbox: native-tmux]",
            theme.secondary,
            theme.pill_bg(),
            theme,
            false,
        ),
    ]);
    frame.render_widget(Paragraph::new(banner), banner_area);

    let diff_text = format!("- kill {}", single_line(target_summary));
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
        Paragraph::new(theme.adapt(footer)).style(theme.style(theme.muted)),
        footer_area,
    );
    scroll
}

fn wrap_description(text: &str, width: u16) -> Vec<String> {
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
                "[ Yes ]",
                theme.style(theme.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    No", theme.style(theme.muted)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  Yes    ", theme.style(theme.muted)),
            Span::styled(
                "[ No ]",
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
        header.extend(gradient_spans("JustMux commands", theme, true));
        header.push(Span::styled(
            theme.adapt("  ·  zero-memorization tmux"),
            muted,
        ));
        lines.push(Line::from(header));
        if area.height >= 14 {
            lines.push(Line::default());
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{:<35}", "Sessions"), sec_style),
            Span::styled("Windows, Panes & App", sec_style),
        ]));
        let pairs = [
            (
                "/resume",
                "session & window grid",
                "/window",
                "open, create, rename, close",
            ),
            (
                "/new",
                "create a tmux session",
                "/pane",
                "open or close panes",
            ),
            (
                "/list",
                "browse sessions & actions",
                "/split",
                "visual split direction preview",
            ),
            (
                "/rename",
                "rename a session",
                "/theme",
                "switch TrueColor palettes",
            ),
            (
                "/kill",
                "terminate a session",
                "/config",
                "settings & save config.toml",
            ),
            ("/help", "show this guide", "/exit", "detach tmux client"),
            ("", "", "/quit", "quit JustMux"),
        ];
        for (l_cmd, l_desc, r_cmd, r_desc) in pairs {
            let left_pad = 9_usize.saturating_sub(l_cmd.len());
            let right_col_pad = 35_usize.saturating_sub(l_cmd.len() + left_pad + l_desc.len());
            let right_pad = 9_usize.saturating_sub(r_cmd.len());
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
        if area.height >= 12 {
            lines.push(Line::default());
        }
        lines.push(Line::from(vec![
            Span::styled("Tributes  ", muted),
            Span::styled(
                theme.adapt("/codex · /claude · /antigravity · /vibe"),
                theme.style(theme.accent),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            theme.adapt(
                "Esc back · ↑↓ navigate · Tab complete · Enter select · Ctrl+T theme · Ctrl+C quit",
            ),
            muted,
        )));
        frame.render_widget(Paragraph::new(lines), area);
    } else {
        let mut lines = vec![Line::from(gradient_spans("JustMux commands", theme, true))];
        let entries = [
            ("/resume", "session & window grid"),
            ("/new", "create a session"),
            ("/list", "browse sessions & actions"),
            ("/rename", "rename a session"),
            ("/kill", "terminate a session"),
            ("/window", "manage windows"),
            ("/pane", "manage panes"),
            ("/split", "split current pane"),
            ("/theme", "switch theme palette"),
            ("/config", "settings & save config.toml"),
            ("/exit", "detach from tmux"),
            ("/help", "show this help"),
            ("/quit", "quit JustMux"),
        ];
        for (cmd, desc) in entries {
            let pad = 9_usize.saturating_sub(cmd.len());
            lines.push(Line::from(vec![
                Span::styled(cmd, cmd_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(desc, text_style),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("Also: ", muted),
            Span::styled(
                theme.adapt("/codex · /claude · /antigravity · /vibe"),
                theme.style(theme.accent),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            theme.adapt("Esc back · ↑↓ navigate · Enter select · Ctrl+C quit"),
            muted,
        )));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
}

#[cfg(test)]
mod tests {
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
    fn split_selector_theme_picker_rich_picker_and_tribute_modal_render_cleanly() {
        let theme = Theme::default().with_preset(ThemePreset::CodexEmerald);
        let mut terminal = Terminal::new(TestBackend::new(72, 16)).expect("terminal");

        terminal
            .draw(|frame| render_split_selector(frame, frame.area(), "work:1.0 zsh", 0, theme))
            .expect("split");
        let split_text = terminal.backend().to_string();
        assert!(split_text.contains("╭──────┬──────╮"));
        assert!(split_text.contains("╭─────────────╮"));

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
                render_split_selector(frame, frame.area(), "work:1.0 zsh", 1, ascii_no_color)
            })
            .expect("ascii split");
        assert!(terminal.backend().to_string().is_ascii());
        assert!(terminal.backend().to_string().contains("+-------------+"));
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
                        }),
                        tip_step: 0,
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
                        }),
                        tip_step: 0,
                    },
                    theme,
                );
            })
            .expect("draw spacious");
        let studio_out = spacious.backend().to_string();
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
}
