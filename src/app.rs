//! Application state and keyboard focus. Views never handle keys below the top
//! of the stack; all tmux mutations happen here, outside the rendering layer.
mod resume;

use std::{env, path::PathBuf};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
};
use resume::ResumePicker;

use crate::{
    commands::{self, CommandKind, ParsedCommand, SlashCommand},
    fuzzy,
    tmux::{
        Context, InsideContext, Pane, Session, SessionId, SplitDirection, Target, Tmux, TmuxResult,
        Window,
    },
    ui::{self, CommandItem, HomeDetails, TextInput, Theme},
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

#[derive(Debug, Clone, Copy)]
enum PickerRow {
    Resource(usize),
    NewWindow,
    Refresh,
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
    fn rows(&self) -> Vec<PickerRow> {
        let mut rows = Vec::new();
        if self.purpose == PickerPurpose::Windows && self.query.value().is_empty() {
            rows.push(PickerRow::NewWindow);
        }
        rows.extend(self.indices().into_iter().map(PickerRow::Resource));
        if self.query.value().is_empty()
            && matches!(
                self.purpose,
                PickerPurpose::List | PickerPurpose::Windows | PickerPurpose::Panes
            )
        {
            rows.push(PickerRow::Refresh);
        }
        rows
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
    directory: Option<String>,
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
            directory: working_directory_label(),
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
            Some(View::Resume(mut picker)) => {
                let target = picker.handle_key(key);
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
            C::Resume => {
                let mut picker = ResumePicker::default();
                self.reload_resume(&mut picker);
                self.views.push(View::Resume(picker));
            }
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
            Ok((sessions, windows)) => picker.replace(sessions, windows),
            Err(error) => self.status = error.to_string(),
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
        match key.code {
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => {
                picker.selected = (picker.selected + 1).min(picker.rows().len().saturating_sub(1))
            }
            KeyCode::Enter => match picker.rows().get(picker.selected).copied() {
                Some(PickerRow::NewWindow) => {
                    self.views.push(View::Picker(picker));
                    self.open_picker(PickerPurpose::NewWindow);
                    return;
                }
                Some(PickerRow::Refresh) => match self.read_resources(picker.purpose) {
                    Ok(resources) => {
                        picker.resources = resources;
                        picker.selected = 0;
                        self.status = "Refreshed".into();
                    }
                    Err(error) => self.status = error.to_string(),
                },
                Some(PickerRow::Resource(index)) => {
                    let resource = picker.resources[index].clone();
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
                    return;
                }
                None => {}
            },
            _ => {
                if picker.query.handle_key(key) {
                    picker.selected = 0;
                }
            }
        }
        self.views.push(View::Picker(picker));
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
                    picker.selected = picker.selected.min(picker.rows().len().saturating_sub(1));
                }
                Err(error) => self.status = error.to_string(),
            }
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
        let area = frame.area();
        let indent = if area.width >= 24 { 2 } else { 0 };
        let area = Rect {
            x: area.x + indent,
            width: area.width - indent,
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
            let (context, session) = match &self.context {
                Context::Outside => ("standalone", None),
                Context::Inside(inside) => (
                    if inside.popup { "popup" } else { "pane" },
                    inside.session_name.as_deref(),
                ),
                Context::ForeignServer => ("different server", None),
                Context::Unavailable => ("client unavailable", None),
            };
            ui::render_home(
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
                self.theme,
            );
            return;
        }
        if let Some(View::Resume(picker)) = self.views.0.last_mut() {
            picker.render(
                frame,
                area,
                self.context.current_session(),
                &self.status,
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
                    .rows()
                    .into_iter()
                    .map(|row| match row {
                        PickerRow::Resource(index) => {
                            picker.resources[index].description(&self.context)
                        }
                        PickerRow::NewWindow => "+ New window".into(),
                        PickerRow::Refresh => "Refresh list".into(),
                    })
                    .collect();
                ui::render_picker(
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
            Some(View::Resume(_)) | None => {}
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
            purpose: PickerPurpose::List,
            query: TextInput::new("ori"),
            resources: vec![
                Resource::Session(session("$1", "server")),
                Resource::Session(session("$2", "orion")),
                Resource::Session(session("$3", "work")),
            ],
            selected: 0,
        };
        let rows = picker.rows();
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
