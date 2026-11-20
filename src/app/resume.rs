use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    fuzzy,
    tmux::{Session, SessionId, Target, Window},
    ui::{TextInput, Theme},
};

#[derive(Debug, Clone)]
struct Column {
    session: Session,
    windows: Vec<Window>,
}

struct VisibleColumn<'a> {
    session: &'a Session,
    windows: Vec<&'a Window>,
}

impl VisibleColumn<'_> {
    fn active_row(&self) -> usize {
        self.windows
            .iter()
            .position(|window| window.active)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResumePicker {
    columns: Vec<Column>,
    query: TextInput,
    selected_column: usize,
    selected_row: usize,
    first_column: usize,
    first_row: usize,
}

impl ResumePicker {
    pub(super) fn replace(&mut self, sessions: Vec<Session>, windows: Vec<Window>) {
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
        self.reset_selection();
    }

    fn visible(&self) -> Vec<VisibleColumn<'_>> {
        let query = self.query.value().trim();
        let mut columns = Vec::new();
        for column in &self.columns {
            let mut score = if query.is_empty() {
                Some(0)
            } else {
                fuzzy::score(query, &column.session.name)
            };
            let windows = if score.is_some() {
                column.windows.iter().collect()
            } else {
                column
                    .windows
                    .iter()
                    .filter(|window| {
                        let Some(matched) = fuzzy::score(query, &window.name) else {
                            return false;
                        };
                        score = Some(score.map_or(matched, |best| best.min(matched)));
                        true
                    })
                    .collect()
            };
            if let Some(score) = score {
                columns.push((
                    score,
                    VisibleColumn {
                        session: &column.session,
                        windows,
                    },
                ));
            }
        }
        columns.sort_by_key(|(score, _)| *score);
        columns.into_iter().map(|(_, column)| column).collect()
    }

    fn reset_selection(&mut self) {
        let row = self.visible().first().map_or(0, VisibleColumn::active_row);
        self.selected_column = 0;
        self.selected_row = row;
        self.first_column = 0;
        self.first_row = 0;
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Option<Target> {
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Left | KeyCode::Right if plain => {
                let columns = self.visible();
                let next = if key.code == KeyCode::Left {
                    self.selected_column.saturating_sub(1)
                } else {
                    (self.selected_column + 1).min(columns.len().saturating_sub(1))
                };
                if next != self.selected_column {
                    let row = columns[next].active_row();
                    self.selected_column = next;
                    self.selected_row = row;
                    self.first_row = 0;
                }
            }
            KeyCode::Up if plain => self.selected_row = self.selected_row.saturating_sub(1),
            KeyCode::Down if plain => {
                let count = self
                    .visible()
                    .get(self.selected_column)
                    .map_or(0, |column| column.windows.len());
                self.selected_row = (self.selected_row + 1).min(count.saturating_sub(1));
            }
            KeyCode::Enter => {
                let columns = self.visible();
                let column = columns.get(self.selected_column)?;
                let window = column.windows.get(self.selected_row)?;
                return Some(Target::Window(column.session.id.clone(), window.id.clone()));
            }
            _ => {
                let before = self.query.value().to_owned();
                if self.query.handle_key(key) && self.query.value() != before {
                    self.reset_selection();
                }
            }
        }
        None
    }

    pub(super) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        current_session: Option<&SessionId>,
        status: &str,
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
        let columns = self.visible();
        let shown = usize::from((body.width / 20).max(1)).min(columns.len());
        let heading = if shown < columns.len() {
            format!("Resume  [{}/{}]", self.selected_column + 1, columns.len())
        } else {
            "Resume".into()
        };
        frame.render_widget(Paragraph::new(heading).style(accent), title);
        let prompt = if search.width < 14 { "? " } else { "Search: " };
        let mut query = self.query.rendered_width(
            true,
            theme,
            search.width.saturating_sub(prompt.len() as u16),
        );
        query.spans.insert(0, Span::styled(prompt, muted));
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
        for ((index, column), area) in columns
            .iter()
            .enumerate()
            .skip(first_column)
            .zip(areas.iter())
        {
            let focused = index == self.selected_column;
            let current = current_session == Some(&column.session.id);
            let name = format!("{}{}", if current { "* " } else { "" }, column.session.name);
            let mut lines = Vec::new();
            if header_rows > 0 {
                lines.push(Line::styled(
                    name,
                    if focused {
                        accent
                    } else {
                        theme.style(theme.text).add_modifier(Modifier::BOLD)
                    },
                ));
            }
            if header_rows == 2 {
                let status = if column.session.is_attached() {
                    "attached"
                } else {
                    "detached"
                };
                lines.push(Line::styled(
                    format!("{}w {status}", column.session.windows),
                    muted,
                ));
            }
            let offset = if focused { first_row } else { 0 };
            for (row, window) in column
                .windows
                .iter()
                .enumerate()
                .skip(offset)
                .take(row_count)
            {
                let label = if header_rows == 0 {
                    format!("{}:{}", column.session.name, window.name)
                } else {
                    format!(
                        "{}{} {}",
                        window.index,
                        if window.active { "*" } else { "" },
                        window.name
                    )
                };
                lines.push(window_line(
                    label,
                    focused && row == self.selected_row,
                    theme,
                ));
            }
            frame.render_widget(Paragraph::new(lines), *area);
        }
        self.first_column = first_column;
        self.first_row = first_row;
    }
}

fn window_line(label: String, selected: bool, theme: Theme) -> Line<'static> {
    if selected {
        Line::styled(
            format!("{} {label}", theme.marker()),
            theme.selected_style(),
        )
    } else {
        Line::raw(format!("  {label}"))
    }
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
                    name: if column == 2 && index == 5 {
                        "last-log".into()
                    } else {
                        format!("shell-{index}")
                    },
                    active: index == 0,
                    panes: 1,
                })
            })
            .collect();
        let mut picker = ResumePicker::default();
        picker.replace(sessions, windows);
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
            picker.handle_key(key(code));
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
            .handle_key(key(KeyCode::Enter))
            .expect("window target");
        assert_eq!(chosen.to_string(), "$2:@25");

        for ch in "last-log".chars() {
            picker.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(picker.visible().len(), 1);
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), Some(chosen));
        picker.query.set_value("build");
        picker.reset_selection();
        assert_eq!(picker.visible()[0].windows.len(), 6);
        assert_eq!(
            picker
                .handle_key(key(KeyCode::Enter))
                .expect("active window target")
                .to_string(),
            "$1:@10"
        );
    }
}
