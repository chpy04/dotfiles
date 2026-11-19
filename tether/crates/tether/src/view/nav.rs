//! Viewer navigation (PRD F3 "Navigation" table).
//!
//! Flowchart: the keys along the flow walk it, the keys across it switch branch, `HJKL` move on
//! screen. Along is `j`/`k` for `TB` and `l`/`h` for `LR` (the keymap picks, see
//! [`super::keys::Keymap::from_keys`]); below, "forward"/"back" and "next"/"previous" mean those keys.
//! * Forward steps along the highlighted outgoing edge (the branch last chosen at this node,
//!   else the first child by position). Back steps along the edge we arrived by, else to
//!   the first parent by position.
//! * Next / previous switch sibling branch: the other children of the node a back step
//!   would return to, ordered by target position (x for `TB`, y for `LR`), wrapping around. The
//!   parent remembers the choice, so back then forward comes back to the same branch. Roots
//!   switch between the roots.
//! * `H` `J` `K` `L` (and the arrows) move to the nearest node in that screen direction
//!   ([`App::spatial_target`]), the same for `TB` and `LR`. `J`/`K` remember the starting column
//!   (curswant).
//!
//! Sequence diagram (unchanged):
//! * `j`/`k` move to the next/previous message or note. On a participant they walk to the next/
//!   previous item on that lifeline after the last item visited.
//! * `h`/`l` move to the previous/next participant; from a message or note they step onto its
//!   left/right-most participant.
//!
//! `Ctrl-o` / `Ctrl-i` (jump list) live in `app.rs`.

use tether_core::diagram::Direction;
use tether_core::layout::{Nav, ObjectId, ObjectKind, Rect, SeqNav};

use super::app::{App, Effect, StatusKind};
use super::keys::Action;

/// Cells are about twice as tall as wide: one vertical cell counts this many horizontal ones.
pub const VERT_WEIGHT: i32 = 2;
/// Weight of the perpendicular offset against the gap along the movement when no candidate lies
/// in the beam.
pub const PERP_WEIGHT: i32 = 2;
/// A candidate may reach back this many cells over the current node's edge and still count as
/// lying in the direction.
pub const TOLERANCE: i32 = 1;

/// `(in beam: 0 / not: 1, weighted distance, perpendicular offset, y, x)`, smaller wins.
type Score = (u8, i32, i32, u16, u16);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Left,
    Right,
    Up,
    Down,
}

enum Step {
    /// Move focus to `target`.
    Go { target: ObjectId, came_from: Option<(ObjectId, ObjectId)>, branches: Vec<(ObjectId, ObjectId)> },
    /// Spatial move; remembers the column for runs of `J`/`K`.
    Spatial { target: ObjectId, col: Option<i32> },
    /// Switch to sibling branch number `index` of `of`; `parent` (the node it hangs off, when it
    /// has one) remembers it and `h` will return there.
    Branch { parent: Option<ObjectId>, target: ObjectId, index: usize, of: usize },
    /// Nothing to do; optionally explain.
    Stay(Option<String>),
}

impl App {
    pub(super) fn navigate(&mut self, a: Action) -> Vec<Effect> {
        if matches!(a, Action::Left | Action::Right) && self.is_flowchart() {
            self.col_ref = None; // H/L end a vertical run, moved or not
        }
        let step = match &self.layout.nav {
            Nav::Flow(_) => self.flow_step(a),
            Nav::Seq(n) => self.seq_step(n, a),
        };
        match step {
            Step::Go { target, came_from, branches } => {
                if let Some((c, p)) = came_from {
                    self.came_from.insert(c, p);
                }
                for (p, c) in branches {
                    self.branch.insert(p, c);
                }
                self.set_focus(target);
            }
            Step::Spatial { target, col } => {
                self.set_focus(target);
                self.col_ref = col;
            }
            Step::Branch { parent, target, index, of } => {
                let target_label = self.elements.get(&target).map(|e| e.label.clone()).unwrap_or_default();
                let text = match &parent {
                    Some(p) => {
                        self.branch.insert(p.clone(), target.clone());
                        self.came_from.insert(target.clone(), p.clone());
                        match self.edge_label(p, &target) {
                            Some(l) => format!("branch {}/{of}: {l} → {target_label}", index + 1),
                            None => format!("branch {}/{of}: → {target_label}", index + 1),
                        }
                    }
                    None => format!("start {}/{of}: {target_label}", index + 1),
                };
                self.set_focus(target);
                self.set_status(StatusKind::Info, text);
            }
            Step::Stay(Some(msg)) => self.set_status(StatusKind::Info, msg),
            Step::Stay(None) => {}
        }
        vec![]
    }

