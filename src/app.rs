//! Application state and keyboard focus. Views never handle keys below the top
//! of the stack; all tmux mutations happen here, outside the rendering layer.
mod resume;

use std::{env, path::PathBuf};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use resume::ResumePicker;

use crate::{
    commands::{self, CommandKind, CommandMatch, ParsedCommand},
    config::{self, Config},
    fuzzy::{FuzzyMatcher, MatchKind},
    terminal::ScreenMode,
    tmux::{
        Context, InsideContext, Pane, Session, SessionId, SplitDirection, Target, Tmux, TmuxResult,
        Window,
    },
    tributes::{self, MicroTributeKind, TributeId},
    ui::{
        self, CommandItem, ComposerMode, ConfigItemView, HomeDetails, HomeSessionEntry,
        HomeTelemetry, TextInput, Theme, ThemePreset,
    },
};

#[derive(Debug, Clone, Default)]
pub struct AppOptions {
    pub initial_command: Option<String>,
}

fn working_directory_label() -> Option<String> {
    let directory = env::current_dir().ok()?;
    if let Some(home) = env::home_dir()
        && let Ok(relative) = directory.strip_prefix(home)
    {
        return Some(if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        });
    }
    Some(directory.display().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Quit,
    Attach(Target),
    Detach(String),
    Switched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerPurpose {
    List,
    Rename,
    Kill,
    Windows,
    Panes,
    Split,
    NewWindow,
}
impl PickerPurpose {
    fn title(self) -> &'static str {
        match self {
            Self::List => "Sessions",
            Self::Rename => "Rename session — select a session",
            Self::Kill => "Kill session — select a session",
            Self::Windows => "Windows",
            Self::Panes => "Panes",
            Self::Split => "Split pane — select a pane",
            Self::NewWindow => "New window — select its session",
        }
    }
    fn footer(self) -> &'static str {
        match self {
            Self::List | Self::Windows | Self::Panes => "Enter Select · Esc Back",
            _ => "↑↓ Navigate · Enter Select · Esc Back",
        }
    }
}

