//! Window-manager state machine.
//!
//! Inputs: window events reported by the platform layer and dispatched
//! actions. Output: a `Desired` snapshot (where every managed window should
//! be, which windows are hidden, what to focus). The Windows layer
//! reconciles real windows with that snapshot. Nothing here touches the OS.
//!
//! Model (after Hyprland): workspaces are numbered 1-99 and belong to a
//! monitor; each monitor shows exactly one workspace; a window belongs to
//! one workspace and is either tiled (owned by the workspace layout) or
//! floating. Inactive workspaces' windows are hidden by the platform.

use crate::actions::{Action, MonTarget, WsTarget};
use crate::config::Config;
use crate::geom::{find_in_direction, Direction, Rect};
use crate::layout::master::Orientation;
use crate::layout::{apply_gaps, Layout, LayoutKind};
use crate::rules::{Classification, Decision, WindowFacts};
use crate::WinId;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct Monitor {
    /// Stable identifier (Windows device name, e.g. `\\.\DISPLAY1`).
    pub id: String,
    pub rect: Rect,
    /// Work area (excludes the taskbar).
    pub work: Rect,
    pub dpi: u32,
    pub active_ws: u32,
}

impl Monitor {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Tiled,
    Floating,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManagedWindow {
    pub id: WinId,
    pub facts: WindowFacts,
    pub ws: u32,
    pub mode: Mode,
    /// Visual rectangle used while floating.
    pub float_rect: Rect,
    pub minimized: bool,
    pub border: bool,
    pub matched_rules: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Workspace {
    pub id: u32,
    pub monitor: usize,
    pub layout: Layout,
    pub last_focus: Option<WinId>,
    pub fullscreen: Option<WinId>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub id: WinId,
    /// Where the window's visible frame should be.
    pub rect: Rect,
    /// Outer rectangle of the border ring, if a border is drawn.
    pub border: Option<Rect>,
    pub floating: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Desired {
    pub placements: Vec<Placement>,
    pub hidden: BTreeSet<WinId>,
}

/// Side effects the platform performs on behalf of an action.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Effect {
    Exec(String),
    Close(WinId),
    Reload,
    Quit,
    OpenSettings,
    OpenConfig,
    /// Paused state changed (platform restores/re-applies everything).
    Paused(bool),
    /// Workspace switch on a monitor, for the slide animation.
    WorkspaceSwitch { monitor: usize, from: u32, to: u32 },
}

#[derive(Debug, Serialize)]
pub struct Wm {
    pub monitors: Vec<Monitor>,
    pub workspaces: BTreeMap<u32, Workspace>,
    pub windows: BTreeMap<WinId, ManagedWindow>,
    pub focused: Option<WinId>,
    pub focused_monitor: usize,
    pub previous_ws: Option<u32>,
    pub paused: bool,
    /// Focus change requested by an action; consumed by the platform.
    #[serde(skip)]
    pub focus_request: Option<WinId>,
    #[serde(skip)]
    cfg: Config,
}

impl Wm {
    pub fn new(cfg: Config, monitors: Vec<Monitor>) -> Wm {
        let mut wm = Wm {
            monitors: Vec::new(),
            workspaces: BTreeMap::new(),
            windows: BTreeMap::new(),
            focused: None,
            focused_monitor: 0,
            previous_ws: None,
            paused: false,
            focus_request: None,
            cfg,
        };
        wm.set_monitors(monitors);
        wm
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Applies a new configuration. Layout kinds of existing workspaces are
    /// kept; spacing, borders and rules take effect immediately.
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
        for ws in self.workspaces.values_mut() {
            if let Layout::Master(m) = &mut ws.layout {
                m.mfact = self.cfg.master.mfact;
                m.orientation = Orientation::parse(&self.cfg.master.orientation).unwrap_or_default();
            }
        }
    }

    // ------------------------------------------------------------ monitors

    fn ws_rule_monitor(&self, id: u32) -> Option<usize> {
        self.cfg.workspaces.iter().find(|w| w.id == id).and_then(|w| w.monitor).map(|m| m.saturating_sub(1)).filter(|m| *m < self.monitors.len())
    }

    fn ws_rule_layout(&self, id: u32) -> LayoutKind {
        self.cfg.workspaces.iter().find(|w| w.id == id).and_then(|w| w.layout.as_deref()).and_then(LayoutKind::parse).unwrap_or(self.cfg.default_layout)
    }

    fn ensure_ws(&mut self, id: u32, monitor: usize) -> &mut Workspace {
        if !self.workspaces.contains_key(&id) {
            let monitor = self.ws_rule_monitor(id).unwrap_or(monitor);
            let layout = Layout::new(self.ws_rule_layout(id), &self.cfg.layout_options());
            self.workspaces.insert(id, Workspace { id, monitor, layout, last_focus: None, fullscreen: None });
        }
        self.workspaces.get_mut(&id).unwrap()
    }

    fn first_free_ws(&self, prefer: u32) -> u32 {
        let active: BTreeSet<u32> = self.monitors.iter().map(|m| m.active_ws).collect();
        let free = |id: u32| !active.contains(&id) && self.workspaces.get(&id).is_none_or(|w| w.layout.is_empty() && !self.windows.values().any(|x| x.ws == id));
        if free(prefer) {
            return prefer;
        }
        (1..=99).find(|i| free(*i)).unwrap_or(99)
    }

    /// Replaces the monitor list (startup, display change). Monitors are
    /// matched by id; workspaces of removed monitors move to the first
    /// monitor; new monitors get a free workspace.
    pub fn set_monitors(&mut self, mut new: Vec<Monitor>) {
        if new.is_empty() {
            return; // keep the old layout during transient "no monitor" states
        }
        let old = std::mem::take(&mut self.monitors);
        let mut index_map: BTreeMap<usize, usize> = BTreeMap::new();
        for (ni, m) in new.iter_mut().enumerate() {
            if let Some((oi, o)) = old.iter().enumerate().find(|(_, o)| o.id == m.id) {
                m.active_ws = o.active_ws;
                index_map.insert(oi, ni);
            } else {
                m.active_ws = 0;
            }
        }
        self.monitors = new;
        for ws in self.workspaces.values_mut() {
            ws.monitor = index_map.get(&ws.monitor).copied().unwrap_or(0);
        }
        // Monitors whose active workspace moved away or which are new.
        for i in 0..self.monitors.len() {
            let a = self.monitors[i].active_ws;
            let valid = a != 0 && self.workspaces.get(&a).is_none_or(|w| w.monitor == i);
            if !valid {
                let want = self.workspaces.values().find(|w| w.monitor == i).map(|w| w.id).filter(|id| !self.monitors.iter().any(|m| m.active_ws == *id));
                let id = want.unwrap_or_else(|| self.first_free_ws(i as u32 + 1));
                self.monitors[i].active_ws = id;
                self.ensure_ws(id, i).monitor = i;
            }
        }
        if self.focused_monitor >= self.monitors.len() {
            self.focused_monitor = 0;
        }
        self.cleanup();
    }

    pub fn monitor_at(&self, x: i32, y: i32) -> usize {
        self.monitors.iter().position(|m| m.rect.contains(x, y)).unwrap_or_else(|| {
            // Nearest monitor by centre distance.
            let d = |m: &Monitor| {
                let (cx, cy) = m.rect.center();
                (cx - x) as i64 * (cx - x) as i64 + (cy - y) as i64 * (cy - y) as i64
            };
            self.monitors.iter().enumerate().min_by_key(|(_, m)| d(m)).map(|(i, _)| i).unwrap_or(0)
        })
    }

    pub fn active_ws(&self) -> u32 {
        self.monitors[self.focused_monitor].active_ws
    }

    fn is_visible_ws(&self, ws: u32) -> bool {
        self.monitors.iter().any(|m| m.active_ws == ws)
    }

    /// Removes empty, inactive workspaces (Hyprland does the same).
    fn cleanup(&mut self) {
        let active: BTreeSet<u32> = self.monitors.iter().map(|m| m.active_ws).collect();
        let used: BTreeSet<u32> = self.windows.values().map(|w| w.ws).collect();
        self.workspaces.retain(|id, _| active.contains(id) || used.contains(id));
    }

    // ------------------------------------------------------------ windows

    /// Monitor work area minus space reserved by `[[monitor]]` rules.
    pub fn usable_area(&self, monitor: usize) -> Rect {
        let mon = &self.monitors[monitor.min(self.monitors.len() - 1)];
        let s = mon.scale();
        match self.cfg.monitors.iter().find(|r| r.index == monitor + 1) {
            Some(r) => {
                let f = |v: i32| (v as f32 * s).round() as i32;
                let rv = r.reserved;
                mon.work.inset(f(rv.left), f(rv.top), f(rv.right), f(rv.bottom))
            }
            None => mon.work,
        }
    }

    fn tiling_area(&self, ws: u32) -> Rect {
        let m = self.workspaces.get(&ws).map(|w| w.monitor).unwrap_or(0).min(self.monitors.len() - 1);
        let s = self.monitors[m].scale();
        let g = (self.cfg.general.gaps_out as f32 * s).round() as i32;
        self.usable_area(m).shrink(g)
    }

    /// Starts managing a window. `adopting` is true for windows that
    /// already existed when the manager started: they stay on the monitor
    /// they are on; new windows open on the focused monitor (Hyprland).
    pub fn add_window(&mut self, facts: WindowFacts, class: &Classification, adopting: bool) -> bool {
        if class.decision == Decision::Ignore || self.windows.contains_key(&facts.hwnd) {
            return false;
        }
        let id = facts.hwnd;
        let (cx, cy) = facts.rect.center();
        let here = self.monitor_at(cx, cy);
        let mon = match class.monitor {
            Some(m) if m >= 1 && m <= self.monitors.len() => m - 1,
            _ if adopting => here,
            _ => self.focused_monitor,
        };
        let ws = class.workspace.unwrap_or(self.monitors[mon].active_ws);
        self.ensure_ws(ws, mon);
        let mode = if class.decision == Decision::Float { Mode::Floating } else { Mode::Tiled };
        let mut float_rect = facts.rect;
        let ws_mon = self.workspaces[&ws].monitor;
        let work = self.usable_area(ws_mon);
        if let Some((w, h)) = class.float_size {
            float_rect = Rect::new(float_rect.x, float_rect.y, w, h);
        }
        if class.center || work.intersection(&float_rect).is_none() {
            float_rect = work.centered(float_rect.w, float_rect.h);
        }
        let minimized = facts.minimized;
        self.windows.insert(
            id,
            ManagedWindow { id, facts, ws, mode, float_rect, minimized, border: class.border, matched_rules: class.matched_rules.clone() },
        );
        if mode == Mode::Tiled && !minimized {
            self.insert_tiled(id, ws);
        }
        if !adopting && self.cfg.general.focus_new_windows && !minimized {
            if self.is_visible_ws(ws) {
                self.focus_window(id, false);
            } else {
                self.workspaces.get_mut(&ws).unwrap().last_focus = Some(id);
            }
        }
        true
    }

    fn insert_tiled(&mut self, id: WinId, ws: u32) {
        let area = self.tiling_area(ws);
        let opts = self.cfg.layout_options();
        let focus = self.focused.filter(|f| self.windows.get(f).is_some_and(|w| w.ws == ws && w.mode == Mode::Tiled));
        let w = self.workspaces.get_mut(&ws).unwrap();
        let target = focus.or(w.last_focus);
        w.layout.insert(id, target, area, &opts);
        w.fullscreen = None;
    }

    fn detach(&mut self, id: WinId) {
        if let Some(w) = self.windows.get(&id) {
            if let Some(ws) = self.workspaces.get_mut(&w.ws) {
                ws.layout.remove(id);
                if ws.fullscreen == Some(id) {
                    ws.fullscreen = None;
                }
                if ws.last_focus == Some(id) {
                    ws.last_focus = None;
                }
            }
        }
    }

    pub fn remove_window(&mut self, id: WinId) -> bool {
        if !self.windows.contains_key(&id) {
            return false;
        }
        let ws = self.windows[&id].ws;
        let was_focused = self.focused == Some(id);
        self.detach(id);
        self.windows.remove(&id);
        if was_focused {
            self.focused = None;
            // Hyprland falls back to another window on the same workspace.
            if let Some(next) = self.fallback_focus(ws) {
                self.focus_window(next, true);
            }
        }
        self.cleanup();
        true
    }

    fn fallback_focus(&self, ws: u32) -> Option<WinId> {
        let w = self.workspaces.get(&ws)?;
        w.last_focus
            .filter(|id| self.windows.get(id).is_some_and(|x| !x.minimized))
            .or_else(|| w.layout.windows().into_iter().next())
            .or_else(|| self.windows.values().find(|x| x.ws == ws && !x.minimized).map(|x| x.id))
    }

    pub fn update_facts(&mut self, facts: WindowFacts) {
        if let Some(w) = self.windows.get_mut(&facts.hwnd) {
            w.facts = facts;
        }
    }

    pub fn set_minimized(&mut self, id: WinId, minimized: bool) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        if w.minimized == minimized {
            return;
        }
        w.minimized = minimized;
        let (ws, mode) = (w.ws, w.mode);
        if minimized {
            self.detach(id);
            if self.focused == Some(id) {
                self.focused = None;
            }
        } else if mode == Mode::Tiled {
            self.insert_tiled(id, ws);
        }
    }

    /// The user (or the OS) activated a window. Switching to its workspace
    /// if it is hidden keeps taskbar/Alt+Tab activation working.
    /// A managed window became the foreground window. `user_restored` is
    /// true when the user brought it back themselves (taskbar click on a
    /// window hidden by minimizing).
    pub fn on_focus(&mut self, id: Option<WinId>, user_restored: bool) {
        match id {
            Some(id) if self.windows.contains_key(&id) => {
                let ws = self.windows[&id].ws;
                if !self.is_visible_ws(ws) {
                    if !(self.cfg.general.focus_on_activate || user_restored) {
                        // The app activated itself while its workspace is hidden:
                        // stay here and give focus back to the visible window.
                        let back = self.focused_ws_window().or_else(|| self.workspaces.get(&self.active_ws()).and_then(|w| w.last_focus));
                        if let Some(b) = back {
                            self.focus_request = Some(b);
                        }
                        return;
                    }
                    let m = self.workspaces[&ws].monitor;
                    self.switch_ws_on(m, ws);
                }
                self.focus_window(id, false);
            }
            _ => {}
        }
    }

    fn focus_window(&mut self, id: WinId, request: bool) {
        let Some(w) = self.windows.get(&id) else { return };
        let ws = w.ws;
        self.focused = Some(id);
        if let Some(wsp) = self.workspaces.get_mut(&ws) {
            wsp.last_focus = Some(id);
            self.focused_monitor = wsp.monitor;
        }
        if request {
            self.focus_request = Some(id);
        }
    }

    pub fn take_focus_request(&mut self) -> Option<WinId> {
        self.focus_request.take()
    }

    /// End of an interactive move/resize. `cursor` is where the mouse was
    /// released. Floating windows keep their new rectangle; tiled windows
    /// are swapped with the tile under the cursor (move), adopt the new
    /// size by adjusting split ratios (resize), or snap back.
    pub fn on_user_move(&mut self, id: WinId, rect: Rect, cursor: (i32, i32), resized: bool) {
        let Some(w) = self.windows.get(&id) else { return };
        if w.mode == Mode::Floating {
            let m = self.monitor_at(rect.center().0, rect.center().1);
            let target_ws = self.monitors[m].active_ws;
            let w = self.windows.get_mut(&id).unwrap();
            w.float_rect = rect;
            if w.ws != target_ws {
                self.move_to_ws(id, target_ws);
            }
            return;
        }
        let ws = w.ws;
        if resized {
            let area = self.tiling_area(ws);
            let tiles = self.tiles(ws);
            let Some(cur) = tiles.iter().find(|(i, _)| *i == id).map(|(_, r)| *r) else { return };
            let opts = self.cfg.layout_options();
            // Gaps are configured at 96 DPI and scaled per monitor (see `tiles`).
            let s = self.monitors[self.workspaces[&ws].monitor.min(self.monitors.len() - 1)].scale();
            let g = (self.cfg.general.gaps_in as f32 * s).round() as i32;
            let wsp = self.workspaces.get_mut(&ws).unwrap();
            match &mut wsp.layout {
                Layout::Dwindle(d) => {
                    // Translate visual edges back to tile edges (gaps).
                    let tgt = Rect::from_ltrb(
                        if cur.x > area.x { rect.x - g } else { area.x },
                        if cur.y > area.y { rect.y - g } else { area.y },
                        if cur.right() < area.right() { rect.right() + g } else { area.right() },
                        if cur.bottom() < area.bottom() { rect.bottom() + g } else { area.bottom() },
                    );
                    d.fit_to(id, area, tgt, &opts.dwindle);
                }
                Layout::Master(m) => {
                    let is_master = m.order.iter().position(|x| *x == id).is_some_and(|i| i < m.masters);
                    let f = match m.orientation {
                        Orientation::Left if is_master => (rect.right() - area.x) as f32 / area.w as f32,
                        Orientation::Left => (rect.x - area.x) as f32 / area.w as f32,
                        Orientation::Right if is_master => (area.right() - rect.x) as f32 / area.w as f32,
                        Orientation::Right => (area.right() - rect.right()) as f32 / area.w as f32,
                        Orientation::Top if is_master => (rect.bottom() - area.y) as f32 / area.h as f32,
                        Orientation::Top => (rect.y - area.y) as f32 / area.h as f32,
                        Orientation::Bottom if is_master => (area.bottom() - rect.y) as f32 / area.h as f32,
                        Orientation::Bottom => (area.bottom() - rect.bottom()) as f32 / area.h as f32,
                        Orientation::Center => m.mfact,
                    };
                    m.adjust_mfact(f, true);
                }
                Layout::Monocle(_) => {}
            }
            return;
        }
        if !self.cfg.general.drag_to_swap {
            return;
        }
        let m = self.monitor_at(cursor.0, cursor.1);
        let target_ws = self.monitors[m].active_ws;
        if target_ws != ws {
            self.move_to_ws(id, target_ws);
            return;
        }
        let tiles = self.tiles(ws);
        if let Some((other, _)) = tiles.iter().find(|(o, r)| *o != id && r.contains(cursor.0, cursor.1)) {
            let other = *other;
            self.workspaces.get_mut(&ws).unwrap().layout.swap(id, other);
        }
    }

    // ------------------------------------------------------------ layout

    /// Tile rectangles (after gaps) of a workspace's tiled windows.
    pub fn tiles(&self, ws: u32) -> Vec<(WinId, Rect)> {
        let Some(w) = self.workspaces.get(&ws) else { return Vec::new() };
        let area = self.tiling_area(ws);
        let mut layout = w.layout.clone();
        let mut t = layout.compute(area, &self.cfg.layout_options());
        let s = self.monitors[w.monitor].scale();
        apply_gaps(&mut t, area, (self.cfg.general.gaps_in as f32 * s).round() as i32);
        t
    }

    fn refresh_layout_cache(&mut self) {
        let opts = self.cfg.layout_options();
        let ids: Vec<u32> = self.workspaces.keys().copied().collect();
        for id in ids {
            let area = self.tiling_area(id);
            self.workspaces.get_mut(&id).unwrap().layout.compute(area, &opts);
        }
    }

    pub fn desired(&self) -> Desired {
        let mut d = Desired::default();
        if self.paused {
            return d;
        }
        let border_on = self.cfg.decoration.border_style == "overlay" && self.cfg.general.border_size > 0;
        for (mi, mon) in self.monitors.iter().enumerate() {
            let ws_id = mon.active_ws;
            let Some(ws) = self.workspaces.get(&ws_id) else { continue };
            let bs = (self.cfg.general.border_size as f32 * mon.scale()).round() as i32;
            let ring = |r: Rect, w: &ManagedWindow| (border_on && w.border).then(|| Rect::new(r.x - bs, r.y - bs, r.w + 2 * bs, r.h + 2 * bs));
            let mut tiles = self.tiles(ws_id);
            // Monocle: focused window last so the platform raises it.
            if ws.layout.kind() == LayoutKind::Monocle {
                if let Some(f) = ws.last_focus {
                    tiles.sort_by_key(|(id, _)| *id == f);
                }
            }
            for (id, rect) in tiles {
                let w = &self.windows[&id];
                let full = ws.fullscreen == Some(id);
                let rect = if full { self.usable_area(mi) } else { rect };
                d.placements.push(Placement {
                    id,
                    rect,
                    border: if full { None } else { ring(rect, w) },
                    floating: false,
                    focused: self.focused == Some(id),
                });
            }
            for w in self.windows.values().filter(|w| w.ws == ws_id && w.mode == Mode::Floating && !w.minimized) {
                let full = ws.fullscreen == Some(w.id);
                let rect = if full { self.usable_area(mi) } else { w.float_rect };
                d.placements.push(Placement { id: w.id, rect, border: if full { None } else { ring(rect, w) }, floating: true, focused: self.focused == Some(w.id) });
            }
        }
        for w in self.windows.values() {
            if !self.is_visible_ws(w.ws) && !w.minimized {
                d.hidden.insert(w.id);
            }
        }
        d
    }

    // ------------------------------------------------------------ actions

    fn focused_ws_window(&self) -> Option<WinId> {
        self.focused.filter(|f| self.windows.get(f).is_some_and(|w| self.is_visible_ws(w.ws)))
    }

    fn current_rect(&self, id: WinId) -> Option<Rect> {
        self.desired().placements.into_iter().find(|p| p.id == id).map(|p| p.rect)
    }

    fn switch_ws_on(&mut self, monitor: usize, to: u32) -> Option<Effect> {
        let from = self.monitors[monitor].active_ws;
        if from == to {
            return None;
        }
        self.ensure_ws(to, monitor);
        // A workspace pinned to another monitor is shown there instead.
        let target_mon = self.workspaces[&to].monitor;
        // If it is already visible on another monitor, just focus that monitor.
        if self.monitors[target_mon].active_ws == to {
            self.focused_monitor = target_mon;
            return None;
        }
        let from = self.monitors[target_mon].active_ws;
        self.previous_ws = Some(from);
        self.monitors[target_mon].active_ws = to;
        self.focused_monitor = target_mon;
        self.cleanup();
        Some(Effect::WorkspaceSwitch { monitor: target_mon, from, to })
    }

    fn resolve_ws(&self, t: WsTarget) -> Option<u32> {
        let cur = self.active_ws();
        match t {
            WsTarget::Id(i) => Some(i),
            WsTarget::Previous => self.previous_ws,
            WsTarget::Next | WsTarget::Prev => {
                // Existing workspaces on the focused monitor, plus the current one.
                let mut ids: Vec<u32> = self.workspaces.values().filter(|w| w.monitor == self.focused_monitor).map(|w| w.id).collect();
                ids.sort();
                let pos = ids.iter().position(|i| *i == cur)?;
                let n = ids.len();
                Some(if t == WsTarget::Next { ids[(pos + 1) % n] } else { ids[(pos + n - 1) % n] })
            }
        }
    }

    fn resolve_monitor(&self, t: MonTarget) -> Option<usize> {
        let n = self.monitors.len();
        let cur = self.focused_monitor;
        match t {
            MonTarget::Next => Some((cur + 1) % n),
            MonTarget::Prev => Some((cur + n - 1) % n),
            MonTarget::Index(i) => (i >= 1 && i <= n).then(|| i - 1),
            MonTarget::Dir(d) => {
                let rects: Vec<Rect> = self.monitors.iter().map(|m| m.rect).collect();
                find_in_direction(&rects[cur], d, &rects)
            }
        }
    }

    fn move_to_ws(&mut self, id: WinId, to: u32) {
        let Some(w) = self.windows.get(&id) else { return };
        let from = w.ws;
        if from == to {
            return;
        }
        let from_mon = self.workspaces[&from].monitor;
        self.detach(id);
        self.ensure_ws(to, self.focused_monitor);
        let to_mon = self.workspaces[&to].monitor;
        let w = self.windows.get_mut(&id).unwrap();
        w.ws = to;
        if from_mon != to_mon {
            // Keep a floating window's relative position on the new monitor.
            let (a, b) = (self.monitors[from_mon].work, self.monitors[to_mon].work);
            let fx = (w.float_rect.x - a.x) as f32 / a.w.max(1) as f32;
            let fy = (w.float_rect.y - a.y) as f32 / a.h.max(1) as f32;
            w.float_rect.x = b.x + (fx * b.w as f32) as i32;
            w.float_rect.y = b.y + (fy * b.h as f32) as i32;
            w.float_rect.w = w.float_rect.w.min(b.w);
            w.float_rect.h = w.float_rect.h.min(b.h);
        }
        if w.mode == Mode::Tiled && !w.minimized {
            let area = self.tiling_area(to);
            let opts = self.cfg.layout_options();
            let wsp = self.workspaces.get_mut(&to).unwrap();
            let target = wsp.last_focus;
            wsp.layout.insert(id, target, area, &opts);
            wsp.fullscreen = None;
        }
        self.workspaces.get_mut(&to).unwrap().last_focus = Some(id);
        if self.focused == Some(id) && !self.is_visible_ws(to) {
            self.focused = None;
            if let Some(n) = self.fallback_focus(from) {
                self.focus_window(n, true);
            }
        }
        self.cleanup();
    }

    pub fn dispatch(&mut self, action: &Action) -> Vec<Effect> {
        let mut fx = Vec::new();
        // While paused only these actions do anything.
        if self.paused && !matches!(action, Action::TogglePause | Action::Reload | Action::Quit | Action::Exec(_) | Action::OpenSettings | Action::OpenConfig | Action::Close) {
            return fx;
        }
        self.refresh_layout_cache();
        let focused = self.focused_ws_window();
        match action {
            Action::Focus(dir) => self.focus_dir(*dir),
            Action::Cycle(next) => {
                let ws = self.active_ws();
                let mut ids: Vec<WinId> = self.workspaces.get(&ws).map(|w| w.layout.windows()).unwrap_or_default();
                ids.extend(self.windows.values().filter(|w| w.ws == ws && w.mode == Mode::Floating && !w.minimized).map(|w| w.id));
                if !ids.is_empty() {
                    let pos = focused.and_then(|f| ids.iter().position(|x| *x == f));
                    let n = ids.len();
                    let i = match pos {
                        Some(p) if *next => (p + 1) % n,
                        Some(p) => (p + n - 1) % n,
                        None => 0,
                    };
                    self.focus_window(ids[i], true);
                }
            }
            Action::Move(dir) => {
                if let Some(id) = focused {
                    self.move_dir(id, *dir);
                }
            }
            Action::Resize(dir, frac) => {
                if let Some(id) = focused {
                    let w = &self.windows[&id];
                    if w.mode == Mode::Tiled {
                        let ws = w.ws;
                        self.workspaces.get_mut(&ws).unwrap().layout.resize(id, *dir, *frac);
                    } else {
                        let mon = &self.monitors[self.workspaces[&w.ws].monitor];
                        let (dw, dh) = ((mon.work.w as f32 * frac) as i32, (mon.work.h as f32 * frac) as i32);
                        let w = self.windows.get_mut(&id).unwrap();
                        let r = &mut w.float_rect;
                        match dir {
                            Direction::Left => *r = Rect::new(r.x - dw, r.y, r.w + dw, r.h),
                            Direction::Right => r.w += dw,
                            Direction::Up => *r = Rect::new(r.x, r.y - dh, r.w, r.h + dh),
                            Direction::Down => r.h += dh,
                        }
                    }
                }
            }
            Action::Workspace(t) => {
                if let Some(to) = self.resolve_ws(*t) {
                    let m = self.focused_monitor;
                    if let Some(e) = self.switch_ws_on(m, to) {
                        fx.push(e);
                    }
                    self.focused = None;
                    let ws = self.active_ws();
                    if let Some(f) = self.fallback_focus(ws) {
                        self.focus_window(f, true);
                    }
                }
            }
            Action::MoveToWorkspace(t, follow) => {
                if let (Some(id), Some(to)) = (focused, self.resolve_ws(*t)) {
                    self.move_to_ws(id, to);
                    if *follow {
                        let m = self.workspaces[&to].monitor;
                        if let Some(e) = self.switch_ws_on(m, to) {
                            fx.push(e);
                        }
                        self.focus_window(id, true);
                    }
                }
            }
            Action::FocusMonitor(t) => {
                if let Some(m) = self.resolve_monitor(*t) {
                    self.focused_monitor = m;
                    self.focused = None;
                    let ws = self.monitors[m].active_ws;
                    if let Some(f) = self.fallback_focus(ws) {
                        self.focus_window(f, true);
                    }
                }
            }
            Action::MoveToMonitor(t) => {
                if let (Some(id), Some(m)) = (focused, self.resolve_monitor(*t)) {
                    let to = self.monitors[m].active_ws;
                    self.move_to_ws(id, to);
                    self.focus_window(id, true);
                }
            }
            Action::SetLayout(k) => self.set_layout(*k),
            Action::NextLayout => {
                let k = self.workspaces.get(&self.active_ws()).map(|w| w.layout.kind().next()).unwrap_or(LayoutKind::Dwindle);
                self.set_layout(k);
            }
            Action::ToggleFloat => {
                if let Some(id) = focused {
                    self.toggle_float(id);
                }
            }
            Action::ToggleFullscreen => {
                if let Some(id) = focused {
                    let ws = self.windows[&id].ws;
                    let w = self.workspaces.get_mut(&ws).unwrap();
                    w.fullscreen = if w.fullscreen == Some(id) { None } else { Some(id) };
                }
            }
            Action::ToggleSplit => {
                if let Some(id) = focused {
                    let ws = self.windows[&id].ws;
                    if let Layout::Dwindle(d) = &mut self.workspaces.get_mut(&ws).unwrap().layout {
                        d.toggle_split(id);
                    }
                }
            }
            Action::SplitRatio(v, exact) => {
                if let Some(id) = focused {
                    let ws = self.windows[&id].ws;
                    match &mut self.workspaces.get_mut(&ws).unwrap().layout {
                        Layout::Dwindle(d) => {
                            d.split_ratio(id, *v, *exact);
                        }
                        Layout::Master(m) => m.adjust_mfact(if *exact { *v / 2.0 } else { *v / 2.0 }, *exact),
                        Layout::Monocle(_) => {}
                    }
                }
            }
            Action::SwapWithMaster => {
                if let Some(id) = focused {
                    let ws = self.windows[&id].ws;
                    if let Layout::Master(m) = &mut self.workspaces.get_mut(&ws).unwrap().layout {
                        m.swap_with_master(id);
                    }
                }
            }
            Action::OrientationNext => {
                let ws = self.active_ws();
                if let Some(Layout::Master(m)) = self.workspaces.get_mut(&ws).map(|w| &mut w.layout) {
                    m.orientation = m.orientation.next();
                }
            }
            Action::Close => {
                if let Some(id) = self.focused {
                    fx.push(Effect::Close(id));
                }
            }
            Action::Exec(c) => fx.push(Effect::Exec(c.clone())),
            Action::Reload => fx.push(Effect::Reload),
            Action::TogglePause => {
                self.paused = !self.paused;
                fx.push(Effect::Paused(self.paused));
            }
            Action::Retile => {}
            Action::OpenSettings => fx.push(Effect::OpenSettings),
            Action::OpenConfig => fx.push(Effect::OpenConfig),
            Action::Quit => fx.push(Effect::Quit),
        }
        fx
    }

    fn set_layout(&mut self, k: LayoutKind) {
        let ws = self.active_ws();
        let area = self.tiling_area(ws);
        let opts = self.cfg.layout_options();
        if let Some(w) = self.workspaces.get_mut(&ws) {
            if w.layout.kind() != k {
                w.layout = w.layout.convert(k, area, &opts);
            }
        }
    }

    fn toggle_float(&mut self, id: WinId) {
        let w = &self.windows[&id];
        let ws = w.ws;
        match w.mode {
            Mode::Tiled => {
                // Float at the current tile size, slightly inset, like Hyprland.
                let cur = self.current_rect(id).unwrap_or(w.float_rect);
                let mon = self.usable_area(self.workspaces[&ws].monitor);
                self.detach(id);
                let w = self.windows.get_mut(&id).unwrap();
                w.mode = Mode::Floating;
                let (fw, fh) = ((cur.w as f32 * 0.8) as i32, (cur.h as f32 * 0.8) as i32);
                w.float_rect = Rect::new(cur.x + (cur.w - fw) / 2, cur.y + (cur.h - fh) / 2, fw, fh);
                if mon.intersection(&w.float_rect).is_none() {
                    w.float_rect = mon.centered(fw, fh);
                }
            }
            Mode::Floating => {
                self.windows.get_mut(&id).unwrap().mode = Mode::Tiled;
                if !self.windows[&id].minimized {
                    self.insert_tiled(id, ws);
                }
            }
        }
        self.workspaces.get_mut(&ws).unwrap().last_focus = Some(id);
    }

    fn visible_rects(&self) -> Vec<(WinId, Rect)> {
        self.desired().placements.into_iter().map(|p| (p.id, p.rect)).collect()
    }

    fn focus_dir(&mut self, dir: Direction) {
        let ws = self.active_ws();
        // Monocle: horizontal directions cycle within the stack.
        if let (Some(f), Some(w)) = (self.focused_ws_window(), self.workspaces.get(&ws)) {
            if w.layout.kind() == LayoutKind::Monocle && w.layout.contains(f) && dir.is_horizontal() {
                let ids = w.layout.windows();
                if ids.len() > 1 {
                    let p = ids.iter().position(|x| *x == f).unwrap();
                    let n = ids.len();
                    let next = if dir == Direction::Right { ids[(p + 1) % n] } else { ids[(p + n - 1) % n] };
                    self.focus_window(next, true);
                    return;
                }
            }
        }
        let rects = self.visible_rects();
        let from = match self.focused_ws_window().and_then(|f| rects.iter().find(|(i, _)| *i == f).map(|(_, r)| *r)) {
            Some(r) => r,
            None => self.monitors[self.focused_monitor].work,
        };
        let others: Vec<(WinId, Rect)> = rects.into_iter().filter(|(i, _)| Some(*i) != self.focused).collect();
        let cands: Vec<Rect> = others.iter().map(|(_, r)| *r).collect();
        if let Some(i) = find_in_direction(&from, dir, &cands) {
            self.focus_window(others[i].0, true);
        } else if let Some(m) = self.resolve_monitor(MonTarget::Dir(dir)) {
            // Empty neighbour monitor: focus it anyway.
            if m != self.focused_monitor {
                self.focused_monitor = m;
                self.focused = None;
                if let Some(f) = self.fallback_focus(self.monitors[m].active_ws) {
                    self.focus_window(f, true);
                }
            }
        }
    }

    fn move_dir(&mut self, id: WinId, dir: Direction) {
        let w = &self.windows[&id];
        let ws = w.ws;
        if w.mode == Mode::Floating {
            let mon = &self.monitors[self.workspaces[&ws].monitor];
            let step = (mon.work.w.min(mon.work.h) as f32 * 0.05) as i32;
            let r = &mut self.windows.get_mut(&id).unwrap().float_rect;
            match dir {
                Direction::Left => r.x -= step,
                Direction::Right => r.x += step,
                Direction::Up => r.y -= step,
                Direction::Down => r.y += step,
            }
            return;
        }
        let tiles = self.tiles(ws);
        let Some(from) = tiles.iter().find(|(i, _)| *i == id).map(|(_, r)| *r) else { return };
        let others: Vec<(WinId, Rect)> = tiles.into_iter().filter(|(i, _)| *i != id).collect();
        let cands: Vec<Rect> = others.iter().map(|(_, r)| *r).collect();
        if let Some(i) = find_in_direction(&from, dir, &cands) {
            let other = others[i].0;
            self.workspaces.get_mut(&ws).unwrap().layout.swap(id, other);
        } else if let Some(m) = self.resolve_monitor(MonTarget::Dir(dir)) {
            if m != self.workspaces[&ws].monitor {
                let to = self.monitors[m].active_ws;
                self.move_to_ws(id, to);
                self.focus_window(id, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_str;
    use crate::rules::{classify, ClassifyOptions};

    fn cfg(extra: &str) -> Config {
        let base = "[general]\ngaps_in = 0\ngaps_out = 0\nborder_size = 0\n[binds]\n\"win+t\" = \"retile\"\n";
        let r = load_str(&format!("{base}{extra}"), "t", None, &|_| None);
        assert!(!r.has_errors(), "{:#?}", r.diagnostics);
        r.config
    }

    fn mon(id: &str, x: i32, w: i32, h: i32) -> Monitor {
        Monitor { id: id.into(), rect: Rect::new(x, 0, w, h), work: Rect::new(x, 0, w, h), dpi: 96, active_ws: 0 }
    }

    fn facts(id: u64, x: i32) -> WindowFacts {
        WindowFacts { hwnd: id, title: format!("w{id}"), class: "Test".into(), process: "test.exe".into(), visible: true, has_caption: true, resizable: true, rect: Rect::new(x + 10, 10, 300, 200), ..Default::default() }
    }

    fn add(wm: &mut Wm, id: u64) {
        add_at(wm, id, 0)
    }

    fn add_at(wm: &mut Wm, id: u64, x: i32) {
        let f = facts(id, x);
        let c = classify(&f, &wm.cfg.rules, &ClassifyOptions { manage_by_default: true, float_unresizable: true, float_dialogs: true });
        assert!(wm.add_window(f, &c, false));
    }

    fn rect_of(wm: &Wm, id: u64) -> Rect {
        wm.desired().placements.iter().find(|p| p.id == id).unwrap().rect
    }

    fn one() -> Wm {
        Wm::new(cfg(""), vec![mon("A", 0, 1600, 900)])
    }

    fn two() -> Wm {
        Wm::new(cfg(""), vec![mon("A", 0, 1600, 900), mon("B", 1600, 1600, 900)])
    }

    #[test]
    fn monitors_get_distinct_workspaces() {
        let wm = two();
        assert_eq!(wm.monitors[0].active_ws, 1);
        assert_eq!(wm.monitors[1].active_ws, 2);
    }

    #[test]
    fn new_windows_tile_and_take_focus() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        assert_eq!(wm.focused, Some(2));
        assert_eq!(rect_of(&wm, 1), Rect::new(0, 0, 800, 900));
        assert_eq!(rect_of(&wm, 2), Rect::new(800, 0, 800, 900));
    }

    #[test]
    fn gaps_and_border_and_dpi_scaling() {
        let c = cfg("");
        let mut c2 = c.clone();
        c2.general.gaps_in = 5;
        c2.general.gaps_out = 10;
        c2.general.border_size = 2;
        let mut m = mon("A", 0, 1600, 900);
        m.dpi = 144; // 150%
        let mut wm = Wm::new(c2, vec![m]);
        add(&mut wm, 1);
        add(&mut wm, 2);
        let p = wm.desired().placements;
        // gaps_out 10 -> 15px, gaps_in 5 -> 8px (rounded), border 2 -> 3px.
        assert_eq!(p[0].rect, Rect::new(15, 15, 785 - 8, 870));
        assert_eq!(p[0].border, Some(Rect::new(12, 12, 777 + 6, 876)));
        assert!(p[1].focused && !p[0].focused);
    }

    #[test]
    fn close_focuses_sibling_and_retiles() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        add(&mut wm, 3);
        assert!(wm.remove_window(3));
        assert!(wm.focused.is_some());
        assert_eq!(rect_of(&wm, 2), Rect::new(800, 0, 800, 900));
        assert!(!wm.remove_window(3), "second removal is a no-op");
    }

    #[test]
    fn workspace_switch_hides_and_restores() {
        let mut wm = one();
        add(&mut wm, 1);
        let fx = wm.dispatch(&Action::Workspace(WsTarget::Id(3)));
        assert_eq!(fx, vec![Effect::WorkspaceSwitch { monitor: 0, from: 1, to: 3 }]);
        assert!(wm.desired().hidden.contains(&1));
        add(&mut wm, 2);
        assert_eq!(wm.windows[&2].ws, 3);
        assert_eq!(rect_of(&wm, 2), Rect::new(0, 0, 1600, 900));
        wm.dispatch(&Action::Workspace(WsTarget::Previous));
        assert_eq!(wm.active_ws(), 1);
        let d = wm.desired();
        assert!(d.hidden.contains(&2) && !d.hidden.contains(&1));
        assert_eq!(wm.take_focus_request(), Some(1));
    }

    #[test]
    fn empty_workspaces_are_cleaned_up() {
        let mut wm = one();
        wm.dispatch(&Action::Workspace(WsTarget::Id(5)));
        wm.dispatch(&Action::Workspace(WsTarget::Id(6)));
        assert!(!wm.workspaces.contains_key(&5));
        assert!(wm.workspaces.contains_key(&6));
    }

    #[test]
    fn move_to_workspace_follow_and_silent() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        wm.dispatch(&Action::MoveToWorkspace(WsTarget::Id(2), false));
        assert_eq!(wm.windows[&2].ws, 2);
        assert_eq!(wm.active_ws(), 1);
        assert_eq!(wm.focused, Some(1), "focus falls back on the old workspace");
        assert_eq!(rect_of(&wm, 1), Rect::new(0, 0, 1600, 900));
        wm.dispatch(&Action::MoveToWorkspace(WsTarget::Id(2), true));
        assert_eq!(wm.active_ws(), 2);
        assert_eq!(wm.windows[&1].ws, 2);
        assert_eq!(wm.focused, Some(1));
    }

    #[test]
    fn focus_and_move_by_direction() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        add(&mut wm, 3); // 1 | 2/3, focus 3
        wm.dispatch(&Action::Focus(Direction::Up));
        assert_eq!(wm.focused, Some(2));
        wm.dispatch(&Action::Focus(Direction::Left));
        assert_eq!(wm.focused, Some(1));
        wm.dispatch(&Action::Move(Direction::Right));
        assert_eq!(rect_of(&wm, 1).x, 800, "swapped with the right neighbour");
        assert_eq!(wm.focused, Some(1));
    }

    #[test]
    fn focus_and_move_across_monitors() {
        let mut wm = two();
        add_at(&mut wm, 1, 0);
        wm.dispatch(&Action::Focus(Direction::Right));
        assert_eq!(wm.focused_monitor, 1, "empty monitor gets focus");
        add(&mut wm, 2);
        assert_eq!(wm.windows[&2].ws, 2, "opens on focused monitor");
        wm.dispatch(&Action::Move(Direction::Left));
        assert_eq!(wm.windows[&2].ws, 1);
        assert_eq!(rect_of(&wm, 2).x, 800);
        assert_eq!(wm.focused, Some(2));
    }

    #[test]
    fn switching_to_workspace_visible_elsewhere_focuses_that_monitor() {
        let mut wm = two();
        add_at(&mut wm, 1, 1700);
        assert_eq!(wm.windows[&1].ws, 1, "new window opens on focused monitor 0");
        wm.dispatch(&Action::Workspace(WsTarget::Id(2)));
        assert_eq!(wm.focused_monitor, 1);
        assert_eq!(wm.monitors[0].active_ws, 1);
    }

    #[test]
    fn adopting_keeps_window_on_its_monitor() {
        let mut wm = two();
        let f = facts(9, 1700);
        let c = classify(&f, &[], &ClassifyOptions { manage_by_default: true, float_unresizable: true, float_dialogs: true });
        wm.add_window(f, &c, true);
        assert_eq!(wm.windows[&9].ws, 2);
        assert_eq!(wm.focused, None, "adoption does not steal focus");
    }

    #[test]
    fn float_toggle_and_fullscreen() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        wm.dispatch(&Action::ToggleFloat);
        assert_eq!(wm.windows[&2].mode, Mode::Floating);
        assert_eq!(rect_of(&wm, 1), Rect::new(0, 0, 1600, 900));
        let fr = rect_of(&wm, 2);
        assert_eq!((fr.w, fr.h), (640, 720));
        wm.dispatch(&Action::ToggleFloat);
        assert_eq!(wm.windows[&2].mode, Mode::Tiled);
        wm.dispatch(&Action::ToggleFullscreen);
        assert_eq!(rect_of(&wm, 2), Rect::new(0, 0, 1600, 900));
        assert!(wm.desired().placements.iter().find(|p| p.id == 2).unwrap().border.is_none());
        wm.dispatch(&Action::ToggleFullscreen);
        assert_eq!(rect_of(&wm, 2).w, 800);
    }