    /// Label of the flowchart edge `from -> to`, if it has one.
    fn edge_label(&self, from: &ObjectId, to: &ObjectId) -> Option<String> {
        let tether_core::diagram::Diagram::Flowchart(f) = &self.diagram else { return None };
        f.edges.iter().find(|e| e.from == from.as_str() && e.to == to.as_str()).and_then(|e| e.label.clone())
    }

    fn rect_of(&self, id: &ObjectId) -> Option<Rect> {
        self.layout.object(id.as_str()).map(|o| o.rect)
    }

    /// Outgoing targets of `id` ordered by position on screen (left to right for `TB`, top to
    /// bottom for `LR`; ties by the other axis, then id).
    pub fn branch_order(&self, id: &ObjectId) -> Vec<ObjectId> {
        let Nav::Flow(n) = &self.layout.nav else { return vec![] };
        let lr = matches!(&self.diagram, tether_core::diagram::Diagram::Flowchart(f) if f.direction() == Direction::LR);
        let mut v: Vec<ObjectId> = n.children.get(id).cloned().unwrap_or_default();
        v.dedup();
        let key = |t: &ObjectId| {
            let r = self.rect_of(t).unwrap_or(Rect { x: 0, y: 0, w: 0, h: 0 });
            let (a, b) = if lr { (r.y, r.x) } else { (r.x, r.y) };
            (a, b, t.clone())
        };
        v.sort_by_key(key);
        v.dedup();
        v
    }

    /// The outgoing edge's target that `J` follows and the viewer draws highlighted: the branch
    /// chosen with `H`/`L` (or by arriving back along it), else the first child.
    pub fn highlighted_branch(&self, id: &ObjectId) -> Option<ObjectId> {
        let Nav::Flow(n) = &self.layout.nav else { return None };
        let children = n.children.get(id)?;
        self.branch.get(id).filter(|c| children.contains(c)).or_else(|| children.first()).cloned()
    }

