//! Controller subview state machines, preview search, and rich tmux action helpers.
//!
//! Keeps new zero-memorization tmux controller features cleanly separated from `src/app.rs`.

use std::{collections::HashSet, env, path::PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    clipboard,
    fuzzy::FuzzyMatcher,
    snapshot::{self, SnapshotSummary, WorkspaceSnapshot},
    tmux::{
        ClientInfo, Context, LayoutPreset, Pane, PaneId, PasteBuffer, ResizeDirection, SessionId,
        SplitDirection, Target, Window, WindowId,
    },
    tmux_conf::{self, TmuxComfortOptions},
    ui::{self, ControllerAction, HomeQuickAction, PanePreview, PreviewAction, TextInput, Theme},
};

use super::{App, Confirm, Exit, Form, FormAction, Resource, ResourceAction, View};

/// Strip ANSI CSI/OSC escape sequences so preview search matches visible text accurately.
pub(super) fn strip_ansi_for_search(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
        } else if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// Interactive search state overlaid on any live terminal preview card (`f` to open).
#[derive(Debug, Clone, Default)]
pub(super) struct PreviewSearchState {
    pub active: bool,
    pub query: TextInput,
    pub match_lines: Vec<usize>,
    pub current_match: usize,
}

impl PreviewSearchState {
    pub fn reset(&mut self) {
        self.active = false;
        self.query.set_value("");
        self.match_lines.clear();
        self.current_match = 0;
    }

    pub fn query_str(&self) -> Option<&str> {
        let q = self.query.value();
        (!q.is_empty()).then_some(q)
    }

    /// Recompute matching line indices (case-insensitive on visible text) against `lines`.
    pub fn recompute(&mut self, lines: &[String]) {
        self.match_lines.clear();
        let needle = self.query.value().to_ascii_lowercase();
        if needle.is_empty() {
            self.current_match = 0;
            return;
        }
        for (idx, line) in lines.iter().enumerate() {
            let plain = strip_ansi_for_search(line).to_ascii_lowercase();
            if plain.contains(&needle) {
                self.match_lines.push(idx);
            }
        }
        if self.match_lines.is_empty() {
            self.current_match = 0;
        } else {
            self.current_match = self.current_match.min(self.match_lines.len() - 1);
        }
    }

    pub fn step_match(&mut self, forward: bool) -> Option<usize> {
        if self.match_lines.is_empty() {
            return None;
        }
        let len = self.match_lines.len();
        self.current_match = if forward {
            (self.current_match + 1) % len
        } else {
            (self.current_match + len - 1) % len
        };
        self.match_lines.get(self.current_match).copied()
    }

    /// Compute the scroll-from-bottom offset so `line_idx` is centered in `visible_rows`.
    pub fn scroll_offset_for_line(
        total_lines: usize,
        visible_rows: usize,
        line_idx: usize,
    ) -> usize {
        if total_lines <= visible_rows {
            return 0;
        }
        let max_scroll = total_lines.saturating_sub(visible_rows);
        let desired_end = line_idx
            .saturating_add(visible_rows / 2 + 1)
            .min(total_lines);
        total_lines.saturating_sub(desired_end).min(max_scroll)
    }
}

/// Target of a destructive `yes`-typed confirmation modal.
#[derive(Debug, Clone)]
pub(super) enum ConfirmTarget {
    Resource(Box<Resource>),
    KillServer,
}

impl ConfirmTarget {
    pub fn summary(&self, lang: ui::Language) -> String {
        match self {
            Self::Resource(res) => res.label(),
            Self::KillServer => {
                if lang.is_zh() {
                    "整个 tmux 服务进程 (kill-server)".into()
                } else {
                    "Entire tmux server (kill-server)".into()
                }
            }
        }
    }
}

/// Interactive two-field form state (e.g., New Window name + optional working directory).
#[derive(Debug, Clone)]
pub(super) enum DualFormAction {
    NewWindow(SessionId),
}

#[derive(Debug, Clone)]
pub(super) struct DualForm {
    pub action: DualFormAction,
    pub first_input: TextInput,
    pub second_input: TextInput,
    pub active_field: usize,
}

impl DualForm {
    pub fn new_window(session_id: SessionId) -> Self {
        Self {
            action: DualFormAction::NewWindow(session_id),
            first_input: TextInput::default(),
            second_input: TextInput::default(),
            active_field: 0,
        }
    }

    pub fn active_input_mut(&mut self) -> &mut TextInput {
        if self.active_field == 0 {
            &mut self.first_input
        } else {
            &mut self.second_input
        }
    }

    pub fn toggle_field(&mut self) {
        self.active_field = if self.active_field == 0 { 1 } else { 0 };
    }
}

/// Purpose for the reusable target selection modal (`View::TargetPick`).
#[derive(Debug, Clone)]
pub(super) enum TargetPickPurpose {
    MoveWindow(Window),
    LinkWindow(Window),
    SwapPane(Pane),
    JoinPane(Pane),
    PasteBuffer { name: String },
}

impl TargetPickPurpose {
    pub fn title(&self, lang: ui::Language) -> &'static str {
        let zh = lang.is_zh();
        match self {
            Self::MoveWindow(_) => {
                if zh {
                    "移动窗口到会话"
                } else {
                    "Move Window to Session"
                }
            }
            Self::LinkWindow(_) => {
                if zh {
                    "共享链接窗口到会话"
                } else {
                    "Link Window to Session"
                }
            }
            Self::SwapPane(_) => {
                if zh {
                    "与目标窗格互换"
                } else {
                    "Swap Pane With"
                }
            }
            Self::JoinPane(_) => {
                if zh {
                    "合并窗格到窗口"
                } else {
                    "Join Pane Into Window"
                }
            }
            Self::PasteBuffer { .. } => {
                if zh {
                    "粘贴缓冲区到窗格"
                } else {
                    "Paste Buffer Into Pane"
                }
            }
        }
    }

    pub fn action_banner(&self, lang: ui::Language) -> String {
        let zh = lang.is_zh();
        match self {
            Self::MoveWindow(w) => {
                if zh {
                    format!(
                        "选择接收窗口 {}:{} ({}) 的目标会话",
                        w.session_name, w.index, w.name
                    )
                } else {
                    format!(
                        "Choose destination session for window {}:{} ({})",
                        w.session_name, w.index, w.name
                    )
                }
            }
            Self::LinkWindow(w) => {
                if zh {
                    format!(
                        "选择共享链接窗口 {}:{} ({}) 的目标会话",
                        w.session_name, w.index, w.name
                    )
                } else {
                    format!(
                        "Choose destination session to link window {}:{} ({})",
                        w.session_name, w.index, w.name
                    )
                }
            }
            Self::SwapPane(p) => {
                if zh {
                    format!(
                        "选择与窗格 {} ({}:{}.{}) 互换位置的目标窗格",
                        p.id, p.session_name, p.window_index, p.index
                    )
                } else {
                    format!(
                        "Choose pane to swap positions with {} ({}:{}.{})",
                        p.id, p.session_name, p.window_index, p.index
                    )
                }
            }
            Self::JoinPane(p) => {
                if zh {
                    format!("选择将窗格 {} ({}) 并入的目标窗口", p.id, p.current_command)
                } else {
                    format!(
                        "Choose destination window to join pane {} ({})",
                        p.id, p.current_command
                    )
                }
            }
            Self::PasteBuffer { name } => {
                if zh {
                    format!("选择要粘贴缓冲区 '{name}' 的目标窗格")
                } else {
                    format!("Choose target pane to paste buffer '{name}' into")
                }
            }
        }
    }
}

/// Generic fuzzy target picker used by Move/Link Window, Swap/Join Pane, and Paste Buffer.
#[derive(Debug, Clone)]
pub(super) struct TargetPickState {
    pub purpose: TargetPickPurpose,
    pub query: TextInput,
    pub resources: Vec<Resource>,
    pub selected: usize,
}

impl TargetPickState {
    pub fn filtered_indices(&self, matcher: &mut FuzzyMatcher) -> Vec<(usize, Vec<usize>)> {
        let mut matched: Vec<_> = self
            .resources
            .iter()
            .enumerate()
            .filter_map(|(idx, res)| {
                matcher
                    .match_one(self.query.value(), &res.label())
                    .map(|m| (m.sort_key(), idx, m.indices))
            })
            .collect();
        matched.sort_by(|(a_key, a_idx, _), (b_key, b_idx, _)| (a_key, a_idx).cmp(&(b_key, b_idx)));
        matched
            .into_iter()
            .map(|(_, idx, indices)| (idx, indices))
            .collect()
    }

    pub fn rich_rows(
        &self,
        matcher: &mut FuzzyMatcher,
        context: &crate::tmux::Context,
        theme: Theme,
    ) -> Vec<ui::RichPickerRow> {
        self.filtered_indices(matcher)
            .into_iter()
            .map(|(idx, indices)| self.resources[idx].rich_row(indices, context, theme))
            .collect()
    }
}

/// Interactive state for `/layout` (`View::LayoutStudio`).
#[derive(Debug, Clone)]
pub(super) struct LayoutStudioState {
    pub window_id: WindowId,
    pub pane_id: PaneId,
    pub target_label: String,
    pub selected_preset: usize,
    pub pane_size: (u32, u32),
    pub pane_count: usize,
    pub resize_step: u16,
    pub zoomed: bool,
    pub synchronized: bool,
    pub status_note: Option<String>,
}

impl LayoutStudioState {
    pub fn preset_at(index: usize) -> LayoutPreset {
        LayoutPreset::ALL
            .get(index)
            .copied()
            .unwrap_or(LayoutPreset::Tiled)
    }
}

/// Interactive state for `/clients` (`View::Clients`).
#[derive(Debug, Clone)]
pub(super) struct ClientsViewState {
    pub clients: Vec<ClientInfo>,
    pub selected: usize,
    pub status_note: Option<String>,
}

impl ClientsViewState {
    /// Returns true when multiple clients attach to the same session with differing dimensions,
    /// which causes tmux's infamous dotted-border size lock.
    pub fn has_size_mismatch(&self) -> bool {
        if self.clients.len() < 2 {
            return false;
        }
        for (i, a) in self.clients.iter().enumerate() {
            for b in self.clients.iter().skip(i + 1) {
                let same_session = a.session_id.is_some() && a.session_id == b.session_id
                    || (!a.session_name.is_empty() && a.session_name == b.session_name);
                if same_session && (a.width != b.width || a.height != b.height) {
                    return true;
                }
            }
        }
        false
    }
}

