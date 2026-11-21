use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    fuzzy::{FuzzyMatch, FuzzyMatcher},
    tmux::{Session, SessionId, Target, Window},
    ui::{TextInput, Theme, highlighted_spans},
};

#[derive(Debug, Clone)]
struct Column {
    session: Session,
    windows: Vec<Window>,
}

#[derive(Debug, Clone)]
struct VisibleWindow {
    index: usize,
    matched: Option<FuzzyMatch>,
}

#[derive(Debug, Clone)]
struct VisibleColumn {
    index: usize,
    matched: Option<FuzzyMatch>,
    windows: Vec<VisibleWindow>,
    initial_row: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResumePicker {
    columns: Vec<Column>,
    visible: Vec<VisibleColumn>,
    query: TextInput,
    selected_column: usize,
    selected_row: usize,
    first_column: usize,
    first_row: usize,
}

impl ResumePicker {
    pub(super) fn replace(
        &mut self,
        sessions: Vec<Session>,
        windows: Vec<Window>,
        matcher: &mut FuzzyMatcher,
    ) {
        let mut grouped: HashMap<_, Vec<_>> = HashMap::new();
        for window in windows {
            grouped
                .entry(window.session_id.clone())
                .or_default()
                .push(window);
        }
        self.columns = sessions
            .into_iter()
            .filter_map(|session| {
                let mut windows = grouped.remove(&session.id)?;
                windows.sort_by_key(|window| window.index);
                Some(Column { session, windows })
            })
            .collect();
        self.refresh_matches(matcher);
    }