    /// The node `dir` of `focus` on screen, if any. See the constants above.
    ///
    /// Candidates are the other nodes whose rect lies in that direction (past the current rect's
    /// edge, with [`TOLERANCE`]). Those overlapping the current rect on the perpendicular axis
    /// (or containing the remembered column) form the beam; the nearest of them wins. With none
    /// in the beam, the lowest `gap + PERP_WEIGHT * offset` wins, vertical distances counting
    /// [`VERT_WEIGHT`] times. Ties break by smaller offset, then reading order, then id.
    fn spatial_target(&self, dir: Dir) -> Option<(ObjectId, i32)> {
        let cur = self.rect_of(&self.focus)?;
        let horizontal = matches!(dir, Dir::Left | Dir::Right);
        // reference coordinate on the perpendicular axis: the current centre; for j/k the
        // remembered column
        let center = |r: &Rect| -> i32 {
            if horizontal {
                r.y as i32 + r.h as i32 / 2
            } else {
                r.x as i32 + r.w as i32 / 2
            }
        };
        let reference = if horizontal { center(&cur) } else { self.col_ref.unwrap_or_else(|| center(&cur)) };
        let (c_lo, c_hi) = if horizontal { (cur.y as i32, cur.bottom() as i32) } else { (cur.x as i32, cur.right() as i32) };
        let mut best: Option<(Score, ObjectId)> = None;
        for o in &self.layout.objects {
            if !matches!(o.kind, ObjectKind::Node { .. }) || o.id == self.focus {
                continue;
            }
            let r = o.rect;
            let (x0, x1, y0, y1) = (r.x as i32, r.right() as i32, r.y as i32, r.bottom() as i32);
            // gap along the movement; None when the candidate is not in that direction
            let gap = match dir {
                Dir::Right => (x0 >= cur.right() as i32 - TOLERANCE).then(|| (x0 - cur.right() as i32).max(0)),
                Dir::Left => (x1 <= cur.x as i32 + TOLERANCE).then(|| (cur.x as i32 - x1).max(0)),
                Dir::Down => (y0 >= cur.bottom() as i32 - TOLERANCE).then(|| ((y0 - cur.bottom() as i32).max(0)) * VERT_WEIGHT),
                Dir::Up => (y1 <= cur.y as i32 + TOLERANCE).then(|| ((cur.y as i32 - y1).max(0)) * VERT_WEIGHT),
            };
            let Some(gap) = gap else { continue };
            let (lo, hi) = if horizontal { (y0, y1) } else { (x0, x1) };
            // offset of the reference from the candidate's span (0 inside it)
            let raw = if reference < lo {
                lo - reference
            } else if reference >= hi {
                reference - hi + 1
            } else {
                0
            };
            let offset = if horizontal { raw * VERT_WEIGHT } else { raw };
            let overlaps = lo < c_hi && c_lo < hi;
            let in_beam = overlaps || raw == 0;
            let key = if in_beam {
                (0u8, gap, offset, r.y, r.x)
            } else {
                (1u8, gap + PERP_WEIGHT * offset, offset, r.y, r.x)
            };
            if best.as_ref().is_none_or(|(k, b)| (key, &o.id) < (*k, b)) {
                best = Some((key, o.id.clone()));
            }
        }
        best.map(|(_, id)| (id, reference))
    }

    fn flow_step(&self, a: Action) -> Step {
        let me = &self.focus;
        let Nav::Flow(n) = &self.layout.nav else { return Step::Stay(None) };
        let spatial = |dir: Dir, word: &str| match self.spatial_target(dir) {
            Some((target, col)) => {
                let vertical = matches!(dir, Dir::Up | Dir::Down);
                Step::Spatial { target, col: vertical.then_some(col) }
            }
            None => Step::Stay(Some(format!("no node {word}"))),
        };
        match a {
            Action::Left => spatial(Dir::Left, "to the left"),
            Action::Right => spatial(Dir::Right, "to the right"),
            Action::Up => spatial(Dir::Up, "above"),
            Action::Down => spatial(Dir::Down, "below"),
            Action::StepForward => match self.highlighted_branch(me) {
                Some(t) => Step::Go { target: t.clone(), came_from: Some((t.clone(), me.clone())), branches: vec![(me.clone(), t)] },
                None => Step::Stay(Some("end of the flow".into())),
            },
            Action::StepBack => match self.back_target(me) {
                Some(t) => Step::Go { target: t.clone(), came_from: None, branches: vec![(t, me.clone())] },
                None => Step::Stay(Some("start of the flow".into())),
            },
            Action::BranchPrev | Action::BranchNext => {
                let next = a == Action::BranchNext;
                let (parent, order) = match self.back_target(me) {
                    Some(p) => {
                        let order = self.branch_order(&p);
                        (Some(p), order)
                    }
                    // a root: the other roots stand in for sibling branches
                    None => (None, n.roots.clone()),
                };
                let len = order.len();
                let Some(cur) = order.iter().position(|x| x == me).filter(|_| len >= 2) else {
                    return Step::Stay(Some("no other branch here".into()));
                };
                let i = if next { (cur + 1) % len } else { (cur + len - 1) % len };
                Step::Branch { parent, target: order[i].clone(), index: i, of: len }
            }
            _ => Step::Stay(None),
        }
    }

