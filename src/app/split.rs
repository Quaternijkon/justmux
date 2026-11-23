//! Interactive 2x2 visual split layout tree and studio state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::layout::Rect;

use crate::tmux::{Pane, PaneId, SessionId, SplitDirection, Window, WindowId};

/// Identifier of a single leaf pane inside [`SplitStudio`].
pub type SlotId = usize;

/// Minimum allowed percentage share for the first child of a split.
pub const MIN_SPLIT_RATIO: u16 = 20;
/// Maximum allowed percentage share for the first child of a split.
pub const MAX_SPLIT_RATIO: u16 = 80;
/// Default balanced 50/50 percentage share for a new split.
pub const DEFAULT_SPLIT_RATIO: u16 = 50;

/// Content assigned to one leaf slot in the 2x2 split layout draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitSlotFill {
    Window(WindowId),
    Pane(PaneId),
    NewShell,
}

/// Metadata for an existing pane inside a multi-pane window opened in Split Studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPaneInfo {
    pub id: PaneId,
    pub window_id: WindowId,
    pub index: u32,
    pub title: String,
    pub command: String,
    pub path: String,
}

/// Built-in 2x2 grid presets cycled by Split Studio's quick preset pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitLayoutPreset {
    OneByTwo,
    TwoByOne,
    MainPlusTwo,
    TwoByTwo,
}

impl SplitLayoutPreset {
    pub const ALL: &'static [Self] = &[
        Self::OneByTwo,
        Self::TwoByOne,
        Self::MainPlusTwo,
        Self::TwoByTwo,
    ];

    pub const fn label(self, zh: bool) -> &'static str {
        match (self, zh) {
            (Self::OneByTwo, false) => "1x2 Side-by-Side",
            (Self::OneByTwo, true) => "1×2 左右双屏",
            (Self::TwoByOne, false) => "2x1 Top/Bottom",
            (Self::TwoByOne, true) => "2×1 上下双屏",
            (Self::MainPlusTwo, false) => "Main + 2",
            (Self::MainPlusTwo, true) => "主屏 + 2 辅屏",
            (Self::TwoByTwo, false) => "2x2 Grid",
            (Self::TwoByTwo, true) => "2×2 四宫格",
        }
    }
}

/// Binary split tree constrained to at most 2 columns by 2 rows (4 panes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitNode {
    Leaf {
        id: SlotId,
        fill: SplitSlotFill,
    },
    Split {
        direction: SplitDirection,
        ratio: u16,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

/// Interactive toolbar button exposed inside the visual Split Studio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitToolbarAction {
    SplitHorizontal,
    SplitVertical,
    FlipDirection,
    RatioPreset,
    SyncToggle,
    PresetGrid,
    BreakOut,
    RemoveSlot,
    Reset,
    ApplyAndEnter,
}

/// Candidate existing tmux window that can be placed into any split slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitWindowOption {
    pub window: Window,
    pub active_pane_id: Option<PaneId>,
    pub command: String,
    pub path: String,
}

/// Full interactive state for the GUI-like 2x2 Split Studio subview.
#[derive(Debug, Clone)]
pub struct SplitStudio {
    pub anchor_session_id: SessionId,
    pub anchor_window_id: Option<WindowId>,
    pub anchor_cwd: PathBuf,
    pub initial_fill: SplitSlotFill,
    pub initial_root: Option<SplitNode>,
    pub root: SplitNode,
    pub next_slot_id: SlotId,
    pub selected_slot: SlotId,
    pub window_cursor: usize,
    pub window_options: Vec<SplitWindowOption>,
    pub editing_existing_panes: Vec<SplitPaneInfo>,
    pub previews: HashMap<WindowId, Vec<String>>,
    pub pane_previews: HashMap<PaneId, Vec<String>>,
    pub preview_scrolls: HashMap<SlotId, usize>,
    pub initial_sync_panes: bool,
    pub sync_panes: bool,
    pub preset_index: usize,
    pub slot_hits: Vec<(Rect, SlotId)>,
    pub toolbar_hits: Vec<(Rect, SplitToolbarAction)>,
    pub window_list_rect: Option<Rect>,
}

impl SplitNode {
    /// Construct a leaf pane node.
    pub fn new_leaf(id: SlotId, fill: SplitSlotFill) -> Self {
        Self::Leaf { id, fill }
    }