#[derive(Debug, Clone)]
enum Resource {
    Session(Session),
    Window(Window),
    Pane(Pane),
}
impl Resource {
    fn label(&self) -> String {
        match self {
            Self::Session(s) => s.name.clone(),
            Self::Window(w) => format!("{}:{} {}", w.session_name, w.index, w.name),
            Self::Pane(p) => format!(
                "{}:{}.{} {}",
                p.session_name, p.window_index, p.index, p.current_command
            ),
        }
    }
    fn target(&self) -> Target {
        match self {
            Self::Session(s) => Target::Session(s.id.clone()),
            Self::Window(w) => Target::Window(w.session_id.clone(), w.id.clone()),
            Self::Pane(p) => Target::Pane(p.id.clone()),
        }
    }
    fn session_id(&self) -> &SessionId {
        match self {
            Self::Session(s) => &s.id,
            Self::Window(w) => &w.session_id,
            Self::Pane(p) => &p.session_id,
        }
    }
    fn actions(&self) -> &'static [(&'static str, ResourceAction)] {
        use ResourceAction::{Close, Open, Rename};
        match self {
            Self::Session(_) => &[
                ("Resume session", Open),
                ("Rename session", Rename),
                ("Close session", Close),
            ],
            Self::Window(_) => &[
                ("Open window", Open),
                ("Rename window", Rename),
                ("Close window", Close),
            ],
            Self::Pane(_) => &[("Open pane", Open), ("Close pane", Close)],
        }
    }
    fn rich_row(
        &self,
        primary_indices: Vec<usize>,
        context: &Context,
        theme: Theme,
    ) -> ui::RichPickerRow {
        let ascii = theme.ascii;
        match self {
            Self::Session(s) => {
                let is_current = context.current_session() == Some(&s.id);
                let badge = if is_current {
                    Some((
                        if ascii { "* current" } else { "★ current" }.to_owned(),
                        theme.accent,
                    ))
                } else if s.attached > 0 {
                    Some((
                        if ascii { "* attached" } else { "● attached" }.to_owned(),
                        theme.success,
                    ))
                } else {
                    Some((
                        if ascii { "o detached" } else { "○ detached" }.to_owned(),
                        theme.muted,
                    ))
                };
                let win_label = format!(
                    "{} window{}",
                    s.windows,
                    if s.windows == 1 { "" } else { "s" }
                );
                let mut parts = Vec::new();
                if let Some(tag) = tributes::session_badge(&s.name, ascii) {
                    parts.push(tag.to_owned());
                }
                parts.push(win_label);
                if is_current && s.attached > 0 {
                    parts.push("attached".to_owned());
                }
                ui::RichPickerRow {
                    primary: s.name.clone(),
                    primary_indices,
                    secondary: parts.join(" · "),
                    badge,
                    is_action: false,
                }
            }
            Self::Window(w) => {
                let badge = w.active.then(|| {
                    (
                        if ascii { "* active" } else { "★ active" }.to_owned(),
                        theme.accent,
                    )
                });
                ui::RichPickerRow {
                    primary: self.label(),
                    primary_indices,
                    secondary: format!(
                        "{} pane{} · {}",
                        w.panes,
                        if w.panes == 1 { "" } else { "s" },
                        w.id
                    ),
                    badge,
                    is_action: false,
                }
            }
            Self::Pane(p) => {
                let badge = if p.busy {
                    Some((
                        if ascii { "! busy" } else { "⚡ busy" }.to_owned(),
                        theme.warning,
                    ))
                } else if p.active {
                    Some((
                        if ascii { "* active" } else { "★ active" }.to_owned(),
                        theme.accent,
                    ))
                } else {
                    None
                };
                ui::RichPickerRow {
                    primary: self.label(),
                    primary_indices,
                    secondary: format!("{} · {}", p.id, p.current_path),
                    badge,
                    is_action: false,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Picker {
    purpose: PickerPurpose,
    query: TextInput,
    resources: Vec<Resource>,
    selected: usize,
}

#[derive(Debug, Clone, Copy)]
enum PickerRow {
    Resource(usize),
    NewWindow,
    Refresh,
}

impl Picker {
    fn rows_with_indices(&self, matcher: &mut FuzzyMatcher) -> Vec<(PickerRow, Vec<usize>)> {
        let mut matched_rows: Vec<_> = self
            .resources
            .iter()
            .enumerate()
            .filter_map(|(i, resource)| {
                matcher
                    .match_one(self.query.value(), &resource.label())
                    .map(|matched| (matched.sort_key(), i, matched.indices))
            })
            .collect();
        matched_rows.sort_by(|(score_a, idx_a, _), (score_b, idx_b, _)| {
            (score_a, idx_a).cmp(&(score_b, idx_b))
        });

        let mut rows = Vec::new();
        if self.purpose == PickerPurpose::Windows && self.query.value().is_empty() {
            rows.push((PickerRow::NewWindow, Vec::new()));
        }
        rows.extend(
            matched_rows
                .into_iter()
                .map(|(_, i, indices)| (PickerRow::Resource(i), indices)),
        );
        if self.query.value().is_empty()
            && matches!(
                self.purpose,
                PickerPurpose::List | PickerPurpose::Windows | PickerPurpose::Panes
            )
        {
            rows.push((PickerRow::Refresh, Vec::new()));
        }
        rows
    }
    fn rows(&self, matcher: &mut FuzzyMatcher) -> Vec<PickerRow> {
        self.rows_with_indices(matcher)
            .into_iter()
            .map(|(row, _)| row)
            .collect()
    }
    fn rich_rows(
        &self,
        matcher: &mut FuzzyMatcher,
        context: &Context,
        theme: Theme,
    ) -> Vec<ui::RichPickerRow> {
        self.rows_with_indices(matcher)
            .into_iter()
            .map(|(row, indices)| match row {
                PickerRow::Resource(index) => {
                    self.resources[index].rich_row(indices, context, theme)
                }
                PickerRow::NewWindow => ui::RichPickerRow {
                    primary: "+ New window".into(),
                    primary_indices: Vec::new(),
                    secondary: String::new(),
                    badge: None,
                    is_action: true,
                },
                PickerRow::Refresh => ui::RichPickerRow {
                    primary: "Refresh list".into(),
                    primary_indices: Vec::new(),
                    secondary: String::new(),
                    badge: None,
                    is_action: true,
                },
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
enum FormAction {
    NewSessionName,
    NewSessionDirectory { name: String },
    Rename(Resource),
    NewWindow(SessionId),
}
#[derive(Debug, Clone)]
struct Form {
    action: FormAction,
    input: TextInput,
}
#[derive(Debug, Clone)]
struct Confirm {
    resource: Resource,
    input: TextInput,
    message: String,
    scroll: u16,
}

#[derive(Debug, Clone, Copy)]
enum ResourceAction {
    Open,
    Rename,
    Close,
}

#[derive(Debug, Clone)]
enum View {
    Picker(Picker),
    Resume(ResumePicker),
    Actions(Resource, usize),
    Form(Form),
    Confirm(Confirm),
    Created(Resource),
    Split(Resource, usize),
    ThemePicker {
        initial: ThemePreset,
        selected: usize,
    },
    Config {
        selected: usize,
    },
    Tribute {
        id: TributeId,
        step: usize,
    },
    Help,
}
#[derive(Debug, Default)]
struct ViewStack(Vec<View>);
impl ViewStack {
    fn push(&mut self, view: View) {
        self.0.push(view);
    }
    fn pop(&mut self) -> Option<View> {
        self.0.pop()
    }
    fn top(&self) -> Option<&View> {
        self.0.last()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CachedTelemetry {
    sessions: Option<usize>,
    windows: Option<usize>,
    attached: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MentionScope {
    Standalone,
    Command(CommandKind),
}

struct MentionCandidate {
    token: String,
    kind_badge: &'static str,
    detail: String,
    indices: Vec<usize>,
    resource: Resource,
}

fn theme_toast(preset: ThemePreset) -> String {
    format!("✦ Theme: {} ({})", preset.name(), preset.origin())
}

const fn composer_mode_slug(mode: ComposerMode) -> &'static str {
    match mode {
        ComposerMode::CommandJump => "command-jump",
        ComposerMode::LiveRadar => "live-radar",
        ComposerMode::QuickLaunch => "quick-launch",
    }
}

fn quick_launch_default_name() -> String {
    let base = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let cleaned: String = base
        .trim()
        .chars()
        .map(|c| match c {
            '.' | ':' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "workspace".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub struct App<'a> {
    tmux: &'a dyn Tmux,
    composer: TextInput,
    matcher: FuzzyMatcher,
    palette_selected: usize,
    views: ViewStack,
    status: String,
    directory: Option<String>,
    theme: Theme,
    config: Config,
    config_path: Option<PathBuf>,
    config_note: Option<String>,
    context: Context,
    telemetry: CachedTelemetry,
    home_sessions: Vec<Session>,
    home_windows: Vec<Window>,
    home_panes: Vec<Pane>,
    composer_mode: ComposerMode,
    radar_selected: usize,
    mention_selected: usize,
    tip_step: usize,
    cached_preview: Option<(String, String, Vec<String>)>,
    picker_list_rect: Option<Rect>,
    picker_offset: usize,
    radar_list_rect: Option<Rect>,
    radar_offset: usize,
    easter_step: usize,
    exit: Option<Exit>,
}

impl<'a> App<'a> {
    pub fn new(tmux: &'a dyn Tmux, options: AppOptions) -> Self {
        let theme = Theme::from_env();
        let config = Config {
            theme: theme.preset,
            ascii: theme.ascii,
            no_color: theme.no_color,
            ..Config::default()
        };
        Self::init_with_state(tmux, options, theme, config, None)
    }

    pub fn with_config(
        tmux: &'a dyn Tmux,
        options: AppOptions,
        config: Config,
        config_path: Option<PathBuf>,
    ) -> Self {
        let theme = config.effective_theme();
        Self::init_with_state(tmux, options, theme, config, config_path)
    }

    fn init_with_state(
        tmux: &'a dyn Tmux,
        options: AppOptions,
        theme: Theme,
        config: Config,
        config_path: Option<PathBuf>,
    ) -> Self {
        let composer_mode = config.composer_mode;
        let mut app = Self {
            tmux,
            composer: TextInput::default(),
            matcher: FuzzyMatcher::new(),
            palette_selected: 0,
            views: ViewStack::default(),
            status: String::new(),
            directory: working_directory_label(),
            theme,
            config,
            config_path,
            config_note: None,
            context: tmux.context(),
            telemetry: CachedTelemetry::default(),
            home_sessions: Vec::new(),
            home_windows: Vec::new(),
            home_panes: Vec::new(),
            composer_mode,
            radar_selected: 0,
            mention_selected: 0,
            tip_step: 0,
            cached_preview: None,
            picker_list_rect: None,
            picker_offset: 0,
            radar_list_rect: None,
            radar_offset: 0,
            easter_step: 0,
            exit: None,
        };
        app.refresh_home_data();
        if let Some(command) = options.initial_command {
            app.execute_text(&format!("/{}", command.trim_start_matches('/')));
        }
        app
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_path(&self) -> Option<&PathBuf> {
        self.config_path.as_ref()
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn handle_event(&mut self, event: Event) {
        if let Event::Paste(text) = event {
            self.paste(&text);
            return;
        }
        if let Event::Mouse(mouse) = event {
            if self.config.mouse {
                self.handle_mouse(mouse);
            }
            return;
        }
        let Event::Key(key) = event else { return };
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.exit = Some(Exit::Quit);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('p' | 'P') => {
                    self.open_universal_palette();
                    return;
                }
                KeyCode::Char('k' | 'K') if self.is_non_text_context() => {
                    self.open_universal_palette();
                    return;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Esc {
            match self.views.pop() {
                Some(View::ThemePicker { initial, .. }) => {
                    self.theme = self.theme.with_preset(initial);
                }
                Some(View::Config { .. }) => {
                    self.config_note = None;
                }
                Some(_) => {}
                None => {
                    if self.composer.value().is_empty() {
                        self.exit = Some(Exit::Quit);
                    } else {
                        self.composer.set_value("");
                        self.palette_selected = 0;
                        self.mention_selected = 0;
                    }
                    self.status.clear();
                    return;
                }
            }
            self.status.clear();
            self.sync_top_view_after_pop();
            return;
        }
        // Take the top view out while handling it to make nested pushes explicit.
        match self.views.pop() {
            None => self.composer_key(key),
            Some(View::Picker(picker)) => self.picker_key(picker, key),
            Some(View::Resume(mut picker)) => {
                let target = picker.handle_key(key, &mut self.matcher);
                if target.is_none() {
                    self.update_resume_preview(&picker);
                }
                self.views.push(View::Resume(picker));
                if let Some(target) = target {
                    self.enter(target);
                }
            }
            Some(View::Actions(resource, selected)) => {
                self.resource_action_key(resource, selected, key)
            }
            Some(View::Form(form)) => self.form_key(form, key),
            Some(View::Confirm(confirm)) => self.confirm_key(confirm, key),
            Some(View::Created(resource)) => {
                if key.code == KeyCode::Enter {
                    self.enter(resource.target());
                } else {
                    self.views.push(View::Created(resource));
                }
            }
            Some(View::Split(resource, mut selected)) => {
                match key.code {
                    KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                        selected = 1 - selected
                    }
                    KeyCode::Enter => {
                        if let Resource::Pane(pane) = &resource {
                            let direction = if selected == 0 {
                                SplitDirection::SideBySide
                            } else {
                                SplitDirection::Stacked
                            };
                            match self.tmux.split_pane(&pane.id, direction) {
                                Ok(_) => {
                                    self.status = "Pane split. Native tmux layout updated.".into();
                                    self.refresh_parent();
                                    return;
                                }
                                Err(error) => self.status = error.to_string(),
                            }
                        }
                    }
                    _ => {}
                }
                self.views.push(View::Split(resource, selected));
            }
            Some(View::ThemePicker { initial, selected }) => {
                self.theme_picker_key(initial, selected, key);
            }
            Some(View::Config { selected }) => self.config_key(selected, key),
            Some(View::Tribute { id, step }) => self.tribute_key(id, step, key),
            Some(View::Help) => self.views.push(View::Help),
        }
    }

    fn is_non_text_context(&self) -> bool {
        match self.views.top() {
            None => self.composer.value().is_empty(),
            Some(
                View::Help
                | View::Tribute { .. }
                | View::ThemePicker { .. }
                | View::Config { .. }
                | View::Actions(..)
                | View::Split(..)
                | View::Created(..),
            ) => true,
            Some(View::Picker(_) | View::Resume(_) | View::Form(_) | View::Confirm(_)) => false,
        }
    }

    fn open_universal_palette(&mut self) {
        while let Some(view) = self.views.pop() {
            if let View::ThemePicker { initial, .. } = view {
                self.theme = self.theme.with_preset(initial);
            }
        }
        self.config_note = None;
        self.refresh_home_data();
        self.composer.set_value("/");
        self.palette_selected = 0;
        self.mention_selected = 0;
        self.status.clear();
    }

    fn sync_top_view_after_pop(&mut self) {
        match self.views.pop() {
            None => {
                self.tip_step = self.tip_step.wrapping_add(1);
                self.refresh_home_data();
            }
            Some(View::Picker(picker)) => {
                self.update_picker_preview(&picker);
                self.views.push(View::Picker(picker));
            }
            Some(View::Resume(picker)) => {
                self.update_resume_preview(&picker);
                self.views.push(View::Resume(picker));
            }
            Some(other) => self.views.push(other),
        }
    }

    fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace(['\r', '\n', '\t'], " ");
        let top = self.views.pop();
        match top {
            None => {
                self.composer.insert_str(&text);
                self.palette_selected = 0;
                self.mention_selected = 0;
                self.status.clear();
            }
            Some(View::Resume(mut picker)) => {
                picker.paste(&text, &mut self.matcher);
                self.update_resume_preview(&picker);
                self.views.push(View::Resume(picker));
            }
            Some(View::Picker(mut picker)) => {
                picker.query.insert_str(&text);
                picker.selected = 0;
                self.update_picker_preview(&picker);
                self.views.push(View::Picker(picker));
            }
            Some(View::Form(mut form)) => {
                form.input.insert_str(&text);
                self.views.push(View::Form(form));
            }
            Some(View::Confirm(mut confirm)) => {
                confirm.input.insert_str(&text);
                self.views.push(View::Confirm(confirm));
            }
            Some(other) => self.views.push(other),
        }
    }

    fn palette(&mut self) -> Vec<CommandMatch> {
        commands::popup_query(self.composer.value()).map_or_else(Vec::new, |query| {
            commands::filter_commands(query, &mut self.matcher)
        })
    }

    fn active_mention(&self) -> Option<(usize, &str, MentionScope)> {
        let text = self.composer.value();
        if commands::popup_query(text).is_some()
            || text.is_empty()
            || text.ends_with(char::is_whitespace)
        {
            return None;
        }
        let token_start = text
            .char_indices()
            .rfind(|&(_, ch)| ch.is_whitespace())
            .map_or(0, |(idx, ch)| idx + ch.len_utf8());
        let token = &text[token_start..];
        let query = token.strip_prefix('@')?;
        let prefix = text[..token_start].trim();
        let scope = if prefix.is_empty() {
            MentionScope::Standalone
        } else if prefix.starts_with('/') {
            let parsed = commands::parse(prefix).ok()?;
            if !matches!(
                parsed.command,
                CommandKind::Kill
                    | CommandKind::Rename
                    | CommandKind::Split
                    | CommandKind::Window
                    | CommandKind::Pane
                    | CommandKind::Resume
            ) {
                return None;
            }
            MentionScope::Command(parsed.command)
        } else {
            return None;
        };
        Some((token_start, query, scope))
    }

    fn all_mention_resources(&self) -> Vec<(String, &'static str, String, Resource)> {
        let mut items = Vec::new();
        for s in &self.home_sessions {
            let detail = format!(
                "{}w{}",
                s.windows,
                if s.is_attached() { " · attached" } else { "" }
            );
            items.push((
                format!("@{}", s.name),
                "session",
                detail,
                Resource::Session(s.clone()),
            ));
        }
        for w in &self.home_windows {
            items.push((
                format!("@{}:{}", w.session_name, w.index),
                "window",
                format!("{} · {}p", w.name, w.panes),
                Resource::Window(w.clone()),
            ));
        }
        for p in &self.home_panes {
            items.push((
                format!("@{}:{}.{}", p.session_name, p.window_index, p.index),
                "pane",
                format!(
                    "{}{}",
                    p.current_command,
                    if p.busy { " · busy" } else { "" }
                ),
                Resource::Pane(p.clone()),
            ));
        }
        items
    }

    fn mention_candidates(&mut self) -> Vec<MentionCandidate> {
        let Some((_, query, scope)) = self.active_mention() else {
            return Vec::new();
        };
        let query = query.to_owned();
        let all = self.all_mention_resources();
        if query.is_empty() {
            return all
                .into_iter()
                .map(|(token, kind_badge, detail, resource)| MentionCandidate {
                    token,
                    kind_badge,
                    detail,
                    indices: Vec::new(),
                    resource,
                })
                .collect();
        }
        let mut ranked = Vec::new();
        for (order, (token, kind_badge, detail, resource)) in all.into_iter().enumerate() {
            let target_str = token.strip_prefix('@').unwrap_or(&token);
            let Some(matched) = self.matcher.match_one(&query, target_str) else {
                continue;
            };
            let tier = match matched.kind {
                MatchKind::Exact => 0_u8,
                MatchKind::Prefix => 1_u8,
                MatchKind::Substring => 2_u8,
                MatchKind::Fuzzy => 3_u8,
            };
            let kind_prio = match (scope, &resource) {
                (
                    MentionScope::Command(CommandKind::Split | CommandKind::Pane),
                    Resource::Pane(_),
                ) => 0_u8,
                (MentionScope::Command(CommandKind::Window), Resource::Window(_)) => 0_u8,
                (_, Resource::Session(_)) => 0_u8,
                (_, Resource::Window(_)) => 1_u8,
                (_, Resource::Pane(_)) => 2_u8,
            };
            let indices = matched.indices.into_iter().map(|i| i + 1).collect();
            ranked.push((
                (tier, kind_prio, std::cmp::Reverse(matched.score), order),
                MentionCandidate {
                    token,
                    kind_badge,
                    detail,
                    indices,
                    resource,
                },
            ));
        }
        ranked.sort_by_key(|(key, _)| *key);
        ranked.into_iter().map(|(_, item)| item).collect()
    }

    fn composer_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl && !alt {
            match key.code {
                KeyCode::Char('b' | 'B') => {
                    self.easter_step = self.easter_step.wrapping_add(1);
                    self.status = tributes::ctrl_b_quip(self.easter_step).to_owned();
                    return;
                }
                KeyCode::Char('t' | 'T') => {
                    let next = self.theme.preset.next();
                    self.theme = self.theme.with_preset(next);
                    self.config.theme = next;
                    self.status = theme_toast(next);
                    return;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::BackTab && !ctrl && !alt {
            self.composer_mode = self.composer_mode.next();
            self.config.composer_mode = self.composer_mode;
            self.tip_step = self.tip_step.wrapping_add(1);
            self.status.clear();
            if self.composer_mode == ComposerMode::LiveRadar {
                self.refresh_home_data();
            } else {
                self.refresh_home_preview();
            }
            return;
        }
        if self.composer.value().is_empty() {
            if self.composer_mode == ComposerMode::CommandJump
                && !ctrl
                && !alt
                && let KeyCode::Char(d @ '1'..='9') = key.code
            {
                let idx = (d as usize) - ('1' as usize);
                if let Some(session) = self.home_sessions.get(idx) {
                    let id = session.id.clone();
                    self.status.clear();
                    self.enter(Target::Session(id));
                    return;
                }
                if self.home_sessions.is_empty() && d == '1' {
                    self.status.clear();
                    self.views.push(View::Form(Form {
                        action: FormAction::NewSessionName,
                        input: TextInput::default(),
                    }));
                    return;
                }
            }
            if self.composer_mode == ComposerMode::LiveRadar && !ctrl && !alt {
                match key.code {
                    KeyCode::Up => {
                        self.radar_selected = self.radar_selected.saturating_sub(1);
                        self.refresh_radar_preview();
                        return;
                    }
                    KeyCode::Down => {
                        self.radar_selected = (self.radar_selected + 1)
                            .min(self.home_sessions.len().saturating_sub(1));
                        self.refresh_radar_preview();
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(session) = self.home_sessions.get(self.radar_selected) {
                            let id = session.id.clone();
                            self.status.clear();
                            self.enter(Target::Session(id));
                        } else {
                            self.status.clear();
                            self.views.push(View::Form(Form {
                                action: FormAction::NewSessionName,
                                input: TextInput::default(),
                            }));
                        }
                        return;
                    }
                    _ => {}
                }
            }
            if key.code == KeyCode::Down && key.modifiers.is_empty() {
                self.composer.set_value("/");
                self.palette_selected = 0;
                self.mention_selected = 0;
                self.status.clear();
                return;
            }
            if key.code == KeyCode::Char('?') && (key.modifiers - KeyModifiers::SHIFT).is_empty() {
                self.status.clear();
                self.views.push(View::Help);
                return;
            }
        }
        let palette = self.palette();
        if !palette.is_empty() {
            match key.code {
                KeyCode::Up => {
                    self.palette_selected = self.palette_selected.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    self.palette_selected =
                        (self.palette_selected + 1).min(palette.len().saturating_sub(1));
                    return;
                }
                KeyCode::Tab => {
                    if let Some(candidate) = palette.get(self.palette_selected) {
                        self.composer
                            .set_value(format!("/{}", candidate.command.name));
                        self.palette_selected = 0;
                    }
                    return;
                }
                KeyCode::Enter => {
                    let mut text = self.composer.value().to_string();
                    if let Some(candidate) = palette.get(self.palette_selected) {
                        text = format!("/{}", candidate.command.name);
                    }
                    self.execute_text(&text);
                    return;
                }
                _ => {}
            }
        }
        if let Some((token_start, _, scope)) = self.active_mention() {
            let mentions = self.mention_candidates();
            if !mentions.is_empty() {
                match key.code {
                    KeyCode::Up => {
                        self.mention_selected = self.mention_selected.saturating_sub(1);
                        return;
                    }
                    KeyCode::Down => {
                        self.mention_selected =
                            (self.mention_selected + 1).min(mentions.len().saturating_sub(1));
                        return;
                    }
                    KeyCode::Tab => {
                        if let Some(candidate) = mentions.get(self.mention_selected) {
                            let mut updated = String::from(&self.composer.value()[..token_start]);
                            updated.push_str(&candidate.token);
                            if matches!(scope, MentionScope::Command(CommandKind::Rename)) {
                                updated.push(' ');
                            }
                            self.composer.set_value(updated);
                            self.mention_selected = 0;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(candidate) = mentions.get(self.mention_selected) {
                            let resource = candidate.resource.clone();
                            self.composer.set_value("");
                            self.mention_selected = 0;
                            self.status.clear();
                            match scope {
                                MentionScope::Standalone => self.enter(resource.target()),
                                MentionScope::Command(cmd) => {
                                    self.execute_command_on_resource(cmd, resource, None);
                                }
                            }
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {}
            KeyCode::Enter => {
                if self.composer_mode == ComposerMode::QuickLaunch
                    && !self.composer.value().trim_start().starts_with('/')
                    && !self.composer.value().trim_start().starts_with('@')
                {
                    self.quick_launch_submit();
                    return;
                }
                let text = self.composer.value().to_string();
                self.execute_text(&text);
            }
            _ => {
                if self.composer.handle_key(key) {
                    self.palette_selected = 0;
                    self.mention_selected = 0;
                    self.status.clear();
                }
            }
        }
    }

    fn quick_launch_submit(&mut self) {
        let raw = self.composer.value().trim().to_owned();
        let name = if raw.is_empty() {
            quick_launch_default_name()
        } else {
            raw
        };
        if let Some(existing) = self
            .home_sessions
            .iter()
            .find(|s| s.name == name || s.name.eq_ignore_ascii_case(&name))
        {
            let id = existing.id.clone();
            self.composer.set_value("");
            self.status.clear();
            self.enter(Target::Session(id));
            return;
        }
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match self.tmux.new_session(&name, &cwd) {
            Ok(session) => {
                self.composer.set_value("");
                self.status.clear();
                self.refresh_home_data();
                self.enter(Target::Session(session.id));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn theme_picker_key(&mut self, initial: ThemePreset, mut selected: usize, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            KeyCode::Down => {
                selected = (selected + 1).min(ThemePreset::ALL.len().saturating_sub(1));
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            KeyCode::Enter => {
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.config.theme = preset;
                self.status = theme_toast(preset);
            }
            KeyCode::Char('s' | 'S') if no_ctrl_alt => {
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.config.theme = preset;
                let target = self
                    .config_path
                    .clone()
                    .or_else(|| config::resolve_config_path(None));
                match target {
                    Some(path) => match self.config.save(&path) {
                        Ok(()) => {
                            self.config_path = Some(path.clone());
                            self.status =
                                format!("✦ Saved theme '{}' to {}", preset.slug(), path.display());
                        }
                        Err(error) => {
                            self.status = format!("Could not save theme: {error}");
                            self.views.push(View::ThemePicker { initial, selected });
                        }
                    },
                    None => {
                        self.status = "No config path available (set HOME or --config)".to_owned();
                        self.views.push(View::ThemePicker { initial, selected });
                    }
                }
            }
            _ => self.views.push(View::ThemePicker { initial, selected }),
        }
    }

    fn config_key(&mut self, mut selected: usize, key: KeyEvent) {
        const COUNT: usize = 8;
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(COUNT - 1),
            KeyCode::Left if no_ctrl_alt => self.cycle_config_setting(selected, false, false),
            KeyCode::Right if no_ctrl_alt => self.cycle_config_setting(selected, true, false),
            KeyCode::Enter => self.cycle_config_setting(selected, true, true),
            KeyCode::Char(' ') if no_ctrl_alt => self.cycle_config_setting(selected, true, true),
            KeyCode::Char('s' | 'S') if no_ctrl_alt => {
                let target = self
                    .config_path
                    .clone()
                    .or_else(|| config::resolve_config_path(None));
                match target {
                    Some(path) => match self.config.save(&path) {
                        Ok(()) => {
                            self.config_path = Some(path.clone());
                            let message = format!("✦ Saved config to {}", path.display());
                            self.config_note = Some(message.clone());
                            self.status = message;
                        }
                        Err(error) => {
                            let message = format!("Could not save config: {error}");
                            self.config_note = Some(message.clone());
                            self.status = message;
                        }
                    },
                    None => {
                        let message = "No config path available (set HOME or --config)".to_owned();
                        self.config_note = Some(message.clone());
                        self.status = message;
                    }
                }
            }
            _ => {}
        }
        self.views.push(View::Config { selected });
    }

    fn cycle_config_setting(&mut self, index: usize, forward: bool, wrap_height: bool) {
        let mut restart_changed = false;
        match index {
            0 => {
                self.config.screen_mode = match self.config.screen_mode {
                    ScreenMode::Fullscreen => ScreenMode::Inline,
                    ScreenMode::Inline => ScreenMode::Fullscreen,
                };
                restart_changed = true;
            }
            1 => {
                self.config.theme = if forward {
                    self.config.theme.next()
                } else {
                    let pos = ThemePreset::ALL
                        .iter()
                        .position(|&preset| preset == self.config.theme)
                        .unwrap_or(0);
                    let prev = (pos + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    ThemePreset::ALL[prev]
                };
                self.theme = self.config.effective_theme();
            }
            2 => {
                self.config.composer_mode = if forward {
                    self.config.composer_mode.next()
                } else {
                    self.config.composer_mode.next().next()
                };
                self.composer_mode = self.config.composer_mode;
                self.refresh_home_preview();
            }
            3 => {
                self.config.live_preview = !self.config.live_preview;
                if self.config.live_preview {
                    self.refresh_home_preview();
                } else {
                    self.cached_preview = None;
                }
            }
            4 => {
                self.config.mouse = !self.config.mouse;
                restart_changed = true;
            }
            5 => {
                let prev = self.config.inline_height;
                self.config.inline_height = if forward {
                    if prev >= 40 {
                        if wrap_height { 8 } else { 40 }
                    } else {
                        (prev + 1).clamp(8, 40)
                    }
                } else {
                    prev.saturating_sub(1).clamp(8, 40)
                };
                restart_changed = self.config.inline_height != prev;
            }
            6 => {
                self.config.ascii = !self.config.ascii;
                self.theme = self.config.effective_theme();
            }
            7 => {
                self.config.no_color = !self.config.no_color;
                self.theme = self.config.effective_theme();
            }
            _ => {}
        }
        if restart_changed {
            self.config_note = Some("Takes effect on next jm launch (press s to save)".into());
        }
    }

    fn config_modal_items(&self) -> Vec<ConfigItemView<'static>> {
        vec![
            ConfigItemView {
                key: "screen_mode",
                value: self.config.screen_mode.as_str().to_owned(),
                description: "Alternate fullscreen or native inline scrollback (restart)",
            },
            ConfigItemView {
                key: "theme",
                value: self.config.theme.slug().to_owned(),
                description: "Color palette preset (aurora, codex, claude, matrix, classic)",
            },
            ConfigItemView {
                key: "composer_mode",
                value: composer_mode_slug(self.config.composer_mode).to_owned(),
                description: "Default home studio mode (Shift+Tab)",
            },
            ConfigItemView {
                key: "live_preview",
                value: self.config.live_preview.to_string(),
                description: "Capture live tmux pane previews",
            },
            ConfigItemView {
                key: "mouse",
                value: self.config.mouse.to_string(),
                description: "Mouse wheel & click support in Fullscreen (restart)",
            },
            ConfigItemView {
                key: "inline_height",
                value: self.config.inline_height.to_string(),
                description: "Reserved viewport rows for inline mode 8..=40 (restart)",
            },
            ConfigItemView {
                key: "ascii",
                value: self.config.ascii.to_string(),
                description: "Pure ASCII box borders and markers",
            },
            ConfigItemView {
                key: "no_color",
                value: self.config.no_color.to_string(),
                description: "Disable ANSI color attributes",
            },
        ]
    }

    fn tribute_key(&mut self, mut id: TributeId, mut step: usize, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(' ') if no_ctrl_alt => {
                step = step.wrapping_add(1);
                self.easter_step = step;
                self.views.push(View::Tribute { id, step });
            }
            KeyCode::Left | KeyCode::BackTab if no_ctrl_alt => {
                id = id.prev();
                self.views.push(View::Tribute { id, step });
            }
            KeyCode::Right | KeyCode::Tab if no_ctrl_alt => {
                id = id.next();
                self.views.push(View::Tribute { id, step });
            }
            KeyCode::Enter => self.apply_tribute_theme(id, step),
            KeyCode::Char('t' | 'T') if no_ctrl_alt => self.apply_tribute_theme(id, step),
            _ => self.views.push(View::Tribute { id, step }),
        }
    }

    fn apply_tribute_theme(&mut self, id: TributeId, step: usize) {
        let preset = match id {
            TributeId::Codex => ThemePreset::CodexEmerald,
            TributeId::ClaudeCode => ThemePreset::ClaudeWarmth,
            TributeId::Antigravity => ThemePreset::AntigravityAurora,
            TributeId::HallOfFame => self.theme.preset.next(),
        };
        self.theme = self.theme.with_preset(preset);
        self.config.theme = preset;
        let card = tributes::card(id, step);
        self.status = format!(
            "✦ {} theme applied: {} ({})",
            card.badge,
            preset.name(),
            preset.origin()
        );
    }

    fn resolve_mention_resource(&mut self, raw_token: &str) -> Option<Resource> {
        let needle = raw_token
            .trim()
            .strip_prefix('@')
            .unwrap_or(raw_token)
            .trim();
        if needle.is_empty() {
            return None;
        }
        self.refresh_home_data();
        let all = self.all_mention_resources();
        if let Some((_, _, _, res)) = all
            .iter()
            .find(|(tok, _, _, _)| tok.strip_prefix('@').unwrap_or(tok) == needle)
        {
            return Some(res.clone());
        }
        if let Some((_, _, _, res)) = all.iter().find(|(tok, _, _, _)| {
            tok.strip_prefix('@')
                .unwrap_or(tok)
                .eq_ignore_ascii_case(needle)
        }) {
            return Some(res.clone());
        }
        if let Some((sess_q, win_q)) = needle.split_once(':')
            && let Some(win) = self.home_windows.iter().find(|w| {
                w.session_name.eq_ignore_ascii_case(sess_q)
                    && (w.index.to_string() == win_q || w.name.eq_ignore_ascii_case(win_q))
            })
        {
            return Some(Resource::Window(win.clone()));
        }
        type MentionScore = (u8, std::cmp::Reverse<u32>, usize);
        let mut best: Option<(MentionScore, Resource)> = None;
        for (order, (tok, _, _, res)) in all.into_iter().enumerate() {
            let candidate = tok.strip_prefix('@').unwrap_or(&tok);
            let Some(matched) = self.matcher.match_one(needle, candidate) else {
                continue;
            };
            let tier = match matched.kind {
                MatchKind::Exact => 0_u8,
                MatchKind::Prefix => 1_u8,
                MatchKind::Substring => 2_u8,
                MatchKind::Fuzzy => 3_u8,
            };
            let key = (tier, std::cmp::Reverse(matched.score), order);
            if best.as_ref().is_none_or(|(prev, _)| key < *prev) {
                best = Some((key, res));
            }
        }
        best.map(|(_, res)| res)
    }

    fn resolve_pane_for_resource(&self, resource: &Resource) -> Option<Pane> {
        match resource {
            Resource::Pane(pane) => Some(pane.clone()),
            Resource::Window(window) => self
                .home_panes
                .iter()
                .find(|p| p.window_id == window.id && p.active)
                .or_else(|| self.home_panes.iter().find(|p| p.window_id == window.id))
                .cloned(),
            Resource::Session(session) => self
                .home_panes
                .iter()
                .find(|p| p.session_id == session.id && p.active)
                .or_else(|| self.home_panes.iter().find(|p| p.session_id == session.id))
                .cloned(),
        }
    }

    fn split_mention_and_arg(&self, text_after_cmd: &str) -> (String, Option<String>) {
        let rest = text_after_cmd.trim_start();
        for (token, _, _, _) in self.all_mention_resources() {
            if rest == token {
                return (token, None);
            }
            if let Some(after) = rest.strip_prefix(&token)
                && after.chars().next().is_some_and(char::is_whitespace)
            {
                let extra = after.trim();
                return (token, (!extra.is_empty()).then(|| extra.to_owned()));
            }
        }
        match rest.split_once(char::is_whitespace) {
            Some((head, tail)) => {
                let extra = tail.trim();
                (
                    head.to_owned(),
                    (!extra.is_empty()).then(|| extra.to_owned()),
                )
            }
            None => (rest.to_owned(), None),
        }
    }

    fn execute_command_on_resource(
        &mut self,
        cmd: CommandKind,
        resource: Resource,
        extra_arg: Option<String>,
    ) {
        match cmd {
            CommandKind::Kill => self.kill_confirm(resource),
            CommandKind::Resume => self.enter(resource.target()),
            CommandKind::Split => {
                if let Some(pane) = self.resolve_pane_for_resource(&resource) {
                    self.views.push(View::Split(Resource::Pane(pane), 0));
                } else {
                    self.open_picker(PickerPurpose::Split);
                }
            }
            CommandKind::Rename => {
                let target_resource = match resource {
                    Resource::Pane(pane) => self
                        .home_windows
                        .iter()
                        .find(|w| w.id == pane.window_id)
                        .cloned()
                        .map(Resource::Window)
                        .unwrap_or(Resource::Pane(pane)),
                    other => other,
                };
                if let Some(new_name) = extra_arg {
                    let renamed_current = matches!(
                        &target_resource,
                        Resource::Session(session)
                            if self.context.current_session() == Some(&session.id)
                    );
                    let result = match &target_resource {
                        Resource::Session(session) => {
                            self.tmux.rename_session(&session.id, &new_name)
                        }
                        Resource::Window(window) => self.tmux.rename_window(&window.id, &new_name),
                        Resource::Pane(_) => {
                            self.open_picker(PickerPurpose::Rename);
                            return;
                        }
                    };
                    match result {
                        Ok(()) => {
                            if renamed_current && let Context::Inside(inside) = &mut self.context {
                                inside.session_name = Some(new_name);
                            }
                            self.refresh_home_data();
                            self.status = "Renamed".into();
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                } else {
                    self.rename_form(target_resource);
                }
            }
            CommandKind::Window => match resource {
                Resource::Window(_) => self.views.push(View::Actions(resource, 0)),
                Resource::Session(session) => {
                    self.open_picker_with_query(PickerPurpose::Windows, &session.name);
                }
                Resource::Pane(pane) => {
                    self.open_picker_with_query(PickerPurpose::Windows, &pane.session_name);
                }
            },
            CommandKind::Pane => match resource {
                Resource::Pane(_) => self.views.push(View::Actions(resource, 0)),
                Resource::Session(session) => {
                    self.open_picker_with_query(PickerPurpose::Panes, &session.name);
                }
                Resource::Window(window) => {
                    let q = format!("{}:{}", window.session_name, window.index);
                    self.open_picker_with_query(PickerPurpose::Panes, &q);
                }
            },
            _ => {}
        }
    }

    fn active_or_first_pane(&self) -> Option<Pane> {
        if let Context::Inside(inside) = &self.context
            && let Some(pane_id) = &inside.pane_id
            && let Some(pane) = self.home_panes.iter().find(|p| &p.id == pane_id)
        {
            return Some(pane.clone());
        }
        if let Some(session) = self.home_sessions.first()
            && let Some(pane) = self
                .home_panes
                .iter()
                .find(|p| p.session_id == session.id && p.active)
                .or_else(|| self.home_panes.iter().find(|p| p.session_id == session.id))
        {
            return Some(pane.clone());
        }
        self.home_panes.first().cloned()
    }

    fn split_quick_direction(&mut self, direction: SplitDirection) {
        self.refresh_home_data();
        if let Some(pane) = self.active_or_first_pane() {
            match self.tmux.split_pane(&pane.id, direction) {
                Ok(_) => {
                    self.status = "Pane split. Native tmux layout updated.".into();
                    self.refresh_home_data();
                }
                Err(error) => self.status = error.to_string(),
            }
        } else {
            self.open_picker(PickerPurpose::Split);
        }
    }

    fn execute_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if let Some(rest_of_slash) = trimmed.strip_prefix('/') {
            let body = rest_of_slash.trim();
            if let Some((cmd_word, rest)) = body.split_once(char::is_whitespace)
                && rest.trim_start().starts_with('@')
                && let Ok(parsed_head) = commands::parse(&format!("/{cmd_word}"))
                && matches!(
                    parsed_head.command,
                    CommandKind::Kill
                        | CommandKind::Rename
                        | CommandKind::Split
                        | CommandKind::Window
                        | CommandKind::Pane
                        | CommandKind::Resume
                )
            {
                self.refresh_home_data();
                let (token, extra_arg) = self.split_mention_and_arg(rest);
                if let Some(resource) = self.resolve_mention_resource(&token) {
                    self.status.clear();
                    self.composer.set_value("");
                    self.palette_selected = 0;
                    self.mention_selected = 0;
                    self.execute_command_on_resource(parsed_head.command, resource, extra_arg);
                } else {
                    self.status = format!("No target matching '{token}'.");
                }
                return;
            }
            self.run_parsed_command(commands::parse(trimmed));
            return;
        }

        if trimmed.starts_with('@') {
            if let Some(resource) = self.resolve_mention_resource(trimmed) {
                self.status.clear();
                self.composer.set_value("");
                self.palette_selected = 0;
                self.mention_selected = 0;
                self.enter(resource.target());
            } else {
                self.status = format!("No target matching '{trimmed}'.");
            }
            return;
        }

        if !trimmed.is_empty() {
            if let Ok(command) = commands::parse(&format!("/{trimmed}")) {
                self.run_parsed_command(Ok(command));
                return;
            }
            let lower = trimmed.to_ascii_lowercase();
            if let Some(rest) = lower
                .strip_prefix("create ")
                .or_else(|| (lower == "create").then_some(""))
            {
                let original_rest = trimmed.get(6..).map(str::trim).unwrap_or("");
                let rewritten = if rest.is_empty() {
                    "/new".to_owned()
                } else {
                    format!("/new {original_rest}")
                };
                self.run_parsed_command(commands::parse(&rewritten));
                return;
            }
            if matches!(lower.as_str(), "split right" | "hsplit") {
                self.status.clear();
                self.composer.set_value("");
                self.palette_selected = 0;
                self.split_quick_direction(SplitDirection::SideBySide);
                return;
            }
            if matches!(lower.as_str(), "split down" | "vsplit") {
                self.status.clear();
                self.composer.set_value("");
                self.palette_selected = 0;
                self.split_quick_direction(SplitDirection::Stacked);
                return;
            }
            if let Some(target_part) = trimmed
                .strip_prefix("kill ")
                .or_else(|| trimmed.strip_prefix("KILL "))
                .or_else(|| trimmed.strip_prefix("close "))
                .or_else(|| trimmed.strip_prefix("CLOSE "))
                .or_else(|| {
                    (lower.starts_with("kill ") || lower.starts_with("close ")).then(|| {
                        let skip = if lower.starts_with("kill ") { 5 } else { 6 };
                        trimmed.get(skip..).unwrap_or("")
                    })
                })
            {
                let target_name = target_part.trim();
                if !target_name.is_empty() {
                    self.refresh_home_data();
                    if let Some(resource) = self.resolve_mention_resource(target_name) {
                        self.status.clear();
                        self.composer.set_value("");
                        self.palette_selected = 0;
                        self.kill_confirm(resource);
                        return;
                    }
                }
            }
            if let Some(session) = self
                .home_sessions
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(trimmed))
            {
                let id = session.id.clone();
                self.status.clear();
                self.composer.set_value("");
                self.palette_selected = 0;
                self.enter(Target::Session(id));
                return;
            }
        }

        self.run_parsed_command(commands::parse(text));
    }

    fn run_parsed_command(&mut self, parsed: Result<ParsedCommand, commands::ParseError>) {
        match parsed {
            Ok(command) => {
                self.status.clear();
                // A submitted slash command becomes a view action. Start the
                // next command with a fresh composer when the user returns.
                self.composer.set_value("");
                self.palette_selected = 0;
                self.mention_selected = 0;
                self.execute(command);
            }
            Err(error) => {
                let message = error.to_string();
                self.status = if message.ends_with('?') {
                    message
                } else {
                    format!("{message}. Type / to choose a command.")
                };
            }
        }
    }

    fn execute(&mut self, parsed: ParsedCommand) {
        use CommandKind as C;
        match parsed.command {
            C::Resume => {
                let mut picker = ResumePicker::default();
                self.reload_resume(&mut picker);
                self.views.push(View::Resume(picker));
            }
            C::List => self.open_picker(PickerPurpose::List),
            C::Rename => self.open_picker(PickerPurpose::Rename),
            C::Kill => {
                if !parsed.args.is_empty() {
                    let target_name = parsed.args.join(" ");
                    self.refresh_home_data();
                    if let Some(resource) = self.resolve_mention_resource(&target_name) {
                        self.kill_confirm(resource);
                        return;
                    }
                    self.open_picker_with_query(PickerPurpose::Kill, &target_name);
                    return;
                }
                self.open_picker(PickerPurpose::Kill);
            }
            C::Window => self.open_picker(PickerPurpose::Windows),
            C::Pane => self.open_picker(PickerPurpose::Panes),
            C::Split => {
                if let Some(arg) = parsed.args.first() {
                    let low = arg.to_ascii_lowercase();
                    if matches!(low.as_str(), "horizontal" | "h" | "right" | "side") {
                        self.split_quick_direction(SplitDirection::SideBySide);
                        return;
                    }
                    if matches!(
                        low.as_str(),
                        "vertical" | "v" | "down" | "stack" | "stacked"
                    ) {
                        self.split_quick_direction(SplitDirection::Stacked);
                        return;
                    }
                }
                self.open_picker(PickerPurpose::Split);
            }
            C::New => {
                if let Some(name) = parsed.args.first() {
                    if let Some(path) = parsed.args.get(1) {
                        self.create_session(name, path);
                    } else {
                        self.views.push(View::Form(Form {
                            action: FormAction::NewSessionDirectory { name: name.clone() },
                            input: TextInput::default(),
                        }));
                    }
                } else {
                    self.views.push(View::Form(Form {
                        action: FormAction::NewSessionName,
                        input: TextInput::default(),
                    }));
                }
            }
            C::Theme => {
                if !parsed.args.is_empty() {
                    let arg = parsed.args.join(" ");
                    if let Some(preset) = ThemePreset::from_name(&arg) {
                        self.theme = self.theme.with_preset(preset);
                        self.config.theme = preset;
                        self.status = theme_toast(preset);
                        return;
                    }
                    self.status = format!("Unknown theme '{arg}'. Pick a preset below.");
                }
                let selected = ThemePreset::ALL
                    .iter()
                    .position(|&p| p == self.theme.preset)
                    .unwrap_or(0);
                self.views.push(View::ThemePicker {
                    initial: self.theme.preset,
                    selected,
                });
            }
            C::Tribute(id) => {
                self.views.push(View::Tribute {
                    id,
                    step: self.easter_step,
                });
            }
            C::MicroTribute(kind) => {
                self.easter_step = self.easter_step.wrapping_add(1);
                if kind == MicroTributeKind::Boost {
                    let next_preset = if self.theme.preset == ThemePreset::AntigravityAurora {
                        ThemePreset::CyberMatrix
                    } else {
                        ThemePreset::AntigravityAurora
                    };
                    self.theme = self.theme.with_preset(next_preset);
                    self.config.theme = next_preset;
                }
                self.status = tributes::micro_message(kind, self.easter_step).to_owned();
            }
            C::Config => {
                self.config_note = None;
                self.views.push(View::Config { selected: 0 });
            }
            C::Help => self.views.push(View::Help),
            C::Exit => self.detach(),
            C::Quit => self.exit = Some(Exit::Quit),
        }
    }

    fn read_resources(&self, purpose: PickerPurpose) -> TmuxResult<Vec<Resource>> {
        match purpose {
            PickerPurpose::Windows => self
                .tmux
                .list_windows(None)
                .map(|rows| rows.into_iter().map(Resource::Window).collect()),
            PickerPurpose::Panes | PickerPurpose::Split => self
                .tmux
                .list_panes(None)
                .map(|rows| rows.into_iter().map(Resource::Pane).collect()),
            _ => self
                .recent_sessions()
                .map(|rows| rows.into_iter().map(Resource::Session).collect()),
        }
    }

    fn recent_sessions(&self) -> TmuxResult<Vec<Session>> {
        let mut sessions = self.tmux.list_sessions()?;
        sessions.sort_by(|a, b| {
            b.recency()
                .cmp(&a.recency())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(sessions)
    }

    fn refresh_home_data(&mut self) {
        let Some(sessions) = self.recent_sessions().ok() else {
            self.home_sessions.clear();
            self.home_windows.clear();
            self.home_panes.clear();
            self.telemetry = CachedTelemetry::default();
            self.radar_selected = 0;
            self.cached_preview = None;
            return;
        };
        if sessions.is_empty() {
            self.home_sessions = sessions;
            self.home_windows.clear();
            self.home_panes.clear();
            self.telemetry = CachedTelemetry {
                sessions: Some(0),
                windows: Some(0),
                attached: Some(0),
            };
            self.radar_selected = 0;
            self.cached_preview = None;
            return;
        }
        let (windows, window_count) = match self.tmux.list_windows(None).ok() {
            Some(list) => {
                let count = list.len();
                (list, count)
            }
            None => (Vec::new(), sessions.iter().map(|s| s.windows).sum()),
        };
        let panes = self.tmux.list_panes(None).ok().unwrap_or_default();
        self.telemetry = CachedTelemetry {
            sessions: Some(sessions.len()),
            windows: Some(window_count),
            attached: Some(sessions.iter().filter(|s| s.is_attached()).count()),
        };
        self.home_sessions = sessions;
        self.home_windows = windows;
        self.home_panes = panes;
        self.radar_selected = self
            .radar_selected
            .min(self.home_sessions.len().saturating_sub(1));
        if self.views.top().is_none() {
            self.refresh_home_preview();
        }
    }

    fn refresh_home_preview(&mut self) {
        if !self.config.live_preview {
            self.cached_preview = None;
            return;
        }
        let index = if self.composer_mode == ComposerMode::LiveRadar {
            self.radar_selected
        } else {
            0
        };
        let Some(session) = self.home_sessions.get(index) else {
            self.cached_preview = None;
            return;
        };
        let subtitle = self
            .home_windows
            .iter()
            .find(|w| w.session_id == session.id && w.active)
            .or_else(|| {
                self.home_windows
                    .iter()
                    .find(|w| w.session_id == session.id)
            })
            .map(|w| w.name.clone())
            .unwrap_or_else(|| format!("{}w", session.windows));
        let lines = self
            .tmux
            .capture_pane(&Target::Session(session.id.clone()), 12)
            .ok()
            .unwrap_or_default();
        self.cached_preview = Some((session.name.clone(), subtitle, lines));
    }

    fn refresh_radar_preview(&mut self) {
        self.refresh_home_preview();
    }

    fn update_picker_preview(&mut self, picker: &Picker) {
        if !self.config.live_preview {
            self.cached_preview = None;
            return;
        }
        let rows = picker.rows(&mut self.matcher);
        let Some(PickerRow::Resource(index)) = rows.get(picker.selected).copied() else {
            self.cached_preview = None;
            return;
        };
        let Some(resource) = picker.resources.get(index) else {
            self.cached_preview = None;
            return;
        };
        let target = resource.target();
        let title = resource.label();
        let subtitle = match resource {
            Resource::Session(s) => format!("{}w", s.windows),
            Resource::Window(w) => w.id.to_string(),
            Resource::Pane(p) => p.id.to_string(),
        };
        let lines = self.tmux.capture_pane(&target, 12).ok().unwrap_or_default();
        self.cached_preview = Some((title, subtitle, lines));
    }

    fn update_resume_preview(&mut self, picker: &ResumePicker) {
        if !self.config.live_preview {
            self.cached_preview = None;
            return;
        }
        let Some((target, label)) = picker.selected_target_info() else {
            self.cached_preview = None;
            return;
        };
        let lines = self.tmux.capture_pane(&target, 12).ok().unwrap_or_default();
        self.cached_preview = Some((label, String::new(), lines));
    }

    fn reload_resume(&mut self, picker: &mut ResumePicker) {
        let result = self.recent_sessions().and_then(|sessions| {
            let windows = if sessions.is_empty() {
                Vec::new()
            } else {
                self.tmux.list_windows(None)?
            };
            Ok((sessions, windows))
        });
        match result {
            Ok((sessions, windows)) => {
                self.telemetry = CachedTelemetry {
                    sessions: Some(sessions.len()),
                    windows: Some(windows.len()),
                    attached: Some(sessions.iter().filter(|s| s.is_attached()).count()),
                };
                self.home_sessions = sessions.clone();
                self.home_windows = windows.clone();
                picker.replace(sessions, windows, &mut self.matcher);
                self.update_resume_preview(picker);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn open_picker(&mut self, purpose: PickerPurpose) {
        self.open_picker_with_query(purpose, "");
    }

    fn open_picker_with_query(&mut self, purpose: PickerPurpose, initial_query: &str) {
        let resources = match self.read_resources(purpose) {
            Ok(resources) => resources,
            Err(error) => {
                self.status = error.to_string();
                Vec::new()
            }
        };
        let picker = Picker {
            purpose,
            query: TextInput::new(initial_query),
            resources,
            selected: 0,
        };
        self.update_picker_preview(&picker);
        self.views.push(View::Picker(picker));
    }

    fn picker_key(&mut self, mut picker: Picker, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                picker.selected = picker.selected.saturating_sub(1);
                self.update_picker_preview(&picker);
            }
            KeyCode::Down => {
                picker.selected = (picker.selected + 1)
                    .min(picker.rows(&mut self.matcher).len().saturating_sub(1));
                self.update_picker_preview(&picker);
            }
            KeyCode::Enter => {
                if let Some(row) = picker.rows(&mut self.matcher).get(picker.selected).copied() {
                    self.activate_picker_row(picker, row);
                    return;
                }
            }
            _ => {
                if picker.query.handle_key(key) {
                    picker.selected = 0;
                    self.update_picker_preview(&picker);
                }
            }
        }
        self.views.push(View::Picker(picker));
    }

    fn activate_picker_row(&mut self, mut picker: Picker, row: PickerRow) {
        match row {
            PickerRow::NewWindow => {
                self.views.push(View::Picker(picker));
                self.open_picker(PickerPurpose::NewWindow);
            }
            PickerRow::Refresh => {
                match self.read_resources(picker.purpose) {
                    Ok(resources) => {
                        picker.resources = resources;
                        picker.selected = 0;
                        self.refresh_home_data();
                        self.update_picker_preview(&picker);
                        self.status = "Refreshed".into();
                    }
                    Err(error) => self.status = error.to_string(),
                }
                self.views.push(View::Picker(picker));
            }
            PickerRow::Resource(index) => {
                let Some(resource) = picker.resources.get(index).cloned() else {
                    self.views.push(View::Picker(picker));
                    return;
                };
                let purpose = picker.purpose;
                self.views.push(View::Picker(picker));
                match purpose {
                    PickerPurpose::Rename => self.rename_form(resource),
                    PickerPurpose::Kill => self.kill_confirm(resource),
                    PickerPurpose::Split => self.views.push(View::Split(resource, 0)),
                    PickerPurpose::NewWindow => self.views.push(View::Form(Form {
                        action: FormAction::NewWindow(resource.session_id().clone()),
                        input: TextInput::default(),
                    })),
                    PickerPurpose::List | PickerPurpose::Windows | PickerPurpose::Panes => {
                        self.views.push(View::Actions(resource, 0));
                    }
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.mouse_scroll(true),
            MouseEventKind::ScrollDown => self.mouse_scroll(false),
            MouseEventKind::Down(MouseButton::Left) => {
                self.mouse_left_click(mouse.column, mouse.row)
            }
            _ => {}
        }
    }

    fn mouse_scroll(&mut self, up: bool) {
        match self.views.pop() {
            None => {
                if commands::popup_query(self.composer.value()).is_some() {
                    let count = self.palette().len();
                    if count > 0 {
                        self.palette_selected = if up {
                            self.palette_selected.saturating_sub(1)
                        } else {
                            (self.palette_selected + 1).min(count - 1)
                        };
                    }
                } else if self.active_mention().is_some() {
                    let count = self.mention_candidates().len();
                    if count > 0 {
                        self.mention_selected = if up {
                            self.mention_selected.saturating_sub(1)
                        } else {
                            (self.mention_selected + 1).min(count - 1)
                        };
                    }
                } else if self.composer_mode == ComposerMode::LiveRadar
                    && !self.home_sessions.is_empty()
                {
                    self.radar_selected = if up {
                        self.radar_selected.saturating_sub(1)
                    } else {
                        (self.radar_selected + 1).min(self.home_sessions.len() - 1)
                    };
                    self.refresh_radar_preview();
                }
            }
            Some(View::Picker(mut picker)) => {
                let count = picker.rows(&mut self.matcher).len();
                picker.selected = if up {
                    picker.selected.saturating_sub(1)
                } else {
                    (picker.selected + 1).min(count.saturating_sub(1))
                };
                self.update_picker_preview(&picker);
                self.views.push(View::Picker(picker));
            }
            Some(View::Resume(mut picker)) => {
                let code = if up { KeyCode::Up } else { KeyCode::Down };
                picker.handle_key(KeyEvent::new(code, KeyModifiers::NONE), &mut self.matcher);
                self.update_resume_preview(&picker);
                self.views.push(View::Resume(picker));
            }
            Some(View::Actions(resource, mut selected)) => {
                let count = resource.actions().len();
                selected = if up {
                    selected.saturating_sub(1)
                } else {
                    (selected + 1).min(count.saturating_sub(1))
                };
                self.views.push(View::Actions(resource, selected));
            }
            Some(View::Split(resource, _)) => {
                self.views.push(View::Split(resource, usize::from(!up)));
            }
            Some(View::ThemePicker {
                initial,
                mut selected,
            }) => {
                selected = if up {
                    selected.saturating_sub(1)
                } else {
                    (selected + 1).min(ThemePreset::ALL.len().saturating_sub(1))
                };
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            Some(View::Config { mut selected }) => {
                selected = if up {
                    selected.saturating_sub(1)
                } else {
                    (selected + 1).min(7)
                };
                self.views.push(View::Config { selected });
            }
            Some(View::Confirm(mut confirm)) => {
                confirm.scroll = if up {
                    confirm.scroll.saturating_sub(1)
                } else {
                    confirm.scroll.saturating_add(1)
                };
                self.views.push(View::Confirm(confirm));
            }
            Some(other) => self.views.push(other),
        }
    }

    fn mouse_left_click(&mut self, column: u16, row: u16) {
        match self.views.pop() {
            None => {
                if self.composer_mode == ComposerMode::LiveRadar
                    && let Some(rect) = self.radar_list_rect
                    && rect.contains((column, row).into())
                {
                    let clicked = self.radar_offset + usize::from(row - rect.y);
                    if let Some(session) = self.home_sessions.get(clicked) {
                        if self.radar_selected == clicked {
                            let id = session.id.clone();
                            self.status.clear();
                            self.enter(Target::Session(id));
                        } else {
                            self.radar_selected = clicked;
                            self.refresh_radar_preview();
                        }
                    }
                }
            }
            Some(View::Picker(mut picker)) => {
                if let Some(rect) = self.picker_list_rect
                    && rect.contains((column, row).into())
                {
                    let rows = picker.rows(&mut self.matcher);
                    let clicked = self.picker_offset + usize::from(row - rect.y);
                    if let Some(target_row) = rows.get(clicked).copied() {
                        if picker.selected == clicked {
                            self.activate_picker_row(picker, target_row);
                            return;
                        }
                        picker.selected = clicked;
                        self.update_picker_preview(&picker);
                    }
                }
                self.views.push(View::Picker(picker));
            }
            Some(other) => self.views.push(other),
        }
    }

    fn resource_action_key(&mut self, resource: Resource, mut selected: usize, key: KeyEvent) {
        let actions = resource.actions();
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(actions.len() - 1),
            KeyCode::Enter => {
                match actions[selected].1 {
                    ResourceAction::Open => self.enter(resource.target()),
                    ResourceAction::Rename => self.rename_form(resource),
                    ResourceAction::Close => self.kill_confirm(resource),
                }
                return;
            }
            _ => {}
        }
        self.views.push(View::Actions(resource, selected));
    }

    fn rename_form(&mut self, resource: Resource) {
        self.views.push(View::Form(Form {
            action: FormAction::Rename(resource),
            input: TextInput::default(),
        }));
    }

    fn kill_confirm(&mut self, resource: Resource) {
        let current = self.context.current_session() == Some(resource.session_id());
        let attached = matches!(&resource, Resource::Session(session) if session.attached > 0);
        let busy = match &resource {
            Resource::Pane(pane) => pane.busy,
            _ => self
                .tmux
                .list_panes(None)
                .map(|panes| {
                    panes.into_iter().any(|pane| {
                        pane.busy
                            && match &resource {
                                Resource::Session(session) => pane.session_id == session.id,
                                Resource::Window(window) => pane.window_id == window.id,
                                Resource::Pane(_) => false,
                            }
                    })
                })
                .unwrap_or(true),
        };
        let mut message = format!(
            "Permanently kill {}? All programs in the target will terminate.",
            resource.label()
        );
        if attached {
            message.push_str(" Attached clients will be affected.");
        }
        if busy {
            message.push_str(" A running task may be lost.");
        }
        if current {
            message
                .push_str(" This is in your current session; tmux may close this popup or client.");
        }
        message.push_str(" Type yes, then Enter to confirm. Empty input cancels.");
        self.views.push(View::Confirm(Confirm {
            resource,
            input: TextInput::default(),
            message,
            scroll: 0,
        }));
    }

    fn confirm_key(&mut self, mut confirm: Confirm, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::PageUp => {
                confirm.scroll = confirm.scroll.saturating_sub(1);
                self.views.push(View::Confirm(confirm));
                return;
            }
            KeyCode::Down | KeyCode::PageDown => {
                confirm.scroll = confirm.scroll.saturating_add(1);
                self.views.push(View::Confirm(confirm));
                return;
            }
            _ => {}
        }
        if key.code == KeyCode::Enter {
            if confirm.input.value() != "yes" {
                self.status = "Cancelled. Nothing was killed.".into();
                return;
            }
            let result = match &confirm.resource {
                Resource::Session(session) => self.tmux.kill_session(&session.id),
                Resource::Window(window) => self.tmux.kill_window(&window.id),
                Resource::Pane(pane) => self.tmux.kill_pane(&pane.id),
            };
            match result {
                Ok(()) => {
                    self.status = "Target killed".into();
                    self.refresh_parent();
                }
                Err(error) => {
                    self.status = error.to_string();
                    self.refresh_parent();
                }
            }
        } else {
            confirm.input.handle_key(key);
            self.views.push(View::Confirm(confirm));
        }
    }

    fn form_key(&mut self, mut form: Form, key: KeyEvent) {
        if key.code != KeyCode::Enter {
            form.input.handle_key(key);
            self.views.push(View::Form(form));
            return;
        }
        let value = form.input.value().to_owned();
        match &form.action {
            FormAction::NewSessionName => {
                if let Err(error) = crate::tmux::validate_session_name(&value) {
                    self.status = error.to_string();
                    self.views.push(View::Form(form));
                    return;
                }
                // Keep the name form beneath the directory form so Esc goes back.
                self.views.push(View::Form(form.clone()));
                self.views.push(View::Form(Form {
                    action: FormAction::NewSessionDirectory { name: value },
                    input: TextInput::default(),
                }));
            }
            FormAction::NewSessionDirectory { name } => {
                let name = name.clone();
                if !self.create_session(&name, &value) {
                    self.views.push(View::Form(form));
                }
            }
            FormAction::Rename(resource) => {
                let renamed_current = matches!(
                    &resource,
                    Resource::Session(session) if self.context.current_session() == Some(&session.id)
                );
                let result = match resource {
                    Resource::Session(session) => self.tmux.rename_session(&session.id, &value),
                    Resource::Window(window) => self.tmux.rename_window(&window.id, &value),
                    Resource::Pane(_) => return,
                };
                match result {
                    Ok(()) => {
                        if renamed_current && let Context::Inside(inside) = &mut self.context {
                            inside.session_name = Some(value);
                        }
                        self.status = "Renamed".into();
                        self.refresh_parent();
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
            FormAction::NewWindow(session) => {
                let name = if value.is_empty() {
                    None
                } else {
                    Some(value.as_str())
                };
                match self.tmux.new_window(session, name, None) {
                    Ok(window) => {
                        self.refresh_home_data();
                        self.status = "Window created".into();
                        self.views = ViewStack::default();
                        self.open_picker(PickerPurpose::Windows);
                        self.views.push(View::Created(Resource::Window(window)));
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
        }
    }

    fn create_session(&mut self, name: &str, directory: &str) -> bool {
        let path = if directory.is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            PathBuf::from(directory)
        };
        match self.tmux.new_session(name, &path) {
            Ok(session) => {
                self.refresh_home_data();
                self.status = "Session created. Enter to enter it, or Esc to keep managing.".into();
                self.views = ViewStack::default();
                self.open_picker(PickerPurpose::List);
                self.views.push(View::Created(Resource::Session(session)));
                true
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        }
    }

    fn refresh_parent(&mut self) {
        self.refresh_home_data();
        let popped = self.views.pop();
        let picker = match popped {
            Some(View::Resume(mut picker)) => {
                self.reload_resume(&mut picker);
                self.views.push(View::Resume(picker));
                return;
            }
            Some(View::Picker(picker)) => Some(picker),
            Some(_) => match self.views.pop() {
                Some(View::Picker(picker)) => Some(picker),
                Some(other) => {
                    self.views.push(other);
                    None
                }
                None => None,
            },
            None => None,
        };
        if let Some(mut picker) = picker {
            match self.read_resources(picker.purpose) {
                Ok(resources) => {
                    picker.resources = resources;
                    picker.selected = picker
                        .selected
                        .min(picker.rows(&mut self.matcher).len().saturating_sub(1));
                }
                Err(error) => self.status = error.to_string(),
            }
            self.update_picker_preview(&picker);
            self.views.push(View::Picker(picker));
        }
    }

    fn detach(&mut self) {
        self.context = self.tmux.context();
        match &self.context {
            Context::Outside => self.exit = Some(Exit::Quit),
            Context::Inside(InsideContext {
                client_name: Some(client),
                ..
            }) => {
                self.exit = Some(Exit::Detach(client.clone()));
            }
            Context::ForeignServer => {
                self.status = "This is a different tmux server. Use tmux's detach key.".into()
            }
            _ => {
                self.status =
                    "Cannot identify this tmux client. Use the Popup or tmux's detach key.".into()
            }
        }
    }

    fn enter(&mut self, target: Target) {
        self.context = self.tmux.context();
        // A selected session may disappear while the picker is open.
        if let Target::Session(id) = &target {
            match self.tmux.has_session(id) {
                Ok(true) => {}
                Ok(false) => {
                    self.status =
                        "That session no longer exists. The list has been refreshed.".into();
                    self.refresh_parent();
                    return;
                }
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            }
        }
        match &self.context {
            Context::Inside(_) => match self.tmux.switch_client(&target) {
                Ok(()) => self.exit = Some(Exit::Switched),
                Err(error) => { self.status = error.to_string(); self.refresh_parent(); }
            },
            Context::ForeignServer => self.status = "You are inside another tmux server. Open jm outside tmux to enter this server; management still works.".into(),
            Context::Unavailable => self.status = "The launching tmux client is unavailable. Return to a live tmux client or open jm outside tmux; no nested client was started.".into(),
            Context::Outside => match self.tmux.select(&target) {
                Ok(()) => self.exit = Some(Exit::Attach(target)),
                Err(error) => { self.status = error.to_string(); self.refresh_parent(); }
            },
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        self.picker_list_rect = None;
        self.radar_list_rect = None;
        let area = frame.area();
        let indent = if area.width >= 24 { 2 } else { 0 };
        let area = Rect {
            x: area.x + indent,
            width: area.width - indent,
            ..area
        };
        if self.views.top().is_none() {
            if self.composer_mode == ComposerMode::LiveRadar
                && !self.home_sessions.is_empty()
                && area.width >= 36
                && area.height >= 12
            {
                let [welcome, _, _] = Layout::vertical([
                    Constraint::Min(0),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .areas(area);
                let raw_inner = Block::default().borders(Borders::ALL).inner(welcome);
                let inner = if raw_inner.width >= 6 {
                    Rect {
                        x: raw_inner.x + 1,
                        width: raw_inner.width - 2,
                        ..raw_inner
                    }
                } else {
                    raw_inner
                };
                let left = if inner.width >= 54 {
                    Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
                        .areas::<2>(inner)[0]
                } else {
                    inner
                };
                if left.height > 1 && left.width > 0 {
                    let visible_rows = usize::from(left.height.saturating_sub(1)).max(1);
                    let selected = self.radar_selected.min(self.home_sessions.len() - 1);
                    self.radar_offset = (selected + 1).saturating_sub(visible_rows);
                    self.radar_list_rect = Some(Rect {
                        x: left.x,
                        y: left.y + 1,
                        width: left.width,
                        height: left.height - 1,
                    });
                }
            }
            let commands = self.palette();
            let items: Vec<_> = commands
                .iter()
                .map(|candidate| CommandItem {
                    name: candidate.command.name,
                    description: candidate.command.description,
                    alias: candidate.alias,
                    indices: &candidate.indices,
                })
                .collect();
            let mention_candidates = if commands::popup_query(self.composer.value()).is_none() {
                self.mention_candidates()
            } else {
                Vec::new()
            };
            let mention_items: Vec<_> = mention_candidates
                .iter()
                .map(|candidate| ui::MentionItem {
                    token: &candidate.token,
                    kind_badge: candidate.kind_badge,
                    detail: &candidate.detail,
                    indices: &candidate.indices,
                })
                .collect();
            let current_session = self.context.current_session().cloned();
            let session_entries: Vec<HomeSessionEntry> = self
                .home_sessions
                .iter()
                .map(|s| {
                    let active_window = self
                        .home_windows
                        .iter()
                        .find(|w| w.session_id == s.id && w.active)
                        .or_else(|| self.home_windows.iter().find(|w| w.session_id == s.id))
                        .map(|w| w.name.clone())
                        .unwrap_or_default();
                    HomeSessionEntry {
                        name: s.name.clone(),
                        windows: s.windows,
                        attached: s.is_attached(),
                        current: current_session.as_ref() == Some(&s.id),
                        active_window,
                        badge: tributes::session_badge(&s.name, self.theme.ascii),
                    }
                })
                .collect();
            let preview = self
                .cached_preview
                .as_ref()
                .filter(|_| self.config.live_preview)
                .map(|(title, subtitle, lines)| ui::PanePreview {
                    title: title.as_str(),
                    subtitle: subtitle.as_str(),
                    lines: lines.as_slice(),
                });
            let (context, session) = match &self.context {
                Context::Outside => ("standalone", None),
                Context::Inside(inside) => (
                    if inside.popup { "popup" } else { "pane" },
                    inside.session_name.as_deref(),
                ),
                Context::ForeignServer => ("different server", None),
                Context::Unavailable => ("client unavailable", None),
            };
            let badge = format!(
                "{} {}",
                self.theme.preset.badge_icon(self.theme.ascii),
                self.theme.preset.slug()
            );
            ui::render_home_studio(
                frame,
                area,
                &self.composer,
                &items,
                self.palette_selected,
                HomeDetails {
                    directory: self.directory.as_deref(),
                    context,
                    session,
                    status: (!self.status.is_empty()).then_some(self.status.as_str()),
                },
                HomeTelemetry {
                    sessions: self.telemetry.sessions,
                    windows: self.telemetry.windows,
                    attached: self.telemetry.attached,
                    hint_badge: (self.theme.preset != ThemePreset::Classic)
                        .then_some(badge.as_str()),
                },
                ui::HomeDashboardState {
                    mode: self.composer_mode,
                    sessions: &session_entries,
                    mentions: &mention_items,
                    mention_selected: self.mention_selected,
                    radar_selected: self.radar_selected,
                    preview,
                    tip_step: self.tip_step,
                },
                self.theme,
            );
            return;
        }
        let preview_lines: Option<Vec<String>> = self
            .cached_preview
            .as_ref()
            .map(|(_, _, lines)| lines.clone());
        let preview_meta = self
            .cached_preview
            .as_ref()
            .map(|(title, subtitle, _)| (title.clone(), subtitle.clone()));
        if let Some(View::Resume(picker)) = self.views.0.last_mut() {
            if let Some(lines) = preview_lines.as_deref() {
                picker.render_with_preview(
                    frame,
                    area,
                    self.context.current_session(),
                    &self.status,
                    Some(lines),
                    self.theme,
                );
            } else {
                picker.render(
                    frame,
                    area,
                    self.context.current_session(),
                    &self.status,
                    self.theme,
                );
            }
            return;
        }
        let chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);
        let main = chunks[0];
        match self.views.0.last_mut() {
            Some(View::Picker(picker)) => {
                let rows = picker.rich_rows(&mut self.matcher, &self.context, self.theme);
                let preview = preview_meta.as_ref().zip(preview_lines.as_deref()).map(
                    |((title, subtitle), lines)| ui::PanePreview {
                        title: title.as_str(),
                        subtitle: subtitle.as_str(),
                        lines,
                    },
                );
                let [_, _, body_area, _] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .areas(main);
                let list_area = match &preview {
                    Some(_) if main.width >= 68 && body_area.height >= 4 => {
                        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
                            .areas::<2>(body_area)[0]
                    }
                    _ => body_area,
                };
                if !rows.is_empty() && list_area.height > 0 && list_area.width > 0 {
                    let visible_rows = usize::from(list_area.height);
                    let selected = picker.selected.min(rows.len() - 1);
                    self.picker_offset = (selected + 1).saturating_sub(visible_rows);
                    self.picker_list_rect = Some(list_area);
                }
                ui::render_rich_picker_with_preview(
                    frame,
                    main,
                    picker.purpose.title(),
                    &picker.query,
                    &rows,
                    picker.selected,
                    match picker.purpose {
                        PickerPurpose::Windows => "No matching windows",
                        PickerPurpose::NewWindow => {
                            "No matching sessions. Use /new to create a session."
                        }
                        _ => "No matches or sessions. Esc back · /new creates a session",
                    },
                    picker.purpose.footer(),
                    preview,
                    self.theme,
                );
            }
            Some(View::Actions(resource, selected)) => {
                let actions = resource.actions();
                let [title, target, choices, footer] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(u16::from(main.height >= actions.len() as u16 + 3)),
                    Constraint::Min(1),
                    Constraint::Length(u16::from(main.height >= actions.len() as u16 + 2)),
                ])
                .areas(main);
                frame.render_widget(
                    Paragraph::new(match resource {
                        Resource::Session(_) => "Session actions",
                        Resource::Window(_) => "Window actions",
                        Resource::Pane(_) => "Pane actions",
                    })
                    .style(
                        self.theme
                            .style(self.theme.accent)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    ),
                    title,
                );
                frame.render_widget(
                    Paragraph::new(resource.label()).style(self.theme.style(self.theme.muted)),
                    target,
                );
                let rows = actions.iter().map(|(label, _)| ListItem::new(*label));
                let mut state = ListState::default().with_selected(Some(*selected));
                frame.render_stateful_widget(
                    List::new(rows)
                        .highlight_style(self.theme.selected_style())
                        .highlight_symbol(format!("{} ", self.theme.marker())),
                    choices,
                    &mut state,
                );
                frame.render_widget(
                    Paragraph::new(self.theme.adapt("Enter Select · Esc Back"))
                        .style(self.theme.style(self.theme.muted)),
                    footer,
                );
            }
            Some(View::Form(form)) => {
                let rename_label;
                let (title, label) = match &form.action {
                    FormAction::NewSessionName => ("New session", "Session name"),
                    FormAction::NewSessionDirectory { .. } => (
                        "New session",
                        "Working directory (empty = current directory)",
                    ),
                    FormAction::Rename(resource) => {
                        rename_label = format!("New name for {}", resource.label());
                        ("Rename", rename_label.as_str())
                    }
                    FormAction::NewWindow(_) => ("New window", "Window name (optional)"),
                };
                ui::render_form(
                    frame,
                    main,
                    title,
                    label,
                    &form.input,
                    "Enter Submit · Esc Back",
                    self.theme,
                );
            }
            Some(View::Confirm(confirm)) => {
                let summary = confirm.resource.label();
                confirm.scroll = ui::render_confirm_card(
                    frame,
                    main,
                    "Confirm irreversible action",
                    &summary,
                    &confirm.message,
                    &confirm.input,
                    "↑↓ Review · Type yes + Enter to kill · Esc Back",
                    self.theme,
                    confirm.scroll,
                );
            }
            Some(View::Created(resource)) => {
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from("Created successfully"),
                        Line::from(""),
                        Line::from(resource.label()),
                        Line::from(""),
                        Line::from("Enter to enter native tmux · Esc to continue managing"),
                    ]),
                    main,
                );
            }
            Some(View::Split(resource, selected)) => {
                ui::render_split_selector(frame, main, &resource.label(), *selected, self.theme);
            }
            Some(View::ThemePicker { initial, selected }) => {
                ui::render_theme_picker(
                    frame,
                    main,
                    ThemePreset::ALL,
                    *selected,
                    *initial,
                    self.theme,
                );
            }
            Some(View::Config { selected }) => {
                let selected = *selected;
                let items = self.config_modal_items();
                let path_label = self
                    .config_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| config::resolve_config_path(None).map(|p| p.display().to_string()))
                    .unwrap_or_else(|| format!("~/{}", config::DEFAULT_CONFIG_REL_PATH));
                ui::render_config_modal(
                    frame,
                    main,
                    &path_label,
                    &items,
                    selected,
                    self.config_note.as_deref(),
                    self.theme,
                );
            }
            Some(View::Tribute { id, step }) => {
                let card = tributes::card(*id, *step);
                ui::render_tribute_modal(
                    frame,
                    main,
                    ui::TributeModal {
                        badge: card.badge,
                        title: card.title,
                        subtitle: card.subtitle,
                        telemetry: card.telemetry,
                        art: if self.theme.ascii {
                            card.ascii_art
                        } else {
                            card.art
                        },
                        highlights: card.highlights,
                        status_quip: &card.status_quip,
                        footer_hint: card.footer_hint,
                    },
                    self.theme,
                );
            }
            Some(View::Help) => ui::render_help(frame, main, self.theme),
            Some(View::Resume(_)) | None => {}
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(format!("{} ", self.theme.marker())),
                Span::styled(self.composer.value(), self.theme.style(self.theme.muted)),
            ])),
            chunks[1],
        );
        let status_style = if self.status.starts_with("Refreshed") || self.status.starts_with('✦')
        {
            self.theme.style(self.theme.success)
        } else {
            self.theme.style(self.theme.error)
        };
        frame.render_widget(
            Paragraph::new(self.theme.adapt(&self.status)).style(status_style),
            chunks[2],
        );
    }

    pub fn exit(&self) -> Option<&Exit> {
        self.exit.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, name: &str) -> Session {
        Session {
            id: SessionId::new(id).expect("valid test id"),
            name: name.into(),
            windows: 2,
            attached: 0,
            created: 1,
            last_attached: None,
            activity: 1,
            path: "/tmp".into(),
        }
    }

    #[test]
    fn picker_filters_and_sorts_fuzzy_matches() {
        let picker = Picker {
            purpose: PickerPurpose::List,
            query: TextInput::new("ori"),
            resources: vec![
                Resource::Session(session("$1", "server")),
                Resource::Session(session("$2", "orion")),
                Resource::Session(session("$3", "work")),
            ],
            selected: 0,
        };
        let rows = picker.rows(&mut FuzzyMatcher::new());
        let [PickerRow::Resource(index)] = rows.as_slice() else {
            panic!("expected one matching session: {rows:?}");
        };
        assert_eq!(picker.resources[*index].label(), "orion");
    }

    #[test]
    fn stack_pop_returns_top_view_only() {
        let mut stack = ViewStack::default();
        stack.push(View::Help);
        stack.push(View::Picker(Picker {
            purpose: PickerPurpose::List,
            query: TextInput::default(),
            resources: vec![],
            selected: 0,
        }));
        assert!(matches!(stack.pop(), Some(View::Picker(_))));
        assert!(matches!(stack.top(), Some(View::Help)));
    }

    #[test]
    fn confirmation_scroll_keeps_focus_and_escape_restores_parent() {
        let tmux = crate::tmux::TmuxClient::new(crate::tmux::SocketSpec::Name(format!(
            "jm-confirm-test-{}",
            std::process::id()
        )))
        .with_program("/nonexistent-justmux-test-tmux");
        let mut app = App::new(&tmux, AppOptions::default());
        app.views.push(View::Help);
        app.views.push(View::Confirm(Confirm {
            resource: Resource::Session(session("$1", "orion")),
            input: TextInput::default(),
            message: "An irreversible action".into(),
            scroll: 0,
        }));
        for code in [
            KeyCode::Char('y'),
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::Char('e'),
            KeyCode::Char('s'),
        ] {
            app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        let Some(View::Confirm(confirm)) = app.views.top() else {
            panic!("confirmation lost focus")
        };
        assert_eq!(confirm.input.value(), "yes");
        assert_eq!(confirm.scroll, 2);
        assert!(app.composer.value().is_empty());
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(matches!(app.views.top(), Some(View::Help)));
        assert!(app.exit().is_none());
    }

    #[test]
    fn empty_confirmation_cancels_without_invoking_tmux() {
        let tmux = crate::tmux::TmuxClient::new(crate::tmux::SocketSpec::Name(format!(
            "jm-confirm-cancel-{}",
            std::process::id()
        )))
        .with_program("/nonexistent-justmux-test-tmux");
        let mut app = App::new(&tmux, AppOptions::default());
        app.views.push(View::Confirm(Confirm {
            resource: Resource::Session(session("$1", "orion")),
            input: TextInput::default(),
            message: "An irreversible action".into(),
            scroll: 0,
        }));
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.status, "Cancelled. Nothing was killed.");
        assert!(app.views.top().is_none());
        assert!(app.exit().is_none());
    }
}