    // Reuse matches on navigation and resize. Recompute after edits or a tmux
    // refresh, when the query or resource names may have changed.
    fn refresh_matches(&mut self, matcher: &mut FuzzyMatcher) {
        let query = self.query.value();
        let mut visible: Vec<_> = self
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                let matched = matcher.match_one(query, &column.session.name);
                let mut windows: Vec<_> = column
                    .windows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, window)| {
                        let hit = matcher.match_one(query, &window.name);
                        (matched.is_some() || hit.is_some()).then_some(VisibleWindow {
                            index,
                            matched: hit,
                        })
                    })
                    .collect();
                if matched.is_none() {
                    windows.sort_by_key(|window| window.matched.as_ref().map(FuzzyMatch::sort_key));
                }
                let session_rank = matched.as_ref().map(FuzzyMatch::sort_key);
                let best_window = windows
                    .iter()
                    .enumerate()
                    .filter_map(|(row, window)| {
                        let rank = window.matched.as_ref()?.sort_key();
                        Some((rank, !column.windows[window.index].active, row))
                    })
                    .min();
                let column_rank = session_rank
                    .into_iter()
                    .chain(best_window.map(|(rank, _, _)| rank))
                    .min()?;
                let initial_row = match best_window {
                    Some((rank, _, row)) if session_rank.is_none_or(|session| rank < session) => {
                        row
                    }
                    _ => windows
                        .iter()
                        .position(|window| column.windows[window.index].active)
                        .unwrap_or(0),
                };
                Some((
                    column_rank,
                    VisibleColumn {
                        index,
                        matched,
                        windows,
                        initial_row,
                    },
                ))
            })
            .collect();
        visible.sort_by_key(|(rank, _)| *rank);
        self.visible = visible.into_iter().map(|(_, column)| column).collect();
        self.selected_column = 0;
        self.selected_row = self.visible.first().map_or(0, |column| column.initial_row);
        self.first_column = 0;
        self.first_row = 0;
    }

    pub(super) fn paste(&mut self, text: &str, matcher: &mut FuzzyMatcher) {
        self.query.insert_str(text);
        self.refresh_matches(matcher);
    }

    pub(super) fn handle_key(
        &mut self,
        key: KeyEvent,
        matcher: &mut FuzzyMatcher,
    ) -> Option<Target> {
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Left | KeyCode::Right if plain => {
                let next = if key.code == KeyCode::Left {
                    self.selected_column.saturating_sub(1)
                } else {
                    (self.selected_column + 1).min(self.visible.len().saturating_sub(1))
                };
                if next != self.selected_column {
                    let row = self.visible[next].initial_row;
                    self.selected_column = next;
                    self.selected_row = row;
                    self.first_row = 0;
                }
            }
            KeyCode::Up if plain => self.selected_row = self.selected_row.saturating_sub(1),
            KeyCode::Down if plain => {
                let count = self
                    .visible
                    .get(self.selected_column)
                    .map_or(0, |column| column.windows.len());
                self.selected_row = (self.selected_row + 1).min(count.saturating_sub(1));
            }
            KeyCode::Enter => {
                let (column, window) = self.selected_entry()?;
                return Some(Target::Window(column.session.id.clone(), window.id.clone()));
            }
            _ => {
                let before = self.query.value().to_owned();
                if self.query.handle_key(key) && self.query.value() != before {
                    self.refresh_matches(matcher);
                }
            }
        }
        None
    }

    fn selected_entry(&self) -> Option<(&Column, &Window)> {
        let visible = self.visible.get(self.selected_column)?;
        let column = self.columns.get(visible.index)?;
        let window = column
            .windows
            .get(visible.windows.get(self.selected_row)?.index)?;
        Some((column, window))
    }

    #[allow(dead_code)]
    pub(super) fn selected_target_info(&self) -> Option<(Target, String)> {
        let (column, window) = self.selected_entry()?;
        Some((
            Target::Window(column.session.id.clone(), window.id.clone()),
            format!("{}:{} {}", column.session.name, window.index, window.name),
        ))
    }

    pub(super) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        current_session: Option<&SessionId>,
        status: &str,
        theme: Theme,
    ) {
        self.render_with_preview(frame, area, current_session, status, None, theme);
    }

    pub(super) fn render_with_preview(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        current_session: Option<&SessionId>,
        status: &str,
        preview: Option<&[String]>,
        theme: Theme,
    ) {
        let [title, search, body, footer] = Layout::vertical([
            Constraint::Length(u16::from(area.height >= 5)),
            Constraint::Length(u16::from(area.height >= 4)),
            Constraint::Min(1),
            Constraint::Length(u16::from(
                area.height >= 5 || (!status.is_empty() && area.height >= 3),
            )),
        ])
        .areas(area);
        let accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
        let muted = theme.style(theme.muted);
        let columns = &self.visible;
        let shown = usize::from((body.width / 20).max(1)).min(columns.len());
        let heading = if shown < columns.len() {
            format!("Resume  [{}/{}]", self.selected_column + 1, columns.len())
        } else {
            "Resume".into()
        };
        let mut title_spans = vec![Span::styled(heading, accent)];
        if area.width >= 44 && !theme.ascii && !columns.is_empty() {
            let total_windows: usize = columns.iter().map(|column| column.windows.len()).sum();
            let left_style = if self.selected_column > 0 {
                theme.style(theme.accent)
            } else {
                muted
            };
            let right_style = if self.selected_column + 1 < columns.len() {
                theme.style(theme.accent)
            } else {
                muted
            };
            title_spans.push(Span::styled("  ", muted));
            title_spans.push(Span::styled("◀", left_style));
            title_spans.push(Span::styled(" ", muted));
            title_spans.push(Span::styled("▶", right_style));
            title_spans.push(Span::styled(format!(" · {total_windows}w"), muted));
        }
        frame.render_widget(Paragraph::new(Line::from(title_spans)), title);
        let prompt = if search.width < 14 { "? " } else { "Search: " };
        let mut query = self.query.rendered_width(
            true,
            theme,
            search.width.saturating_sub(prompt.len() as u16),
        );
        query.spans.insert(0, Span::styled(prompt, muted));
        if self.query.value().is_empty() && search.width >= 42 {
            let placeholder = theme.adapt("filter sessions or windows…");
            if query.width() + placeholder.width() <= usize::from(search.width) {
                query.spans.push(Span::styled(placeholder, muted));
            }
        }
        frame.render_widget(Paragraph::new(query), search);
        let hint = if theme.ascii {
            "Arrows | Enter Open | Esc Back"
        } else if area.width < 48 {
            "←→ ↑↓ · Enter · Esc"
        } else {
            "←→ Session · ↑↓ Window · Enter Open · Esc Back"
        };
        let (footer_text, style) = if status.is_empty() {
            (hint, muted)
        } else {
            (status, theme.style(theme.error))
        };
        frame.render_widget(Paragraph::new(footer_text).style(style), footer);

        if shown == 0 {
            let label = if self.columns.is_empty() {
                "No tmux sessions"
            } else {
                "No matching sessions or windows"
            };
            frame.render_widget(Paragraph::new(label).style(muted), body);
            return;
        }
        let (body, preview_area) = match preview {
            Some(lines) if area.height >= 14 && area.width >= 64 && !columns.is_empty() => {
                let [top, bottom] =
                    Layout::vertical([Constraint::Min(6), Constraint::Length(5)]).areas(body);
                (top, Some((bottom, lines)))
            }
            _ => (body, None),
        };
        let first_column = scroll_to(
            self.first_column,
            self.selected_column,
            shown,
            columns.len(),
        );
        let header_rows = match body.height {
            0 | 1 => 0,
            2 | 3 => 1,
            _ => 2,
        };
        let row_count = usize::from(body.height.saturating_sub(header_rows));
        let selected_windows = columns[self.selected_column].windows.len();
        let first_row = scroll_to(
            self.first_row,
            self.selected_row,
            row_count.max(1),
            selected_windows,
        );
        let areas = Layout::horizontal(vec![Constraint::Fill(1); shown])
            .spacing(1)
            .split(body);
        for ((index, visible), area) in columns
            .iter()
            .enumerate()
            .skip(first_column)
            .zip(areas.iter())
        {
            let column = &self.columns[visible.index];
            let focused = index == self.selected_column;
            let current = current_session == Some(&column.session.id);
            let session_indices = visible
                .matched
                .as_ref()
                .map_or(&[][..], |matched| matched.indices.as_slice());
            let mut lines = Vec::new();
            if header_rows > 0 {
                let style = if focused {
                    accent
                } else {
                    theme.style(theme.text).add_modifier(Modifier::BOLD)
                };
                let mut spans = Vec::new();
                if current {
                    spans.push(Span::styled("* ", style));
                }
                spans.extend(highlighted_spans(
                    &column.session.name,
                    session_indices,
                    style,
                ));
                lines.push(Line::from(spans));
            }
            if header_rows == 2 {
                let status_span = match (column.session.is_attached(), theme.ascii) {
                    (true, false) => Span::styled("● attached", theme.style(Color::Green)),
                    (true, true) => Span::styled("attached", theme.style(Color::Green)),
                    (false, false) => Span::styled("○ detached", muted),
                    (false, true) => Span::styled("detached", muted),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}w ", column.session.windows), muted),
                    status_span,
                ]));
            }
            let offset = if focused { first_row } else { 0 };
            for (row, matched_window) in visible
                .windows
                .iter()
                .enumerate()
                .skip(offset)
                .take(row_count)
            {
                let window = &column.windows[matched_window.index];
                let indices = matched_window
                    .matched
                    .as_ref()
                    .map_or(&[][..], |matched| matched.indices.as_slice());
                let selected = focused && row == self.selected_row;
                let style = if selected {
                    theme.selected_style()
                } else {
                    theme.style(theme.text)
                };
                let mut spans = vec![
                    Span::styled(if selected { theme.marker() } else { " " }, style),
                    Span::styled(" ", style),
                ];
                if header_rows == 0 {
                    spans.extend(highlighted_spans(
                        &column.session.name,
                        session_indices,
                        style,
                    ));
                    spans.push(Span::styled(":", style));
                } else {
                    let index = window.index.to_string();
                    if window.active {
                        if selected {
                            spans.push(Span::styled(format!("{index}* "), style));
                        } else {
                            spans.push(Span::styled(index, style));
                            spans.push(Span::styled("*", theme.style(theme.accent)));
                            spans.push(Span::styled(" ", style));
                        }
                    } else {
                        spans.push(Span::styled(format!("{index} "), style));
                    }
                }
                spans.extend(highlighted_spans(&window.name, indices, style));
                if window.panes > 1 && area.width >= 26 {
                    let badge = theme.adapt(&format!(" · {}p", window.panes));
                    let used: usize = spans.iter().map(|span| span.content.width()).sum();
                    if used + badge.width() <= usize::from(area.width) {
                        let badge_style = if selected { style } else { muted };
                        spans.push(Span::styled(badge, badge_style));
                    }
                }
                lines.push(Line::from(spans));
            }
            frame.render_widget(Paragraph::new(lines), *area);
        }
        if let Some((card_area, preview_lines)) = preview_area {
            let target_label = self
                .selected_entry()
                .map_or_else(String::new, |(col, win)| {
                    format!("{}:{}", col.session.name, win.name)
                });
            render_preview_card(frame, card_area, &target_label, preview_lines, theme);
        }
        self.first_column = first_column;
        self.first_row = first_row;
    }
}