/// Interactive state for `/buffers` (`View::Buffers`).
#[derive(Debug, Clone)]
pub(super) struct BuffersViewState {
    pub buffers: Vec<PasteBuffer>,
    pub selected: usize,
    pub preview_lines: Vec<String>,
    pub preview_scroll: usize,
    pub status_note: Option<String>,
}

/// Interactive state for `/snapshot` (`View::Snapshots`).
#[derive(Debug, Clone)]
pub(super) struct SnapshotsViewState {
    pub snapshots: Vec<SnapshotSummary>,
    pub selected: usize,
    pub tree_preview: Vec<String>,
    pub preview_scroll: usize,
    pub status_note: Option<String>,
}

/// Build the rich, localized, context-aware action menu for a Session, Window, or Pane.
pub(super) fn build_resource_actions(
    resource: &Resource,
    piping_panes: &HashSet<PaneId>,
    lang: ui::Language,
) -> Vec<(String, ResourceAction)> {
    use ResourceAction as A;
    let zh = lang.is_zh();
    match resource {
        Resource::Session(_) => {
            if zh {
                vec![
                    ("恢复附着会话".into(), A::Open),
                    ("只读模式附着 (attach -r)".into(), A::AttachReadOnly),
                    ("新建窗口".into(), A::NewWindow),
                    ("重命名会话".into(), A::Rename),
                    ("踢出其他客户端（解除点阵边框）".into(), A::DetachOthers),
                    ("保存会话快照".into(), A::SaveSnapshot),
                    ("关闭会话".into(), A::Close),
                ]
            } else {
                vec![
                    ("Resume session".into(), A::Open),
                    ("Attach read-only (attach -r)".into(), A::AttachReadOnly),
                    ("New window".into(), A::NewWindow),
                    ("Rename session".into(), A::Rename),
                    (
                        "Detach other clients (fix dotted border)".into(),
                        A::DetachOthers,
                    ),
                    ("Save snapshot".into(), A::SaveSnapshot),
                    ("Close session".into(), A::Close),
                ]
            }
        }
        Resource::Window(w) => {
            let sync_label = if zh {
                if w.synchronized {
                    "关闭同步输入 [当前: 已开启]"
                } else {
                    "开启同步输入 (synchronize-panes)"
                }
            } else if w.synchronized {
                "Disable synchronize panes [currently ON]"
            } else {
                "Enable synchronize panes"
            };
            let monitor_label = if zh {
                if w.monitor_activity {
                    "切换监控模式 [当前: 活动监控 -> 静默 30s]"
                } else if w.monitor_silence > 0 {
                    "切换监控模式 [当前: 静默监控 -> 关闭]"
                } else {
                    "切换监控模式 [当前: 关闭 -> 活动监控]"
                }
            } else if w.monitor_activity {
                "Cycle monitor [Activity -> Silence 30s]"
            } else if w.monitor_silence > 0 {
                "Cycle monitor [Silence -> Off]"
            } else {
                "Cycle monitor [Off -> Activity]"
            };
            if zh {
                vec![
                    ("打开窗口".into(), A::Open),
                    ("在分屏工作台编辑".into(), A::EditInSplitStudio),
                    ("布局预设与窗格缩放".into(), A::LayoutAndResize),
                    (sync_label.into(), A::ToggleSyncPanes),
                    (monitor_label.into(), A::CycleMonitor),
                    ("移动到其他会话".into(), A::MoveWindowToSession),
                    ("共享链接到其他会话".into(), A::LinkWindowToSession),
                    ("向左调整顺序 (swap-window)".into(), A::ReorderWindowLeft),
                    ("向右调整顺序 (swap-window)".into(), A::ReorderWindowRight),
                    ("重命名窗口".into(), A::Rename),
                    ("关闭窗口".into(), A::Close),
                ]
            } else {
                vec![
                    ("Open window".into(), A::Open),
                    ("Edit in Split Studio".into(), A::EditInSplitStudio),
                    ("Layout & Resize".into(), A::LayoutAndResize),
                    (sync_label.into(), A::ToggleSyncPanes),
                    (monitor_label.into(), A::CycleMonitor),
                    ("Move to another session".into(), A::MoveWindowToSession),
                    ("Link to another session".into(), A::LinkWindowToSession),
                    ("Reorder left (swap-window)".into(), A::ReorderWindowLeft),
                    ("Reorder right (swap-window)".into(), A::ReorderWindowRight),
                    ("Rename window".into(), A::Rename),
                    ("Close window".into(), A::Close),
                ]
            }
        }
        Resource::Pane(p) => {
            let zoom_label = if zh {
                if p.zoomed {
                    "取消全屏缩放 (unzoom)"
                } else {
                    "全屏缩放窗格 (zoom)"
                }
            } else if p.zoomed {
                "Unzoom pane (resize-pane -Z)"
            } else {
                "Zoom pane (resize-pane -Z)"
            };
            let is_piping = piping_panes.contains(&p.id);
            let pipe_label = if zh {
                if is_piping {
                    "停止实时日志记录 (pipe-pane)"
                } else {
                    "开启实时日志记录 (pipe-pane)"
                }
            } else if is_piping {
                "Stop live pipe-pane logging"
            } else {
                "Start live pipe-pane logging"
            };
            if zh {
                vec![
                    ("跳转到窗格".into(), A::Open),
                    (zoom_label.into(), A::ToggleZoomPane),
                    ("向右拆分窗格".into(), A::SplitPaneRight),
                    ("向下拆分窗格".into(), A::SplitPaneDown),
                    ("拆出为独立窗口 (break-pane)".into(), A::BreakPaneOut),
                    ("与其他窗格互换 (swap-pane)".into(), A::SwapPaneWith),
                    ("合并到其他窗口 (join-pane)".into(), A::JoinPaneInto),
                    ("交互式缩放与布局".into(), A::LayoutAndResize),
                    ("复制输出到剪贴板".into(), A::CopyPaneOutput),
                    ("导出输出日志到文件".into(), A::ExportPaneLog),
                    (pipe_label.into(), A::TogglePipePane),
                    ("发送 Ctrl+C 中断信号".into(), A::SendCtrlC),
                    ("发送命令 (send-keys)".into(), A::SendCommand),
                    ("清空滚动历史 (clear-history)".into(), A::ClearPaneHistory),
                    ("重启窗格进程 (respawn-pane -k)".into(), A::RespawnPane),
                    ("重命名窗格标题 (select-pane -T)".into(), A::RenamePaneTitle),
                    ("关闭窗格".into(), A::Close),
                ]
            } else {
                vec![
                    ("Jump to pane".into(), A::Open),
                    (zoom_label.into(), A::ToggleZoomPane),
                    ("Split right".into(), A::SplitPaneRight),
                    ("Split down".into(), A::SplitPaneDown),
                    (
                        "Break out to new window (break-pane)".into(),
                        A::BreakPaneOut,
                    ),
                    ("Swap with pane (swap-pane)".into(), A::SwapPaneWith),
                    ("Join into window (join-pane)".into(), A::JoinPaneInto),
                    ("Interactive resize & layout".into(), A::LayoutAndResize),
                    ("Copy output to clipboard".into(), A::CopyPaneOutput),
                    ("Export log to file".into(), A::ExportPaneLog),
                    (pipe_label.into(), A::TogglePipePane),
                    ("Send Ctrl+C interrupt".into(), A::SendCtrlC),
                    ("Send command (send-keys)".into(), A::SendCommand),
                    ("Clear scrollback history".into(), A::ClearPaneHistory),
                    ("Respawn pane (respawn-pane -k)".into(), A::RespawnPane),
                    (
                        "Rename pane title (select-pane -T)".into(),
                        A::RenamePaneTitle,
                    ),
                    ("Close pane".into(), A::Close),
                ]
            }
        }
    }
}

impl<'a> App<'a> {
    /// Resolve the pane currently focused by the user (selected Home preview pane, or active pane).
    pub(super) fn focused_or_active_pane(&self) -> Option<Pane> {
        if let Some(Target::Pane(pid)) = &self.preview_target
            && let Some(pane) = self.home_panes.iter().find(|p| &p.id == pid)
        {
            return Some(pane.clone());
        }
        if let Some((_, win)) = self.matrix.preview_window()
            && let Some(pane) = self.representative_pane_for_window(&win.id)
        {
            return Some(pane);
        }
        self.active_or_first_pane()
    }

    /// Resolve the window currently focused by the user (Home matrix window or active pane's window).
    pub(super) fn focused_or_active_window(&self) -> Option<Window> {
        if let Some((_, win)) = self.matrix.preview_window() {
            return Some(win.clone());
        }
        self.active_or_first_pane()
            .and_then(|p| {
                self.home_windows
                    .iter()
                    .find(|w| w.id == p.window_id)
                    .cloned()
            })
            .or_else(|| self.home_windows.first().cloned())
    }

