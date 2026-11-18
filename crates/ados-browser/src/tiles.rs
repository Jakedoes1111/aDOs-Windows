//! Tiled page tabs and follower tabs, as in Vivaldi. Pure bookkeeping and geometry;
//! `tile_view.rs` draws it, `driver.rs` gives each tile its own engine window.
use serde::{Deserialize, Serialize};

/// At most this many tiles in one group: each shows a live page in its own engine window.
pub const MAX: usize = 4;
/// Space between tiles, in logical pixels; the divider is drawn and dragged there.
pub const GAP: f32 = 6.;
/// No tile is made smaller than this share of its row or column.
const MIN_SHARE: f32 = 0.1;
/// One keyboard step of a divider or of Grow / Shrink.
pub const STEP: f32 = 0.05;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    /// Side by side.
    #[default]
    Columns,
    /// Stacked.
    Rows,
    Grid,
}
impl Layout {
    pub fn label(self) -> &'static str {
        match self { Self::Columns => "side by side", Self::Rows => "stacked", Self::Grid => "in a grid" }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Tabs in strip order.
    pub tabs: Vec<u64>,
    pub layout: Layout,
    /// Shares of the columns (side by side, grid) or rows (stacked); they sum to 1.
    pub sizes: Vec<f32>,
    /// Shares of the grid's rows.
    pub cross: Vec<f32>,
}

/// Which way a divider runs: a `Vertical` one sits between columns and moves left and right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis { Vertical, Horizontal }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub axis: Axis,
    /// Between share `index` and `index + 1`.
    pub index: usize,
    pub rect: Area,
}

fn even(n: usize) -> Vec<f32> { vec![1. / n.max(1) as f32; n.max(1)] }

/// Columns and rows of a grid of `n` tiles: 3 tiles are two on top and one wide below.
fn grid_shape(n: usize) -> (usize, usize) {
    let cols = (n as f32).sqrt().ceil().max(1.) as usize;
    (cols, n.div_ceil(cols))
}

impl Group {
    fn new(tabs: Vec<u64>, layout: Layout) -> Self {
        let mut group = Self { tabs, layout, sizes: vec![], cross: vec![] };
        group.reset_sizes();
        group
    }
    fn reset_sizes(&mut self) {
        let n = self.tabs.len();
        let (cols, rows) = match self.layout {
            Layout::Columns => (n, 1),
            Layout::Rows => (n, 1),
            Layout::Grid => grid_shape(n),
        };
        self.sizes = even(cols);
        self.cross = even(rows);
    }
    /// Saved shares that do not fit the tile count are replaced by even ones.
    pub fn with_sizes(mut self, sizes: Vec<f32>, cross: Vec<f32>) -> Self {
        let valid = |v: &Vec<f32>, n: usize| v.len() == n && v.iter().all(|s| s.is_finite() && *s > 0.) && (v.iter().sum::<f32>() - 1.).abs() < 0.01;
        if valid(&sizes, self.sizes.len()) { self.sizes = sizes; }
        if valid(&cross, self.cross.len()) { self.cross = cross; }
        self
    }

    /// Each tile's rectangle inside `area`, in the group's order.
    pub fn rects(&self, area: Area) -> Vec<(u64, Area)> {
        let n = self.tabs.len();
        match self.layout {
            Layout::Columns => split(area.x, area.w, &self.sizes).into_iter().zip(&self.tabs)
                .map(|((x, w), id)| (*id, Area { x, y: area.y, w, h: area.h })).collect(),
            Layout::Rows => split(area.y, area.h, &self.sizes).into_iter().zip(&self.tabs)
                .map(|((y, h), id)| (*id, Area { x: area.x, y, w: area.w, h })).collect(),
            Layout::Grid => {
                let (cols, _) = grid_shape(n);
                let rows = split(area.y, area.h, &self.cross);
                let full = split(area.x, area.w, &self.sizes);
                self.tabs.iter().enumerate().map(|(i, id)| {
                    let (row, col) = (i / cols, i % cols);
                    let (y, h) = rows[row];
                    let in_row = (n - row * cols).min(cols);
                    // A short last row shares its width evenly.
                    let (x, w) = if in_row == cols { full[col] } else { split(area.x, area.w, &even(in_row))[col] };
                    (*id, Area { x, y, w, h })
                }).collect()
            }
        }
    }