    /// Number of leaf panes in this subtree.
    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Ordered left-to-right / top-to-bottom list of leaves.
    pub fn leaves(&self) -> Vec<(SlotId, SplitSlotFill)> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<(SlotId, SplitSlotFill)>) {
        match self {
            Self::Leaf { id, fill } => out.push((*id, fill.clone())),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Leftmost/topmost leaf in this subtree.
    pub fn first_leaf(&self) -> (SlotId, SplitSlotFill) {
        match self {
            Self::Leaf { id, fill } => (*id, fill.clone()),
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Lookup fill assigned to `target`.
    pub fn slot_fill(&self, target: SlotId) -> Option<&SplitSlotFill> {
        match self {
            Self::Leaf { id, fill } if *id == target => Some(fill),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => {
                first.slot_fill(target).or_else(|| second.slot_fill(target))
            }
        }
    }

    /// Find which slot (if any) currently displays `window_id`.
    pub fn find_slot_with_window(&self, window_id: &WindowId) -> Option<SlotId> {
        match self {
            Self::Leaf {
                id,
                fill: SplitSlotFill::Window(wid),
            } if wid == window_id => Some(*id),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => first
                .find_slot_with_window(window_id)
                .or_else(|| second.find_slot_with_window(window_id)),
        }
    }

    /// Find which slot (if any) currently displays `pane_id`.
    pub fn find_slot_with_pane(&self, pane_id: &PaneId) -> Option<SlotId> {
        match self {
            Self::Leaf {
                id,
                fill: SplitSlotFill::Pane(pid),
            } if pid == pane_id => Some(*id),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => first
                .find_slot_with_pane(pane_id)
                .or_else(|| second.find_slot_with_pane(pane_id)),
        }
    }

    /// Count `(horizontal_splits, vertical_splits)` along the path from root to `target`.
    pub fn axis_depths(&self, target: SlotId) -> Option<(usize, usize)> {
        match self {
            Self::Leaf { id, .. } => (*id == target).then_some((0, 0)),
            Self::Split {
                direction,
                first,
                second,
                ..
            } => {
                let (h, v) = first
                    .axis_depths(target)
                    .or_else(|| second.axis_depths(target))?;
                Some((
                    h + usize::from(*direction == SplitDirection::SideBySide),
                    v + usize::from(*direction == SplitDirection::Stacked),
                ))
            }
        }
    }

    /// Enforce at most a 2x2 grid: at most 1 horizontal split and 1 vertical split on any path.
    pub fn can_split(&self, target: SlotId, direction: SplitDirection) -> bool {
        if self.leaf_count() >= 4 {
            return false;
        }
        let Some((h, v)) = self.axis_depths(target) else {
            return false;
        };
        match direction {
            SplitDirection::SideBySide => h == 0,
            SplitDirection::Stacked => v == 0,
        }
    }

    /// Whether this layout has at least two panes whose split directions can be transposed.
    pub fn can_flip(&self) -> bool {
        self.leaf_count() >= 2
    }

    /// Transpose every split direction (`SideBySide <-> Stacked`), preserving max 2x2 invariants.
    pub fn flip_directions(&mut self) -> bool {
        if !self.can_flip() {
            return false;
        }
        self.flip_directions_inner();
        true
    }

    fn flip_directions_inner(&mut self) {
        if let Self::Split {
            direction,
            first,
            second,
            ..
        } = self
        {
            *direction = match *direction {
                SplitDirection::SideBySide => SplitDirection::Stacked,
                SplitDirection::Stacked => SplitDirection::SideBySide,
            };
            first.flip_directions_inner();
            second.flip_directions_inner();
        }
    }

    /// Return the split ratio of the nearest enclosing split containing `target`.
    pub fn parent_split_ratio(&self, target: SlotId) -> Option<u16> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if first.slot_fill(target).is_some() {
                    first.parent_split_ratio(target).or(Some(*ratio))
                } else if second.slot_fill(target).is_some() {
                    second.parent_split_ratio(target).or(Some(*ratio))
                } else {
                    None
                }
            }
        }
    }

    /// Adjust the ratio of the nearest enclosing split containing `target` by `delta` (`20..=80`).
    pub fn adjust_ratio(&mut self, target: SlotId, delta: i16) -> Option<u16> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if first.slot_fill(target).is_some() {
                    if let Some(updated) = first.adjust_ratio(target, delta) {
                        return Some(updated);
                    }
                    let next = (i32::from(*ratio) + i32::from(delta))
                        .clamp(i32::from(MIN_SPLIT_RATIO), i32::from(MAX_SPLIT_RATIO))
                        as u16;
                    *ratio = next;
                    Some(next)
                } else if second.slot_fill(target).is_some() {
                    if let Some(updated) = second.adjust_ratio(target, delta) {
                        return Some(updated);
                    }
                    let next = (i32::from(*ratio) + i32::from(delta))
                        .clamp(i32::from(MIN_SPLIT_RATIO), i32::from(MAX_SPLIT_RATIO))
                        as u16;
                    *ratio = next;
                    Some(next)
                } else {
                    None
                }
            }
        }
    }

    /// Cycle the nearest enclosing split's ratio through `50 -> 70 -> 30 -> 50`.
    pub fn cycle_ratio_preset(&mut self, target: SlotId) -> Option<u16> {
        let current = self.parent_split_ratio(target)?;
        let next: u16 = if current < 40 {
            50
        } else if current < 65 {
            70
        } else {
            30
        };
        let delta = (i32::from(next) - i32::from(current)) as i16;
        self.adjust_ratio(target, delta)
    }

    /// Look up the `(direction, ratio)` of a split node by heap index (`root=0`, `first=2*i+1`, `second=2*i+2`).
    pub fn split_ratio_at(&self, target_idx: usize) -> Option<(SplitDirection, u16)> {
        self.split_ratio_at_inner(0, target_idx)
    }

    fn split_ratio_at_inner(
        &self,
        current_idx: usize,
        target_idx: usize,
    ) -> Option<(SplitDirection, u16)> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if current_idx == target_idx {
                    return Some((*direction, *ratio));
                }
                if target_idx < current_idx {
                    return None;
                }
                let left = current_idx.saturating_mul(2).saturating_add(1);
                let right = current_idx.saturating_mul(2).saturating_add(2);
                first
                    .split_ratio_at_inner(left, target_idx)
                    .or_else(|| second.split_ratio_at_inner(right, target_idx))
            }
        }
    }

    /// Set the ratio (`20..=80`) of the split node at heap index `target_idx`.
    /// Returns `true` if the node exists and its ratio changed.
    pub fn set_split_ratio_at(&mut self, target_idx: usize, new_ratio: u16) -> bool {
        let clamped = new_ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        self.set_split_ratio_at_inner(0, target_idx, clamped)
    }

    fn set_split_ratio_at_inner(
        &mut self,
        current_idx: usize,
        target_idx: usize,
        clamped: u16,
    ) -> bool {
        match self {
            Self::Leaf { .. } => false,
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if current_idx == target_idx {
                    let changed = *ratio != clamped;
                    *ratio = clamped;
                    return changed;
                }
                if target_idx < current_idx {
                    return false;
                }
                let left = current_idx.saturating_mul(2).saturating_add(1);
                let right = current_idx.saturating_mul(2).saturating_add(2);
                first.set_split_ratio_at_inner(left, target_idx, clamped)
                    || second.set_split_ratio_at_inner(right, target_idx, clamped)
            }
        }
    }

    /// Split `target` leaf in `direction`, placing `new_id` before (`Left`/`Top`) or after (`Right`/`Bottom`).
    pub fn split_slot_ordered(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        before: bool,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        if !self.can_split(target, direction) {
            return false;
        }
        self.split_slot_ordered_inner(target, direction, before, new_id, new_fill)
    }

    fn split_slot_ordered_inner(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        before: bool,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        match self {
            Self::Leaf { id, fill } if *id == target => {
                let kept_id = *id;
                let kept_fill = std::mem::replace(fill, SplitSlotFill::NewShell);
                let kept = Box::new(Self::new_leaf(kept_id, kept_fill));
                let added = Box::new(Self::new_leaf(new_id, new_fill));
                let (first, second) = if before { (added, kept) } else { (kept, added) };
                *self = Self::Split {
                    direction,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first,
                    second,
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_slot_ordered_inner(target, direction, before, new_id, new_fill.clone())
                    || second.split_slot_ordered_inner(target, direction, before, new_id, new_fill)
            }
        }
    }

    /// Swap the fills of two distinct leaf slots `a` and `b`.
    pub fn swap_slot_fills(&mut self, a: SlotId, b: SlotId) -> bool {
        if a == b {
            return false;
        }
        let (Some(fill_a), Some(fill_b)) = (self.slot_fill(a).cloned(), self.slot_fill(b).cloned())
        else {
            return false;
        };
        self.replace_slot_fill_direct(a, fill_b);
        self.replace_slot_fill_direct(b, fill_a);
        true
    }

    fn replace_slot_fill_direct(&mut self, target: SlotId, next: SplitSlotFill) {
        match self {
            Self::Leaf { id, fill } if *id == target => *fill = next,
            Self::Leaf { .. } => {}
            Self::Split { first, second, .. } => {
                first.replace_slot_fill_direct(target, next.clone());
                second.replace_slot_fill_direct(target, next);
            }
        }
    }

    /// Rebuild this tree into a preset layout, preserving existing fills in reading order.
    /// Returns `(next_slot_id, first_slot_id)`.
    pub fn apply_preset_layout(&mut self, preset: SplitLayoutPreset) -> (SlotId, SlotId) {
        let mut fills: Vec<SplitSlotFill> = self.leaves().into_iter().map(|(_, f)| f).collect();
        let take_fill = |idx: usize| -> SplitSlotFill {
            fills.get(idx).cloned().unwrap_or(SplitSlotFill::NewShell)
        };
        let (node, next_id) = match preset {
            SplitLayoutPreset::OneByTwo => (
                Self::Split {
                    direction: SplitDirection::SideBySide,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(Self::new_leaf(1, take_fill(0))),
                    second: Box::new(Self::new_leaf(2, take_fill(1))),
                },
                3,
            ),
            SplitLayoutPreset::TwoByOne => (
                Self::Split {
                    direction: SplitDirection::Stacked,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(Self::new_leaf(1, take_fill(0))),
                    second: Box::new(Self::new_leaf(2, take_fill(1))),
                },
                3,
            ),
            SplitLayoutPreset::MainPlusTwo => (
                Self::Split {
                    direction: SplitDirection::SideBySide,
                    ratio: 60,
                    first: Box::new(Self::new_leaf(1, take_fill(0))),
                    second: Box::new(Self::Split {
                        direction: SplitDirection::Stacked,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(Self::new_leaf(2, take_fill(1))),
                        second: Box::new(Self::new_leaf(3, take_fill(2))),
                    }),
                },
                4,
            ),
            SplitLayoutPreset::TwoByTwo => (
                Self::Split {
                    direction: SplitDirection::SideBySide,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(Self::Split {
                        direction: SplitDirection::Stacked,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(Self::new_leaf(1, take_fill(0))),
                        second: Box::new(Self::new_leaf(3, take_fill(2))),
                    }),
                    second: Box::new(Self::Split {
                        direction: SplitDirection::Stacked,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(Self::new_leaf(2, take_fill(1))),
                        second: Box::new(Self::new_leaf(4, take_fill(3))),
                    }),
                },
                5,
            ),
        };
        fills.clear();
        *self = node;
        (next_id, 1)
    }

    /// Reconstruct a 2x2 split tree from real tmux [`Pane`] geometries.
    /// Returns `(root, next_slot_id)` if at least one pane is present.
    pub fn from_existing_panes(panes: &[Pane]) -> Option<(Self, SlotId)> {
        let geoms: Vec<(PaneId, u32, u32, u32, u32)> = panes
            .iter()
            .map(|p| (p.id.clone(), p.left, p.top, p.width, p.height))
            .collect();
        Self::from_pane_geometries(&geoms)
    }

    /// Reconstruct a 2x2 split tree from `(pane_id, left, top, width, height)` geometries.
    /// Returns `(root, next_slot_id)` if at least one pane is supplied.
    pub fn from_pane_geometries(panes: &[(PaneId, u32, u32, u32, u32)]) -> Option<(Self, SlotId)> {
        if panes.is_empty() {
            return None;
        }
        let mut sorted: Vec<(PaneId, u32, u32, u32, u32)> = panes.to_vec();
        sorted.sort_by_key(|(_, left, top, _, _)| (*top, *left));
        sorted.truncate(4);
        let mut next_id: SlotId = 1;
        let root = Self::build_from_geometries(&sorted, 0, 0, &mut next_id);
        Some((root, next_id))
    }

    fn build_from_geometries(
        panes: &[(PaneId, u32, u32, u32, u32)],
        h_depth: usize,
        v_depth: usize,
        next_id: &mut SlotId,
    ) -> Self {
        if panes.len() == 1 {
            let id = *next_id;
            *next_id += 1;
            return Self::new_leaf(id, SplitSlotFill::Pane(panes[0].0.clone()));
        }

        let min_x = panes.iter().map(|p| p.1).min().unwrap_or(0);
        let max_x = panes
            .iter()
            .map(|p| p.1.saturating_add(p.3))
            .max()
            .unwrap_or(0);
        let min_y = panes.iter().map(|p| p.2).min().unwrap_or(0);
        let max_y = panes
            .iter()
            .map(|p| p.2.saturating_add(p.4))
            .max()
            .unwrap_or(0);
        let total_w = max_x.saturating_sub(min_x);
        let total_h = max_y.saturating_sub(min_y);

        // Try vertical cut (SideBySide) if we haven't spent our horizontal split budget.
        if h_depth == 0 && total_w > 0 {
            let mut cut_xs: Vec<u32> = panes.iter().map(|p| p.1).filter(|&x| x > min_x).collect();
            cut_xs.sort_unstable();
            cut_xs.dedup();
            for cut_x in cut_xs {
                let left: Vec<_> = panes
                    .iter()
                    .filter(|p| p.1.saturating_add(p.3) <= cut_x)
                    .cloned()
                    .collect();
                let right: Vec<_> = panes.iter().filter(|p| p.1 >= cut_x).cloned().collect();
                if !left.is_empty()
                    && !right.is_empty()
                    && left.len() + right.len() == panes.len()
                    && left.len() <= 2
                    && right.len() <= 2
                {
                    let left_edge = left
                        .iter()
                        .map(|p| p.1.saturating_add(p.3))
                        .max()
                        .unwrap_or(cut_x);
                    let mid = (left_edge.saturating_add(cut_x)) / 2;
                    let ratio = ((mid.saturating_sub(min_x) * 100) / total_w)
                        .clamp(u32::from(MIN_SPLIT_RATIO), u32::from(MAX_SPLIT_RATIO))
                        as u16;
                    let first = Self::build_from_geometries(&left, h_depth + 1, v_depth, next_id);
                    let second = Self::build_from_geometries(&right, h_depth + 1, v_depth, next_id);
                    return Self::Split {
                        direction: SplitDirection::SideBySide,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    };
                }
            }
        }

        // Try horizontal cut (Stacked) if we haven't spent our vertical split budget.
        if v_depth == 0 && total_h > 0 {
            let mut cut_ys: Vec<u32> = panes.iter().map(|p| p.2).filter(|&y| y > min_y).collect();
            cut_ys.sort_unstable();
            cut_ys.dedup();
            for cut_y in cut_ys {
                let top: Vec<_> = panes
                    .iter()
                    .filter(|p| p.2.saturating_add(p.4) <= cut_y)
                    .cloned()
                    .collect();
                let bottom: Vec<_> = panes.iter().filter(|p| p.2 >= cut_y).cloned().collect();
                if !top.is_empty()
                    && !bottom.is_empty()
                    && top.len() + bottom.len() == panes.len()
                    && top.len() <= 2
                    && bottom.len() <= 2
                {
                    let top_edge = top
                        .iter()
                        .map(|p| p.2.saturating_add(p.4))
                        .max()
                        .unwrap_or(cut_y);
                    let mid = (top_edge.saturating_add(cut_y)) / 2;
                    let ratio = ((mid.saturating_sub(min_y) * 100) / total_h)
                        .clamp(u32::from(MIN_SPLIT_RATIO), u32::from(MAX_SPLIT_RATIO))
                        as u16;
                    let first = Self::build_from_geometries(&top, h_depth, v_depth + 1, next_id);
                    let second =
                        Self::build_from_geometries(&bottom, h_depth, v_depth + 1, next_id);
                    return Self::Split {
                        direction: SplitDirection::Stacked,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    };
                }
            }
        }

        // Fallback: arrange 2..=4 panes into a guaranteed-valid 2x2 binary split tree.
        let mut alloc = |pane: &PaneId| -> Self {
            let id = *next_id;
            *next_id += 1;
            Self::new_leaf(id, SplitSlotFill::Pane(pane.clone()))
        };
        let direction = if h_depth == 0 {
            SplitDirection::SideBySide
        } else {
            SplitDirection::Stacked
        };
        match panes.len() {
            2 => Self::Split {
                direction,
                ratio: DEFAULT_SPLIT_RATIO,
                first: Box::new(alloc(&panes[0].0)),
                second: Box::new(alloc(&panes[1].0)),
            },
            3 => {
                let sub_dir = if direction == SplitDirection::SideBySide {
                    SplitDirection::Stacked
                } else {
                    SplitDirection::SideBySide
                };
                Self::Split {
                    direction,
                    ratio: 60,
                    first: Box::new(alloc(&panes[0].0)),
                    second: Box::new(Self::Split {
                        direction: sub_dir,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(alloc(&panes[1].0)),
                        second: Box::new(alloc(&panes[2].0)),
                    }),
                }
            }
            _ => {
                let sub_dir = if direction == SplitDirection::SideBySide {
                    SplitDirection::Stacked
                } else {
                    SplitDirection::SideBySide
                };
                Self::Split {
                    direction,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(Self::Split {
                        direction: sub_dir,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(alloc(&panes[0].0)),
                        second: Box::new(alloc(&panes[2].0)),
                    }),
                    second: Box::new(Self::Split {
                        direction: sub_dir,
                        ratio: DEFAULT_SPLIT_RATIO,
                        first: Box::new(alloc(&panes[1].0)),
                        second: Box::new(alloc(&panes[3].0)),
                    }),
                }
            }
        }
    }

    /// Replace any [`SplitSlotFill::Window`] references absent from `live_windows` with [`SplitSlotFill::NewShell`].
    pub fn prune_missing_windows(&mut self, live_windows: &HashSet<WindowId>) {
        match self {
            Self::Leaf { fill, .. } => {
                if let SplitSlotFill::Window(wid) = fill
                    && !live_windows.contains(wid)
                {
                    *fill = SplitSlotFill::NewShell;
                }
            }
            Self::Split { first, second, .. } => {
                first.prune_missing_windows(live_windows);
                second.prune_missing_windows(live_windows);
            }
        }
    }

    /// Replace any [`SplitSlotFill::Pane`] references absent from `live_panes` with [`SplitSlotFill::NewShell`].
    pub fn prune_missing_panes(&mut self, live_panes: &HashSet<PaneId>) {
        match self {
            Self::Leaf { fill, .. } => {
                if let SplitSlotFill::Pane(pid) = fill
                    && !live_panes.contains(pid)
                {
                    *fill = SplitSlotFill::NewShell;
                }
            }
            Self::Split { first, second, .. } => {
                first.prune_missing_panes(live_panes);
                second.prune_missing_panes(live_panes);
            }
        }
    }

    /// Split `target` leaf in `direction`, moving existing fill to `first` and adding `new_id` as `second`.
    pub fn split_slot(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        if !self.can_split(target, direction) {
            return false;
        }
        self.split_slot_inner(target, direction, new_id, new_fill)
    }

    fn split_slot_inner(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        match self {
            Self::Leaf { id, fill } if *id == target => {
                let kept_id = *id;
                let kept_fill = std::mem::replace(fill, SplitSlotFill::NewShell);
                *self = Self::Split {
                    direction,
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(Self::new_leaf(kept_id, kept_fill)),
                    second: Box::new(Self::new_leaf(new_id, new_fill)),
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_slot_inner(target, direction, new_id, new_fill.clone())
                    || second.split_slot_inner(target, direction, new_id, new_fill)
            }
        }
    }

    /// Remove `target` leaf, collapsing its parent split to its sibling and returning the sibling's first leaf.
    pub fn remove_slot(&mut self, target: SlotId) -> Option<SlotId> {
        if self.leaf_count() <= 1 || self.slot_fill(target).is_none() {
            return None;
        }
        let owned = std::mem::replace(self, Self::new_leaf(0, SplitSlotFill::NewShell));
        let (rebuilt, next_focus) = Self::remove_inner(owned, target);
        *self = rebuilt;
        next_focus
    }

    fn remove_inner(node: Self, target: SlotId) -> (Self, Option<SlotId>) {
        match node {
            Self::Leaf { .. } => (node, None),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if matches!(&*first, Self::Leaf { id, .. } if *id == target) {
                    let next = second.first_leaf().0;
                    return (*second, Some(next));
                }
                if matches!(&*second, Self::Leaf { id, .. } if *id == target) {
                    let next = first.first_leaf().0;
                    return (*first, Some(next));
                }
                let (new_first, focus_first) = Self::remove_inner(*first, target);
                if let Some(next) = focus_first {
                    return (
                        Self::Split {
                            direction,
                            ratio,
                            first: Box::new(new_first),
                            second,
                        },
                        Some(next),
                    );
                }
                let (new_second, focus_second) = Self::remove_inner(*second, target);
                (
                    Self::Split {
                        direction,
                        ratio,
                        first: Box::new(new_first),
                        second: Box::new(new_second),
                    },
                    focus_second,
                )
            }
        }
    }

    /// Assign `fill` to `target`, swapping if `Window(wid)` or `Pane(pid)` is already held by another slot.
    pub fn set_slot_fill(&mut self, target: SlotId, fill: SplitSlotFill) -> bool {
        let Some(current) = self.slot_fill(target).cloned() else {
            return false;
        };
        if current == fill {
            return true;
        }
        let existing_owner = match &fill {
            SplitSlotFill::Window(wid) => self.find_slot_with_window(wid),
            SplitSlotFill::Pane(pid) => self.find_slot_with_pane(pid),
            SplitSlotFill::NewShell => None,
        };
        if let Some(other) = existing_owner
            && other != target
        {
            return self.replace_leaf_fill(other, current) && self.replace_leaf_fill(target, fill);
        }
        self.replace_leaf_fill(target, fill)
    }

    fn replace_leaf_fill(&mut self, target: SlotId, next_fill: SplitSlotFill) -> bool {
        match self {
            Self::Leaf { id, fill } if *id == target => {
                *fill = next_fill;
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.replace_leaf_fill(target, next_fill.clone())
                    || second.replace_leaf_fill(target, next_fill)
            }
        }
    }
}

