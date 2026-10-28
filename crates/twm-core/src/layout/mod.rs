//! Tiling layouts. Each workspace owns one `Layout`; the state machine asks
//! it for tile rectangles inside the workspace area and applies gaps
//! afterwards (see `apply_gaps`), so layouts only deal with exact partitions.

pub mod dwindle;
pub mod master;

use crate::geom::{Direction, Rect};
use crate::WinId;
use dwindle::{Dwindle, DwindleOptions};
use master::{Master, MasterOptions, Orientation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutKind {
    Dwindle,
    Master,
    Monocle,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 3] = [LayoutKind::Dwindle, LayoutKind::Master, LayoutKind::Monocle];

    pub fn parse(s: &str) -> Option<LayoutKind> {
        match s {
            "dwindle" => Some(LayoutKind::Dwindle),
            "master" => Some(LayoutKind::Master),
            "monocle" => Some(LayoutKind::Monocle),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Dwindle => "dwindle",
            LayoutKind::Master => "master",
            LayoutKind::Monocle => "monocle",
        }
    }

    pub fn next(self) -> LayoutKind {
        match self {
            LayoutKind::Dwindle => LayoutKind::Master,
            LayoutKind::Master => LayoutKind::Monocle,
            LayoutKind::Monocle => LayoutKind::Dwindle,
        }
    }
}

/// Options for all layouts, derived from configuration.
#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    pub dwindle: DwindleOptions,
    pub master: MasterOptions,
    pub mfact: f32,
    pub orientation: Orientation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Layout {
    Dwindle(Dwindle),
    Master(Master),
    /// Every window fills the area; the focused one is on top.
    Monocle(Vec<WinId>),
}

impl Layout {
    pub fn new(kind: LayoutKind, opts: &LayoutOptions) -> Layout {
        match kind {
            LayoutKind::Dwindle => Layout::Dwindle(Dwindle::default()),
            LayoutKind::Master => Layout::Master(Master::new(opts.mfact, opts.orientation)),
            LayoutKind::Monocle => Layout::Monocle(Vec::new()),
        }
    }

    pub fn kind(&self) -> LayoutKind {
        match self {
            Layout::Dwindle(_) => LayoutKind::Dwindle,
            Layout::Master(_) => LayoutKind::Master,
            Layout::Monocle(_) => LayoutKind::Monocle,
        }
    }

    pub fn windows(&self) -> Vec<WinId> {
        match self {
            Layout::Dwindle(d) => d.windows(),
            Layout::Master(m) => m.order.clone(),
            Layout::Monocle(v) => v.clone(),
        }
    }

    pub fn contains(&self, w: WinId) -> bool {
        match self {
            Layout::Dwindle(d) => d.contains(w),
            Layout::Master(m) => m.order.contains(&w),
            Layout::Monocle(v) => v.contains(&w),
        }
    }