    /// The draggable gaps between tiles.
    pub fn dividers(&self, area: Area) -> Vec<Divider> {
        let gaps = |spans: &[(f32, f32)]| -> Vec<f32> { spans.windows(2).map(|w| w[0].0 + w[0].1).collect() };
        match self.layout {
            Layout::Columns => gaps(&split(area.x, area.w, &self.sizes)).into_iter().enumerate()
                .map(|(index, x)| Divider { axis: Axis::Vertical, index, rect: Area { x, y: area.y, w: GAP, h: area.h } }).collect(),
            Layout::Rows => gaps(&split(area.y, area.h, &self.sizes)).into_iter().enumerate()
                .map(|(index, y)| Divider { axis: Axis::Horizontal, index, rect: Area { x: area.x, y, w: area.w, h: GAP } }).collect(),
            Layout::Grid => {
                let n = self.tabs.len();
                let (cols, _) = grid_shape(n);
                let rows = split(area.y, area.h, &self.cross);
                // Column dividers run down the full rows only.
                let full_rows = n / cols;
                let bottom = rows.get(full_rows.saturating_sub(1)).map_or(area.y, |(y, h)| y + h);
                let mut out: Vec<Divider> = gaps(&split(area.x, area.w, &self.sizes)).into_iter().enumerate()
                    .map(|(index, x)| Divider { axis: Axis::Vertical, index, rect: Area { x, y: area.y, w: GAP, h: bottom - area.y } })
                    .filter(|d| d.rect.h > 0.).collect();
                out.extend(gaps(&rows).into_iter().enumerate()
                    .map(|(index, y)| Divider { axis: Axis::Horizontal, index, rect: Area { x: area.x, y, w: area.w, h: GAP } }));
                out
            }
        }
    }

    fn shares_mut(&mut self, axis: Axis) -> &mut Vec<f32> {
        match (self.layout, axis) {
            (Layout::Grid, Axis::Horizontal) => &mut self.cross,
            _ => &mut self.sizes,
        }
    }
    /// Move divider `index` by `delta` of the whole length (keyboard).
    pub fn nudge(&mut self, axis: Axis, index: usize, delta: f32) {
        let shares = self.shares_mut(axis);
        if index + 1 >= shares.len() { return; }
        let pair = shares[index] + shares[index + 1];
        let left = (shares[index] + delta).clamp(MIN_SHARE.min(pair / 2.), pair - MIN_SHARE.min(pair / 2.));
        shares[index] = left;
        shares[index + 1] = pair - left;
    }
    /// Put divider `index` where the pointer is (`pos` along its axis, logical pixels).
    pub fn drag(&mut self, axis: Axis, index: usize, pos: f32, area: Area) {
        let (start, len) = match axis { Axis::Vertical => (area.x, area.w), Axis::Horizontal => (area.y, area.h) };
        let shares = self.shares_mut(axis);
        if index + 1 >= shares.len() { return; }
        let spans = split(start, len, shares);
        let (a, b) = (spans[index], spans[index + 1]);
        let pair_len = a.1 + b.1;
        if pair_len <= 0. { return; }
        let pair = shares[index] + shares[index + 1];
        let left = (pos - GAP / 2. - a.0).clamp(0., pair_len) / pair_len * pair;
        let floor = MIN_SHARE.min(pair / 2.);
        shares[index] = left.clamp(floor, pair - floor);
        shares[index + 1] = pair - shares[index];
    }
    /// Grow (or shrink, `delta < 0`) the tile `id` along the layout's main direction, taking
    /// from the neighbour after it, or before it when it is last. In a grid both its column
    /// and its row change.
    pub fn grow(&mut self, id: u64, delta: f32) {
        let Some(i) = self.tabs.iter().position(|t| *t == id) else { return };
        let (index, row) = match self.layout {
            Layout::Columns | Layout::Rows => (i, None),
            Layout::Grid => { let (cols, _) = grid_shape(self.tabs.len()); (i % cols, Some(i / cols)) }
        };
        let main = if self.layout == Layout::Rows { Axis::Horizontal } else { Axis::Vertical };
        let step = |group: &mut Self, axis: Axis, index: usize| {
            let n = group.shares_mut(axis).len();
            if n < 2 { return; }
            if index + 1 < n { group.nudge(axis, index, delta) } else { group.nudge(axis, index - 1, -delta) }
        };
        step(self, main, index);
        if let Some(row) = row { step(self, Axis::Horizontal, row); }
    }
}

/// Lengths for `shares` along a span, with `GAP` between: (start, length) each.
fn split(start: f32, len: f32, shares: &[f32]) -> Vec<(f32, f32)> {
    let n = shares.len().max(1);
    let free = (len - GAP * (n - 1) as f32).max(0.);
    let total: f32 = shares.iter().sum::<f32>().max(f32::EPSILON);
    let mut at = start;
    shares.iter().map(|s| {
        let l = free * s / total;
        let span = (at, l);
        at += l + GAP;
        span
    }).collect()
}

#[derive(Clone, Debug, Default)]
pub struct Tiles {
    pub groups: Vec<Group>,
    /// (follower, leader): plain link clicks in the leader open in the follower.
    pub follows: Vec<(u64, u64)>,
}