impl SplitStudio {
    /// Build a fresh 1x1 studio seeded with `initial_window` (if present in `options`) or `NewShell`.
    pub fn new(
        anchor_session_id: SessionId,
        anchor_cwd: PathBuf,
        initial_window: Option<&WindowId>,
        mut window_options: Vec<SplitWindowOption>,
        previews: HashMap<WindowId, Vec<String>>,
    ) -> Self {
        window_options.sort_by_key(|opt| usize::from(opt.window.session_id != anchor_session_id));
        let initial_fill = initial_window
            .filter(|wid| window_options.iter().any(|opt| opt.window.id == **wid))
            .cloned()
            .map_or(SplitSlotFill::NewShell, SplitSlotFill::Window);
        let mut studio = Self {
            anchor_session_id,
            anchor_window_id: initial_window.cloned(),
            anchor_cwd,
            initial_fill: initial_fill.clone(),
            initial_root: None,
            root: SplitNode::new_leaf(1, initial_fill),
            next_slot_id: 2,
            selected_slot: 1,
            window_cursor: 0,
            window_options,
            editing_existing_panes: Vec::new(),
            previews,
            pane_previews: HashMap::new(),
            preview_scrolls: HashMap::new(),
            initial_sync_panes: false,
            sync_panes: false,
            preset_index: 0,
            slot_hits: Vec::new(),
            toolbar_hits: Vec::new(),
            window_list_rect: None,
        };
        studio.sync_cursor_to_selected_slot();
        studio
    }

