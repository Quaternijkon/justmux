use std::collections::{HashMap, HashSet};

#[cfg(test)]
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier},
    text::{Line, Span},
    widgets::Paragraph,
};
#[cfg(test)]
use ratatui::{
    symbols,
    widgets::{Block, BorderType, Borders},
};
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
use crate::tmux::Target;
use crate::{
    fuzzy::{FuzzyMatch, FuzzyMatcher},
    tmux::{Session, SessionId, Window, WindowId},
    ui::{TextInput, Theme, highlighted_spans, styled_key_hint_line},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RowFocus {
    SessionHeader,
    #[default]
    Window,
    NewWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatrixSelection<'a> {
    Session(&'a Session),
    Window(&'a Session, &'a Window),
    NewWindow(&'a Session),
    NewSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MatrixHitTarget {
    Session(usize),
    Window { col: usize, row: usize },
    NewWindow(usize),
    NewSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MatrixHit {
    pub rect: Rect,
    pub target: MatrixHitTarget,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResumePicker {
    columns: Vec<Column>,
    visible: Vec<VisibleColumn>,
    query: TextInput,
    selected_column: usize,
    selected_row: usize,
    row_focus: RowFocus,
    first_column: usize,
    first_row: usize,
    create_actions: bool,
}

impl ResumePicker {
    pub(super) fn with_create_actions() -> Self {
        Self {
            create_actions: true,
            ..Self::default()
        }
    }

    fn show_create(&self) -> bool {
        self.create_actions && self.query.value().trim().is_empty()
    }

    fn total_columns(&self) -> usize {
        self.visible.len() + usize::from(self.show_create())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub(super) fn has_visible_matches(&self) -> bool {
        !self.visible.is_empty()
    }

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

    pub(super) fn sync(
        &mut self,
        sessions: Vec<Session>,
        windows: Vec<Window>,
        filter: &str,
        matcher: &mut FuzzyMatcher,
    ) {
        enum SavedTarget {
            Session(SessionId),
            Window(SessionId, WindowId),
            NewWindow(SessionId),
            NewSession,
        }

        let prev_target = match self.selection() {
            Some(MatrixSelection::Session(session)) => {
                Some(SavedTarget::Session(session.id.clone()))
            }
            Some(MatrixSelection::Window(session, window)) => {
                Some(SavedTarget::Window(session.id.clone(), window.id.clone()))
            }
            Some(MatrixSelection::NewWindow(session)) => {
                Some(SavedTarget::NewWindow(session.id.clone()))
            }
            Some(MatrixSelection::NewSession) if !self.columns.is_empty() => {
                Some(SavedTarget::NewSession)
            }
            Some(MatrixSelection::NewSession) | None => None,
        };
        let prev_first_column = self.first_column;
        let prev_first_row = self.first_row;
        let same_filter = self.query.value() == filter;
        self.query.set_value(filter);
        self.replace(sessions, windows, matcher);
        if same_filter && let Some(saved) = prev_target {
            match saved {
                SavedTarget::NewSession => {
                    if self.show_create() {
                        self.selected_column = self.visible.len();
                        self.selected_row = 0;
                        self.row_focus = RowFocus::Window;
                        self.first_column = prev_first_column;
                    }
                }
                SavedTarget::Session(prev_session_id) => {
                    if let Some(col_idx) = self.find_visible_session(&prev_session_id) {
                        self.selected_column = col_idx;
                        self.row_focus = RowFocus::SessionHeader;
                        self.selected_row = 0;
                        self.first_column = prev_first_column;
                        self.first_row = 0;
                    }
                }
                SavedTarget::NewWindow(prev_session_id) => {
                    if let Some(col_idx) = self.find_visible_session(&prev_session_id) {
                        self.selected_column = col_idx;
                        self.first_column = prev_first_column;
                        let visible_col = &self.visible[col_idx];
                        if self.show_create() {
                            self.row_focus = RowFocus::NewWindow;
                            self.selected_row = visible_col.windows.len();
                            self.first_row = prev_first_row;
                        } else {
                            self.row_focus = RowFocus::Window;
                            self.selected_row = visible_col.initial_row;
                            self.first_row = 0;
                        }
                    }
                }
                SavedTarget::Window(prev_session_id, prev_window_id) => {
                    if let Some(col_idx) = self.find_visible_session(&prev_session_id) {
                        self.selected_column = col_idx;
                        self.first_column = prev_first_column;
                        let visible_col = &self.visible[col_idx];
                        self.row_focus = RowFocus::Window;
                        if let Some(column) = self.columns.get(visible_col.index)
                            && let Some(row_idx) = visible_col
                                .windows
                                .iter()
                                .position(|win| column.windows[win.index].id == prev_window_id)
                        {
                            self.selected_row = row_idx;
                            self.first_row = prev_first_row;
                        } else {
                            self.selected_row = visible_col.initial_row;
                            self.first_row = 0;
                        }
                    }
                }
            }
        }
    }

    fn find_visible_session(&self, session_id: &SessionId) -> Option<usize> {
        self.visible.iter().position(|col| {
            self.columns
                .get(col.index)
                .is_some_and(|c| &c.session.id == session_id)
        })
    }

    #[cfg(test)]
    pub(super) fn set_filter(&mut self, filter: &str, matcher: &mut FuzzyMatcher) -> bool {
        if self.query.value() == filter {
            return false;
        }
        self.query.set_value(filter);
        self.refresh_matches(matcher);
        true
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
        self.row_focus = if self
            .visible
            .first()
            .is_some_and(|column| column.windows.is_empty())
            && self.show_create()
        {
            RowFocus::NewWindow
        } else {
            RowFocus::Window
        };
        self.first_column = 0;
        self.first_row = 0;
    }

    #[cfg(test)]
    pub(super) fn paste(&mut self, text: &str, matcher: &mut FuzzyMatcher) {
        self.query.insert_str(text);
        self.refresh_matches(matcher);
    }

    #[cfg(test)]
    pub(super) fn handle_key(
        &mut self,
        key: KeyEvent,
        matcher: &mut FuzzyMatcher,
    ) -> Option<Target> {
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Left | KeyCode::Right if plain => {
                self.move_column(if key.code == KeyCode::Left { -1 } else { 1 });
            }
            KeyCode::Up if plain => {
                self.move_row(-1);
            }
            KeyCode::Down if plain => {
                self.move_row(1);
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

    pub(super) fn move_column(&mut self, delta: isize) -> bool {
        let total = self.total_columns();
        if total <= 1 || delta == 0 {
            return false;
        }
        let next = super::wrap_step(self.selected_column, total, delta);
        self.select_column_index(next)
    }

    #[cfg(test)]
    pub(super) fn cycle_column(&mut self) -> bool {
        let total = self.total_columns();
        if total <= 1 {
            return false;
        }
        let next = (self.selected_column + 1) % total;
        self.select_column_index(next)
    }

    fn select_column_index(&mut self, next: usize) -> bool {
        if next == self.selected_column {
            return false;
        }
        let keep_header = self.create_actions
            && self.selected_column < self.visible.len()
            && self.row_focus == RowFocus::SessionHeader;
        self.selected_column = next;
        self.first_row = 0;
        if let Some(visible_col) = self.visible.get(next) {
            if keep_header {
                self.row_focus = RowFocus::SessionHeader;
                self.selected_row = 0;
            } else if visible_col.windows.is_empty() && self.show_create() {
                self.row_focus = RowFocus::NewWindow;
                self.selected_row = 0;
            } else {
                self.row_focus = RowFocus::Window;
                self.selected_row = visible_col.initial_row;
            }
        } else {
            self.row_focus = RowFocus::Window;
            self.selected_row = 0;
        }
        true
    }

    pub(super) fn move_row(&mut self, delta: isize) -> bool {
        let Some(column) = self.visible.get(self.selected_column) else {
            return false;
        };
        let win_count = column.windows.len();
        if !self.create_actions {
            if win_count <= 1 || delta == 0 {
                return false;
            }
            let next = super::wrap_step(self.selected_row, win_count, delta);
            if next == self.selected_row {
                return false;
            }
            self.selected_row = next;
            self.row_focus = RowFocus::Window;
            return true;
        }
        if delta == 0 {
            return false;
        }
        let show_create = self.show_create();
        let total_stops = 1 + win_count + usize::from(show_create);
        if total_stops <= 1 {
            return false;
        }
        let current_stop = match self.row_focus {
            RowFocus::SessionHeader => 0,
            RowFocus::Window => 1 + self.selected_row.min(win_count.saturating_sub(1)),
            RowFocus::NewWindow => 1 + win_count,
        };
        let next_stop = super::wrap_step(current_stop, total_stops, delta);
        if next_stop == current_stop {
            return false;
        }
        if next_stop == 0 {
            self.row_focus = RowFocus::SessionHeader;
            self.selected_row = 0;
        } else if next_stop <= win_count {
            self.row_focus = RowFocus::Window;
            self.selected_row = next_stop - 1;
        } else {
            self.row_focus = RowFocus::NewWindow;
            self.selected_row = win_count;
        }
        true
    }

    pub(super) fn select_session(&mut self, session_id: &SessionId) -> bool {
        let Some(col_idx) = self.find_visible_session(session_id) else {
            return false;
        };
        let visible_col = &self.visible[col_idx];
        if col_idx != self.selected_column {
            self.first_row = 0;
        }
        self.selected_column = col_idx;
        self.selected_row = visible_col.initial_row;
        self.row_focus = if visible_col.windows.is_empty() && self.show_create() {
            RowFocus::NewWindow
        } else {
            RowFocus::Window
        };
        true
    }

    pub(super) fn select_visible_cell(&mut self, col: usize, row: Option<usize>) -> bool {
        let Some(visible_col) = self.visible.get(col) else {
            return false;
        };
        match row {
            None => {
                if self.selected_column == col {
                    return true;
                }
                self.selected_column = col;
                self.selected_row = visible_col.initial_row;
                self.row_focus = RowFocus::Window;
                self.first_row = 0;
                false
            }
            Some(row) => {
                if row >= visible_col.windows.len() {
                    return false;
                }
                if self.selected_column == col
                    && self.row_focus == RowFocus::Window
                    && self.selected_row == row
                {
                    return true;
                }
                if self.selected_column != col {
                    self.first_row = 0;
                }
                self.selected_column = col;
                self.selected_row = row;
                self.row_focus = RowFocus::Window;
                false
            }
        }
    }

    pub(super) fn resolve_hit(&self, target: &MatrixHitTarget) -> Option<MatrixSelection<'_>> {
        match target {
            MatrixHitTarget::Session(col) => {
                let visible_col = self.visible.get(*col)?;
                let column = self.columns.get(visible_col.index)?;
                Some(MatrixSelection::Session(&column.session))
            }
            MatrixHitTarget::Window { col, row } => {
                let visible_col = self.visible.get(*col)?;
                let column = self.columns.get(visible_col.index)?;
                let vis_win = visible_col.windows.get(*row)?;
                let window = column.windows.get(vis_win.index)?;
                Some(MatrixSelection::Window(&column.session, window))
            }
            MatrixHitTarget::NewWindow(col) => {
                let visible_col = self.visible.get(*col)?;
                let column = self.columns.get(visible_col.index)?;
                Some(MatrixSelection::NewWindow(&column.session))
            }
            MatrixHitTarget::NewSession => Some(MatrixSelection::NewSession),
        }
    }

    pub(super) fn select_hit(&mut self, target: &MatrixHitTarget) -> bool {
        match target {
            MatrixHitTarget::Session(col) => {
                let Some(visible_col) = self.visible.get(*col) else {
                    return false;
                };
                if self.create_actions {
                    if self.selected_column == *col && self.row_focus == RowFocus::SessionHeader {
                        return true;
                    }
                    if self.selected_column != *col {
                        self.first_row = 0;
                    }
                    self.selected_column = *col;
                    self.row_focus = RowFocus::SessionHeader;
                    self.selected_row = visible_col.initial_row;
                    false
                } else {
                    self.select_visible_cell(*col, None)
                }
            }
            MatrixHitTarget::Window { col, row } => self.select_visible_cell(*col, Some(*row)),
            MatrixHitTarget::NewWindow(col) => {
                if !self.show_create() {
                    return false;
                }
                let Some(visible_col) = self.visible.get(*col) else {
                    return false;
                };
                if self.selected_column != *col {
                    self.first_row = 0;
                }
                self.selected_column = *col;
                self.row_focus = RowFocus::NewWindow;
                self.selected_row = visible_col.windows.len();
                true
            }
            MatrixHitTarget::NewSession => {
                if !self.show_create() {
                    return false;
                }
                self.selected_column = self.visible.len();
                self.row_focus = RowFocus::Window;
                self.selected_row = 0;
                self.first_row = 0;
                true
            }
        }
    }

    pub(super) fn focus_session_header(&mut self) {
        if self.selected_column < self.visible.len() {
            self.row_focus = RowFocus::SessionHeader;
            self.first_row = 0;
        }
    }

    pub(super) fn focus_window_row(&mut self) {
        let Some(visible_col) = self.visible.get(self.selected_column) else {
            return;
        };
        if visible_col.windows.is_empty() {
            self.row_focus = if self.show_create() {
                RowFocus::NewWindow
            } else {
                RowFocus::Window
            };
            self.selected_row = 0;
            return;
        }
        if self.row_focus != RowFocus::Window {
            self.row_focus = RowFocus::Window;
            if self.selected_row >= visible_col.windows.len() {
                self.selected_row = visible_col.initial_row;
            }
        }
    }

    pub(super) fn selection(&self) -> Option<MatrixSelection<'_>> {
        if self.show_create() && self.selected_column == self.visible.len() {
            return Some(MatrixSelection::NewSession);
        }
        let visible = self.visible.get(self.selected_column)?;
        let column = self.columns.get(visible.index)?;
        match self.row_focus {
            RowFocus::SessionHeader => Some(MatrixSelection::Session(&column.session)),
            RowFocus::NewWindow => self
                .show_create()
                .then_some(MatrixSelection::NewWindow(&column.session)),
            RowFocus::Window => {
                let window = column
                    .windows
                    .get(visible.windows.get(self.selected_row)?.index)?;
                Some(MatrixSelection::Window(&column.session, window))
            }
        }
    }

    pub(super) fn is_session_selected(&self) -> bool {
        matches!(self.selection(), Some(MatrixSelection::Session(_)))
    }

    pub(super) fn preview_window(&self) -> Option<(&Session, &Window)> {
        let visible = self.visible.get(self.selected_column)?;
        let column = self.columns.get(visible.index)?;
        if self.row_focus == RowFocus::Window
            && let Some(win_ref) = visible.windows.get(self.selected_row)
            && let Some(window) = column.windows.get(win_ref.index)
        {
            return Some((&column.session, window));
        }
        let fallback = visible
            .windows
            .iter()
            .find(|win| column.windows.get(win.index).is_some_and(|w| w.active))
            .or_else(|| visible.windows.first())?;
        let window = column.windows.get(fallback.index)?;
        Some((&column.session, window))
    }

    #[cfg(test)]
    fn selected_entry(&self) -> Option<(&Column, &Window)> {
        if self.row_focus != RowFocus::Window {
            return None;
        }
        let visible = self.visible.get(self.selected_column)?;
        let column = self.columns.get(visible.index)?;
        let window = column
            .windows
            .get(visible.windows.get(self.selected_row)?.index)?;
        Some((column, window))
    }

    pub(super) fn selected_session(&self) -> Option<&Session> {
        let visible = self.visible.get(self.selected_column)?;
        let column = self.columns.get(visible.index)?;
        Some(&column.session)
    }

    #[cfg(test)]
    pub(super) fn selected_window(&self) -> Option<(&Session, &Window)> {
        let (column, window) = self.selected_entry()?;
        Some((&column.session, window))
    }

    #[cfg(test)]
    pub(super) fn selected_target_info(&self) -> Option<(Target, String)> {
        let (session, window) = self.selected_window()?;
        Some((
            Target::Window(session.id.clone(), window.id.clone()),
            format!("{}:{} {}", session.name, window.index, window.name),
        ))
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
        let column_count = self.visible.len();
        let shown = usize::from((body.width / 20).max(1)).min(column_count);
        let heading = if shown < column_count {
            format!(
                "{}  [{}/{}]",
                theme.tr("Resume", "恢复"),
                self.selected_column + 1,
                column_count
            )
        } else {
            theme.tr("Resume", "恢复").into()
        };
        let mut title_spans = vec![Span::styled(heading, accent)];
        if area.width >= 44 && !theme.ascii && column_count > 0 {
            let total_windows: usize = self.visible.iter().map(|column| column.windows.len()).sum();
            let left_style = if self.selected_column > 0 {
                theme.style(theme.accent)
            } else {
                muted
            };
            let right_style = if self.selected_column + 1 < column_count {
                theme.style(theme.accent)
            } else {
                muted
            };
            title_spans.push(Span::styled("  ", muted));
            title_spans.push(Span::styled("◀", left_style));
            title_spans.push(Span::styled(" ", muted));
            title_spans.push(Span::styled("▶", right_style));
            title_spans.push(Span::styled(
                format!(
                    " · {total_windows}{}",
                    if theme.language.is_zh() {
                        " 窗口"
                    } else {
                        "w"
                    }
                ),
                muted,
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(title_spans)), title);
        let prompt = if search.width < 14 {
            "? "
        } else {
            theme.tr("Search: ", "搜索: ")
        };
        let mut query = self.query.rendered_width(
            true,
            theme,
            search.width.saturating_sub(prompt.width() as u16),
        );
        query.spans.insert(0, Span::styled(prompt, muted));
        if self.query.value().is_empty() && search.width >= 42 {
            let placeholder =
                theme.adapt(theme.tr("filter sessions or windows…", "过滤会话或窗口…"));
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
            theme.tr(
                "←→ Session · ↑↓ Window · Enter Open · Esc Back",
                "←→ 会话 · ↑↓ 窗口 · Enter 打开 · Esc 返回",
            )
        };
        if status.is_empty() {
            frame.render_widget(
                Paragraph::new(styled_key_hint_line(hint, muted, theme)),
                footer,
            );
        } else {
            frame.render_widget(
                Paragraph::new(status).style(theme.style(theme.error)),
                footer,
            );
        }

        let (grid_body, preview_area) = match preview {
            Some(lines) if area.height >= 14 && area.width >= 64 && column_count > 0 => {
                let [top, bottom] =
                    Layout::vertical([Constraint::Min(6), Constraint::Length(5)]).areas(body);
                (top, Some((bottom, lines)))
            }
            _ => (body, None),
        };
        let _ = self.render_matrix_body(frame, grid_body, current_session, false, theme);
        if let Some((card_area, preview_lines)) = preview_area {
            let target_label = self
                .selected_entry()
                .map_or_else(String::new, |(col, win)| {
                    format!("{}:{}", col.session.name, win.name)
                });
            render_preview_card(frame, card_area, &target_label, preview_lines, theme);
        }
    }

    #[cfg(test)]
    pub(super) fn render_matrix_body(
        &mut self,
        frame: &mut Frame<'_>,
        body: Rect,
        current_session: Option<&SessionId>,
        highlight_session_header: bool,
        theme: Theme,
    ) -> Vec<MatrixHit> {
        let empty_dead = HashSet::new();
        self.render_matrix_body_with_dead_windows(
            frame,
            body,
            current_session,
            highlight_session_header,
            &empty_dead,
            theme,
        )
    }

    pub(super) fn render_matrix_body_with_dead_windows(
        &mut self,
        frame: &mut Frame<'_>,
        body: Rect,
        current_session: Option<&SessionId>,
        highlight_session_header: bool,
        dead_windows: &HashSet<WindowId>,
        theme: Theme,
    ) -> Vec<MatrixHit> {
        let accent = theme.style(theme.accent).add_modifier(Modifier::BOLD);
        let muted = theme.style(theme.muted);
        if self.columns.is_empty() {
            if self.create_actions {
                let create_style = theme
                    .style(theme.success)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD);
                let new_session_label = theme.tr("+ New session", "+ 新建会话");
                let no_sessions_label = theme.tr("No tmux sessions", "暂无 tmux 会话");
                let lines = if body.height <= 1 {
                    vec![Line::from(vec![
                        Span::styled(
                            format!("{} {new_session_label}", theme.marker()),
                            create_style,
                        ),
                        Span::styled(format!("  {no_sessions_label}"), muted),
                    ])]
                } else {
                    let mut out = vec![Line::styled(
                        format!("{} {new_session_label}", theme.marker()),
                        create_style,
                    )];
                    if body.height >= 3 {
                        out.push(Line::default());
                    }
                    out.push(styled_key_hint_line(
                        theme.tr(
                            "No tmux sessions — press Enter to create",
                            "暂无 tmux 会话 — 按 Enter 新建",
                        ),
                        muted,
                        theme,
                    ));
                    out
                };
                frame.render_widget(Paragraph::new(lines), body);
                if body.width > 0 && body.height > 0 {
                    return vec![MatrixHit {
                        rect: body,
                        target: MatrixHitTarget::NewSession,
                    }];
                }
                return Vec::new();
            }
            let mut lines = vec![Line::styled(
                theme.tr("No tmux sessions", "暂无 tmux 会话"),
                muted,
            )];
            if body.height >= 3 {
                lines.push(Line::default());
                lines.push(styled_key_hint_line(
                    theme.tr(
                        "Press Enter or type /new <name> to create a session",
                        "按 Enter 或输入 /new <名称> 新建会话",
                    ),
                    muted,
                    theme,
                ));
            }
            frame.render_widget(Paragraph::new(lines), body);
            return Vec::new();
        }
        if self.visible.is_empty() {
            frame.render_widget(
                Paragraph::new(theme.tr("No matching sessions or windows", "没有匹配的会话或窗口"))
                    .style(muted),
                body,
            );
            return Vec::new();
        }
        let show_create = self.show_create();
        let total_cols = self.total_columns();
        let shown = usize::from((body.width / 20).max(1)).min(total_cols);
        let first_column = scroll_to(self.first_column, self.selected_column, shown, total_cols);
        let header_rows = match body.height {
            0 | 1 => 0,
            2 | 3 => 1,
            _ => 2,
        };
        let row_count = usize::from(body.height.saturating_sub(header_rows));
        let (selected_total_rows, focus_row) = match self.visible.get(self.selected_column) {
            Some(column) => {
                let win_count = column.windows.len();
                let total = win_count + usize::from(show_create);
                let focus = match self.row_focus {
                    RowFocus::SessionHeader => 0,
                    RowFocus::Window => self.selected_row.min(win_count.saturating_sub(1)),
                    RowFocus::NewWindow => win_count,
                };
                (total, focus)
            }
            None => (1, 0),
        };
        let first_row = scroll_to(
            self.first_row,
            focus_row,
            row_count.max(1),
            selected_total_rows,
        );
        let areas = Layout::horizontal(vec![Constraint::Fill(1); shown])
            .spacing(1)
            .split(body);
        let mut hits = Vec::new();
        for (col_index, area) in (first_column..total_cols).zip(areas.iter()) {
            let Some(visible) = self.visible.get(col_index) else {
                let focused = col_index == self.selected_column;
                let mut lines = Vec::new();
                if header_rows > 0 {
                    let header_style = if focused {
                        theme.style(theme.success).add_modifier(Modifier::BOLD)
                    } else {
                        theme.style(theme.success)
                    };
                    lines.push(Line::styled(
                        theme.tr("+ New session", "+ 新建会话"),
                        header_style,
                    ));
                }
                if header_rows == 2 {
                    lines.push(Line::styled(theme.tr("workspace", "工作区"), muted));
                }
                if row_count > 0 || header_rows == 0 {
                    let label = if header_rows == 0 {
                        theme.tr("+ New session", "+ 新建会话")
                    } else {
                        theme.tr("+ Create session", "+ 创建会话")
                    };
                    let row_style = if focused {
                        theme
                            .style(theme.success)
                            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                    } else {
                        theme.style(theme.success)
                    };
                    let marker = if focused { theme.marker() } else { " " };
                    lines.push(Line::styled(format!("{marker} {label}"), row_style));
                }
                frame.render_widget(Paragraph::new(lines), *area);
                if area.width > 0 && area.height > 0 {
                    hits.push(MatrixHit {
                        rect: *area,
                        target: MatrixHitTarget::NewSession,
                    });
                }
                continue;
            };
            let column = &self.columns[visible.index];
            let focused = col_index == self.selected_column;
            let current = current_session == Some(&column.session.id);
            let session_indices = visible
                .matched
                .as_ref()
                .map_or(&[][..], |matched| matched.indices.as_slice());
            let mut lines = Vec::new();
            if header_rows > 0 {
                let header_selected = focused
                    && (highlight_session_header || self.row_focus == RowFocus::SessionHeader);
                let style = if header_selected {
                    theme.selected_style()
                } else if focused {
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
                if area.width > 0 && area.height > 0 {
                    hits.push(MatrixHit {
                        rect: Rect {
                            x: area.x,
                            y: area.y,
                            width: area.width,
                            height: header_rows.min(area.height),
                        },
                        target: MatrixHitTarget::Session(col_index),
                    });
                }
            }
            if header_rows == 2 {
                let att_label = theme.tr("● attached", "● 已附着");
                let att_ascii = theme.tr("attached", "已附着");
                let det_label = theme.tr("○ detached", "○ 未附着");
                let det_ascii = theme.tr("detached", "未附着");
                let status_span = match (column.session.is_attached(), theme.ascii) {
                    (true, false) => Span::styled(att_label, theme.style(Color::Green)),
                    (true, true) => Span::styled(att_ascii, theme.style(Color::Green)),
                    (false, false) => Span::styled(det_label, muted),
                    (false, true) => Span::styled(det_ascii, muted),
                };
                let mut sub_spans = vec![
                    Span::styled(format!("{}w ", column.session.windows), muted),
                    status_span,
                ];
                if column.session.attached > 1 {
                    let multi_spans = crate::ui::window_status_badge_spans(
                        crate::ui::WindowStatusFlags {
                            multi_clients: column.session.attached,
                            ..Default::default()
                        },
                        theme,
                    );
                    if !multi_spans.is_empty() {
                        sub_spans.push(Span::raw(" "));
                        sub_spans.extend(multi_spans);
                    }
                }
                lines.push(Line::from(sub_spans));
            }
            let offset = if focused { first_row } else { 0 };
            let mut rendered_rows = 0usize;
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
                let selected =
                    focused && self.row_focus == RowFocus::Window && row == self.selected_row;
                let row_is_reversed = selected && !highlight_session_header;
                let style = if row_is_reversed {
                    theme.selected_style()
                } else if selected {
                    theme.style(theme.accent)
                } else {
                    theme.style(theme.text)
                };
                let marker_style = if selected && !row_is_reversed {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                };
                let mut spans = vec![
                    Span::styled(if selected { theme.marker() } else { " " }, marker_style),
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
                    let window_index = window.index.to_string();
                    if window.active {
                        if row_is_reversed {
                            spans.push(Span::styled(format!("{window_index}* "), style));
                        } else {
                            spans.push(Span::styled(window_index, style));
                            spans.push(Span::styled("*", theme.style(theme.accent)));
                            spans.push(Span::styled(" ", style));
                        }
                    } else {
                        spans.push(Span::styled(format!("{window_index} "), style));
                    }
                }
                spans.extend(highlighted_spans(&window.name, indices, style));
                if window.panes > 1 && area.width >= 26 {
                    let badge = theme.adapt(&format!(" · {}p", window.panes));
                    let used: usize = spans.iter().map(|span| span.content.width()).sum();
                    if used + badge.width() <= usize::from(area.width) {
                        let badge_style = if row_is_reversed { style } else { muted };
                        spans.push(Span::styled(badge, badge_style));
                    }
                }
                let status_badges = crate::ui::window_status_badge_spans(
                    crate::ui::WindowStatusFlags {
                        zoomed: window.zoomed,
                        synchronized: window.synchronized,
                        activity: window.activity_flag,
                        bell: window.bell_flag,
                        silence: window.silence_flag,
                        dead: dead_windows.contains(&window.id),
                        multi_clients: 0,
                    },
                    theme,
                );
                if !status_badges.is_empty() {
                    let badge_w: usize = 1 + status_badges
                        .iter()
                        .map(|s| s.content.width())
                        .sum::<usize>();
                    let used: usize = spans.iter().map(|span| span.content.width()).sum();
                    if used + badge_w <= usize::from(area.width) {
                        spans.push(Span::styled(" ", style));
                        for badge_span in status_badges {
                            if row_is_reversed {
                                spans.push(Span::styled(badge_span.content, style));
                            } else {
                                spans.push(badge_span);
                            }
                        }
                    }
                }
                lines.push(Line::from(spans));
                let win_y = area.y + header_rows + rendered_rows as u16;
                if area.width > 0 && win_y < area.y + area.height {
                    hits.push(MatrixHit {
                        rect: Rect {
                            x: area.x,
                            y: win_y,
                            width: area.width,
                            height: 1,
                        },
                        target: MatrixHitTarget::Window {
                            col: col_index,
                            row,
                        },
                    });
                }
                rendered_rows += 1;
            }
            if show_create && rendered_rows < row_count && offset <= visible.windows.len() {
                let new_win_selected = focused && self.row_focus == RowFocus::NewWindow;
                let row_style = if new_win_selected {
                    theme
                        .style(theme.success)
                        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    theme.style(theme.success)
                };
                let marker = if new_win_selected {
                    theme.marker()
                } else {
                    " "
                };
                let new_win_label = theme.tr("+ New window", "+ 新建窗口");
                lines.push(Line::styled(format!("{marker} {new_win_label}"), row_style));
                let new_win_y = area.y + header_rows + rendered_rows as u16;
                if area.width > 0 && new_win_y < area.y + area.height {
                    hits.push(MatrixHit {
                        rect: Rect {
                            x: area.x,
                            y: new_win_y,
                            width: area.width,
                            height: 1,
                        },
                        target: MatrixHitTarget::NewWindow(col_index),
                    });
                }
            }
            frame.render_widget(Paragraph::new(lines), *area);
        }
        self.first_column = first_column;
        self.first_row = first_row;
        hits
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
                    zoomed: false,
                    synchronized: false,
                    activity_flag: false,
                    bell_flag: false,
                    silence_flag: false,
                    layout: String::new(),
                    monitor_activity: false,
                    monitor_silence: 0,
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
            zoomed: false,
            synchronized: false,
            activity_flag: false,
            bell_flag: false,
            silence_flag: false,
            layout: String::new(),
            monitor_activity: false,
            monitor_silence: 0,
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

    #[test]
    fn sync_preserves_selection_and_handles_missing_window_or_filter_change() {
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 2, 0);
        let gamma = make_session("$2", "gamma", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 1),
            make_window("@2", &beta, 0, "api", true, 1),
            make_window("@3", &beta, 1, "worker", false, 2),
        ];
        let mut picker = ResumePicker::default();
        assert!(picker.is_empty());
        assert!(!picker.has_visible_matches());

        let mut matcher = FuzzyMatcher::new();
        picker.sync(vec![alpha.clone(), beta.clone()], windows, "", &mut matcher);
        assert!(!picker.is_empty());
        assert!(picker.has_visible_matches());
        assert!(picker.select_session(&beta.id));
        assert!(picker.move_row(1));
        assert_eq!(
            picker.selected_window().map(|(s, w)| (&s.id, &w.id)),
            Some((&beta.id, &WindowId::new("@3").expect("id")))
        );

        // Live refresh reorders sessions and adds gamma, preserving beta:worker.
        let refreshed_windows = vec![
            make_window("@2", &beta, 0, "api", true, 1),
            make_window("@3", &beta, 1, "worker", false, 2),
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 1),
            make_window("@4", &gamma, 0, "shell", true, 1),
        ];
        picker.sync(
            vec![gamma.clone(), beta.clone(), alpha.clone()],
            refreshed_windows,
            "",
            &mut matcher,
        );
        assert_eq!(
            picker.selected_window().map(|(s, w)| (&s.id, &w.id)),
            Some((&beta.id, &WindowId::new("@3").expect("id")))
        );

        // Removing the previously focused window falls back to that session's initial_row.
        let without_worker = vec![
            make_window("@2", &beta, 0, "api", true, 1),
            make_window("@0", &alpha, 0, "editor", true, 1),
        ];
        picker.sync(
            vec![alpha.clone(), beta.clone()],
            without_worker,
            "",
            &mut matcher,
        );
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&beta.id));
        assert_eq!(
            picker.selected_window().map(|(_, w)| &w.id),
            Some(&WindowId::new("@2").expect("id"))
        );

        // Changing the filter recomputes focus to the best match instead of keeping old selection.
        let re_windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@2", &beta, 0, "api", true, 1),
        ];
        picker.sync(vec![alpha.clone(), beta], re_windows, "alpha", &mut matcher);
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&alpha.id));

        assert!(!picker.set_filter("alpha", &mut matcher));
        assert!(picker.set_filter("nope", &mut matcher));
        assert!(!picker.is_empty());
        assert!(!picker.has_visible_matches());
    }

    #[test]
    fn navigation_and_click_helpers_report_state_transitions() {
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 2, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 1),
            make_window("@2", &beta, 0, "api", true, 1),
            make_window("@3", &beta, 1, "worker", false, 1),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        assert!(!picker.move_column(1));
        assert!(!picker.cycle_column());
        assert!(!picker.move_row(1));
        assert!(!picker.select_session(&alpha.id));
        assert!(!picker.select_visible_cell(0, None));

        picker.replace(vec![alpha.clone(), beta.clone()], windows, &mut matcher);
        // Columns wrap circularly in both directions (alpha <-> beta).
        assert!(picker.move_column(-1));
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&beta.id));
        assert!(picker.move_column(1));
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&alpha.id));
        assert!(picker.move_column(1));
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&beta.id));

        assert!(picker.cycle_column());
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&alpha.id));

        // Rows wrap circularly in both directions (row 0 <-> row 1).
        assert!(picker.move_row(-1));
        assert_eq!(picker.selected_row, 1);
        assert!(picker.move_row(1));
        assert_eq!(picker.selected_row, 0);
        assert!(picker.move_row(1));
        assert_eq!(picker.selected_row, 1);

        // Single-column / single-window still returns false without pointless redraws.
        let mut single = ResumePicker::default();
        single.replace(
            vec![alpha.clone()],
            vec![make_window("@0", &alpha, 0, "editor", true, 1)],
            &mut matcher,
        );
        assert!(!single.move_column(1));
        assert!(!single.move_column(-1));
        assert!(!single.move_row(1));
        assert!(!single.move_row(-1));

        let missing = SessionId::new("$999").expect("missing id");
        assert!(!picker.select_session(&missing));
        assert!(picker.select_session(&beta.id));
        assert_eq!(picker.selected_session().map(|s| &s.id), Some(&beta.id));
        assert_eq!(picker.selected_row, 0);

        // Mouse click semantics: first click focuses (false), second click on focused returns true.
        assert!(picker.select_visible_cell(1, None));
        assert!(!picker.select_visible_cell(0, None));
        assert!(picker.select_visible_cell(0, None));
        assert!(!picker.select_visible_cell(1, Some(1)));
        assert_eq!(
            picker.selected_window().map(|(s, w)| (&s.id, &w.id)),
            Some((&beta.id, &WindowId::new("@3").expect("id")))
        );
        assert!(picker.select_visible_cell(1, Some(1)));
        assert!(!picker.select_visible_cell(9, None));
        assert!(!picker.select_visible_cell(1, Some(9)));
    }

    #[test]
    fn render_matrix_body_highlights_session_header_records_hits_and_shows_empty_guidance() {
        let mut empty_picker = ResumePicker::default();
        let mut term = Terminal::new(TestBackend::new(60, 5)).expect("empty term");
        let mut hits = Vec::new();
        term.draw(|frame| {
            hits =
                empty_picker.render_matrix_body(frame, frame.area(), None, false, Theme::default());
        })
        .expect("draw empty");
        assert!(hits.is_empty());
        let empty_screen = term.backend().to_string();
        assert!(empty_screen.contains("No tmux sessions"));
        assert!(empty_screen.contains("Press Enter or type /new <name> to create a session"));

        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let windows = vec![
            make_window("@0", &alpha, 0, "editor", true, 1),
            make_window("@1", &alpha, 1, "logs", false, 2),
            make_window("@2", &beta, 0, "api", true, 1),
        ];
        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![alpha.clone(), beta], windows, &mut matcher);

        let body = Rect::new(2, 3, 56, 8);
        let mut matrix_term = Terminal::new(TestBackend::new(60, 12)).expect("matrix term");
        matrix_term
            .draw(|frame| {
                hits =
                    picker.render_matrix_body(frame, body, Some(&alpha.id), true, Theme::default());
            })
            .expect("draw matrix body");

        assert!(hits.contains(&MatrixHit {
            rect: Rect::new(2, 3, 28, 2),
            target: MatrixHitTarget::Session(0),
        }));
        assert!(hits.contains(&MatrixHit {
            rect: Rect::new(2, 5, 28, 1),
            target: MatrixHitTarget::Window { col: 0, row: 0 },
        }));
        assert!(hits.contains(&MatrixHit {
            rect: Rect::new(2, 6, 28, 1),
            target: MatrixHitTarget::Window { col: 0, row: 1 },
        }));
        assert!(hits.contains(&MatrixHit {
            rect: Rect::new(31, 3, 27, 2),
            target: MatrixHitTarget::Session(1),
        }));
        assert!(hits.contains(&MatrixHit {
            rect: Rect::new(31, 5, 27, 1),
            target: MatrixHitTarget::Window { col: 1, row: 0 },
        }));

        let buf = matrix_term.backend().buffer();
        // Focused session header row (y = 3) uses REVERSED | BOLD.
        assert!(
            (2..12)
                .map(|x| &buf[(x, 3)])
                .any(|cell| cell.symbol() == "a"
                    && cell.modifier.contains(Modifier::REVERSED | Modifier::BOLD))
        );
        // Focused window row (y = 5) keeps `›` marker without REVERSED (no double-reversing).
        assert_eq!(buf[(2, 5)].symbol(), "›");
        assert!(!(2..16).any(|x| buf[(x, 5)].modifier.contains(Modifier::REVERSED)));

        assert!(picker.set_filter("unmatched-query", &mut matcher));
        matrix_term
            .draw(|frame| {
                hits = picker.render_matrix_body(frame, body, None, true, Theme::default());
            })
            .expect("draw no match");
        assert!(hits.is_empty());
        assert!(
            matrix_term
                .backend()
                .to_string()
                .contains("No matching sessions or windows")
        );
    }

    #[test]
    fn create_actions_supports_2d_header_new_window_and_new_session_navigation() {
        let mut picker = ResumePicker::with_create_actions();
        let mut matcher = FuzzyMatcher::new();

        // Empty matrix with create_actions renders highlighted + New session and emits NewSession hit.
        let mut empty_term = Terminal::new(TestBackend::new(60, 5)).expect("empty term");
        let mut hits = Vec::new();
        empty_term
            .draw(|frame| {
                hits =
                    picker.render_matrix_body(frame, frame.area(), None, false, Theme::default());
            })
            .expect("draw empty create");
        let empty_screen = empty_term.backend().to_string();
        assert!(empty_screen.contains("› + New session"));
        assert!(empty_screen.contains("No tmux sessions — press Enter to create"));
        assert_eq!(
            hits,
            vec![MatrixHit {
                rect: Rect::new(0, 0, 60, 5),
                target: MatrixHitTarget::NewSession,
            }]
        );
        assert_eq!(picker.selection(), Some(MatrixSelection::NewSession));
        assert_eq!(picker.preview_window(), None);

        // Populate 2 sessions (alpha with 2 windows, beta with 1 window).
        let alpha = make_session("$0", "alpha", 2, 1);
        let beta = make_session("$1", "beta", 1, 0);
        let w0 = make_window("@0", &alpha, 0, "editor", true, 1);
        let w1 = make_window("@1", &alpha, 1, "logs", false, 1);
        let w2 = make_window("@2", &beta, 0, "api", true, 1);
        picker.sync(
            vec![alpha.clone(), beta.clone()],
            vec![w0.clone(), w1.clone(), w2.clone()],
            "",
            &mut matcher,
        );
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w0))
        );
        assert!(!picker.is_session_selected());

        // ↑ from row 0 enters SessionHeader; ↑ again wraps to + New window, ↓ wraps back to SessionHeader.
        assert!(picker.move_row(-1));
        assert!(picker.is_session_selected());
        assert_eq!(picker.selection(), Some(MatrixSelection::Session(&alpha)));
        assert_eq!(picker.selected_window(), None);
        assert_eq!(picker.preview_window(), Some((&alpha, &w0)));
        assert!(picker.move_row(-1));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewWindow(&alpha)));
        assert!(picker.move_row(1));
        assert_eq!(picker.selection(), Some(MatrixSelection::Session(&alpha)));

        // ↓ from SessionHeader goes to row 0, then row 1, then + New window; ↓ again wraps to SessionHeader.
        assert!(picker.move_row(1));
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w0))
        );
        assert!(picker.move_row(1));
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w1))
        );
        assert!(picker.move_row(1));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewWindow(&alpha)));
        assert_eq!(picker.preview_window(), Some((&alpha, &w0)));
        assert!(picker.move_row(1));
        assert_eq!(picker.selection(), Some(MatrixSelection::Session(&alpha)));
        assert!(picker.move_row(-1));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewWindow(&alpha)));

        // sync preserves RowFocus::NewWindow when filter & session remain unchanged.
        picker.sync(
            vec![alpha.clone(), beta.clone()],
            vec![w0.clone(), w1.clone(), w2.clone()],
            "",
            &mut matcher,
        );
        assert_eq!(picker.selection(), Some(MatrixSelection::NewWindow(&alpha)));

        // ↑ from + New window returns to last window (w1).
        assert!(picker.move_row(-1));
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w1))
        );

        // → traverses alpha -> beta -> + New session -> wraps to alpha; ← from alpha wraps to + New session.
        assert!(picker.move_column(1));
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&beta, &w2))
        );
        assert!(picker.move_column(1));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewSession));
        assert_eq!(picker.selected_session(), None);
        assert_eq!(picker.preview_window(), None);
        assert!(picker.move_column(1));
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w0))
        );
        assert!(picker.move_column(-1));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewSession));
        assert!(picker.cycle_column());
        assert_eq!(
            picker.selection(),
            Some(MatrixSelection::Window(&alpha, &w0))
        );

        // focus_session_header / focus_window_row and sync preserving SessionHeader.
        picker.focus_session_header();
        assert!(picker.is_session_selected());
        picker.sync(
            vec![alpha.clone(), beta.clone()],
            vec![w0.clone(), w1.clone(), w2.clone()],
            "",
            &mut matcher,
        );
        assert!(picker.is_session_selected());
        assert_eq!(picker.selection(), Some(MatrixSelection::Session(&alpha)));

        // Render 3 columns (alpha, beta, + New session) and verify hits & styling.
        let mut matrix_term = Terminal::new(TestBackend::new(72, 9)).expect("matrix term");
        matrix_term
            .draw(|frame| {
                hits = picker.render_matrix_body(
                    frame,
                    frame.area(),
                    Some(&alpha.id),
                    false,
                    Theme::default(),
                );
            })
            .expect("draw 3-col create matrix");
        let screen = matrix_term.backend().to_string();
        assert!(screen.contains("+ New window"));
        assert!(screen.contains("+ New session"));
        assert!(screen.contains("workspace"));
        assert!(screen.contains("+ Create session"));
        assert!(
            hits.iter()
                .any(|h| matches!(h.target, MatrixHitTarget::NewWindow(0)))
        );
        assert!(
            hits.iter()
                .any(|h| matches!(h.target, MatrixHitTarget::NewWindow(1)))
        );
        assert!(
            hits.iter()
                .any(|h| matches!(h.target, MatrixHitTarget::NewSession))
        );
        let buf = matrix_term.backend().buffer();
        assert!(
            (0..14)
                .map(|x| &buf[(x, 0)])
                .any(|cell| cell.symbol() == "a"
                    && cell.modifier.contains(Modifier::REVERSED | Modifier::BOLD))
        );

        // Clicking hits via select_hit: Session focuses on 1st click, confirms on 2nd;
        // NewWindow and NewSession return true on single click.
        picker.focus_window_row();
        assert!(!picker.is_session_selected());
        assert!(!picker.select_hit(&MatrixHitTarget::Session(1)));
        assert_eq!(picker.selection(), Some(MatrixSelection::Session(&beta)));
        assert!(picker.select_hit(&MatrixHitTarget::Session(1)));
        assert!(picker.select_hit(&MatrixHitTarget::NewWindow(0)));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewWindow(&alpha)));
        assert!(picker.select_hit(&MatrixHitTarget::NewSession));
        assert_eq!(picker.selection(), Some(MatrixSelection::NewSession));

        // Filtering hides + New window and + New session.
        picker.sync(vec![alpha, beta], vec![w0, w1, w2], "editor", &mut matcher);
        matrix_term
            .draw(|frame| {
                hits =
                    picker.render_matrix_body(frame, frame.area(), None, false, Theme::default());
            })
            .expect("draw filtered create matrix");
        let filtered = matrix_term.backend().to_string();
        assert!(!filtered.contains("+ New window"));
        assert!(!filtered.contains("+ New session"));
        assert!(!hits.iter().any(|h| matches!(
            h.target,
            MatrixHitTarget::NewWindow(_) | MatrixHitTarget::NewSession
        )));
    }

    #[test]
    fn matrix_renders_zoom_sync_activity_bell_dead_and_multiclient_badges() {
        let sess = make_session("$0", "ops", 3, 2);
        let mut w0 = make_window("@0", &sess, 0, "main", true, 2);
        w0.zoomed = true;
        w0.synchronized = true;
        let mut w1 = make_window("@1", &sess, 1, "logs", false, 1);
        w1.bell_flag = true;
        let mut w2 = make_window("@2", &sess, 2, "worker", false, 1);
        w2.activity_flag = true;

        let mut picker = ResumePicker::default();
        let mut matcher = FuzzyMatcher::new();
        picker.replace(vec![sess.clone()], vec![w0, w1, w2], &mut matcher);

        let dead_windows: HashSet<WindowId> =
            [WindowId::new("@2").expect("wid")].into_iter().collect();
        let mut term = Terminal::new(TestBackend::new(60, 10)).expect("term");
        term.draw(|frame| {
            picker.render_matrix_body_with_dead_windows(
                frame,
                frame.area(),
                Some(&sess.id),
                false,
                &dead_windows,
                Theme::default(),
            );
        })
        .expect("draw badges");
        let screen = term.backend().to_string();
        assert!(screen.contains("👥 2"), "{screen}");
        assert!(screen.contains("⛶"), "{screen}");
        assert!(screen.contains("⚡SYNC"), "{screen}");
        assert!(screen.contains("🔔"), "{screen}");
        assert!(screen.contains("●"), "{screen}");
        assert!(screen.contains("✕"), "{screen}");

        let ascii_theme = Theme {
            ascii: true,
            ..Theme::default()
        };
        term.draw(|frame| {
            picker.render_matrix_body_with_dead_windows(
                frame,
                frame.area(),
                Some(&sess.id),
                false,
                &dead_windows,
                ascii_theme,
            );
        })
        .expect("draw ascii badges");
        let ascii_screen = term.backend().to_string();
        assert!(ascii_screen.is_ascii(), "{ascii_screen}");
        assert!(ascii_screen.contains("2c"), "{ascii_screen}");
        assert!(ascii_screen.contains("[Z]"), "{ascii_screen}");
        assert!(ascii_screen.contains("[!SYNC]"), "{ascii_screen}");
        assert!(ascii_screen.contains("[!]"), "{ascii_screen}");
        assert!(ascii_screen.contains("[X]"), "{ascii_screen}");
    }
}