const ASCII_PREVIEW_BORDER: symbols::border::Set<'_> = symbols::border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

fn render_preview_card(
    frame: &mut Frame<'_>,
    area: Rect,
    target_label: &str,
    lines: &[String],
    theme: Theme,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let title = if target_label.is_empty() {
        vec![
            Span::raw(" "),
            Span::styled(
                "Window Preview",
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]
    } else {
        vec![
            Span::raw(" "),
            Span::styled(
                "Window Preview",
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(theme.adapt(" · "), theme.style(theme.muted)),
            Span::styled(single_line(target_label), theme.style(theme.text)),
            Span::raw(" "),
        ]
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.style(theme.muted))
        .title(Line::from(title));
    block = if theme.ascii {
        block.border_set(ASCII_PREVIEW_BORDER)
    } else {
        block.border_type(BorderType::Rounded)
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let content: Vec<Line<'static>> = if lines.is_empty() {
        vec![Line::styled(
            "  (empty shell pane)",
            theme.style(theme.muted),
        )]
    } else {
        lines
            .iter()
            .take(3)
            .map(|line| Line::styled(format!(" {}", single_line(line)), theme.style(theme.text)))
            .collect()
    };
    frame.render_widget(Paragraph::new(content), inner);
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn scroll_to(offset: usize, selected: usize, shown: usize, count: usize) -> usize {
    offset
        .clamp((selected + 1).saturating_sub(shown), selected)
        .min(count.saturating_sub(shown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::WindowId;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn grid_scrolls_both_axes_and_filters_session_or_window() {
        let sessions: Vec<_> = ["alpha", "build", "project"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Session {
                id: SessionId::new(format!("${index}")).expect("session ID"),
                name: name.into(),
                windows: 6,
                attached: 0,
                created: 0,
                last_attached: None,
                activity: 0,
                path: String::new(),
            })
            .collect();
        let windows = sessions
            .iter()
            .enumerate()
            .flat_map(|(column, session)| {
                (0..6).map(move |index| Window {
                    id: WindowId::new(format!("@{}", column * 10 + index as usize))
                        .expect("window ID"),
                    session_id: session.id.clone(),
                    session_name: session.name.clone(),
                    index,
                    name: match (column, index) {
                        (0, 1) => "last-log-dev".into(),
                        (2, 4) => "last-log-live".into(),
                        (2, 5) => "last-log".into(),
                        _ => format!("shell-{index}"),
                    },
                    active: index == 0,
                    panes: 1,
                })
            })
            .collect();
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(sessions, windows, &mut matcher);
        let mut wide = Terminal::new(TestBackend::new(72, 12)).expect("wide screen");
        wide.draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw grid");
        assert!(
            wide.backend()
                .to_string()
                .lines()
                .any(|line| ["alpha", "build", "project"]
                    .iter()
                    .all(|name| line.contains(name))),
            "session headers should share a row"
        );

        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for code in [KeyCode::Right, KeyCode::Right]
            .into_iter()
            .chain([KeyCode::Down; 5])
        {
            picker.handle_key(key(code), &mut matcher);
        }
        let mut narrow = Terminal::new(TestBackend::new(28, 7)).expect("narrow screen");
        narrow
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw narrow grid");
        let screen = narrow.backend().to_string();
        assert!(screen.contains("Resume  [3/3]"));
        assert!(screen.contains("› 5 last-log"));
        assert!(!screen.contains("alpha"));
        let chosen = picker
            .handle_key(key(KeyCode::Enter), &mut matcher)
            .expect("window target");
        assert_eq!(chosen.to_string(), "$2:@25");

        picker.paste("last-log", &mut matcher);
        assert_eq!(picker.visible.len(), 2);
        assert_eq!(picker.visible[0].index, 2);
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter), &mut matcher),
            Some(chosen)
        );
        narrow
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw matching rows");
        assert!(
            narrow.backend().buffer().content().iter().any(|cell| {
                cell.symbol() == "l"
                    && cell
                        .modifier
                        .contains(Modifier::UNDERLINED | Modifier::REVERSED)
            }),
            "the best match should be selected with its matching text highlighted"
        );
        picker.query.set_value("build");
        picker.refresh_matches(&mut matcher);
        assert_eq!(picker.visible[0].windows.len(), 6);
        assert_eq!(
            picker
                .handle_key(key(KeyCode::Enter), &mut matcher)
                .expect("active window target")
                .to_string(),
            "$1:@10"
        );
    }

    fn make_session(id: &str, name: &str, windows: usize, attached: usize) -> Session {
        Session {
            id: SessionId::new(id).expect("session ID"),
            name: name.into(),
            windows,
            attached,
            created: 0,
            last_attached: None,
            activity: 0,
            path: String::new(),
        }
    }

    fn make_window(
        id: &str,
        session: &Session,
        index: u32,
        name: &str,
        active: bool,
        panes: usize,
    ) -> Window {
        Window {
            id: WindowId::new(id).expect("window ID"),
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            index,
            name: name.into(),
            active,
            panes,
        }
    }

    #[test]
    fn wide_render_shows_summary_placeholder_status_dots_active_star_and_pane_badges() {
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 3),
            make_window("@2", &beta, 0, "api", true, 2),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![alpha.clone(), beta], windows, &mut matcher);

        let mut terminal = Terminal::new(TestBackend::new(72, 12)).expect("wide screen");
        terminal
            .draw(|frame| picker.render(frame, frame.area(), Some(&alpha.id), "", Theme::default()))
            .expect("draw wide");
        let screen = terminal.backend().to_string();
        let lines: Vec<_> = screen.lines().collect();

        assert!(lines[0].contains("Resume"));
        assert!(lines[0].contains("◀"));
        assert!(lines[0].contains("▶"));
        assert!(lines[0].contains("· 3w"));
        assert!(lines[1].trim_start_matches('"').starts_with("Search: "));
        assert!(lines[1].contains("filter sessions or windows…"));
        assert!(screen.contains("* alpha"));
        assert!(screen.contains("2w ● attached"));
        assert!(screen.contains("1w ○ detached"));
        assert!(screen.contains("› 0* editor"));
        assert!(screen.contains("  1 logs · 3p"));
        assert!(screen.contains("  0* api · 2p"));

        let buf = terminal.backend().buffer();
        let find_cell = |y: u16, sym: &str| {
            (0..72)
                .map(|x| &buf[(x, y)])
                .find(|cell| cell.symbol() == sym)
                .expect("cell")
        };
        assert_eq!(find_cell(1, "f").fg, Color::DarkGray);
        assert!((0..72).map(|x| &buf[(x, 2)]).any(|c| c.symbol() == "a"
            && c.fg == Color::Cyan
            && c.modifier.contains(Modifier::BOLD)));
        assert_eq!(find_cell(3, "●").fg, Color::Green);
        assert_eq!(find_cell(3, "○").fg, Color::DarkGray);
        assert!(
            (0..72)
                .map(|x| &buf[(x, 4)])
                .any(|c| c.symbol() == "*" && c.fg == Color::Cyan)
        );
        assert!(
            (0..72)
                .map(|x| &buf[(x, 5)])
                .any(|c| c.symbol() == "·" && c.fg == Color::DarkGray)
        );

        picker.paste("beta", &mut matcher);
        terminal
            .draw(|frame| picker.render(frame, frame.area(), Some(&alpha.id), "", Theme::default()))
            .expect("draw filtered");
        let filtered = terminal.backend().to_string();
        assert!(!filtered.contains("filter sessions or windows"));
        assert!(filtered.contains("· 1w"));
    }

    #[test]
    fn narrow_render_omits_placeholder_title_hint_and_truncating_pane_badges() {
        let session = make_session("$0", "work", 2, 1);
        let windows = vec![
            make_window("@0", &session, 0, "app", true, 4),
            make_window("@1", &session, 1, "very-long-window-name-here", false, 5),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![session], windows, &mut matcher);

        let mut ultra_narrow = Terminal::new(TestBackend::new(13, 7)).expect("13w screen");
        ultra_narrow
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 13w");
        let screen_13 = ultra_narrow.backend().to_string();
        assert!(screen_13.contains("Resume"));
        assert!(!screen_13.contains("◀"));
        assert!(
            screen_13
                .lines()
                .nth(1)
                .is_some_and(|line| line.trim_start_matches('"').starts_with("? "))
        );
        assert!(!screen_13.contains("filter sessions"));

        let mut col_24 = Terminal::new(TestBackend::new(24, 8)).expect("24w screen");
        col_24
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 24w");
        let screen_24 = col_24.backend().to_string();
        assert!(!screen_24.contains("· 4p"));
        assert!(!screen_24.contains("· 5p"));

        let mut col_28 = Terminal::new(TestBackend::new(28, 8)).expect("28w screen");
        col_28
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 28w");
        let screen_28 = col_28.backend().to_string();
        assert!(screen_28.contains("› 0* app · 4p"));
        assert!(!screen_28.contains("· 5p"));

        let mut below_threshold = Terminal::new(TestBackend::new(41, 7)).expect("41w screen");
        below_threshold
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 41w");
        assert!(
            !below_threshold
                .backend()
                .to_string()
                .contains("filter sessions")
        );

        let mut at_threshold = Terminal::new(TestBackend::new(42, 7)).expect("42w screen");
        at_threshold
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 42w");
        assert!(
            at_threshold
                .backend()
                .to_string()
                .contains("filter sessions or windows…")
        );
    }

    #[test]
    fn ascii_and_no_color_render_is_pure_ascii_with_reset_colors() {
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 3),
            make_window("@2", &beta, 0, "api", true, 2),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![alpha.clone(), beta], windows, &mut matcher);

        let theme = Theme {
            ascii: true,
            no_color: true,
            ..Theme::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(72, 12)).expect("ascii screen");
        terminal
            .draw(|frame| picker.render(frame, frame.area(), Some(&alpha.id), "", theme))
            .expect("draw ascii");
        let screen = terminal.backend().to_string();

        assert!(
            screen.is_ascii(),
            "ASCII theme output must be pure ASCII:\n{screen}"
        );
        assert!(screen.contains("Resume"));
        assert!(screen.contains("Search: "));
        assert!(screen.contains("filter sessions or windows..."));
        assert!(screen.contains("2w attached"));
        assert!(screen.contains("1w detached"));
        assert!(screen.contains("> 0* editor"));
        assert!(screen.contains("| 3p"));
        assert!(screen.contains("| 2p"));
        assert!(screen.contains("Arrows | Enter Open | Esc Back"));
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::REVERSED))
        );
    }

    #[test]
    fn compact_heights_render_cleanly() {
        let session = make_session("$0", "work", 1, 0);
        let windows = vec![make_window("@0", &session, 0, "shell", true, 2)];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![session], windows, &mut matcher);

        let mut single_line = Terminal::new(TestBackend::new(40, 1)).expect("1h screen");
        single_line
            .draw(|frame| picker.render(frame, frame.area(), None, "", Theme::default()))
            .expect("draw 1h");
        assert!(
            single_line
                .backend()
                .to_string()
                .contains("› work:shell · 2p")
        );
    }

    #[test]
    fn selected_target_info_tracks_navigation_and_empty_state() {
        let mut picker = ResumePicker::default();
        assert_eq!(picker.selected_target_info(), None);

        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 3, "workspace", false, 2),
            make_window("@2", &beta, 1, "api", true, 1),
        ];
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![alpha.clone(), beta.clone()], windows, &mut matcher);

        assert_eq!(
            picker.selected_target_info(),
            Some((
                Target::Window(alpha.id.clone(), WindowId::new("@0").expect("id")),
                "alpha:0 editor".into()
            ))
        );

        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        picker.handle_key(key(KeyCode::Down), &mut matcher);
        assert_eq!(
            picker.selected_target_info(),
            Some((
                Target::Window(alpha.id, WindowId::new("@1").expect("id")),
                "alpha:3 workspace".into()
            ))
        );

        picker.handle_key(key(KeyCode::Right), &mut matcher);
        assert_eq!(
            picker.selected_target_info(),
            Some((
                Target::Window(beta.id, WindowId::new("@2").expect("id")),
                "beta:1 api".into()
            ))
        );

        picker.paste("no-such-match", &mut matcher);
        assert_eq!(picker.selected_target_info(), None);
    }

    #[test]
    fn render_with_preview_shows_card_on_wide_tall_screens_and_hides_otherwise() {
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 3),
            make_window("@2", &beta, 0, "api", true, 2),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![alpha.clone(), beta], windows, &mut matcher);

        let sample = vec![
            "$ cargo test".to_owned(),
            "running 5 tests".to_owned(),
            "test result: ok.".to_owned(),
            "fourth-line-omitted".to_owned(),
        ];

        // Exact minimum threshold (64x14) renders preview card with up to 3 lines.
        let mut min_ok = Terminal::new(TestBackend::new(64, 14)).expect("64x14");
        min_ok
            .draw(|frame| {
                picker.render_with_preview(
                    frame,
                    frame.area(),
                    Some(&alpha.id),
                    "",
                    Some(&sample),
                    Theme::default(),
                )
            })
            .expect("draw 64x14");
        let screen_min = min_ok.backend().to_string();
        assert!(screen_min.contains("Window Preview · alpha:editor"));
        assert!(screen_min.contains("$ cargo test"));
        assert!(screen_min.contains("running 5 tests"));
        assert!(screen_min.contains("test result: ok."));
        assert!(!screen_min.contains("fourth-line-omitted"));
        assert!(screen_min.contains("2w ● attached"));
        assert!(screen_min.contains("› 0* editor"));

        // Empty preview shows "(empty shell pane)" placeholder.
        let empty: Vec<String> = Vec::new();
        let mut wide = Terminal::new(TestBackend::new(72, 16)).expect("72x16");
        wide.draw(|frame| {
            picker.render_with_preview(
                frame,
                frame.area(),
                Some(&alpha.id),
                "",
                Some(&empty),
                Theme::default(),
            )
        })
        .expect("draw empty preview");
        assert!(wide.backend().to_string().contains("  (empty shell pane)"));

        // ASCII + no_color preview stays pure ASCII with Reset colors.
        let ascii_theme = Theme {
            ascii: true,
            no_color: true,
            ..Theme::default()
        };
        wide.draw(|frame| {
            picker.render_with_preview(
                frame,
                frame.area(),
                Some(&alpha.id),
                "",
                Some(&sample),
                ascii_theme,
            )
        })
        .expect("draw ascii preview");
        let ascii_screen = wide.backend().to_string();
        assert!(ascii_screen.is_ascii(), "{ascii_screen}");
        assert!(ascii_screen.contains("Window Preview | alpha:editor"));
        assert!(
            wide.backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );

        // Narrow (63x14) and short (64x13) hide the preview gracefully.
        let mut too_narrow = Terminal::new(TestBackend::new(63, 14)).expect("63x14");
        too_narrow
            .draw(|frame| {
                picker.render_with_preview(
                    frame,
                    frame.area(),
                    Some(&alpha.id),
                    "",
                    Some(&sample),
                    Theme::default(),
                )
            })
            .expect("draw 63x14");
        assert!(!too_narrow.backend().to_string().contains("Window Preview"));

        let mut too_short = Terminal::new(TestBackend::new(64, 13)).expect("64x13");
        too_short
            .draw(|frame| {
                picker.render_with_preview(
                    frame,
                    frame.area(),
                    Some(&alpha.id),
                    "",
                    Some(&sample),
                    Theme::default(),
                )
            })
            .expect("draw 64x13");
        assert!(!too_short.backend().to_string().contains("Window Preview"));
    }
}