    /// Where `h` goes from `id`: the parent we arrived from, else the first parent by position.
    fn back_target(&self, id: &ObjectId) -> Option<ObjectId> {
        let Nav::Flow(n) = &self.layout.nav else { return None };
        let parents = n.parents.get(id)?;
        self.came_from.get(id).filter(|p| parents.contains(p)).or_else(|| parents.first()).cloned()
    }

    /// Participants (as indices into `n.participants`) at the ends of an item.
    fn seq_ends(&self, n: &SeqNav, id: &ObjectId) -> Vec<usize> {
        let ids: Vec<&ObjectId> = match self.layout.object(id.as_str()).map(|o| &o.kind) {
            Some(ObjectKind::Message { from, to, .. }) => vec![from, to],
            Some(ObjectKind::Note { over }) => over.iter().collect(),
            _ => vec![],
        };
        ids.iter().filter_map(|p| n.participants.iter().position(|q| q == *p)).collect()
    }

    fn seq_step(&self, n: &SeqNav, a: Action) -> Step {
        let me = &self.focus;
        let item_pos = n.items.iter().position(|x| x == me);
        let part_pos = n.participants.iter().position(|x| x == me);
        let go = |t: &ObjectId| Step::Go { target: t.clone(), came_from: None, branches: vec![] };
        match a {
            Action::Down | Action::Up => {
                let fwd = a == Action::Down;
                if let Some(i) = item_pos {
                    let j = if fwd { i + 1 } else { i.wrapping_sub(1) };
                    return match n.items.get(j) {
                        Some(t) => go(t),
                        None => Step::Stay(Some((if fwd { "last message" } else { "first message" }).into())),
                    };
                }
                if let Some(p) = part_pos {
                    // walk the lifeline of this participant from the last visited item
                    let on_lane = |idx: usize| self.seq_ends(n, &n.items[idx]).contains(&p);
                    let cur = self.seq_cursor;
                    let found = if fwd {
                        (cur + 1..n.items.len()).find(|&i| on_lane(i))
                    } else {
                        (0..cur).rev().find(|&i| on_lane(i))
                    };
                    return match found {
                        Some(i) => go(&n.items[i]),
                        None => Step::Stay(Some((if fwd { "no later message on this lifeline" } else { "no earlier message on this lifeline" }).into())),
                    };
                }
                Step::Stay(None)
            }
            Action::Left | Action::Right => {
                let fwd = a == Action::Right;
                if let Some(p) = part_pos {
                    let q = if fwd { p + 1 } else { p.wrapping_sub(1) };
                    return match n.participants.get(q) {
                        Some(t) => go(t),
                        None => Step::Stay(None),
                    };
                }
                if item_pos.is_some() {
                    let ends = self.seq_ends(n, me);
                    let pick = if fwd { ends.iter().max() } else { ends.iter().min() };
                    if let Some(&p) = pick {
                        return go(&n.participants[p]);
                    }
                }
                Step::Stay(None)
            }
            _ => Step::Stay(None),
        }
    }

    /// `g` / `G`: first or last focusable object.
    pub(super) fn jump_end(&mut self, last: bool) -> Vec<Effect> {
        let target = match &self.layout.nav {
            Nav::Flow(n) => {
                if last {
                    n.order.last().cloned()
                } else {
                    n.roots.first().or(n.order.first()).cloned()
                }
            }
            Nav::Seq(n) => {
                if last {
                    n.items.last().or(n.participants.last()).cloned()
                } else {
                    n.items.first().or(n.participants.first()).cloned()
                }
            }
        };
        if let Some(t) = target {
            self.came_from.remove(&t);
            self.set_focus(t);
        }
        vec![]
    }
}