    #[test]
    fn layout_switching_and_monocle_cycle() {
        let mut wm = one();
        for i in 1..=3 {
            add(&mut wm, i);
        }
        wm.dispatch(&Action::SetLayout(LayoutKind::Master));
        assert_eq!(rect_of(&wm, 1).w, 880);
        wm.dispatch(&Action::NextLayout);
        assert_eq!(wm.workspaces[&1].layout.kind(), LayoutKind::Monocle);
        assert!(wm.desired().placements.iter().all(|p| p.rect == Rect::new(0, 0, 1600, 900)));
        assert_eq!(wm.desired().placements.last().unwrap().id, 3, "focused last = on top");
        wm.dispatch(&Action::Focus(Direction::Right));
        assert_eq!(wm.focused, Some(1));
    }

    #[test]
    fn minimize_removes_from_layout() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        wm.set_minimized(2, true);
        assert_eq!(rect_of(&wm, 1).w, 1600);
        assert!(!wm.desired().hidden.contains(&2), "minimized windows are left alone");
        wm.set_minimized(2, false);
        assert_eq!(rect_of(&wm, 1).w, 800);
    }

    #[test]
    fn self_activation_of_hidden_window_does_not_switch_by_default() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        wm.dispatch(&Action::MoveToWorkspace(WsTarget::Id(4), false));
        assert_eq!(wm.active_ws(), 1);
        let visible = wm.focused.expect("a window on workspace 1 keeps focus");
        wm.take_focus_request();
        wm.on_focus(Some(2), false);
        assert_eq!(wm.active_ws(), 1, "focus_on_activate = false keeps the workspace");
        assert_eq!(wm.take_focus_request(), Some(visible), "focus is given back to the visible window");
        // The user restoring it themselves (taskbar, minimize hiding) switches.
        wm.on_focus(Some(2), true);
        assert_eq!(wm.active_ws(), 4);
    }

    #[test]
    fn focus_on_activate_switches_workspace() {
        let c = load_str("[general]\nfocus_on_activate = true\ngaps_in = 0\ngaps_out = 0\nborder_size = 0\n", "t", None, &|_| None).config;
        let mut wm = Wm::new(c, vec![mon("A", 0, 1600, 900)]);
        add(&mut wm, 1);
        wm.dispatch(&Action::MoveToWorkspace(WsTarget::Id(4), false));
        assert_eq!(wm.active_ws(), 1);
        wm.on_focus(Some(1), false);
        assert_eq!(wm.active_ws(), 4);
    }

    #[test]
    fn rules_place_windows() {
        let c = cfg("[[rule]]\nmatch = { title = \"w7\" }\nworkspace = 7\n[[rule]]\nmatch = { title = \"w8\" }\nfloat = true\nsize = [400, 300]\ncenter = true\n");
        let mut wm = Wm::new(c, vec![mon("A", 0, 1600, 900)]);
        add(&mut wm, 7);
        add(&mut wm, 8);
        assert_eq!(wm.windows[&7].ws, 7);
        assert!(wm.desired().hidden.contains(&7));
        assert_eq!(wm.windows[&7].matched_rules.len(), 1);
        assert_eq!(rect_of(&wm, 8), Rect::new(600, 300, 400, 300));
    }

    #[test]
    fn workspace_rule_pins_monitor_and_layout() {
        let c = cfg("[[workspace]]\nid = 5\nmonitor = 2\nlayout = \"monocle\"\n");
        let mut wm = Wm::new(c, vec![mon("A", 0, 1600, 900), mon("B", 1600, 1600, 900)]);
        wm.dispatch(&Action::Workspace(WsTarget::Id(5)));
        assert_eq!(wm.monitors[1].active_ws, 5);
        assert_eq!(wm.monitors[0].active_ws, 1);
        assert_eq!(wm.workspaces[&5].layout.kind(), LayoutKind::Monocle);
    }

    #[test]
    fn monitor_removal_moves_workspaces() {
        let mut wm = two();
        add_at(&mut wm, 1, 0);
        wm.dispatch(&Action::FocusMonitor(MonTarget::Index(2)));
        add(&mut wm, 2);
        assert_eq!(wm.windows[&2].ws, 2);
        wm.set_monitors(vec![mon("A", 0, 1600, 900)]);
        assert_eq!(wm.workspaces[&2].monitor, 0);
        assert!(wm.desired().hidden.contains(&2), "orphaned workspace is hidden, not lost");
        wm.dispatch(&Action::Workspace(WsTarget::Id(2)));
        assert!(wm.desired().placements.iter().any(|p| p.id == 2));
        // Monitor comes back: it gets a workspace again.
        wm.set_monitors(vec![mon("A", 0, 1600, 900), mon("B", 1600, 1600, 900)]);
        assert_ne!(wm.monitors[1].active_ws, 0);
        assert_ne!(wm.monitors[1].active_ws, wm.monitors[0].active_ws);
    }

    #[test]
    fn pause_releases_everything() {
        let mut wm = one();
        add(&mut wm, 1);
        wm.dispatch(&Action::MoveToWorkspace(WsTarget::Id(2), false));
        assert_eq!(wm.dispatch(&Action::TogglePause), vec![Effect::Paused(true)]);
        let d = wm.desired();
        assert!(d.placements.is_empty() && d.hidden.is_empty());
        assert!(wm.dispatch(&Action::Workspace(WsTarget::Id(2))).is_empty(), "ignored while paused");
        wm.dispatch(&Action::TogglePause);
        assert!(wm.desired().hidden.contains(&1));
    }

    #[test]
    fn user_drag_swaps_and_resize_adjusts_ratio() {
        let mut wm = one();
        add(&mut wm, 1);
        add(&mut wm, 2);
        wm.on_user_move(1, Rect::new(900, 100, 300, 300), (1000, 400), false);
        assert_eq!(rect_of(&wm, 1).x, 800, "dropped on window 2 -> swapped");
        // Window 2 is now left; user drags its right edge to x=1000.
        wm.on_user_move(2, Rect::new(0, 0, 1000, 900), (1000, 450), true);
        assert_eq!(rect_of(&wm, 2).w, 1000);
        assert_eq!(rect_of(&wm, 1).x, 1000);
    }

    #[test]
    fn reserved_area_shrinks_tiling_and_fullscreen() {
        let c = cfg("[[monitor]]
index = 1
reserved = { top = 100, left = 600 }
");
        let mut wm = Wm::new(c, vec![mon("A", 0, 1600, 900)]);
        add(&mut wm, 1);
        assert_eq!(rect_of(&wm, 1), Rect::new(600, 100, 1000, 800));
        wm.dispatch(&Action::ToggleFullscreen);
        assert_eq!(rect_of(&wm, 1), Rect::new(600, 100, 1000, 800));
    }

    #[test]
    fn workspace_next_prev_cycle_existing() {
        let mut wm = one();
        add(&mut wm, 1);
        wm.dispatch(&Action::Workspace(WsTarget::Id(3)));
        add(&mut wm, 2);
        wm.dispatch(&Action::Workspace(WsTarget::Next));
        assert_eq!(wm.active_ws(), 1);
        wm.dispatch(&Action::Workspace(WsTarget::Prev));
        assert_eq!(wm.active_ws(), 3);
    }
}