    /// Build a Split Studio pre-loaded with an existing multi-pane window's real geometry layout.
    #[allow(clippy::too_many_arguments)]
    pub fn from_existing_panes_geometry(
        anchor_session_id: SessionId,
        anchor_window_id: WindowId,
        anchor_cwd: PathBuf,
        sync_panes: bool,
        geometries: &[(PaneId, u32, u32, u32, u32)],
        existing_panes: Vec<SplitPaneInfo>,
        active_pane: Option<&PaneId>,
        mut window_options: Vec<SplitWindowOption>,
        previews: HashMap<WindowId, Vec<String>>,
        pane_previews: HashMap<PaneId, Vec<String>>,
    ) -> Self {
        window_options.sort_by_key(|opt| usize::from(opt.window.session_id != anchor_session_id));
        let (root, next_slot_id) =
            SplitNode::from_pane_geometries(geometries).unwrap_or_else(|| {
                (
                    SplitNode::new_leaf(1, SplitSlotFill::Window(anchor_window_id.clone())),
                    2,
                )
            });
        let selected_slot = active_pane
            .and_then(|pid| root.find_slot_with_pane(pid))
            .unwrap_or_else(|| root.first_leaf().0);
        let initial_fill = SplitSlotFill::Window(anchor_window_id.clone());
        let mut studio = Self {
            anchor_session_id,
            anchor_window_id: Some(anchor_window_id),
            anchor_cwd,
            initial_fill,
            initial_root: Some(root.clone()),
            root,
            next_slot_id,
            selected_slot,
            window_cursor: 0,
            window_options,
            editing_existing_panes: existing_panes,
            previews,
            pane_previews,
            preview_scrolls: HashMap::new(),
            initial_sync_panes: sync_panes,
            sync_panes,
            preset_index: 0,
            slot_hits: Vec::new(),
            toolbar_hits: Vec::new(),
            window_list_rect: None,
        };
        studio.sync_cursor_to_selected_slot();
        studio
    }

