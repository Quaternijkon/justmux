//! Application state and keyboard focus. Views never handle keys below the top
//! of the stack; all tmux mutations happen here, outside the rendering layer.
use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    commands::{self, CommandKind, ParsedCommand, SlashCommand},
    fuzzy,
    tmux::{Context, Pane, Session, SessionId, SplitDirection, Target, Tmux, TmuxResult, Window},
    ui::{self, CommandItem, TextInput, Theme},
};

#[derive(Debug, Clone, Default)]
pub struct AppOptions {
    pub initial_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Quit,
    Attach(Target),
    Switched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerPurpose {
    Resume,
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
            Self::Resume => "Resume session",
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
            Self::Windows => {
                "↑↓ Navigate · Enter Select · Ctrl+N New · Ctrl+E Rename · Ctrl+K Kill · Ctrl+R Refresh · Esc Back"
            }
            Self::Panes => "↑↓ Navigate · Enter Select · Ctrl+K Kill · Ctrl+R Refresh · Esc Back",
            _ => {
                "↑↓ Navigate · Enter Select · Ctrl+E Rename · Ctrl+K Kill · Ctrl+R Refresh · Esc Back"
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
            Self::Window(w) => Target::Window(w.id.clone()),
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
    fn description(&self, context: &Context) -> String {
        match self {
            Self::Session(s) => format!(
                "{}  {} window{} · {}{}",
                s.name,
                s.windows,
                if s.windows == 1 { "" } else { "s" },
                if s.attached > 0 {
                    "attached"
                } else {
                    "detached"
                },
                if context.current_session() == Some(&s.id) {
                    " · current"
                } else {
                    ""
                }
            ),
            Self::Window(w) => format!(
                "{}  {} pane{}{}  {}",
                self.label(),
                w.panes,
                if w.panes == 1 { "" } else { "s" },
                if w.active { " · active" } else { "" },
                w.id
            ),
            Self::Pane(p) => format!(
                "{}  {}{}  {}",
                self.label(),
                p.id,
                if p.busy { " · running task" } else { "" },
                p.current_path
            ),
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
impl Picker {
    fn indices(&self) -> Vec<usize> {
        let mut rows: Vec<_> = self
            .resources
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                fuzzy::score(self.query.value(), &r.label()).map(|score| (score, i))
            })
            .collect();
        rows.sort_by_key(|&(score, i)| (score, i));
        rows.into_iter().map(|(_, i)| i).collect()
    }
    fn selected_resource(&self) -> Option<Resource> {
        self.indices()
            .get(self.selected)
            .and_then(|&i| self.resources.get(i))
            .cloned()
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
#[derive(Debug, Clone)]
enum View {
    Picker(Picker),
    Form(Form),
    Confirm(Confirm),
    Created(Resource),
    Split(Resource, usize),
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

pub struct App<'a> {
    tmux: &'a dyn Tmux,
    composer: TextInput,
    palette_selected: usize,
    views: ViewStack,
    status: String,
    theme: Theme,
    context: Context,
    exit: Option<Exit>,
}

impl<'a> App<'a> {
    pub fn new(tmux: &'a dyn Tmux, options: AppOptions) -> Self {
        let mut app = Self {
            tmux,
            composer: TextInput::default(),
            palette_selected: 0,
            views: ViewStack::default(),
            status: String::new(),
            theme: Theme::from_env(),
            context: tmux.context(),
            exit: None,
        };
        if let Some(command) = options.initial_command {
            app.execute_text(&format!("/{}", command.trim_start_matches('/')));
        }
        app
    }

    pub fn handle_event(&mut self, event: Event) {
        if let Event::Paste(text) = event {
            // Pasting text is editing, never command execution.
            for c in text.chars().filter(|c| !c.is_control()) {
                self.handle_event(Event::Key(KeyEvent::new(
                    KeyCode::Char(c),
                    KeyModifiers::NONE,
                )));
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
        if key.code == KeyCode::Esc {
            if self.views.pop().is_none() {
                if self.composer.value().is_empty() {
                    self.exit = Some(Exit::Quit);
                } else {
                    self.composer.set_value("");
                    self.palette_selected = 0;
                }
            }
            self.status.clear();
            return;
        }
        // Take the top view out while handling it to make nested pushes explicit.
        match self.views.pop() {
            None => self.composer_key(key),
            Some(View::Picker(picker)) => self.picker_key(picker, key),
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
            Some(View::Help) => self.views.push(View::Help),
        }
    }

    fn palette(&self) -> Vec<&'static SlashCommand> {
        if !self.composer.value().starts_with('/') {
            return Vec::new();
        }
        let query = self.composer.value().trim_start_matches('/');
        if query.contains(char::is_whitespace) {
            return Vec::new();
        }
        let mut matches = fuzzy::rank_by(query, commands::registry(), |command| command.name);
        // An exact alias remains discoverable under its canonical name.
        for command in commands::registry() {
            if command.aliases.contains(&query) && !matches.contains(&command) {
                matches.insert(0, command);
            }
        }
        matches
    }

    fn composer_key(&mut self, key: KeyEvent) {
        let candidates = self.palette();
        match key.code {
            KeyCode::Up => self.palette_selected = self.palette_selected.saturating_sub(1),
            KeyCode::Down if !candidates.is_empty() => {
                self.palette_selected = (self.palette_selected + 1).min(candidates.len() - 1)
            }
            KeyCode::Tab if !candidates.is_empty() => {
                if let Some(command) = candidates.get(self.palette_selected) {
                    self.composer.set_value(format!("/{}", command.name));
                    self.palette_selected = 0;
                }
            }
            KeyCode::Enter => {
                let mut text = self.composer.value().to_string();
                if !text.contains(char::is_whitespace)
                    && let Some(command) = candidates.get(self.palette_selected)
                {
                    text = format!("/{}", command.name);
                }
                self.execute_text(&text);
            }
            _ => {
                if self.composer.handle_key(key) {
                    self.palette_selected = 0;
                    self.status.clear();
                }
            }
        }
    }

    fn execute_text(&mut self, text: &str) {
        match commands::parse(text) {
            Ok(command) => {
                self.status.clear();
                // A submitted slash command becomes a view action. Start the
                // next command with a fresh composer when the user returns.
                self.composer.set_value("");
                self.palette_selected = 0;
                self.execute(command);
            }
            Err(error) => self.status = format!("{error}. Type / to choose a command."),
        }
    }

    fn execute(&mut self, parsed: ParsedCommand) {
        use CommandKind as C;
        match parsed.command {
            C::Resume => self.open_picker(PickerPurpose::Resume),
            C::List => self.open_picker(PickerPurpose::List),
            C::Rename => self.open_picker(PickerPurpose::Rename),
            C::Kill => self.open_picker(PickerPurpose::Kill),
            C::Window => self.open_picker(PickerPurpose::Windows),
            C::Pane => self.open_picker(PickerPurpose::Panes),
            C::Split => self.open_picker(PickerPurpose::Split),
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
            C::Help => self.views.push(View::Help),
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
            _ => self.tmux.list_sessions().map(|mut rows| {
                rows.sort_by(|a, b| {
                    b.recency()
                        .cmp(&a.recency())
                        .then_with(|| a.name.cmp(&b.name))
                });
                rows.into_iter().map(Resource::Session).collect()
            }),
        }
    }

    fn open_picker(&mut self, purpose: PickerPurpose) {
        let resources = match self.read_resources(purpose) {
            Ok(resources) => resources,
            Err(error) => {
                self.status = error.to_string();
                Vec::new()
            }
        };
        self.views.push(View::Picker(Picker {
            purpose,
            query: TextInput::default(),
            resources,
            selected: 0,
        }));
    }

    fn picker_key(&mut self, mut picker: Picker, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('r') {
            match self.read_resources(picker.purpose) {
                Ok(rows) => {
                    picker.resources = rows;
                    picker.selected = 0;
                    self.status = "Refreshed".into();
                }
                Err(error) => self.status = error.to_string(),
            }
            self.views.push(View::Picker(picker));
            return;
        }
        let resource = picker.selected_resource();
        if ctrl && key.code == KeyCode::Char('n') && picker.purpose == PickerPurpose::Windows {
            self.views.push(View::Picker(picker));
            self.open_picker(PickerPurpose::NewWindow);
            return;
        }
        if ctrl && key.code == KeyCode::Char('e') {
            self.views.push(View::Picker(picker));
            if let Some(resource) = resource {
                self.rename_form(resource);
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('k') {
            self.views.push(View::Picker(picker));
            if let Some(resource) = resource {
                self.kill_confirm(resource);
            }
            return;
        }
        match key.code {
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => {
                picker.selected =
                    (picker.selected + 1).min(picker.indices().len().saturating_sub(1))
            }
            KeyCode::Enter => {
                let purpose = picker.purpose;
                self.views.push(View::Picker(picker));
                if let Some(resource) = resource {
                    match purpose {
                        PickerPurpose::Rename => self.rename_form(resource),
                        PickerPurpose::Kill => self.kill_confirm(resource),
                        PickerPurpose::Split => self.views.push(View::Split(resource, 0)),
                        PickerPurpose::NewWindow => self.views.push(View::Form(Form {
                            action: FormAction::NewWindow(resource.session_id().clone()),
                            input: TextInput::default(),
                        })),
                        _ => self.enter(resource.target()),
                    }
                }
                return;
            }
            _ => {
                if picker.query.handle_key(key) {
                    picker.selected = 0;
                }
            }
        }
        self.views.push(View::Picker(picker));
    }

    fn rename_form(&mut self, resource: Resource) {
        let name = match &resource {
            Resource::Session(session) => session.name.clone(),
            Resource::Window(window) => window.name.clone(),
            Resource::Pane(_) => {
                self.status =
                    "Panes have no independent tmux name. Use /window to rename their window."
                        .into();
                return;
            }
        };
        self.views.push(View::Form(Form {
            action: FormAction::Rename(resource),
            input: TextInput::new(name),
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
                let result = match resource {
                    Resource::Session(session) => self.tmux.rename_session(&session.id, &value),
                    Resource::Window(window) => self.tmux.rename_window(&window.id, &value),
                    Resource::Pane(_) => return,
                };
                match result {
                    Ok(()) => {
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
                        .min(picker.indices().len().saturating_sub(1));
                }
                Err(error) => self.status = error.to_string(),
            }
            self.views.push(View::Picker(picker));
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
        let area = frame.area();
        let area = Rect {
            x: area.x + area.width.min(2),
            width: area.width.saturating_sub(2),
            ..area
        };
        if self.views.top().is_none() {
            let commands = self.palette();
            let items: Vec<_> = commands
                .iter()
                .map(|c| CommandItem {
                    name: c.name,
                    description: c.description,
                })
                .collect();
            ui::render_home(
                frame,
                area,
                &self.composer,
                &items,
                self.palette_selected,
                if self.status.is_empty() {
                    None
                } else {
                    Some(&self.status)
                },
                self.theme,
            );
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
                let rows: Vec<_> = picker
                    .indices()
                    .iter()
                    .filter_map(|&i| picker.resources.get(i))
                    .map(|resource| resource.description(&self.context))
                    .collect();
                ui::render_picker(
                    frame,
                    main,
                    picker.purpose.title(),
                    &picker.query,
                    &rows,
                    picker.selected,
                    "No matches or sessions. Esc back · /new creates a session · Ctrl+R refreshes",
                    picker.purpose.footer(),
                    self.theme,
                );
            }
            Some(View::Form(form)) => {
                let (title, label) = match &form.action {
                    FormAction::NewSessionName => ("New session", "Session name"),
                    FormAction::NewSessionDirectory { .. } => (
                        "New session",
                        "Working directory (empty = current directory)",
                    ),
                    FormAction::Rename(_) => ("Rename", "New name"),
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
                confirm.scroll = ui::render_form_scrolled(
                    frame,
                    main,
                    "Confirm irreversible action",
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
            Some(View::Split(_, selected)) => {
                let rows = vec![
                    "Side by side — new pane on the right (tmux -h)".into(),
                    "Stacked — new pane below (tmux -v)".into(),
                ];
                ui::render_picker(
                    frame,
                    main,
                    "Split direction",
                    &TextInput::default(),
                    &rows,
                    *selected,
                    "",
                    "↑↓ Navigate · Enter Split · Esc Back",
                    self.theme,
                );
            }
            Some(View::Help) => ui::render_help(frame, main, self.theme),
            None => {}
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("› "),
                Span::styled(self.composer.value(), self.theme.style(self.theme.muted)),
            ])),
            chunks[1],
        );
        frame.render_widget(
            Paragraph::new(self.status.as_str()).style(self.theme.style(self.theme.error)),
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
            purpose: PickerPurpose::Resume,
            query: TextInput::new("ori"),
            resources: vec![
                Resource::Session(session("$1", "server")),
                Resource::Session(session("$2", "orion")),
                Resource::Session(session("$3", "work")),
            ],
            selected: 0,
        };
        let indices = picker.indices();
        assert_eq!(indices.len(), 1);
        assert_eq!(
            picker.selected_resource().map(|r| r.label()),
            Some("orion".into())
        );
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
