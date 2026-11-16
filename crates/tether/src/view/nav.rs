//! hjkl navigation (PRD F3 "Navigation" table).
//!
//! Flowchart:
//! * `j` follows an outgoing edge: the branch last chosen at this node, else the first child.
//! * `k` goes back to the parent we came from (else the first parent).
//! * `h`/`l` at a decision (two or more children) pick the previous/next branch, which `j` then
//!   follows (the chosen edge is drawn bold). Past the first/last branch, and on every other
//!   node, they move between siblings: the neighbours in the same rank, left/right.
//!
//! Sequence diagram:
//! * `j`/`k` move to the next/previous message or note. On a participant they walk to the next/
//!   previous item on that lifeline after the last item visited.
//! * `h`/`l` move to the previous/next participant; from a message or note they step onto its
//!   left/right-most participant.

use tether_core::layout::{FlowNav, Nav, ObjectId, ObjectKind, SeqNav};

use super::app::{App, Effect, StatusKind};
use super::keys::Action;

enum Step {
    /// Move focus to `target`.
    Go {
        target: ObjectId,
        came_from: Option<(ObjectId, ObjectId)>,
        branches: Vec<(ObjectId, ObjectId)>,
    },
    /// Only remember a branch choice (and say so).
    Pick { parent: ObjectId, child: ObjectId },
    /// Nothing to do; optionally explain.
    Stay(Option<&'static str>),
}

impl App {
    pub(super) fn navigate(&mut self, a: Action) -> Vec<Effect> {
        let step = match &self.layout.nav {
            Nav::Flow(n) => self.flow_step(n, a),
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
            Step::Pick { parent, child } => {
                let label = self.edge_label(&parent, &child);
                let child_label = self.elements.get(&child).map(|e| e.label.clone()).unwrap_or_default();
                self.branch.insert(parent, child);
                self.set_status(
                    StatusKind::Info,
                    match label {
                        Some(l) => format!("branch: {l} → {child_label} · j to follow"),
                        None => format!("branch → {child_label} · j to follow"),
                    },
                );
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

    fn flow_step(&self, n: &FlowNav, a: Action) -> Step {
        let me = &self.focus;
        let empty = Vec::new();
        let children = n.children.get(me).unwrap_or(&empty);
        let parents = n.parents.get(me).unwrap_or(&empty);
        match a {
            Action::Down => {
                let chosen = self.branch.get(me).filter(|c| children.contains(c));
                match chosen.or_else(|| children.first()) {
                    Some(t) => Step::Go {
                        target: t.clone(),
                        came_from: Some((t.clone(), me.clone())),
                        branches: vec![(me.clone(), t.clone())],
                    },
                    None => Step::Stay(Some("end of the flow")),
                }
            }
            Action::Up => {
                let back = self.came_from.get(me).filter(|p| parents.contains(p));
                match back.or_else(|| parents.first()) {
                    Some(t) => Step::Go { target: t.clone(), came_from: None, branches: vec![(t.clone(), me.clone())] },
                    None => Step::Stay(Some("start of the flow")),
                }
            }
            Action::Left | Action::Right => {
                let d: i32 = if a == Action::Left { -1 } else { 1 };
                if children.len() >= 2 {
                    let cur = self.branch.get(me).and_then(|c| children.iter().position(|x| x == c)).unwrap_or(0) as i32;
                    let next = cur + d;
                    if next >= 0 && (next as usize) < children.len() {
                        return Step::Pick { parent: me.clone(), child: children[next as usize].clone() };
                    }
                }
                match n.sibling(me.as_str(), d) {
                    Some(sib) => {
                        let common: Vec<ObjectId> = n
                            .parents
                            .get(sib)
                            .map(|ps| ps.iter().filter(|p| parents.contains(p)).cloned().collect())
                            .unwrap_or_default();
                        let came_from = self
                            .came_from
                            .get(me)
                            .filter(|p| common.contains(p))
                            .map(|p| (sib.clone(), p.clone()));
                        Step::Go { target: sib.clone(), came_from, branches: common.into_iter().map(|p| (p, sib.clone())).collect() }
                    }
                    None => Step::Stay(None),
                }
            }
            _ => Step::Stay(None),
        }
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
                        None => Step::Stay(Some(if fwd { "last message" } else { "first message" })),
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
                        None => Step::Stay(Some(if fwd { "no later message on this lifeline" } else { "no earlier message on this lifeline" })),
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