    /// Build a Split Studio pre-loaded from real tmux [`Pane`]s in an existing multi-pane window.
    #[allow(clippy::too_many_arguments)]
    pub fn from_existing_window(
        anchor_session_id: SessionId,
        anchor_window_id: WindowId,
        anchor_cwd: PathBuf,
        sync_panes: bool,
        panes: &[Pane],
        active_pane: Option<&PaneId>,
        window_options: Vec<SplitWindowOption>,
        previews: HashMap<WindowId, Vec<String>>,
        pane_previews: HashMap<PaneId, Vec<String>>,
    ) -> Self {
        let geoms: Vec<(PaneId, u32, u32, u32, u32)> = panes
            .iter()
            .map(|p| (p.id.clone(), p.left, p.top, p.width, p.height))
            .collect();
        let existing_panes: Vec<SplitPaneInfo> = panes
            .iter()
            .map(|p| SplitPaneInfo {
                id: p.id.clone(),
                window_id: p.window_id.clone(),
                index: p.index,
                title: p.title.clone(),
                command: p.current_command.clone(),
                path: p.current_path.clone(),
            })
            .collect();
        let mut studio = Self::from_existing_panes_geometry(
            anchor_session_id,
            anchor_window_id,
            anchor_cwd,
            sync_panes,
            &geoms,
            existing_panes,
            active_pane,
            window_options,
            previews,
            pane_previews,
        );
        if let Some((root, next_slot_id)) = SplitNode::from_existing_panes(panes) {
            studio.selected_slot = active_pane
                .and_then(|pid| root.find_slot_with_pane(pid))
                .unwrap_or_else(|| root.first_leaf().0);
            studio.initial_root = Some(root.clone());
            studio.root = root;
            studio.next_slot_id = next_slot_id;
            studio.sync_cursor_to_selected_slot();
        }
        studio
    }

    /// Whether this Split Studio session was opened to edit an existing multi-pane window.
    pub fn is_editing_existing(&self) -> bool {
        !self.editing_existing_panes.is_empty()
    }

    /// Lookup metadata for an existing pane being edited in this studio.
    pub fn pane_info(&self, pane_id: &PaneId) -> Option<&SplitPaneInfo> {
        self.editing_existing_panes
            .iter()
            .find(|info| &info.id == pane_id)
    }

    /// Return the `PaneId` if the currently focused slot holds an existing pane.
    pub fn selected_existing_pane(&self) -> Option<PaneId> {
        match self.root.slot_fill(self.selected_slot) {
            Some(SplitSlotFill::Pane(pid)) => Some(pid.clone()),
            _ => None,
        }
    }

    /// Active split ratio (percentage of first child) for the currently selected slot.
    pub fn active_ratio(&self) -> u16 {
        self.root
            .parent_split_ratio(self.selected_slot)
            .unwrap_or(DEFAULT_SPLIT_RATIO)
    }

    /// Whether the selected slot sits inside a split whose ratio can be adjusted.
    pub fn can_adjust_ratio(&self) -> bool {
        self.root.parent_split_ratio(self.selected_slot).is_some()
    }

    /// Adjust the ratio of the split enclosing `selected_slot`.
    pub fn adjust_selected_ratio(&mut self, delta: i16) -> Option<u16> {
        self.root.adjust_ratio(self.selected_slot, delta)
    }

    /// Cycle the ratio of the split enclosing `selected_slot` (`50 -> 70 -> 30 -> 50`).
    pub fn cycle_selected_ratio(&mut self) -> Option<u16> {
        self.root.cycle_ratio_preset(self.selected_slot)
    }

    /// Look up `(direction, ratio)` of a split node by heap index (`root=0`, `first=2*i+1`, `second=2*i+2`).
    pub fn ratio_at_heap_index(&self, heap_idx: usize) -> Option<(SplitDirection, u16)> {
        self.root.split_ratio_at(heap_idx)
    }

    /// Update the ratio (`20..=80`) of a split node by heap index.
    pub fn set_ratio_at_heap_index(&mut self, heap_idx: usize, new_ratio: u16) -> bool {
        self.root.set_split_ratio_at(heap_idx, new_ratio)
    }