    /// Handle keys while interactive live-preview search (`f`) is active.
    pub(super) fn handle_preview_search_key(&mut self, key: KeyEvent) -> bool {
        if !self.preview_search.active {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.preview_search.reset();
                self.status.clear();
                true
            }
            KeyCode::Enter => {
                self.preview_search.active = false;
                if self.preview_search.query.value().is_empty() {
                    self.preview_search.reset();
                }
                true
            }
            KeyCode::Down | KeyCode::Tab => {
                self.step_preview_search_match(true);
                true
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.step_preview_search_match(false);
                true
            }
            _ => {
                if self.preview_search.query.handle_key(key) {
                    self.recompute_preview_search(true);
                }
                true
            }
        }
    }

    pub(super) fn open_preview_search(&mut self) {
        if !self.config.live_preview || self.cached_preview.is_none() {
            self.status = self
                .theme
                .tr(
                    "Enable live_preview in /config to search pane output.",
                    "请先在 /config 中启用实时预览后再搜索窗格输出。",
                )
                .into();
            return;
        }
        self.preview_search.active = true;
        self.recompute_preview_search(true);
    }

    pub(super) fn recompute_preview_search(&mut self, jump_to_match: bool) {
        let Some((_, _, lines)) = self.cached_preview.as_ref() else {
            self.preview_search.match_lines.clear();
            self.preview_search.current_match = 0;
            return;
        };
        let total = lines.len();
        self.preview_search.recompute(lines);
        if jump_to_match
            && let Some(&line_idx) = self
                .preview_search
                .match_lines
                .get(self.preview_search.current_match)
        {
            let rows = self.preview_visible_rows();
            self.preview_scroll = PreviewSearchState::scroll_offset_for_line(total, rows, line_idx);
        }
    }

    pub(super) fn step_preview_search_match(&mut self, forward: bool) {
        let Some((_, _, lines)) = self.cached_preview.as_ref() else {
            return;
        };
        let total = lines.len();
        if let Some(line_idx) = self.preview_search.step_match(forward) {
            let rows = self.preview_visible_rows();
            self.preview_scroll = PreviewSearchState::scroll_offset_for_line(total, rows, line_idx);
        }
    }

    pub(super) fn cycle_home_preview_pane(&mut self, forward: bool) -> bool {
        if self.home_preview_pane_total <= 1 {
            return false;
        }
        let n = self.home_preview_pane_total;
        self.home_preview_pane_idx = if forward {
            (self.home_preview_pane_idx + 1) % n
        } else {
            (self.home_preview_pane_idx + n - 1) % n
        };
        self.refresh_home_preview();
        true
    }

    /// Execute any rich Session/Window/Pane context action.
    pub(super) fn execute_resource_action(&mut self, resource: Resource, action: ResourceAction) {
        use ResourceAction as A;
        match action {
            A::Open => self.enter(resource.target()),
            A::AttachReadOnly => self.enter_readonly(resource.target()),
            A::NewWindow => {
                if let Resource::Session(session) = resource {
                    self.views
                        .push(View::DualForm(DualForm::new_window(session.id)));
                }
            }
            A::Rename => self.rename_form(resource),
            A::DetachOthers => {
                let sess = match &resource {
                    Resource::Session(s) => Some(s.id.clone()),
                    _ => None,
                };
                self.detach_other_clients_action(sess.as_ref());
            }
            A::SaveSnapshot => {
                let filter = match &resource {
                    Resource::Session(s) => Some(s.id.clone()),
                    _ => None,
                };
                self.views.push(View::Form(Form {
                    action: FormAction::SaveSnapshotName {
                        session_filter: filter,
                    },
                    input: TextInput::default(),
                }));
            }
            A::Close => self.kill_confirm(resource),
            A::EditInSplitStudio => self.open_split_studio(Some(resource)),
            A::LayoutAndResize => self.open_layout_studio(Some(resource)),
            A::ToggleSyncPanes => {
                if let Resource::Window(w) = resource {
                    self.toggle_sync_panes_action(&w);
                }
            }
            A::CycleMonitor => {
                if let Resource::Window(w) = resource {
                    self.cycle_window_monitor_action(&w);
                }
            }
            A::MoveWindowToSession => {
                if let Resource::Window(w) = resource {
                    self.open_target_pick(TargetPickPurpose::MoveWindow(w));
                }
            }
            A::LinkWindowToSession => {
                if let Resource::Window(w) = resource {
                    self.open_target_pick(TargetPickPurpose::LinkWindow(w));
                }
            }
            A::ReorderWindowLeft => {
                if let Resource::Window(w) = resource {
                    self.reorder_window_action(&w, true);
                }
            }
            A::ReorderWindowRight => {
                if let Resource::Window(w) = resource {
                    self.reorder_window_action(&w, false);
                }
            }
            A::ToggleZoomPane => {
                if let Resource::Pane(p) = resource {
                    self.toggle_zoom_pane_action(&p);
                }
            }
            A::SplitPaneRight => {
                if let Resource::Pane(p) = resource {
                    self.split_specific_pane_action(&p, SplitDirection::SideBySide);
                }
            }
            A::SplitPaneDown => {
                if let Resource::Pane(p) = resource {
                    self.split_specific_pane_action(&p, SplitDirection::Stacked);
                }
            }
            A::BreakPaneOut => {
                if let Resource::Pane(p) = resource {
                    self.break_pane_action(&p);
                }
            }
            A::SwapPaneWith => {
                if let Resource::Pane(p) = resource {
                    self.open_target_pick(TargetPickPurpose::SwapPane(p));
                }
            }
            A::JoinPaneInto => {
                if let Resource::Pane(p) = resource {
                    self.open_target_pick(TargetPickPurpose::JoinPane(p));
                }
            }
            A::CopyPaneOutput => {
                if let Resource::Pane(p) = resource {
                    self.copy_target_output_to_clipboard(&Target::Pane(p.id));
                }
            }
            A::ExportPaneLog => {
                if let Resource::Pane(p) = resource {
                    self.export_target_log_to_file(
                        &Target::Pane(p.id),
                        &p.session_name,
                        &p.window_name,
                    );
                }
            }
            A::TogglePipePane => {
                if let Resource::Pane(p) = resource {
                    self.toggle_pipe_pane_action(&p);
                }
            }
            A::SendCtrlC => {
                if let Resource::Pane(p) = resource {
                    self.send_ctrl_c_action(&p);
                }
            }
            A::SendCommand => {
                if let Resource::Pane(p) = resource {
                    self.views.push(View::Form(Form {
                        action: FormAction::SendCommandToPane(p),
                        input: TextInput::default(),
                    }));
                }
            }
            A::ClearPaneHistory => {
                if let Resource::Pane(p) = resource {
                    self.clear_pane_history_action(&p);
                }
            }
            A::RespawnPane => {
                if let Resource::Pane(p) = resource {
                    self.respawn_pane_action(&p);
                }
            }
            A::RenamePaneTitle => {
                if let Resource::Pane(p) = resource {
                    let mut input = TextInput::default();
                    if !p.title.is_empty() {
                        input.set_value(&p.title);
                    }
                    self.views.push(View::Form(Form {
                        action: FormAction::RenamePaneTitle(p),
                        input,
                    }));
                }
            }
        }
    }

    pub(super) fn enter_readonly(&mut self, target: Target) {
        self.context = self.tmux.context();
        match &self.context {
            Context::Outside => match self.tmux.select(&target) {
                Ok(()) => self.exit = Some(Exit::AttachReadOnly(target)),
                Err(error) => self.status = error.to_string(),
            },
            Context::Inside(_) => {
                self.status = self
                    .theme
                    .tr(
                        "Read-only attach requires launching jm outside tmux (or run `tmux attach -r`).",
                        "只读模式附着需要在 tmux 外部启动 jm（或运行 `tmux attach -r`）。",
                    )
                    .into();
            }
            Context::ForeignServer => {
                self.status = self
                    .theme
                    .tr(
                        "Cannot attach read-only from another tmux server; open jm outside tmux.",
                        "无法在另一个 tmux 服务中嵌套只读附着，请在 tmux 外部打开 jm。",
                    )
                    .into();
            }
            Context::Unavailable => {
                self.status = self
                    .theme
                    .tr(
                        "Launching tmux client is unavailable; open jm outside tmux to attach read-only.",
                        "启动时的 tmux 客户端不可用，请在 tmux 外部打开 jm 进行只读附着。",
                    )
                    .into();
            }
        }
    }

    pub(super) fn toggle_zoom_pane_action(&mut self, pane: &Pane) {
        match self.tmux.toggle_zoom_pane(&pane.id) {
            Ok(()) => {
                self.refresh_parent();
                self.status = if pane.zoomed {
                    self.theme
                        .tr("✦ Restored pane layout (unzoomed)", "✦ 已恢复窗格分屏大小")
                        .into()
                } else {
                    self.theme
                        .tr("✦ Zoomed pane to fullscreen (⛶)", "✦ 已将窗格全屏放大 (⛶)")
                        .into()
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn split_specific_pane_action(&mut self, pane: &Pane, direction: SplitDirection) {
        match self.tmux.split_pane(&pane.id, direction) {
            Ok(_) => {
                self.refresh_parent();
                self.status = self.theme.tr("✦ Pane split.", "✦ 窗格已拆分。").into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn break_pane_action(&mut self, pane: &Pane) {
        match self.tmux.break_pane(&pane.id, None) {
            Ok(win) => {
                self.refresh_parent();
                self.status = if self.theme.language.is_zh() {
                    format!(
                        "✦ 已拆出窗格为独立窗口 {}:{} ({})",
                        win.session_name, win.index, win.name
                    )
                } else {
                    format!(
                        "✦ Broke pane out into window {}:{} ({})",
                        win.session_name, win.index, win.name
                    )
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn toggle_sync_panes_action(&mut self, window: &Window) {
        let next = !window.synchronized;
        match self.tmux.set_synchronize_panes(&window.id, next) {
            Ok(()) => {
                self.refresh_parent();
                self.status = if next {
                    self.theme
                        .tr(
                            "⚡ Synchronize-panes ON — typing broadcasts to EVERY pane in this window!",
                            "⚡ 同步输入已开启 — 按键将同步广播到本窗口全部窗格！",
                        )
                        .into()
                } else {
                    self.theme
                        .tr("✦ Synchronize-panes OFF", "✦ 已关闭同步输入")
                        .into()
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn cycle_window_monitor_action(&mut self, window: &Window) {
        let (res, msg_en, msg_zh) = if window.monitor_activity {
            let r = self
                .tmux
                .set_monitor_activity(&window.id, false)
                .and_then(|()| self.tmux.set_monitor_silence(&window.id, 30));
            (
                r,
                "✦ Monitor set to Silence (30s)",
                "✦ 监控模式已切换为：静默监控 (30秒)",
            )
        } else if window.monitor_silence > 0 {
            let r = self
                .tmux
                .set_monitor_activity(&window.id, false)
                .and_then(|()| self.tmux.set_monitor_silence(&window.id, 0));
            (r, "✦ Window monitoring OFF", "✦ 已关闭窗口监控")
        } else {
            let r = self
                .tmux
                .set_monitor_silence(&window.id, 0)
                .and_then(|()| self.tmux.set_monitor_activity(&window.id, true));
            (
                r,
                "✦ Monitor set to Activity (●)",
                "✦ 监控模式已切换为：活动监控 (●)",
            )
        };
        match res {
            Ok(()) => {
                self.refresh_parent();
                self.status = self.theme.tr(msg_en, msg_zh).into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn reorder_window_action(&mut self, window: &Window, move_left: bool) {
        self.refresh_home_data();
        let mut siblings: Vec<&Window> = self
            .home_windows
            .iter()
            .filter(|w| w.session_id == window.session_id)
            .collect();
        siblings.sort_by_key(|w| w.index);
        let Some(pos) = siblings.iter().position(|w| w.id == window.id) else {
            return;
        };
        let neighbor = if move_left {
            pos.checked_sub(1).and_then(|i| siblings.get(i))
        } else {
            siblings.get(pos + 1)
        };
        let Some(neighbor) = neighbor else {
            self.status = if move_left {
                self.theme
                    .tr("Window is already leftmost.", "该窗口已在最左侧。")
                    .into()
            } else {
                self.theme
                    .tr("Window is already rightmost.", "该窗口已在最右侧。")
                    .into()
            };
            return;
        };
        let neighbor_id = neighbor.id.clone();
        match self.tmux.swap_window(&window.id, &neighbor_id) {
            Ok(()) => {
                self.refresh_parent();
                self.status = self
                    .theme
                    .tr("✦ Reordered window", "✦ 已调整窗口顺序")
                    .into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn detach_other_clients_action(&mut self, session: Option<&SessionId>) {
        self.context = self.tmux.context();
        let keep = match &self.context {
            Context::Inside(inside) => inside.client_name.as_deref(),
            _ => None,
        };
        match self.tmux.detach_other_clients(session, keep) {
            Ok(0) => {
                self.status = self
                    .theme
                    .tr(
                        "No other attached clients to detach.",
                        "没有其他附着的客户端需要踢出。",
                    )
                    .into();
            }
            Ok(count) => {
                self.refresh_parent();
                self.status = if self.theme.language.is_zh() {
                    format!("✦ 已踢出 {count} 个其他客户端，终端尺寸锁定已解除")
                } else {
                    format!("✦ Detached {count} other client(s) — full screen size restored")
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn send_ctrl_c_action(&mut self, pane: &Pane) {
        match self.tmux.send_keys(&pane.id, &["C-c"], false) {
            Ok(()) => {
                self.refresh_parent();
                self.status = if self.theme.language.is_zh() {
                    format!("✦ 已向窗格 {} 发送 Ctrl+C 中断信号", pane.id)
                } else {
                    format!("✦ Sent Ctrl+C interrupt to pane {}", pane.id)
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn clear_pane_history_action(&mut self, pane: &Pane) {
        match self.tmux.clear_pane_history(&pane.id) {
            Ok(()) => {
                self.refresh_parent();
                self.status = self
                    .theme
                    .tr("✦ Cleared pane scrollback history", "✦ 已清空窗格滚动历史")
                    .into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn respawn_pane_action(&mut self, pane: &Pane) {
        match self.tmux.respawn_pane(&pane.id, true) {
            Ok(()) => {
                self.refresh_parent();
                self.status = if self.theme.language.is_zh() {
                    format!("✦ 已重启窗格 {} 的进程", pane.id)
                } else {
                    format!("✦ Respawned pane {}", pane.id)
                };
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn copy_target_output_to_clipboard(&mut self, target: &Target) {
        match self
            .tmux
            .capture_pane_plain(target, super::PREVIEW_CAPTURE_LINES)
        {
            Ok(lines) => {
                let text = lines.join("\n");
                match clipboard::copy_to_clipboard(&text, Some(self.tmux)) {
                    Ok(outcome) => {
                        self.status = format!("✦ {}", outcome.label(self.theme.language.is_zh()));
                    }
                    Err(error) => self.status = error,
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn copy_current_preview_to_clipboard(&mut self) {
        if let Some(target) = self.preview_target.clone() {
            self.copy_target_output_to_clipboard(&target);
            return;
        }
        if let Some((_, _, lines)) = &self.cached_preview {
            let text = lines
                .iter()
                .map(|l| clipboard::strip_ansi_escapes(l))
                .collect::<Vec<_>>()
                .join("\n");
            match clipboard::copy_to_clipboard(&text, Some(self.tmux)) {
                Ok(outcome) => {
                    self.status = format!("✦ {}", outcome.label(self.theme.language.is_zh()));
                }
                Err(error) => self.status = error,
            }
        }
    }

    pub(super) fn export_target_log_to_file(
        &mut self,
        target: &Target,
        session: &str,
        window: &str,
    ) {
        match self
            .tmux
            .capture_pane_plain(target, super::PREVIEW_CAPTURE_LINES)
        {
            Ok(lines) => match clipboard::export_log_to_file(session, window, &lines, None) {
                Ok(path) => {
                    let short = Self::shorten_home_path(&path);
                    self.status = if self.theme.language.is_zh() {
                        format!("✦ 已导出日志到 {short}")
                    } else {
                        format!("✦ Exported log to {short}")
                    };
                }
                Err(error) => self.status = error,
            },
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn export_current_preview_log(&mut self) {
        let Some(target) = self.preview_target.clone() else {
            return;
        };
        let (session_name, window_name) = self.preview_target_session_window(&target);
        self.export_target_log_to_file(&target, &session_name, &window_name);
    }

    pub(super) fn toggle_pipe_pane_action(&mut self, pane: &Pane) {
        if self.piping_panes.contains(&pane.id) {
            match self.tmux.toggle_pipe_pane(&pane.id, None) {
                Ok(()) => {
                    self.piping_panes.remove(&pane.id);
                    self.status = self
                        .theme
                        .tr("✦ Stopped live pipe-pane logging", "✦ 已停止实时日志记录")
                        .into();
                }
                Err(error) => self.status = error.to_string(),
            }
        } else {
            let log_path = clipboard::default_pipe_log_path(
                &pane.session_name,
                &pane.window_name,
                pane.id.as_str(),
            );
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match self.tmux.toggle_pipe_pane(&pane.id, Some(&log_path)) {
                Ok(()) => {
                    self.piping_panes.insert(pane.id.clone());
                    let short = Self::shorten_home_path(&log_path);
                    self.status = if self.theme.language.is_zh() {
                        format!("✦ 已开启实时日志记录 -> {short}")
                    } else {
                        format!("✦ Live pipe-pane logging -> {short}")
                    };
                }
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    pub(super) fn toggle_current_preview_pipe(&mut self) {
        if let Some(pane) = self.focused_or_active_pane() {
            self.toggle_pipe_pane_action(&pane);
        }
    }

    fn preview_target_session_window(&self, target: &Target) -> (String, String) {
        match target {
            Target::Pane(pid) => self
                .home_panes
                .iter()
                .find(|p| &p.id == pid)
                .map(|p| (p.session_name.clone(), p.window_name.clone()))
                .unwrap_or_else(|| ("pane".into(), pid.as_str().to_owned())),
            Target::Window(_, wid) => self
                .home_windows
                .iter()
                .find(|w| &w.id == wid)
                .map(|w| (w.session_name.clone(), w.name.clone()))
                .unwrap_or_else(|| ("window".into(), wid.as_str().to_owned())),
            Target::Session(sid) => self
                .home_sessions
                .iter()
                .find(|s| &s.id == sid)
                .map(|s| (s.name.clone(), "active".into()))
                .unwrap_or_else(|| ("session".into(), sid.as_str().to_owned())),
        }
    }

    /// Build the rich [`PanePreview`] view-model for Home, LiveRadar, Picker, and TargetPicker.
    pub(super) fn build_pane_preview<'p>(
        &'p self,
        show_pane_switcher: bool,
    ) -> Option<PanePreview<'p>> {
        let (title, subtitle, lines) = self
            .cached_preview
            .as_ref()
            .filter(|_| self.config.live_preview)?;
        let (zoomed, synchronized, piping) = self.preview_status_flags();
        let pane_index = (show_pane_switcher && self.home_preview_pane_total > 1)
            .then_some((self.home_preview_pane_idx + 1, self.home_preview_pane_total));
        let match_count = self.preview_search.match_lines.len();
        let current_match = if match_count > 0 {
            self.preview_search.current_match + 1
        } else {
            0
        };
        let pip_active = self.desktop.pip_target.as_ref().is_some_and(|pid| {
            self.preview_target.as_ref() == Some(&Target::Pane(pid.clone()))
                || self
                    .focused_or_active_pane()
                    .is_some_and(|pane| &pane.id == pid)
        });
        let hovered_action = match self.desktop.hover {
            super::desktop::HoverTarget::PreviewAction(act) => Some(act),
            _ => None,
        };
        let dock_highlight = self.desktop.drag.as_ref().and_then(|d| {
            d.dock_target
                .and_then(|(rect, zone)| (self.preview_rect == Some(rect)).then_some(zone))
        });
        Some(PanePreview {
            title: title.as_str(),
            subtitle: subtitle.as_str(),
            lines: lines.as_slice(),
            scroll: self.preview_scroll,
            search_query: self.preview_search.query_str(),
            search_active: self.preview_search.active,
            match_count,
            current_match,
            pane_index,
            zoomed,
            synchronized,
            piping,
            show_actions: self.preview_target.is_some(),
            pip_active,
            hovered_action,
            scrollbar_hovered: self.desktop.hover == super::desktop::HoverTarget::PreviewScrollbar,
            scrollbar_dragging: self.desktop.is_dragging_scrollbar(),
            search_match_lines: &self.preview_search.match_lines,
            error_lines: &self.desktop.preview_error_lines,
            selection_range: self.desktop.preview_selection,
            dock_highlight,
        })
    }

    pub(super) fn preview_status_flags(&self) -> (bool, bool, bool) {
        match &self.preview_target {
            Some(Target::Pane(pid)) => {
                let pane = self.home_panes.iter().find(|p| &p.id == pid);
                let win = pane.and_then(|p| self.home_windows.iter().find(|w| w.id == p.window_id));
                let zoomed = pane.is_some_and(|p| p.zoomed) || win.is_some_and(|w| w.zoomed);
                let synced =
                    pane.is_some_and(|p| p.synchronized) || win.is_some_and(|w| w.synchronized);
                let piping = self.piping_panes.contains(pid);
                (zoomed, synced, piping)
            }
            Some(Target::Window(_, wid)) => {
                let win = self.home_windows.iter().find(|w| &w.id == wid);
                let piping = self
                    .home_panes
                    .iter()
                    .any(|p| &p.window_id == wid && self.piping_panes.contains(&p.id));
                (
                    win.is_some_and(|w| w.zoomed),
                    win.is_some_and(|w| w.synchronized),
                    piping,
                )
            }
            Some(Target::Session(sid)) => {
                let win = self
                    .home_windows
                    .iter()
                    .find(|w| &w.session_id == sid && w.active)
                    .or_else(|| self.home_windows.iter().find(|w| &w.session_id == sid));
                let piping = win.is_some_and(|w| {
                    self.home_panes
                        .iter()
                        .any(|p| p.window_id == w.id && self.piping_panes.contains(&p.id))
                });
                (
                    win.is_some_and(|w| w.zoomed),
                    win.is_some_and(|w| w.synchronized),
                    piping,
                )
            }
            None => (false, false, false),
        }
    }

    pub(super) fn handle_preview_action(&mut self, action: PreviewAction) {
        match action {
            PreviewAction::PrevPane => {
                self.cycle_home_preview_pane(false);
            }
            PreviewAction::NextPane => {
                self.cycle_home_preview_pane(true);
            }
            PreviewAction::Search => {
                if self.preview_search.active {
                    self.preview_search.reset();
                } else {
                    self.open_preview_search();
                }
            }
            PreviewAction::Copy => self.copy_current_preview_to_clipboard(),
            PreviewAction::ExportLog => self.export_current_preview_log(),
            PreviewAction::TogglePipe => self.toggle_current_preview_pipe(),
            other => {
                let _ = self.handle_desktop_preview_action(other);
            }
        }
    }

    pub(super) fn handle_home_quick_action(&mut self, action: HomeQuickAction) {
        self.status.clear();
        match action {
            HomeQuickAction::Detach => self.detach(),
            HomeQuickAction::Panes => self.open_picker(super::PickerPurpose::Panes),
            HomeQuickAction::Layout => self.open_layout_studio(None),
            HomeQuickAction::Clients => self.open_clients_view(),
            HomeQuickAction::Buffers => self.open_buffers_view(),
            HomeQuickAction::Snapshots => self.open_snapshots_view(),
        }
    }

    pub(super) fn handle_controller_action(&mut self, action: ControllerAction) {
        match action {
            ControllerAction::SelectLayoutPreset(idx) => {
                if let Some(View::LayoutStudio(mut state)) = self.views.pop() {
                    self.apply_layout_preset(&mut state, idx);
                    self.views.push(View::LayoutStudio(state));
                }
            }
            ControllerAction::ResizeLeft => {
                if let Some(View::LayoutStudio(mut state)) = self.views.pop() {
                    self.resize_layout_pane(&mut state, ResizeDirection::Left);
                    self.views.push(View::LayoutStudio(state));
                }
            }
            ControllerAction::ResizeRight => {
                if let Some(View::LayoutStudio(mut state)) = self.views.pop() {
                    self.resize_layout_pane(&mut state, ResizeDirection::Right);
                    self.views.push(View::LayoutStudio(state));
                }
            }
            ControllerAction::ResizeUp => {
                if let Some(View::LayoutStudio(mut state)) = self.views.pop() {
                    self.resize_layout_pane(&mut state, ResizeDirection::Up);
                    self.views.push(View::LayoutStudio(state));
                }
            }
            ControllerAction::ResizeDown => {
                if let Some(View::LayoutStudio(mut state)) = self.views.pop() {
                    self.resize_layout_pane(&mut state, ResizeDirection::Down);
                    self.views.push(View::LayoutStudio(state));
                }
            }
            ControllerAction::ToggleZoom => {
                if let Some(View::LayoutStudio(state)) = self.views.pop() {
                    self.layout_studio_key(
                        state,
                        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
                    );
                }
            }
            ControllerAction::ToggleSync => {
                if let Some(View::LayoutStudio(state)) = self.views.pop() {
                    self.layout_studio_key(
                        state,
                        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
                    );
                }
            }
            ControllerAction::DetachOtherClients => {
                if let Some(View::Clients(state)) = self.views.pop() {
                    self.clients_key(state, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
                }
            }
            ControllerAction::DetachSelectedClient => {
                if let Some(View::Clients(state)) = self.views.pop() {
                    self.clients_key(state, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
                }
            }
            ControllerAction::KillServer => {
                self.open_kill_server_confirm();
            }
            ControllerAction::PasteBuffer => {
                if let Some(View::Buffers(state)) = self.views.pop() {
                    self.buffers_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            ControllerAction::CopyBufferToSystem => {
                if let Some(View::Buffers(state)) = self.views.pop() {
                    self.buffers_key(state, KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
                }
            }
            ControllerAction::NewBuffer => {
                if let Some(View::Buffers(state)) = self.views.pop() {
                    self.buffers_key(state, KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
                }
            }
            ControllerAction::DeleteBuffer => {
                if let Some(View::Buffers(state)) = self.views.pop() {
                    self.buffers_key(state, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
                }
            }
            ControllerAction::SaveSnapshot => {
                if let Some(View::Snapshots(state)) = self.views.pop() {
                    self.snapshots_key(
                        state,
                        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                    );
                }
            }
            ControllerAction::RestoreSnapshot => {
                if let Some(View::Snapshots(state)) = self.views.pop() {
                    self.snapshots_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            ControllerAction::DeleteSnapshot => {
                if let Some(View::Snapshots(state)) = self.views.pop() {
                    self.snapshots_key(
                        state,
                        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                    );
                }
            }
        }
    }

    /// Update one opt-in `~/.tmux.conf` comfort setting, sync the managed comfort block,
    /// and hot-apply the transition to the live tmux server.
    pub(super) fn update_tmux_comfort_setting(
        &mut self,
        update: impl FnOnce(&mut TmuxComfortOptions),
    ) {
        let prev = self.config.tmux_comfort_options();
        let mut next = prev;
        update(&mut next);
        if next == prev {
            return;
        }
        let Some(conf_path) = self.tmux_conf_path.clone() else {
            self.config.set_tmux_comfort_options(next);
            let msg = self
                .theme
                .tr(
                    "No ~/.tmux.conf path available (set HOME or JUSTMUX_TMUX_CONF)",
                    "未找到 ~/.tmux.conf 路径（请设置 HOME 或 JUSTMUX_TMUX_CONF）",
                )
                .to_owned();
            self.config_note = Some(msg.clone());
            self.status = msg;
            return;
        };
        let short = Self::shorten_home_path(&conf_path);
        if let Err(err) = tmux_conf::sync_comfort_block(&conf_path, &next) {
            let msg = if self.theme.language.is_zh() {
                format!("无法更新 {short}：{err}")
            } else {
                format!("Could not update {short}: {err}")
            };
            self.config_note = Some(msg.clone());
            self.status = msg;
            return;
        }
        self.config.set_tmux_comfort_options(next);
        let cmds = next.live_tmux_transition_commands(&prev);
        let msg = match tmux_conf::apply_comfort_commands_live(self.tmux, &cmds) {
            Ok(n) if n > 0 => {
                if self.theme.language.is_zh() {
                    format!("✦ 已同步 {short} 并热生效 {n} 项 tmux 设置")
                } else {
                    format!("✦ Synced {short} & applied {n} live tmux option(s)")
                }
            }
            Ok(_) => {
                if self.theme.language.is_zh() {
                    format!("✦ 已更新 {short}（下次启动 tmux 自动生效）")
                } else {
                    format!("✦ Updated {short} (active when tmux starts)")
                }
            }
            Err(err) => {
                if self.theme.language.is_zh() {
                    format!("✦ 已更新 {short}（热生效失败：{err}）")
                } else {
                    format!("✦ Updated {short} (live apply failed: {err})")
                }
            }
        };
        self.config_note = Some(msg.clone());
        self.status = msg;
    }

    pub(super) fn open_snapshots_view(&mut self) {
        let mut state = SnapshotsViewState {
            snapshots: Vec::new(),
            selected: 0,
            tree_preview: Vec::new(),
            preview_scroll: 0,
            status_note: None,
        };
        self.reload_snapshots_state(&mut state);
        self.views.push(View::Snapshots(state));
    }

    pub(super) fn reload_snapshots_state(&self, state: &mut SnapshotsViewState) {
        let Some(dir) = snapshot::resolve_snapshot_dir() else {
            state.snapshots.clear();
            state.tree_preview.clear();
            state.status_note = Some(
                self.theme
                    .tr(
                        "No snapshot directory available (set HOME or JUSTMUX_SNAPSHOT_DIR)",
                        "未找到快照目录（请设置 HOME 或 JUSTMUX_SNAPSHOT_DIR）",
                    )
                    .into(),
            );
            return;
        };
        match snapshot::list_snapshots(&dir) {
            Ok(list) => {
                state.snapshots = list;
                state.selected = state.selected.min(state.snapshots.len().saturating_sub(1));
                self.refresh_selected_snapshot_preview(state);
            }
            Err(err) => {
                state.snapshots.clear();
                state.tree_preview.clear();
                state.status_note = Some(err);
            }
        }
    }

    pub(super) fn refresh_selected_snapshot_preview(&self, state: &mut SnapshotsViewState) {
        state.preview_scroll = 0;
        let Some(summary) = state.snapshots.get(state.selected) else {
            state.tree_preview.clear();
            return;
        };
        match snapshot::load_snapshot_file(&summary.path) {
            Ok(snap) => state.tree_preview = snap.format_tree_preview(),
            Err(err) => state.tree_preview = vec![err],
        }
    }

    pub(super) fn save_snapshot_by_name(
        &mut self,
        name: &str,
        session_filter: Option<&SessionId>,
    ) -> Result<WorkspaceSnapshot, String> {
        let dir = snapshot::resolve_snapshot_dir().ok_or_else(|| {
            self.theme
                .tr(
                    "No snapshot directory available (set HOME or JUSTMUX_SNAPSHOT_DIR)",
                    "未找到快照目录（请设置 HOME 或 JUSTMUX_SNAPSHOT_DIR）",
                )
                .to_owned()
        })?;
        let snap = snapshot::capture_snapshot(self.tmux, name, session_filter)?;
        snapshot::save_snapshot(&dir, &snap)?;
        Ok(snap)
    }

    pub(super) fn restore_snapshot_by_name(&mut self, name: &str) {
        let Some(dir) = snapshot::resolve_snapshot_dir() else {
            self.status = self
                .theme
                .tr(
                    "No snapshot directory available (set HOME or JUSTMUX_SNAPSHOT_DIR)",
                    "未找到快照目录（请设置 HOME 或 JUSTMUX_SNAPSHOT_DIR）",
                )
                .into();
            return;
        };
        match snapshot::load_snapshot(&dir, name)
            .and_then(|snap| snapshot::restore_snapshot(self.tmux, &snap))
        {
            Ok(report) => {
                self.refresh_home_data();
                let warn = if report.warnings.is_empty() {
                    String::new()
                } else if self.theme.language.is_zh() {
                    format!("（{} 条提示）", report.warnings.len())
                } else {
                    format!(" ({} warning(s))", report.warnings.len())
                };
                self.status = if self.theme.language.is_zh() {
                    format!(
                        "✦ 已恢复快照 '{name}'：新建 {} 会话 / 复用 {} / {} 窗口 / {} 窗格{warn}",
                        report.sessions_created,
                        report.sessions_reused,
                        report.windows_created,
                        report.panes_created
                    )
                } else {
                    format!(
                        "✦ Restored '{name}': {} new / {} reused sessions, {} windows, {} panes{warn}",
                        report.sessions_created,
                        report.sessions_reused,
                        report.windows_created,
                        report.panes_created
                    )
                };
            }
            Err(err) => self.status = err,
        }
    }

    pub(super) fn snapshots_key(&mut self, mut state: SnapshotsViewState, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => {
                state.selected = super::wrap_step(state.selected, state.snapshots.len(), -1);
                self.refresh_selected_snapshot_preview(&mut state);
            }
            KeyCode::Down => {
                state.selected = super::wrap_step(state.selected, state.snapshots.len(), 1);
                self.refresh_selected_snapshot_preview(&mut state);
            }
            KeyCode::PageUp => {
                state.preview_scroll = state.preview_scroll.saturating_sub(6);
            }
            KeyCode::PageDown => {
                let max = state.tree_preview.len().saturating_sub(1);
                state.preview_scroll = (state.preview_scroll + 6).min(max);
            }
            KeyCode::Char('n' | 'N') if no_ctrl_alt => {
                self.views.push(View::Snapshots(state));
                self.views.push(View::Form(Form {
                    action: FormAction::SaveSnapshotName {
                        session_filter: None,
                    },
                    input: TextInput::default(),
                }));
                return;
            }
            KeyCode::Enter => {
                if let Some(summary) = state.snapshots.get(state.selected).cloned() {
                    self.restore_snapshot_by_name(&summary.name);
                    state.status_note = Some(self.status.clone());
                }
            }
            KeyCode::Char('x' | 'X') | KeyCode::Delete if no_ctrl_alt => {
                if let Some(summary) = state.snapshots.get(state.selected).cloned()
                    && let Some(dir) = snapshot::resolve_snapshot_dir()
                {
                    match snapshot::delete_snapshot(&dir, &summary.name) {
                        Ok(_) => {
                            let msg = if self.theme.language.is_zh() {
                                format!("✦ 已删除快照 '{}'", summary.name)
                            } else {
                                format!("✦ Deleted snapshot '{}'", summary.name)
                            };
                            self.reload_snapshots_state(&mut state);
                            state.status_note = Some(msg.clone());
                            self.status = msg;
                        }
                        Err(err) => state.status_note = Some(err),
                    }
                }
            }
            _ => {}
        }
        self.views.push(View::Snapshots(state));
    }

    pub(super) fn open_kill_server_confirm(&mut self) {
        let zh = self.theme.language.is_zh();
        let message = if zh {
            "确定要关闭整个 tmux 服务吗？所有会话、窗口和正在运行的后台进程都将立即终止！输入 yes 后按 Enter 确认，留空则取消。".to_owned()
        } else {
            "Permanently stop the ENTIRE tmux server? Every session, window, and background process will terminate immediately! Type yes, then Enter to confirm. Empty input cancels.".to_owned()
        };
        self.views.push(View::Confirm(Confirm {
            target: ConfirmTarget::KillServer,
            input: TextInput::default(),
            message,
            scroll: 0,
        }));
    }

    pub(super) fn open_popup_shell(&mut self) {
        self.context = self.tmux.context();
        if !self.context.is_inside() {
            self.status = self
                .theme
                .tr(
                    "/popup requires running inside a tmux client (tmux 3.2+).",
                    "/popup 悬浮终端需要在 tmux 客户端内部运行（需 tmux 3.2+）。",
                )
                .into();
            return;
        }
        if let Ok(version) = self.tmux.version()
            && !version.supports_popup()
        {
            self.status = if self.theme.language.is_zh() {
                format!("悬浮终端需要 tmux 3.2+（当前版本：{version}）。")
            } else {
                format!("Floating scratchpad popup requires tmux 3.2+ (found {version}).")
            };
            return;
        }
        let cwd: Option<PathBuf> = self
            .focused_or_active_pane()
            .map(|p| PathBuf::from(p.current_path))
            .filter(|p| p.is_dir())
            .or_else(|| env::current_dir().ok());
        match self.tmux.display_popup_shell(cwd.as_deref()) {
            Ok(()) => {
                self.status = self
                    .theme
                    .tr("✦ Closed scratchpad popup", "✦ 悬浮临时终端已关闭")
                    .into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn open_target_pick(&mut self, purpose: TargetPickPurpose) {
        self.refresh_home_data();
        let resources: Vec<Resource> = match &purpose {
            TargetPickPurpose::MoveWindow(win) | TargetPickPurpose::LinkWindow(win) => self
                .home_sessions
                .iter()
                .filter(|s| s.id != win.session_id)
                .cloned()
                .map(Resource::Session)
                .collect(),
            TargetPickPurpose::SwapPane(pane) => self
                .home_panes
                .iter()
                .filter(|p| p.id != pane.id)
                .cloned()
                .map(Resource::Pane)
                .collect(),
            TargetPickPurpose::JoinPane(pane) => self
                .home_windows
                .iter()
                .filter(|w| w.id != pane.window_id)
                .cloned()
                .map(Resource::Window)
                .collect(),
            TargetPickPurpose::PasteBuffer { .. } => self
                .home_panes
                .iter()
                .cloned()
                .map(Resource::Pane)
                .collect(),
        };
        if resources.is_empty() {
            self.status = self
                .theme
                .tr(
                    "No other available target found in tmux.",
                    "tmux 中没有其他可选目标。",
                )
                .into();
            return;
        }
        let state = TargetPickState {
            purpose,
            query: TextInput::default(),
            resources,
            selected: 0,
        };
        self.update_target_pick_preview(&state);
        self.views.push(View::TargetPick(state));
    }

    pub(super) fn update_target_pick_preview(&mut self, state: &TargetPickState) {
        if !self.config.live_preview {
            self.clear_preview();
            return;
        }
        let rows = state.filtered_indices(&mut self.matcher);
        let Some((res_idx, _)) = rows.get(state.selected) else {
            self.clear_preview();
            return;
        };
        let Some(res) = state.resources.get(*res_idx) else {
            self.clear_preview();
            return;
        };
        let target = res.target();
        let title = res.label();
        let subtitle = match res {
            Resource::Session(s) => format!("{}w", s.windows),
            Resource::Window(w) => w.id.to_string(),
            Resource::Pane(p) => p.id.to_string(),
        };
        self.capture_and_store_preview(target, title, subtitle);
    }

    pub(super) fn target_pick_key(&mut self, mut state: TargetPickState, key: KeyEvent) {
        if self.handle_preview_paging_key(key, state.query.value().is_empty()) {
            self.views.push(View::TargetPick(state));
            return;
        }
        match key.code {
            KeyCode::Up => {
                let len = state.filtered_indices(&mut self.matcher).len();
                state.selected = super::wrap_step(state.selected, len, -1);
                self.update_target_pick_preview(&state);
            }
            KeyCode::Down => {
                let len = state.filtered_indices(&mut self.matcher).len();
                state.selected = super::wrap_step(state.selected, len, 1);
                self.update_target_pick_preview(&state);
            }
            KeyCode::Enter => {
                let rows = state.filtered_indices(&mut self.matcher);
                if let Some((idx, _)) = rows.get(state.selected)
                    && let Some(chosen) = state.resources.get(*idx).cloned()
                {
                    self.execute_target_pick_choice(state.purpose, chosen);
                    return;
                }
            }
            _ => {
                if state.query.handle_key(key) {
                    state.selected = 0;
                    self.update_target_pick_preview(&state);
                }
            }
        }
        self.views.push(View::TargetPick(state));
    }

    pub(super) fn execute_target_pick_choice(
        &mut self,
        purpose: TargetPickPurpose,
        chosen: Resource,
    ) {
        match (purpose, chosen) {
            (TargetPickPurpose::MoveWindow(win), Resource::Session(sess)) => {
                match self.tmux.move_window(&win.id, &sess.id) {
                    Ok(()) => {
                        self.refresh_parent();
                        self.status = if self.theme.language.is_zh() {
                            format!("✦ 已将窗口 '{}' 移动到会话 '{}'", win.name, sess.name)
                        } else {
                            format!("✦ Moved window '{}' to session '{}'", win.name, sess.name)
                        };
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            (TargetPickPurpose::LinkWindow(win), Resource::Session(sess)) => {
                match self.tmux.link_window(&win.id, &sess.id) {
                    Ok(()) => {
                        self.refresh_parent();
                        self.status = if self.theme.language.is_zh() {
                            format!("✦ 已将窗口 '{}' 共享链接到会话 '{}'", win.name, sess.name)
                        } else {
                            format!(
                                "✦ Linked window '{}' into session '{}'",
                                win.name, sess.name
                            )
                        };
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            (TargetPickPurpose::SwapPane(src), Resource::Pane(dst)) => {
                match self.tmux.swap_pane(&src.id, &dst.id) {
                    Ok(()) => {
                        self.refresh_parent();
                        self.status = if self.theme.language.is_zh() {
                            format!("✦ 已互换窗格 {} 与 {}", src.id, dst.id)
                        } else {
                            format!("✦ Swapped panes {} and {}", src.id, dst.id)
                        };
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            (TargetPickPurpose::JoinPane(src), Resource::Window(dst_win)) => {
                let target_pane = self.representative_pane_for_window(&dst_win.id);
                if let Some(dst_pane) = target_pane {
                    match self
                        .tmux
                        .join_pane(&src.id, &dst_pane.id, SplitDirection::SideBySide)
                    {
                        Ok(()) => {
                            self.refresh_parent();
                            self.status = if self.theme.language.is_zh() {
                                format!("✦ 已将窗格 {} 合并到窗口 '{}'", src.id, dst_win.name)
                            } else {
                                format!("✦ Joined pane {} into window '{}'", src.id, dst_win.name)
                            };
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            (TargetPickPurpose::PasteBuffer { name }, Resource::Pane(dst)) => {
                match self.tmux.paste_buffer(&name, &dst.id) {
                    Ok(()) => {
                        self.refresh_parent();
                        self.status = if self.theme.language.is_zh() {
                            format!("✦ 已将缓冲区 '{name}' 粘贴到窗格 {}", dst.id)
                        } else {
                            format!("✦ Pasted buffer '{name}' into pane {}", dst.id)
                        };
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            _ => {}
        }
    }

    pub(super) fn open_layout_studio(&mut self, resource: Option<Resource>) {
        self.refresh_home_data();
        let pane = match resource {
            Some(Resource::Pane(p)) => Some(p),
            Some(Resource::Window(w)) => self.representative_pane_for_window(&w.id),
            Some(Resource::Session(s)) => self
                .home_panes
                .iter()
                .find(|p| p.session_id == s.id && p.active)
                .or_else(|| self.home_panes.iter().find(|p| p.session_id == s.id))
                .cloned(),
            None => self.focused_or_active_pane(),
        };
        let Some(pane) = pane else {
            self.status = self
                .theme
                .tr(
                    "No active tmux pane found for layout studio.",
                    "未找到可用的 tmux 窗格以打开布局工作台。",
                )
                .into();
            return;
        };
        let win = self
            .home_windows
            .iter()
            .find(|w| w.id == pane.window_id)
            .cloned();
        let (pane_count, zoomed, synchronized, target_label) = match win {
            Some(w) => (
                w.panes,
                w.zoomed || pane.zoomed,
                w.synchronized || pane.synchronized,
                format!("{}:{} {} ({})", w.session_name, w.index, w.name, pane.id),
            ),
            None => (
                1,
                pane.zoomed,
                pane.synchronized,
                format!("{}:{}.{}", pane.session_name, pane.window_index, pane.index),
            ),
        };
        self.views.push(View::LayoutStudio(LayoutStudioState {
            window_id: pane.window_id,
            pane_id: pane.id,
            target_label,
            selected_preset: 0,
            pane_size: (pane.width, pane.height),
            pane_count,
            resize_step: 5,
            zoomed,
            synchronized,
            status_note: None,
        }));
    }

    pub(super) fn refresh_layout_studio_state(&mut self, state: &mut LayoutStudioState) {
        self.refresh_home_data();
        if let Some(p) = self.home_panes.iter().find(|p| p.id == state.pane_id) {
            state.pane_size = (p.width, p.height);
            state.zoomed = p.zoomed;
            state.synchronized = p.synchronized;
        }
        if let Some(w) = self.home_windows.iter().find(|w| w.id == state.window_id) {
            state.pane_count = w.panes;
            state.zoomed = w.zoomed;
            state.synchronized = w.synchronized;
        }
    }

    pub(super) fn apply_layout_preset(&mut self, state: &mut LayoutStudioState, preset_idx: usize) {
        let preset = LayoutStudioState::preset_at(preset_idx);
        state.selected_preset = preset_idx.min(LayoutPreset::ALL.len().saturating_sub(1));
        match self.tmux.select_layout(&state.window_id, preset) {
            Ok(()) => {
                self.refresh_layout_studio_state(state);
                let label = preset.label(self.theme.language.is_zh());
                let msg = if self.theme.language.is_zh() {
                    format!("✦ 已应用布局：{label}")
                } else {
                    format!("✦ Applied layout: {label}")
                };
                state.status_note = Some(msg.clone());
                self.status = msg;
            }
            Err(error) => {
                let msg = error.to_string();
                state.status_note = Some(msg.clone());
                self.status = msg;
            }
        }
    }

    pub(super) fn resize_layout_pane(
        &mut self,
        state: &mut LayoutStudioState,
        dir: ResizeDirection,
    ) {
        match self
            .tmux
            .resize_pane(&state.pane_id, dir, state.resize_step)
        {
            Ok(()) => {
                self.refresh_layout_studio_state(state);
                let (w, h) = state.pane_size;
                let msg = if self.theme.language.is_zh() {
                    format!("✦ 窗格尺寸已调整为 {w}×{h}")
                } else {
                    format!("✦ Resized pane to {w}x{h}")
                };
                state.status_note = Some(msg.clone());
                self.status = msg;
            }
            Err(error) => {
                let msg = error.to_string();
                state.status_note = Some(msg.clone());
                self.status = msg;
            }
        }
    }

    pub(super) fn layout_studio_key(&mut self, mut state: LayoutStudioState, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(d @ '1'..='5') if no_ctrl_alt => {
                let idx = (d as usize) - ('1' as usize);
                self.apply_layout_preset(&mut state, idx);
            }
            KeyCode::Tab if no_ctrl_alt => {
                state.selected_preset = (state.selected_preset + 1) % LayoutPreset::ALL.len();
            }
            KeyCode::BackTab if no_ctrl_alt => {
                let len = LayoutPreset::ALL.len();
                state.selected_preset = (state.selected_preset + len - 1) % len;
            }
            KeyCode::Enter => {
                let idx = state.selected_preset;
                self.apply_layout_preset(&mut state, idx);
            }
            KeyCode::Left if no_ctrl_alt => {
                self.resize_layout_pane(&mut state, ResizeDirection::Left);
            }
            KeyCode::Right if no_ctrl_alt => {
                self.resize_layout_pane(&mut state, ResizeDirection::Right);
            }
            KeyCode::Up if no_ctrl_alt => {
                self.resize_layout_pane(&mut state, ResizeDirection::Up);
            }
            KeyCode::Down if no_ctrl_alt => {
                self.resize_layout_pane(&mut state, ResizeDirection::Down);
            }
            KeyCode::Char('+' | '=') if no_ctrl_alt => {
                state.resize_step = (state.resize_step + 1).min(20);
            }
            KeyCode::Char('-') if no_ctrl_alt => {
                state.resize_step = state.resize_step.saturating_sub(1).max(1);
            }
            KeyCode::Char('z' | 'Z') if no_ctrl_alt => {
                match self.tmux.toggle_zoom_pane(&state.pane_id) {
                    Ok(()) => {
                        self.refresh_layout_studio_state(&mut state);
                        let msg = if state.zoomed {
                            self.theme.tr("✦ Zoomed pane (⛶)", "✦ 已全屏缩放窗格 (⛶)")
                        } else {
                            self.theme.tr("✦ Unzoomed pane", "✦ 已取消全屏缩放")
                        }
                        .to_owned();
                        state.status_note = Some(msg.clone());
                        self.status = msg;
                    }
                    Err(error) => {
                        state.status_note = Some(error.to_string());
                    }
                }
            }
            KeyCode::Char('s' | 'S') if no_ctrl_alt => {
                let next = !state.synchronized;
                match self.tmux.set_synchronize_panes(&state.window_id, next) {
                    Ok(()) => {
                        self.refresh_layout_studio_state(&mut state);
                        let msg = if state.synchronized {
                            self.theme
                                .tr("⚡ Synchronize-panes ON", "⚡ 同步输入已开启")
                        } else {
                            self.theme.tr("✦ Synchronize-panes OFF", "✦ 同步输入已关闭")
                        }
                        .to_owned();
                        state.status_note = Some(msg.clone());
                        self.status = msg;
                    }
                    Err(error) => {
                        state.status_note = Some(error.to_string());
                    }
                }
            }
            _ => {}
        }
        self.views.push(View::LayoutStudio(state));
    }

    pub(super) fn open_clients_view(&mut self) {
        self.context = self.tmux.context();
        let (clients, note) = match self.tmux.list_clients() {
            Ok(list) => (list, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        self.views.push(View::Clients(ClientsViewState {
            clients,
            selected: 0,
            status_note: note,
        }));
    }

    pub(super) fn reload_clients_state(&mut self, state: &mut ClientsViewState) {
        self.context = self.tmux.context();
        match self.tmux.list_clients() {
            Ok(list) => {
                state.clients = list;
                state.selected = state.selected.min(state.clients.len().saturating_sub(1));
            }
            Err(error) => state.status_note = Some(error.to_string()),
        }
        self.refresh_home_data();
    }

    pub(super) fn clients_key(&mut self, mut state: ClientsViewState, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => {
                state.selected = super::wrap_step(state.selected, state.clients.len(), -1);
            }
            KeyCode::Down => {
                state.selected = super::wrap_step(state.selected, state.clients.len(), 1);
            }
            KeyCode::Char('d' | 'D' | 'o' | 'O') if no_ctrl_alt => {
                let keep = match &self.context {
                    Context::Inside(inside) => inside.client_name.as_deref(),
                    _ => state.clients.get(state.selected).map(|c| c.name.as_str()),
                };
                match self.tmux.detach_other_clients(None, keep) {
                    Ok(count) => {
                        let msg = if self.theme.language.is_zh() {
                            format!("✦ 已踢出 {count} 个其他客户端")
                        } else {
                            format!("✦ Detached {count} other client(s)")
                        };
                        self.reload_clients_state(&mut state);
                        state.status_note = Some(msg.clone());
                        self.status = msg;
                    }
                    Err(error) => state.status_note = Some(error.to_string()),
                }
            }
            KeyCode::Char('x' | 'X') | KeyCode::Delete | KeyCode::Enter if no_ctrl_alt => {
                if let Some(client) = state.clients.get(state.selected).cloned() {
                    match self.tmux.detach_client(&client.name) {
                        Ok(()) => {
                            let msg = if self.theme.language.is_zh() {
                                format!("✦ 已踢出客户端 {}", client.name)
                            } else {
                                format!("✦ Detached client {}", client.name)
                            };
                            self.reload_clients_state(&mut state);
                            state.status_note = Some(msg.clone());
                            self.status = msg;
                        }
                        Err(error) => state.status_note = Some(error.to_string()),
                    }
                }
            }
            KeyCode::Char('r' | 'R') if no_ctrl_alt => {
                self.reload_clients_state(&mut state);
                state.status_note = Some(
                    self.theme
                        .tr("Refreshed clients", "已刷新客户端列表")
                        .into(),
                );
            }
            KeyCode::Char('K') if no_ctrl_alt => {
                self.views.push(View::Clients(state));
                self.open_kill_server_confirm();
                return;
            }
            _ => {}
        }
        self.views.push(View::Clients(state));
    }

    pub(super) fn open_buffers_view(&mut self) {
        let (buffers, note) = match self.tmux.list_buffers() {
            Ok(list) => (list, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        let mut state = BuffersViewState {
            buffers,
            selected: 0,
            preview_lines: Vec::new(),
            preview_scroll: 0,
            status_note: note,
        };
        self.refresh_selected_buffer_preview(&mut state);
        self.views.push(View::Buffers(state));
    }

    pub(super) fn refresh_selected_buffer_preview(&self, state: &mut BuffersViewState) {
        state.preview_scroll = 0;
        let Some(buf) = state.buffers.get(state.selected) else {
            state.preview_lines.clear();
            return;
        };
        match self.tmux.show_buffer(&buf.name) {
            Ok(content) => {
                state.preview_lines = content.lines().map(ToOwned::to_owned).collect();
            }
            Err(_) => {
                state.preview_lines = vec![buf.sample.clone()];
            }
        }
    }

    pub(super) fn reload_buffers_state(&mut self, state: &mut BuffersViewState) {
        match self.tmux.list_buffers() {
            Ok(list) => {
                state.buffers = list;
                state.selected = state.selected.min(state.buffers.len().saturating_sub(1));
                self.refresh_selected_buffer_preview(state);
            }
            Err(error) => state.status_note = Some(error.to_string()),
        }
    }

    pub(super) fn buffers_key(&mut self, mut state: BuffersViewState, key: KeyEvent) {
        let no_ctrl_alt = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Up => {
                state.selected = super::wrap_step(state.selected, state.buffers.len(), -1);
                self.refresh_selected_buffer_preview(&mut state);
            }
            KeyCode::Down => {
                state.selected = super::wrap_step(state.selected, state.buffers.len(), 1);
                self.refresh_selected_buffer_preview(&mut state);
            }
            KeyCode::PageUp => {
                state.preview_scroll = state.preview_scroll.saturating_sub(6);
            }
            KeyCode::PageDown => {
                let max = state.preview_lines.len().saturating_sub(1);
                state.preview_scroll = (state.preview_scroll + 6).min(max);
            }
            KeyCode::Enter => {
                if let Some(buf) = state.buffers.get(state.selected).cloned() {
                    self.views.push(View::Buffers(state));
                    self.open_target_pick(TargetPickPurpose::PasteBuffer { name: buf.name });
                    return;
                }
            }
            KeyCode::Char('y' | 'Y') if no_ctrl_alt => {
                if let Some(buf) = state.buffers.get(state.selected).cloned() {
                    match self.tmux.show_buffer(&buf.name) {
                        Ok(content) => match clipboard::copy_to_clipboard(&content, None) {
                            Ok(outcome) => {
                                let msg =
                                    format!("✦ {}", outcome.label(self.theme.language.is_zh()));
                                state.status_note = Some(msg.clone());
                                self.status = msg;
                            }
                            Err(err) => {
                                state.status_note = Some(err.clone());
                                self.status = err;
                            }
                        },
                        Err(err) => {
                            let msg = err.to_string();
                            state.status_note = Some(msg.clone());
                            self.status = msg;
                        }
                    }
                }
            }
            KeyCode::Char('n' | 'N') if no_ctrl_alt => {
                self.views.push(View::Buffers(state));
                self.views.push(View::Form(Form {
                    action: FormAction::NewBufferContent,
                    input: TextInput::default(),
                }));
                return;
            }
            KeyCode::Char('x' | 'X') | KeyCode::Delete if no_ctrl_alt => {
                if let Some(buf) = state.buffers.get(state.selected).cloned() {
                    match self.tmux.delete_buffer(&buf.name) {
                        Ok(()) => {
                            let msg = if self.theme.language.is_zh() {
                                format!("✦ 已删除缓冲区 {}", buf.name)
                            } else {
                                format!("✦ Deleted buffer {}", buf.name)
                            };
                            self.reload_buffers_state(&mut state);
                            state.status_note = Some(msg.clone());
                            self.status = msg;
                        }
                        Err(error) => state.status_note = Some(error.to_string()),
                    }
                }
            }
            _ => {}
        }
        self.views.push(View::Buffers(state));
    }

    pub(super) fn dual_form_key(&mut self, mut form: DualForm, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                form.toggle_field();
                self.views.push(View::DualForm(form));
            }
            KeyCode::Enter => {
                let name_raw = form.first_input.value().trim().to_owned();
                let dir_raw = form.second_input.value().trim().to_owned();
                match &form.action {
                    DualFormAction::NewWindow(session_id) => {
                        let name = (!name_raw.is_empty()).then_some(name_raw.as_str());
                        let cwd_buf = (!dir_raw.is_empty()).then(|| PathBuf::from(&dir_raw));
                        match self.tmux.new_window(session_id, name, cwd_buf.as_deref()) {
                            Ok(window) => {
                                self.views = super::ViewStack::default();
                                self.matrix_action = ui::MatrixAction::Window;
                                self.composer.set_value("");
                                self.refresh_home_data();
                                self.status = self.theme.tr("Window created", "窗口已创建").into();
                                self.views.push(View::Created(Resource::Window(window)));
                            }
                            Err(error) => {
                                self.status = error.to_string();
                                self.views.push(View::DualForm(form));
                            }
                        }
                    }
                }
            }
            _ => {
                form.active_input_mut().handle_key(key);
                self.views.push(View::DualForm(form));
            }
        }
    }

    pub(super) fn reload_tmux_conf_action(&mut self) {
        let Some(conf_path) = self.tmux_conf_path.clone() else {
            let msg = self
                .theme
                .tr(
                    "No ~/.tmux.conf path available (set HOME or JUSTMUX_TMUX_CONF)",
                    "未找到 ~/.tmux.conf 路径（请设置 HOME 或 JUSTMUX_TMUX_CONF）",
                )
                .to_owned();
            self.config_note = Some(msg.clone());
            self.status = msg;
            return;
        };
        let short = Self::shorten_home_path(&conf_path);
        if !conf_path.exists() {
            let msg = if self.theme.language.is_zh() {
                format!("{short} 不存在，暂无可重载配置")
            } else {
                format!("{short} does not exist yet")
            };
            self.config_note = Some(msg.clone());
            self.status = msg;
            return;
        }
        match self.tmux.source_file(&conf_path) {
            Ok(()) => {
                let msg = if self.theme.language.is_zh() {
                    format!("✦ 已热重载 {short}")
                } else {
                    format!("✦ Reloaded {short} into live tmux")
                };
                self.config_note = Some(msg.clone());
                self.status = msg;
            }
            Err(error) => {
                let msg = if self.theme.language.is_zh() {
                    format!("重载 {short} 失败：{error}")
                } else {
                    format!("Failed to reload {short}: {error}")
                };
                self.config_note = Some(msg.clone());
                self.status = msg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_and_preview_search_find_matches_and_step_cleanly() {
        assert_eq!(
            strip_ansi_for_search("\x1b[32mOK\x1b[0m build \x1b]0;title\x07done"),
            "OK build done"
        );
        let lines = vec![
            "\x1b[31merror[E0001]\x1b[0m: first".to_owned(),
            "normal output line".to_owned(),
            "another ERROR here".to_owned(),
            "tail line".to_owned(),
        ];
        let mut search = PreviewSearchState {
            active: true,
            ..Default::default()
        };
        search.query.set_value("error");
        search.recompute(&lines);
        assert_eq!(search.match_lines, vec![0, 2]);
        assert_eq!(search.current_match, 0);
        assert_eq!(search.step_match(true), Some(2));
        assert_eq!(search.step_match(true), Some(0));
        assert_eq!(search.step_match(false), Some(2));

        let scroll = PreviewSearchState::scroll_offset_for_line(100, 10, 20);
        assert!(scroll > 0 && scroll <= 90);
        assert_eq!(PreviewSearchState::scroll_offset_for_line(5, 10, 2), 0);
    }

    #[test]
    fn clients_size_mismatch_detects_dotted_border_condition() {
        let make = |name: &str, sess: &str, w: u32, h: u32| ClientInfo {
            name: name.into(),
            session_name: sess.into(),
            session_id: SessionId::new("$1"),
            width: w,
            height: h,
            tty: name.into(),
            readonly: false,
            control_mode: false,
            activity: 0,
        };
        let single = ClientsViewState {
            clients: vec![make("/dev/ttys001", "dev", 120, 40)],
            selected: 0,
            status_note: None,
        };
        assert!(!single.has_size_mismatch());

        let same_size = ClientsViewState {
            clients: vec![
                make("/dev/ttys001", "dev", 120, 40),
                make("/dev/ttys002", "dev", 120, 40),
            ],
            selected: 0,
            status_note: None,
        };
        assert!(!same_size.has_size_mismatch());

        let mismatched = ClientsViewState {
            clients: vec![
                make("/dev/ttys001", "dev", 120, 40),
                make("/dev/ttys002", "dev", 80, 24),
            ],
            selected: 0,
            status_note: None,
        };
        assert!(mismatched.has_size_mismatch());
    }
}
