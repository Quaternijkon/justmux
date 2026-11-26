//! Desktop-grade GUI interaction engine: Live PIP type-through, Mission Control
//! (`/expose`), top IDE menubar, right-click context menus, inline rename (`F2`),
//! hover tooltips, VS Code drag-and-drop & sash resizing, interactive scrollbar,
//! drag-to-select copy, toast notifications, and smart dirty-check live streaming.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use super::{
    App, MatrixAction, MatrixHitTarget, MatrixSelection, PREVIEW_CAPTURE_LINES, Resource,
    ResourceAction, SplitDirection, SplitSlotFill, SplitToolbarAction, Target, TargetScope, View,
    ViewStack, theme_toast, wrap_step,
};
use crate::{
    clipboard::{self, strip_ansi_escapes as strip_ansi},
    commands::CommandKind,
    terminal::ScreenMode,
    tmux::{PaneId, ResizeDirection, SessionId, Window, WindowId},
    ui::{
        ActiveDesktopTab, ComposerMode, ContextMenuItemView, ContextMenuView, DesktopMenubarState,
        DesktopOverlayLayout, DesktopOverlaySpec, DockZone, DragGhostView, ExposeTileHit,
        ExposeTileView, ExposeView, HomeQuickAction, HoverTooltipView, InlineRenameView,
        MenubarItem, PreviewAction, PreviewCardLayout, SashId, TextInput, TileControl, ToastKind,
        ToastView, WindowStatusFlags,
    },
};

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(350);
const DRAG_THRESHOLD_CELLS: u16 = 2;
const TOAST_DURATION: Duration = Duration::from_millis(3200);
const PIP_POLL_INTERVAL: Duration = Duration::from_millis(80);
const EXPOSE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(400);
const ANIM_POLL_INTERVAL: Duration = Duration::from_millis(85);
const EXPOSE_TILES_PER_PAGE: usize = 9;
const EXPOSE_CAPTURE_LINES: usize = 36;
const MIN_HOME_SPLIT_RATIO: u16 = 25;
const MAX_HOME_SPLIT_RATIO: u16 = 75;
const DEFAULT_HOME_SPLIT_RATIO: u16 = 46;

/// Compute a fast deterministic hash of captured preview lines for dirty-check redraw skipping.
pub(super) fn hash_lines(lines: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    lines.len().hash(&mut hasher);
    for line in lines {
        line.hash(&mut hasher);
    }
    hasher.finish()
}

/// Find line indices containing error-like keywords (case-insensitive) for VS Code scrollbar ticks.
pub(super) fn detect_error_lines(lines: &[String]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, raw)| {
            let plain = strip_ansi(raw).to_ascii_lowercase();
            (plain.contains("error")
                || plain.contains("fatal")
                || plain.contains("panic")
                || plain.contains("fail"))
            .then_some(idx)
        })
        .collect()
}

/// Compute VS Code edge dock zone (`Left`/`Right`/`Top`/`Bottom`/`Center`) for `(col, row)` inside `rect`.
pub(super) fn compute_dock_zone(rect: Rect, col: u16, row: u16) -> Option<DockZone> {
    if !rect.contains((col, row).into()) || rect.width < 4 || rect.height < 3 {
        return None;
    }
    let rel_x = col.saturating_sub(rect.x);
    let rel_y = row.saturating_sub(rect.y);
    let edge_x = (rect.width / 4).max(1);
    let edge_y = (rect.height / 4).max(1);
    let dist_left = rel_x;
    let dist_right = rect.width.saturating_sub(1).saturating_sub(rel_x);
    let dist_top = rel_y;
    let dist_bottom = rect.height.saturating_sub(1).saturating_sub(rel_y);

    let in_left = dist_left < edge_x;
    let in_right = dist_right < edge_x;
    let in_top = dist_top < edge_y;
    let in_bottom = dist_bottom < edge_y;

    if !in_left && !in_right && !in_top && !in_bottom {
        return Some(DockZone::Center);
    }

    // Pick the edge with smallest normalized penetration ratio.
    let mut best = (f32::MAX, DockZone::Center);
    if in_left {
        let score = f32::from(dist_left) / f32::from(edge_x);
        if score < best.0 {
            best = (score, DockZone::Left);
        }
    }
    if in_right {
        let score = f32::from(dist_right) / f32::from(edge_x);
        if score < best.0 {
            best = (score, DockZone::Right);
        }
    }
    if in_top {
        let score = f32::from(dist_top) / f32::from(edge_y);
        if score < best.0 {
            best = (score, DockZone::Top);
        }
    }
    if in_bottom {
        let score = f32::from(dist_bottom) / f32::from(edge_y);
        if score < best.0 {
            best = (score, DockZone::Bottom);
        }
    }
    Some(best.1)
}

/// Push a new velocity sample (`0..=8`) into a 12-slot sparkline ring buffer.
pub(super) fn push_sparkline_sample(ring: &mut [u8; 12], value: u8) {
    ring.copy_within(1.., 0);
    ring[11] = value.min(8);
}

/// Normalize two `(row, col)` selection coordinates into `(start, end)` in reading order.
fn normalize_selection(a: (usize, usize), b: (usize, usize)) -> ((usize, usize), (usize, usize)) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Extract plain ANSI-stripped substring across a `(start_row, start_col)..=(end_row, end_col)`
/// selection within the visible slice of `lines`.
pub(super) fn extract_selected_preview_text(
    lines: &[String],
    scroll: usize,
    visible_rows: usize,
    selection: ((usize, usize), (usize, usize)),
) -> String {
    if lines.is_empty() || visible_rows == 0 {
        return String::new();
    }
    let total = lines.len();
    let max_scroll = total.saturating_sub(visible_rows);
    let clamped_scroll = scroll.min(max_scroll);
    let end = total.saturating_sub(clamped_scroll);
    let start = end.saturating_sub(visible_rows);
    let visible = &lines[start..end];
    if visible.is_empty() {
        return String::new();
    }
    let ((r0, c0), (r1, c1)) = normalize_selection(selection.0, selection.1);
    let r0 = r0.min(visible.len() - 1);
    let r1 = r1.min(visible.len() - 1);
    let mut out = Vec::new();
    for (row_idx, raw) in visible.iter().enumerate().take(r1 + 1).skip(r0) {
        let plain = strip_ansi(raw);
        let chars: Vec<char> = plain.chars().collect();
        let lo = if row_idx == r0 { c0 } else { 0 };
        let hi = if row_idx == r1 {
            c1.saturating_add(1)
        } else {
            chars.len()
        };
        let lo = lo.min(chars.len());
        let hi = hi.min(chars.len());
        if lo < hi {
            out.push(chars[lo..hi].iter().collect::<String>());
        } else if r0 != r1 {
            out.push(String::new());
        }
    }
    out.join("\n")
}

/// Per-window live tile inside Mission Control (`/expose`).
#[derive(Debug, Clone)]
pub(super) struct ExposeTileData {
    pub window: Window,
    pub active_pane_id: Option<PaneId>,
    pub command: String,
    pub preview_lines: Vec<String>,
    pub last_hash: u64,
    pub sparkline: [u8; 12],
    pub flags: WindowStatusFlags,
}

/// View state for Mission Control / Exposé (`View::Expose`).
#[derive(Debug, Clone, Default)]
pub(super) struct ExposeState {
    pub tiles: Vec<ExposeTileData>,
    pub selected: usize,
    pub page: usize,
    pub status_note: Option<String>,
}

impl ExposeState {
    pub fn total_pages(&self) -> usize {
        self.tiles.len().div_ceil(EXPOSE_TILES_PER_PAGE).max(1)
    }

    pub fn page_range(&self) -> std::ops::Range<usize> {
        let total_pages = self.total_pages();
        let page = self.page.min(total_pages - 1);
        let start = page * EXPOSE_TILES_PER_PAGE;
        let end = (start + EXPOSE_TILES_PER_PAGE).min(self.tiles.len());
        start..end
    }

    pub fn sync_page_to_selected(&mut self) {
        if self.tiles.is_empty() {
            self.selected = 0;
            self.page = 0;
            return;
        }
        self.selected = self.selected.min(self.tiles.len() - 1);
        self.page = self.selected / EXPOSE_TILES_PER_PAGE;
    }

    pub fn grid_columns(&self) -> usize {
        let visible = self.page_range().len();
        match visible {
            0..=1 => 1,
            2..=4 => 2,
            _ => 3,
        }
    }
}

/// Hovered interactive hit target used for dirty-checked `MouseEventKind::Moved`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum HoverTarget {
    #[default]
    None,
    Menubar(MenubarItem),
    QuickAction(HomeQuickAction),
    MatrixTab(MatrixAction),
    MatrixCell(MatrixHitTarget),
    PreviewAction(PreviewAction),
    PreviewScrollbar,
    Sash(SashId),
    Controller(crate::ui::ControllerAction),
    SplitSlot(usize),
    SplitToolbar(SplitToolbarAction),
    ExposeTile(usize),
    ExposeControl(usize, TileControl),
    ContextMenuItem(usize),
}

/// Clickable item tracked for true `<= 350ms` double-click activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ClickableItem {
    Matrix(MatrixHitTarget),
    Radar(SessionId),
    Picker(usize),
    TargetPick(usize),
    Action(usize),
    Theme(usize),
    Config(usize),
    Client(usize),
    Buffer(usize),
    Snapshot(usize),
    SplitSlot(usize),
    SplitWindow(usize),
    ExposeTile(usize),
    DualFormCompletion(usize),
}

/// Action executed when a floating Context Menu or Top Menubar Dropdown item is chosen.
#[derive(Debug, Clone)]
pub(super) enum ContextMenuAction {
    Resource(Resource, ResourceAction),
    RunCommand(CommandKind),
    ReloadTmuxConf,
    TogglePip(PaneId),
    InlineRename(RenameTarget),
    SplitAction(SplitToolbarAction),
    SplitRemoveSlot(usize),
    ExposeEnter(usize),
    ExposeZoom(usize),
    ExposeClose(usize),
    PreviewCopy,
    PreviewSearch,
    PreviewExportLog,
    PreviewTogglePipe,
    PreviewClearHistory,
}

#[derive(Debug, Clone)]
pub(super) struct ContextMenuEntry {
    pub view: ContextMenuItemView,
    pub action: ContextMenuAction,
}

/// Floating context menu / menubar dropdown state.
#[derive(Debug, Clone)]
pub(super) struct ContextMenuState {
    pub title: String,
    pub anchor: (u16, u16),
    pub items: Vec<ContextMenuEntry>,
    pub selected: usize,
    pub dim_backdrop: bool,
    pub from_menubar: bool,
}

/// Target for `F2` / `[✎]` floating inline rename popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RenameTarget {
    Session(SessionId, String),
    Window(WindowId, String),
    PaneTitle(PaneId, String),
}

impl RenameTarget {
    pub fn current_name(&self) -> &str {
        match self {
            Self::Session(_, name) | Self::Window(_, name) | Self::PaneTitle(_, name) => name,
        }
    }

    pub fn title(&self, zh: bool) -> &'static str {
        match self {
            Self::Session(..) => {
                if zh {
                    "重命名会话"
                } else {
                    "Rename Session"
                }
            }
            Self::Window(..) => {
                if zh {
                    "重命名窗口"
                } else {
                    "Rename Window"
                }
            }
            Self::PaneTitle(..) => {
                if zh {
                    "重命名窗格标题"
                } else {
                    "Rename Pane Title"
                }
            }
        }
    }

    pub fn target_label(&self) -> String {
        match self {
            Self::Session(id, name) => format!("{name} ({id})"),
            Self::Window(id, name) => format!("{name} ({id})"),
            Self::PaneTitle(id, name) => {
                if name.is_empty() {
                    id.to_string()
                } else {
                    format!("{name} ({id})")
                }
            }
        }
    }
}

/// Floating inline rename state (`F2` or `[✎]`).
#[derive(Debug, Clone)]
pub(super) struct InlineRenameState {
    pub target: RenameTarget,
    pub input: TextInput,
    pub anchor: Option<(u16, u16)>,
}

/// Active desktop toast notification with shrinking countdown progress.
#[derive(Debug, Clone)]
pub(super) struct ToastState {
    pub kind: ToastKind,
    pub message: String,
    pub created_at: Instant,
    pub duration: Duration,
    pub last_rendered_bucket: u8,
}

impl ToastState {
    pub fn new(kind: ToastKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            created_at: Instant::now(),
            duration: TOAST_DURATION,
            last_rendered_bucket: 20,
        }
    }

    pub fn remaining_fraction(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32().max(0.001);
        (1.0 - elapsed / total).clamp(0.0, 1.0)
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }
}

/// Payload carried by an in-progress mouse drag operation.
#[derive(Debug, Clone)]
pub(super) enum DragKind {
    Sash {
        id: SashId,
        start_col: u16,
        start_row: u16,
        start_ratio: u16,
        last_step_col: u16,
        last_step_row: u16,
    },
    Scrollbar {
        track: Rect,
    },
    PreviewSelect {
        start: (usize, usize),
        current: (usize, usize),
    },
    MatrixWindow {
        window: Window,
        pane_id: Option<PaneId>,
        previous_preview_pane: Option<PaneId>,
        label: String,
    },
    SplitSlot {
        slot_id: usize,
        label: String,
    },
    ExposeTile {
        tile_index: usize,
        window: Window,
        pane_id: Option<PaneId>,
        label: String,
    },
}

/// Active or pending left-button mouse drag gesture.
#[derive(Debug, Clone)]
pub(super) struct DragState {
    pub origin: (u16, u16),
    pub cursor: (u16, u16),
    pub active: bool,
    pub kind: DragKind,
    pub hover_detail: String,
    pub dock_target: Option<(Rect, DockZone)>,
    pub split_dock_slot: Option<(usize, DockZone)>,
    pub expose_dock_tile: Option<(usize, DockZone)>,
}

/// Unified desktop GUI runtime state attached to [`App`].
#[derive(Debug)]
pub(super) struct DesktopState {
    pub pip_target: Option<PaneId>,
    pub pip_label: String,
    pub home_split_ratio: u16,
    pub hover: HoverTarget,
    pub hover_pos: (u16, u16),
    pub tooltip_text: Option<String>,
    pub last_click: Option<(ClickableItem, Instant)>,
    pub context_menu: Option<ContextMenuState>,
    pub inline_rename: Option<InlineRenameState>,
    pub drag: Option<DragState>,
    pub toast: Option<ToastState>,
    pub activity_sparkline: [u8; 12],
    pub last_preview_hash: u64,
    pub preview_error_lines: Vec<usize>,
    pub preview_selection: Option<((usize, usize), (usize, usize))>,
    pub last_stream_poll: Instant,
    pub last_frame_area: Rect,
    pub menubar_hits: Vec<(Rect, MenubarItem)>,
    pub sash_hits: Vec<(Rect, SashId)>,
    pub preview_layout: PreviewCardLayout,
    pub expose_hits: Vec<ExposeTileHit>,
    pub overlay_layout: DesktopOverlayLayout,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            pip_target: None,
            pip_label: String::new(),
            home_split_ratio: 0,
            hover: HoverTarget::None,
            hover_pos: (0, 0),
            tooltip_text: None,
            last_click: None,
            context_menu: None,
            inline_rename: None,
            drag: None,
            toast: None,
            activity_sparkline: [0; 12],
            last_preview_hash: 0,
            preview_error_lines: Vec::new(),
            preview_selection: None,
            last_stream_poll: Instant::now(),
            last_frame_area: Rect::default(),
            menubar_hits: Vec::new(),
            sash_hits: Vec::new(),
            preview_layout: PreviewCardLayout::default(),
            expose_hits: Vec::new(),
            overlay_layout: DesktopOverlayLayout::default(),
        }
    }
}

impl DesktopState {
    pub fn has_active_toast(&self) -> bool {
        self.toast.as_ref().is_some_and(|t| !t.is_expired())
    }

    pub fn is_dragging_sash(&self, id: SashId) -> bool {
        matches!(
            &self.drag,
            Some(DragState {
                active: true,
                kind: DragKind::Sash { id: active_id, .. },
                ..
            }) if *active_id == id
        )
    }

    pub fn dragging_sash_id(&self) -> Option<SashId> {
        match &self.drag {
            Some(DragState {
                active: true,
                kind: DragKind::Sash { id, .. },
                ..
            }) => Some(*id),
            _ => None,
        }
    }

    pub fn hovered_sash_id(&self) -> Option<SashId> {
        match self.hover {
            HoverTarget::Sash(id) => Some(id),
            _ => None,
        }
    }

    pub fn is_dragging_scrollbar(&self) -> bool {
        matches!(
            &self.drag,
            Some(DragState {
                kind: DragKind::Scrollbar { .. },
                ..
            })
        )
    }

    pub fn record_click(&mut self, item: ClickableItem) -> bool {
        let now = Instant::now();
        if let Some((prev, at)) = &self.last_click
            && prev == &item
            && now.duration_since(*at) <= DOUBLE_CLICK_WINDOW
        {
            self.last_click = None;
            return true;
        }
        self.last_click = Some((item, now));
        false
    }

    pub fn clear_render_hits(&mut self, frame_area: Rect) {
        self.last_frame_area = frame_area;
        self.menubar_hits.clear();
        self.sash_hits.clear();
        self.preview_layout = PreviewCardLayout::default();
        self.expose_hits.clear();
        self.overlay_layout = DesktopOverlayLayout::default();
    }

    pub fn tick_toast(&mut self) -> bool {
        let Some(toast) = self.toast.as_mut() else {
            return false;
        };
        if toast.is_expired() {
            self.toast = None;
            return true;
        }
        let bucket = (toast.remaining_fraction() * 20.0).ceil() as u8;
        if bucket != toast.last_rendered_bucket {
            toast.last_rendered_bucket = bucket;
            return true;
        }
        false
    }
}