    /// Swap the fills of two split slots.
    pub fn swap_slots(&mut self, slot_a: SlotId, slot_b: SlotId) -> bool {
        if self.root.swap_slot_fills(slot_a, slot_b) {
            self.preview_scrolls.remove(&slot_a);
            self.preview_scrolls.remove(&slot_b);
            self.selected_slot = slot_b;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Remove a specific slot by `SlotId` (preserving at least 1 leaf).
    pub fn remove_slot_by_id(&mut self, slot_id: SlotId) -> bool {
        if let Some(next_focus) = self.root.remove_slot(slot_id) {
            self.preview_scrolls.remove(&slot_id);
            if self.selected_slot == slot_id {
                self.selected_slot = next_focus;
            }
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Dock `source_slot` onto an edge (`direction`, `before`) of `target_slot`.
    /// Falls back to swapping fills if 2x2 depth constraints prevent re-splitting.
    pub fn dock_slot_onto(
        &mut self,
        source_slot: SlotId,
        target_slot: SlotId,
        direction: SplitDirection,
        before: bool,
    ) -> bool {
        if source_slot == target_slot {
            return false;
        }
        let Some(src_fill) = self.root.slot_fill(source_slot).cloned() else {
            return false;
        };
        let mut candidate = self.root.clone();
        if candidate.remove_slot(source_slot).is_some() {
            let new_id = self.next_slot_id;
            if candidate.split_slot_ordered(target_slot, direction, before, new_id, src_fill) {
                self.root = candidate;
                self.next_slot_id += 1;
                self.preview_scrolls.remove(&source_slot);
                self.selected_slot = new_id;
                self.sync_cursor_to_selected_slot();
                return true;
            }
        }
        self.swap_slots(source_slot, target_slot)
    }

    /// Apply the next preset layout (`1x2 -> 2x1 -> Main+2 -> 2x2`) while preserving existing fills.
    pub fn apply_next_preset(&mut self) -> SplitLayoutPreset {
        let preset = SplitLayoutPreset::ALL[self.preset_index % SplitLayoutPreset::ALL.len()];
        self.preset_index = (self.preset_index + 1) % SplitLayoutPreset::ALL.len();
        let (next_id, first_slot) = self.root.apply_preset_layout(preset);
        self.next_slot_id = next_id;
        self.selected_slot = first_slot;
        self.preview_scrolls.clear();
        self.sync_cursor_to_selected_slot();
        preset
    }

    /// Toggle `synchronize-panes` intent for this Split Studio layout.
    pub fn toggle_sync_panes(&mut self) -> bool {
        self.sync_panes = !self.sync_panes;
        self.sync_panes
    }

    /// Existing windows plus final `+ New shell` option row.
    pub fn total_window_rows(&self) -> usize {
        self.window_options.len() + 1
    }

    /// Resolve which fill corresponds to row `row` in the bottom window selector.
    pub fn fill_for_row(&self, row: usize) -> SplitSlotFill {
        self.window_options
            .get(row)
            .map_or(SplitSlotFill::NewShell, |opt| {
                SplitSlotFill::Window(opt.window.id.clone())
            })
    }

    /// Point `window_cursor` to whatever fills `selected_slot`.
    pub fn sync_cursor_to_selected_slot(&mut self) {
        self.window_cursor = match self.root.slot_fill(self.selected_slot) {
            Some(SplitSlotFill::Window(wid)) => self
                .window_options
                .iter()
                .position(|opt| opt.window.id == *wid)
                .unwrap_or(self.window_options.len()),
            Some(SplitSlotFill::Pane(pid)) => self
                .pane_info(pid)
                .and_then(|info| {
                    self.window_options
                        .iter()
                        .position(|opt| opt.window.id == info.window_id)
                })
                .unwrap_or(self.window_options.len()),
            Some(SplitSlotFill::NewShell) | None => self.window_options.len(),
        };
    }

    /// Split currently focused slot in `direction` and move focus to the newly created `NewShell` slot.
    pub fn split_selected(&mut self, direction: SplitDirection) -> bool {
        let new_id = self.next_slot_id;
        if self.root.split_slot(
            self.selected_slot,
            direction,
            new_id,
            SplitSlotFill::NewShell,
        ) {
            self.next_slot_id += 1;
            self.selected_slot = new_id;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Remove currently focused slot if more than 1 slot exists.
    pub fn remove_selected(&mut self) -> bool {
        if let Some(next_focus) = self.root.remove_slot(self.selected_slot) {
            self.preview_scrolls.remove(&self.selected_slot);
            self.selected_slot = next_focus;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Reset back to initial layout (or 1x1 when not editing an existing multi-pane window).
    pub fn reset(&mut self) {
        if let Some(initial_root) = self.initial_root.clone() {
            let max_id = initial_root
                .leaves()
                .into_iter()
                .map(|(id, _)| id)
                .max()
                .unwrap_or(1);
            self.selected_slot = initial_root.first_leaf().0;
            self.root = initial_root;
            self.next_slot_id = max_id + 1;
        } else {
            self.root = SplitNode::new_leaf(1, self.initial_fill.clone());
            self.next_slot_id = 2;
            self.selected_slot = 1;
        }
        self.preview_scrolls.clear();
        self.sync_cursor_to_selected_slot();
    }

    /// Select slot by `SlotId`.
    pub fn select_slot(&mut self, slot_id: SlotId) -> bool {
        if self.root.slot_fill(slot_id).is_some() {
            self.selected_slot = slot_id;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Cycle focus forward or backward through slots in visual reading order.
    pub fn cycle_slot(&mut self, forward: bool) {
        let leaves = self.root.leaves();
        if leaves.is_empty() {
            return;
        }
        let pos = leaves
            .iter()
            .position(|(id, _)| *id == self.selected_slot)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % leaves.len()
        } else {
            (pos + leaves.len() - 1) % leaves.len()
        };
        self.selected_slot = leaves[next].0;
        self.sync_cursor_to_selected_slot();
    }

    /// Focus slot by 1-based visual number (`1..=4`).
    pub fn select_slot_by_number(&mut self, one_based: usize) -> bool {
        let leaves = self.root.leaves();
        if let Some((slot_id, _)) = one_based.checked_sub(1).and_then(|idx| leaves.get(idx)) {
            self.select_slot(*slot_id)
        } else {
            false
        }
    }

    /// Return 1-based visual index (`1..=4`) for `slot_id`.
    pub fn slot_ordinal(&self, slot_id: SlotId) -> Option<usize> {
        self.root
            .leaves()
            .iter()
            .position(|(id, _)| *id == slot_id)
            .map(|idx| idx + 1)
    }

    /// Navigate candidate window list with circular wrap-around: auto-fills free windows or `+ New shell`, without stealing occupied windows or clobbering existing edited panes.
    pub fn move_window_cursor(&mut self, up: bool) {
        let total = self.total_window_rows();
        if total == 0 {
            return;
        }
        let next = super::wrap_step(self.window_cursor, total, if up { -1 } else { 1 });
        self.window_cursor = next;
        if matches!(
            self.root.slot_fill(self.selected_slot),
            Some(SplitSlotFill::Pane(_))
        ) {
            return;
        }
        let candidate = self.fill_for_row(next);
        let should_autofill = match &candidate {
            SplitSlotFill::NewShell => true,
            SplitSlotFill::Window(wid) => self
                .root
                .find_slot_with_window(wid)
                .is_none_or(|owner| owner == self.selected_slot),
            SplitSlotFill::Pane(pid) => self
                .root
                .find_slot_with_pane(pid)
                .is_none_or(|owner| owner == self.selected_slot),
        };
        if should_autofill {
            let previous = self.root.slot_fill(self.selected_slot).cloned();
            if previous.as_ref() != Some(&candidate)
                && self.root.set_slot_fill(self.selected_slot, candidate)
            {
                self.preview_scrolls.remove(&self.selected_slot);
            }
        }
    }

    /// Explicitly assign row `row` to the selected slot, swapping if held by another slot.
    pub fn assign_cursor_row(&mut self, row: usize) -> bool {
        if row >= self.total_window_rows() {
            return false;
        }
        self.window_cursor = row;
        let fill = self.fill_for_row(row);
        let changed = self.root.slot_fill(self.selected_slot) != Some(&fill);
        let applied = self.root.set_slot_fill(self.selected_slot, fill);
        if applied && changed {
            self.preview_scrolls.remove(&self.selected_slot);
        }
        applied
    }

    /// Scroll preview history inside `slot_id`.
    pub fn scroll_slot_preview(&mut self, slot_id: SlotId, up: bool, step: usize, max: usize) {
        let entry = self.preview_scrolls.entry(slot_id).or_insert(0);
        if up {
            *entry = entry.saturating_add(step).min(max);
        } else {
            *entry = (*entry).min(max).saturating_sub(step);
        }
    }

    /// Flip horizontal and vertical split directions across the layout tree.
    pub fn flip_layout(&mut self) -> bool {
        self.root.flip_directions()
    }

    /// Lookup window option metadata by ID.
    pub fn window_option(&self, window_id: &WindowId) -> Option<&SplitWindowOption> {
        self.window_options
            .iter()
            .find(|opt| &opt.window.id == window_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(raw: &str) -> SessionId {
        SessionId::new(raw).expect("valid session id")
    }

    fn wid(raw: &str) -> WindowId {
        WindowId::new(raw).expect("valid window id")
    }

    fn win(raw_sid: &str, sname: &str, raw_wid: &str, index: u32, name: &str) -> SplitWindowOption {
        SplitWindowOption {
            window: Window {
                id: wid(raw_wid),
                session_id: sid(raw_sid),
                session_name: sname.into(),
                index,
                name: name.into(),
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
            },
            active_pane_id: Some(PaneId::new(format!("%{}", index + 1)).expect("pane id")),
            command: "zsh".into(),
            path: "/tmp/demo".into(),
        }
    }

    #[test]
    fn split_node_enforces_max_2x2_grid_and_collapses_on_remove() {
        let mut tree = SplitNode::new_leaf(1, SplitSlotFill::Window(wid("@1")));
        assert!(tree.can_split(1, SplitDirection::SideBySide));
        assert!(tree.can_split(1, SplitDirection::Stacked));

        // 1 -> 2 columns (1 | 2)
        assert!(tree.split_slot(1, SplitDirection::SideBySide, 2, SplitSlotFill::NewShell));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
        assert!(!tree.can_split(2, SplitDirection::SideBySide));
        assert!(tree.can_split(1, SplitDirection::Stacked));
        assert!(tree.can_split(2, SplitDirection::Stacked));

        // 2 columns -> split left vertically (1/3 | 2), then right vertically (1/3 | 2/4)
        assert!(tree.split_slot(1, SplitDirection::Stacked, 3, SplitSlotFill::NewShell));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
        assert!(!tree.can_split(1, SplitDirection::Stacked));
        assert!(!tree.can_split(3, SplitDirection::Stacked));
        assert!(tree.split_slot(2, SplitDirection::Stacked, 4, SplitSlotFill::NewShell));
        assert_eq!(tree.leaf_count(), 4);
        for leaf in [1, 2, 3, 4] {
            assert!(!tree.can_split(leaf, SplitDirection::SideBySide));
            assert!(!tree.can_split(leaf, SplitDirection::Stacked));
        }

        // Row-first then column split also reaches 2x2 cleanly.
        let mut row_first = SplitNode::new_leaf(1, SplitSlotFill::NewShell);
        assert!(row_first.split_slot(1, SplitDirection::Stacked, 2, SplitSlotFill::NewShell));
        assert!(!row_first.can_split(1, SplitDirection::Stacked));
        assert!(row_first.split_slot(1, SplitDirection::SideBySide, 3, SplitSlotFill::NewShell));
        assert!(row_first.split_slot(2, SplitDirection::SideBySide, 4, SplitSlotFill::NewShell));
        assert_eq!(row_first.leaf_count(), 4);

        // Removing slot 3 collapses its vertical split back to 1 | (2/4).
        let next = tree.remove_slot(3).expect("removed slot 3");
        assert_eq!(next, 1);
        assert_eq!(tree.leaf_count(), 3);
        assert!(tree.can_split(1, SplitDirection::Stacked));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
    }

    #[test]
    fn studio_orders_anchor_session_first_auto_fills_free_windows_and_swaps_on_explicit_assign() {
        let w_other = win("$2", "ops", "@20", 0, "logs");
        let w_a0 = win("$1", "dev", "@10", 0, "editor");
        let w_a1 = win("$1", "dev", "@11", 1, "server");

        let mut studio = SplitStudio::new(
            sid("$1"),
            PathBuf::from("/tmp/dev"),
            Some(&wid("@10")),
            vec![w_other, w_a0, w_a1],
            HashMap::new(),
        );
        // Anchor session ($1) sorted first cleanly.
        assert_eq!(studio.window_options[0].window.id, wid("@10"));
        assert_eq!(studio.window_options[1].window.id, wid("@11"));
        assert_eq!(studio.window_options[2].window.id, wid("@20"));
        assert_eq!(studio.selected_slot, 1);
        assert_eq!(studio.window_cursor, 0);

        // Split H -> creates slot 2 (NewShell) and focuses it (row 3 = + New shell).
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert_eq!(studio.selected_slot, 2);
        assert_eq!(studio.window_cursor, 3);
        assert_eq!(studio.root.slot_fill(2), Some(&SplitSlotFill::NewShell));

        // Moving Up to row 2 (@20, unassigned) auto-fills slot 2 without stealing anything.
        studio.move_window_cursor(true);
        assert_eq!(studio.window_cursor, 2);
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@20")))
        );

        // Moving Up twice onto row 0 (@10, owned by slot 1) moves cursor ONLY and does NOT steal @10!
        studio.move_window_cursor(true);
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@11")))
        );
        studio.move_window_cursor(true);
        assert_eq!(studio.window_cursor, 0);
        assert_eq!(
            studio.root.slot_fill(1),
            Some(&SplitSlotFill::Window(wid("@10")))
        );
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@11")))
        );

        // Explicit assign (Space / click) on occupied row 0 SWAPS slot 1 and slot 2 cleanly!
        assert!(studio.assign_cursor_row(0));
        assert_eq!(
            studio.root.slot_fill(1),
            Some(&SplitSlotFill::Window(wid("@11")))
        );
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@10")))
        );

        // Window cursor wraps circularly across 0..total_window_rows().
        assert_eq!(studio.window_cursor, 0);
        studio.move_window_cursor(true);
        assert_eq!(studio.window_cursor, studio.total_window_rows() - 1);
        studio.move_window_cursor(false);
        assert_eq!(studio.window_cursor, 0);
    }

    #[test]
    fn flip_directions_and_studio_flip_layout_transpose_and_preserve_2x2_caps() {
        let w1 = win("$1", "dev", "@10", 0, "editor");
        let w2 = win("$1", "dev", "@11", 1, "server");

        let mut studio = SplitStudio::new(
            sid("$1"),
            PathBuf::from("/tmp/dev"),
            Some(&wid("@10")),
            vec![w1, w2],
            HashMap::new(),
        );
        // 1x1 layout cannot flip.
        assert!(!studio.root.can_flip());
        assert!(!studio.flip_layout());

        // 1x2 (`1 | 2`) flips to 2x1 (`1 / 2`) and flips back.
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert!(studio.root.can_flip());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::SideBySide,
                ..
            }
        ));
        assert!(studio.flip_layout());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::Stacked,
                ..
            }
        ));
        assert!(studio.flip_layout());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::SideBySide,
                ..
            }
        ));

        // 3-pane flip: `(1 | (2 / 3))` -> `(1 / (2 | 3))` preserves slot caps accurately.
        assert!(studio.select_slot_by_number(2));
        assert!(studio.split_selected(SplitDirection::Stacked));
        assert_eq!(studio.root.leaf_count(), 3);
        assert!(studio.flip_layout());
        assert!(studio.root.can_split(1, SplitDirection::SideBySide));
        assert!(!studio.root.can_split(1, SplitDirection::Stacked));
        assert!(!studio.root.can_split(2, SplitDirection::SideBySide));
        assert!(!studio.root.can_split(2, SplitDirection::Stacked));

        // Reach 2x2 (`(1 | 4) / (2 | 3)`) and verify double flip restores exact tree.
        assert!(studio.select_slot_by_number(1));
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert_eq!(studio.root.leaf_count(), 4);
        for leaf in [1, 2, 3, 4] {
            assert!(!studio.root.can_split(leaf, SplitDirection::SideBySide));
            assert!(!studio.root.can_split(leaf, SplitDirection::Stacked));
        }
        let before_2x2 = studio.root.clone();
        assert!(studio.flip_layout());
        assert_ne!(studio.root, before_2x2);
        for leaf in [1, 2, 3, 4] {
            assert!(!studio.root.can_split(leaf, SplitDirection::SideBySide));
            assert!(!studio.root.can_split(leaf, SplitDirection::Stacked));
        }
        assert!(studio.flip_layout());
        assert_eq!(studio.root, before_2x2);
    }

    #[test]
    fn prune_missing_windows_resets_vanished_leaves_to_new_shell() {
        let mut tree = SplitNode::new_leaf(1, SplitSlotFill::Window(wid("@10")));
        assert!(tree.split_slot(
            1,
            SplitDirection::SideBySide,
            2,
            SplitSlotFill::Window(wid("@11"))
        ));
        assert!(tree.split_slot(2, SplitDirection::Stacked, 3, SplitSlotFill::NewShell));

        let live: HashSet<WindowId> = [wid("@10")].into_iter().collect();
        tree.prune_missing_windows(&live);

        assert_eq!(tree.slot_fill(1), Some(&SplitSlotFill::Window(wid("@10"))));
        assert_eq!(tree.slot_fill(2), Some(&SplitSlotFill::NewShell));
        assert_eq!(tree.slot_fill(3), Some(&SplitSlotFill::NewShell));
    }

    #[test]
    fn ratios_presets_and_geometry_reconstruction_preserve_2x2_invariants() {
        let p1 = PaneId::new("%1").expect("p1");
        let p2 = PaneId::new("%2").expect("p2");
        let p3 = PaneId::new("%3").expect("p3");
        let p4 = PaneId::new("%4").expect("p4");

        // 70/30 side-by-side reconstruction
        let (mut tree, next_id) = SplitNode::from_pane_geometries(&[
            (p1.clone(), 0, 0, 70, 40),
            (p2.clone(), 71, 0, 29, 40),
        ])
        .expect("2-pane tree");
        assert_eq!(next_id, 3);
        assert_eq!(tree.leaf_count(), 2);
        assert_eq!(tree.parent_split_ratio(1), Some(70));
        assert_eq!(tree.adjust_ratio(1, 15), Some(80)); // clamped to MAX_SPLIT_RATIO
        assert_eq!(tree.adjust_ratio(1, -70), Some(20)); // clamped to MIN_SPLIT_RATIO
        assert_eq!(tree.cycle_ratio_preset(1), Some(50));
        assert_eq!(tree.cycle_ratio_preset(1), Some(70));
        assert_eq!(tree.cycle_ratio_preset(1), Some(30));
        assert_eq!(tree.cycle_ratio_preset(1), Some(50));

        // Main + 2 reconstruction
        let (main_plus_two, _) = SplitNode::from_pane_geometries(&[
            (p1.clone(), 0, 0, 60, 40),
            (p2.clone(), 61, 0, 39, 19),
            (p3.clone(), 61, 20, 39, 20),
        ])
        .expect("3-pane tree");
        assert_eq!(main_plus_two.leaf_count(), 3);
        assert_eq!(main_plus_two.find_slot_with_pane(&p1), Some(1));
        assert_eq!(main_plus_two.parent_split_ratio(1), Some(60));

        // 2x2 grid reconstruction
        let (grid, _) = SplitNode::from_pane_geometries(&[
            (p1.clone(), 0, 0, 49, 19),
            (p2.clone(), 50, 0, 50, 19),
            (p3.clone(), 0, 20, 49, 20),
            (p4.clone(), 50, 20, 50, 20),
        ])
        .expect("4-pane tree");
        assert_eq!(grid.leaf_count(), 4);
        for id in 1..=4 {
            assert!(!grid.can_split(id, SplitDirection::SideBySide));
            assert!(!grid.can_split(id, SplitDirection::Stacked));
        }

        // Apply presets preserving fills
        let mut studio = SplitStudio::new(
            sid("$1"),
            PathBuf::from("/tmp/dev"),
            Some(&wid("@10")),
            vec![win("$1", "dev", "@10", 0, "editor")],
            HashMap::new(),
        );
        assert!(!studio.sync_panes);
        assert!(studio.toggle_sync_panes());
        assert!(studio.sync_panes);

        assert_eq!(studio.apply_next_preset(), SplitLayoutPreset::OneByTwo);
        assert_eq!(studio.root.leaf_count(), 2);
        assert_eq!(
            studio.root.slot_fill(1),
            Some(&SplitSlotFill::Window(wid("@10")))
        );
        assert_eq!(studio.apply_next_preset(), SplitLayoutPreset::TwoByOne);
        assert_eq!(studio.root.leaf_count(), 2);
        assert_eq!(studio.apply_next_preset(), SplitLayoutPreset::MainPlusTwo);
        assert_eq!(studio.root.leaf_count(), 3);
        assert_eq!(studio.active_ratio(), 60);
        assert_eq!(studio.apply_next_preset(), SplitLayoutPreset::TwoByTwo);
        assert_eq!(studio.root.leaf_count(), 4);

        // Heap-index ratio setter & drag-dock / swap helpers
        assert_eq!(
            studio.ratio_at_heap_index(0),
            Some((SplitDirection::SideBySide, 50))
        );
        assert!(studio.set_ratio_at_heap_index(0, 68));
        assert_eq!(
            studio.ratio_at_heap_index(0),
            Some((SplitDirection::SideBySide, 68))
        );
        assert!(studio.set_ratio_at_heap_index(1, 35));
        assert_eq!(
            studio.ratio_at_heap_index(1),
            Some((SplitDirection::Stacked, 35))
        );
        assert!(studio.swap_slots(1, 2));
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@10")))
        );
    }
}
