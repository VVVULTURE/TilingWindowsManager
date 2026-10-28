//! Master/stack layout modelled on Hyprland's master layout: the first
//! window(s) occupy the master area (`mfact` of the space), the rest share
//! the stack area evenly. Orientation selects where the master area sits.

use crate::geom::Rect;
use crate::WinId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
    /// Master in the middle; stack windows alternate right/left.
    Center,
}

impl Orientation {
    pub fn parse(s: &str) -> Option<Orientation> {
        Some(match s {
            "left" => Orientation::Left,
            "right" => Orientation::Right,
            "top" => Orientation::Top,
            "bottom" => Orientation::Bottom,
            "center" => Orientation::Center,
            _ => return None,
        })
    }

    pub fn next(self) -> Orientation {
        match self {
            Orientation::Left => Orientation::Top,
            Orientation::Top => Orientation::Right,
            Orientation::Right => Orientation::Bottom,
            Orientation::Bottom => Orientation::Center,
            Orientation::Center => Orientation::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NewStatus {
    Master,
    #[default]
    Slave,
    /// Same role as the focused window.
    Inherit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Master {
    /// Window order: the first `masters` entries are masters.
    pub order: Vec<WinId>,
    pub masters: usize,
    pub mfact: f32,
    pub orientation: Orientation,
}

#[derive(Clone, Copy, Debug)]
pub struct MasterOptions {
    pub new_status: NewStatus,
    pub new_on_top: bool,
}

impl Master {
    pub fn new(mfact: f32, orientation: Orientation) -> Self {
        Master { order: Vec::new(), masters: 1, mfact: mfact.clamp(0.05, 0.95), orientation }
    }

    pub fn insert(&mut self, w: WinId, focused: Option<WinId>, opts: &MasterOptions) {
        if self.order.contains(&w) {
            return;
        }
        let focused_is_master = focused.and_then(|f| self.order.iter().position(|x| *x == f)).is_some_and(|i| i < self.masters);
        let as_master = match opts.new_status {
            NewStatus::Master => true,
            NewStatus::Slave => false,
            NewStatus::Inherit => focused_is_master,
        } || self.order.is_empty();
        if as_master {
            self.order.insert(0, w);
        } else if opts.new_on_top {
            let at = self.masters.min(self.order.len());
            self.order.insert(at, w);
        } else {
            self.order.push(w);
        }
    }

    pub fn remove(&mut self, w: WinId) -> bool {
        let before = self.order.len();
        self.order.retain(|x| *x != w);
        before != self.order.len()
    }

    pub fn swap(&mut self, a: WinId, b: WinId) {
        let ia = self.order.iter().position(|x| *x == a);
        let ib = self.order.iter().position(|x| *x == b);
        if let (Some(ia), Some(ib)) = (ia, ib) {
            self.order.swap(ia, ib);
        }
    }

    /// Hyprland `swapwithmaster`: exchanges `w` with the first master (or,
    /// when `w` is the master, with the first stack window).
    pub fn swap_with_master(&mut self, w: WinId) {
        let Some(i) = self.order.iter().position(|x| *x == w) else { return };
        let j = if i == 0 { 1 } else { 0 };
        if j < self.order.len() {
            self.order.swap(i, j);
        }
    }

    pub fn adjust_mfact(&mut self, delta: f32, exact: bool) {
        self.mfact = if exact { delta } else { self.mfact + delta }.clamp(0.05, 0.95);
    }

    pub fn compute(&self, area: Rect) -> Vec<(WinId, Rect)> {
        let n = self.order.len();
        if n == 0 {
            return Vec::new();
        }
        let nm = self.masters.clamp(1, n);
        let ns = n - nm;
        let (masters, stack) = self.order.split_at(nm);
        let mut out = Vec::with_capacity(n);
        if ns == 0 {
            out.extend(split_even(masters, area, self.orientation_is_vertical_stack()));
            return out;
        }
        let horizontal = matches!(self.orientation, Orientation::Left | Orientation::Right | Orientation::Center);
        if self.orientation == Orientation::Center && ns >= 2 {
            let mw = (area.w as f32 * self.mfact).round() as i32;
            let side = (area.w - mw) / 2;
            let left = Rect::new(area.x, area.y, side, area.h);
            let mid = Rect::new(area.x + side, area.y, mw, area.h);
            let right = Rect::new(area.x + side + mw, area.y, area.w - side - mw, area.h);
            out.extend(split_even(masters, mid, true));
            // Alternate: first stack window right, then left (Hyprland).
            let rs: Vec<WinId> = stack.iter().step_by(2).copied().collect();
            let ls: Vec<WinId> = stack.iter().skip(1).step_by(2).copied().collect();
            out.extend(split_even(&rs, right, true));
            out.extend(split_even(&ls, left, true));
            return out;
        }
        let (m_area, s_area) = if horizontal {
            let mw = (area.w as f32 * self.mfact).round() as i32;
            match self.orientation {
                Orientation::Right => (Rect::new(area.x + area.w - mw, area.y, mw, area.h), Rect::new(area.x, area.y, area.w - mw, area.h)),
                _ => (Rect::new(area.x, area.y, mw, area.h), Rect::new(area.x + mw, area.y, area.w - mw, area.h)),
            }
        } else {
            let mh = (area.h as f32 * self.mfact).round() as i32;
            match self.orientation {
                Orientation::Bottom => (Rect::new(area.x, area.y + area.h - mh, area.w, mh), Rect::new(area.x, area.y, area.w, area.h - mh)),
                _ => (Rect::new(area.x, area.y, area.w, mh), Rect::new(area.x, area.y + mh, area.w, area.h - mh)),
            }
        };
        out.extend(split_even(masters, m_area, horizontal));
        out.extend(split_even(stack, s_area, horizontal));
        out
    }

    fn orientation_is_vertical_stack(&self) -> bool {
        matches!(self.orientation, Orientation::Left | Orientation::Right | Orientation::Center)
    }
}

/// Splits `area` evenly among `ids`, stacked vertically when `vertical`.
/// The last tile absorbs rounding so the tiles cover `area` exactly.
fn split_even(ids: &[WinId], area: Rect, vertical: bool) -> Vec<(WinId, Rect)> {
    let n = ids.len() as i32;
    let mut out = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        let i = i as i32;
        let r = if vertical {
            let y0 = area.y + area.h * i / n;
            let y1 = area.y + area.h * (i + 1) / n;
            Rect::new(area.x, y0, area.w, y1 - y0)
        } else {
            let x0 = area.x + area.w * i / n;
            let x1 = area.x + area.w * (i + 1) / n;
            Rect::new(x0, area.y, x1 - x0, area.h)
        };
        out.push((*id, r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1000, 900);
    const OPTS: MasterOptions = MasterOptions { new_status: NewStatus::Slave, new_on_top: false };

    fn build(n: u64, o: Orientation) -> Master {
        let mut m = Master::new(0.6, o);
        for i in 1..=n {
            m.insert(i, None, &OPTS);
        }
        m
    }

    #[test]
    fn single_window_fills() {
        assert_eq!(build(1, Orientation::Left).compute(AREA), vec![(1, AREA)]);
    }

    #[test]
    fn left_master_and_even_stack() {
        let r = build(4, Orientation::Left).compute(AREA);
        assert_eq!(r[0], (1, Rect::new(0, 0, 600, 900)));
        assert_eq!(r[1], (2, Rect::new(600, 0, 400, 300)));
        assert_eq!(r[3], (4, Rect::new(600, 600, 400, 300)));
    }

    #[test]
    fn all_orientations_cover_area() {
        for o in [Orientation::Left, Orientation::Right, Orientation::Top, Orientation::Bottom, Orientation::Center] {
            for n in 1..=7 {
                let r = build(n, o).compute(AREA);
                let total: i64 = r.iter().map(|(_, r)| r.area()).sum();
                assert_eq!(total, AREA.area(), "{o:?} n={n}");
            }
        }
    }

    #[test]
    fn new_status_master_and_on_top() {
        let mut m = Master::new(0.5, Orientation::Left);
        m.insert(1, None, &OPTS);
        m.insert(2, None, &MasterOptions { new_status: NewStatus::Master, new_on_top: false });
        assert_eq!(m.order, vec![2, 1]);
        m.insert(3, None, &MasterOptions { new_status: NewStatus::Slave, new_on_top: true });
        assert_eq!(m.order, vec![2, 3, 1]);
        m.insert(4, Some(2), &MasterOptions { new_status: NewStatus::Inherit, new_on_top: false });
        assert_eq!(m.order[0], 4);
    }

    #[test]
    fn swap_with_master_and_mfact() {
        let mut m = build(3, Orientation::Left);
        m.swap_with_master(3);
        assert_eq!(m.order, vec![3, 2, 1]);
        m.swap_with_master(3);
        assert_eq!(m.order, vec![2, 3, 1]);
        m.adjust_mfact(0.5, false);
        assert_eq!(m.mfact, 0.95);
    }
}
