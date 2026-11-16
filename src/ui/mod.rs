//! Small, tmux-agnostic Ratatui widgets used by JustMux.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub error: Color,
    pub selected: Color,
    pub no_color: bool,
    pub ascii: bool,
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
        }
    }
}
impl Theme {
    pub fn from_env() -> Self {
        Self {
            no_color: std::env::var_os("NO_COLOR").is_some(),
            ascii: !locale_supports_unicode(),
            ..Self::default()
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
                .replace("·", "|")
                .replace("›", ">")
                .replace("…", "...")
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
}

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

pub fn render_home(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    popup: &[CommandItem<'_>],
    selected: usize,
    status: Option<&str>,
    theme: Theme,
) {
    let c = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);
    let body = vec![
        Line::from(Span::styled(
            "JustMux",
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Just tmux. Just simpler."),
        Line::from(""),
        Line::from("Manage your tmux sessions without memorizing commands."),
    ];
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: true }), c[0]);
    frame.render_widget(
        Paragraph::new(input.rendered_width(true, theme, c[1].width)),
        c[1],
    );
    frame.render_widget(
        Paragraph::new(theme.adapt(status.unwrap_or("Type / for commands")))
            .style(theme.style(theme.muted)),
        c[2],
    );
    if !popup.is_empty() {
        render_command_popup(frame, c[1], popup, selected, theme);
    }
}

pub fn render_command_popup(
    frame: &mut Frame<'_>,
    anchor: Rect,
    items: &[CommandItem<'_>],
    selected: usize,
    theme: Theme,
) {
    let h = (items.len() as u16 + 2).min(anchor.y.max(1));
    let area = Rect {
        x: anchor.x,
        y: anchor.y.saturating_sub(h),
        width: anchor.width.min(80),
        height: h,
    };
    frame.render_widget(Clear, area);
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("/{:<12}", c.name),
                    if i == selected {
                        theme.selected_style()
                    } else {
                        theme.style(theme.text)
                    },
                ),
                Span::styled(c.description.to_string(), theme.style(theme.muted)),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(selected.min(items.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(rows).block(Block::default().borders(Borders::TOP)),
        area,
        &mut state,
    );
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
            .title(title)
            .borders(Borders::ALL)
            .style(theme.style(theme.error)),
        r,
    );
    let inner = Rect {
        x: r.x + 2,
        y: r.y + 1,
        width: w.saturating_sub(4),
        height: h.saturating_sub(2),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(theme.adapt(message)),
            Line::from(""),
            Line::from(if yes {
                theme.adapt("[ Yes ]    No")
            } else {
                theme.adapt("  Yes    [ No ]")
            }),
        ])
        .wrap(Wrap { trim: true })
        .style(theme.style(theme.text)),
        inner,
    );
}
pub fn render_help(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let s = theme.adapt("JustMux commands\n\n/resume  attach or switch to a session\n/new     create a session\n/list    list sessions\n/rename  rename a session\n/kill    terminate a session\n/window  manage windows\n/pane    manage panes\n/split   split the current pane\n/help    show this help\n/quit    exit\n\nEsc back · ↑↓ navigate · Enter select · Ctrl+C quit");
    frame.render_widget(
        Paragraph::new(s)
            .wrap(Wrap { trim: true })
            .style(theme.style(theme.text)),
        area,
    );
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
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                render_home(
                    frame,
                    frame.area(),
                    &TextInput::new("/res"),
                    &[CommandItem {
                        name: "resume",
                        description: "Resume session",
                    }],
                    0,
                    None,
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
}
