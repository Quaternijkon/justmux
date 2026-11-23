//! Application state and keyboard focus. Views never handle keys below the top
//! of the stack; all tmux mutations happen here, outside the rendering layer.
mod controller;
mod desktop;
mod resume;
mod split;

use std::{
    collections::{HashMap, HashSet},
    env,
    path::PathBuf,
};

use controller::{
    BuffersViewState, ClientsViewState, ConfirmTarget, DualForm, LayoutStudioState,
    PreviewSearchState, SnapshotsViewState, TargetPickState, build_resource_actions,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use desktop::{DesktopState, ExposeState, HoverTarget};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use resume::{MatrixHitTarget, MatrixSelection, ResumePicker};
use split::{SlotId, SplitNode, SplitSlotFill, SplitStudio, SplitToolbarAction, SplitWindowOption};

use crate::{
    commands::{self, CommandKind, CommandMatch, ParsedCommand},
    config::{self, Config},
    fuzzy::{FuzzyMatcher, MatchKind},
    snapshot::WorkspaceSnapshot,
    terminal::ScreenMode,
    tmux::{
        Context, InsideContext, LayoutPreset, Pane, PaneId, Session, SessionId, SplitDirection,
        Target, Tmux, TmuxResult, Window, WindowId,
    },
    tmux_conf::{HISTORY_LIMIT_PRESETS, TmuxComfortOptions},
    tributes::{self, MicroTributeKind, TributeId},
    ui::{
        self, BufferRowView, BuffersView, ClientRowView, ClientsView, CommandItem, ComposerMode,
        ConfigItemView, ControllerAction, DesktopMenubarState, EmblemMode, HomeDetails,
        HomeQuickAction, HomeSessionEntry, HomeTelemetry, LayoutStudioView, MatrixAction,
        PreviewAction, SashId, SnapshotRowView, SnapshotsView, SplitCanvasNode,
        SplitCanvasPaneView, SplitStudioView, SplitToolbarButton, SplitWindowRowView,
        SubviewContent, TargetScope, TextInput, Theme, ThemePreset, ToastKind, WorkspaceHomeSpec,
        WorkspaceSubviewSpec, render_workspace_home, render_workspace_subview,
    },
};

const PREVIEW_CAPTURE_LINES: usize = 400;
const PREVIEW_WHEEL_STEP: usize = 3;

/// Advance an index within `0..len` by `delta` with circular wrap-around.
/// Returns `0` when `len == 0`.
pub(crate) fn wrap_step(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let cur = current.min(len - 1);
    let step = delta.rem_euclid(len as isize) as usize;
    (cur + step) % len
}

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
    AttachReadOnly(Target),
    Detach(String),
    Switched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerPurpose {
    Panes,
}
impl PickerPurpose {
    fn title_for(self, lang: ui::Language) -> &'static str {
        match self {
            Self::Panes => {
                if lang.is_zh() {
                    "窗格列表"
                } else {
                    "Panes"
                }
            }
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
    fn actions_for(
        &self,
        piping: &HashSet<PaneId>,
        lang: ui::Language,
    ) -> Vec<(String, ResourceAction)> {
        build_resource_actions(self, piping, lang)
    }
    fn rich_row(
        &self,
        primary_indices: Vec<usize>,
        context: &Context,
        theme: Theme,
    ) -> ui::RichPickerRow {
        let ascii = theme.ascii;
        let zh = theme.language.is_zh() && !ascii;
        match self {
            Self::Session(s) => {
                let is_current = context.current_session() == Some(&s.id);
                let badge = if is_current {
                    Some((
                        if ascii {
                            "* current"
                        } else if zh {
                            "★ 当前"
                        } else {
                            "★ current"
                        }
                        .to_owned(),
                        theme.accent,
                    ))
                } else if s.attached > 0 {
                    Some((
                        if ascii {
                            "* attached"
                        } else if zh {
                            "● 已附着"
                        } else {
                            "● attached"
                        }
                        .to_owned(),
                        theme.success,
                    ))
                } else {
                    Some((
                        if ascii {
                            "o detached"
                        } else if zh {
                            "○ 未附着"
                        } else {
                            "○ detached"
                        }
                        .to_owned(),
                        theme.muted,
                    ))
                };
                let win_label = if zh {
                    format!("{} 个窗口", s.windows)
                } else {
                    format!(
                        "{} window{}",
                        s.windows,
                        if s.windows == 1 { "" } else { "s" }
                    )
                };
                let mut parts = Vec::new();
                if let Some(tag) = tributes::session_badge(&s.name, ascii) {
                    parts.push(tag.to_owned());
                }
                parts.push(win_label);
                if is_current && s.attached > 0 {
                    parts.push(if zh { "已附着" } else { "attached" }.to_owned());
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
                let badge = if w.synchronized {
                    Some((
                        if ascii {
                            "[!SYNC]"
                        } else if zh {
                            "⚡同步"
                        } else {
                            "⚡SYNC"
                        }
                        .to_owned(),
                        theme.warning,
                    ))
                } else if w.zoomed {
                    Some((
                        if ascii { "[Z]" } else { "⛶ zoom" }.to_owned(),
                        theme.accent,
                    ))
                } else if w.active {
                    Some((
                        if ascii {
                            "* active"
                        } else if zh {
                            "★ 当前"
                        } else {
                            "★ active"
                        }
                        .to_owned(),
                        theme.accent,
                    ))
                } else {
                    None
                };
                ui::RichPickerRow {
                    primary: self.label(),
                    primary_indices,
                    secondary: if zh {
                        format!("{} 个窗格 · {}", w.panes, w.id)
                    } else {
                        format!(
                            "{} pane{} · {}",
                            w.panes,
                            if w.panes == 1 { "" } else { "s" },
                            w.id
                        )
                    },
                    badge,
                    is_action: false,
                }
            }
            Self::Pane(p) => {
                let badge = if p.dead {
                    Some((
                        if ascii {
                            "[X] dead"
                        } else if zh {
                            "✕ 已退出"
                        } else {
                            "✕ dead"
                        }
                        .to_owned(),
                        theme.error,
                    ))
                } else if p.zoomed {
                    Some((
                        if ascii {
                            "[Z] zoom"
                        } else if zh {
                            "⛶ 缩放"
                        } else {
                            "⛶ zoom"
                        }
                        .to_owned(),
                        theme.accent,
                    ))
                } else if p.synchronized {
                    Some((
                        if ascii {
                            "[!SYNC]"
                        } else if zh {
                            "⚡同步"
                        } else {
                            "⚡SYNC"
                        }
                        .to_owned(),
                        theme.warning,
                    ))
                } else if p.busy {
                    Some((
                        if ascii {
                            "! busy"
                        } else if zh {
                            "⚡ 运行中"
                        } else {
                            "⚡ busy"
                        }
                        .to_owned(),
                        theme.warning,
                    ))
                } else if p.active {
                    let label = if ascii {
                        theme.tr("* active", "* 当前")
                    } else {
                        theme.tr("★ active", "★ 当前")
                    };
                    Some((label.to_owned(), theme.accent))
                } else {
                    None
                };
                let title_part = if !p.title.trim().is_empty() && p.title != p.current_command {
                    format!(" · {}", p.title)
                } else {
                    String::new()
                };
                ui::RichPickerRow {
                    primary: self.label(),
                    primary_indices,
                    secondary: format!(
                        "{} ({}x{}){} · {}",
                        p.id, p.width, p.height, title_part, p.current_path
                    ),
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

        let mut rows: Vec<(PickerRow, Vec<usize>)> = matched_rows
            .into_iter()
            .map(|(_, i, indices)| (PickerRow::Resource(i), indices))
            .collect();
        if self.query.value().is_empty() && matches!(self.purpose, PickerPurpose::Panes) {
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
                PickerRow::Refresh => ui::RichPickerRow {
                    primary: theme.tr("Refresh list", "刷新列表").into(),
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
    RenamePaneTitle(Pane),
    SendCommandToPane(Pane),
    NewBufferContent,
    SaveSnapshotName { session_filter: Option<SessionId> },
}
#[derive(Debug, Clone)]
struct Form {
    action: FormAction,
    input: TextInput,
}
#[derive(Debug, Clone)]
struct Confirm {
    target: ConfirmTarget,
    input: TextInput,
    message: String,
    scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceAction {
    Open,
    AttachReadOnly,
    NewWindow,
    Rename,
    DetachOthers,
    SaveSnapshot,
    Close,
    EditInSplitStudio,
    LayoutAndResize,
    ToggleSyncPanes,
    CycleMonitor,
    MoveWindowToSession,
    LinkWindowToSession,
    ReorderWindowLeft,
    ReorderWindowRight,
    ToggleZoomPane,
    SplitPaneRight,
    SplitPaneDown,
    BreakPaneOut,
    SwapPaneWith,
    JoinPaneInto,
    CopyPaneOutput,
    ExportPaneLog,
    TogglePipePane,
    SendCtrlC,
    SendCommand,
    ClearPaneHistory,
    RespawnPane,
    RenamePaneTitle,
}

#[derive(Debug, Clone)]
enum View {
    Picker(Picker),
    TargetPick(TargetPickState),
    Actions(Resource, usize),
    Form(Form),
    DualForm(DualForm),
    Confirm(Confirm),
    Created(Resource),
    Split(SplitStudio),
    LayoutStudio(LayoutStudioState),
    Expose(ExposeState),
    Clients(ClientsViewState),
    Buffers(BuffersViewState),
    Snapshots(SnapshotsViewState),
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

const CONFIG_ITEM_COUNT: usize = 18;

fn theme_toast(preset: ThemePreset, lang: ui::Language) -> String {
    if lang.is_zh() {
        format!("✦ 已切换主题：{} ({})", preset.name(), preset.origin())
    } else {
        format!("✦ Theme: {} ({})", preset.name(), preset.origin())
    }
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
    tmux_conf_path: Option<PathBuf>,
    popup_fallback_exe: Option<PathBuf>,
    context: Context,
    telemetry: CachedTelemetry,
    client_count: Option<usize>,
    home_sessions: Vec<Session>,
    home_windows: Vec<Window>,
    home_panes: Vec<Pane>,
    matrix: ResumePicker,
    matrix_action: MatrixAction,
    matrix_hits: Vec<resume::MatrixHit>,
    tab_hits: [(Rect, MatrixAction); 4],
    quick_action_hits: Vec<(Rect, HomeQuickAction)>,
    preview_action_hits: Vec<(Rect, PreviewAction)>,
    controller_hits: Vec<(Rect, ControllerAction)>,
    subview_list_rect: Option<Rect>,
    composer_mode: ComposerMode,
    radar_selected: usize,
    mention_selected: usize,
    tip_step: usize,
    cached_preview: Option<(String, String, Vec<String>)>,
    preview_scroll: usize,
    preview_rect: Option<Rect>,
    preview_target: Option<Target>,
    home_preview_pane_idx: usize,
    home_preview_pane_window: Option<WindowId>,
    home_preview_pane_total: usize,
    preview_search: PreviewSearchState,
    piping_panes: HashSet<PaneId>,
    picker_list_rect: Option<Rect>,
    picker_offset: usize,
    radar_list_rect: Option<Rect>,
    radar_offset: usize,
    emblem_step: usize,
    hydra_visible: bool,
    easter_step: usize,
    desktop: DesktopState,
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
            tmux_conf_path: None,
            popup_fallback_exe: None,
            context: tmux.context(),
            telemetry: CachedTelemetry::default(),
            client_count: None,
            home_sessions: Vec::new(),
            home_windows: Vec::new(),
            home_panes: Vec::new(),
            matrix: ResumePicker::with_create_actions(),
            matrix_action: MatrixAction::Open,
            matrix_hits: Vec::new(),
            tab_hits: Default::default(),
            quick_action_hits: Vec::new(),
            preview_action_hits: Vec::new(),
            controller_hits: Vec::new(),
            subview_list_rect: None,
            composer_mode,
            radar_selected: 0,
            mention_selected: 0,
            tip_step: 0,
            cached_preview: None,
            preview_scroll: 0,
            preview_rect: None,
            preview_target: None,
            home_preview_pane_idx: 0,
            home_preview_pane_window: None,
            home_preview_pane_total: 0,
            preview_search: PreviewSearchState::default(),
            piping_panes: HashSet::new(),
            picker_list_rect: None,
            picker_offset: 0,
            radar_list_rect: None,
            radar_offset: 0,
            emblem_step: 0,
            hydra_visible: false,
            easter_step: 0,
            desktop: DesktopState::default(),
            exit: None,
        };
        app.refresh_home_data();
        if let Some(command) = options.initial_command {
            app.execute_text(&format!("/{}", command.trim_start_matches('/')));
        }
        app
    }

    /// Configure the user `tmux.conf` path and auto-detect the active executable
    /// for opt-in popup keybind management in `/config`.
    pub fn with_tmux_conf_path(self, path: Option<PathBuf>) -> Self {
        self.with_tmux_conf(path, crate::tmux_conf::detect_fallback_exe())
    }

    /// Configure an explicit `tmux.conf` path and optional fallback executable
    /// for opt-in popup keybind & comfort management in `/config`.
    pub fn with_tmux_conf(mut self, path: Option<PathBuf>, fallback_exe: Option<PathBuf>) -> Self {
        if let Some(ref conf_path) = path {
            if let Ok(installed) = crate::tmux_conf::is_installed(conf_path) {
                self.config.tmux_popup = installed;
            }
            if let Ok(comfort) = crate::tmux_conf::read_comfort_block(conf_path) {
                self.config.set_tmux_comfort_options(comfort);
            }
        }
        self.tmux_conf_path = path;
        self.popup_fallback_exe = fallback_exe;
        self
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

    pub fn set_startup_tip(&mut self, tip: impl Into<String>) {
        if self.status.is_empty() && self.views.top().is_none() {
            self.status = tip.into();
        }
    }

    pub fn emblem_step(&self) -> usize {
        self.emblem_step
    }

    pub fn should_animate(&self) -> bool {
        self.config.emblem_mode.should_animate() && self.hydra_visible
    }

    pub fn tick_animation(&mut self) {
        self.emblem_step = self.emblem_step.wrapping_add(1);
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        // Live PIP direct type-through MUST intercept keys & paste BEFORE Ctrl+C / Ctrl+P.
        if let Some(changed) = self.try_handle_live_pip_event(&event) {
            return changed;
        }
        if let Event::Resize(_, _) = event {
            return true;
        }
        if let Event::Paste(text) = event {
            if let Some(rename) = self.desktop.inline_rename.as_mut() {
                rename.input.insert_str(&text);
                return true;
            }
            self.paste(&text);
            return true;
        }
        if let Event::Mouse(mouse) = event {
            return if self.config.mouse {
                self.handle_desktop_mouse(mouse)
            } else {
                false
            };
        }
        let Event::Key(key) = event else {
            return false;
        };
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.desktop.context_menu = None;
            self.desktop.inline_rename = None;
            self.exit = Some(Exit::Quit);
            return true;
        }
        if self.desktop.inline_rename.is_some() {
            return self.handle_inline_rename_key(key);
        }
        if self.desktop.context_menu.is_some() && self.handle_context_menu_key(key) {
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('p' | 'P') => {
                    self.open_universal_palette();
                    return true;
                }
                KeyCode::Char('k' | 'K') if self.is_non_text_context() => {
                    self.open_universal_palette();
                    return true;
                }
                KeyCode::Char('g' | 'G') => {
                    self.open_expose_view();
                    return true;
                }
                KeyCode::Char('i' | 'I') => {
                    self.toggle_live_pip(None);
                    return true;
                }
                _ => {}
            }
        }
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            if key.code == KeyCode::F(3) {
                self.open_expose_view();
                return true;
            }
            if key.code == KeyCode::F(4) {
                self.toggle_live_pip(None);
                return true;
            }
            if key.code == KeyCode::F(2)
                && !self.preview_search.active
                && !matches!(
                    self.views.top(),
                    Some(View::Form(_) | View::DualForm(_) | View::Confirm(_))
                )
                && self.open_inline_rename_for_focused()
            {
                return true;
            }
        }
        if self.handle_preview_search_key(key) {
            return true;
        }
        if key.code == KeyCode::Esc {
            if !self.preview_search.active && self.preview_search.query_str().is_some() {
                self.preview_search.reset();
                self.status.clear();
                return true;
            }
            match self.views.pop() {
                Some(View::ThemePicker { initial, .. }) => {
                    self.theme = self.theme.with_preset(initial);
                }
                Some(View::Config { .. }) => {
                    self.config_note = None;
                }
                Some(_) => {}
                None => {
                    if !self.composer.value().is_empty() {
                        self.composer.set_value("");
                        self.palette_selected = 0;
                        self.mention_selected = 0;
                        self.status.clear();
                        self.sync_home_matrix();
                        self.refresh_home_preview();
                    } else if self.matrix_action != MatrixAction::Open {
                        self.matrix_action = MatrixAction::Open;
                        self.status.clear();
                    } else {
                        self.status.clear();
                        self.exit = Some(Exit::Quit);
                    }
                    return true;
                }
            }
            self.status.clear();
            self.sync_top_view_after_pop();
            return true;
        }
        // Take the top view out while handling it to make nested pushes explicit.
        match self.views.pop() {
            None => self.composer_key(key),
            Some(View::Picker(picker)) => self.picker_key(picker, key),
            Some(View::TargetPick(state)) => self.target_pick_key(state, key),
            Some(View::Actions(resource, selected)) => {
                self.resource_action_key(resource, selected, key)
            }
            Some(View::Form(form)) => self.form_key(form, key),
            Some(View::DualForm(form)) => self.dual_form_key(form, key),
            Some(View::Confirm(confirm)) => self.confirm_key(confirm, key),
            Some(View::Created(resource)) => {
                if key.code == KeyCode::Enter {
                    self.enter(resource.target());
                } else {
                    self.views.push(View::Created(resource));
                }
            }
            Some(View::Split(studio)) => self.split_studio_key(studio, key),
            Some(View::LayoutStudio(state)) => self.layout_studio_key(state, key),
            Some(View::Expose(state)) => self.expose_key(state, key),
            Some(View::Clients(state)) => self.clients_key(state, key),
            Some(View::Buffers(state)) => self.buffers_key(state, key),
            Some(View::Snapshots(state)) => self.snapshots_key(state, key),
            Some(View::ThemePicker { initial, selected }) => {
                self.theme_picker_key(initial, selected, key);
            }
            Some(View::Config { selected }) => self.config_key(selected, key),
            Some(View::Tribute { id, step }) => self.tribute_key(id, step, key),
            Some(View::Help) => self.views.push(View::Help),
        }
        true
    }

    fn is_non_text_context(&self) -> bool {
        if self.preview_search.active {
            return false;
        }
        match self.views.top() {
            None => self.composer.value().is_empty(),
            Some(
                View::Help
                | View::Tribute { .. }
                | View::ThemePicker { .. }
                | View::Config { .. }
                | View::Actions(..)
                | View::Split(..)
                | View::LayoutStudio(..)
                | View::Expose(..)
                | View::Clients(..)
                | View::Buffers(..)
                | View::Snapshots(..)
                | View::Created(..),
            ) => true,
            Some(
                View::Picker(_)
                | View::TargetPick(_)
                | View::Form(_)
                | View::DualForm(_)
                | View::Confirm(_),
            ) => false,
        }
    }

    fn open_universal_palette(&mut self) {
        self.preview_search.reset();
        while let Some(view) = self.views.pop() {
            if let View::ThemePicker { initial, .. } = view {
                self.theme = self.theme.with_preset(initial);
            }
        }
        self.config_note = None;
        self.composer.set_value("/");
        self.palette_selected = 0;
        self.mention_selected = 0;
        self.status.clear();
        self.refresh_home_data();
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
            Some(View::TargetPick(state)) => {
                self.update_target_pick_preview(&state);
                self.views.push(View::TargetPick(state));
            }
            Some(View::LayoutStudio(mut state)) => {
                self.refresh_layout_studio_state(&mut state);
                self.views.push(View::LayoutStudio(state));
            }
            Some(View::Clients(mut state)) => {
                self.reload_clients_state(&mut state);
                self.views.push(View::Clients(state));
            }
            Some(View::Buffers(mut state)) => {
                self.reload_buffers_state(&mut state);
                self.views.push(View::Buffers(state));
            }
            Some(View::Snapshots(mut state)) => {
                self.reload_snapshots_state(&mut state);
                self.views.push(View::Snapshots(state));
            }
            Some(other) => self.views.push(other),
        }
    }

    fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace(['\r', '\n', '\t'], " ");
        if self.preview_search.active {
            self.preview_search.query.insert_str(&text);
            self.recompute_preview_search(true);
            return;
        }
        let top = self.views.pop();
        match top {
            None => {
                self.composer.insert_str(&text);
                self.palette_selected = 0;
                self.mention_selected = 0;
                self.status.clear();
                self.sync_home_matrix();
                self.refresh_home_preview();
            }
            Some(View::Picker(mut picker)) => {
                picker.query.insert_str(&text);
                picker.selected = 0;
                self.update_picker_preview(&picker);
                self.views.push(View::Picker(picker));
            }
            Some(View::TargetPick(mut state)) => {
                state.query.insert_str(&text);
                state.selected = 0;
                self.update_target_pick_preview(&state);
                self.views.push(View::TargetPick(state));
            }
            Some(View::Form(mut form)) => {
                form.input.insert_str(&text);
                self.views.push(View::Form(form));
            }
            Some(View::DualForm(mut form)) => {
                form.active_input_mut().insert_str(&text);
                self.views.push(View::DualForm(form));
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
                    | CommandKind::Layout
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
        let attached_suffix = self.theme.tr(" · attached", " · 已附着");
        let session_badge = self.theme.tr("session", "会话");
        let window_badge = self.theme.tr("window", "窗口");
        let pane_badge = self.theme.tr("pane", "窗格");
        let busy_suffix = self.theme.tr(" · busy", " · 运行中");
        let win_unit = self.theme.tr("w", "窗");
        let pane_unit = self.theme.tr("p", "格");
        for s in &self.home_sessions {
            let detail = format!(
                "{}{win_unit}{}",
                s.windows,
                if s.is_attached() { attached_suffix } else { "" }
            );
            items.push((
                format!("@{}", s.name),
                session_badge,
                detail,
                Resource::Session(s.clone()),
            ));
        }
        for w in &self.home_windows {
            items.push((
                format!("@{}:{}", w.session_name, w.index),
                window_badge,
                format!("{} · {}{pane_unit}", w.name, w.panes),
                Resource::Window(w.clone()),
            ));
        }
        for p in &self.home_panes {
            items.push((
                format!("@{}:{}.{}", p.session_name, p.window_index, p.index),
                pane_badge,
                format!(
                    "{}{}",
                    p.current_command,
                    if p.busy { busy_suffix } else { "" }
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
                    self.status = theme_toast(next, self.theme.language);
                    return;
                }
                KeyCode::Char('f' | 'F')
                    if self.config.live_preview && self.cached_preview.is_some() =>
                {
                    self.open_preview_search();
                    return;
                }
                KeyCode::Char('y' | 'Y')
                    if self.config.live_preview && self.cached_preview.is_some() =>
                {
                    self.copy_current_preview_to_clipboard();
                    return;
                }
                _ => {}
            }
        }
        if self.handle_preview_paging_key(key, self.composer.value().is_empty()) {
            return;
        }
        if key.code == KeyCode::BackTab && !ctrl && !alt {
            self.composer_mode = self.composer_mode.next();
            self.config.composer_mode = self.composer_mode;
            self.tip_step = self.tip_step.wrapping_add(1);
            self.status.clear();
            if self.composer_mode == ComposerMode::LiveRadar {
                self.refresh_home_data();
            } else {
                self.sync_home_matrix();
                self.refresh_home_preview();
            }
            return;
        }
        if self.composer.value().is_empty() {
            if self.composer_mode == ComposerMode::CommandJump {
                if !ctrl
                    && !alt
                    && let KeyCode::Char(d @ '1'..='9') = key.code
                {
                    let idx = (d as usize) - ('1' as usize);
                    if let Some(session) = self.home_sessions.get(idx).cloned() {
                        self.status.clear();
                        if self.matrix_action == MatrixAction::Open {
                            self.enter(Target::Session(session.id));
                        } else {
                            self.matrix.select_session(&session.id);
                            self.refresh_home_preview();
                        }
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
                if key.modifiers.is_empty() {
                    match key.code {
                        KeyCode::Char('[') if self.cycle_home_preview_pane(false) => {
                            self.status.clear();
                            return;
                        }
                        KeyCode::Char(']') if self.cycle_home_preview_pane(true) => {
                            self.status.clear();
                            return;
                        }
                        KeyCode::Char('f')
                            if self.config.live_preview
                                && self.cached_preview.is_some()
                                && self.preview_target.is_some() =>
                        {
                            self.open_preview_search();
                            return;
                        }
                        KeyCode::Char('y')
                            if self.config.live_preview
                                && self.cached_preview.is_some()
                                && self.preview_target.is_some() =>
                        {
                            self.copy_current_preview_to_clipboard();
                            return;
                        }
                        KeyCode::Char('n')
                            if self.preview_search.query_str().is_some()
                                && self.matrix_action != MatrixAction::Window =>
                        {
                            self.step_preview_search_match(true);
                            return;
                        }
                        KeyCode::Left => {
                            if self.matrix.move_column(-1) {
                                self.sync_action_scope_with_matrix();
                                self.status.clear();
                                self.refresh_home_preview();
                            }
                            return;
                        }
                        KeyCode::Right => {
                            if self.matrix.move_column(1) {
                                self.sync_action_scope_with_matrix();
                                self.status.clear();
                                self.refresh_home_preview();
                            }
                            return;
                        }
                        KeyCode::Up => {
                            if self.matrix.move_row(-1) {
                                self.sync_action_scope_with_matrix();
                                self.status.clear();
                                self.refresh_home_preview();
                            }
                            return;
                        }
                        KeyCode::Down => {
                            if self.matrix.is_empty() {
                                self.composer.set_value("/");
                                self.palette_selected = 0;
                                self.mention_selected = 0;
                                self.status.clear();
                            } else if self.matrix.move_row(1) {
                                self.sync_action_scope_with_matrix();
                                self.status.clear();
                                self.refresh_home_preview();
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                if key.code == KeyCode::Char('N')
                    && !ctrl
                    && !alt
                    && self.preview_search.query_str().is_some()
                {
                    self.step_preview_search_match(false);
                    return;
                }
                let is_new_window = !ctrl
                    && !alt
                    && (key.code == KeyCode::Char('+')
                        || (self.matrix_action == MatrixAction::Window
                            && key.code == KeyCode::Char('n')
                            && key.modifiers.is_empty()));
                if is_new_window && let Some(session) = self.matrix.selected_session().cloned() {
                    self.status.clear();
                    self.views
                        .push(View::DualForm(DualForm::new_window(session.id)));
                    return;
                }
            }
            if self.composer_mode == ComposerMode::LiveRadar && !ctrl && !alt {
                match key.code {
                    KeyCode::Char('f')
                        if key.modifiers.is_empty()
                            && self.config.live_preview
                            && self.cached_preview.is_some() =>
                    {
                        self.open_preview_search();
                        return;
                    }
                    KeyCode::Char('y')
                        if key.modifiers.is_empty()
                            && self.config.live_preview
                            && self.cached_preview.is_some() =>
                    {
                        self.copy_current_preview_to_clipboard();
                        return;
                    }
                    KeyCode::Char('n')
                        if key.modifiers.is_empty()
                            && self.preview_search.query_str().is_some() =>
                    {
                        self.step_preview_search_match(true);
                        return;
                    }
                    KeyCode::Char('N') if self.preview_search.query_str().is_some() => {
                        self.step_preview_search_match(false);
                        return;
                    }
                    KeyCode::Up => {
                        self.radar_selected =
                            wrap_step(self.radar_selected, self.home_sessions.len(), -1);
                        self.refresh_radar_preview();
                        return;
                    }
                    KeyCode::Down => {
                        self.radar_selected =
                            wrap_step(self.radar_selected, self.home_sessions.len(), 1);
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
                    self.palette_selected = wrap_step(self.palette_selected, palette.len(), -1);
                    return;
                }
                KeyCode::Down => {
                    self.palette_selected = wrap_step(self.palette_selected, palette.len(), 1);
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
                        self.mention_selected =
                            wrap_step(self.mention_selected, mentions.len(), -1);
                        return;
                    }
                    KeyCode::Down => {
                        self.mention_selected = wrap_step(self.mention_selected, mentions.len(), 1);
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
        if self.composer_mode == ComposerMode::CommandJump
            && !self.composer.value().trim_start().starts_with('/')
            && !self.composer.value().trim_start().starts_with('@')
            && key.modifiers.is_empty()
        {
            match key.code {
                KeyCode::Up => {
                    if self.matrix.move_row(-1) {
                        self.sync_action_scope_with_matrix();
                        self.refresh_home_preview();
                    }
                    return;
                }
                KeyCode::Down => {
                    if self.matrix.move_row(1) {
                        self.sync_action_scope_with_matrix();
                        self.refresh_home_preview();
                    }
                    return;
                }
                KeyCode::Tab => {
                    self.matrix_action = self
                        .matrix_action
                        .cycle_workspace_tab(self.matrix.is_session_selected());
                    self.status.clear();
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {}
            KeyCode::Enter => {
                let trimmed = self.composer.value().trim_start();
                if self.composer_mode == ComposerMode::QuickLaunch
                    && !trimmed.starts_with('/')
                    && !trimmed.starts_with('@')
                {
                    self.quick_launch_submit();
                    return;
                }
                if trimmed.starts_with('/') || trimmed.starts_with('@') {
                    let text = self.composer.value().to_string();
                    self.execute_text(&text);
                    return;
                }
                self.submit_home_input();
            }
            _ => {
                if self.composer.handle_key(key) {
                    self.palette_selected = 0;
                    self.mention_selected = 0;
                    self.status.clear();
                    self.sync_home_matrix();
                    self.refresh_home_preview();
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
                selected = wrap_step(selected, ThemePreset::ALL.len(), -1);
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            KeyCode::Down => {
                selected = wrap_step(selected, ThemePreset::ALL.len(), 1);
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            KeyCode::Enter => {
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.config.theme = preset;
                self.status = theme_toast(preset, self.theme.language);
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
                            self.status = if self.theme.language.is_zh() {
                                format!("✦ 已将主题 '{}' 保存到 {}", preset.slug(), path.display())
                            } else {
                                format!("✦ Saved theme '{}' to {}", preset.slug(), path.display())
                            };
                        }
                        Err(error) => {
                            self.status = if self.theme.language.is_zh() {
                                format!("无法保存主题：{error}")
                            } else {
                                format!("Could not save theme: {error}")
                            };
                            self.views.push(View::ThemePicker { initial, selected });
                        }
                    },
                    None => {
                        self.status = self
                            .theme
                            .tr(
                                "No config path available (set HOME or --config)",
                                "未找到可用配置路径（请设置 HOME 或传入 --config）",
                            )
                            .to_owned();
                        self.views.push(View::ThemePicker { initial, selected });
                    }
                }
            }
            _ => self.views.push(View::ThemePicker { initial, selected }),
        }
    }

    fn config_key(&mut self, mut selected: usize, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => selected = wrap_step(selected, CONFIG_ITEM_COUNT, -1),
            KeyCode::Down => selected = wrap_step(selected, CONFIG_ITEM_COUNT, 1),
            KeyCode::Left if no_ctrl_alt => self.cycle_config_setting(selected, false),
            KeyCode::Right if no_ctrl_alt => self.cycle_config_setting(selected, true),
            KeyCode::Enter => self.cycle_config_setting(selected, true),
            KeyCode::Char(' ') if no_ctrl_alt => self.cycle_config_setting(selected, true),
            KeyCode::Char('s' | 'S') if no_ctrl_alt => {
                let target = self
                    .config_path
                    .clone()
                    .or_else(|| config::resolve_config_path(None));
                match target {
                    Some(path) => match self.config.save(&path) {
                        Ok(()) => {
                            self.config_path = Some(path.clone());
                            let message = if self.theme.language.is_zh() {
                                format!("✦ 已保存配置到 {}", path.display())
                            } else {
                                format!("✦ Saved config to {}", path.display())
                            };
                            self.config_note = Some(message.clone());
                            self.status = message;
                        }
                        Err(error) => {
                            let message = if self.theme.language.is_zh() {
                                format!("无法保存配置：{error}")
                            } else {
                                format!("Could not save config: {error}")
                            };
                            self.config_note = Some(message.clone());
                            self.status = message;
                        }
                    },
                    None => {
                        let message = self
                            .theme
                            .tr(
                                "No config path available (set HOME or --config)",
                                "未找到可用配置路径（请设置 HOME 或传入 --config）",
                            )
                            .to_owned();
                        self.config_note = Some(message.clone());
                        self.status = message;
                    }
                }
            }
            _ => {}
        }
        self.views.push(View::Config { selected });
    }

    fn cycle_config_setting(&mut self, index: usize, forward: bool) {
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
                    self.clear_preview();
                }
            }
            4 => {
                self.config.mouse = !self.config.mouse;
                restart_changed = true;
            }
            5 => {
                self.config.emblem_mode = if forward {
                    self.config.emblem_mode.next()
                } else {
                    self.config.emblem_mode.prev()
                };
                if !self.config.emblem_mode.should_animate() {
                    self.emblem_step = 0;
                }
            }
            6 => {
                let prev = self.config.inline_height;
                self.config.inline_height = if forward {
                    if prev >= 40 {
                        8
                    } else {
                        (prev + 1).clamp(8, 40)
                    }
                } else if prev <= 8 {
                    40
                } else {
                    prev.saturating_sub(1).clamp(8, 40)
                };
                restart_changed = self.config.inline_height != prev;
            }
            7 => {
                self.config.ascii = !self.config.ascii;
                self.theme = self.config.effective_theme();
            }
            8 => {
                self.config.no_color = !self.config.no_color;
                self.theme = self.config.effective_theme();
            }
            9 => {
                self.config.language = self.config.language.toggle();
                self.theme = self.config.effective_theme();
            }
            10 => self.toggle_tmux_popup(),
            11 => self.update_tmux_comfort_setting(|opts| opts.mouse = !opts.mouse),
            12 => self.update_tmux_comfort_setting(|opts| {
                opts.history_limit = if forward {
                    TmuxComfortOptions::next_history_limit_preset(opts.history_limit)
                } else {
                    let pos = HISTORY_LIMIT_PRESETS
                        .iter()
                        .position(|&p| p == opts.history_limit)
                        .unwrap_or(0);
                    let prev =
                        (pos + HISTORY_LIMIT_PRESETS.len() - 1) % HISTORY_LIMIT_PRESETS.len();
                    HISTORY_LIMIT_PRESETS[prev]
                };
            }),
            13 => {
                self.update_tmux_comfort_setting(|opts| opts.base_index_one = !opts.base_index_one);
            }
            14 => self.update_tmux_comfort_setting(|opts| {
                opts.renumber_windows = !opts.renumber_windows;
            }),
            15 => self.update_tmux_comfort_setting(|opts| {
                opts.zero_escape_time = !opts.zero_escape_time;
            }),
            16 => self.reload_tmux_conf_action(),
            17 => {
                self.config.live_stream = !self.config.live_stream;
            }
            _ => {}
        }
        if restart_changed {
            self.config_note = Some(
                self.theme
                    .tr(
                        "Takes effect on next jm launch (press s to save)",
                        "重启 jm 后生效 · 按 s 保存",
                    )
                    .into(),
            );
        }
    }

    fn shorten_home_path(path: &std::path::Path) -> String {
        if let Some(home) = env::home_dir()
            && let Ok(relative) = path.strip_prefix(home)
        {
            return if relative.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", relative.display())
            };
        }
        path.display().to_string()
    }

    fn toggle_tmux_popup(&mut self) {
        let target = !self.config.tmux_popup;
        let Some(conf_path) = self.tmux_conf_path.clone() else {
            self.config.tmux_popup = target;
            let message = self
                .theme
                .tr(
                    "No ~/.tmux.conf path available (set HOME or JUSTMUX_TMUX_CONF)",
                    "未找到 ~/.tmux.conf 路径（请设置 HOME 或 JUSTMUX_TMUX_CONF）",
                )
                .to_owned();
            self.config_note = Some(message.clone());
            self.status = message;
            return;
        };

        let short = Self::shorten_home_path(&conf_path);
        if target {
            if let Err(err) =
                crate::tmux_conf::install(&conf_path, self.popup_fallback_exe.as_deref())
            {
                let message = if self.theme.language.is_zh() {
                    format!("无法更新 {short}：{err}")
                } else {
                    format!("Could not update {short}: {err}")
                };
                self.config_note = Some(message.clone());
                self.status = message;
                return;
            }
            self.config.tmux_popup = true;
            let supports_popup = self
                .tmux
                .version()
                .map(|version| version.supports_popup())
                .unwrap_or(true);
            let cmd = crate::tmux_conf::popup_run_shell_command(self.popup_fallback_exe.as_deref());
            let message = match self.tmux.bind_popup_key(&cmd) {
                Ok(true) if !supports_popup => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已写入 {short}，但浮窗需要 tmux 3.2+")
                    } else {
                        format!("✦ Updated {short}, but display-popup requires tmux 3.2+")
                    }
                }
                Ok(true) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已写入 {short} 并热加载 Ctrl+b j（终端与 agy 内均可唤出）")
                    } else {
                        format!(
                            "✦ Enabled Ctrl+b j in {short} & reloaded tmux (works in shell & agy)"
                        )
                    }
                }
                Ok(false) if !supports_popup => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已写入 {short}，但浮窗需要 tmux 3.2+")
                    } else {
                        format!("✦ Updated {short}, but display-popup requires tmux 3.2+")
                    }
                }
                Ok(false) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已写入 {short}（启动 tmux 后按 Ctrl+b j 即可唤出）")
                    } else {
                        format!("✦ Enabled Ctrl+b j in {short} (active when tmux starts)")
                    }
                }
                Err(err) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已写入 {short}（请执行 tmux source-file 重载：{err}）")
                    } else {
                        format!("✦ Updated {short} (run tmux source-file to reload: {err})")
                    }
                }
            };
            self.config_note = Some(message.clone());
            self.status = message;
        } else {
            if let Err(err) = crate::tmux_conf::uninstall(&conf_path) {
                let message = if self.theme.language.is_zh() {
                    format!("无法更新 {short}：{err}")
                } else {
                    format!("Could not update {short}: {err}")
                };
                self.config_note = Some(message.clone());
                self.status = message;
                return;
            }
            self.config.tmux_popup = false;
            let message = match self.tmux.unbind_popup_key() {
                Ok(true) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已从 {short} 移除并解除当前 tmux 的 Ctrl+b j 绑定")
                    } else {
                        format!("✦ Removed Ctrl+b j from {short} & unbound live tmux")
                    }
                }
                Ok(false) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已从 {short} 移除 JustMux 快捷键配置")
                    } else {
                        format!("✦ Removed JustMux popup keybind from {short}")
                    }
                }
                Err(err) => {
                    if self.theme.language.is_zh() {
                        format!("✦ 已更新 {short}（请执行 tmux source-file 重载：{err}）")
                    } else {
                        format!("✦ Removed from {short} (run tmux source-file to reload: {err})")
                    }
                }
            };
            self.config_note = Some(message.clone());
            self.status = message;
        }
    }

    fn config_modal_items(&self) -> Vec<ConfigItemView<'static>> {
        let tr = |en, zh| self.theme.tr(en, zh);
        vec![
            ConfigItemView {
                key: "screen_mode",
                value: self.config.screen_mode.as_str().to_owned(),
                description: tr(
                    "Alternate fullscreen or native inline scrollback (restart)",
                    "备用全屏或终端内嵌模式（重启生效）",
                ),
            },
            ConfigItemView {
                key: "theme",
                value: self.config.theme.slug().to_owned(),
                description: tr(
                    "Color palette preset (aurora, codex, claude, matrix, classic)",
                    "色彩主题预设（aurora / codex / claude / matrix / classic）",
                ),
            },
            ConfigItemView {
                key: "composer_mode",
                value: composer_mode_slug(self.config.composer_mode).to_owned(),
                description: tr(
                    "Default home studio mode (Shift+Tab)",
                    "主页默认工作模式 · Shift+Tab 切换",
                ),
            },
            ConfigItemView {
                key: "live_preview",
                value: self.config.live_preview.to_string(),
                description: tr(
                    "Capture live tmux pane previews",
                    "实时捕获 tmux 窗格终端预览内容",
                ),
            },
            ConfigItemView {
                key: "mouse",
                value: self.config.mouse.to_string(),
                description: tr(
                    "Mouse wheel & click support in Fullscreen (restart)",
                    "全屏模式下启用鼠标滚轮与点击（重启生效）",
                ),
            },
            ConfigItemView {
                key: "emblem_mode",
                value: self.config.emblem_mode.as_str().to_owned(),
                description: tr(
                    "Hydra Braille emblem style (static, spin, inverted-spin)",
                    "九头蛇点阵徽标动画模式（static / spin / inverted-spin）",
                ),
            },
            ConfigItemView {
                key: "inline_height",
                value: self.config.inline_height.to_string(),
                description: tr(
                    "Reserved viewport rows for inline mode 8..=40 (restart)",
                    "内嵌模式预留视口行数 8..=40（重启生效）",
                ),
            },
            ConfigItemView {
                key: "ascii",
                value: self.config.ascii.to_string(),
                description: tr(
                    "Pure ASCII box borders and markers",
                    "纯 7-bit ASCII 边框与符号兼容模式",
                ),
            },
            ConfigItemView {
                key: "no_color",
                value: self.config.no_color.to_string(),
                description: tr(
                    "Disable ANSI color attributes",
                    "禁用所有 ANSI 颜色输出（NO_COLOR）",
                ),
            },
            ConfigItemView {
                key: "language",
                value: self.config.language.as_str().to_owned(),
                description: tr(
                    "Interface language (en = English, zh = Chinese)",
                    "界面语言：英文 (en) / 简体中文 (zh)",
                ),
            },
            ConfigItemView {
                key: "tmux_popup",
                value: self.config.tmux_popup.to_string(),
                description: tr(
                    "Bind Ctrl+b j in ~/.tmux.conf to pop up jm in shell or agy",
                    "写入 ~/.tmux.conf 绑定 Ctrl+b j，在终端或 agy 内随时唤出 jm",
                ),
            },
            ConfigItemView {
                key: "tmux_mouse",
                value: self.config.tmux_mouse.to_string(),
                description: tr(
                    "Enable native tmux mouse scroll, focus & pane drag (set -g mouse on)",
                    "开启 tmux 原生鼠标滚轮、点击切窗格与拖拽调尺寸 (set -g mouse on)",
                ),
            },
            ConfigItemView {
                key: "tmux_history_limit",
                value: if self.config.tmux_history_limit == 0 {
                    tr("off", "未托管").to_owned()
                } else {
                    self.config.tmux_history_limit.to_string()
                },
                description: tr(
                    "Scrollback lines in ~/.tmux.conf (off / 10000 / 50000 / 100000)",
                    "tmux 滚动历史行数预设（未托管 / 10000 / 50000 / 100000）",
                ),
            },
            ConfigItemView {
                key: "tmux_base_index_one",
                value: self.config.tmux_base_index_one.to_string(),
                description: tr(
                    "Number windows & panes from 1 instead of 0 (base-index 1)",
                    "窗口与窗格编号从 1 开始，更贴合键盘数字键顺序 (base-index 1)",
                ),
            },
            ConfigItemView {
                key: "tmux_renumber_windows",
                value: self.config.tmux_renumber_windows.to_string(),
                description: tr(
                    "Auto-close numbering gaps when a window is killed (renumber-windows)",
                    "关闭窗口后自动重排剩余窗口编号，不留空号 (renumber-windows)",
                ),
            },
            ConfigItemView {
                key: "tmux_zero_escape_time",
                value: self.config.tmux_zero_escape_time.to_string(),
                description: tr(
                    "Eliminate Esc key lag in Vim/Neovim & TUI apps (escape-time 0)",
                    "消除 Vim/Neovim 与 TUI 中按 Esc 的延迟卡顿 (escape-time 0)",
                ),
            },
            ConfigItemView {
                key: "tmux_reload",
                value: tr("[Reload]", "[立即重载]").to_owned(),
                description: tr(
                    "Press Enter to live-reload ~/.tmux.conf (tmux source-file)",
                    "按 Enter 立即热重载 ~/.tmux.conf (tmux source-file)",
                ),
            },
            ConfigItemView {
                key: "live_stream",
                value: self.config.live_stream.to_string(),
                description: tr(
                    "Auto-stream live pane previews & Expose wall in Fullscreen",
                    "全屏模式下自动实时流式刷新终端预览与全景墙",
                ),
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
        self.status = if self.theme.language.is_zh() {
            format!(
                "✦ 已应用 {} 主题：{} ({})",
                card.badge,
                preset.name(),
                preset.origin()
            )
        } else {
            format!(
                "✦ {} theme applied: {} ({})",
                card.badge,
                preset.name(),
                preset.origin()
            )
        };
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
            CommandKind::Split => self.open_split_studio(Some(resource)),
            CommandKind::Layout => self.open_layout_studio(Some(resource)),
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
                            self.views = ViewStack::default();
                            self.matrix_action = MatrixAction::Rename(TargetScope::Window);
                            self.refresh_home_data();
                            return;
                        }
                    };
                    match result {
                        Ok(()) => {
                            if renamed_current && let Context::Inside(inside) = &mut self.context {
                                inside.session_name = Some(new_name);
                            }
                            self.refresh_home_data();
                            self.status = self.theme.tr("Renamed", "已重命名").into();
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
                    self.views = ViewStack::default();
                    self.matrix_action = MatrixAction::Window;
                    self.composer.set_value(&session.name);
                    self.refresh_home_data();
                }
                Resource::Pane(pane) => {
                    self.views = ViewStack::default();
                    self.matrix_action = MatrixAction::Window;
                    self.composer.set_value(&pane.session_name);
                    self.refresh_home_data();
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
                    self.status = self
                        .theme
                        .tr(
                            "Pane split. Native tmux layout updated.",
                            "窗格已拆分，tmux 原生布局已更新。",
                        )
                        .into();
                    self.refresh_home_data();
                }
                Err(error) => self.status = error.to_string(),
            }
        } else {
            self.views = ViewStack::default();
            self.matrix_action = MatrixAction::Split;
            self.refresh_home_data();
        }
    }

    fn open_split_studio_from_home(&mut self, resource: Resource) {
        self.matrix_action = MatrixAction::Open;
        self.composer.set_value("");
        self.sync_home_matrix();
        self.open_split_studio(Some(resource));
    }

    fn open_split_studio(&mut self, resource: Option<Resource>) {
        self.refresh_home_data();
        if self.home_windows.is_empty() {
            self.views = ViewStack::default();
            self.matrix_action = MatrixAction::Split;
            return;
        }
        let anchor_window = match &resource {
            Some(Resource::Window(window)) => Some(
                self.home_windows
                    .iter()
                    .find(|w| w.id == window.id)
                    .cloned()
                    .unwrap_or_else(|| window.clone()),
            ),
            Some(Resource::Pane(pane)) => self
                .home_windows
                .iter()
                .find(|w| w.id == pane.window_id)
                .cloned()
                .or_else(|| self.home_windows.first().cloned()),
            Some(Resource::Session(session)) => self
                .home_windows
                .iter()
                .find(|w| w.session_id == session.id && w.active)
                .or_else(|| {
                    self.home_windows
                        .iter()
                        .find(|w| w.session_id == session.id)
                })
                .cloned()
                .or_else(|| self.home_windows.first().cloned()),
            None => self
                .active_or_first_pane()
                .and_then(|pane| {
                    self.home_windows
                        .iter()
                        .find(|w| w.id == pane.window_id)
                        .cloned()
                })
                .or_else(|| self.home_windows.first().cloned()),
        };
        let Some(anchor_window) = anchor_window else {
            self.views = ViewStack::default();
            self.matrix_action = MatrixAction::Split;
            return;
        };

        let anchor_pane = match &resource {
            Some(Resource::Pane(pane)) => Some(pane.clone()),
            _ => self.representative_pane_for_window(&anchor_window.id),
        };
        let anchor_cwd = anchor_pane
            .as_ref()
            .map(|p| PathBuf::from(&p.current_path))
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let anchor_panes: Vec<Pane> = self
            .home_panes
            .iter()
            .filter(|p| p.window_id == anchor_window.id)
            .cloned()
            .collect();
        let options = self.collect_split_window_options(
            &anchor_window.session_id,
            Some(&anchor_window.id),
            anchor_pane.as_ref(),
        );
        let mut previews = HashMap::new();
        for opt in options.iter().take(24) {
            if let Some(lines) = self.capture_split_option_preview(opt) {
                previews.insert(opt.window.id.clone(), lines);
            }
        }

        let studio = if anchor_panes.len() >= 2 {
            let mut pane_previews = HashMap::new();
            if self.config.live_preview {
                for pane in anchor_panes.iter().take(4) {
                    if let Ok(lines) = self
                        .tmux
                        .capture_pane(&Target::Pane(pane.id.clone()), PREVIEW_CAPTURE_LINES)
                    {
                        pane_previews.insert(pane.id.clone(), lines);
                    }
                }
            }
            SplitStudio::from_existing_window(
                anchor_window.session_id,
                anchor_window.id,
                anchor_cwd,
                anchor_window.synchronized,
                &anchor_panes,
                anchor_pane.as_ref().map(|p| &p.id),
                options,
                previews,
                pane_previews,
            )
        } else {
            let mut s = SplitStudio::new(
                anchor_window.session_id,
                anchor_cwd,
                Some(&anchor_window.id),
                options,
                previews,
            );
            s.initial_sync_panes = anchor_window.synchronized;
            s.sync_panes = anchor_window.synchronized;
            s
        };
        self.views.push(View::Split(studio));
    }

    fn collect_split_window_options(
        &self,
        anchor_session_id: &SessionId,
        preferred_anchor_window_id: Option<&WindowId>,
        preferred_anchor_pane: Option<&Pane>,
    ) -> Vec<SplitWindowOption> {
        let mut ordered_windows: Vec<&Window> = Vec::with_capacity(self.home_windows.len());
        if let Some(pref_id) = preferred_anchor_window_id
            && let Some(pref) = self.home_windows.iter().find(|w| &w.id == pref_id)
        {
            ordered_windows.push(pref);
        }
        for window in &self.home_windows {
            if Some(&window.id) != preferred_anchor_window_id
                && &window.session_id == anchor_session_id
            {
                ordered_windows.push(window);
            }
        }
        for window in &self.home_windows {
            if &window.session_id != anchor_session_id {
                ordered_windows.push(window);
            }
        }

        ordered_windows
            .into_iter()
            .map(|window| {
                let rep_pane = if preferred_anchor_window_id == Some(&window.id) {
                    preferred_anchor_pane
                        .cloned()
                        .or_else(|| self.representative_pane_for_window(&window.id))
                } else {
                    self.representative_pane_for_window(&window.id)
                };
                let command = rep_pane
                    .as_ref()
                    .map(|p| p.current_command.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                let path = rep_pane
                    .as_ref()
                    .map(|p| p.current_path.clone())
                    .unwrap_or_default();
                SplitWindowOption {
                    window: window.clone(),
                    active_pane_id: rep_pane.map(|p| p.id),
                    command,
                    path,
                }
            })
            .collect()
    }

    fn capture_split_option_preview(&self, opt: &SplitWindowOption) -> Option<Vec<String>> {
        if !self.config.live_preview {
            return None;
        }
        let target = opt.active_pane_id.as_ref().map_or_else(
            || Target::Window(opt.window.session_id.clone(), opt.window.id.clone()),
            |pane_id| Target::Pane(pane_id.clone()),
        );
        self.tmux.capture_pane(&target, PREVIEW_CAPTURE_LINES).ok()
    }

    fn ensure_split_preview(&self, studio: &mut SplitStudio, window_id: &WindowId) {
        if !self.config.live_preview || studio.previews.contains_key(window_id) {
            return;
        }
        if let Some(opt) = studio.window_option(window_id).cloned()
            && let Some(lines) = self.capture_split_option_preview(&opt)
        {
            studio.previews.insert(window_id.clone(), lines);
        }
    }

    fn ensure_selected_split_preview(&self, studio: &mut SplitStudio) {
        match studio.root.slot_fill(studio.selected_slot).cloned() {
            Some(SplitSlotFill::Window(window_id)) => {
                self.ensure_split_preview(studio, &window_id);
            }
            Some(SplitSlotFill::Pane(pane_id)) => {
                if self.config.live_preview
                    && !studio.pane_previews.contains_key(&pane_id)
                    && let Ok(lines) = self
                        .tmux
                        .capture_pane(&Target::Pane(pane_id.clone()), PREVIEW_CAPTURE_LINES)
                {
                    studio.pane_previews.insert(pane_id, lines);
                }
            }
            _ => {}
        }
    }

    fn recover_split_studio(&mut self, mut studio: SplitStudio) {
        self.refresh_home_data();
        if !self.home_windows.is_empty()
            && !self
                .home_windows
                .iter()
                .any(|w| w.session_id == studio.anchor_session_id)
        {
            studio.anchor_session_id = self.home_windows[0].session_id.clone();
        }
        let preferred_window_id = match &studio.initial_fill {
            SplitSlotFill::Window(wid) => Some(wid.clone()),
            SplitSlotFill::Pane(_) | SplitSlotFill::NewShell => studio.anchor_window_id.clone(),
        };
        let options = self.collect_split_window_options(
            &studio.anchor_session_id,
            preferred_window_id.as_ref(),
            None,
        );
        let live_windows: HashSet<WindowId> =
            options.iter().map(|opt| opt.window.id.clone()).collect();
        let live_panes: HashSet<PaneId> = self.home_panes.iter().map(|p| p.id.clone()).collect();
        studio.root.prune_missing_windows(&live_windows);
        studio.root.prune_missing_panes(&live_panes);
        if let SplitSlotFill::Window(wid) = &studio.initial_fill
            && !live_windows.contains(wid)
        {
            studio.initial_fill = SplitSlotFill::NewShell;
        }
        studio.preview_scrolls.retain(|slot_id, _| {
            matches!(
                studio.root.slot_fill(*slot_id),
                Some(SplitSlotFill::Window(_) | SplitSlotFill::Pane(_))
            )
        });
        studio.previews.clear();
        for opt in options.iter().take(24) {
            if let Some(lines) = self.capture_split_option_preview(opt) {
                studio.previews.insert(opt.window.id.clone(), lines);
            }
        }
        studio.window_options = options;
        let leaf_windows: Vec<WindowId> = studio
            .root
            .leaves()
            .into_iter()
            .filter_map(|(_, fill)| match fill {
                SplitSlotFill::Window(wid) => Some(wid),
                SplitSlotFill::Pane(_) | SplitSlotFill::NewShell => None,
            })
            .collect();
        for wid in leaf_windows {
            self.ensure_split_preview(&mut studio, &wid);
        }
        studio.sync_cursor_to_selected_slot();
        self.ensure_selected_split_preview(&mut studio);
        self.views.push(View::Split(studio));
    }

    fn representative_pane_for_window(&self, window_id: &WindowId) -> Option<Pane> {
        self.home_panes
            .iter()
            .find(|p| &p.window_id == window_id && p.active)
            .or_else(|| self.home_panes.iter().find(|p| &p.window_id == window_id))
            .cloned()
    }

    fn split_studio_key(&mut self, mut studio: SplitStudio, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Tab => {
                studio.cycle_slot(true);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::BackTab => {
                studio.cycle_slot(false);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::Left if !shift && !ctrl && !alt => {
                studio.cycle_slot(false);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::Right if !shift && !ctrl && !alt => {
                studio.cycle_slot(true);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::Up if !ctrl && !alt => {
                studio.move_window_cursor(true);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::Down if !ctrl && !alt => {
                studio.move_window_cursor(false);
                self.ensure_selected_split_preview(&mut studio);
            }
            KeyCode::PageUp => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, true, 6);
            }
            KeyCode::PageDown => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, false, 6);
            }
            KeyCode::Home => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, true, usize::MAX);
            }
            KeyCode::End => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, false, usize::MAX);
            }
            KeyCode::Char('u' | 'U') if ctrl && !alt => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, true, 12);
            }
            KeyCode::Char('d' | 'D') if ctrl && !alt => {
                let selected = studio.selected_slot;
                self.scroll_split_slot_preview(&mut studio, selected, false, 12);
            }
            KeyCode::Char('1') if !ctrl && !alt => {
                if studio.select_slot_by_number(1) {
                    self.ensure_selected_split_preview(&mut studio);
                }
            }
            KeyCode::Char('2') if !ctrl && !alt => {
                if studio.select_slot_by_number(2) {
                    self.ensure_selected_split_preview(&mut studio);
                }
            }
            KeyCode::Char('3') if !ctrl && !alt => {
                if studio.select_slot_by_number(3) {
                    self.ensure_selected_split_preview(&mut studio);
                }
            }
            KeyCode::Char('4') if !ctrl && !alt => {
                if studio.select_slot_by_number(4) {
                    self.ensure_selected_split_preview(&mut studio);
                }
            }
            KeyCode::Char('h' | 'H' | '|') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::SplitHorizontal);
            }
            KeyCode::Char('v' | 'V' | '-') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::SplitVertical);
            }
            KeyCode::Char('t' | 'T') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::FlipDirection);
            }
            KeyCode::Char('[') if !ctrl && !alt => {
                if let Some(ratio) = studio.adjust_selected_ratio(-5) {
                    let other = 100u16.saturating_sub(ratio);
                    self.status = if self.theme.language.is_zh() {
                        format!("✦ 分屏比例：{ratio}/{other}")
                    } else {
                        format!("✦ Split ratio: {ratio}/{other}")
                    };
                }
            }
            KeyCode::Char(']') if !ctrl && !alt => {
                if let Some(ratio) = studio.adjust_selected_ratio(5) {
                    let other = 100u16.saturating_sub(ratio);
                    self.status = if self.theme.language.is_zh() {
                        format!("✦ 分屏比例：{ratio}/{other}")
                    } else {
                        format!("✦ Split ratio: {ratio}/{other}")
                    };
                }
            }
            KeyCode::Char('p' | 'P') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::RatioPreset);
            }
            KeyCode::Char('y' | 'Y') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::SyncToggle);
            }
            KeyCode::Char('g' | 'G') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::PresetGrid);
            }
            KeyCode::Char('b' | 'B') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::BreakOut);
            }
            KeyCode::Char('x' | 'X') | KeyCode::Delete | KeyCode::Backspace if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::RemoveSlot);
            }
            KeyCode::Char('r' | 'R') if !ctrl && !alt => {
                self.run_split_toolbar_action(&mut studio, SplitToolbarAction::Reset);
            }
            KeyCode::Char(' ') if !ctrl && !alt => {
                let row = studio.window_cursor;
                if studio.assign_cursor_row(row) {
                    self.ensure_selected_split_preview(&mut studio);
                }
            }
            KeyCode::Enter => {
                self.apply_split_studio_and_enter(studio);
                return;
            }
            _ => {}
        }
        self.views.push(View::Split(studio));
    }

    fn scroll_split_slot_preview(
        &self,
        studio: &mut SplitStudio,
        slot_id: SlotId,
        up: bool,
        step: usize,
    ) {
        if let Some(SplitSlotFill::Window(wid)) = studio.root.slot_fill(slot_id).cloned() {
            self.ensure_split_preview(studio, &wid);
        }
        let max = match studio.root.slot_fill(slot_id) {
            Some(SplitSlotFill::Window(wid)) => studio
                .previews
                .get(wid)
                .map_or(0, |lines| lines.len().saturating_sub(1)),
            Some(SplitSlotFill::Pane(pid)) => studio
                .pane_previews
                .get(pid)
                .map_or(0, |lines| lines.len().saturating_sub(1)),
            _ => 0,
        };
        studio.scroll_slot_preview(slot_id, up, step, max);
    }

    fn run_split_toolbar_action(&mut self, studio: &mut SplitStudio, action: SplitToolbarAction) {
        match action {
            SplitToolbarAction::SplitHorizontal => {
                if studio.split_selected(SplitDirection::SideBySide) {
                    self.status.clear();
                } else {
                    self.status = self
                        .theme
                        .tr(
                            "Max 2x2 layout reached (at most 2 columns x 2 rows).",
                            "已达 2x2 分屏上限（最多 2 列 × 2 行）。",
                        )
                        .into();
                }
            }
            SplitToolbarAction::SplitVertical => {
                if studio.split_selected(SplitDirection::Stacked) {
                    self.status.clear();
                } else {
                    self.status = self
                        .theme
                        .tr(
                            "Max 2x2 layout reached (at most 2 columns x 2 rows).",
                            "已达 2x2 分屏上限（最多 2 列 × 2 行）。",
                        )
                        .into();
                }
            }
            SplitToolbarAction::FlipDirection => {
                if studio.flip_layout() {
                    self.status = self
                        .theme
                        .tr("✦ Flipped split layout", "✦ 已翻转分屏方向")
                        .into();
                } else {
                    self.status = self
                        .theme
                        .tr(
                            "Need at least 2 panes to flip split direction.",
                            "至少需要 2 个窗格才能翻转分屏方向。",
                        )
                        .into();
                }
            }
            SplitToolbarAction::RatioPreset => {
                if let Some(ratio) = studio.cycle_selected_ratio() {
                    let other = 100u16.saturating_sub(ratio);
                    self.status = if self.theme.language.is_zh() {
                        format!("✦ 分屏比例：{ratio}/{other}")
                    } else {
                        format!("✦ Split ratio: {ratio}/{other}")
                    };
                } else {
                    self.status = self
                        .theme
                        .tr(
                            "Split into at least 2 panes to adjust ratio.",
                            "至少拆分为 2 个窗格才可调整比例。",
                        )
                        .into();
                }
            }
            SplitToolbarAction::SyncToggle => {
                let on = studio.toggle_sync_panes();
                self.status = if on {
                    self.theme
                        .tr(
                            "⚡ Synchronize-panes ON (input broadcasts to all panes on apply)",
                            "⚡ 同步输入已开启（应用后按键将广播到所有窗格）",
                        )
                        .into()
                } else {
                    self.theme
                        .tr("Synchronize-panes OFF", "同步输入已关闭")
                        .into()
                };
            }
            SplitToolbarAction::PresetGrid => {
                let preset = studio.apply_next_preset();
                self.ensure_selected_split_preview(studio);
                let label = preset.label(self.theme.language.is_zh());
                self.status = if self.theme.language.is_zh() {
                    format!("✦ 已切换分屏预设：{label}")
                } else {
                    format!("✦ Split preset: {label}")
                };
            }
            SplitToolbarAction::BreakOut => {
                let Some(pane_id) = studio.selected_existing_pane() else {
                    self.status = self
                        .theme
                        .tr(
                            "Select an existing pane to break out.",
                            "请选中已有窗格后再拆出为独立窗口。",
                        )
                        .into();
                    return;
                };
                if studio.root.leaf_count() <= 1 {
                    self.status = self
                        .theme
                        .tr(
                            "Cannot break out the only pane in this window.",
                            "无法将窗口中唯一的窗格拆出。",
                        )
                        .into();
                    return;
                }
                match self.tmux.break_pane(&pane_id, None) {
                    Ok(created) => {
                        studio.editing_existing_panes.retain(|p| p.id != pane_id);
                        studio.pane_previews.remove(&pane_id);
                        studio.remove_selected();
                        self.refresh_home_data();
                        let opts = self.collect_split_window_options(
                            &studio.anchor_session_id,
                            studio.anchor_window_id.as_ref(),
                            None,
                        );
                        studio.window_options = opts;
                        studio.sync_cursor_to_selected_slot();
                        self.ensure_selected_split_preview(studio);
                        self.status = if self.theme.language.is_zh() {
                            format!(
                                "✦ 已将窗格拆出为独立窗口 {}:{}",
                                created.session_name, created.index
                            )
                        } else {
                            format!(
                                "✦ Broke pane out to window {}:{} ({})",
                                created.session_name, created.index, created.name
                            )
                        };
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            SplitToolbarAction::RemoveSlot => {
                if studio.remove_selected() {
                    self.ensure_selected_split_preview(studio);
                    self.status.clear();
                } else {
                    self.status = self
                        .theme
                        .tr(
                            "Cannot remove the last remaining pane.",
                            "无法移除最后一个窗格。",
                        )
                        .into();
                }
            }
            SplitToolbarAction::Reset => {
                studio.reset();
                self.ensure_selected_split_preview(studio);
                self.status = self
                    .theme
                    .tr("Reset split canvas to 1x1", "分屏画布已重置为 1x1")
                    .into();
            }
            SplitToolbarAction::ApplyAndEnter => {}
        }
    }

    fn apply_split_studio_and_enter(&mut self, studio: SplitStudio) {
        let windows = match self.tmux.list_windows(None) {
            Ok(w) => w,
            Err(error) => {
                self.status = error.to_string();
                self.recover_split_studio(studio);
                return;
            }
        };
        let panes = match self.tmux.list_panes(None) {
            Ok(p) => p,
            Err(error) => {
                self.status = error.to_string();
                self.recover_split_studio(studio);
                return;
            }
        };

        let mut window_map: HashMap<WindowId, Window> =
            windows.into_iter().map(|w| (w.id.clone(), w)).collect();
        let mut rep_panes: HashMap<WindowId, (PaneId, usize, Option<String>)> = HashMap::new();
        for pane in &panes {
            let entry = rep_panes.entry(pane.window_id.clone()).or_insert_with(|| {
                (
                    pane.id.clone(),
                    0,
                    (!pane.current_path.trim().is_empty()).then(|| pane.current_path.clone()),
                )
            });
            entry.1 += 1;
            if pane.active {
                entry.0 = pane.id.clone();
                if !pane.current_path.trim().is_empty() {
                    entry.2 = Some(pane.current_path.clone());
                }
            }
        }

        let missing_staged_window = studio.root.leaves().into_iter().any(|(_, fill)| {
            matches!(
                fill,
                SplitSlotFill::Window(ref wid)
                    if !window_map.contains_key(wid) || !rep_panes.contains_key(wid)
            )
        });
        if missing_staged_window {
            self.status = self
                .theme
                .tr(
                    "Selected window no longer exists in tmux.",
                    "所选窗口在 tmux 中已不存在。",
                )
                .into();
            self.recover_split_studio(studio);
            return;
        }

        let anchor_cwd = (!studio.anchor_cwd.as_os_str().is_empty() && studio.anchor_cwd.is_dir())
            .then_some(studio.anchor_cwd.as_path());

        if let SplitNode::Leaf { fill, .. } = &studio.root {
            match fill {
                SplitSlotFill::Window(window_id) => {
                    if let Some(win) = window_map.get(window_id) {
                        if let Some((pane_id, _, _)) = rep_panes.get(window_id) {
                            let _ = self.tmux.select(&Target::Pane(pane_id.clone()));
                        }
                        self.enter(Target::Window(win.session_id.clone(), win.id.clone()));
                        return;
                    }
                    self.status = self
                        .theme
                        .tr(
                            "Selected window no longer exists in tmux.",
                            "所选窗口在 tmux 中已不存在。",
                        )
                        .into();
                    self.recover_split_studio(studio);
                    return;
                }
                SplitSlotFill::Pane(pane_id) => {
                    let _ = self.tmux.select(&Target::Pane(pane_id.clone()));
                    self.enter(Target::Pane(pane_id.clone()));
                    return;
                }
                SplitSlotFill::NewShell => {
                    match self
                        .tmux
                        .new_window(&studio.anchor_session_id, None, anchor_cwd)
                    {
                        Ok(created) => {
                            if let Ok(created_panes) = self.tmux.list_panes(Some(&created.id))
                                && let Some(first_pane) = created_panes.first()
                            {
                                let _ = self.tmux.select(&Target::Pane(first_pane.id.clone()));
                            }
                            self.enter(Target::Window(studio.anchor_session_id, created.id));
                        }
                        Err(error) => {
                            self.status = error.to_string();
                            self.recover_split_studio(studio);
                        }
                    }
                    return;
                }
            }
        }

        let (_, first_fill) = studio.root.first_leaf();
        let (target_session_id, target_window_id, first_anchor_pane) = if studio
            .is_editing_existing()
            && let Some(anchor_wid) = studio.anchor_window_id.clone()
            && let Some(anchor_win) = window_map.get(&anchor_wid).cloned()
        {
            let existing_in_anchor: Vec<PaneId> = panes
                .iter()
                .filter(|p| p.window_id == anchor_wid)
                .map(|p| p.id.clone())
                .collect();
            let desired_anchor = match &first_fill {
                SplitSlotFill::Pane(pid) if existing_in_anchor.contains(pid) => Some(pid.clone()),
                _ => None,
            };
            let first_pane = if let Some(keeper) = desired_anchor {
                for pid in &existing_in_anchor {
                    if pid != &keeper
                        && let Err(error) = self.tmux.break_pane(pid, None)
                    {
                        self.status = error.to_string();
                        self.recover_split_studio(studio);
                        return;
                    }
                }
                keeper
            } else if let Some(temp_pane) = existing_in_anchor.first().cloned() {
                let inserted = match &first_fill {
                    SplitSlotFill::Window(win_id) => {
                        if let Some((src_pane, _, _)) = rep_panes.get(win_id)
                            && src_pane != &temp_pane
                        {
                            if let Err(error) = self.tmux.join_pane(
                                src_pane,
                                &temp_pane,
                                SplitDirection::SideBySide,
                            ) {
                                self.status = error.to_string();
                                self.recover_split_studio(studio);
                                return;
                            }
                            src_pane.clone()
                        } else {
                            match self.tmux.split_pane(&temp_pane, SplitDirection::SideBySide) {
                                Ok(p) => p.id,
                                Err(error) => {
                                    self.status = error.to_string();
                                    self.recover_split_studio(studio);
                                    return;
                                }
                            }
                        }
                    }
                    SplitSlotFill::Pane(src_pane) => {
                        if src_pane != &temp_pane {
                            if let Err(error) = self.tmux.join_pane(
                                src_pane,
                                &temp_pane,
                                SplitDirection::SideBySide,
                            ) {
                                self.status = error.to_string();
                                self.recover_split_studio(studio);
                                return;
                            }
                            src_pane.clone()
                        } else {
                            temp_pane.clone()
                        }
                    }
                    SplitSlotFill::NewShell => {
                        match self.tmux.split_pane(&temp_pane, SplitDirection::SideBySide) {
                            Ok(p) => p.id,
                            Err(error) => {
                                self.status = error.to_string();
                                self.recover_split_studio(studio);
                                return;
                            }
                        }
                    }
                };
                for pid in &existing_in_anchor {
                    if pid != &inserted
                        && let Err(error) = self.tmux.break_pane(pid, None)
                    {
                        self.status = error.to_string();
                        self.recover_split_studio(studio);
                        return;
                    }
                }
                inserted
            } else {
                self.status = self
                    .theme
                    .tr(
                        "Target window no longer has live panes.",
                        "目标窗口中已没有活跃窗格。",
                    )
                    .into();
                self.recover_split_studio(studio);
                return;
            };
            (anchor_win.session_id, anchor_win.id, first_pane)
        } else if let SplitSlotFill::Window(win_id) = &first_fill
            && let Some(win) = window_map.get(win_id)
            && let Some((pane_id, 1, _)) = rep_panes.get(win_id)
        {
            (win.session_id.clone(), win.id.clone(), pane_id.clone())
        } else {
            let target_sess = match &first_fill {
                SplitSlotFill::Window(win_id) => window_map
                    .get(win_id)
                    .map(|w| w.session_id.clone())
                    .unwrap_or_else(|| studio.anchor_session_id.clone()),
                SplitSlotFill::Pane(_) | SplitSlotFill::NewShell => {
                    studio.anchor_session_id.clone()
                }
            };
            let created = match self.tmux.new_window(&target_sess, None, anchor_cwd) {
                Ok(w) => w,
                Err(error) => {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
            };
            let created_panes = match self.tmux.list_panes(Some(&created.id)) {
                Ok(p) => p,
                Err(error) => {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
            };
            let Some(fresh_pane) = created_panes.first() else {
                self.status = "Created window has no active pane.".into();
                self.recover_split_studio(studio);
                return;
            };
            let anchor_pane = if let SplitSlotFill::Window(win_id) = &first_fill
                && let Some((src_pane, _, _)) = rep_panes.get(win_id)
            {
                if let Err(error) =
                    self.tmux
                        .join_pane(src_pane, &fresh_pane.id, SplitDirection::SideBySide)
                {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
                if let Err(error) = self.tmux.kill_pane(&fresh_pane.id) {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
                src_pane.clone()
            } else if let SplitSlotFill::Pane(src_pane) = &first_fill {
                if let Err(error) =
                    self.tmux
                        .join_pane(src_pane, &fresh_pane.id, SplitDirection::SideBySide)
                {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
                if let Err(error) = self.tmux.kill_pane(&fresh_pane.id) {
                    self.status = error.to_string();
                    self.recover_split_studio(studio);
                    return;
                }
                src_pane.clone()
            } else {
                fresh_pane.id.clone()
            };
            window_map.insert(created.id.clone(), created.clone());
            (target_sess, created.id, anchor_pane)
        };

        let mut slot_panes: HashMap<SlotId, PaneId> = HashMap::new();
        if let Err(error) = self.materialize_split_subtree(
            &studio.root,
            &first_anchor_pane,
            &rep_panes,
            &mut slot_panes,
        ) {
            self.status = error.to_string();
            self.recover_split_studio(studio);
            return;
        }
        self.apply_split_ratios(&studio.root, &slot_panes);
        if studio.sync_panes || studio.sync_panes != studio.initial_sync_panes {
            let _ = self
                .tmux
                .set_synchronize_panes(&target_window_id, studio.sync_panes);
        }

        if let Some(selected_pane_id) = slot_panes.get(&studio.selected_slot) {
            let _ = self.tmux.select(&Target::Pane(selected_pane_id.clone()));
        }
        self.refresh_home_data();
        self.enter(Target::Window(target_session_id, target_window_id));
    }

    fn apply_split_ratios(&self, node: &SplitNode, slot_panes: &HashMap<SlotId, PaneId>) {
        if let SplitNode::Split {
            direction,
            ratio,
            first,
            second,
        } = node
        {
            if *ratio != split::DEFAULT_SPLIT_RATIO {
                let first_slot = first.first_leaf().0;
                if let Some(pane_id) = slot_panes.get(&first_slot) {
                    let pct = (*ratio).clamp(split::MIN_SPLIT_RATIO, split::MAX_SPLIT_RATIO) as u8;
                    let (w, h) = match direction {
                        SplitDirection::SideBySide => (Some(pct), None),
                        SplitDirection::Stacked => (None, Some(pct)),
                    };
                    let _ = self.tmux.resize_pane_percent(pane_id, w, h);
                }
            }
            self.apply_split_ratios(first, slot_panes);
            self.apply_split_ratios(second, slot_panes);
        }
    }

    fn materialize_split_subtree(
        &self,
        node: &SplitNode,
        first_anchor: &PaneId,
        rep_panes: &HashMap<WindowId, (PaneId, usize, Option<String>)>,
        slot_panes: &mut HashMap<SlotId, PaneId>,
    ) -> TmuxResult<()> {
        match node {
            SplitNode::Leaf { id, .. } => {
                slot_panes.insert(*id, first_anchor.clone());
                Ok(())
            }
            SplitNode::Split {
                direction,
                first,
                second,
                ..
            } => {
                let second_anchor = match second.first_leaf().1 {
                    SplitSlotFill::Window(win_id) => {
                        if let Some((src_pane, _, _)) = rep_panes.get(&win_id)
                            && src_pane != first_anchor
                        {
                            self.tmux.join_pane(src_pane, first_anchor, *direction)?;
                            src_pane.clone()
                        } else {
                            self.tmux.split_pane(first_anchor, *direction)?.id
                        }
                    }
                    SplitSlotFill::Pane(src_pane) => {
                        if &src_pane != first_anchor {
                            self.tmux.join_pane(&src_pane, first_anchor, *direction)?;
                            src_pane
                        } else {
                            self.tmux.split_pane(first_anchor, *direction)?.id
                        }
                    }
                    SplitSlotFill::NewShell => self.tmux.split_pane(first_anchor, *direction)?.id,
                };
                self.materialize_split_subtree(first, first_anchor, rep_panes, slot_panes)?;
                self.materialize_split_subtree(second, &second_anchor, rep_panes, slot_panes)
            }
        }
    }

    fn ui_toolbar_to_action(button: SplitToolbarButton) -> SplitToolbarAction {
        match button {
            SplitToolbarButton::SplitHorizontal => SplitToolbarAction::SplitHorizontal,
            SplitToolbarButton::SplitVertical => SplitToolbarAction::SplitVertical,
            SplitToolbarButton::FlipDirection => SplitToolbarAction::FlipDirection,
            SplitToolbarButton::RatioPreset => SplitToolbarAction::RatioPreset,
            SplitToolbarButton::SyncToggle => SplitToolbarAction::SyncToggle,
            SplitToolbarButton::PresetGrid => SplitToolbarAction::PresetGrid,
            SplitToolbarButton::BreakOut => SplitToolbarAction::BreakOut,
            SplitToolbarButton::RemoveSlot => SplitToolbarAction::RemoveSlot,
            SplitToolbarButton::Reset => SplitToolbarAction::Reset,
            SplitToolbarButton::ApplyAndEnter => SplitToolbarAction::ApplyAndEnter,
        }
    }

    fn build_split_canvas_node<'s>(
        &self,
        studio: &'s SplitStudio,
        node: &'s SplitNode,
    ) -> SplitCanvasNode<'s> {
        match node {
            SplitNode::Leaf { id, fill } => {
                let ordinal = studio.slot_ordinal(*id).unwrap_or(1);
                let preview_lines: &'s [String] = match fill {
                    SplitSlotFill::Window(wid) => {
                        studio.previews.get(wid).map(Vec::as_slice).unwrap_or(&[])
                    }
                    SplitSlotFill::Pane(pid) => studio
                        .pane_previews
                        .get(pid)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    SplitSlotFill::NewShell => &[],
                };
                let (title, subtitle, is_new_shell) = match fill {
                    SplitSlotFill::Window(win_id) => {
                        if let Some(opt) = studio.window_option(win_id) {
                            (
                                format!(
                                    "{}:{} {}",
                                    opt.window.session_name, opt.window.index, opt.window.name
                                ),
                                opt.command.clone(),
                                false,
                            )
                        } else {
                            (win_id.as_str().to_owned(), String::new(), false)
                        }
                    }
                    SplitSlotFill::Pane(pane_id) => {
                        if let Some(info) = studio.pane_info(pane_id) {
                            let label = if !info.title.trim().is_empty() {
                                format!(".{} {}", info.index, info.title)
                            } else {
                                format!(".{} {}", info.index, info.command)
                            };
                            (label, info.command.clone(), false)
                        } else {
                            (pane_id.as_str().to_owned(), String::new(), false)
                        }
                    }
                    SplitSlotFill::NewShell => (
                        self.theme.tr("+ New shell", "+ 新建终端").to_owned(),
                        self.theme
                            .tr("Opens fresh shell in tmux", "进入 tmux 时新建终端")
                            .to_owned(),
                        true,
                    ),
                };
                SplitCanvasNode::Pane(SplitCanvasPaneView {
                    slot_id: *id,
                    ordinal,
                    title,
                    subtitle,
                    selected: studio.selected_slot == *id,
                    is_new_shell,
                    preview_lines,
                    preview_scroll: studio.preview_scrolls.get(id).copied().unwrap_or(0),
                })
            }
            SplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => SplitCanvasNode::Split {
                horizontal: *direction == SplitDirection::SideBySide,
                ratio: *ratio,
                first: Box::new(self.build_split_canvas_node(studio, first)),
                second: Box::new(self.build_split_canvas_node(studio, second)),
            },
        }
    }

    fn try_run_natural_phrase(&mut self, trimmed: &str) -> bool {
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
            return true;
        }
        if lower == "new" || lower.starts_with("new ") || lower.starts_with("theme ") {
            self.run_parsed_command(commands::parse(&format!("/{trimmed}")));
            return true;
        }
        if matches!(lower.as_str(), "split right" | "hsplit") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.split_quick_direction(SplitDirection::SideBySide);
            return true;
        }
        if matches!(lower.as_str(), "split down" | "vsplit") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.split_quick_direction(SplitDirection::Stacked);
            return true;
        }
        if matches!(lower.as_str(), "zoom" | "unzoom" | "zoom pane") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.refresh_home_data();
            if let Some(pane) = self.focused_or_active_pane() {
                self.toggle_zoom_pane_action(&pane);
            } else {
                self.status = self
                    .theme
                    .tr("No active pane to zoom.", "没有可缩放的活跃窗格。")
                    .into();
            }
            return true;
        }
        if matches!(lower.as_str(), "break pane" | "break out" | "break") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.refresh_home_data();
            if let Some(pane) = self.focused_or_active_pane() {
                self.break_pane_action(&pane);
            } else {
                self.status = self
                    .theme
                    .tr("No active pane to break out.", "没有可拆出的活跃窗格。")
                    .into();
            }
            return true;
        }
        if matches!(
            lower.as_str(),
            "sync panes" | "sync" | "synchronize panes" | "unsync panes"
        ) {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.refresh_home_data();
            if let Some(window) = self.focused_or_active_window() {
                self.toggle_sync_panes_action(&window);
            } else {
                self.status = self
                    .theme
                    .tr(
                        "No active window to synchronize.",
                        "没有可切换同步输入的窗口。",
                    )
                    .into();
            }
            return true;
        }
        if matches!(lower.as_str(), "detach") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.detach();
            return true;
        }
        if matches!(
            lower.as_str(),
            "detach others" | "detach other clients" | "fix dotted border"
        ) {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.detach_other_clients_action(None);
            return true;
        }
        if matches!(lower.as_str(), "clients" | "list clients") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.open_clients_view();
            return true;
        }
        if matches!(lower.as_str(), "buffers" | "paste buffers" | "clipboard") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.open_buffers_view();
            return true;
        }
        if matches!(lower.as_str(), "save workspace" | "save snapshot") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.views.push(View::Form(Form {
                action: FormAction::SaveSnapshotName {
                    session_filter: None,
                },
                input: TextInput::default(),
            }));
            return true;
        }
        if matches!(
            lower.as_str(),
            "restore workspace" | "restore snapshot" | "snapshots"
        ) {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.open_snapshots_view();
            return true;
        }
        if matches!(
            lower.as_str(),
            "reload tmux" | "reload conf" | "reload tmux.conf"
        ) {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.reload_tmux_conf_action();
            return true;
        }
        if matches!(lower.as_str(), "popup" | "scratchpad") {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.open_popup_shell();
            return true;
        }
        if matches!(
            lower.as_str(),
            "expose" | "mission control" | "mission" | "wall"
        ) {
            self.status.clear();
            self.composer.set_value("");
            self.palette_selected = 0;
            self.open_expose_view();
            return true;
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
                    return true;
                }
            }
        }
        false
    }

    fn submit_home_input(&mut self) {
        let text = self.composer.value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            self.activate_matrix_selection();
            return;
        }
        if trimmed.starts_with('/') || trimmed.starts_with('@') {
            self.execute_text(&text);
            return;
        }
        if self.try_run_natural_phrase(trimmed) {
            return;
        }
        if !trimmed.contains(char::is_whitespace)
            && let Ok(parsed) = commands::parse(&format!("/{trimmed}"))
        {
            let non_matrix = matches!(
                parsed.command,
                CommandKind::Update
                    | CommandKind::Help
                    | CommandKind::Config
                    | CommandKind::Quit
                    | CommandKind::Exit
                    | CommandKind::Theme
                    | CommandKind::Layout
                    | CommandKind::Expose
                    | CommandKind::Clients
                    | CommandKind::Buffers
                    | CommandKind::Snapshot
                    | CommandKind::Popup
                    | CommandKind::KillServer
                    | CommandKind::Tribute(_)
                    | CommandKind::MicroTribute(_)
            );
            if non_matrix || !self.matrix.has_visible_matches() {
                self.run_parsed_command(Ok(parsed));
                return;
            }
        }
        if self.matrix.has_visible_matches() {
            self.activate_matrix_selection();
            return;
        }
        self.run_parsed_command(commands::parse(&text));
    }

    fn sync_action_scope_with_matrix(&mut self) {
        let scope = match self.matrix.selection() {
            Some(MatrixSelection::Session(_)) => TargetScope::Session,
            Some(MatrixSelection::Window(_, _)) => TargetScope::Window,
            _ => return,
        };
        match &mut self.matrix_action {
            MatrixAction::Rename(current) => *current = scope,
            MatrixAction::Kill(current) => *current = scope,
            _ => {}
        }
    }

    fn activate_matrix_selection(&mut self) {
        enum SelectedMatrixTarget {
            Session(Session),
            Window(Session, Window),
            NewWindow(Session),
            NewSession,
        }

        let target = match self.matrix.selection() {
            Some(MatrixSelection::Session(session)) => {
                Some(SelectedMatrixTarget::Session(session.clone()))
            }
            Some(MatrixSelection::Window(session, window)) => Some(SelectedMatrixTarget::Window(
                session.clone(),
                window.clone(),
            )),
            Some(MatrixSelection::NewWindow(session)) => {
                Some(SelectedMatrixTarget::NewWindow(session.clone()))
            }
            Some(MatrixSelection::NewSession) => Some(SelectedMatrixTarget::NewSession),
            None if self.home_sessions.is_empty() => Some(SelectedMatrixTarget::NewSession),
            None => None,
        };
        let Some(target) = target else {
            return;
        };

        match target {
            SelectedMatrixTarget::NewSession => {
                self.status.clear();
                self.composer.set_value("");
                self.sync_home_matrix();
                self.views.push(View::Form(Form {
                    action: FormAction::NewSessionName,
                    input: TextInput::default(),
                }));
            }
            SelectedMatrixTarget::NewWindow(session) => {
                self.status.clear();
                self.composer.set_value("");
                self.sync_home_matrix();
                self.views
                    .push(View::DualForm(DualForm::new_window(session.id)));
            }
            SelectedMatrixTarget::Session(session) => match self.matrix_action {
                MatrixAction::Open => {
                    self.enter(Target::Session(session.id));
                    if self.exit.is_some() {
                        self.composer.set_value("");
                    }
                }
                MatrixAction::Rename(_) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.rename_form(Resource::Session(session));
                }
                MatrixAction::Kill(_) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.kill_confirm(Resource::Session(session));
                }
                MatrixAction::Split => {
                    self.open_split_studio_from_home(Resource::Session(session));
                }
                MatrixAction::Window => {
                    self.status.clear();
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.views
                        .push(View::DualForm(DualForm::new_window(session.id)));
                }
                MatrixAction::Inspect => {
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.views
                        .push(View::Actions(Resource::Session(session), 0));
                }
            },
            SelectedMatrixTarget::Window(session, window) => match self.matrix_action {
                MatrixAction::Open => {
                    self.enter(Target::Window(session.id, window.id));
                    if self.exit.is_some() {
                        self.composer.set_value("");
                    }
                }
                MatrixAction::Rename(TargetScope::Window) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.rename_form(Resource::Window(window));
                }
                MatrixAction::Rename(TargetScope::Session) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.rename_form(Resource::Session(session));
                }
                MatrixAction::Kill(TargetScope::Window) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.kill_confirm(Resource::Window(window));
                }
                MatrixAction::Kill(TargetScope::Session) => {
                    self.matrix_action = MatrixAction::Open;
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.kill_confirm(Resource::Session(session));
                }
                MatrixAction::Split => {
                    self.open_split_studio_from_home(Resource::Window(window));
                }
                MatrixAction::Window => {
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.views.push(View::Actions(Resource::Window(window), 0));
                }
                MatrixAction::Inspect => {
                    self.composer.set_value("");
                    self.sync_home_matrix();
                    self.views
                        .push(View::Actions(Resource::Session(session), 0));
                }
            },
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
                        | CommandKind::Layout
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
                    self.status = if self.theme.language.is_zh() {
                        format!("未找到匹配 '{token}' 的目标。")
                    } else {
                        format!("No target matching '{token}'.")
                    };
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
                self.status = if self.theme.language.is_zh() {
                    format!("未找到匹配 '{trimmed}' 的目标。")
                } else {
                    format!("No target matching '{trimmed}'.")
                };
            }
            return;
        }

        if !trimmed.is_empty() {
            if let Ok(command) = commands::parse(&format!("/{trimmed}")) {
                self.run_parsed_command(Ok(command));
                return;
            }
            if self.try_run_natural_phrase(trimmed) {
                return;
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
                } else if self.theme.language.is_zh() {
                    format!("{message}。输入 / 可选择命令。")
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
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Open;
                self.refresh_home_data();
            }
            C::List => {
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Inspect;
                self.refresh_home_data();
            }
            C::Rename => {
                self.views = ViewStack::default();
                self.refresh_home_data();
                self.matrix.focus_session_header();
                self.matrix_action = MatrixAction::Rename(TargetScope::Session);
                self.refresh_home_preview();
            }
            C::Kill => {
                self.views = ViewStack::default();
                if !parsed.args.is_empty() {
                    let target_name = parsed.args.join(" ");
                    self.refresh_home_data();
                    if let Some(resource) = self.resolve_mention_resource(&target_name) {
                        self.kill_confirm(resource);
                        return;
                    }
                    self.composer.set_value(target_name);
                    self.matrix_action = MatrixAction::Kill(TargetScope::Session);
                    self.sync_home_matrix();
                    self.refresh_home_preview();
                    return;
                }
                self.refresh_home_data();
                self.matrix.focus_window_row();
                let scope = if self.matrix.is_session_selected() {
                    TargetScope::Session
                } else {
                    TargetScope::Window
                };
                self.matrix_action = MatrixAction::Kill(scope);
                self.refresh_home_preview();
            }
            C::Window => {
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Window;
                self.refresh_home_data();
            }
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
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Split;
                self.refresh_home_data();
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
                        self.status = theme_toast(preset, self.theme.language);
                        return;
                    }
                    self.status = if self.theme.language.is_zh() {
                        format!("未知主题 '{arg}'，请从下方选择预设。")
                    } else {
                        format!("Unknown theme '{arg}'. Pick a preset below.")
                    };
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
            C::Update => {
                self.status = self
                    .theme
                    .tr(
                        "✦ Run 'jm update' (or 'justmux update') in your terminal to check & install updates.",
                        "✦ 请在终端运行 'jm update'（或 'justmux update'）检查并安装更新。",
                    )
                    .to_owned();
            }
            C::Layout => {
                if let Some(arg) = parsed.args.first() {
                    let low = arg.to_ascii_lowercase();
                    let preset = match low.as_str() {
                        "even-horizontal" | "eh" | "horizontal" => {
                            Some(LayoutPreset::EvenHorizontal)
                        }
                        "even-vertical" | "ev" | "vertical" => Some(LayoutPreset::EvenVertical),
                        "main-horizontal" | "mh" => Some(LayoutPreset::MainHorizontal),
                        "main-vertical" | "mv" => Some(LayoutPreset::MainVertical),
                        "tiled" | "grid" => Some(LayoutPreset::Tiled),
                        _ => None,
                    };
                    if let Some(preset) = preset {
                        self.refresh_home_data();
                        if let Some(window) = self.focused_or_active_window() {
                            match self.tmux.select_layout(&window.id, preset) {
                                Ok(()) => {
                                    self.refresh_home_data();
                                    let label = preset.label(self.theme.language.is_zh());
                                    let msg = if self.theme.language.is_zh() {
                                        format!("✦ 已应用布局：{label}")
                                    } else {
                                        format!("✦ Applied layout: {label}")
                                    };
                                    self.push_toast(ToastKind::Success, msg);
                                }
                                Err(error) => self.push_toast(ToastKind::Error, error.to_string()),
                            }
                            return;
                        }
                    }
                }
                self.open_layout_studio(None);
            }
            C::Expose => self.open_expose_view(),
            C::Clients => self.open_clients_view(),
            C::Buffers => self.open_buffers_view(),
            C::Snapshot => {
                if let Some(sub) = parsed.args.first().map(|s| s.to_ascii_lowercase()) {
                    match sub.as_str() {
                        "save" => {
                            if let Some(name) = parsed.args.get(1) {
                                match self.save_snapshot_by_name(name, None) {
                                    Ok(snap) => {
                                        self.status = if self.theme.language.is_zh() {
                                            format!(
                                                "✦ 已保存工作区快照 '{}'（{} 会话 / {} 窗口）",
                                                snap.name,
                                                snap.session_count(),
                                                snap.window_count()
                                            )
                                        } else {
                                            format!(
                                                "✦ Saved snapshot '{}' ({} session(s), {} window(s))",
                                                snap.name,
                                                snap.session_count(),
                                                snap.window_count()
                                            )
                                        };
                                    }
                                    Err(err) => self.status = err,
                                }
                            } else {
                                self.views.push(View::Form(Form {
                                    action: FormAction::SaveSnapshotName {
                                        session_filter: None,
                                    },
                                    input: TextInput::default(),
                                }));
                            }
                            return;
                        }
                        "restore" => {
                            if let Some(name) = parsed.args.get(1) {
                                self.restore_snapshot_by_name(name);
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                self.open_snapshots_view();
            }
            C::Popup => self.open_popup_shell(),
            C::KillServer => self.open_kill_server_confirm(),
            C::Exit => self.detach(),
            C::Quit => self.exit = Some(Exit::Quit),
        }
    }

    fn read_resources(&self, purpose: PickerPurpose) -> TmuxResult<Vec<Resource>> {
        match purpose {
            PickerPurpose::Panes => self
                .tmux
                .list_panes(None)
                .map(|rows| rows.into_iter().map(Resource::Pane).collect()),
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

    fn sync_home_matrix(&mut self) {
        let raw = self.composer.value();
        let trimmed = raw.trim_start();
        let filter = if self.composer_mode == ComposerMode::QuickLaunch
            || trimmed.starts_with('/')
            || trimmed.starts_with('@')
        {
            ""
        } else {
            raw
        };
        self.matrix.sync(
            self.home_sessions.clone(),
            self.home_windows.clone(),
            filter,
            &mut self.matcher,
        );
        self.sync_action_scope_with_matrix();
    }

    fn refresh_home_data(&mut self) {
        let Some(sessions) = self.recent_sessions().ok() else {
            self.home_sessions.clear();
            self.home_windows.clear();
            self.home_panes.clear();
            self.client_count = None;
            self.telemetry = CachedTelemetry::default();
            self.radar_selected = 0;
            self.sync_home_matrix();
            self.cached_preview = None;
            return;
        };
        if sessions.is_empty() {
            self.home_sessions = sessions;
            self.home_windows.clear();
            self.home_panes.clear();
            self.client_count = Some(0);
            self.telemetry = CachedTelemetry {
                sessions: Some(0),
                windows: Some(0),
                attached: Some(0),
            };
            self.radar_selected = 0;
            self.sync_home_matrix();
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
        self.client_count = self.tmux.list_clients().ok().map(|c| c.len());
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
        self.sync_home_matrix();
        if self.views.top().is_none() {
            self.refresh_home_preview();
        }
    }

    fn clear_preview(&mut self) {
        self.cached_preview = None;
        self.preview_target = None;
        self.preview_scroll = 0;
        self.home_preview_pane_total = 0;
        self.preview_search.reset();
        self.sync_preview_desktop_metadata();
    }

    fn preview_visible_rows(&self) -> usize {
        self.desktop
            .preview_layout
            .content_rect
            .map(|r| usize::from(r.height).max(1))
            .unwrap_or_else(|| {
                self.preview_rect
                    .map_or(10, |r| usize::from(r.height.saturating_sub(2)).max(1))
            })
    }

    fn max_preview_scroll(&self) -> usize {
        if !self.config.live_preview {
            return 0;
        }
        self.cached_preview
            .as_ref()
            .map_or(0, |(_, _, lines)| lines.len())
            .saturating_sub(self.preview_visible_rows())
    }

    fn scroll_preview_up(&mut self, lines: usize) {
        if !self.config.live_preview {
            return;
        }
        self.preview_scroll = self
            .preview_scroll
            .saturating_add(lines)
            .min(self.max_preview_scroll());
    }

    fn scroll_preview_down(&mut self, lines: usize) {
        if !self.config.live_preview {
            return;
        }
        let max = self.max_preview_scroll();
        self.preview_scroll = self.preview_scroll.min(max).saturating_sub(lines);
    }

    fn page_preview(&mut self, up: bool, half: bool) {
        let rows = self.preview_visible_rows();
        let step = if half {
            (rows / 2).max(1)
        } else {
            rows.saturating_sub(2).max(1)
        };
        if up {
            self.scroll_preview_up(step);
        } else {
            self.scroll_preview_down(step);
        }
    }

    fn handle_preview_paging_key(&mut self, key: KeyEvent, input_empty: bool) -> bool {
        if !self.config.live_preview || key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::PageUp => {
                self.page_preview(true, false);
                true
            }
            KeyCode::PageDown => {
                self.page_preview(false, false);
                true
            }
            KeyCode::Char('u' | 'U') if input_empty && ctrl => {
                self.page_preview(true, true);
                true
            }
            KeyCode::Char('d' | 'D') if input_empty && ctrl => {
                self.page_preview(false, true);
                true
            }
            KeyCode::Home if input_empty && !ctrl => {
                self.preview_scroll = self.max_preview_scroll();
                true
            }
            KeyCode::End if input_empty && !ctrl => {
                self.preview_scroll = 0;
                true
            }
            _ => false,
        }
    }

    fn scroll_preview_wheel(&mut self, column: u16, row: u16, up: bool) -> bool {
        if self.config.live_preview
            && self
                .preview_rect
                .is_some_and(|rect| rect.contains((column, row).into()))
        {
            if up {
                self.scroll_preview_up(PREVIEW_WHEEL_STEP);
            } else {
                self.scroll_preview_down(PREVIEW_WHEEL_STEP);
            }
            true
        } else {
            false
        }
    }

    fn capture_and_store_preview(&mut self, target: Target, title: String, subtitle: String) {
        let lines = self
            .tmux
            .capture_pane(&target, PREVIEW_CAPTURE_LINES)
            .ok()
            .unwrap_or_default();
        let max_scroll = lines.len().saturating_sub(self.preview_visible_rows());
        if self.preview_target.as_ref() != Some(&target) {
            self.preview_target = Some(target);
            self.preview_scroll = 0;
        } else {
            self.preview_scroll = self.preview_scroll.min(max_scroll);
        }
        self.cached_preview = Some((title, subtitle, lines));
        self.sync_preview_desktop_metadata();
        if self.preview_search.query_str().is_some() {
            self.recompute_preview_search(false);
        }
    }

    fn refresh_home_preview(&mut self) {
        if !self.config.live_preview {
            self.clear_preview();
            return;
        }
        if self.composer_mode == ComposerMode::LiveRadar {
            self.home_preview_pane_total = 0;
            let Some(session) = self.home_sessions.get(self.radar_selected) else {
                self.clear_preview();
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
            self.capture_and_store_preview(
                Target::Session(session.id.clone()),
                session.name.clone(),
                subtitle,
            );
            return;
        }
        if matches!(self.matrix.selection(), Some(MatrixSelection::NewSession))
            && !self.matrix.is_empty()
        {
            self.preview_target = None;
            self.preview_scroll = 0;
            self.home_preview_pane_total = 0;
            self.cached_preview = Some((
                self.theme.tr("New Session", "新建会话").into(),
                self.theme
                    .tr("create tmux workspace", "创建 tmux 工作区")
                    .into(),
                vec![
                    self.theme
                        .tr(
                            "Press Enter to name and launch a new tmux session.",
                            "按 Enter 输入名称并启动新的 tmux 会话。",
                        )
                        .into(),
                    self.theme
                        .tr(
                            "Tip: Leave working directory empty to use the current folder.",
                            "提示：工作目录留空将默认使用当前目录。",
                        )
                        .into(),
                ],
            ));
            return;
        }
        let Some((session, window)) = self
            .matrix
            .preview_window()
            .map(|(s, w)| (s.clone(), w.clone()))
        else {
            self.clear_preview();
            return;
        };
        let mut window_panes: Vec<Pane> = self
            .home_panes
            .iter()
            .filter(|p| p.window_id == window.id)
            .cloned()
            .collect();
        window_panes.sort_by_key(|p| p.index);
        if self.home_preview_pane_window.as_ref() != Some(&window.id) {
            self.home_preview_pane_window = Some(window.id.clone());
            self.home_preview_pane_idx = window_panes.iter().position(|p| p.active).unwrap_or(0);
        }
        self.home_preview_pane_total = window_panes.len();
        if window_panes.len() > 1 {
            let idx = self.home_preview_pane_idx.min(window_panes.len() - 1);
            self.home_preview_pane_idx = idx;
            let pane = &window_panes[idx];
            let target = Target::Pane(pane.id.clone());
            let subtitle = format!(
                "{}: {} · .{} {}",
                window.index, window.name, pane.index, pane.current_command
            );
            self.capture_and_store_preview(target, session.name, subtitle);
        } else {
            self.home_preview_pane_idx = 0;
            let target = Target::Window(session.id.clone(), window.id.clone());
            let subtitle = format!("{}: {}", window.index, window.name);
            self.capture_and_store_preview(target, session.name, subtitle);
        }
    }

    fn refresh_radar_preview(&mut self) {
        self.refresh_home_preview();
    }

    fn update_picker_preview(&mut self, picker: &Picker) {
        if !self.config.live_preview {
            self.clear_preview();
            return;
        }
        let rows = picker.rows(&mut self.matcher);
        let Some(PickerRow::Resource(index)) = rows.get(picker.selected).copied() else {
            self.clear_preview();
            return;
        };
        let Some(resource) = picker.resources.get(index) else {
            self.clear_preview();
            return;
        };
        let target = resource.target();
        let title = resource.label();
        let subtitle = match resource {
            Resource::Session(s) => format!("{}w", s.windows),
            Resource::Window(w) => w.id.to_string(),
            Resource::Pane(p) => p.id.to_string(),
        };
        self.capture_and_store_preview(target, title, subtitle);
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
        let mut query = TextInput::default();
        if !initial_query.is_empty() {
            query.set_value(initial_query);
        }
        let picker = Picker {
            purpose,
            query,
            resources,
            selected: 0,
        };
        self.update_picker_preview(&picker);
        self.views.push(View::Picker(picker));
    }

    fn picker_key(&mut self, mut picker: Picker, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if self.handle_preview_paging_key(key, picker.query.value().is_empty()) {
            self.views.push(View::Picker(picker));
            return;
        }
        if self.config.live_preview
            && self.cached_preview.is_some()
            && ((ctrl && !alt && matches!(key.code, KeyCode::Char('f' | 'F')))
                || (picker.query.value().is_empty()
                    && key.modifiers.is_empty()
                    && key.code == KeyCode::Char('f')))
        {
            self.open_preview_search();
            self.views.push(View::Picker(picker));
            return;
        }
        if self.config.live_preview
            && self.cached_preview.is_some()
            && ((ctrl && !alt && matches!(key.code, KeyCode::Char('y' | 'Y')))
                || (picker.query.value().is_empty()
                    && key.modifiers.is_empty()
                    && key.code == KeyCode::Char('y')))
        {
            self.copy_current_preview_to_clipboard();
            self.views.push(View::Picker(picker));
            return;
        }
        if picker.query.value().is_empty()
            && !ctrl
            && !alt
            && self.preview_search.query_str().is_some()
        {
            if key.code == KeyCode::Char('n') && key.modifiers.is_empty() {
                self.step_preview_search_match(true);
                self.views.push(View::Picker(picker));
                return;
            }
            if key.code == KeyCode::Char('N') {
                self.step_preview_search_match(false);
                self.views.push(View::Picker(picker));
                return;
            }
        }
        match key.code {
            KeyCode::Up => {
                let len = picker.rows(&mut self.matcher).len();
                picker.selected = wrap_step(picker.selected, len, -1);
                self.update_picker_preview(&picker);
            }
            KeyCode::Down => {
                let len = picker.rows(&mut self.matcher).len();
                picker.selected = wrap_step(picker.selected, len, 1);
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
                self.views.push(View::Picker(picker));
                self.views.push(View::Actions(resource, 0));
            }
        }
    }

    fn mouse_scroll(&mut self, column: u16, row: u16, up: bool) {
        let delta = if up { -1 } else { 1 };
        match self.views.pop() {
            None => {
                if commands::popup_query(self.composer.value()).is_some() {
                    let count = self.palette().len();
                    if count > 0 {
                        self.palette_selected = wrap_step(self.palette_selected, count, delta);
                    }
                } else if self.active_mention().is_some() {
                    let count = self.mention_candidates().len();
                    if count > 0 {
                        self.mention_selected = wrap_step(self.mention_selected, count, delta);
                    }
                } else if self.scroll_preview_wheel(column, row, up) {
                } else if self.composer_mode == ComposerMode::LiveRadar
                    && !self.home_sessions.is_empty()
                {
                    self.radar_selected =
                        wrap_step(self.radar_selected, self.home_sessions.len(), delta);
                    self.refresh_radar_preview();
                } else if self.composer_mode == ComposerMode::CommandJump
                    && self.matrix.move_row(delta)
                {
                    self.sync_action_scope_with_matrix();
                    self.refresh_home_preview();
                }
            }
            Some(View::Picker(mut picker)) => {
                if !self.scroll_preview_wheel(column, row, up) {
                    let count = picker.rows(&mut self.matcher).len();
                    picker.selected = wrap_step(picker.selected, count, delta);
                    self.update_picker_preview(&picker);
                }
                self.views.push(View::Picker(picker));
            }
            Some(View::TargetPick(mut state)) => {
                if !self.scroll_preview_wheel(column, row, up) {
                    let count = state.filtered_indices(&mut self.matcher).len();
                    state.selected = wrap_step(state.selected, count, delta);
                    self.update_target_pick_preview(&state);
                }
                self.views.push(View::TargetPick(state));
            }
            Some(View::Actions(resource, mut selected)) => {
                let count = resource
                    .actions_for(&self.piping_panes, self.theme.language)
                    .len();
                selected = wrap_step(selected, count, delta);
                self.views.push(View::Actions(resource, selected));
            }
            Some(View::LayoutStudio(mut state)) => {
                state.selected_preset =
                    wrap_step(state.selected_preset, LayoutPreset::ALL.len(), delta);
                self.views.push(View::LayoutStudio(state));
            }
            Some(View::Expose(mut state)) => {
                let count = state.tiles.len();
                state.selected = wrap_step(state.selected, count, delta);
                state.sync_page_to_selected();
                self.views.push(View::Expose(state));
            }
            Some(View::Clients(mut state)) => {
                let count = state.clients.len();
                state.selected = wrap_step(state.selected, count, delta);
                self.views.push(View::Clients(state));
            }
            Some(View::Buffers(mut state)) => {
                if self
                    .preview_rect
                    .is_some_and(|rect| rect.contains((column, row).into()))
                {
                    let max = state.preview_lines.len().saturating_sub(1);
                    state.preview_scroll = if up {
                        state.preview_scroll.saturating_sub(PREVIEW_WHEEL_STEP)
                    } else {
                        (state.preview_scroll + PREVIEW_WHEEL_STEP).min(max)
                    };
                } else {
                    let count = state.buffers.len();
                    state.selected = wrap_step(state.selected, count, delta);
                    self.refresh_selected_buffer_preview(&mut state);
                }
                self.views.push(View::Buffers(state));
            }
            Some(View::Snapshots(mut state)) => {
                if self
                    .preview_rect
                    .is_some_and(|rect| rect.contains((column, row).into()))
                {
                    let max = state.tree_preview.len().saturating_sub(1);
                    state.preview_scroll = if up {
                        state.preview_scroll.saturating_sub(PREVIEW_WHEEL_STEP)
                    } else {
                        (state.preview_scroll + PREVIEW_WHEEL_STEP).min(max)
                    };
                } else {
                    let count = state.snapshots.len();
                    state.selected = wrap_step(state.selected, count, delta);
                    self.refresh_selected_snapshot_preview(&mut state);
                }
                self.views.push(View::Snapshots(state));
            }
            Some(View::Split(mut studio)) => {
                if let Some((_, slot_id)) = studio
                    .slot_hits
                    .iter()
                    .find(|(rect, _)| rect.contains((column, row).into()))
                    .copied()
                {
                    self.scroll_split_slot_preview(&mut studio, slot_id, up, PREVIEW_WHEEL_STEP);
                } else {
                    studio.move_window_cursor(up);
                    self.ensure_selected_split_preview(&mut studio);
                }
                self.views.push(View::Split(studio));
            }
            Some(View::ThemePicker {
                initial,
                mut selected,
            }) => {
                selected = wrap_step(selected, ThemePreset::ALL.len(), delta);
                let preset = ThemePreset::ALL.get(selected).copied().unwrap_or(initial);
                self.theme = self.theme.with_preset(preset);
                self.views.push(View::ThemePicker { initial, selected });
            }
            Some(View::Config { mut selected }) => {
                selected = wrap_step(selected, CONFIG_ITEM_COUNT, delta);
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

    fn list_scroll_offset(selected: usize, height: u16) -> usize {
        let visible = usize::from(height);
        if visible > 0 && selected >= visible {
            selected + 1 - visible
        } else {
            0
        }
    }

    fn resource_action_key(&mut self, resource: Resource, mut selected: usize, key: KeyEvent) {
        let actions = resource.actions_for(&self.piping_panes, self.theme.language);
        match key.code {
            KeyCode::Up => selected = wrap_step(selected, actions.len(), -1),
            KeyCode::Down => selected = wrap_step(selected, actions.len(), 1),
            KeyCode::Enter => {
                if let Some((_, action)) = actions.get(selected) {
                    let action = *action;
                    self.execute_resource_action(resource, action);
                    return;
                }
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
        let label = resource.label();
        let zh = self.theme.language.is_zh();
        let mut message = if zh {
            format!("确定要永久关闭 {label} 吗？该目标内的所有进程都将终止。")
        } else {
            format!("Permanently kill {label}? All programs in the target will terminate.")
        };
        if attached {
            message.push_str(if zh {
                " 已附着的客户端将受影响。"
            } else {
                " Attached clients will be affected."
            });
        }
        if busy {
            message.push_str(if zh {
                " 正在运行的任务可能会丢失。"
            } else {
                " A running task may be lost."
            });
        }
        if current {
            message.push_str(if zh {
                " 这是你当前所在会话；tmux 可能会关闭此弹窗或客户端。"
            } else {
                " This is in your current session; tmux may close this popup or client."
            });
        }
        message.push_str(if zh {
            " 输入 yes 后按 Enter 确认，留空则取消。"
        } else {
            " Type yes, then Enter to confirm. Empty input cancels."
        });
        self.views.push(View::Confirm(Confirm {
            target: ConfirmTarget::Resource(Box::new(resource)),
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
                self.status = self
                    .theme
                    .tr("Cancelled. Nothing was killed.", "已取消，未关闭任何目标。")
                    .into();
                return;
            }
            let result = match &confirm.target {
                ConfirmTarget::Resource(res) => match res.as_ref() {
                    Resource::Session(session) => self.tmux.kill_session(&session.id),
                    Resource::Window(window) => self.tmux.kill_window(&window.id),
                    Resource::Pane(pane) => self.tmux.kill_pane(&pane.id),
                },
                ConfirmTarget::KillServer => self.tmux.kill_server(),
            };
            match result {
                Ok(()) => {
                    self.status = match confirm.target {
                        ConfirmTarget::KillServer => self
                            .theme
                            .tr("✦ tmux server stopped.", "✦ tmux 服务已关闭。")
                            .into(),
                        ConfirmTarget::Resource(_) => {
                            self.theme.tr("Target killed", "已关闭目标").into()
                        }
                    };
                    if matches!(confirm.target, ConfirmTarget::KillServer) {
                        self.views = ViewStack::default();
                        self.refresh_home_data();
                    } else {
                        self.refresh_parent();
                    }
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
                        self.status = self.theme.tr("Renamed", "已重命名").into();
                        self.refresh_parent();
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
            FormAction::RenamePaneTitle(pane) => {
                match self.tmux.rename_pane_title(&pane.id, &value) {
                    Ok(()) => {
                        self.status = self
                            .theme
                            .tr("✦ Updated pane title", "✦ 已更新窗格标题")
                            .into();
                        self.refresh_parent();
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
            FormAction::SendCommandToPane(pane) => {
                if value.is_empty() {
                    self.status = self
                        .theme
                        .tr("Command cannot be empty.", "发送的命令不能为空。")
                        .into();
                    self.views.push(View::Form(form));
                    return;
                }
                let result = self
                    .tmux
                    .send_keys(&pane.id, &[value.as_str()], true)
                    .and_then(|()| self.tmux.send_keys(&pane.id, &["Enter"], false));
                match result {
                    Ok(()) => {
                        self.status = if self.theme.language.is_zh() {
                            format!("✦ 已发送命令到窗格 {}", pane.id)
                        } else {
                            format!("✦ Sent command to pane {}", pane.id)
                        };
                        self.refresh_parent();
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
            FormAction::NewBufferContent => {
                if value.is_empty() {
                    self.status = self
                        .theme
                        .tr("Buffer content cannot be empty.", "缓冲区内容不能为空。")
                        .into();
                    self.views.push(View::Form(form));
                    return;
                }
                match self.tmux.set_buffer(&value) {
                    Ok(()) => {
                        self.status = self
                            .theme
                            .tr("✦ Created tmux paste buffer", "✦ 已新建 tmux 粘贴缓冲区")
                            .into();
                        if let Some(View::Buffers(mut state)) = self.views.pop() {
                            self.reload_buffers_state(&mut state);
                            state.status_note = Some(self.status.clone());
                            self.views.push(View::Buffers(state));
                        }
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.views.push(View::Form(form));
                    }
                }
            }
            FormAction::SaveSnapshotName { session_filter } => {
                let filter = session_filter.clone();
                match self.save_snapshot_by_name(value.trim(), filter.as_ref()) {
                    Ok(snap) => {
                        self.status = if self.theme.language.is_zh() {
                            format!(
                                "✦ 已保存工作区快照 '{}'（{} 会话 / {} 窗口）",
                                snap.name,
                                snap.session_count(),
                                snap.window_count()
                            )
                        } else {
                            format!(
                                "✦ Saved snapshot '{}' ({} session(s), {} window(s))",
                                snap.name,
                                snap.session_count(),
                                snap.window_count()
                            )
                        };
                        if let Some(View::Snapshots(mut state)) = self.views.pop() {
                            self.reload_snapshots_state(&mut state);
                            state.status_note = Some(self.status.clone());
                            self.views.push(View::Snapshots(state));
                        }
                    }
                    Err(error) => {
                        self.status = error;
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
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Inspect;
                self.composer.set_value("");
                self.refresh_home_data();
                self.matrix.select_session(&session.id);
                self.refresh_home_preview();
                self.status = self
                    .theme
                    .tr(
                        "Session created. Enter to enter it, or Esc to keep managing.",
                        "会话已创建 · 按 Enter 立即进入，或 Esc 继续管理。",
                    )
                    .into();
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
                self.status = self
                    .theme
                    .tr(
                        "This is a different tmux server. Use tmux's detach key.",
                        "当前处于另一个 tmux 服务，请使用 tmux 原生 detach 快捷键。",
                    )
                    .into()
            }
            _ => {
                self.status = self
                    .theme
                    .tr(
                        "Cannot identify this tmux client. Use the Popup or tmux's detach key.",
                        "无法识别当前 tmux 客户端，请使用 Popup 或 tmux 原生 detach 快捷键。",
                    )
                    .into()
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
                    self.status = self
                        .theme
                        .tr(
                            "That session no longer exists. The list has been refreshed.",
                            "该会话已不存在，列表已自动刷新。",
                        )
                        .into();
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
                Err(error) => {
                    self.status = error.to_string();
                    self.refresh_home_data();
                }
            },
            Context::ForeignServer => {
                self.status = self
                    .theme
                    .tr(
                        "You are inside another tmux server. Open jm outside tmux to enter this server; management still works.",
                        "你当前处于另一个 tmux 服务中，请在 tmux 外部打开 jm 进入该服务；会话管理仍可正常使用。",
                    )
                    .into()
            }
            Context::Unavailable => {
                self.status = self
                    .theme
                    .tr(
                        "The launching tmux client is unavailable. Return to a live tmux client or open jm outside tmux; no nested client was started.",
                        "启动时的 tmux 客户端已不可用，请返回活跃的 tmux 客户端或在 tmux 外部打开 jm（未嵌套启动客户端）。",
                    )
                    .into()
            }
            Context::Outside => match self.tmux.select(&target) {
                Ok(()) => self.exit = Some(Exit::Attach(target)),
                Err(error) => {
                    self.status = error.to_string();
                    self.refresh_home_data();
                }
            },
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let full_area = frame.area();
        self.desktop.clear_render_hits(full_area);
        self.preview_rect = None;
        self.picker_list_rect = None;
        self.subview_list_rect = None;
        self.radar_list_rect = None;
        self.tab_hits = Default::default();
        self.quick_action_hits.clear();
        self.preview_action_hits.clear();
        self.controller_hits.clear();
        self.matrix_hits.clear();
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
                let body_area = if area.height >= 17 {
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas::<2>(area)
                        [1]
                } else {
                    area
                };
                let [welcome, _, _] = Layout::vertical([
                    Constraint::Min(0),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .areas(body_area);
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
            let lang = self.theme.language;
            let items: Vec<_> = commands
                .iter()
                .map(|candidate| CommandItem {
                    name: candidate.command.name,
                    description: commands::description_for(candidate.command.kind, lang),
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
            let (preview_zoomed, preview_synced, preview_piping) = self.preview_status_flags();
            let preview_pip_active = self.desktop.pip_target.as_ref().is_some_and(|pid| {
                self.preview_target.as_ref() == Some(&Target::Pane(pid.clone()))
                    || self
                        .focused_or_active_pane()
                        .is_some_and(|pane| &pane.id == pid)
            });
            let preview_pane_index = (self.composer_mode == ComposerMode::CommandJump
                && self.home_preview_pane_total > 1)
                .then_some((self.home_preview_pane_idx + 1, self.home_preview_pane_total));
            let preview_match_count = self.preview_search.match_lines.len();
            let preview_current_match = if preview_match_count > 0 {
                self.preview_search.current_match + 1
            } else {
                0
            };
            let preview_hovered_action = match self.desktop.hover {
                HoverTarget::PreviewAction(act) => Some(act),
                _ => None,
            };
            let preview_dock_highlight = self.desktop.drag.as_ref().and_then(|d| {
                d.dock_target
                    .and_then(|(rect, zone)| (self.preview_rect == Some(rect)).then_some(zone))
            });
            let active_tab = self.active_desktop_tab();
            let menubar_hovered = match self.desktop.hover {
                HoverTarget::Menubar(item) => Some(item),
                _ => None,
            };
            let menubar_menu_open = self
                .desktop
                .context_menu
                .as_ref()
                .is_some_and(|m| m.from_menubar);
            let inside_tmux = self.context.is_inside();
            let home_split_ratio = self.desktop.home_split_ratio;
            let home_sash_hovered =
                self.desktop.hover == HoverTarget::Sash(SashId::HomePreviewMatrix);
            let home_sash_dragging = self.desktop.is_dragging_sash(SashId::HomePreviewMatrix);
            let scrollbar_hovered = self.desktop.hover == HoverTarget::PreviewScrollbar;
            let scrollbar_dragging = self.desktop.is_dragging_scrollbar();
            let preview_selection = self.desktop.preview_selection;
            let preview_show_actions = self.preview_target.is_some();
            let preview = self
                .cached_preview
                .as_ref()
                .filter(|_| self.config.live_preview)
                .map(|(title, subtitle, lines)| ui::PanePreview {
                    title: title.as_str(),
                    subtitle: subtitle.as_str(),
                    lines: lines.as_slice(),
                    scroll: self.preview_scroll,
                    search_query: self.preview_search.query_str(),
                    search_active: self.preview_search.active,
                    match_count: preview_match_count,
                    current_match: preview_current_match,
                    pane_index: preview_pane_index,
                    zoomed: preview_zoomed,
                    synchronized: preview_synced,
                    piping: preview_piping,
                    show_actions: preview_show_actions,
                    pip_active: preview_pip_active,
                    hovered_action: preview_hovered_action,
                    scrollbar_hovered,
                    scrollbar_dragging,
                    search_match_lines: &self.preview_search.match_lines,
                    error_lines: &self.desktop.preview_error_lines,
                    selection_range: preview_selection,
                    dock_highlight: preview_dock_highlight,
                });
            let menubar = DesktopMenubarState {
                active_tab,
                hovered: menubar_hovered,
                menu_open: menubar_menu_open,
                pip_active: self.desktop.pip_target.is_some(),
                inside_tmux,
                sparkline: &self.desktop.activity_sparkline,
            };
            let (context, session) = match &self.context {
                Context::Outside => (self.theme.tr("standalone", "独立模式"), None),
                Context::Inside(inside) => (
                    if inside.popup {
                        self.theme.tr("popup", "弹窗")
                    } else {
                        self.theme.tr("pane", "窗格")
                    },
                    inside.session_name.as_deref(),
                ),
                Context::ForeignServer => (self.theme.tr("different server", "外部服务"), None),
                Context::Unavailable => (self.theme.tr("client unavailable", "客户端不可用"), None),
            };
            let badge = format!(
                "{} {}",
                self.theme.preset.badge_icon(self.theme.ascii),
                self.theme.preset.slug()
            );
            let highlight_header = matches!(
                self.matrix_action,
                MatrixAction::Rename(TargetScope::Session)
                    | MatrixAction::Kill(TargetScope::Session)
            );
            let dead_windows: HashSet<WindowId> = self
                .home_panes
                .iter()
                .filter(|p| p.dead)
                .map(|p| p.window_id.clone())
                .collect();
            let mut hits = Vec::new();
            let matrix = &mut self.matrix;
            let theme = self.theme;
            let layout = render_workspace_home(
                frame,
                area,
                &self.composer,
                WorkspaceHomeSpec {
                    details: HomeDetails {
                        directory: self.directory.as_deref(),
                        context,
                        session,
                        status: (!self.status.is_empty()).then_some(self.status.as_str()),
                    },
                    telemetry: HomeTelemetry {
                        sessions: self.telemetry.sessions,
                        windows: self.telemetry.windows,
                        attached: self.telemetry.attached,
                        clients: self.client_count,
                        hint_badge: (self.theme.preset != ThemePreset::Classic)
                            .then_some(badge.as_str()),
                    },
                    action: self.matrix_action,
                    composer_mode: self.composer_mode,
                    sessions: &session_entries,
                    mentions: &mention_items,
                    mention_selected: self.mention_selected,
                    radar_selected: self.radar_selected,
                    preview,
                    live_preview: self.config.live_preview,
                    inside_tmux,
                    emblem_mode: self.config.emblem_mode,
                    emblem_step: if self.config.emblem_mode.should_animate() {
                        self.emblem_step
                    } else {
                        0
                    },
                    popup: &items,
                    popup_selected: self.palette_selected,
                    menubar,
                    home_split_ratio,
                    home_sash_hovered,
                    home_sash_dragging,
                },
                theme,
                |frame, body| {
                    hits = matrix.render_matrix_body_with_dead_windows(
                        frame,
                        body,
                        current_session.as_ref(),
                        highlight_header,
                        &dead_windows,
                        theme,
                    );
                },
            );
            self.matrix_hits = hits;
            self.tab_hits = layout.tab_hits;
            self.quick_action_hits = layout.quick_action_hits;
            self.preview_action_hits = layout.preview_action_hits;
            self.preview_rect = layout.preview_area;
            self.hydra_visible = layout.hydra_rendered;
            self.desktop.menubar_hits = layout.menubar_hits;
            self.desktop.sash_hits = layout.sash_hits;
            self.desktop.preview_layout = layout.preview_layout;
            self.render_desktop_overlays_for_frame(frame, full_area);
            return;
        }
        #[allow(clippy::too_many_arguments)]
        fn draw_subview(
            frame: &mut Frame<'_>,
            area: Rect,
            details: HomeDetails<'_>,
            telemetry: HomeTelemetry<'_>,
            emblem: (EmblemMode, usize),
            menubar: DesktopMenubarState<'_>,
            theme: Theme,
            content: SubviewContent<'_>,
        ) -> ui::WorkspaceSubviewLayout {
            render_workspace_subview(
                frame,
                area,
                &WorkspaceSubviewSpec {
                    details,
                    telemetry,
                    emblem_mode: emblem.0,
                    emblem_step: emblem.1,
                    footer_hint: "",
                    content,
                    menubar,
                },
                theme,
            )
        }

        let compact = area.width < 36 || area.height < 12;
        let (subview_area, compact_chunks) = if compact {
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .areas::<3>(area);
            (chunks[0], Some(chunks))
        } else {
            (area, None)
        };
        let picker_rows = match self.views.0.last() {
            Some(View::Picker(picker)) => {
                Some(picker.rich_rows(&mut self.matcher, &self.context, self.theme))
            }
            Some(View::TargetPick(state)) => {
                Some(state.rich_rows(&mut self.matcher, &self.context, self.theme))
            }
            _ => None,
        };
        let (context, session) = match &self.context {
            Context::Outside => (self.theme.tr("standalone", "独立模式"), None),
            Context::Inside(inside) => (
                if inside.popup {
                    self.theme.tr("popup", "弹窗")
                } else {
                    self.theme.tr("pane", "窗格")
                },
                inside.session_name.as_deref(),
            ),
            Context::ForeignServer => (self.theme.tr("different server", "外部服务"), None),
            Context::Unavailable => (self.theme.tr("client unavailable", "客户端不可用"), None),
        };
        let badge = format!(
            "{} {}",
            self.theme.preset.badge_icon(self.theme.ascii),
            self.theme.preset.slug()
        );
        let details = HomeDetails {
            directory: self.directory.as_deref(),
            context,
            session,
            status: (!self.status.is_empty()).then_some(self.status.as_str()),
        };
        let telemetry = HomeTelemetry {
            sessions: self.telemetry.sessions,
            windows: self.telemetry.windows,
            attached: self.telemetry.attached,
            clients: self.client_count,
            hint_badge: (self.theme.preset != ThemePreset::Classic).then_some(badge.as_str()),
        };
        let emblem = (
            self.config.emblem_mode,
            if self.config.emblem_mode.should_animate() {
                self.emblem_step
            } else {
                0
            },
        );
        let preview = self.build_pane_preview(false);
        let menubar = self.build_menubar_state();
        let mut picker_selected_and_len = None;
        let sub_layout = match self.views.0.last() {
            Some(View::Picker(picker)) => {
                let rows = picker_rows.as_deref().unwrap_or(&[]);
                picker_selected_and_len = Some((picker.selected, rows.len()));
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Picker {
                        title: picker.purpose.title_for(self.theme.language),
                        query: &picker.query,
                        rows,
                        selected: picker.selected,
                        empty: self
                            .theme
                            .tr("No matching panes. Esc back", "没有匹配的窗格 · Esc 返回"),
                        preview,
                    },
                )
            }
            Some(View::TargetPick(state)) => {
                let rows = picker_rows.as_deref().unwrap_or(&[]);
                picker_selected_and_len = Some((state.selected, rows.len()));
                let banner = state.purpose.action_banner(self.theme.language);
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::TargetPicker {
                        title: state.purpose.title(self.theme.language),
                        action_banner: &banner,
                        query: &state.query,
                        rows,
                        selected: state.selected,
                        empty: self
                            .theme
                            .tr("No matching targets. Esc back", "没有匹配的目标 · Esc 返回"),
                        preview,
                    },
                )
            }
            Some(View::Actions(resource, selected)) => {
                let actions = resource.actions_for(&self.piping_panes, self.theme.language);
                let action_labels: Vec<&str> =
                    actions.iter().map(|(label, _)| label.as_str()).collect();
                let title = match resource {
                    Resource::Session(_) => self.theme.tr("Session actions", "会话操作"),
                    Resource::Window(_) => self.theme.tr("Window actions", "窗口操作"),
                    Resource::Pane(_) => self.theme.tr("Pane actions", "窗格操作"),
                };
                let target_label = resource.label();
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Actions {
                        title,
                        target_label: &target_label,
                        actions: &action_labels,
                        selected: *selected,
                    },
                )
            }
            Some(View::Form(form)) => {
                let dynamic_label;
                let (title, label) = match &form.action {
                    FormAction::NewSessionName => (
                        self.theme.tr("New session", "新建会话"),
                        self.theme.tr("Session name", "会话名称"),
                    ),
                    FormAction::NewSessionDirectory { .. } => (
                        self.theme.tr("New session", "新建会话"),
                        self.theme.tr(
                            "Working directory (empty = current directory)",
                            "工作目录（留空则使用当前目录）",
                        ),
                    ),
                    FormAction::Rename(resource) => {
                        dynamic_label = if self.theme.language.is_zh() {
                            format!("将 {} 重命名为", resource.label())
                        } else {
                            format!("New name for {}", resource.label())
                        };
                        (self.theme.tr("Rename", "重命名"), dynamic_label.as_str())
                    }
                    FormAction::RenamePaneTitle(pane) => {
                        dynamic_label = if self.theme.language.is_zh() {
                            format!("窗格 {} 的新标题", pane.id)
                        } else {
                            format!("New title for pane {}", pane.id)
                        };
                        (
                            self.theme.tr("Rename pane title", "重命名窗格标题"),
                            dynamic_label.as_str(),
                        )
                    }
                    FormAction::SendCommandToPane(pane) => {
                        dynamic_label = if self.theme.language.is_zh() {
                            format!("向窗格 {} 发送命令并回车执行", pane.id)
                        } else {
                            format!("Command to run in pane {} (Enter to send)", pane.id)
                        };
                        (
                            self.theme.tr("Send command", "发送命令"),
                            dynamic_label.as_str(),
                        )
                    }
                    FormAction::NewBufferContent => (
                        self.theme.tr("New paste buffer", "新建粘贴缓冲区"),
                        self.theme
                            .tr("Text to store in tmux buffer", "要存入 tmux 缓冲区的文本"),
                    ),
                    FormAction::SaveSnapshotName { session_filter } => {
                        let scope = if session_filter.is_some() {
                            self.theme.tr("Session snapshot name", "会话快照名称")
                        } else {
                            self.theme.tr("Workspace snapshot name", "工作区快照名称")
                        };
                        (self.theme.tr("Save snapshot", "保存工作区快照"), scope)
                    }
                };
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Form {
                        title,
                        label,
                        input: &form.input,
                    },
                )
            }
            Some(View::DualForm(form)) => {
                let (title, first_label, second_label) = match &form.action {
                    controller::DualFormAction::NewWindow(_) => (
                        self.theme.tr("New window", "新建窗口"),
                        self.theme.tr("Window name (optional)", "窗口名称（可选）"),
                        self.theme.tr(
                            "Working directory (optional, Tab to edit)",
                            "工作目录（可选，按 Tab 切换）",
                        ),
                    ),
                };
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::DualForm {
                        title,
                        first_label,
                        first_input: &form.first_input,
                        second_label,
                        second_input: &form.second_input,
                        active_field: form.active_field,
                    },
                )
            }
            Some(View::Confirm(confirm)) => {
                let summary = confirm.target.summary(self.theme.language);
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Confirm {
                        title: self
                            .theme
                            .tr("Confirm irreversible action", "确认不可逆操作"),
                        target_summary: &summary,
                        message: &confirm.message,
                        input: &confirm.input,
                        scroll: confirm.scroll,
                    },
                )
            }
            Some(View::Created(resource)) => {
                let target_label = resource.label();
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Created {
                        target_label: &target_label,
                    },
                )
            }
            Some(View::Split(studio)) => {
                let root = self.build_split_canvas_node(studio, &studio.root);
                let mut window_rows: Vec<SplitWindowRowView> = studio
                    .window_options
                    .iter()
                    .map(|opt| {
                        let detail = if opt.window.panes > 1 {
                            let active_note = self.theme.tr("active pane", "仅迁活跃窗格");
                            if opt.command.is_empty() {
                                format!("{}p · {active_note}", opt.window.panes)
                            } else {
                                format!("{} · {}p · {active_note}", opt.command, opt.window.panes)
                            }
                        } else {
                            opt.command.clone()
                        };
                        SplitWindowRowView {
                            primary: format!(
                                "{}:{} {}",
                                opt.window.session_name, opt.window.index, opt.window.name
                            ),
                            detail,
                            active_in_session: opt.window.active,
                            assigned_slot: studio
                                .root
                                .find_slot_with_window(&opt.window.id)
                                .and_then(|sid| studio.slot_ordinal(sid)),
                            is_new_shell: false,
                        }
                    })
                    .collect();
                window_rows.push(SplitWindowRowView {
                    primary: self.theme.tr("+ New shell", "+ 新建终端").to_owned(),
                    detail: self
                        .theme
                        .tr("Open fresh shell in this slot", "在此窗格新建空白终端")
                        .to_owned(),
                    active_in_session: false,
                    assigned_slot: None,
                    is_new_shell: true,
                });
                let view = SplitStudioView {
                    root,
                    selected_slot: studio.selected_slot,
                    selected_ordinal: studio.slot_ordinal(studio.selected_slot).unwrap_or(1),
                    slot_count: studio.root.leaf_count(),
                    can_split_h: studio
                        .root
                        .can_split(studio.selected_slot, SplitDirection::SideBySide),
                    can_split_v: studio
                        .root
                        .can_split(studio.selected_slot, SplitDirection::Stacked),
                    can_flip: studio.root.can_flip(),
                    can_remove: studio.root.leaf_count() > 1,
                    active_ratio: studio.active_ratio(),
                    can_adjust_ratio: studio.can_adjust_ratio(),
                    sync_panes: studio.sync_panes,
                    can_break_out: studio.selected_existing_pane().is_some()
                        && studio.root.leaf_count() > 1,
                    editing_existing: studio.is_editing_existing(),
                    window_rows: &window_rows,
                    window_cursor: studio.window_cursor,
                    hovered_sash: self.desktop.hovered_sash_id(),
                    dragging_sash: self.desktop.dragging_sash_id(),
                    hovered_dock: self.desktop.drag.as_ref().and_then(|d| d.split_dock_slot),
                };
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Split(view),
                )
            }
            Some(View::LayoutStudio(state)) => draw_subview(
                frame,
                subview_area,
                details,
                telemetry,
                emblem,
                menubar,
                self.theme,
                SubviewContent::LayoutStudio(LayoutStudioView {
                    target_label: &state.target_label,
                    selected_preset: state.selected_preset,
                    pane_size: state.pane_size,
                    pane_count: state.pane_count,
                    resize_step: state.resize_step,
                    zoomed: state.zoomed,
                    synchronized: state.synchronized,
                    status_note: state.status_note.as_deref(),
                    hovered_sash: self.desktop.hovered_sash_id(),
                    dragging_sash: self.desktop.dragging_sash_id(),
                }),
            ),
            Some(View::Expose(state)) => {
                let tiles = self.build_expose_tile_views(state);
                let expose_view = self.expose_view_summary(state, &tiles);
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Expose(expose_view),
                )
            }
            Some(View::Clients(state)) => {
                let current_client = match &self.context {
                    Context::Inside(inside) => inside.client_name.as_deref(),
                    _ => None,
                };
                let rows: Vec<ClientRowView> = state
                    .clients
                    .iter()
                    .map(|c| ClientRowView {
                        name: c.name.clone(),
                        session_name: c.session_name.clone(),
                        width: c.width,
                        height: c.height,
                        tty: c.tty.clone(),
                        readonly: c.readonly,
                        is_current: current_client == Some(c.name.as_str()),
                    })
                    .collect();
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Clients(ClientsView {
                        clients: &rows,
                        selected: state.selected,
                        size_mismatch_warning: state.has_size_mismatch(),
                        status_note: state.status_note.as_deref(),
                    }),
                )
            }
            Some(View::Buffers(state)) => {
                let rows: Vec<BufferRowView> = state
                    .buffers
                    .iter()
                    .map(|b| BufferRowView {
                        name: b.name.clone(),
                        size: b.size,
                        sample: b.sample.clone(),
                    })
                    .collect();
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Buffers(BuffersView {
                        buffers: &rows,
                        selected: state.selected,
                        preview_lines: &state.preview_lines,
                        preview_scroll: state.preview_scroll,
                        status_note: state.status_note.as_deref(),
                    }),
                )
            }
            Some(View::Snapshots(state)) => {
                let rows: Vec<SnapshotRowView> = state
                    .snapshots
                    .iter()
                    .map(|s| SnapshotRowView {
                        name: s.name.clone(),
                        created_label: WorkspaceSnapshot::format_epoch_utc(s.created_epoch),
                        session_count: s.session_count,
                        window_count: s.window_count,
                        pane_count: s.pane_count,
                    })
                    .collect();
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Snapshots(SnapshotsView {
                        snapshots: &rows,
                        selected: state.selected,
                        tree_preview: &state.tree_preview,
                        preview_scroll: state.preview_scroll,
                        status_note: state.status_note.as_deref(),
                    }),
                )
            }
            Some(View::ThemePicker { initial, selected }) => draw_subview(
                frame,
                subview_area,
                details,
                telemetry,
                emblem,
                menubar,
                self.theme,
                SubviewContent::Theme {
                    presets: ThemePreset::ALL,
                    selected: *selected,
                    active: *initial,
                },
            ),
            Some(View::Config { selected }) => {
                let items = self.config_modal_items();
                let path_label = self
                    .config_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| config::resolve_config_path(None).map(|p| p.display().to_string()))
                    .unwrap_or_else(|| format!("~/{}", config::DEFAULT_CONFIG_REL_PATH));
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Config {
                        path_label: &path_label,
                        items: &items,
                        selected: *selected,
                        status_note: self.config_note.as_deref(),
                    },
                )
            }
            Some(View::Tribute { id, step }) => {
                let card = tributes::card(*id, *step);
                draw_subview(
                    frame,
                    subview_area,
                    details,
                    telemetry,
                    emblem,
                    menubar,
                    self.theme,
                    SubviewContent::Tribute(ui::TributeModal {
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
                    }),
                )
            }
            Some(View::Help) => draw_subview(
                frame,
                subview_area,
                details,
                telemetry,
                emblem,
                menubar,
                self.theme,
                SubviewContent::Help,
            ),
            None => ui::WorkspaceSubviewLayout::default(),
        };
        self.hydra_visible = sub_layout.hydra_rendered;
        self.preview_rect = sub_layout.preview_area;
        self.subview_list_rect = sub_layout.list_rect;
        self.preview_action_hits = sub_layout.preview_action_hits;
        self.controller_hits = sub_layout.controller_hits;
        self.desktop.menubar_hits = sub_layout.menubar_hits;
        self.desktop.sash_hits = sub_layout.sash_hits;
        self.desktop.expose_hits = sub_layout.expose_tile_hits;
        self.desktop.preview_layout = sub_layout.preview_layout;
        if let Some((selected, rows_len)) = picker_selected_and_len
            && rows_len > 0
            && let Some(list_area) = sub_layout.list_rect
            && list_area.height > 0
        {
            let visible_rows = usize::from(list_area.height);
            let clamped = selected.min(rows_len - 1);
            self.picker_offset = (clamped + 1).saturating_sub(visible_rows);
            self.picker_list_rect = Some(list_area);
        }
        if let Some(View::Confirm(confirm)) = self.views.0.last_mut() {
            confirm.scroll = sub_layout.confirm_scroll;
        }
        if let Some(View::Split(studio)) = self.views.0.last_mut() {
            studio.slot_hits = sub_layout.split_slot_hits;
            studio.toolbar_hits = sub_layout
                .split_toolbar_hits
                .into_iter()
                .map(|(rect, btn)| (rect, Self::ui_toolbar_to_action(btn)))
                .collect();
            studio.window_list_rect = sub_layout.list_rect;
        }
        if let Some(chunks) = compact_chunks {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(format!("{} ", self.theme.marker())),
                    Span::styled(self.composer.value(), self.theme.style(self.theme.muted)),
                ])),
                chunks[1],
            );
            let status_style = ui::status_style(&self.status, self.theme);
            frame.render_widget(
                Paragraph::new(self.theme.adapt(&self.status)).style(status_style),
                chunks[2],
            );
        }
        self.render_desktop_overlays_for_frame(frame, full_area);
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
            purpose: PickerPurpose::Panes,
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
            purpose: PickerPurpose::Panes,
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
            target: ConfirmTarget::Resource(Box::new(Resource::Session(session("$1", "orion")))),
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
            target: ConfirmTarget::Resource(Box::new(Resource::Session(session("$1", "orion")))),
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

    #[test]
    fn kill_server_requires_exact_yes_and_private_socket_controller_flows_work() {
        use ratatui::{Terminal, backend::TestBackend};

        let socket = format!(
            "jm-app-ctrl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmux = crate::tmux::TmuxClient::new(crate::tmux::SocketSpec::Name(socket.clone()))
            .with_config_file("/dev/null");
        // Ensure clean server teardown even on assertion failure.
        struct Cleanup(crate::tmux::TmuxClient);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.kill_server();
            }
        }
        let _cleanup = Cleanup(tmux.clone());

        let cwd = std::env::temp_dir();
        let sess = tmux.new_session("ctrl-demo", &cwd).expect("new session");
        let wins = tmux.list_windows(Some(&sess.id)).expect("list windows");
        let panes = tmux.list_panes(Some(&wins[0].id)).expect("list panes");
        let _second_pane = tmux
            .split_pane(&panes[0].id, SplitDirection::SideBySide)
            .expect("split pane");

        let conf_dir = tempfile::tempdir().expect("tempdir");
        let conf_path = conf_dir.path().join(".tmux.conf");

        let mut app =
            App::new(&tmux, AppOptions::default()).with_tmux_conf(Some(conf_path.clone()), None);
        let send_key = |app: &mut App<'_>, code: KeyCode| {
            app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        };
        let send_str = |app: &mut App<'_>, s: &str| {
            for c in s.chars() {
                send_key(app, KeyCode::Char(c));
            }
        };
        let mut term = Terminal::new(TestBackend::new(110, 32)).expect("test backend");

        // 1. Multi-pane Home preview cycling (`[` / `]`) & preview search (`f` .. `Esc`).
        assert_eq!(app.home_preview_pane_total, 2);
        let first_idx = app.home_preview_pane_idx;
        send_key(&mut app, KeyCode::Char(']'));
        assert_ne!(app.home_preview_pane_idx, first_idx);
        send_key(&mut app, KeyCode::Char('['));
        assert_eq!(app.home_preview_pane_idx, first_idx);

        send_key(&mut app, KeyCode::Char('f'));
        assert!(app.preview_search.active);
        send_str(&mut app, "nonexistent");
        term.draw(|f| app.render(f)).expect("draw preview search");
        send_key(&mut app, KeyCode::Esc);
        assert!(!app.preview_search.active);
        assert!(app.exit().is_none());

        // 2. Natural phrases: zoom, sync panes, clients, buffers, layout.
        send_str(&mut app, "zoom");
        send_key(&mut app, KeyCode::Enter);
        assert!(
            tmux.list_windows(Some(&sess.id))
                .expect("windows after zoom")[0]
                .zoomed
        );
        send_str(&mut app, "unzoom");
        send_key(&mut app, KeyCode::Enter);
        assert!(
            !tmux
                .list_windows(Some(&sess.id))
                .expect("windows after unzoom")[0]
                .zoomed
        );

        send_str(&mut app, "sync panes");
        send_key(&mut app, KeyCode::Enter);
        assert!(
            tmux.list_windows(Some(&sess.id))
                .expect("windows after sync")[0]
                .synchronized
        );
        send_str(&mut app, "sync panes");
        send_key(&mut app, KeyCode::Enter);

        // 3. /layout, /clients, /buffers, /snapshot subviews open & render cleanly.
        send_str(&mut app, "/layout");
        send_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.views.top(), Some(View::LayoutStudio(_))));
        term.draw(|f| app.render(f)).expect("draw layout studio");
        send_key(&mut app, KeyCode::Char('1'));
        send_key(&mut app, KeyCode::Esc);

        send_str(&mut app, "/clients");
        send_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.views.top(), Some(View::Clients(_))));
        term.draw(|f| app.render(f)).expect("draw clients");
        send_key(&mut app, KeyCode::Esc);

        tmux.set_buffer("hello-justmux-buffer").expect("set buffer");
        send_str(&mut app, "/buffers");
        send_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.views.top(), Some(View::Buffers(_))));
        term.draw(|f| app.render(f)).expect("draw buffers");
        send_key(&mut app, KeyCode::Esc);

        send_str(&mut app, "/snapshot");
        send_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.views.top(), Some(View::Snapshots(_))));
        term.draw(|f| app.render(f)).expect("draw snapshots");
        send_key(&mut app, KeyCode::Esc);

        // 4. Split Studio loads existing 2-pane window with real panes and renders cleanly.
        send_str(&mut app, "/split");
        send_key(&mut app, KeyCode::Enter);
        send_key(&mut app, KeyCode::Enter); // activate selected window in Split mode
        assert!(matches!(
            app.views.top(),
            Some(View::Split(s)) if s.is_editing_existing() && s.root.leaf_count() == 2
        ));
        send_key(&mut app, KeyCode::Char('p')); // cycle ratio preset
        send_key(&mut app, KeyCode::Char('y')); // toggle sync_panes
        term.draw(|f| app.render(f)).expect("draw split studio");
        send_key(&mut app, KeyCode::Esc);

        // 5. DualForm New Window (Tab between Name & Directory, 1-step Enter works).
        app.views
            .push(View::DualForm(DualForm::new_window(sess.id.clone())));
        send_str(&mut app, "dual-win");
        send_key(&mut app, KeyCode::Tab);
        send_str(&mut app, &cwd.display().to_string());
        term.draw(|f| app.render(f)).expect("draw dual form");
        send_key(&mut app, KeyCode::Enter);
        assert!(
            tmux.list_windows(Some(&sess.id))
                .expect("windows after dual form")
                .iter()
                .any(|w| w.name == "dual-win")
        );
        send_key(&mut app, KeyCode::Esc);

        // 6. /config comfort toggles sync isolated tempdir tmux.conf & live tmux.
        send_str(&mut app, "/config");
        send_key(&mut app, KeyCode::Enter);
        for _ in 0..11 {
            send_key(&mut app, KeyCode::Down);
        }
        send_key(&mut app, KeyCode::Enter); // toggle tmux_mouse ON (row 11)
        assert!(app.config().tmux_mouse);
        let conf_text = std::fs::read_to_string(&conf_path).expect("read temp tmux.conf");
        assert!(conf_text.contains("set -g mouse on"), "{conf_text}");
        term.draw(|f| app.render(f)).expect("draw config comfort");
        send_key(&mut app, KeyCode::Enter); // toggle tmux_mouse OFF
        assert!(!app.config().tmux_mouse);
        send_key(&mut app, KeyCode::Esc);

        // 7. /kill-server cancels on empty Enter and kills private server on exact "yes".
        send_str(&mut app, "/kill-server");
        send_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.views.top(), Some(View::Confirm(_))));
        send_key(&mut app, KeyCode::Enter); // empty cancels
        assert!(tmux.server_running().expect("server still running"));

        send_str(&mut app, "/kill-server");
        send_key(&mut app, KeyCode::Enter);
        send_str(&mut app, "yes");
        send_key(&mut app, KeyCode::Enter);
        assert!(!tmux.server_running().expect("server stopped"));
    }

    #[test]
    fn with_tmux_conf_syncs_removed_comfort_block_and_clients_keys_match_ui_pills() {
        use std::cell::RefCell;

        struct ClientsStubTmux {
            detached: RefCell<Vec<String>>,
            detached_others: RefCell<Vec<Option<String>>>,
        }

        impl Tmux for ClientsStubTmux {
            fn version(&self) -> TmuxResult<crate::tmux::TmuxVersion> {
                Ok(crate::tmux::TmuxVersion {
                    raw: "3.4".into(),
                    number: Some((3, 4)),
                })
            }
            fn server_running(&self) -> TmuxResult<bool> {
                Ok(true)
            }
            fn context(&self) -> Context {
                Context::Inside(InsideContext {
                    client_name: Some("/dev/ttys001".into()),
                    ..InsideContext::default()
                })
            }
            fn list_sessions(&self) -> TmuxResult<Vec<Session>> {
                Ok(Vec::new())
            }
            fn has_session(&self, _: &SessionId) -> TmuxResult<bool> {
                Ok(false)
            }
            fn new_session(&self, _: &str, _: &std::path::Path) -> TmuxResult<Session> {
                Err(crate::tmux::TmuxError::NoServer)
            }
            fn rename_session(&self, _: &SessionId, _: &str) -> TmuxResult<()> {
                Ok(())
            }
            fn kill_session(&self, _: &SessionId) -> TmuxResult<()> {
                Ok(())
            }
            fn list_windows(&self, _: Option<&SessionId>) -> TmuxResult<Vec<Window>> {
                Ok(Vec::new())
            }
            fn new_window(
                &self,
                _: &SessionId,
                _: Option<&str>,
                _: Option<&std::path::Path>,
            ) -> TmuxResult<Window> {
                Err(crate::tmux::TmuxError::NoServer)
            }
            fn rename_window(&self, _: &WindowId, _: &str) -> TmuxResult<()> {
                Ok(())
            }
            fn kill_window(&self, _: &WindowId) -> TmuxResult<()> {
                Ok(())
            }
            fn list_panes(&self, _: Option<&WindowId>) -> TmuxResult<Vec<Pane>> {
                Ok(Vec::new())
            }
            fn split_pane(&self, _: &PaneId, _: SplitDirection) -> TmuxResult<Pane> {
                Err(crate::tmux::TmuxError::NoServer)
            }
            fn join_pane(&self, _: &PaneId, _: &PaneId, _: SplitDirection) -> TmuxResult<()> {
                Ok(())
            }
            fn kill_pane(&self, _: &PaneId) -> TmuxResult<()> {
                Ok(())
            }
            fn select(&self, _: &Target) -> TmuxResult<()> {
                Ok(())
            }
            fn switch_client(&self, _: &Target) -> TmuxResult<()> {
                Ok(())
            }
            fn attach_command(&self, _: &Target) -> std::process::Command {
                std::process::Command::new("tmux")
            }
            fn detach_client(&self, client: &str) -> TmuxResult<()> {
                self.detached.borrow_mut().push(client.to_owned());
                Ok(())
            }
            fn list_clients(&self) -> TmuxResult<Vec<crate::tmux::ClientInfo>> {
                Ok(vec![
                    crate::tmux::ClientInfo {
                        name: "/dev/ttys001".into(),
                        session_name: "dev".into(),
                        session_id: SessionId::new("$1"),
                        width: 120,
                        height: 40,
                        tty: "/dev/ttys001".into(),
                        readonly: false,
                        control_mode: false,
                        activity: 0,
                    },
                    crate::tmux::ClientInfo {
                        name: "/dev/ttys002".into(),
                        session_name: "dev".into(),
                        session_id: SessionId::new("$1"),
                        width: 80,
                        height: 24,
                        tty: "/dev/ttys002".into(),
                        readonly: false,
                        control_mode: false,
                        activity: 0,
                    },
                ])
            }
            fn detach_other_clients(
                &self,
                _: Option<&SessionId>,
                keep_client: Option<&str>,
            ) -> TmuxResult<usize> {
                self.detached_others
                    .borrow_mut()
                    .push(keep_client.map(ToOwned::to_owned));
                Ok(1)
            }
        }

        let stub = ClientsStubTmux {
            detached: RefCell::new(Vec::new()),
            detached_others: RefCell::new(Vec::new()),
        };

        // Removing comfort block from tmux.conf must clear stale config.toml comfort flags.
        let dir = tempfile::tempdir().expect("tempdir");
        let empty_conf = dir.path().join(".tmux.conf");
        std::fs::write(&empty_conf, "# user config without comfort block\n").expect("write conf");
        let stale_cfg = Config {
            tmux_mouse: true,
            tmux_history_limit: 50_000,
            tmux_base_index_one: true,
            tmux_renumber_windows: true,
            tmux_zero_escape_time: true,
            ..Config::default()
        };
        let mut app = App::with_config(&stub, AppOptions::default(), stale_cfg, None)
            .with_tmux_conf(Some(empty_conf), None);
        assert!(!app.config().tmux_mouse);
        assert_eq!(app.config().tmux_history_limit, 0);
        assert!(!app.config().tmux_base_index_one);
        assert!(!app.config().tmux_renumber_windows);
        assert!(!app.config().tmux_zero_escape_time);

        // /clients: `d` / `o` / DetachOtherClients detach others; `x` / Enter / DetachSelectedClient detach selected.
        app.open_clients_view();
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert_eq!(
            stub.detached_others.borrow().as_slice(),
            &[Some("/dev/ttys001".to_owned())]
        );
        assert!(stub.detached.borrow().is_empty());

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));
        assert_eq!(
            stub.detached.borrow().as_slice(),
            &["/dev/ttys002".to_owned()]
        );

        app.handle_controller_action(ControllerAction::DetachOtherClients);
        assert_eq!(stub.detached_others.borrow().len(), 2);
        app.handle_controller_action(ControllerAction::DetachSelectedClient);
        assert_eq!(stub.detached.borrow().len(), 2);
    }

    #[test]
    fn circular_selection_wraps_across_lists_config_and_layout_wheel() {
        // 1. wrap_step edge cases
        assert_eq!(wrap_step(0, 0, 1), 0);
        assert_eq!(wrap_step(5, 0, -1), 0);
        assert_eq!(wrap_step(0, 1, 1), 0);
        assert_eq!(wrap_step(0, 1, -1), 0);
        assert_eq!(wrap_step(0, 5, -1), 4);
        assert_eq!(wrap_step(4, 5, 1), 0);
        assert_eq!(wrap_step(99, 5, 1), 0); // stale index clamps to 4, then +1 wraps to 0
        assert_eq!(wrap_step(99, 5, -1), 3); // stale index clamps to 4, then -1 gives 3

        let tmux = crate::tmux::TmuxClient::new(crate::tmux::SocketSpec::Name(format!(
            "jm-wrap-test-{}",
            std::process::id()
        )))
        .with_program("/nonexistent-justmux-test-tmux");
        let mut app = App::new(&tmux, AppOptions::default());

        // 2. `/` command palette wraps keyboard Up/Down and mouse wheel.
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        let total_cmds = app.palette().len();
        assert_eq!(total_cmds, 19);
        assert_eq!(app.palette_selected, 0);
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.palette_selected, total_cmds - 1);
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(app.palette_selected, 0);
        app.mouse_scroll(0, 0, true);
        assert_eq!(app.palette_selected, total_cmds - 1);
        app.mouse_scroll(0, 0, false);
        assert_eq!(app.palette_selected, 0);

        // 3. Config wraps selection (Up/Down + wheel) and inline_height wraps 8 <-> 40.
        app.config.inline_height = 8;
        app.views.push(View::Config { selected: 0 });
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert!(matches!(
            app.views.top(),
            Some(View::Config { selected }) if *selected == CONFIG_ITEM_COUNT - 1
        ));
        app.mouse_scroll(0, 0, false);
        assert!(matches!(
            app.views.top(),
            Some(View::Config { selected: 0 })
        ));
        app.views.pop();
        app.views.push(View::Config { selected: 6 });
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        assert_eq!(app.config.inline_height, 40);
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.config.inline_height, 8);
        app.views.pop();

        // 4. LayoutStudio mouse wheel wraps preset selection circularly.
        app.views.push(View::LayoutStudio(LayoutStudioState {
            window_id: WindowId::new("@1").expect("window id"),
            pane_id: PaneId::new("%1").expect("pane id"),
            target_label: "dev:0 main".into(),
            selected_preset: 0,
            pane_size: (80, 24),
            pane_count: 2,
            resize_step: 5,
            zoomed: false,
            synchronized: false,
            status_note: None,
        }));
        app.mouse_scroll(0, 0, true);
        assert!(matches!(
            app.views.top(),
            Some(View::LayoutStudio(s)) if s.selected_preset == LayoutPreset::ALL.len() - 1
        ));
        app.mouse_scroll(0, 0, false);
        assert!(matches!(
            app.views.top(),
            Some(View::LayoutStudio(s)) if s.selected_preset == 0
        ));
        app.views.pop();

        // 5. Confirm text scroll stays clamped (does not wrap).
        app.views.push(View::Confirm(Confirm {
            target: ConfirmTarget::Resource(Box::new(Resource::Session(session("$1", "orion")))),
            input: TextInput::default(),
            message: "msg".into(),
            scroll: 0,
        }));
        app.mouse_scroll(0, 0, true);
        assert!(matches!(
            app.views.top(),
            Some(View::Confirm(c)) if c.scroll == 0
        ));
    }
}