    pub fn len(&self) -> usize {
        self.windows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn insert(&mut self, w: WinId, focused: Option<WinId>, area: Rect, opts: &LayoutOptions) {
        match self {
            Layout::Dwindle(d) => d.insert(w, focused, area, &opts.dwindle),
            Layout::Master(m) => m.insert(w, focused, &opts.master),
            Layout::Monocle(v) => {
                if !v.contains(&w) {
                    v.push(w)
                }
            }
        }
    }

    pub fn remove(&mut self, w: WinId) -> bool {
        match self {
            Layout::Dwindle(d) => d.remove(w),
            Layout::Master(m) => m.remove(w),
            Layout::Monocle(v) => {
                let n = v.len();
                v.retain(|x| *x != w);
                n != v.len()
            }
        }
    }

    pub fn swap(&mut self, a: WinId, b: WinId) {
        match self {
            Layout::Dwindle(d) => d.swap(a, b),
            Layout::Master(m) => m.swap(a, b),
            Layout::Monocle(v) => {
                let ia = v.iter().position(|x| *x == a);
                let ib = v.iter().position(|x| *x == b);
                if let (Some(ia), Some(ib)) = (ia, ib) {
                    v.swap(ia, ib)
                }
            }
        }
    }

    /// Tile rectangles (before gaps) for every window, in layout order.
    pub fn compute(&mut self, area: Rect, opts: &LayoutOptions) -> Vec<(WinId, Rect)> {
        match self {
            Layout::Dwindle(d) => d.compute(area, &opts.dwindle),
            Layout::Master(m) => m.compute(area),
            Layout::Monocle(v) => v.iter().map(|w| (*w, area)).collect(),
        }
    }

    /// Grows `w` towards `dir` by `frac` of the available size.
    pub fn resize(&mut self, w: WinId, dir: Direction, frac: f32) -> bool {
        match self {
            Layout::Dwindle(d) => d.resize(w, dir, frac),
            Layout::Master(m) => {
                // "Grow towards dir": only the edge shared by master and stack
                // can move. The master grows by raising mfact, the stack by
                // lowering it.
                let is_master = m.order.iter().position(|x| *x == w).is_some_and(|i| i < m.masters);
                use Direction::*;
                let toward_stack = match m.orientation {
                    Orientation::Left => Right,
                    Orientation::Right => Left,
                    Orientation::Top => Down,
                    Orientation::Bottom => Up,
                    Orientation::Center => {
                        if !dir.is_horizontal() {
                            return false;
                        }
                        dir
                    }
                };
                let toward_master = match toward_stack {
                    Left => Right,
                    Right => Left,
                    Up => Down,
                    Down => Up,
                };
                if m.orientation == Orientation::Center {
                    m.adjust_mfact(if is_master { frac } else { -frac }, false);
                } else if is_master && dir == toward_stack {
                    m.adjust_mfact(frac, false);
                } else if !is_master && dir == toward_master && m.order.len() > m.masters {
                    m.adjust_mfact(-frac, false);
                } else {
                    return false;
                }
                true
            }
            Layout::Monocle(_) => false,
        }
    }

    /// Rebuilds this layout as `kind`, preserving window order.
    pub fn convert(&self, kind: LayoutKind, area: Rect, opts: &LayoutOptions) -> Layout {
        let mut l = Layout::new(kind, opts);
        let mut prev = None;
        for w in self.windows() {
            l.insert(w, prev, area, &LayoutOptions { master: MasterOptions { new_on_top: false, new_status: master::NewStatus::Slave }, ..*opts });
            prev = Some(w);
        }
        l
    }
}

/// Applies Hyprland-style gaps: the area is first shrunk by `gaps_out`; then
/// every tile edge that does not touch the outer area is shrunk by `gaps_in`,
/// giving `2 * gaps_in` between neighbours.
pub fn apply_gaps(tiles: &mut [(WinId, Rect)], area: Rect, gaps_in: i32) {
    for (_, r) in tiles.iter_mut() {
        let l = if r.x > area.x { gaps_in } else { 0 };
        let t = if r.y > area.y { gaps_in } else { 0 };
        let rr = if r.right() < area.right() { gaps_in } else { 0 };
        let b = if r.bottom() < area.bottom() { gaps_in } else { 0 };
        *r = r.inset(l, t, rr, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> LayoutOptions {
        LayoutOptions {
            dwindle: DwindleOptions::default(),
            master: MasterOptions { new_status: master::NewStatus::Slave, new_on_top: false },
            mfact: 0.55,
            orientation: Orientation::Left,
        }
    }

    #[test]
    fn gaps_between_and_at_edges() {
        let area = Rect::new(0, 0, 1000, 500);
        let mut l = Layout::new(LayoutKind::Dwindle, &opts());
        l.insert(1, None, area, &opts());
        l.insert(2, Some(1), area, &opts());
        let mut t = l.compute(area, &opts());
        apply_gaps(&mut t, area, 5);
        assert_eq!(t[0].1, Rect::new(0, 0, 495, 500));
        assert_eq!(t[1].1, Rect::new(505, 0, 495, 500));
    }

    #[test]
    fn convert_preserves_order() {
        let area = Rect::new(0, 0, 1000, 500);
        let mut l = Layout::new(LayoutKind::Master, &opts());
        for i in 1..=4 {
            l.insert(i, None, area, &opts());
        }
        let d = l.convert(LayoutKind::Dwindle, area, &opts());
        assert_eq!(d.windows(), vec![1, 2, 3, 4]);
        let m = d.convert(LayoutKind::Monocle, area, &opts());
        assert_eq!(m.windows(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn master_resize_direction() {
        let area = Rect::new(0, 0, 1000, 500);
        let mut l = Layout::new(LayoutKind::Master, &opts());
        l.insert(1, None, area, &opts());
        l.insert(2, None, area, &opts());
        let mfact = |l: &Layout| if let Layout::Master(m) = l { m.mfact } else { unreachable!() };
        assert!(l.resize(1, Direction::Right, 0.05));
        assert!((mfact(&l) - 0.60).abs() < 1e-6);
        assert!(l.resize(2, Direction::Left, 0.10)); // stack grows towards master
        assert!((mfact(&l) - 0.50).abs() < 1e-6);
        assert!(!l.resize(1, Direction::Left, 0.05), "screen edge");
        assert!(!l.resize(1, Direction::Up, 0.05));
    }
}