impl App<'_> {
    /// Push a desktop toast notification (also mirrors into `self.status` for compact/inline views).
    pub(super) fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        let msg = message.into();
        self.status = msg.clone();
        self.desktop.toast = Some(ToastState::new(kind, msg));
    }

    /// Recompute cached preview hash & error tick lines whenever `self.cached_preview` changes.
    pub(super) fn sync_preview_desktop_metadata(&mut self) {
        if let Some((_, _, lines)) = &self.cached_preview {
            let new_hash = hash_lines(lines);
            if self.desktop.last_preview_hash != 0 && new_hash != self.desktop.last_preview_hash {
                push_sparkline_sample(&mut self.desktop.activity_sparkline, 6);
            }
            self.desktop.last_preview_hash = new_hash;
            self.desktop.preview_error_lines = detect_error_lines(lines);
        } else {
            self.desktop.last_preview_hash = 0;
            self.desktop.preview_error_lines.clear();
            self.desktop.preview_selection = None;
        }
    }

    /// Adaptive poll interval for `cli::run_tui`.
    /// Strictly returns `None` for `live_stream` when `ScreenMode::Inline` so idle inline
    /// terminals continue emitting 0 idle bytes!
    pub fn next_poll_interval(&self, mode: ScreenMode) -> Option<Duration> {
        if self.desktop.pip_target.is_some() {
            return Some(PIP_POLL_INTERVAL);
        }
        if self.should_animate() || self.desktop.has_active_toast() {
            return Some(ANIM_POLL_INTERVAL);
        }
        if mode == ScreenMode::Fullscreen && self.config.live_stream {
            if matches!(self.views.top(), Some(View::Expose(_))) {
                return Some(EXPOSE_POLL_INTERVAL);
            }
            if self.config.live_preview || matches!(self.views.top(), Some(View::Split(_))) {
                return Some(STREAM_POLL_INTERVAL);
            }
        }
        None
    }

    /// Advance animation/toast countdown and perform dirty-checked live preview/expose streaming.
    /// Returns `true` ONLY if visible output actually changed (skipping redraws on unchanged tmux state).
    pub fn tick_live_frame(&mut self, mode: ScreenMode) -> bool {
        let mut dirty = false;
        if self.should_animate() {
            self.tick_animation();
            dirty = true;
        }
        if self.desktop.tick_toast() {
            dirty = true;
        }

        let stream_interval = if self.desktop.pip_target.is_some() {
            Some(PIP_POLL_INTERVAL)
        } else if mode == ScreenMode::Fullscreen && self.config.live_stream {
            if matches!(self.views.top(), Some(View::Expose(_))) {
                Some(EXPOSE_POLL_INTERVAL)
            } else if self.config.live_preview || matches!(self.views.top(), Some(View::Split(_))) {
                Some(STREAM_POLL_INTERVAL)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(interval) = stream_interval
            && self.desktop.last_stream_poll.elapsed() >= interval
        {
            self.desktop.last_stream_poll = Instant::now();
            if self.refresh_live_stream_dirty() {
                dirty = true;
            }
        }
        dirty
    }

    /// Recapture currently visible live pane(s), returning `true` ONLY when captured output hash changed.
    pub(super) fn refresh_live_stream_dirty(&mut self) -> bool {
        match self.views.0.last_mut() {
            Some(View::Expose(state)) => {
                let range = state.page_range();
                let mut changed = false;
                let mut active_tiles = 0_u8;
                for tile in state.tiles.get_mut(range).into_iter().flatten() {
                    let target = tile.active_pane_id.as_ref().map_or_else(
                        || Target::Window(tile.window.session_id.clone(), tile.window.id.clone()),
                        |pid| Target::Pane(pid.clone()),
                    );
                    if let Ok(lines) = self.tmux.capture_pane(&target, EXPOSE_CAPTURE_LINES) {
                        let next_hash = hash_lines(&lines);
                        let delta = if next_hash != tile.last_hash {
                            changed = true;
                            active_tiles = active_tiles.saturating_add(1);
                            let line_diff = tile.preview_lines.len().abs_diff(lines.len());
                            tile.preview_lines = lines;
                            tile.last_hash = next_hash;
                            (line_diff.clamp(2, 8)) as u8
                        } else {
                            tile.sparkline[11].saturating_sub(2)
                        };
                        push_sparkline_sample(&mut tile.sparkline, delta);
                    }
                }
                let global_sample = if active_tiles > 0 {
                    (active_tiles.saturating_mul(2) + 2).min(8)
                } else {
                    self.desktop.activity_sparkline[11].saturating_sub(1)
                };
                push_sparkline_sample(&mut self.desktop.activity_sparkline, global_sample);
                changed
            }
            Some(View::Split(studio)) => {
                if !self.config.live_preview {
                    return false;
                }
                let mut changed = false;
                for (_, fill) in studio.root.leaves() {
                    match fill {
                        SplitSlotFill::Pane(pid) => {
                            if let Ok(lines) = self
                                .tmux
                                .capture_pane(&Target::Pane(pid.clone()), PREVIEW_CAPTURE_LINES)
                            {
                                let prev_hash = studio
                                    .pane_previews
                                    .get(&pid)
                                    .map_or(0, |old| hash_lines(old));
                                if hash_lines(&lines) != prev_hash {
                                    studio.pane_previews.insert(pid, lines);
                                    changed = true;
                                }
                            }
                        }
                        SplitSlotFill::Window(wid) => {
                            if let Some(opt) = studio.window_option(&wid).cloned() {
                                let target = opt.active_pane_id.as_ref().map_or_else(
                                    || {
                                        Target::Window(
                                            opt.window.session_id.clone(),
                                            opt.window.id.clone(),
                                        )
                                    },
                                    |pid| Target::Pane(pid.clone()),
                                );
                                if let Ok(lines) =
                                    self.tmux.capture_pane(&target, PREVIEW_CAPTURE_LINES)
                                {
                                    let prev_hash =
                                        studio.previews.get(&wid).map_or(0, |old| hash_lines(old));
                                    if hash_lines(&lines) != prev_hash {
                                        studio.previews.insert(wid, lines);
                                        changed = true;
                                    }
                                }
                            }
                        }
                        SplitSlotFill::NewShell => {}
                    }
                }
                if changed {
                    push_sparkline_sample(&mut self.desktop.activity_sparkline, 6);
                }
                changed
            }
            _ => {
                if !self.config.live_preview && self.desktop.pip_target.is_none() {
                    return false;
                }
                let target = if let Some(pid) = self.desktop.pip_target.clone() {
                    Some(Target::Pane(pid))
                } else {
                    self.preview_target.clone()
                };
                let Some(target) = target else {
                    return false;
                };
                let Ok(lines) = self.tmux.capture_pane(&target, PREVIEW_CAPTURE_LINES) else {
                    return false;
                };
                let new_hash = hash_lines(&lines);
                if new_hash == self.desktop.last_preview_hash {
                    let decay = self.desktop.activity_sparkline[11].saturating_sub(1);
                    if decay != self.desktop.activity_sparkline[11] {
                        push_sparkline_sample(&mut self.desktop.activity_sparkline, decay);
                    }
                    return false;
                }
                let max_scroll = lines.len().saturating_sub(self.preview_visible_rows());
                self.preview_scroll = self.preview_scroll.min(max_scroll);
                if let Some((_, _, cached_lines)) = self.cached_preview.as_mut() {
                    *cached_lines = lines;
                } else {
                    self.cached_preview =
                        Some((self.desktop.pip_label.clone(), "live".into(), lines));
                }
                self.desktop.last_preview_hash = new_hash;
                if let Some((_, _, cached_lines)) = &self.cached_preview {
                    self.desktop.preview_error_lines = detect_error_lines(cached_lines);
                }
                if self.preview_search.query_str().is_some() {
                    self.recompute_preview_search(false);
                }
                push_sparkline_sample(&mut self.desktop.activity_sparkline, 7);
                true
            }
        }
    }

    // -------------------------------------------------------------------------
    // Live PIP Direct Type-Through Engine
    // -------------------------------------------------------------------------

    /// Whether Live PIP direct keyboard type-through is currently active.
    pub fn is_live_pip_active(&self) -> bool {
        self.desktop.pip_target.is_some()
    }

    /// Resolve the best pane for toggling Live PIP from the current view.
    fn resolve_pip_candidate(&self) -> Option<(PaneId, String)> {
        if let Some(View::Expose(state)) = self.views.top()
            && let Some(tile) = state.tiles.get(state.selected)
        {
            let pane_id = tile.active_pane_id.clone().or_else(|| {
                self.representative_pane_for_window(&tile.window.id)
                    .map(|p| p.id)
            })?;
            let label = format!(
                "{}:{} {}",
                tile.window.session_name, tile.window.index, tile.window.name
            );
            return Some((pane_id, label));
        }
        let pane = self.focused_or_active_pane()?;
        let label = format!(
            "{}:{}.{} {}",
            pane.session_name, pane.window_index, pane.index, pane.current_command
        );
        Some((pane.id, label))
    }

    pub(super) fn toggle_live_pip(&mut self, explicit_pane: Option<(PaneId, String)>) {
        if let Some((pane_id, label)) = explicit_pane {
            if self.desktop.pip_target.as_ref() == Some(&pane_id) {
                self.exit_live_pip();
            } else {
                self.enter_live_pip(pane_id, label);
            }
            return;
        }
        if self.desktop.pip_target.is_some() {
            self.exit_live_pip();
            return;
        }
        if let Some((pane_id, label)) = self.resolve_pip_candidate() {
            self.enter_live_pip(pane_id, label);
        } else {
            let msg = self
                .theme
                .tr(
                    "No active pane to enter Live PIP.",
                    "当前没有可直控的活跃窗格。",
                )
                .to_owned();
            self.push_toast(ToastKind::Warning, msg);
        }
    }

    pub(super) fn enter_live_pip(&mut self, pane_id: PaneId, label: String) {
        self.desktop.context_menu = None;
        self.desktop.inline_rename = None;
        self.preview_search.active = false;
        self.desktop.pip_target = Some(pane_id.clone());
        self.desktop.pip_label = label.clone();
        self.preview_target = Some(Target::Pane(pane_id));
        self.preview_scroll = 0;
        let _ = self.refresh_live_stream_dirty();
        let msg = if self.theme.language.is_zh() {
            format!("● 直控已开启：{label}（Esc 退出，所有按键直通终端）")
        } else {
            format!("● LIVE PIP active: {label} (Esc exits, all keys forward)")
        };
        self.push_toast(ToastKind::Success, msg);
    }

    pub(super) fn exit_live_pip(&mut self) {
        if self.desktop.pip_target.take().is_some() {
            let msg = self
                .theme
                .tr("Exited Live PIP input mode", "已退出 Live PIP 直控输入模式")
                .to_owned();
            self.push_toast(ToastKind::Info, msg);
        }
    }

    pub(super) fn send_literal_esc_to_pip(&mut self) {
        let Some(pane_id) = self.desktop.pip_target.clone() else {
            return;
        };
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        if let Err(err) = self.tmux.send_key_event(&pane_id, esc) {
            self.push_toast(ToastKind::Error, err.to_string());
        } else {
            let _ = self.refresh_live_stream_dirty();
        }
    }

    /// Intercepts keyboard & paste events at the VERY TOP of `handle_event` when Live PIP is active.
    /// Returns `Some(changed)` if consumed by Live PIP; `None` if Live PIP is inactive or event is non-key/paste.
    pub(super) fn try_handle_live_pip_event(&mut self, event: &Event) -> Option<bool> {
        let pane_id = self.desktop.pip_target.clone()?;
        match event {
            Event::Paste(text) => {
                if let Err(err) = self.tmux.send_paste(&pane_id, text) {
                    self.push_toast(ToastKind::Error, err.to_string());
                } else {
                    self.desktop.last_stream_poll = Instant::now();
                    let _ = self.refresh_live_stream_dirty();
                }
                Some(true)
            }
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return Some(false);
                }
                if key.code == KeyCode::Esc
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.exit_live_pip();
                    return Some(true);
                }
                if let Err(err) = self.tmux.send_key_event(&pane_id, *key) {
                    self.push_toast(ToastKind::Error, err.to_string());
                } else {
                    self.desktop.last_stream_poll = Instant::now();
                    let _ = self.refresh_live_stream_dirty();
                }
                Some(true)
            }
            _ => None,
        }
    }

    // -------------------------------------------------------------------------
    // Top IDE Menubar & Active Tab State
    // -------------------------------------------------------------------------

    pub(super) fn active_desktop_tab(&self) -> ActiveDesktopTab {
        match self.views.top() {
            None => ActiveDesktopTab::Studio,
            Some(View::Expose(_)) => ActiveDesktopTab::Expose,
            Some(View::Split(_)) => ActiveDesktopTab::Split,
            Some(View::LayoutStudio(_)) => ActiveDesktopTab::Layout,
            Some(_) => ActiveDesktopTab::Other,
        }
    }

    pub(super) fn build_menubar_state(&self) -> DesktopMenubarState<'_> {
        let hovered = match self.desktop.hover {
            HoverTarget::Menubar(item) => Some(item),
            _ => None,
        };
        let menu_open = self
            .desktop
            .context_menu
            .as_ref()
            .is_some_and(|m| m.from_menubar);
        DesktopMenubarState {
            active_tab: self.active_desktop_tab(),
            hovered,
            menu_open,
            pip_active: self.desktop.pip_target.is_some(),
            inside_tmux: self.context.is_inside(),
            sparkline: &self.desktop.activity_sparkline,
        }
    }

    pub(super) fn handle_menubar_click(&mut self, item: MenubarItem, rect: Rect) {
        self.desktop.inline_rename = None;
        if item != MenubarItem::MenuDropdown {
            self.desktop.context_menu = None;
        }
        match item {
            MenubarItem::TabStudio => {
                self.desktop.pip_target = None;
                self.views = ViewStack::default();
                self.matrix_action = MatrixAction::Open;
                self.refresh_home_data();
            }
            MenubarItem::TabExpose => {
                self.desktop.pip_target = None;
                self.open_expose_view();
            }
            MenubarItem::TabSplit => self.open_split_from_menubar(),
            MenubarItem::TabLayout => {
                self.desktop.pip_target = None;
                self.views = ViewStack::default();
                self.open_layout_studio(None);
            }
            MenubarItem::MenuDropdown => {
                if self
                    .desktop
                    .context_menu
                    .as_ref()
                    .is_some_and(|m| m.from_menubar)
                {
                    self.desktop.context_menu = None;
                } else {
                    self.open_menubar_dropdown((rect.x, rect.bottom()));
                }
            }
            MenubarItem::PipToggle => self.toggle_live_pip(None),
            MenubarItem::ThemeCycle => {
                let next = self.theme.preset.next();
                self.theme = self.theme.with_preset(next);
                self.config.theme = next;
                self.push_toast(ToastKind::Info, theme_toast(next, self.theme.language));
            }
            MenubarItem::LanguageToggle => {
                self.config.language = self.config.language.toggle();
                self.theme = self.config.effective_theme();
                let msg = if self.theme.language.is_zh() {
                    "✦ 界面语言已切换为简体中文"
                } else {
                    "✦ Interface language switched to English"
                };
                self.push_toast(ToastKind::Success, msg);
            }
            MenubarItem::Config => {
                self.desktop.pip_target = None;
                self.config_note = None;
                if !matches!(self.views.top(), Some(View::Config { .. })) {
                    self.views.push(View::Config { selected: 0 });
                }
            }
            MenubarItem::Detach => {
                self.desktop.pip_target = None;
                self.detach();
            }
        }
    }

    /// Safe helper for Split tab click (never panics even if there are zero sessions).
    pub(super) fn open_split_from_menubar(&mut self) {
        self.desktop.pip_target = None;
        self.views = ViewStack::default();
        self.refresh_home_data();
        if let Some(win) = self.focused_or_active_window() {
            self.open_split_studio_from_home(Resource::Window(win));
        } else if let Some(sess) = self.home_sessions.first().cloned() {
            self.open_split_studio_from_home(Resource::Session(sess));
        } else {
            self.matrix_action = MatrixAction::Split;
            let msg = self
                .theme
                .tr(
                    "Create a session first to open Split Studio.",
                    "请先创建一个会话后再打开分屏工作台。",
                )
                .to_owned();
            self.push_toast(ToastKind::Info, msg);
        }
    }

    // -------------------------------------------------------------------------
    // Mission Control / Exposé (`/expose`, `Ctrl+G`, `F3`)
    // -------------------------------------------------------------------------

    pub(super) fn open_expose_view(&mut self) {
        self.desktop.context_menu = None;
        self.desktop.inline_rename = None;
        self.refresh_home_data();
        let state = self.build_expose_state(0);
        // Replace existing Expose view if already on top.
        if matches!(self.views.top(), Some(View::Expose(_))) {
            self.views.pop();
        }
        self.views.push(View::Expose(state));
    }

    pub(super) fn build_expose_state(&self, prev_selected: usize) -> ExposeState {
        let mut tiles = Vec::with_capacity(self.home_windows.len());
        for window in &self.home_windows {
            let rep_pane = self.representative_pane_for_window(&window.id);
            let active_pane_id = rep_pane.as_ref().map(|p| p.id.clone());
            let command = rep_pane
                .as_ref()
                .map(|p| p.current_command.clone())
                .unwrap_or_default();
            let target = active_pane_id.as_ref().map_or_else(
                || Target::Window(window.session_id.clone(), window.id.clone()),
                |pid| Target::Pane(pid.clone()),
            );
            let preview_lines = self
                .tmux
                .capture_pane(&target, EXPOSE_CAPTURE_LINES)
                .ok()
                .unwrap_or_default();
            let last_hash = hash_lines(&preview_lines);
            let dead = self
                .home_panes
                .iter()
                .any(|p| p.window_id == window.id && p.dead);
            let multi_clients = self
                .home_sessions
                .iter()
                .find(|s| s.id == window.session_id)
                .map_or(0, |s| s.attached);
            let flags = WindowStatusFlags {
                zoomed: window.zoomed,
                synchronized: window.synchronized,
                activity: window.activity_flag,
                bell: window.bell_flag,
                silence: window.silence_flag,
                dead,
                multi_clients,
            };
            tiles.push(ExposeTileData {
                window: window.clone(),
                active_pane_id,
                command,
                preview_lines,
                last_hash,
                sparkline: [0; 12],
                flags,
            });
        }
        let mut state = ExposeState {
            tiles,
            selected: prev_selected,
            page: 0,
            status_note: None,
        };
        state.sync_page_to_selected();
        state
    }

    pub(super) fn refresh_expose_state(&mut self, state: &mut ExposeState) {
        self.refresh_home_data();
        let old_sparks: std::collections::HashMap<WindowId, ([u8; 12], u64)> = state
            .tiles
            .iter()
            .map(|t| (t.window.id.clone(), (t.sparkline, t.last_hash)))
            .collect();
        let mut rebuilt = self.build_expose_state(state.selected);
        for tile in &mut rebuilt.tiles {
            if let Some((spark, _)) = old_sparks.get(&tile.window.id) {
                tile.sparkline = *spark;
            }
        }
        rebuilt.status_note = state.status_note.take();
        *state = rebuilt;
    }

    pub(super) fn expose_key(&mut self, mut state: ExposeState, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let no_ctrl_alt = !ctrl && !alt;
        let total = state.tiles.len();
        let cols = state.grid_columns() as isize;
        match key.code {
            KeyCode::Left => {
                state.selected = wrap_step(state.selected, total, -1);
                state.sync_page_to_selected();
            }
            KeyCode::Char('h') if no_ctrl_alt => {
                state.selected = wrap_step(state.selected, total, -1);
                state.sync_page_to_selected();
            }
            KeyCode::Right => {
                state.selected = wrap_step(state.selected, total, 1);
                state.sync_page_to_selected();
            }
            KeyCode::Char('l') if no_ctrl_alt => {
                state.selected = wrap_step(state.selected, total, 1);
                state.sync_page_to_selected();
            }
            KeyCode::Up => {
                state.selected = wrap_step(state.selected, total, -cols);
                state.sync_page_to_selected();
            }
            KeyCode::Char('k') if no_ctrl_alt => {
                state.selected = wrap_step(state.selected, total, -cols);
                state.sync_page_to_selected();
            }
            KeyCode::Char('p') if ctrl && !alt => {
                state.selected = wrap_step(state.selected, total, -cols);
                state.sync_page_to_selected();
            }
            KeyCode::Down => {
                state.selected = wrap_step(state.selected, total, cols);
                state.sync_page_to_selected();
            }
            KeyCode::Char('j') if no_ctrl_alt => {
                state.selected = wrap_step(state.selected, total, cols);
                state.sync_page_to_selected();
            }
            KeyCode::Char('n') if ctrl && !alt => {
                state.selected = wrap_step(state.selected, total, cols);
                state.sync_page_to_selected();
            }
            KeyCode::PageUp => {
                let pages = state.total_pages();
                state.page = wrap_step(state.page, pages, -1);
                state.selected = state.page * EXPOSE_TILES_PER_PAGE;
                state.sync_page_to_selected();
            }
            KeyCode::PageDown => {
                let pages = state.total_pages();
                state.page = wrap_step(state.page, pages, 1);
                state.selected = state.page * EXPOSE_TILES_PER_PAGE;
                state.sync_page_to_selected();
            }
            KeyCode::Enter => {
                if let Some(tile) = state.tiles.get(state.selected).cloned() {
                    self.views.push(View::Expose(state));
                    self.enter(Target::Window(tile.window.session_id, tile.window.id));
                    return;
                }
            }
            KeyCode::Char('i' | 'I') if no_ctrl_alt => {
                if let Some(tile) = state.tiles.get(state.selected).cloned() {
                    let pane_id = tile.active_pane_id.or_else(|| {
                        self.representative_pane_for_window(&tile.window.id)
                            .map(|p| p.id)
                    });
                    self.views.push(View::Expose(state));
                    if let Some(pid) = pane_id {
                        let label = format!(
                            "{}:{} {}",
                            tile.window.session_name, tile.window.index, tile.window.name
                        );
                        self.toggle_live_pip(Some((pid, label)));
                    }
                    return;
                }
            }
            KeyCode::Char('z' | 'Z') if no_ctrl_alt => {
                if let Some(tile) = state.tiles.get(state.selected).cloned()
                    && let Some(pid) = tile.active_pane_id.or_else(|| {
                        self.representative_pane_for_window(&tile.window.id)
                            .map(|p| p.id)
                    })
                {
                    match self.tmux.toggle_zoom_pane(&pid) {
                        Ok(()) => {
                            self.refresh_expose_state(&mut state);
                            let msg = self
                                .theme
                                .tr("✦ Toggled zoom on window", "✦ 已切换窗口全屏缩放")
                                .to_owned();
                            state.status_note = Some(msg.clone());
                            self.push_toast(ToastKind::Success, msg);
                        }
                        Err(err) => {
                            state.status_note = Some(err.to_string());
                            self.push_toast(ToastKind::Error, err.to_string());
                        }
                    }
                }
            }
            KeyCode::F(2) => {
                if let Some(tile) = state.tiles.get(state.selected).cloned() {
                    self.views.push(View::Expose(state));
                    self.open_inline_rename(
                        RenameTarget::Window(tile.window.id, tile.window.name),
                        None,
                    );
                    return;
                }
            }
            KeyCode::Char('x' | 'X') | KeyCode::Delete if no_ctrl_alt => {
                if let Some(tile) = state.tiles.get(state.selected).cloned() {
                    self.views.push(View::Expose(state));
                    self.kill_confirm(Resource::Window(tile.window));
                    return;
                }
            }
            KeyCode::Char('r' | 'R') if no_ctrl_alt => {
                self.refresh_expose_state(&mut state);
                let msg = self
                    .theme
                    .tr("Refreshed Mission Control wall", "已刷新全景墙实时画面")
                    .to_owned();
                state.status_note = Some(msg.clone());
                self.push_toast(ToastKind::Info, msg);
            }
            _ => {}
        }
        self.views.push(View::Expose(state));
    }

    pub(super) fn build_expose_tile_views<'s>(
        &self,
        state: &'s ExposeState,
    ) -> Vec<ExposeTileView<'s>> {
        let range = state.page_range();
        let drag_dock_tile = self.desktop.drag.as_ref().and_then(|d| d.expose_dock_tile);
        range
            .filter_map(|idx| {
                let tile = state.tiles.get(idx)?;
                let hovered = matches!(
                    self.desktop.hover,
                    HoverTarget::ExposeTile(h) | HoverTarget::ExposeControl(h, _) if h == idx
                );
                let hovered_control = match self.desktop.hover {
                    HoverTarget::ExposeControl(h, ctrl) if h == idx => Some(ctrl),
                    _ => None,
                };
                let pip_active =
                    tile.active_pane_id.is_some() && self.desktop.pip_target == tile.active_pane_id;
                let dock_highlight = drag_dock_tile.and_then(|(t, z)| (t == idx).then_some(z));
                Some(ExposeTileView {
                    index: idx,
                    session_name: &tile.window.session_name,
                    window_index: tile.window.index,
                    window_name: &tile.window.name,
                    command: &tile.command,
                    preview_lines: &tile.preview_lines,
                    sparkline: &tile.sparkline,
                    flags: tile.flags,
                    selected: idx == state.selected,
                    hovered,
                    hovered_control,
                    pip_active,
                    dock_highlight,
                })
            })
            .collect()
    }

    pub(super) fn expose_view_summary<'s>(
        &self,
        state: &'s ExposeState,
        tiles: &'s [ExposeTileView<'s>],
    ) -> ExposeView<'s> {
        ExposeView {
            tiles,
            selected: state.selected,
            page: state.page,
            total_pages: state.total_pages(),
            total_windows: state.tiles.len(),
            pip_active: self.desktop.pip_target.is_some(),
            status_note: state.status_note.as_deref(),
        }
    }

    // -------------------------------------------------------------------------
    // Inline Rename Popover (`F2` / `[✎]`)
    // -------------------------------------------------------------------------

    pub(super) fn open_inline_rename(&mut self, target: RenameTarget, anchor: Option<(u16, u16)>) {
        self.desktop.context_menu = None;
        let initial = target.current_name().to_owned();
        self.desktop.inline_rename = Some(InlineRenameState {
            target,
            input: TextInput::new_selected(initial),
            anchor,
        });
    }

    pub(super) fn open_inline_rename_for_focused(&mut self) -> bool {
        match self.views.top() {
            None => match self.composer_mode {
                ComposerMode::LiveRadar => {
                    if let Some(sess) = self.home_sessions.get(self.radar_selected).cloned() {
                        self.open_inline_rename(RenameTarget::Session(sess.id, sess.name), None);
                        return true;
                    }
                    false
                }
                ComposerMode::CommandJump => match self.matrix.selection() {
                    Some(MatrixSelection::Session(s)) => {
                        self.open_inline_rename(
                            RenameTarget::Session(s.id.clone(), s.name.clone()),
                            None,
                        );
                        true
                    }
                    Some(MatrixSelection::Window(_, w)) => {
                        self.open_inline_rename(
                            RenameTarget::Window(w.id.clone(), w.name.clone()),
                            None,
                        );
                        true
                    }
                    _ => false,
                },
                ComposerMode::QuickLaunch => false,
            },
            Some(View::Expose(state)) => {
                if let Some(tile) = state.tiles.get(state.selected).cloned() {
                    self.open_inline_rename(
                        RenameTarget::Window(tile.window.id, tile.window.name),
                        None,
                    );
                    true
                } else {
                    false
                }
            }
            Some(View::Picker(picker)) => {
                let mut matcher = self.matcher.clone();
                let rows = picker.rows(&mut matcher);
                if let Some(super::PickerRow::Resource(idx)) = rows.get(picker.selected).copied()
                    && let Some(res) = picker.resources.get(idx).cloned()
                {
                    return self.open_inline_rename_for_resource(res, None);
                }
                false
            }
            Some(View::LayoutStudio(_)) | Some(View::Split(_)) => {
                if let Some(win) = self.focused_or_active_window() {
                    self.open_inline_rename(RenameTarget::Window(win.id, win.name), None);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(super) fn open_inline_rename_for_resource(
        &mut self,
        resource: Resource,
        anchor: Option<(u16, u16)>,
    ) -> bool {
        let target = match resource {
            Resource::Session(s) => RenameTarget::Session(s.id, s.name),
            Resource::Window(w) => RenameTarget::Window(w.id, w.name),
            Resource::Pane(p) => RenameTarget::PaneTitle(p.id, p.title),
        };
        self.open_inline_rename(target, anchor);
        true
    }

    pub(super) fn handle_inline_rename_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut rename) = self.desktop.inline_rename.take() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.push_toast(
                    ToastKind::Info,
                    self.theme.tr("Rename cancelled", "已取消重命名"),
                );
                true
            }
            KeyCode::Enter => {
                let next = rename.input.value().trim().to_owned();
                if next.is_empty() {
                    self.push_toast(
                        ToastKind::Warning,
                        self.theme.tr("Name cannot be empty", "名称不能为空"),
                    );
                    self.desktop.inline_rename = Some(rename);
                    return true;
                }
                let res = match &rename.target {
                    RenameTarget::Session(id, _) => self.tmux.rename_session(id, &next),
                    RenameTarget::Window(id, _) => self.tmux.rename_window(id, &next),
                    RenameTarget::PaneTitle(id, _) => self.tmux.rename_pane_title(id, &next),
                };
                match res {
                    Ok(()) => {
                        self.refresh_home_data();
                        if let Some(View::Expose(mut state)) = self.views.pop() {
                            self.refresh_expose_state(&mut state);
                            self.views.push(View::Expose(state));
                        }
                        let msg = if self.theme.language.is_zh() {
                            format!("✦ 已重命名为 {next}")
                        } else {
                            format!("✦ Renamed to {next}")
                        };
                        self.push_toast(ToastKind::Success, msg);
                    }
                    Err(err) => {
                        self.push_toast(ToastKind::Error, err.to_string());
                    }
                }
                true
            }
            _ => {
                let changed = rename.input.handle_key(key);
                self.desktop.inline_rename = Some(rename);
                changed
            }
        }
    }

    // -------------------------------------------------------------------------
    // Floating Context Menu & Top-Left Menubar Dropdown
    // -------------------------------------------------------------------------

    fn menu_entry(
        icon: &str,
        label: impl Into<String>,
        shortcut: &str,
        danger: bool,
        divider_before: bool,
        action: ContextMenuAction,
    ) -> ContextMenuEntry {
        ContextMenuEntry {
            view: ContextMenuItemView {
                label: label.into(),
                icon: icon.into(),
                shortcut: shortcut.into(),
                danger,
                divider_before,
                enabled: true,
            },
            action,
        }
    }

    pub(super) fn open_menubar_dropdown(&mut self, anchor: (u16, u16)) {
        let zh = self.theme.language.is_zh();
        let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
        let mut items = vec![
            Self::menu_entry(
                "⊞",
                tr("Panes Inspector", "窗格总览"),
                "/pane",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Pane),
            ),
            Self::menu_entry(
                "👥",
                tr("Attached Clients", "客户端管理"),
                "/clients",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Clients),
            ),
            Self::menu_entry(
                "📋",
                tr("Paste Buffers", "剪贴板缓冲区"),
                "/buffers",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Buffers),
            ),
            Self::menu_entry(
                "💾",
                tr("Workspace Snapshots", "工作区快照"),
                "/snapshot",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Snapshot),
            ),
            Self::menu_entry(
                "❐",
                tr("Floating Scratchpad Popup", "原生悬浮终端"),
                "/popup",
                false,
                true,
                ContextMenuAction::RunCommand(CommandKind::Popup),
            ),
            Self::menu_entry(
                "↻",
                tr("Reload ~/.tmux.conf", "热重载 ~/.tmux.conf"),
                "",
                false,
                false,
                ContextMenuAction::ReloadTmuxConf,
            ),
            Self::menu_entry(
                "?",
                tr("Keyboard Shortcuts & Help", "快捷键与帮助"),
                "/help",
                false,
                true,
                ContextMenuAction::RunCommand(CommandKind::Help),
            ),
        ];
        if self.context.is_inside() {
            items.push(Self::menu_entry(
                "⏏",
                tr("Detach Current Client", "分离当前客户端"),
                "/exit",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Exit),
            ));
        }
        items.push(Self::menu_entry(
            "⏻",
            tr("Quit JustMux", "退出 JustMux"),
            "Esc",
            true,
            true,
            ContextMenuAction::RunCommand(CommandKind::Quit),
        ));
        self.desktop.context_menu = Some(ContextMenuState {
            title: tr("JustMux Menu", "JustMux 菜单").into(),
            anchor,
            items,
            selected: 0,
            dim_backdrop: false,
            from_menubar: true,
        });
    }

    pub(super) fn open_resource_context_menu(&mut self, resource: Resource, anchor: (u16, u16)) {
        let zh = self.theme.language.is_zh();
        let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
        let title = resource.label();
        let mut items = Vec::new();
        let pip_pane = match &resource {
            Resource::Pane(p) => Some(p.id.clone()),
            Resource::Window(w) => self.representative_pane_for_window(&w.id).map(|p| p.id),
            Resource::Session(s) => self
                .home_windows
                .iter()
                .find(|w| w.session_id == s.id && w.active)
                .or_else(|| self.home_windows.iter().find(|w| w.session_id == s.id))
                .and_then(|w| self.representative_pane_for_window(&w.id))
                .map(|p| p.id),
        };
        if let Some(pid) = pip_pane {
            let active = self.desktop.pip_target.as_ref() == Some(&pid);
            items.push(Self::menu_entry(
                "⌨",
                if active {
                    tr("Exit Live PIP Input", "退出 Live PIP 直控")
                } else {
                    tr("Live PIP Type-Through", "Live PIP 实时直控")
                },
                "i",
                false,
                false,
                ContextMenuAction::TogglePip(pid),
            ));
        }
        let rename_target = match &resource {
            Resource::Session(s) => RenameTarget::Session(s.id.clone(), s.name.clone()),
            Resource::Window(w) => RenameTarget::Window(w.id.clone(), w.name.clone()),
            Resource::Pane(p) => RenameTarget::PaneTitle(p.id.clone(), p.title.clone()),
        };
        items.push(Self::menu_entry(
            "✎",
            tr("Inline Rename", "行内重命名"),
            "F2",
            false,
            false,
            ContextMenuAction::InlineRename(rename_target),
        ));

        let actions = resource.actions_for(&self.piping_panes, self.theme.language);
        for (idx, (label, act)) in actions.into_iter().enumerate() {
            let (icon, shortcut, danger) = match act {
                ResourceAction::Open => ("▸", "Enter", false),
                ResourceAction::AttachReadOnly => ("👁", "", false),
                ResourceAction::NewWindow => ("+", "n", false),
                ResourceAction::Rename | ResourceAction::RenamePaneTitle => ("✎", "F2", false),
                ResourceAction::DetachOthers => ("⏏", "", false),
                ResourceAction::SaveSnapshot => ("💾", "", false),
                ResourceAction::Close => ("✕", "Mid", true),
                ResourceAction::EditInSplitStudio => ("◫", "", false),
                ResourceAction::LayoutAndResize => ("▦", "", false),
                ResourceAction::ToggleSyncPanes => ("⚡", "", false),
                ResourceAction::CycleMonitor => ("🔔", "", false),
                ResourceAction::MoveWindowToSession => ("⇢", "", false),
                ResourceAction::LinkWindowToSession => ("∞", "", false),
                ResourceAction::ReorderWindowLeft => ("←", "", false),
                ResourceAction::ReorderWindowRight => ("→", "", false),
                ResourceAction::ToggleZoomPane => ("⛶", "z", false),
                ResourceAction::SplitPaneRight => ("◫", "h", false),
                ResourceAction::SplitPaneDown => ("⬓", "v", false),
                ResourceAction::BreakPaneOut => ("↗", "", false),
                ResourceAction::SwapPaneWith => ("⇄", "", false),
                ResourceAction::JoinPaneInto => ("⤓", "", false),
                ResourceAction::CopyPaneOutput => ("📋", "y", false),
                ResourceAction::ExportPaneLog => ("📄", "", false),
                ResourceAction::TogglePipePane => ("●", "", false),
                ResourceAction::SendCtrlC => ("^C", "", false),
                ResourceAction::SendCommand => ("⌘", "", false),
                ResourceAction::ClearPaneHistory => ("⌫", "", false),
                ResourceAction::RespawnPane => ("↻", "", true),
            };
            items.push(Self::menu_entry(
                icon,
                label,
                shortcut,
                danger,
                idx == 0 || danger,
                ContextMenuAction::Resource(resource.clone(), act),
            ));
        }

        self.desktop.context_menu = Some(ContextMenuState {
            title,
            anchor,
            items,
            selected: 0,
            dim_backdrop: true,
            from_menubar: false,
        });
    }

    pub(super) fn open_preview_context_menu(&mut self, anchor: (u16, u16)) {
        let zh = self.theme.language.is_zh();
        let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
        let mut items = Vec::new();
        if let Some(pane) = self.focused_or_active_pane() {
            let active = self.desktop.pip_target.as_ref() == Some(&pane.id);
            items.push(Self::menu_entry(
                "⌨",
                if active {
                    tr("Exit Live PIP Input", "退出 Live PIP 直控")
                } else {
                    tr("Live PIP Type-Through", "Live PIP 实时直控")
                },
                "i",
                false,
                false,
                ContextMenuAction::TogglePip(pane.id),
            ));
        }
        items.extend([
            Self::menu_entry(
                "📋",
                tr("Copy Output to Clipboard", "复制输出到剪贴板"),
                "y",
                false,
                false,
                ContextMenuAction::PreviewCopy,
            ),
            Self::menu_entry(
                "⌕",
                tr("Search Preview Output", "搜索预览输出"),
                "f",
                false,
                false,
                ContextMenuAction::PreviewSearch,
            ),
            Self::menu_entry(
                "📄",
                tr("Export Scrollback Log", "导出终端日志"),
                "",
                false,
                true,
                ContextMenuAction::PreviewExportLog,
            ),
            Self::menu_entry(
                "●",
                tr("Toggle Live Pipe-Pane Log", "切换实时 Pipe 日志"),
                "",
                false,
                false,
                ContextMenuAction::PreviewTogglePipe,
            ),
            Self::menu_entry(
                "⌫",
                tr("Clear Pane History", "清空窗格滚动历史"),
                "",
                false,
                true,
                ContextMenuAction::PreviewClearHistory,
            ),
        ]);
        self.desktop.context_menu = Some(ContextMenuState {
            title: tr("Preview Actions", "终端预览操作").into(),
            anchor,
            items,
            selected: 0,
            dim_backdrop: true,
            from_menubar: false,
        });
    }

    pub(super) fn open_background_context_menu(&mut self, anchor: (u16, u16)) {
        let zh = self.theme.language.is_zh();
        let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
        let items = vec![
            Self::menu_entry(
                "+",
                tr("New Session", "新建会话"),
                "/new",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::New),
            ),
            Self::menu_entry(
                "⊞",
                tr("Mission Control / Expose", "全景监控墙 Expose"),
                "F3",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Expose),
            ),
            Self::menu_entry(
                "◫",
                tr("Split Studio", "分屏工作台"),
                "/split",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Split),
            ),
            Self::menu_entry(
                "▦",
                tr("Layout Studio", "布局与尺寸"),
                "/layout",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Layout),
            ),
            Self::menu_entry(
                "💾",
                tr("Workspace Snapshots", "工作区快照"),
                "/snapshot",
                false,
                true,
                ContextMenuAction::RunCommand(CommandKind::Snapshot),
            ),
            Self::menu_entry(
                "🎨",
                tr("Theme Picker", "配色主题"),
                "/theme",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Theme),
            ),
            Self::menu_entry(
                "⚙",
                tr("Settings & tmux.conf", "设置与 tmux.conf"),
                "/config",
                false,
                false,
                ContextMenuAction::RunCommand(CommandKind::Config),
            ),
        ];
        self.desktop.context_menu = Some(ContextMenuState {
            title: tr("Workspace Actions", "工作区快捷菜单").into(),
            anchor,
            items,
            selected: 0,
            dim_backdrop: true,
            from_menubar: false,
        });
    }

    pub(super) fn handle_context_menu_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut menu) = self.desktop.context_menu.take() else {
            return false;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let no_ctrl_alt = !ctrl && !alt;
        let len = menu.items.len();
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up => {
                menu.selected = wrap_step(menu.selected, len, -1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Char('p') if ctrl && !alt => {
                menu.selected = wrap_step(menu.selected, len, -1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Char('k') if no_ctrl_alt => {
                menu.selected = wrap_step(menu.selected, len, -1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Down => {
                menu.selected = wrap_step(menu.selected, len, 1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Char('n') if ctrl && !alt => {
                menu.selected = wrap_step(menu.selected, len, 1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Char('j') if no_ctrl_alt => {
                menu.selected = wrap_step(menu.selected, len, 1);
                self.desktop.context_menu = Some(menu);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(entry) = menu.items.get(menu.selected) {
                    let action = entry.action.clone();
                    self.execute_context_menu_action(action);
                }
                true
            }
            _ => {
                self.desktop.context_menu = Some(menu);
                false
            }
        }
    }

    pub(super) fn execute_context_menu_action(&mut self, action: ContextMenuAction) {
        self.desktop.context_menu = None;
        match action {
            ContextMenuAction::Resource(res, act) => self.execute_resource_action(res, act),
            ContextMenuAction::RunCommand(kind) => {
                self.run_parsed_command(Ok(crate::commands::ParsedCommand {
                    command: kind,
                    args: Vec::new(),
                }));
            }
            ContextMenuAction::ReloadTmuxConf => self.reload_tmux_conf_action(),
            ContextMenuAction::TogglePip(pane_id) => {
                let label = self
                    .home_panes
                    .iter()
                    .find(|p| p.id == pane_id)
                    .map(|p| {
                        format!(
                            "{}:{}.{} {}",
                            p.session_name, p.window_index, p.index, p.current_command
                        )
                    })
                    .unwrap_or_else(|| pane_id.to_string());
                self.toggle_live_pip(Some((pane_id, label)));
            }
            ContextMenuAction::InlineRename(target) => self.open_inline_rename(target, None),
            ContextMenuAction::SplitAction(act) => {
                if let Some(View::Split(mut studio)) = self.views.pop() {
                    if act == SplitToolbarAction::ApplyAndEnter {
                        self.apply_split_studio_and_enter(studio);
                        return;
                    }
                    self.run_split_toolbar_action(&mut studio, act);
                    self.views.push(View::Split(studio));
                }
            }
            ContextMenuAction::SplitRemoveSlot(slot_id) => {
                if let Some(View::Split(mut studio)) = self.views.pop() {
                    if studio.remove_slot_by_id(slot_id) {
                        self.ensure_selected_split_preview(&mut studio);
                        self.push_toast(
                            ToastKind::Info,
                            self.theme.tr("Removed split slot", "已移除分屏槽位"),
                        );
                    }
                    self.views.push(View::Split(studio));
                }
            }
            ContextMenuAction::ExposeEnter(idx) => {
                if let Some(View::Expose(state)) = self.views.top()
                    && let Some(tile) = state.tiles.get(idx).cloned()
                {
                    self.enter(Target::Window(tile.window.session_id, tile.window.id));
                }
            }
            ContextMenuAction::ExposeZoom(idx) => {
                if let Some(View::Expose(mut state)) = self.views.pop() {
                    state.selected = idx.min(state.tiles.len().saturating_sub(1));
                    self.expose_key(state, KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
                }
            }
            ContextMenuAction::ExposeClose(idx) => {
                if let Some(View::Expose(mut state)) = self.views.pop() {
                    state.selected = idx.min(state.tiles.len().saturating_sub(1));
                    self.expose_key(state, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
                }
            }
            ContextMenuAction::PreviewCopy => self.copy_current_preview_to_clipboard(),
            ContextMenuAction::PreviewSearch => self.open_preview_search(),
            ContextMenuAction::PreviewExportLog => self.export_current_preview_log(),
            ContextMenuAction::PreviewTogglePipe => self.toggle_current_preview_pipe(),
            ContextMenuAction::PreviewClearHistory => {
                if let Some(pane) = self.focused_or_active_pane() {
                    match self.tmux.clear_pane_history(&pane.id) {
                        Ok(()) => {
                            self.refresh_home_preview();
                            self.push_toast(
                                ToastKind::Success,
                                self.theme.tr("✦ Cleared pane history", "✦ 已清空窗格历史"),
                            );
                        }
                        Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Desktop Mouse Dispatch: Move/Hover, Down(Left|Right|Middle), Drag, Up
    // -------------------------------------------------------------------------

    /// Full desktop mouse dispatcher returning `true` ONLY when visible state or hover target changed.
    pub(super) fn handle_desktop_mouse(&mut self, mouse: MouseEvent) -> bool {
        let (col, row) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Moved => self.handle_mouse_moved(col, row),
            MouseEventKind::ScrollUp => {
                if let Some(menu) = self.desktop.context_menu.as_mut() {
                    let len = menu.items.len();
                    menu.selected = wrap_step(menu.selected, len, -1);
                    return true;
                }
                if self.desktop.inline_rename.is_some() {
                    return false;
                }
                self.mouse_scroll(col, row, true);
                true
            }
            MouseEventKind::ScrollDown => {
                if let Some(menu) = self.desktop.context_menu.as_mut() {
                    let len = menu.items.len();
                    menu.selected = wrap_step(menu.selected, len, 1);
                    return true;
                }
                if self.desktop.inline_rename.is_some() {
                    return false;
                }
                self.mouse_scroll(col, row, false);
                true
            }
            MouseEventKind::Down(MouseButton::Right) => self.handle_mouse_right_click(col, row),
            MouseEventKind::Down(MouseButton::Middle) => self.handle_mouse_middle_click(col, row),
            MouseEventKind::Down(MouseButton::Left) => self.handle_mouse_left_down(col, row),
            MouseEventKind::Drag(MouseButton::Left) => self.handle_mouse_left_drag(col, row),
            MouseEventKind::Up(MouseButton::Left) => self.handle_mouse_left_up(col, row),
            _ => false,
        }
    }

    fn compute_hover(&self, col: u16, row: u16) -> (HoverTarget, Option<String>) {
        let zh = self.theme.language.is_zh();
        let tr = |en: &str, zh_s: &str| if zh { zh_s.to_owned() } else { en.to_owned() };
        let pt = (col, row).into();

        if self.desktop.context_menu.is_some() {
            if let Some((_, idx)) = self
                .desktop
                .overlay_layout
                .menu_item_hits
                .iter()
                .find(|(rect, _)| rect.contains(pt))
            {
                return (HoverTarget::ContextMenuItem(*idx), None);
            }
            return (HoverTarget::None, None);
        }

        if let Some((_, item)) = self
            .desktop
            .menubar_hits
            .iter()
            .find(|(rect, _)| rect.contains(pt))
            .copied()
        {
            let tip = match item {
                MenubarItem::TabStudio => tr("Home Session×Window Studio", "主页会话矩阵工作台"),
                MenubarItem::TabExpose => tr(
                    "Mission Control live wall (F3 / Ctrl+G)",
                    "多窗口实时全景墙 (F3 / Ctrl+G)",
                ),
                MenubarItem::TabSplit => tr(
                    "2×2 Visual Split Studio (/split)",
                    "2×2 可视化分屏工作台 (/split)",
                ),
                MenubarItem::TabLayout => tr(
                    "Layout presets & resize (/layout)",
                    "布局预设与尺寸调整 (/layout)",
                ),
                MenubarItem::MenuDropdown => tr("Open workspace tools menu", "打开工作区工具菜单"),
                MenubarItem::PipToggle => tr(
                    "Toggle Live PIP direct terminal input",
                    "切换 Live PIP 实时直控输入",
                ),
                MenubarItem::ThemeCycle => {
                    tr("Cycle color theme (Ctrl+T)", "切换配色主题 (Ctrl+T)")
                }
                MenubarItem::LanguageToggle => {
                    tr("Switch English / 中文 instantly", "一键切换中文 / English")
                }
                MenubarItem::Config => tr(
                    "Open Settings & ~/.tmux.conf (/config)",
                    "打开设置与 ~/.tmux.conf (/config)",
                ),
                MenubarItem::Detach => tr(
                    "Detach current tmux client (/exit)",
                    "分离当前 tmux 客户端 (/exit)",
                ),
            };
            return (HoverTarget::Menubar(item), Some(tip));
        }

        if let Some((_, sash_id)) = self
            .desktop
            .sash_hits
            .iter()
            .find(|(rect, _)| rect.contains(pt))
            .copied()
        {
            let tip = tr("Drag divider to resize", "拖动分隔线实时调整比例");
            return (HoverTarget::Sash(sash_id), Some(tip));
        }

        if let Some((_, action)) = self
            .preview_action_hits
            .iter()
            .find(|(rect, _)| rect.contains(pt))
            .copied()
        {
            let tip = match action {
                PreviewAction::PrevPane => tr("Previous pane ([)", "上一个窗格 ([)"),
                PreviewAction::NextPane => tr("Next pane (])", "下一个窗格 (])"),
                PreviewAction::Search => tr("Search preview output (f)", "搜索预览内容 (f)"),
                PreviewAction::Copy => tr("Copy pane output (y)", "复制窗格内容 (y)"),
                PreviewAction::ExportLog => {
                    tr("Export scrollback log to file", "导出滚动历史日志到文件")
                }
                PreviewAction::TogglePipe => {
                    tr("Toggle live pipe-pane logging", "开关实时 pipe-pane 日志")
                }
                PreviewAction::TogglePip => {
                    tr("Toggle Live PIP direct input", "切换 Live PIP 实时直控")
                }
                PreviewAction::SendEsc => {
                    tr("Send literal Esc to live pane", "向直控窗格发送 Esc 键")
                }
                PreviewAction::Zoom => tr("Zoom / unzoom pane", "全屏缩放 / 还原窗格"),
                PreviewAction::Rename => tr("Inline rename (F2)", "行内重命名 (F2)"),
                PreviewAction::Close => {
                    tr("Kill target (requires yes)", "关闭目标（需输入 yes 确认）")
                }
            };
            return (HoverTarget::PreviewAction(action), Some(tip));
        }

        if self
            .desktop
            .preview_layout
            .scrollbar_track_rect
            .is_some_and(|r| r.contains(pt))
        {
            return (
                HoverTarget::PreviewScrollbar,
                Some(tr("Click or drag scrollbar", "点击或拖动滚动条")),
            );
        }

        if let Some((_, action)) = self
            .controller_hits
            .iter()
            .find(|(rect, _)| rect.contains(pt))
            .copied()
        {
            return (HoverTarget::Controller(action), None);
        }

        match self.views.top() {
            None => {
                if let Some((_, quick)) = self
                    .quick_action_hits
                    .iter()
                    .find(|(rect, _)| rect.contains(pt))
                    .copied()
                {
                    return (HoverTarget::QuickAction(quick), None);
                }
                if self.composer_mode == ComposerMode::CommandJump {
                    if let Some((_, action)) = self
                        .tab_hits
                        .iter()
                        .find(|(rect, _)| rect.contains(pt))
                        .copied()
                    {
                        return (HoverTarget::MatrixTab(action), None);
                    }
                    if let Some(hit) = self.matrix_hits.iter().find(|h| h.rect.contains(pt)) {
                        let tip = match self.matrix.resolve_hit(&hit.target) {
                            Some(MatrixSelection::Window(_, _)) => Some(tr(
                                "Double-click enter · Drag to move/dock · Right-click menu",
                                "双击进入 · 拖拽移动/分屏 · 右键菜单",
                            )),
                            Some(MatrixSelection::Session(_)) => Some(tr(
                                "Double-click enter · F2 rename · Right-click menu",
                                "双击进入 · F2 重命名 · 右键菜单",
                            )),
                            _ => None,
                        };
                        return (HoverTarget::MatrixCell(hit.target.clone()), tip);
                    }
                }
            }
            Some(View::Expose(_)) => {
                for hit in &self.desktop.expose_hits {
                    if let Some((_, ctrl)) = hit
                        .control_hits
                        .iter()
                        .find(|(r, _)| r.contains(pt))
                        .copied()
                    {
                        let tip = match ctrl {
                            TileControl::Pip => {
                                tr("Live PIP direct input (i)", "Live PIP 实时直控 (i)")
                            }
                            TileControl::Zoom => tr("Toggle pane zoom (z)", "切换窗格全屏缩放 (z)"),
                            TileControl::Rename => {
                                tr("Inline rename window (F2)", "行内重命名窗口 (F2)")
                            }
                            TileControl::Close => {
                                tr("Close window (yes confirm)", "关闭窗口（需 yes 确认）")
                            }
                        };
                        return (HoverTarget::ExposeControl(hit.tile_index, ctrl), Some(tip));
                    }
                    if hit.rect.contains(pt) {
                        return (HoverTarget::ExposeTile(hit.tile_index), None);
                    }
                }
            }
            Some(View::Split(studio)) => {
                if let Some((_, act)) = studio
                    .toolbar_hits
                    .iter()
                    .find(|(r, _)| r.contains(pt))
                    .copied()
                {
                    return (HoverTarget::SplitToolbar(act), None);
                }
                if let Some((_, slot_id)) = studio
                    .slot_hits
                    .iter()
                    .find(|(r, _)| r.contains(pt))
                    .copied()
                {
                    return (HoverTarget::SplitSlot(slot_id), None);
                }
            }
            _ => {}
        }

        (HoverTarget::None, None)
    }

    fn handle_mouse_moved(&mut self, col: u16, row: u16) -> bool {
        let (next_hover, next_tip) = self.compute_hover(col, row);
        let mut changed = false;
        if let HoverTarget::ContextMenuItem(idx) = next_hover
            && let Some(menu) = self.desktop.context_menu.as_mut()
            && menu.selected != idx
        {
            menu.selected = idx;
            changed = true;
        }
        if next_hover != self.desktop.hover || next_tip != self.desktop.tooltip_text {
            self.desktop.hover = next_hover;
            self.desktop.hover_pos = (col, row);
            self.desktop.tooltip_text = next_tip;
            changed = true;
        }
        changed
    }

    fn handle_mouse_right_click(&mut self, col: u16, row: u16) -> bool {
        let pt = (col, row).into();
        self.desktop.inline_rename = None;
        self.desktop.drag = None;

        // Right-clicking inside preview card opens preview actions menu.
        if self.preview_rect.is_some_and(|r| r.contains(pt)) {
            self.open_preview_context_menu((col, row));
            return true;
        }

        match self.views.top() {
            None => {
                if self.composer_mode == ComposerMode::CommandJump
                    && let Some(hit) = self
                        .matrix_hits
                        .iter()
                        .find(|h| h.rect.contains(pt))
                        .cloned()
                {
                    self.matrix.select_hit(&hit.target);
                    self.sync_action_scope_with_matrix();
                    self.refresh_home_preview();
                    match self.matrix.resolve_hit(&hit.target) {
                        Some(MatrixSelection::Session(s)) => {
                            let s = s.clone();
                            self.open_resource_context_menu(Resource::Session(s), (col, row));
                            return true;
                        }
                        Some(MatrixSelection::Window(_, w)) => {
                            let w = w.clone();
                            self.open_resource_context_menu(Resource::Window(w), (col, row));
                            return true;
                        }
                        _ => {}
                    }
                } else if self.composer_mode == ComposerMode::LiveRadar
                    && let Some(rect) = self.radar_list_rect
                    && rect.contains(pt)
                {
                    let idx = self.radar_offset + usize::from(row - rect.y);
                    if let Some(sess) = self.home_sessions.get(idx).cloned() {
                        self.radar_selected = idx;
                        self.refresh_radar_preview();
                        self.open_resource_context_menu(Resource::Session(sess), (col, row));
                        return true;
                    }
                }
                self.open_background_context_menu((col, row));
                true
            }
            Some(View::Picker(picker)) => {
                if let Some(rect) = self.picker_list_rect
                    && rect.contains(pt)
                {
                    let mut matcher = self.matcher.clone();
                    let rows = picker.rows(&mut matcher);
                    let idx = self.picker_offset + usize::from(row - rect.y);
                    if let Some(super::PickerRow::Resource(res_idx)) = rows.get(idx).copied()
                        && let Some(res) = picker.resources.get(res_idx).cloned()
                    {
                        self.open_resource_context_menu(res, (col, row));
                        return true;
                    }
                }
                self.open_background_context_menu((col, row));
                true
            }
            Some(View::Expose(state)) => {
                if let Some(hit) = self
                    .desktop
                    .expose_hits
                    .iter()
                    .find(|h| h.rect.contains(pt))
                    && let Some(tile) = state.tiles.get(hit.tile_index).cloned()
                {
                    let idx = hit.tile_index;
                    let zh = self.theme.language.is_zh();
                    let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
                    let mut items = Vec::new();
                    if let Some(pid) = tile.active_pane_id.clone() {
                        items.push(Self::menu_entry(
                            "⌨",
                            tr("Live PIP Type-Through", "Live PIP 实时直控"),
                            "i",
                            false,
                            false,
                            ContextMenuAction::TogglePip(pid),
                        ));
                    }
                    items.extend([
                        Self::menu_entry(
                            "▸",
                            tr("Open Window", "进入此窗口"),
                            "Enter",
                            false,
                            false,
                            ContextMenuAction::ExposeEnter(idx),
                        ),
                        Self::menu_entry(
                            "⛶",
                            tr("Toggle Zoom", "切换全屏缩放"),
                            "z",
                            false,
                            false,
                            ContextMenuAction::ExposeZoom(idx),
                        ),
                        Self::menu_entry(
                            "✎",
                            tr("Inline Rename", "行内重命名"),
                            "F2",
                            false,
                            false,
                            ContextMenuAction::InlineRename(RenameTarget::Window(
                                tile.window.id.clone(),
                                tile.window.name.clone(),
                            )),
                        ),
                        Self::menu_entry(
                            "✕",
                            tr("Close Window (yes confirm)", "关闭窗口（需确认）"),
                            "x",
                            true,
                            true,
                            ContextMenuAction::ExposeClose(idx),
                        ),
                    ]);
                    self.desktop.context_menu = Some(ContextMenuState {
                        title: format!(
                            "{}:{} {}",
                            tile.window.session_name, tile.window.index, tile.window.name
                        ),
                        anchor: (col, row),
                        items,
                        selected: 0,
                        dim_backdrop: true,
                        from_menubar: false,
                    });
                    return true;
                }
                self.open_background_context_menu((col, row));
                true
            }
            Some(View::Split(studio)) => {
                if let Some((_, slot_id)) = studio
                    .slot_hits
                    .iter()
                    .find(|(r, _)| r.contains(pt))
                    .copied()
                {
                    let zh = self.theme.language.is_zh();
                    let tr = |en: &'static str, zh_s: &'static str| if zh { zh_s } else { en };
                    let items = vec![
                        Self::menu_entry(
                            "◫",
                            tr("Split Horizontally", "左右分屏"),
                            "h",
                            false,
                            false,
                            ContextMenuAction::SplitAction(SplitToolbarAction::SplitHorizontal),
                        ),
                        Self::menu_entry(
                            "⬓",
                            tr("Split Vertically", "上下分屏"),
                            "v",
                            false,
                            false,
                            ContextMenuAction::SplitAction(SplitToolbarAction::SplitVertical),
                        ),
                        Self::menu_entry(
                            "⇄",
                            tr("Flip Layout Axes", "翻转分屏方向"),
                            "t",
                            false,
                            false,
                            ContextMenuAction::SplitAction(SplitToolbarAction::FlipDirection),
                        ),
                        Self::menu_entry(
                            "⚡",
                            tr("Toggle Synchronize-Panes", "切换同步输入"),
                            "y",
                            false,
                            true,
                            ContextMenuAction::SplitAction(SplitToolbarAction::SyncToggle),
                        ),
                        Self::menu_entry(
                            "✕",
                            tr("Remove This Slot", "移除此分屏槽位"),
                            "Mid",
                            true,
                            true,
                            ContextMenuAction::SplitRemoveSlot(slot_id),
                        ),
                    ];
                    self.desktop.context_menu = Some(ContextMenuState {
                        title: format!("Split #{slot_id}"),
                        anchor: (col, row),
                        items,
                        selected: 0,
                        dim_backdrop: true,
                        from_menubar: false,
                    });
                    return true;
                }
                self.open_background_context_menu((col, row));
                true
            }
            _ => {
                self.open_background_context_menu((col, row));
                true
            }
        }
    }

    fn handle_mouse_middle_click(&mut self, col: u16, row: u16) -> bool {
        let pt = (col, row).into();
        self.desktop.context_menu = None;
        self.desktop.inline_rename = None;
        match self.views.pop() {
            None => {
                if self.composer_mode == ComposerMode::CommandJump
                    && let Some(hit) = self
                        .matrix_hits
                        .iter()
                        .find(|h| h.rect.contains(pt))
                        .cloned()
                {
                    match self.matrix.resolve_hit(&hit.target) {
                        Some(MatrixSelection::Session(s)) => {
                            let s = s.clone();
                            self.kill_confirm(Resource::Session(s));
                            return true;
                        }
                        Some(MatrixSelection::Window(_, w)) => {
                            let w = w.clone();
                            self.kill_confirm(Resource::Window(w));
                            return true;
                        }
                        _ => {}
                    }
                } else if self.composer_mode == ComposerMode::LiveRadar
                    && let Some(rect) = self.radar_list_rect
                    && rect.contains(pt)
                {
                    let idx = self.radar_offset + usize::from(row - rect.y);
                    if let Some(sess) = self.home_sessions.get(idx).cloned() {
                        self.kill_confirm(Resource::Session(sess));
                        return true;
                    }
                }
                false
            }
            Some(View::Picker(picker)) => {
                let mut target_res = None;
                if let Some(rect) = self.picker_list_rect
                    && rect.contains(pt)
                {
                    let rows = picker.rows(&mut self.matcher);
                    let idx = self.picker_offset + usize::from(row - rect.y);
                    if let Some(super::PickerRow::Resource(res_idx)) = rows.get(idx).copied() {
                        target_res = picker.resources.get(res_idx).cloned();
                    }
                }
                self.views.push(View::Picker(picker));
                if let Some(res) = target_res {
                    self.kill_confirm(res);
                    return true;
                }
                false
            }
            Some(View::Expose(state)) => {
                let target_win = self
                    .desktop
                    .expose_hits
                    .iter()
                    .find(|h| h.rect.contains(pt))
                    .and_then(|h| state.tiles.get(h.tile_index))
                    .map(|t| t.window.clone());
                self.views.push(View::Expose(state));
                if let Some(win) = target_win {
                    self.kill_confirm(Resource::Window(win));
                    return true;
                }
                false
            }
            Some(View::Split(mut studio)) => {
                let removed = if let Some((_, slot_id)) = studio
                    .slot_hits
                    .iter()
                    .find(|(r, _)| r.contains(pt))
                    .copied()
                {
                    let ok = studio.remove_slot_by_id(slot_id);
                    if ok {
                        self.ensure_selected_split_preview(&mut studio);
                    }
                    ok
                } else {
                    false
                };
                self.views.push(View::Split(studio));
                if removed {
                    self.push_toast(
                        ToastKind::Info,
                        self.theme.tr("Removed split slot", "已移除分屏槽位"),
                    );
                }
                removed
            }
            Some(other) => {
                self.views.push(other);
                false
            }
        }
    }

    fn handle_mouse_left_down(&mut self, col: u16, row: u16) -> bool {
        let pt = (col, row).into();

        // 1. Floating inline rename click-outside dismiss.
        if self.desktop.inline_rename.is_some() {
            if self
                .desktop
                .overlay_layout
                .rename_rect
                .is_some_and(|r| r.contains(pt))
            {
                return true;
            }
            self.desktop.inline_rename = None;
            return true;
        }

        // 2. Floating context menu click handling.
        if self.desktop.context_menu.is_some() {
            if let Some((_, idx)) = self
                .desktop
                .overlay_layout
                .menu_item_hits
                .iter()
                .find(|(r, _)| r.contains(pt))
                .copied()
            {
                if let Some(menu) = self.desktop.context_menu.take()
                    && let Some(entry) = menu.items.get(idx)
                {
                    let action = entry.action.clone();
                    self.execute_context_menu_action(action);
                }
                return true;
            }
            let was_menubar = self
                .desktop
                .context_menu
                .as_ref()
                .is_some_and(|m| m.from_menubar);
            self.desktop.context_menu = None;
            // If clicking the Menu toggle pill again, simply close it without reopening.
            if was_menubar
                && self
                    .desktop
                    .menubar_hits
                    .iter()
                    .any(|(r, item)| *item == MenubarItem::MenuDropdown && r.contains(pt))
            {
                return true;
            }
            // Clicking outside a context menu dismisses it without accidentally activating underlying UI.
            if !was_menubar {
                return true;
            }
        }

        // 3. Top IDE Menubar clicks.
        if let Some((rect, item)) = self
            .desktop
            .menubar_hits
            .iter()
            .find(|(r, _)| r.contains(pt))
            .copied()
        {
            if item == MenubarItem::TabSplit {
                self.open_split_from_menubar();
            } else {
                self.handle_menubar_click(item, rect);
            }
            return true;
        }

        // 4. Draggable Sash grab (Home divider, Split Studio sash, Layout Studio sash).
        if let Some((_, sash_id)) = self
            .desktop
            .sash_hits
            .iter()
            .find(|(r, _)| r.contains(pt))
            .copied()
        {
            let start_ratio = match sash_id {
                SashId::HomePreviewMatrix => {
                    if self.desktop.home_split_ratio == 0 {
                        DEFAULT_HOME_SPLIT_RATIO
                    } else {
                        self.desktop.home_split_ratio
                    }
                }
                SashId::SplitNode(idx) => {
                    if let Some(View::Split(studio)) = self.views.top() {
                        studio.ratio_at_heap_index(idx).map_or(50, |(_, r)| r)
                    } else {
                        50
                    }
                }
                SashId::LayoutResize(_) => 50,
            };
            self.desktop.last_click = None;
            self.desktop.drag = Some(DragState {
                origin: (col, row),
                cursor: (col, row),
                active: true,
                kind: DragKind::Sash {
                    id: sash_id,
                    start_col: col,
                    start_row: row,
                    start_ratio,
                    last_step_col: col,
                    last_step_row: row,
                },
                hover_detail: String::new(),
                dock_target: None,
                split_dock_slot: None,
                expose_dock_tile: None,
            });
            return true;
        }

        // 5. Preview action pills (including `[⌨]`, `[Send Esc]`, `[⛶]`, `[✎]`, `[✕]`).
        if let Some((_, action)) = self
            .preview_action_hits
            .iter()
            .find(|(r, _)| r.contains(pt))
            .copied()
        {
            self.handle_preview_action(action);
            return true;
        }

        // 6. VS Code Scrollbar click / drag on preview card.
        if let Some(track) = self.desktop.preview_layout.scrollbar_track_rect
            && track.contains(pt)
        {
            self.desktop.last_click = None;
            self.apply_scrollbar_position(track, row);
            self.desktop.drag = Some(DragState {
                origin: (col, row),
                cursor: (col, row),
                active: true,
                kind: DragKind::Scrollbar { track },
                hover_detail: String::new(),
                dock_target: None,
                split_dock_slot: None,
                expose_dock_tile: None,
            });
            return true;
        }

        // 7. Preview interior text click -> begin potential drag-to-select copy.
        if let Some(content) = self.desktop.preview_layout.content_rect
            && content.contains(pt)
            && self.cached_preview.is_some()
        {
            let pos = (usize::from(row - content.y), usize::from(col - content.x));
            self.desktop.preview_selection = None;
            self.desktop.drag = Some(DragState {
                origin: (col, row),
                cursor: (col, row),
                active: false,
                kind: DragKind::PreviewSelect {
                    start: pos,
                    current: pos,
                },
                hover_detail: String::new(),
                dock_target: None,
                split_dock_slot: None,
                expose_dock_tile: None,
            });
            return true;
        }

        // 8. Controller buttons (Layout presets, Clients, Buffers, Snapshots).
        if let Some((_, action)) = self
            .controller_hits
            .iter()
            .find(|(r, _)| r.contains(pt))
            .copied()
        {
            self.handle_controller_action(action);
            return true;
        }

        // 9. Per-view left click + double-click + drag arming.
        match self.views.pop() {
            None => self.handle_home_left_down(col, row),
            Some(View::Expose(mut state)) => {
                let mut handled = false;
                if let Some(hit) = self
                    .desktop
                    .expose_hits
                    .iter()
                    .find(|h| h.rect.contains(pt))
                    .cloned()
                {
                    if let Some((_, ctrl)) = hit
                        .control_hits
                        .iter()
                        .find(|(r, _)| r.contains(pt))
                        .copied()
                    {
                        state.selected = hit.tile_index.min(state.tiles.len().saturating_sub(1));
                        let tile = state.tiles.get(state.selected).cloned();
                        match ctrl {
                            TileControl::Pip => {
                                self.views.push(View::Expose(state));
                                if let Some(t) = tile
                                    && let Some(pid) = t.active_pane_id.or_else(|| {
                                        self.representative_pane_for_window(&t.window.id)
                                            .map(|p| p.id)
                                    })
                                {
                                    let lbl = format!(
                                        "{}:{} {}",
                                        t.window.session_name, t.window.index, t.window.name
                                    );
                                    self.toggle_live_pip(Some((pid, lbl)));
                                }
                                return true;
                            }
                            TileControl::Zoom => {
                                self.expose_key(
                                    state,
                                    KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
                                );
                                return true;
                            }
                            TileControl::Rename => {
                                self.views.push(View::Expose(state));
                                if let Some(t) = tile {
                                    self.open_inline_rename(
                                        RenameTarget::Window(t.window.id, t.window.name),
                                        Some((col, row)),
                                    );
                                }
                                return true;
                            }
                            TileControl::Close => {
                                self.views.push(View::Expose(state));
                                if let Some(t) = tile {
                                    self.kill_confirm(Resource::Window(t.window));
                                }
                                return true;
                            }
                        }
                    }
                    state.selected = hit.tile_index.min(state.tiles.len().saturating_sub(1));
                    let dbl = self
                        .desktop
                        .record_click(ClickableItem::ExposeTile(hit.tile_index));
                    if dbl {
                        if let Some(tile) = state.tiles.get(hit.tile_index).cloned() {
                            self.views.push(View::Expose(state));
                            self.enter(Target::Window(tile.window.session_id, tile.window.id));
                            return true;
                        }
                    } else if let Some(tile) = state.tiles.get(hit.tile_index).cloned() {
                        let label = format!(
                            "{}:{} {}",
                            tile.window.session_name, tile.window.index, tile.window.name
                        );
                        self.desktop.drag = Some(DragState {
                            origin: (col, row),
                            cursor: (col, row),
                            active: false,
                            kind: DragKind::ExposeTile {
                                tile_index: hit.tile_index,
                                window: tile.window,
                                pane_id: tile.active_pane_id,
                                label,
                            },
                            hover_detail: String::new(),
                            dock_target: None,
                            split_dock_slot: None,
                            expose_dock_tile: None,
                        });
                    }
                    handled = true;
                }
                self.views.push(View::Expose(state));
                handled
            }
            Some(View::Split(mut studio)) => {
                if let Some((_, action)) = studio
                    .toolbar_hits
                    .iter()
                    .find(|(rect, _)| rect.contains(pt))
                    .copied()
                {
                    if action == SplitToolbarAction::ApplyAndEnter {
                        self.apply_split_studio_and_enter(studio);
                        return true;
                    }
                    self.run_split_toolbar_action(&mut studio, action);
                    self.views.push(View::Split(studio));
                    return true;
                }
                if let Some((_, slot_id)) = studio
                    .slot_hits
                    .iter()
                    .find(|(rect, _)| rect.contains(pt))
                    .copied()
                {
                    let _ = self.desktop.record_click(ClickableItem::SplitSlot(slot_id));
                    if studio.select_slot(slot_id) {
                        self.ensure_selected_split_preview(&mut studio);
                    }
                    let ord = studio.slot_ordinal(slot_id).unwrap_or(slot_id);
                    self.desktop.drag = Some(DragState {
                        origin: (col, row),
                        cursor: (col, row),
                        active: false,
                        kind: DragKind::SplitSlot {
                            slot_id,
                            label: format!("Slot #{ord}"),
                        },
                        hover_detail: String::new(),
                        dock_target: None,
                        split_dock_slot: None,
                        expose_dock_tile: None,
                    });
                    self.views.push(View::Split(studio));
                    return true;
                }
                if let Some(rect) = studio.window_list_rect
                    && rect.contains(pt)
                {
                    let clicked = Self::list_scroll_offset(studio.window_cursor, rect.height)
                        + usize::from(row - rect.y);
                    let _ = self
                        .desktop
                        .record_click(ClickableItem::SplitWindow(clicked));
                    if studio.assign_cursor_row(clicked) {
                        self.ensure_selected_split_preview(&mut studio);
                    }
                    self.views.push(View::Split(studio));
                    return true;
                }
                self.views.push(View::Split(studio));
                false
            }
            Some(View::Picker(mut picker)) => {
                let mut changed = false;
                if let Some(rect) = self.picker_list_rect
                    && rect.contains(pt)
                {
                    let rows = picker.rows(&mut self.matcher);
                    let clicked = self.picker_offset + usize::from(row - rect.y);
                    if let Some(target_row) = rows.get(clicked).copied() {
                        let dbl = self.desktop.record_click(ClickableItem::Picker(clicked));
                        if dbl {
                            self.activate_picker_row(picker, target_row);
                            return true;
                        }
                        picker.selected = clicked;
                        self.update_picker_preview(&picker);
                        changed = true;
                    }
                }
                self.views.push(View::Picker(picker));
                changed
            }
            Some(View::TargetPick(mut state)) => {
                let mut changed = false;
                if let Some(rect) = self.picker_list_rect
                    && rect.contains(pt)
                {
                    let rows = state.filtered_indices(&mut self.matcher);
                    let clicked = self.picker_offset + usize::from(row - rect.y);
                    if let Some((res_idx, _)) = rows.get(clicked) {
                        let res_idx = *res_idx;
                        let dbl = self
                            .desktop
                            .record_click(ClickableItem::TargetPick(clicked));
                        if dbl && let Some(chosen) = state.resources.get(res_idx).cloned() {
                            self.execute_target_pick_choice(state.purpose, chosen);
                            return true;
                        }
                        state.selected = clicked;
                        self.update_target_pick_preview(&state);
                        changed = true;
                    }
                }
                self.views.push(View::TargetPick(state));
                changed
            }
            Some(View::Actions(resource, mut selected)) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let actions = resource.actions_for(&self.piping_panes, self.theme.language);
                    let clicked =
                        Self::list_scroll_offset(selected, rect.height) + usize::from(row - rect.y);
                    if let Some((_, action)) = actions.get(clicked) {
                        let action = *action;
                        let dbl = self.desktop.record_click(ClickableItem::Action(clicked));
                        if dbl || clicked == selected {
                            self.execute_resource_action(resource, action);
                            return true;
                        }
                        selected = clicked;
                        changed = true;
                    }
                }
                self.views.push(View::Actions(resource, selected));
                changed
            }
            Some(View::Clients(mut state)) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let clicked = Self::list_scroll_offset(state.selected, rect.height)
                        + usize::from(row - rect.y);
                    if clicked < state.clients.len() {
                        let _ = self.desktop.record_click(ClickableItem::Client(clicked));
                        state.selected = clicked;
                        changed = true;
                    }
                }
                self.views.push(View::Clients(state));
                changed
            }
            Some(View::Buffers(mut state)) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let clicked = Self::list_scroll_offset(state.selected, rect.height)
                        + usize::from(row - rect.y);
                    if clicked < state.buffers.len() {
                        let _ = self.desktop.record_click(ClickableItem::Buffer(clicked));
                        state.selected = clicked;
                        self.refresh_selected_buffer_preview(&mut state);
                        changed = true;
                    }
                }
                self.views.push(View::Buffers(state));
                changed
            }
            Some(View::Snapshots(mut state)) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let clicked = Self::list_scroll_offset(state.selected, rect.height)
                        + usize::from(row - rect.y);
                    if clicked < state.snapshots.len() {
                        let _ = self.desktop.record_click(ClickableItem::Snapshot(clicked));
                        state.selected = clicked;
                        self.refresh_selected_snapshot_preview(&mut state);
                        changed = true;
                    }
                }
                self.views.push(View::Snapshots(state));
                changed
            }
            Some(View::Config { mut selected }) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let clicked =
                        Self::list_scroll_offset(selected, rect.height) + usize::from(row - rect.y);
                    if clicked < super::CONFIG_ITEM_COUNT {
                        let dbl = self.desktop.record_click(ClickableItem::Config(clicked));
                        if clicked == selected || dbl {
                            self.cycle_config_setting(clicked, true);
                        } else {
                            selected = clicked;
                            self.config_note = None;
                        }
                        changed = true;
                    }
                }
                self.views.push(View::Config { selected });
                changed
            }
            Some(View::ThemePicker {
                initial,
                mut selected,
            }) => {
                let mut changed = false;
                if let Some(rect) = self.subview_list_rect
                    && rect.contains(pt)
                {
                    let clicked =
                        Self::list_scroll_offset(selected, rect.height) + usize::from(row - rect.y);
                    if let Some(&preset) = crate::ui::ThemePreset::ALL.get(clicked) {
                        let dbl = self.desktop.record_click(ClickableItem::Theme(clicked));
                        if dbl {
                            self.theme = self.theme.with_preset(preset);
                            self.config.theme = preset;
                            self.push_toast(
                                ToastKind::Success,
                                theme_toast(preset, self.theme.language),
                            );
                            return true;
                        }
                        selected = clicked;
                        self.theme = self.theme.with_preset(preset);
                        changed = true;
                    }
                }
                self.views.push(View::ThemePicker { initial, selected });
                changed
            }
            Some(View::DualForm(mut form)) => {
                if form.field_hits[0].is_some_and(|r| r.contains(pt)) {
                    form.completion.close();
                    form.active_field = 0;
                    if !form.name_manually_edited {
                        let v = form.first_input.value().to_owned();
                        form.first_input.set_value_selected(v);
                    }
                    self.views.push(View::DualForm(form));
                    return true;
                }
                if form.field_hits[1].is_some_and(|r| r.contains(pt)) {
                    form.active_field = 1;
                    self.views.push(View::DualForm(form));
                    return true;
                }
                if form.completion.open
                    && let Some(rect) = form.completion_list_rect
                    && rect.contains(pt)
                {
                    let clicked = form.completion_offset + usize::from(row - rect.y);
                    if let Some(entry) = form.completion.entries.get(clicked).cloned() {
                        let dbl = self
                            .desktop
                            .record_click(ClickableItem::DualFormCompletion(clicked));
                        form.active_field = 1;
                        form.completion.selected = clicked;
                        form.completion.previewing = true;
                        form.second_input.set_value(&entry.full_input);
                        form.sync_default_name_from_dir();
                        if dbl {
                            self.drill_into_dual_form_completion(&mut form);
                        }
                        self.views.push(View::DualForm(form));
                        return true;
                    }
                }
                self.views.push(View::DualForm(form));
                false
            }
            Some(other) => {
                self.views.push(other);
                false
            }
        }
    }

    fn handle_home_left_down(&mut self, col: u16, row: u16) -> bool {
        let pt = (col, row).into();
        if let Some((_, quick)) = self
            .quick_action_hits
            .iter()
            .find(|(rect, _)| rect.contains(pt))
            .copied()
        {
            self.handle_home_quick_action(quick);
            return true;
        }
        if self.composer_mode == ComposerMode::LiveRadar
            && let Some(rect) = self.radar_list_rect
            && rect.contains(pt)
        {
            let clicked = self.radar_offset + usize::from(row - rect.y);
            if let Some(session) = self.home_sessions.get(clicked).cloned() {
                let dbl = self
                    .desktop
                    .record_click(ClickableItem::Radar(session.id.clone()));
                if dbl {
                    self.status.clear();
                    self.enter(Target::Session(session.id));
                } else {
                    self.radar_selected = clicked;
                    self.refresh_radar_preview();
                }
                return true;
            }
            return false;
        }
        if self.composer_mode == ComposerMode::CommandJump {
            if let Some((_, action)) = self
                .tab_hits
                .iter()
                .find(|(rect, _)| rect.contains(pt))
                .copied()
            {
                let scope = if self.matrix.is_session_selected() {
                    TargetScope::Session
                } else {
                    TargetScope::Window
                };
                self.matrix_action = match action {
                    MatrixAction::Rename(_) => MatrixAction::Rename(scope),
                    MatrixAction::Kill(_) => MatrixAction::Kill(scope),
                    other => other,
                };
                self.status.clear();
                return true;
            }
            if let Some(hit) = self
                .matrix_hits
                .iter()
                .find(|hit| hit.rect.contains(pt))
                .cloned()
            {
                let previous_preview_pane = self.focused_or_active_pane().map(|p| p.id);
                let dbl = self
                    .desktop
                    .record_click(ClickableItem::Matrix(hit.target.clone()));
                let _already_selected = self.matrix.select_hit(&hit.target);
                self.sync_action_scope_with_matrix();
                match &hit.target {
                    MatrixHitTarget::NewWindow(_) | MatrixHitTarget::NewSession => {
                        self.activate_matrix_selection();
                    }
                    MatrixHitTarget::Session(_) => {
                        if dbl
                            && matches!(
                                self.matrix_action,
                                MatrixAction::Open
                                    | MatrixAction::Inspect
                                    | MatrixAction::Split
                                    | MatrixAction::Rename(_)
                                    | MatrixAction::Kill(_)
                            )
                        {
                            self.activate_matrix_selection();
                        } else {
                            self.refresh_home_preview();
                        }
                    }
                    MatrixHitTarget::Window { .. } => {
                        if dbl {
                            self.activate_matrix_selection();
                        } else {
                            self.refresh_home_preview();
                            if let Some(MatrixSelection::Window(_, win)) =
                                self.matrix.resolve_hit(&hit.target)
                            {
                                let win = win.clone();
                                let pane_id =
                                    self.representative_pane_for_window(&win.id).map(|p| p.id);
                                let label =
                                    format!("{}:{} {}", win.session_name, win.index, win.name);
                                self.desktop.drag = Some(DragState {
                                    origin: (col, row),
                                    cursor: (col, row),
                                    active: false,
                                    kind: DragKind::MatrixWindow {
                                        window: win,
                                        pane_id,
                                        previous_preview_pane,
                                        label,
                                    },
                                    hover_detail: String::new(),
                                    dock_target: None,
                                    split_dock_slot: None,
                                    expose_dock_tile: None,
                                });
                            }
                        }
                    }
                }
                return true;
            }
        }
        false
    }

    fn apply_scrollbar_position(&mut self, track: Rect, row: u16) {
        let Some((_, _, lines)) = &self.cached_preview else {
            return;
        };
        let visible_rows = self.preview_visible_rows();
        let max_scroll = lines.len().saturating_sub(visible_rows);
        if max_scroll == 0 {
            self.preview_scroll = 0;
            return;
        }
        let clamped_row = row.clamp(track.y, track.bottom().saturating_sub(1));
        let span = track.height.saturating_sub(1).max(1);
        let frac = f32::from(clamped_row.saturating_sub(track.y)) / f32::from(span);
        self.preview_scroll = ((1.0 - frac) * (max_scroll as f32)).round() as usize;
        self.preview_scroll = self.preview_scroll.min(max_scroll);
    }

    fn handle_mouse_left_drag(&mut self, col: u16, row: u16) -> bool {
        let Some(mut drag) = self.desktop.drag.take() else {
            return false;
        };
        drag.cursor = (col, row);
        let dist = drag.origin.0.abs_diff(col).max(drag.origin.1.abs_diff(row));
        if !drag.active && dist >= DRAG_THRESHOLD_CELLS {
            drag.active = true;
            self.desktop.last_click = None;
        }

        let zh = self.theme.language.is_zh();
        let tr = |en: &str, zh_s: &str| if zh { zh_s.to_owned() } else { en.to_owned() };

        match &mut drag.kind {
            DragKind::Sash {
                id,
                start_col,
                start_row,
                start_ratio,
                last_step_col,
                last_step_row,
            } => {
                let id = *id;
                match id {
                    SashId::HomePreviewMatrix => {
                        let total_h = self
                            .desktop
                            .last_frame_area
                            .height
                            .saturating_sub(6)
                            .max(10);
                        let dy = i32::from(row) - i32::from(*start_row);
                        let delta_pct = (dy * 100) / i32::from(total_h);
                        let next = (i32::from(*start_ratio) + delta_pct).clamp(
                            i32::from(MIN_HOME_SPLIT_RATIO),
                            i32::from(MAX_HOME_SPLIT_RATIO),
                        ) as u16;
                        self.desktop.home_split_ratio = next;
                    }
                    SashId::SplitNode(heap_idx) => {
                        if let Some(View::Split(mut studio)) = self.views.pop() {
                            if let Some((dir, _)) = studio.ratio_at_heap_index(heap_idx) {
                                let (delta_cells, span) = match dir {
                                    SplitDirection::SideBySide => (
                                        i32::from(col) - i32::from(*start_col),
                                        self.desktop
                                            .last_frame_area
                                            .width
                                            .saturating_sub(6)
                                            .max(12),
                                    ),
                                    SplitDirection::Stacked => (
                                        i32::from(row) - i32::from(*start_row),
                                        self.desktop
                                            .last_frame_area
                                            .height
                                            .saturating_sub(12)
                                            .max(8),
                                    ),
                                };
                                let delta_pct = (delta_cells * 100) / i32::from(span);
                                let next =
                                    (i32::from(*start_ratio) + delta_pct).clamp(20, 80) as u16;
                                studio.set_ratio_at_heap_index(heap_idx, next);
                            }
                            self.views.push(View::Split(studio));
                        }
                    }
                    SashId::LayoutResize(horizontal) => {
                        let dx = i32::from(col) - i32::from(*last_step_col);
                        let dy = i32::from(row) - i32::from(*last_step_row);
                        let dir_opt = if horizontal {
                            if dx >= 2 {
                                *last_step_col = col;
                                Some(ResizeDirection::Right)
                            } else if dx <= -2 {
                                *last_step_col = col;
                                Some(ResizeDirection::Left)
                            } else {
                                None
                            }
                        } else if dy >= 2 {
                            *last_step_row = row;
                            Some(ResizeDirection::Down)
                        } else if dy <= -2 {
                            *last_step_row = row;
                            Some(ResizeDirection::Up)
                        } else {
                            None
                        };
                        if let Some(dir) = dir_opt
                            && let Some(View::LayoutStudio(mut state)) = self.views.pop()
                        {
                            self.resize_layout_pane(&mut state, dir);
                            self.views.push(View::LayoutStudio(state));
                        }
                    }
                }
                self.desktop.drag = Some(drag);
                true
            }
            DragKind::Scrollbar { track } => {
                let track = *track;
                self.apply_scrollbar_position(track, row);
                self.desktop.drag = Some(drag);
                true
            }
            DragKind::PreviewSelect { start, current } => {
                if let Some(content) = self.desktop.preview_layout.content_rect {
                    let clamped_row = row.clamp(content.y, content.bottom().saturating_sub(1));
                    let clamped_col = col.clamp(content.x, content.right().saturating_sub(1));
                    *current = (
                        usize::from(clamped_row - content.y),
                        usize::from(clamped_col - content.x),
                    );
                    if *current != *start {
                        drag.active = true;
                        self.desktop.preview_selection =
                            Some(normalize_selection(*start, *current));
                    }
                }
                self.desktop.drag = Some(drag);
                true
            }
            DragKind::MatrixWindow { window, .. } => {
                drag.dock_target = None;
                drag.hover_detail.clear();
                if drag.active {
                    let pt = (col, row).into();
                    if let Some(prev_rect) = self.preview_rect
                        && let Some(zone) = compute_dock_zone(prev_rect, col, row)
                    {
                        drag.dock_target = Some((prev_rect, zone));
                        drag.hover_detail = dock_zone_hint(zone, zh);
                    } else if let Some(hit) = self.matrix_hits.iter().find(|h| h.rect.contains(pt))
                    {
                        drag.hover_detail = match self.matrix.resolve_hit(&hit.target) {
                            Some(MatrixSelection::Session(s)) if s.id != window.session_id => {
                                if zh {
                                    format!("移动到会话 {}", s.name)
                                } else {
                                    format!("Move to session {}", s.name)
                                }
                            }
                            Some(MatrixSelection::NewSession) => {
                                tr("Create new session & move here", "新建会话并移入此窗口")
                            }
                            Some(MatrixSelection::Window(_, target_w))
                                if target_w.id != window.id
                                    && target_w.session_id == window.session_id =>
                            {
                                if zh {
                                    format!("与窗口 {} 互换位置", target_w.name)
                                } else {
                                    format!("Swap with window {}", target_w.name)
                                }
                            }
                            Some(MatrixSelection::Window(_, target_w))
                                if target_w.session_id != window.session_id =>
                            {
                                if zh {
                                    format!("移动到会话 {}", target_w.session_name)
                                } else {
                                    format!("Move to session {}", target_w.session_name)
                                }
                            }
                            _ => String::new(),
                        };
                    }
                }
                let redraw = drag.active;
                self.desktop.drag = Some(drag);
                redraw
            }
            DragKind::SplitSlot { slot_id, .. } => {
                let src_slot = *slot_id;
                drag.dock_target = None;
                drag.split_dock_slot = None;
                drag.hover_detail.clear();
                if drag.active
                    && let Some(View::Split(studio)) = self.views.top()
                    && let Some((rect, target_slot)) = studio
                        .slot_hits
                        .iter()
                        .find(|(r, sid)| *sid != src_slot && r.contains((col, row).into()))
                        .copied()
                    && let Some(zone) = compute_dock_zone(rect, col, row)
                {
                    drag.dock_target = Some((rect, zone));
                    drag.split_dock_slot = Some((target_slot, zone));
                    drag.hover_detail = dock_zone_hint(zone, zh);
                }
                let redraw = drag.active;
                self.desktop.drag = Some(drag);
                redraw
            }
            DragKind::ExposeTile { tile_index, .. } => {
                let src_idx = *tile_index;
                drag.dock_target = None;
                drag.expose_dock_tile = None;
                drag.hover_detail.clear();
                if drag.active
                    && let Some(hit) = self
                        .desktop
                        .expose_hits
                        .iter()
                        .find(|h| h.tile_index != src_idx && h.rect.contains((col, row).into()))
                        .cloned()
                    && let Some(zone) = compute_dock_zone(hit.rect, col, row)
                {
                    drag.dock_target = Some((hit.rect, zone));
                    drag.expose_dock_tile = Some((hit.tile_index, zone));
                    drag.hover_detail = dock_zone_hint(zone, zh);
                }
                let redraw = drag.active;
                self.desktop.drag = Some(drag);
                redraw
            }
        }
    }

    fn handle_mouse_left_up(&mut self, col: u16, row: u16) -> bool {
        let Some(drag) = self.desktop.drag.take() else {
            return false;
        };
        if !drag.active {
            return false;
        }
        let pt = (col, row).into();
        match drag.kind {
            DragKind::Sash { id, .. } => {
                if id == SashId::HomePreviewMatrix {
                    let msg = if self.theme.language.is_zh() {
                        format!("✦ 主页分栏比例已调整为 {}%", self.desktop.home_split_ratio)
                    } else {
                        format!("✦ Home split resized to {}%", self.desktop.home_split_ratio)
                    };
                    self.push_toast(ToastKind::Info, msg);
                }
                true
            }
            DragKind::Scrollbar { .. } => true,
            DragKind::PreviewSelect { start, current } => {
                let selection = normalize_selection(start, current);
                self.desktop.preview_selection = None;
                if selection.0 != selection.1
                    && let Some((_, _, lines)) = &self.cached_preview
                {
                    let visible_rows = self.preview_visible_rows();
                    let text = extract_selected_preview_text(
                        lines,
                        self.preview_scroll,
                        visible_rows,
                        selection,
                    );
                    if !text.is_empty() {
                        let chars = text.chars().count();
                        match clipboard::copy_to_clipboard(&text, Some(self.tmux)) {
                            Ok(_) => {
                                let msg = if self.theme.language.is_zh() {
                                    format!("✦ 已复制选中文本（{chars} 字符）")
                                } else {
                                    format!("✦ Copied selection ({chars} chars)")
                                };
                                self.push_toast(ToastKind::Success, msg);
                            }
                            Err(err) => self.push_toast(ToastKind::Error, err),
                        }
                    }
                }
                true
            }
            DragKind::MatrixWindow {
                window,
                pane_id,
                previous_preview_pane,
                ..
            } => {
                // 1. Drop onto Home Preview Card Edge Dock Zone -> join_pane_ordered!
                if let Some((prev_rect, zone)) = drag.dock_target
                    && prev_rect.contains(pt)
                {
                    if zone == DockZone::Center {
                        self.refresh_home_preview();
                        return true;
                    }
                    let src_pane = pane_id.or_else(|| {
                        self.representative_pane_for_window(&window.id)
                            .map(|p| p.id)
                    });
                    let dst_pane = previous_preview_pane
                        .filter(|d| Some(d) != src_pane.as_ref())
                        .or_else(|| {
                            self.home_panes
                                .iter()
                                .find(|p| {
                                    p.window_id != window.id && p.session_id == window.session_id
                                })
                                .map(|p| p.id.clone())
                        })
                        .or_else(|| self.focused_or_active_pane().map(|p| p.id));
                    if let (Some(src), Some(dst)) = (src_pane, dst_pane)
                        && src != dst
                    {
                        let (dir, before) = dock_zone_to_split(zone);
                        match self.tmux.join_pane_ordered(&src, &dst, dir, before) {
                            Ok(()) => {
                                self.refresh_home_data();
                                let msg = if self.theme.language.is_zh() {
                                    format!("✦ 已停靠分屏：{}", dock_zone_hint(zone, true))
                                } else {
                                    format!("✦ Docked pane: {}", dock_zone_hint(zone, false))
                                };
                                self.push_toast(ToastKind::Success, msg);
                            }
                            Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                        }
                    }
                    return true;
                }

                // 2. Drop onto Home Matrix cell (Session / Window / + New session).
                if let Some(hit) = self
                    .matrix_hits
                    .iter()
                    .find(|h| h.rect.contains(pt))
                    .cloned()
                {
                    match self.matrix.resolve_hit(&hit.target) {
                        Some(MatrixSelection::Session(target_s))
                            if target_s.id != window.session_id =>
                        {
                            let sid = target_s.id.clone();
                            let sname = target_s.name.clone();
                            match self.tmux.move_window(&window.id, &sid) {
                                Ok(()) => {
                                    self.refresh_home_data();
                                    let msg = if self.theme.language.is_zh() {
                                        format!("✦ 已移动窗口 {} 到会话 {sname}", window.name)
                                    } else {
                                        format!("✦ Moved window {} → {sname}", window.name)
                                    };
                                    self.push_toast(ToastKind::Success, msg);
                                }
                                Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                            }
                        }
                        Some(MatrixSelection::NewSession) => {
                            let prefix = sanitize_session_prefix(&window.name);
                            let base = format!("{prefix}-ws");
                            let mut candidate = base.clone();
                            let mut suffix = 2_usize;
                            while self.home_sessions.iter().any(|s| s.name == candidate) {
                                candidate = format!("{base}-{suffix}");
                                suffix += 1;
                            }
                            let cwd = std::env::current_dir()
                                .unwrap_or_else(|_| std::path::PathBuf::from("."));
                            match self.tmux.new_session(&candidate, &cwd) {
                                Ok(new_sess) => match self
                                    .tmux
                                    .move_window(&window.id, &new_sess.id)
                                {
                                    Ok(()) => {
                                        self.refresh_home_data();
                                        let msg = if self.theme.language.is_zh() {
                                            format!(
                                                "✦ 已新建会话 {candidate} 并移入窗口 {}",
                                                window.name
                                            )
                                        } else {
                                            format!(
                                                "✦ Created session {candidate} & moved {}",
                                                window.name
                                            )
                                        };
                                        self.push_toast(ToastKind::Success, msg);
                                    }
                                    Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                                },
                                Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                            }
                        }
                        Some(MatrixSelection::Window(_, target_w)) if target_w.id != window.id => {
                            let target_wid = target_w.id.clone();
                            let target_sid = target_w.session_id.clone();
                            let target_wname = target_w.name.clone();
                            let target_sname = target_w.session_name.clone();
                            if target_sid == window.session_id {
                                match self.tmux.swap_window(&window.id, &target_wid) {
                                    Ok(()) => {
                                        self.refresh_home_data();
                                        let msg = if self.theme.language.is_zh() {
                                            format!("✦ 已互换窗口 {} ⇄ {target_wname}", window.name)
                                        } else {
                                            format!("✦ Swapped {} ⇄ {target_wname}", window.name)
                                        };
                                        self.push_toast(ToastKind::Success, msg);
                                    }
                                    Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                                }
                            } else {
                                match self.tmux.move_window(&window.id, &target_sid) {
                                    Ok(()) => {
                                        self.refresh_home_data();
                                        let msg = if self.theme.language.is_zh() {
                                            format!(
                                                "✦ 已移动窗口 {} 到会话 {target_sname}",
                                                window.name
                                            )
                                        } else {
                                            format!("✦ Moved {} → {target_sname}", window.name)
                                        };
                                        self.push_toast(ToastKind::Success, msg);
                                    }
                                    Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                                }
                            }
                        }
                        _ => {}
                    }
                }
                true
            }
            DragKind::SplitSlot { slot_id, .. } => {
                if let Some((target_slot, zone)) = drag.split_dock_slot
                    && let Some(View::Split(mut studio)) = self.views.pop()
                {
                    let changed = if zone == DockZone::Center {
                        studio.swap_slots(slot_id, target_slot)
                    } else {
                        let (dir, before) = dock_zone_to_split(zone);
                        studio.dock_slot_onto(slot_id, target_slot, dir, before)
                    };
                    if changed {
                        self.ensure_selected_split_preview(&mut studio);
                        let msg = if zone == DockZone::Center {
                            self.theme.tr("✦ Swapped split slots", "✦ 已互换分屏槽位")
                        } else {
                            self.theme.tr("✦ Docked split slot", "✦ 已停靠调整分屏布局")
                        };
                        self.push_toast(ToastKind::Success, msg);
                    }
                    self.views.push(View::Split(studio));
                }
                true
            }
            DragKind::ExposeTile {
                tile_index,
                window,
                pane_id,
                ..
            } => {
                if let Some((target_idx, zone)) = drag.expose_dock_tile
                    && target_idx != tile_index
                    && let Some(View::Expose(mut state)) = self.views.pop()
                {
                    if let Some(target_tile) = state.tiles.get(target_idx).cloned() {
                        if zone == DockZone::Center {
                            let res = if target_tile.window.session_id == window.session_id {
                                self.tmux.swap_window(&window.id, &target_tile.window.id)
                            } else {
                                self.tmux
                                    .move_window(&window.id, &target_tile.window.session_id)
                            };
                            match res {
                                Ok(()) => {
                                    self.refresh_expose_state(&mut state);
                                    self.push_toast(
                                        ToastKind::Success,
                                        self.theme.tr("✦ Rearranged windows", "✦ 已重排窗口"),
                                    );
                                }
                                Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                            }
                        } else {
                            let src = pane_id.or_else(|| {
                                self.representative_pane_for_window(&window.id)
                                    .map(|p| p.id)
                            });
                            let dst = target_tile.active_pane_id.or_else(|| {
                                self.representative_pane_for_window(&target_tile.window.id)
                                    .map(|p| p.id)
                            });
                            if let (Some(src), Some(dst)) = (src, dst)
                                && src != dst
                            {
                                let (dir, before) = dock_zone_to_split(zone);
                                match self.tmux.join_pane_ordered(&src, &dst, dir, before) {
                                    Ok(()) => {
                                        self.refresh_expose_state(&mut state);
                                        let msg = if self.theme.language.is_zh() {
                                            format!("✦ 已停靠分屏：{}", dock_zone_hint(zone, true))
                                        } else {
                                            format!(
                                                "✦ Docked pane: {}",
                                                dock_zone_hint(zone, false)
                                            )
                                        };
                                        self.push_toast(ToastKind::Success, msg);
                                    }
                                    Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                                }
                            }
                        }
                    }
                    self.views.push(View::Expose(state));
                }
                true
            }
        }
    }

    // -------------------------------------------------------------------------
    // Floating Desktop Overlays Rendering
    // -------------------------------------------------------------------------

    pub(super) fn render_desktop_overlays_for_frame(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
    ) {
        let menu_items: Vec<ContextMenuItemView> = self
            .desktop
            .context_menu
            .as_ref()
            .map(|m| m.items.iter().map(|e| e.view.clone()).collect())
            .unwrap_or_default();
        let context_menu = self.desktop.context_menu.as_ref().map(|m| ContextMenuView {
            title: &m.title,
            anchor: m.anchor,
            items: &menu_items,
            selected: m.selected,
            dim_backdrop: m.dim_backdrop,
        });

        let drag_ghost = self.desktop.drag.as_ref().and_then(|d| {
            if !d.active {
                return None;
            }
            let label = match &d.kind {
                DragKind::MatrixWindow { label, .. }
                | DragKind::SplitSlot { label, .. }
                | DragKind::ExposeTile { label, .. } => label.as_str(),
                DragKind::Sash { .. }
                | DragKind::Scrollbar { .. }
                | DragKind::PreviewSelect { .. } => {
                    return None;
                }
            };
            Some(DragGhostView {
                label,
                detail: &d.hover_detail,
                cursor: d.cursor,
                dock_target: d.dock_target,
            })
        });

        let hover_tooltip = (context_menu.is_none() && drag_ghost.is_none())
            .then(|| {
                self.desktop
                    .tooltip_text
                    .as_deref()
                    .map(|text| HoverTooltipView {
                        text,
                        anchor: self.desktop.hover_pos,
                    })
            })
            .flatten();

        let toast = self
            .desktop
            .toast
            .as_ref()
            .filter(|t| !t.is_expired())
            .map(|t| ToastView {
                kind: t.kind,
                message: &t.message,
                progress: t.remaining_fraction(),
            });

        let rename_target_label = self
            .desktop
            .inline_rename
            .as_ref()
            .map(|r| r.target.target_label());
        let inline_rename = self
            .desktop
            .inline_rename
            .as_ref()
            .map(|r| InlineRenameView {
                title: r.target.title(self.theme.language.is_zh()),
                target_label: rename_target_label.as_deref().unwrap_or(""),
                input: &r.input,
                anchor: r.anchor,
            });

        if context_menu.is_none()
            && drag_ghost.is_none()
            && hover_tooltip.is_none()
            && toast.is_none()
            && inline_rename.is_none()
        {
            self.desktop.overlay_layout = DesktopOverlayLayout::default();
            return;
        }

        let spec = DesktopOverlaySpec {
            context_menu,
            drag_ghost,
            hover_tooltip,
            toast,
            inline_rename,
        };
        self.desktop.overlay_layout =
            crate::ui::render_desktop_overlays(frame, area, &spec, self.theme);
    }

    /// Resolve a quick `Window` or `Pane` action from new PreviewAction variants (`TogglePip`, `SendEsc`, `Zoom`, `Rename`, `Close`).
    pub(super) fn handle_desktop_preview_action(&mut self, action: PreviewAction) -> bool {
        match action {
            PreviewAction::TogglePip => {
                self.toggle_live_pip(None);
                true
            }
            PreviewAction::SendEsc => {
                self.send_literal_esc_to_pip();
                true
            }
            PreviewAction::Zoom => {
                if let Some(pane) = self.focused_or_active_pane() {
                    match self.tmux.toggle_zoom_pane(&pane.id) {
                        Ok(()) => {
                            self.refresh_home_data();
                            self.push_toast(
                                ToastKind::Success,
                                self.theme.tr("✦ Toggled pane zoom", "✦ 已切换窗格全屏缩放"),
                            );
                        }
                        Err(err) => self.push_toast(ToastKind::Error, err.to_string()),
                    }
                }
                true
            }
            PreviewAction::Rename => {
                if !self.open_inline_rename_for_focused()
                    && let Some(pane) = self.focused_or_active_pane()
                {
                    self.open_inline_rename(RenameTarget::PaneTitle(pane.id, pane.title), None);
                }
                true
            }
            PreviewAction::Close => {
                match self.preview_target.clone() {
                    Some(Target::Pane(pid)) => {
                        if let Some(pane) = self.home_panes.iter().find(|p| p.id == pid).cloned() {
                            self.kill_confirm(Resource::Pane(pane));
                        }
                    }
                    Some(Target::Window(_, wid)) => {
                        if let Some(win) = self.home_windows.iter().find(|w| w.id == wid).cloned() {
                            self.kill_confirm(Resource::Window(win));
                        }
                    }
                    Some(Target::Session(sid)) => {
                        if let Some(sess) = self.home_sessions.iter().find(|s| s.id == sid).cloned()
                        {
                            self.kill_confirm(Resource::Session(sess));
                        }
                    }
                    None => {}
                }
                true
            }
            _ => false,
        }
    }
}

const fn dock_zone_to_split(zone: DockZone) -> (SplitDirection, bool) {
    match zone {
        DockZone::Left => (SplitDirection::SideBySide, true),
        DockZone::Right => (SplitDirection::SideBySide, false),
        DockZone::Top => (SplitDirection::Stacked, true),
        DockZone::Bottom | DockZone::Center => (SplitDirection::Stacked, false),
    }
}

/// Sanitize a window name into a valid tmux session prefix (fallback `"workspace"`).
pub(super) fn sanitize_session_prefix(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        if ch.is_control() || matches!(ch, '.' | ':') || ch.is_whitespace() {
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(ch);
            last_dash = false;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "workspace".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn dock_zone_hint(zone: DockZone, zh: bool) -> String {
    match (zone, zh) {
        (DockZone::Left, false) => "Dock Left 50%".into(),
        (DockZone::Left, true) => "左侧 50% 分屏".into(),
        (DockZone::Right, false) => "Dock Right 50%".into(),
        (DockZone::Right, true) => "右侧 50% 分屏".into(),
        (DockZone::Top, false) => "Dock Top 50%".into(),
        (DockZone::Top, true) => "上方 50% 分屏".into(),
        (DockZone::Bottom, false) => "Dock Bottom 50%".into(),
        (DockZone::Bottom, true) => "下方 50% 分屏".into(),
        (DockZone::Center, false) => "Swap / Focus".into(),
        (DockZone::Center, true) => "互换 / 聚焦".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::{
        config::Config,
        tmux::{Context, Pane, Session, Tmux, TmuxError, TmuxResult, TmuxVersion},
    };

    #[test]
    fn dock_zone_detection_maps_edges_and_center() {
        let rect = Rect::new(10, 10, 40, 20);
        assert_eq!(compute_dock_zone(rect, 0, 0), None);
        assert_eq!(compute_dock_zone(rect, 12, 20), Some(DockZone::Left));
        assert_eq!(compute_dock_zone(rect, 48, 20), Some(DockZone::Right));
        assert_eq!(compute_dock_zone(rect, 30, 11), Some(DockZone::Top));
        assert_eq!(compute_dock_zone(rect, 30, 28), Some(DockZone::Bottom));
        assert_eq!(compute_dock_zone(rect, 30, 20), Some(DockZone::Center));
    }

    #[test]
    fn error_detection_and_selection_extraction_strip_ansi_and_respect_scroll() {
        let lines = vec![
            "\x1b[32mINFO\x1b[0m startup complete".to_owned(),
            "\x1b[31mERROR\x1b[0m database failed".to_owned(),
            "thread main panic at line 42".to_owned(),
            "all checks ok".to_owned(),
            "FATAL: abort".to_owned(),
        ];
        assert_eq!(detect_error_lines(&lines), vec![1, 2, 4]);

        // Visible rows = 3, scroll = 0 -> lines[2..5]
        let selected = extract_selected_preview_text(&lines, 0, 3, ((0, 7), (0, 16)));
        assert_eq!(selected, "main panic");

        // Multi-line reversed drag selection across scrolled window (scroll = 1 -> lines[1..4])
        let multi = extract_selected_preview_text(&lines, 1, 3, ((1, 5), (0, 6)));
        assert_eq!(multi, "database failed\nthread");
    }

    #[test]
    fn sparkline_and_hash_track_changes_deterministically() {
        let a = vec!["line 1".to_owned(), "line 2".to_owned()];
        let b = vec!["line 1".to_owned(), "line 2".to_owned()];
        let c = vec!["line 1".to_owned(), "line 3".to_owned()];
        assert_eq!(hash_lines(&a), hash_lines(&b));
        assert_ne!(hash_lines(&a), hash_lines(&c));

        let mut ring = [0_u8; 12];
        push_sparkline_sample(&mut ring, 5);
        push_sparkline_sample(&mut ring, 99);
        assert_eq!(ring[10], 5);
        assert_eq!(ring[11], 8);

        for (raw, expected) in [
            ("api.v2:logs", "api-v2-logs"),
            ("  my app\t\n ", "my-app"),
            (".: \x07 ", "workspace"),
            ("前端:服务", "前端-服务"),
        ] {
            let clean = sanitize_session_prefix(raw);
            assert_eq!(clean, expected);
            assert!(
                crate::tmux::validate_session_name(&format!("{clean}-ws")).is_ok(),
                "sanitized prefix must satisfy validate_session_name: {clean}"
            );
        }
    }

    struct DesktopStubTmux {
        sent_keys: RefCell<Vec<(PaneId, KeyEvent)>>,
        sent_pastes: RefCell<Vec<(PaneId, String)>>,
        renamed_windows: RefCell<Vec<(WindowId, String)>>,
        capture: RefCell<Vec<String>>,
    }

    impl DesktopStubTmux {
        fn new() -> Self {
            Self {
                sent_keys: RefCell::new(Vec::new()),
                sent_pastes: RefCell::new(Vec::new()),
                renamed_windows: RefCell::new(Vec::new()),
                capture: RefCell::new(vec!["$ ready".into()]),
            }
        }
    }

    impl Tmux for DesktopStubTmux {
        fn version(&self) -> TmuxResult<TmuxVersion> {
            Ok(TmuxVersion {
                raw: "3.4".into(),
                number: Some((3, 4)),
            })
        }
        fn server_running(&self) -> TmuxResult<bool> {
            Ok(true)
        }
        fn context(&self) -> Context {
            Context::Outside
        }
        fn list_sessions(&self) -> TmuxResult<Vec<Session>> {
            Ok(vec![Session {
                id: SessionId::new("$1").expect("sid"),
                name: "dev".into(),
                windows: 1,
                attached: 1,
                created: 1,
                last_attached: Some(2),
                activity: 3,
                path: "/tmp".into(),
            }])
        }
        fn has_session(&self, _: &SessionId) -> TmuxResult<bool> {
            Ok(true)
        }
        fn new_session(&self, _: &str, _: &std::path::Path) -> TmuxResult<Session> {
            Err(TmuxError::NoServer)
        }
        fn rename_session(&self, _: &SessionId, _: &str) -> TmuxResult<()> {
            Ok(())
        }
        fn kill_session(&self, _: &SessionId) -> TmuxResult<()> {
            Ok(())
        }
        fn list_windows(&self, _: Option<&SessionId>) -> TmuxResult<Vec<Window>> {
            Ok(vec![Window {
                id: WindowId::new("@1").expect("wid"),
                session_id: SessionId::new("$1").expect("sid"),
                session_name: "dev".into(),
                index: 0,
                name: "shell".into(),
                active: true,
                panes: 1,
                zoomed: false,
                synchronized: false,
                activity_flag: false,
                bell_flag: false,
                silence_flag: false,
                layout: String::new(),
                monitor_activity: false,
                monitor_silence: 0,
            }])
        }
        fn new_window(
            &self,
            _: &SessionId,
            _: Option<&str>,
            _: Option<&std::path::Path>,
        ) -> TmuxResult<Window> {
            Err(TmuxError::NoServer)
        }
        fn rename_window(&self, id: &WindowId, name: &str) -> TmuxResult<()> {
            self.renamed_windows
                .borrow_mut()
                .push((id.clone(), name.to_owned()));
            Ok(())
        }
        fn kill_window(&self, _: &WindowId) -> TmuxResult<()> {
            Ok(())
        }
        fn list_panes(&self, _: Option<&WindowId>) -> TmuxResult<Vec<Pane>> {
            Ok(vec![Pane {
                id: PaneId::new("%1").expect("pid"),
                window_id: WindowId::new("@1").expect("wid"),
                window_index: 0,
                window_name: "shell".into(),
                session_id: SessionId::new("$1").expect("sid"),
                session_name: "dev".into(),
                index: 0,
                active: true,
                pid: Some(1234),
                current_command: "zsh".into(),
                current_path: "/tmp".into(),
                title: "term".into(),
                busy: false,
                width: 80,
                height: 24,
                left: 0,
                top: 0,
                zoomed: false,
                dead: false,
                synchronized: false,
            }])
        }
        fn split_pane(&self, _: &PaneId, _: SplitDirection) -> TmuxResult<Pane> {
            Err(TmuxError::NoServer)
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
        fn detach_client(&self, _: &str) -> TmuxResult<()> {
            Ok(())
        }
        fn capture_pane(&self, _: &Target, _: usize) -> TmuxResult<Vec<String>> {
            Ok(self.capture.borrow().clone())
        }
        fn send_key_event(&self, id: &PaneId, key: KeyEvent) -> TmuxResult<()> {
            self.sent_keys.borrow_mut().push((id.clone(), key));
            Ok(())
        }
        fn send_paste(&self, id: &PaneId, text: &str) -> TmuxResult<()> {
            self.sent_pastes
                .borrow_mut()
                .push((id.clone(), text.to_owned()));
            Ok(())
        }
    }

    #[test]
    fn live_pip_intercepts_ctrl_c_paste_and_esc_and_inline_stays_idle() {
        let stub = DesktopStubTmux::new();
        let cfg = Config {
            live_stream: true,
            ..Config::default()
        };
        let mut app = App::with_config(&stub, super::super::AppOptions::default(), cfg, None);

        // Inline mode MUST disable live_stream polling when idle (0 idle bytes invariant).
        assert_eq!(app.next_poll_interval(ScreenMode::Inline), None);
        assert_eq!(
            app.next_poll_interval(ScreenMode::Fullscreen),
            Some(STREAM_POLL_INTERVAL)
        );

        // Moving mouse over empty space returns `false` (0 redraws / 0 bytes).
        assert!(!app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })));

        // Enter Live PIP -> Ctrl+C MUST forward to pane instead of quitting JustMux!
        app.toggle_live_pip(None);
        assert!(app.is_live_pip_active());
        assert_eq!(
            app.next_poll_interval(ScreenMode::Fullscreen),
            Some(PIP_POLL_INTERVAL)
        );

        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))));
        assert!(
            app.exit().is_none(),
            "Ctrl+C in Live PIP must NOT quit JustMux"
        );
        assert_eq!(stub.sent_keys.borrow().len(), 1);
        assert_eq!(stub.sent_keys.borrow()[0].1.code, KeyCode::Char('c'));

        // Paste forwards directly to `send_paste`.
        assert!(app.handle_event(Event::Paste("echo hi\n".into())));
        assert_eq!(stub.sent_pastes.borrow().len(), 1);
        assert_eq!(stub.sent_pastes.borrow()[0].1, "echo hi\n");

        // `[Send Esc]` sends literal Esc without leaving Live PIP.
        app.send_literal_esc_to_pip();
        assert!(app.is_live_pip_active());
        assert_eq!(
            stub.sent_keys.borrow().last().map(|k| k.1.code),
            Some(KeyCode::Esc)
        );

        // Keyboard `Esc` exits Live PIP cleanly.
        assert!(app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,))));
        assert!(!app.is_live_pip_active());

        // `F2` inline rename pre-fills current window name selected-all:
        // moving cursor right cancels selection and appends `2` -> `shell2`,
        // while immediate typing replaces the whole text cleanly.
        assert!(app.handle_event(Event::Key(
            KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE,)
        )));
        assert!(app.desktop.inline_rename.is_some());
        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        ))));
        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('2'),
            KeyModifiers::NONE,
        ))));
        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))));
        assert!(app.desktop.inline_rename.is_none());

        assert!(app.handle_event(Event::Key(
            KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE,)
        )));
        for ch in ['a', 'p', 'i'] {
            assert!(app.handle_event(Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ))));
        }
        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))));
        assert_eq!(
            stub.renamed_windows.borrow().as_slice(),
            &[
                (WindowId::new("@1").expect("wid"), "shell2".to_owned()),
                (WindowId::new("@1").expect("wid"), "api".to_owned()),
            ]
        );

        // `F3` opens Mission Control `/expose` with 250ms poll interval.
        assert!(app.handle_event(Event::Key(
            KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE,)
        )));
        assert!(matches!(app.views.top(), Some(View::Expose(_))));
        app.desktop.toast = None;
        assert_eq!(
            app.next_poll_interval(ScreenMode::Fullscreen),
            Some(EXPOSE_POLL_INTERVAL)
        );
        assert_eq!(app.next_poll_interval(ScreenMode::Inline), None);
    }

    #[test]
    fn desktop_gui_end_to_end_menubar_sash_dock_drag_context_and_expose_on_private_socket() {
        use ratatui::{Terminal, backend::TestBackend};

        let socket = format!(
            "jm-desktop-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmux = crate::tmux::TmuxClient::new(crate::tmux::SocketSpec::Name(socket))
            .with_config_file("/dev/null");
        struct Cleanup(crate::tmux::TmuxClient);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.kill_server();
            }
        }
        let _cleanup = Cleanup(tmux.clone());

        let cwd = std::env::temp_dir();
        let sess1 = tmux.new_session("alpha", &cwd).expect("session alpha");
        let _win2 = tmux
            .new_window(&sess1.id, Some("worker"), Some(&cwd))
            .expect("second window");
        let _sess2 = tmux.new_session("beta", &cwd).expect("session beta");

        let mut app = App::with_config(
            &tmux,
            super::super::AppOptions::default(),
            Config::default(),
            None,
        );
        let mut term = Terminal::new(TestBackend::new(120, 32)).expect("test backend");
        term.draw(|f| app.render(f)).expect("initial Home render");

        // 1. Top IDE Menubar hit-boxes exist; clicking `[中/EN]` toggles language live!
        let lang_rect = app
            .desktop
            .menubar_hits
            .iter()
            .find(|(_, item)| *item == MenubarItem::LanguageToggle)
            .map(|(r, _)| *r)
            .expect("language toggle menubar hit");
        assert!(!app.theme().language.is_zh());
        assert!(app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: lang_rect.x,
            row: lang_rect.y,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(app.theme().language.is_zh());
        assert!(app.desktop.has_active_toast());
        term.draw(|f| app.render(f))
            .expect("zh Home + Toast render");

        // Toggle back to EN.
        let lang_rect2 = app
            .desktop
            .menubar_hits
            .iter()
            .find(|(_, item)| *item == MenubarItem::LanguageToggle)
            .map(|(r, _)| *r)
            .expect("language toggle hit 2");
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: lang_rect2.x,
            row: lang_rect2.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(!app.theme().language.is_zh());

        // 2. Dragging Home Preview/Matrix Sash updates `home_split_ratio`.
        term.draw(|f| app.render(f)).expect("render before sash");
        if let Some((sash_rect, _)) = app
            .desktop
            .sash_hits
            .iter()
            .find(|(_, id)| *id == SashId::HomePreviewMatrix)
            .copied()
        {
            assert!(app.handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: sash_rect.x + 2,
                row: sash_rect.y,
                modifiers: KeyModifiers::NONE,
            })));
            assert!(app.handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: sash_rect.x + 2,
                row: sash_rect.y.saturating_add(4),
                modifiers: KeyModifiers::NONE,
            })));
            assert!(app.handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: sash_rect.x + 2,
                row: sash_rect.y.saturating_add(4),
                modifiers: KeyModifiers::NONE,
            })));
            assert!(app.desktop.home_split_ratio > DEFAULT_HOME_SPLIT_RATIO);
        }

        // 3. Drag Matrix window onto Preview Card Left dock zone -> joins panes with `join_pane_ordered`!
        term.draw(|f| app.render(f)).expect("render before dock");
        let prev_rect = app.preview_rect.expect("preview rect");
        let second_win_hit = app
            .matrix_hits
            .iter()
            .find(|h| matches!(h.target, MatrixHitTarget::Window { col: 0, row: 1 }))
            .cloned()
            .or_else(|| {
                app.matrix_hits
                    .iter()
                    .find(|h| matches!(h.target, MatrixHitTarget::Window { col: 1, row: 0 }))
                    .cloned()
            })
            .expect("second window matrix hit");
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: second_win_hit.rect.x + 1,
            row: second_win_hit.rect.y,
            modifiers: KeyModifiers::NONE,
        }));
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: prev_rect.x + 1,
            row: prev_rect.y + prev_rect.height / 2,
            modifiers: KeyModifiers::NONE,
        }));
        term.draw(|f| app.render(f))
            .expect("render drag ghost + dock zone");
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: prev_rect.x + 1,
            row: prev_rect.y + prev_rect.height / 2,
            modifiers: KeyModifiers::NONE,
        }));
        let panes_after_dock = tmux.list_panes(None).expect("list panes after dock");
        assert!(
            panes_after_dock.iter().any(|p| p.left > 0),
            "expected side-by-side docked pane after Left edge drop: {panes_after_dock:?}"
        );

        // 4. Right-click opens floating context menu; Down wraps; Esc dismisses.
        term.draw(|f| app.render(f))
            .expect("render before right click");
        assert!(app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: prev_rect.x + 2,
            row: prev_rect.y + 1,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(app.desktop.context_menu.is_some());
        term.draw(|f| app.render(f))
            .expect("render floating context menu");
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.desktop.context_menu.is_none());

        // 5. Clicking `[⊞ Expose]` tab switches to Mission Control & renders live tiles.
        let expose_rect = app
            .desktop
            .menubar_hits
            .iter()
            .find(|(_, item)| *item == MenubarItem::TabExpose)
            .map(|(r, _)| *r)
            .expect("expose tab hit");
        assert!(app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: expose_rect.x,
            row: expose_rect.y,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(matches!(app.views.top(), Some(View::Expose(_))));
        term.draw(|f| app.render(f))
            .expect("render Mission Control Expose wall");
        assert!(!app.desktop.expose_hits.is_empty());

        // 6. Middle-click on Expose tile opens safe `yes` Kill Confirm!
        let tile0_rect = app.desktop.expose_hits[0].rect;
        assert!(app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Middle),
            column: tile0_rect.x + 2,
            row: tile0_rect.y + 2,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(matches!(app.views.top(), Some(View::Confirm(_))));
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(matches!(app.views.top(), Some(View::Expose(_))));
    }
}