impl Tiles {
    pub fn group_of(&self, id: u64) -> Option<usize> { self.groups.iter().position(|g| g.tabs.contains(&id)) }
    pub fn group(&self, id: u64) -> Option<&Group> { self.group_of(id).map(|g| &self.groups[g]) }
    pub fn group_mut(&mut self, id: u64) -> Option<&mut Group> { self.group_of(id).map(|g| &mut self.groups[g]) }
    pub fn leader_of(&self, id: u64) -> Option<u64> { self.follows.iter().find(|(f, _)| *f == id).map(|(_, l)| *l) }
    pub fn follower_of(&self, id: u64) -> Option<u64> { self.follows.iter().find(|(_, l)| *l == id).map(|(f, _)| *f) }

    /// The tabs shown when `id` is selected: its whole group, or just itself.
    pub fn shown(&self, id: u64) -> Vec<u64> { self.group(id).map_or(vec![id], |g| g.tabs.clone()) }

    /// Tile `tabs` (strip order) as one new group; they leave any group they were in.
    pub fn tile(&mut self, tabs: Vec<u64>, layout: Layout) -> Result<(), String> {
        if tabs.len() < 2 { return Err("Select two or more tabs to tile: Ctrl+click or Shift+click them in the tab strip.".into()); }
        if tabs.len() > MAX { return Err(format!("Up to {MAX} tiles: each shows a live page in its own engine window.")); }
        // The same tabs again: only the arrangement changes, sizes start even.
        if let Some(g) = self.group_of(tabs[0]).filter(|g| self.groups[*g].tabs == tabs) {
            let group = &mut self.groups[g];
            group.layout = layout;
            group.reset_sizes();
            return Ok(());
        }
        for id in &tabs { self.leave(*id); }
        self.groups.push(Group::new(tabs, layout));
        Ok(())
    }
    /// The arrangement of `id`'s group.
    pub fn arrange(&mut self, id: u64, layout: Layout) -> bool {
        let Some(group) = self.group_mut(id) else { return false };
        if group.layout != layout { group.layout = layout; group.reset_sizes(); }
        true
    }
    /// Tile `id` with `next`: a new pair side by side, or `next` joins `id`'s group.
    pub fn tile_with(&mut self, id: u64, next: u64) -> Result<(), String> {
        if id == next { return Err("There is no other tab to tile with.".into()); }
        match self.group_of(id) {
            Some(g) if self.groups[g].tabs.contains(&next) => Ok(()),
            Some(g) if self.groups[g].tabs.len() >= MAX => Err(format!("Up to {MAX} tiles: each shows a live page in its own engine window.")),
            Some(g) => {
                let layout = self.groups[g].layout;
                let mut tabs = self.groups[g].tabs.clone();
                let at = tabs.iter().position(|t| *t == id).map_or(tabs.len(), |i| i + 1);
                tabs.insert(at, next);
                self.leave(next);
                let g = self.group_of(id).unwrap_or(0);
                if let Some(group) = self.groups.get_mut(g) { *group = Group::new(tabs, layout); }
                Ok(())
            }
            None => self.tile(vec![id, next], Layout::Columns),
        }
    }
    /// `id` leaves its group (a group of one is no group); its follower link ends.
    pub fn untile(&mut self, id: u64) { self.leave(id); }
    pub fn untile_group(&mut self, id: u64) {
        if let Some(g) = self.group_of(id) {
            for tab in self.groups[g].tabs.clone() { self.unfollow(tab); }
            self.groups.remove(g);
        }
    }
    fn leave(&mut self, id: u64) {
        self.unfollow(id);
        let Some(g) = self.group_of(id) else { return };
        let group = &mut self.groups[g];
        let layout = group.layout;
        let tabs: Vec<u64> = group.tabs.iter().copied().filter(|t| *t != id).collect();
        if tabs.len() < 2 {
            if let Some(last) = tabs.first() { self.unfollow(*last); }
            self.groups.remove(g);
        } else {
            *group = Group::new(tabs, layout);
        }
    }
    /// Tab `id` closed: it leaves its group; a tab that followed it becomes an ordinary tile.
    pub fn closed(&mut self, id: u64) { self.leave(id); }
    fn unfollow(&mut self, id: u64) { self.follows.retain(|(f, l)| *f != id && *l != id); }

    /// Make `id` follow the tile before it in its group (after it when it is first), or stop
    /// following. A tile leads at most one follower, and a follower leads none.
    pub fn toggle_follow(&mut self, id: u64) -> Result<bool, String> {
        if self.leader_of(id).is_some() {
            self.follows.retain(|(f, _)| *f != id);
            return Ok(false);
        }
        let Some(group) = self.group(id) else { return Err("Tile this tab first: a follower is a tile that shows the links clicked in another tile.".into()) };
        let i = group.tabs.iter().position(|t| *t == id).unwrap_or(0);
        let others: Vec<u64> = group.tabs[..i].iter().rev().chain(group.tabs[i + 1..].iter()).copied().collect();
        let leader = others.into_iter().find(|l| self.leader_of(*l).is_none() && self.follower_of(*l).is_none());
        let Some(leader) = leader else { return Err("Every other tile already leads or follows a tile.".into()) };
        self.follows.retain(|(_, l)| *l != id);
        self.follows.push((id, leader));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Area = Area { x: 0., y: 0., w: 806., h: 606. };

    #[test]
    fn tile_needs_two_and_caps() {
        let mut t = Tiles::default();
        assert!(t.tile(vec![1], Layout::Columns).is_err());
        assert!(t.tile(vec![1, 2, 3, 4, 5], Layout::Grid).is_err());
        t.tile(vec![1, 2, 3], Layout::Rows).unwrap();
        assert_eq!(t.shown(2), [1, 2, 3]);
        assert_eq!(t.shown(9), [9]);
    }
    #[test]
    fn columns_rows_grid_rects() {
        let g = Group::new(vec![1, 2], Layout::Columns);
        let r = g.rects(AREA);
        assert_eq!(r[0].1, Area { x: 0., y: 0., w: 400., h: 606. });
        assert_eq!(r[1].1, Area { x: 406., y: 0., w: 400., h: 606. });
        let g = Group::new(vec![1, 2], Layout::Rows);
        assert_eq!(g.rects(AREA)[1].1, Area { x: 0., y: 306., w: 806., h: 300. });
        let g = Group::new(vec![1, 2, 3], Layout::Grid);
        let r = g.rects(AREA);
        assert_eq!(r[1].1, Area { x: 406., y: 0., w: 400., h: 300. });
        // Three in a grid: the third is wide below.
        assert_eq!(r[2].1, Area { x: 0., y: 306., w: 806., h: 300. });
        let d = g.dividers(AREA);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].rect, Area { x: 400., y: 0., w: GAP, h: 300. });
        assert_eq!(d[1].rect, Area { x: 0., y: 300., w: 806., h: GAP });
    }
    #[test]
    fn drag_and_nudge_keep_sum_and_minimum() {
        let mut g = Group::new(vec![1, 2], Layout::Columns);
        g.drag(Axis::Vertical, 0, 203., AREA);
        assert!((g.sizes[0] - 0.5 * 200. / 400.).abs() < 0.01, "{:?}", g.sizes);
        g.drag(Axis::Vertical, 0, -500., AREA);
        assert!((g.sizes[0] - MIN_SHARE).abs() < 1e-4);
        g.nudge(Axis::Vertical, 0, STEP);
        assert!((g.sizes.iter().sum::<f32>() - 1.).abs() < 1e-4);
        assert!((g.sizes[0] - (MIN_SHARE + STEP)).abs() < 1e-4);
        let mut g = Group::new(vec![1, 2, 3], Layout::Columns);
        g.grow(3, STEP);
        assert!(g.sizes[2] > g.sizes[1]);
    }
    #[test]
    fn untile_and_followers() {
        let mut t = Tiles::default();
        t.tile(vec![1, 2, 3], Layout::Columns).unwrap();
        assert_eq!(t.toggle_follow(2), Ok(true));
        assert_eq!(t.leader_of(2), Some(1));
        // 1 leads 2 and 2 follows: 3 cannot pick either.
        assert!(t.toggle_follow(3).is_err());
        // The leader closes: the follower is an ordinary tile again.
        t.closed(1);
        assert_eq!(t.leader_of(2), None);
        assert_eq!(t.shown(2), [2, 3]);
        t.untile(3);
        assert!(t.groups.is_empty());
        assert!(t.toggle_follow(2).is_err());
    }
    #[test]
    fn tile_with_next_grows_group() {
        let mut t = Tiles::default();
        t.tile_with(1, 2).unwrap();
        t.tile_with(2, 3).unwrap();
        assert_eq!(t.shown(1), [1, 2, 3]);
        t.tile_with(3, 4).unwrap();
        assert!(t.tile_with(4, 5).is_err());
        t.untile_group(1);
        assert!(t.groups.is_empty());
    }
    #[test]
    fn saved_sizes_must_fit() {
        let g = Group::new(vec![1, 2], Layout::Columns).with_sizes(vec![0.3, 0.7], vec![]);
        assert_eq!(g.sizes, [0.3, 0.7]);
        let g = Group::new(vec![1, 2], Layout::Columns).with_sizes(vec![0.3], vec![]);
        assert_eq!(g.sizes, [0.5, 0.5]);
    }
}
