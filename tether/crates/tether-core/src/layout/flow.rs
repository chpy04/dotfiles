//! Flowchart layout.
//!
//! Pipeline (all positions in terminal cells):
//!
//! 1. Measure every node (wrapped label, icon, badge) and, for expanded nodes, build the
//!    details panel. The node and its panel form one *box* that is handed to the placement
//!    engine (Graphviz `dot -Tplain`, or the naive fallback) so that neighbours make room.
//!    The panel sits beside the node on the cross axis (right in `TB`, below in `LR`).
//! 2. Take ranks and cross-axis centres from the engine and snap them to the cell grid.
//! 3. Route every edge ourselves, orthogonally, in "rank space": rank bands hold nodes,
//!    *channels* between bands hold horizontal lanes. See [`route`] for the scheme.
//! 4. Place edge labels next to the lines, convert to real x/y and build objects + navigation.
//!
//! Internally everything is computed in abstract coordinates `(a, b)`: `a` runs along the flow
//! (down for `TB`, right for `LR`), `b` across it. `TB` maps `(a, b)` to `(x=b, y=a)`, `LR` to
//! `(x=a, y=b)`.

// Index-based graph code: indexing by rank/node/edge number is clearer than iterators here.
#![allow(clippy::needless_range_loop, clippy::type_complexity)]

use std::collections::{HashMap, HashSet};

use crate::diagram::{Direction, EdgeStyle, Flowchart, Shape};

use super::common::{adorn_width, box_decor, build_panel, panel_object, PanelSpec};
use super::dotgraph::place;
use super::text::{max_width, wrap};
use super::{
    Dir, FlowNav, IRect, Layout, LayoutError, LayoutOptions, Nav, ObjectId, RawEdge, RawKind,
    RawObject,
};

/// Minimum rows/columns of an inter-rank channel (stub row, one lane, arrow row).
const GMIN: i32 = 3;
/// Empty cells between a node and its details panel.
const PANEL_GAP: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anchor {
    /// The vertical part runs from the top of the channel down to the lane.
    Top,
    /// The vertical part runs from the lane down to the bottom of the channel.
    Bottom,
}

/// One edge's passage through one channel: a horizontal run on a lane between two vertical
/// parts. Both parts lie on columns `c1`/`c2`.
#[derive(Clone, Copy, Debug)]
struct Seg {
    edge: usize,
    c1: i32,
    a1: Anchor,
    c2: i32,
    a2: Anchor,
}

impl Seg {
    fn lo(&self) -> i32 {
        self.c1.min(self.c2)
    }
    fn hi(&self) -> i32 {
        self.c1.max(self.c2)
    }
    fn ends(&self) -> [(i32, Anchor); 2] {
        [(self.c1, self.a1), (self.c2, self.a2)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Fwd,
    Back,
    Same,
    Loop,
}

struct N {
    id: String,
    shape: Shape,
    lines: Vec<String>,
    label_w: i32,
    details: usize,
    has_code: bool,
    /// Real node size.
    nw: i32,
    nh: i32,
    panel: Option<PanelSpec>,
    /// Abstract sizes of node and panel.
    na: i32,
    nb: i32,
    pa: i32,
    pb: i32,
    box_b: i32,
    rank: usize,
    /// Left edge of the box on the cross axis.
    b0: i32,
    /// Node start on the rank axis.
    a0: i32,
    /// Panel start on the rank axis.
    pa0: i32,
    has_loop: bool,
}

impl N {
    /// Centre cell of the node on the cross axis.
    fn nc(&self) -> i32 {
        self.b0 + (self.nb - 1) / 2
    }
}

struct E {
    spec: usize,
    s: usize,
    t: usize,
    kind: Kind,
    /// Cross column at each intermediate rank (forward edges spanning 2+ ranks).
    virt: Vec<i32>,
    /// Back edges: corridor on the right (true) or left of the nodes.
    right: bool,
    corridor: i32,
    p_from: i32,
    p_to: i32,
    dot: Option<usize>,
}

pub(super) fn layout_flowchart(
    f: &Flowchart,
    expanded: &HashSet<ObjectId>,
    opts: &LayoutOptions,
) -> Result<Layout, LayoutError> {
    let lr = f.direction() == Direction::LR;
    // Minimum empty cells between boxes of one rank, and the gap handed to the engine.
    let (min_gap, dot_gap) = if lr { (3, 4) } else { (5, 6) };

    // ---------------------------------------------------------------- 1. measure
    let mut id_ix: HashMap<&str, usize> = HashMap::new();
    for (i, n) in f.nodes.iter().enumerate() {
        if id_ix.insert(n.id.as_str(), i).is_some() {
            return Err(LayoutError::DuplicateId(n.id.clone()));
        }
    }
    let mut nodes: Vec<N> = Vec::new();
    for n in &f.nodes {
        let lines = wrap(&n.label, opts.max_label_width as usize);
        let label_w = max_width(&lines) as i32;
        let details = n.details.len();
        let has_code = n.code.is_some();
        let nw = (6 + label_w + adorn_width(details, has_code)).max(10);
        let mut nh = lines.len() as i32 + 2;
        if lr {
            // ports sit on the left/right sides: give each edge its own row
            let out = f
                .edges
                .iter()
                .filter(|e| e.from == n.id && e.to != n.id)
                .count();
            let inn = f
                .edges
                .iter()
                .filter(|e| e.to == n.id && e.from != n.id)
                .count();
            nh = nh.max(out.max(inn) as i32 + 2);
        }
        let panel = if expanded.contains(n.id.as_str()) {
            build_panel(&n.details, n.code.as_ref(), opts.max_panel_width)
        } else {
            None
        };
        let (na, nb) = if lr { (nw, nh) } else { (nh, nw) };
        let (pa, pb) = match &panel {
            Some(p) if lr => (p.w, p.h),
            Some(p) => (p.h, p.w),
            None => (0, 0),
        };
        let box_b = nb + if panel.is_some() { PANEL_GAP + pb } else { 0 };
        nodes.push(N {
            id: n.id.clone(),
            shape: n.shape(),
            lines,
            label_w,
            details,
            has_code,
            nw,
            nh,
            panel,
            na,
            nb,
            pa,
            pb,
            box_b,
            rank: 0,
            b0: 0,
            a0: 0,
            pa0: 0,
            has_loop: false,
        });
    }
    let mut edges: Vec<E> = Vec::new();
    for (i, e) in f.edges.iter().enumerate() {
        let s = *id_ix
            .get(e.from.as_str())
            .ok_or_else(|| LayoutError::UnknownId(e.from.clone()))?;
        let t = *id_ix
            .get(e.to.as_str())
            .ok_or_else(|| LayoutError::UnknownId(e.to.clone()))?;
        edges.push(E {
            spec: i,
            s,
            t,
            kind: Kind::Fwd,
            virt: Vec::new(),
            right: true,
            corridor: 0,
            p_from: 0,
            p_to: 0,
            dot: None,
        });
        if s == t {
            nodes[s].has_loop = true;
        }
    }

    // ---------------------------------------------------------------- 2. placement
    let sizes: Vec<(i32, i32)> = nodes
        .iter()
        .map(|n| {
            let has_p = n.panel.is_some();
            if lr {
                let w = n.nw.max(if has_p { n.pa } else { 0 });
                let h = n.nh + if has_p { PANEL_GAP + n.pb } else { 0 };
                (w, h)
            } else {
                let w = n.nw + if has_p { PANEL_GAP + n.pb } else { 0 };
                let h = n.nh.max(if has_p { n.pa } else { 0 });
                (w, h)
            }
        })
        .collect();
    let mut dot_edges: Vec<(usize, usize)> = Vec::new();
    for e in edges.iter_mut() {
        if e.s != e.t {
            e.dot = Some(dot_edges.len());
            dot_edges.push((e.s, e.t));
        }
    }
    let placement = place(&sizes, &dot_edges, lr, dot_gap, opts)?;
    let nranks = placement.rank.iter().copied().max().map_or(1, |m| m + 1);
    for (i, n) in nodes.iter_mut().enumerate() {
        n.rank = placement.rank[i];
        n.b0 = placement.cross[i].round() as i32 - (n.box_b - 1) / 2;
    }
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); nranks];
    for (i, n) in nodes.iter().enumerate() {
        rows[n.rank].push(i);
    }
    for row in rows.iter_mut() {
        row.sort_by_key(|&i| (nodes[i].b0, i));
        // enforce minimum separation after rounding
        let mut prev_right: Option<(i32, bool)> = None;
        for &i in row.iter() {
            if let Some((pr, ploop)) = prev_right {
                let gap = min_gap + if ploop || nodes[i].has_loop { 4 } else { 0 };
                if nodes[i].b0 < pr + gap {
                    nodes[i].b0 = pr + gap;
                }
            }
            prev_right = Some((nodes[i].b0 + nodes[i].box_b, nodes[i].has_loop));
        }
    }

    // classify edges
    for e in edges.iter_mut() {
        let (rs, rt) = (nodes[e.s].rank, nodes[e.t].rank);
        e.kind = if e.s == e.t {
            Kind::Loop
        } else if rs < rt {
            Kind::Fwd
        } else if rs > rt {
            Kind::Back
        } else {
            Kind::Same
        };
    }

    // Chains: a node with one incoming and a parent with one outgoing port are drawn as a
    // straight line when rounding left them one cell apart.
    {
        let mut low_n = vec![0usize; nodes.len()];
        let mut high_n = vec![0usize; nodes.len()];
        for e in &edges {
            match e.kind {
                Kind::Loop => {}
                Kind::Fwd | Kind::Back => {
                    low_n[e.s] += 1;
                    high_n[e.t] += 1;
                }
                Kind::Same => {
                    low_n[e.s] += 1;
                    low_n[e.t] += 1;
                }
            }
        }
        for r in 0..nranks {
            for k in 0..rows[r].len() {
                let t = rows[r][k];
                let parent = edges
                    .iter()
                    .find(|e| e.kind == Kind::Fwd && e.t == t && nodes[e.s].rank + 1 == r);
                let Some(e) = parent else { continue };
                if high_n[t] != 1 || low_n[e.s] != 1 {
                    continue;
                }
                let delta = nodes[e.s].nc() - nodes[t].nc();
                if delta == 0 || delta.abs() > 1 {
                    continue;
                }
                let new_b0 = nodes[t].b0 + delta;
                let own = |i: usize| {
                    min_gap
                        + if nodes[i].has_loop || nodes[t].has_loop {
                            4
                        } else {
                            0
                        }
                };
                let left_ok = k == 0 || {
                    let p = rows[r][k - 1];
                    new_b0 >= nodes[p].b0 + nodes[p].box_b + own(p)
                };
                let right_ok = k + 1 == rows[r].len() || {
                    let q = rows[r][k + 1];
                    new_b0 + nodes[t].box_b + own(q) <= nodes[q].b0
                };
                if left_ok && right_ok {
                    nodes[t].b0 = new_b0;
                }
            }
        }
    }

    // ---------------------------------------------------------------- 3a. virtual columns
    let mut band_used: Vec<Vec<i32>> = vec![Vec::new(); nranks];
    let blocked = |nodes: &[N], rows: &[Vec<usize>], r: usize, col: i32, margin: i32| -> bool {
        rows[r].iter().any(|&i| {
            let n = &nodes[i];
            let ext = if n.has_loop { 4 } else { 0 };
            col >= n.b0 - margin - ext && col < n.b0 + n.box_b + margin + ext
        })
    };
    for ei in 0..edges.len() {
        if edges[ei].kind != Kind::Fwd {
            continue;
        }
        let (rs, rt) = (nodes[edges[ei].s].rank, nodes[edges[ei].t].rank);
        if rt <= rs + 1 {
            continue;
        }
        let want = edges[ei]
            .dot
            .map(|d| placement.edge_cross[d].clone())
            .unwrap_or_default();
        let mut virt: Vec<i32> = Vec::new();
        for (k, r) in (rs + 1..rt).enumerate() {
            let sampled = want
                .get(k)
                .map(|v| v.round() as i32)
                .unwrap_or_else(|| nodes[edges[ei].s].nc());
            // stay in the previous rank's column when that is close: fewer jogs
            let init = match virt.last() {
                Some(&p) if (p - sampled).abs() <= 3 => p,
                _ => sampled,
            };
            let mut chosen = init;
            'passes: for (margin, sep) in [(1, 2), (1, 1), (0, 1)] {
                for step in 0..400i32 {
                    let off = if step % 2 == 0 {
                        step / 2
                    } else {
                        -(step + 1) / 2
                    };
                    let c = init + off;
                    if blocked(&nodes, &rows, r, c, margin) {
                        continue;
                    }
                    if band_used[r].iter().any(|&u| (u - c).abs() < sep) {
                        continue;
                    }
                    chosen = c;
                    break 'passes;
                }
            }
            band_used[r].push(chosen);
            virt.push(chosen);
        }
        edges[ei].virt = virt;
    }

    // ---------------------------------------------------------------- 3b. corridors
    let g_left = nodes.iter().map(|n| n.b0).min().unwrap_or(0);
    let g_right = nodes.iter().map(|n| n.b0 + n.box_b - 1).max().unwrap_or(0);
    let mut corridors: Vec<(i32, usize, usize)> = Vec::new(); // (col, ch_lo, ch_hi)
    for ei in 0..edges.len() {
        if edges[ei].kind != Kind::Back {
            continue;
        }
        let (s, t) = (edges[ei].s, edges[ei].t);
        let (rs, rt) = (nodes[s].rank, nodes[t].rank);
        let mid = (nodes[s].nc() + nodes[t].nc()) as f64 / 2.0;
        let right = (g_right as f64 - mid) <= (mid - g_left as f64);
        let mut ext_r = i32::MIN;
        let mut ext_l = i32::MAX;
        for r in rt..=rs {
            for &i in &rows[r] {
                let ext = if nodes[i].has_loop { 4 } else { 0 };
                ext_r = ext_r.max(nodes[i].b0 + nodes[i].box_b - 1 + ext);
                ext_l = ext_l.min(nodes[i].b0 - ext);
            }
            for &u in &band_used[r] {
                ext_r = ext_r.max(u);
                ext_l = ext_l.min(u);
            }
        }
        let (ch_lo, ch_hi) = (rt, rs + 1);
        let free = |c: i32, corridors: &[(i32, usize, usize)]| -> bool {
            for r in rt..=rs {
                if band_used[r].iter().any(|&u| (u - c).abs() < 2) {
                    return false;
                }
            }
            !corridors
                .iter()
                .any(|&(cc, lo, hi)| (cc - c).abs() < 2 && lo <= ch_hi && ch_lo <= hi)
        };
        let mut c = if right { ext_r + 2 } else { ext_l - 2 };
        while !free(c, &corridors) {
            c += if right { 1 } else { -1 };
        }
        corridors.push((c, ch_lo, ch_hi));
        edges[ei].right = right;
        edges[ei].corridor = c;
    }

    // ---------------------------------------------------------------- 3c. ports
    // (cross position of the far end, rank of the far end, edge, is_from_end): ports on a side
    // are ordered by where the edge heads (departures) or comes from (arrivals) across the flow,
    // ties by the far end's rank, so that the lines leaving a side do not have to cross.
    let mut low: Vec<Vec<(i64, usize, usize, bool)>> = vec![Vec::new(); nodes.len()];
    let mut high: Vec<Vec<(i64, usize, usize, bool)>> = vec![Vec::new(); nodes.len()];
    const BIG: i64 = 1_000_000;
    for (ei, e) in edges.iter().enumerate() {
        let (s, t) = (e.s, e.t);
        match e.kind {
            Kind::Loop => {}
            Kind::Fwd => {
                let first = e.virt.first().copied().unwrap_or_else(|| nodes[t].nc());
                let last = e.virt.last().copied().unwrap_or_else(|| nodes[s].nc());
                low[s].push((first as i64, nodes[t].rank, ei, true));
                high[t].push((last as i64, nodes[s].rank, ei, false));
            }
            Kind::Back => {
                let side = if e.right { BIG } else { -BIG };
                low[s].push((side + nodes[t].nc() as i64, nodes[t].rank, ei, true));
                high[t].push((side + nodes[s].nc() as i64, nodes[s].rank, ei, false));
            }
            Kind::Same => {
                low[s].push((nodes[t].nc() as i64, nodes[t].rank, ei, true));
                low[t].push((nodes[s].nc() as i64, nodes[s].rank, ei, false));
            }
        }
    }
    for (side_lists, is_low) in [(&low, true), (&high, false)] {
        let _ = is_low;
        for (ni, list) in side_lists.iter().enumerate() {
            let mut l = list.clone();
            l.sort();
            let n = &nodes[ni];
            let cells = port_cells(n.b0, n.nb, l.len());
            for (k, (_, _, ei, from_end)) in l.iter().enumerate() {
                if *from_end {
                    edges[*ei].p_from = cells[k];
                } else {
                    edges[*ei].p_to = cells[k];
                }
            }
        }
    }

    // ---------------------------------------------------------------- 3d. lanes
    let mut segs: Vec<Vec<Seg>> = vec![Vec::new(); nranks + 1];
    for (ei, e) in edges.iter().enumerate() {
        let (rs, rt) = (nodes[e.s].rank, nodes[e.t].rank);
        match e.kind {
            Kind::Loop => {}
            Kind::Fwd => {
                let mut cols = vec![e.p_from];
                cols.extend(e.virt.iter().copied());
                cols.push(e.p_to);
                for i in 0..cols.len() - 1 {
                    if cols[i] != cols[i + 1] {
                        segs[rs + 1 + i].push(Seg {
                            edge: ei,
                            c1: cols[i],
                            a1: Anchor::Top,
                            c2: cols[i + 1],
                            a2: Anchor::Bottom,
                        });
                    }
                }
            }
            Kind::Back => {
                segs[rs + 1].push(Seg {
                    edge: ei,
                    c1: e.p_from,
                    a1: Anchor::Top,
                    c2: e.corridor,
                    a2: Anchor::Top,
                });
                segs[rt].push(Seg {
                    edge: ei,
                    c1: e.corridor,
                    a1: Anchor::Bottom,
                    c2: e.p_to,
                    a2: Anchor::Bottom,
                });
            }
            Kind::Same => {
                segs[rs + 1].push(Seg {
                    edge: ei,
                    c1: e.p_from,
                    a1: Anchor::Top,
                    c2: e.p_to,
                    a2: Anchor::Top,
                });
            }
        }
    }
    let mut lane_of: HashMap<(usize, usize), i32> = HashMap::new();
    let mut max_lane = vec![0i32; nranks + 1];
    for (j, ss) in segs.iter().enumerate() {
        let (lanes, ml) = assign_lanes(ss);
        for (k, s) in ss.iter().enumerate() {
            lane_of.insert((s.edge, j), lanes[k]);
        }
        max_lane[j] = ml;
    }

    // rank thickness along the flow
    let mut thick = vec![0i32; nranks];
    let mut node_thick = vec![0i32; nranks];
    for n in &nodes {
        node_thick[n.rank] = node_thick[n.rank].max(n.na);
        thick[n.rank] = thick[n.rank].max(n.na).max(n.pa);
    }

    // ---------------------------------------------------------------- 4. geometry + labels
    let has_labels = f.edges.iter().any(|e| e.label.is_some());
    let mut extra = 0;
    let mut attempt = 0;
    loop {
        // channel sizes
        let g: Vec<i32> = (0..=nranks)
            .map(|j| {
                if (j == 0 || j == nranks) && segs[j].is_empty() {
                    0
                } else {
                    (max_lane[j] + 2).max(GMIN) + extra
                }
            })
            .collect();
        let mut chan_top = vec![0i32; nranks + 1];
        let mut band_top = vec![0i32; nranks];
        let mut y = 0;
        for j in 0..=nranks {
            chan_top[j] = y;
            y += g[j];
            if j < nranks {
                band_top[j] = y;
                y += thick[j];
            }
        }
        for n in nodes.iter_mut() {
            let bt = band_top[n.rank];
            n.a0 = bt + (node_thick[n.rank] - n.na) / 2;
            n.pa0 = bt;
        }
        let lane_row = |j: usize, e: usize| -> i32 { chan_top[j] + lane_of[&(e, j)] };

        // abstract polylines
        let mut polys: Vec<Vec<(i32, i32)>> = Vec::new();
        let mut arrows: Vec<Dir> = Vec::new();
        for (ei, e) in edges.iter().enumerate() {
            let (s, t) = (&nodes[e.s], &nodes[e.t]);
            let fwd_dir = if lr { Dir::Right } else { Dir::Down };
            let back_dir = if lr { Dir::Left } else { Dir::Up };
            let (pts, dir): (Vec<(i32, i32)>, Dir) = match e.kind {
                Kind::Loop => (Vec::new(), Dir::Left),
                Kind::Fwd => {
                    let mut cols = vec![e.p_from];
                    cols.extend(e.virt.iter().copied());
                    cols.push(e.p_to);
                    let mut p = vec![(s.a0 + s.na, e.p_from)];
                    for i in 0..cols.len() - 1 {
                        if cols[i] != cols[i + 1] {
                            let lr_ = lane_row(s.rank + 1 + i, ei);
                            p.push((lr_, cols[i]));
                            p.push((lr_, cols[i + 1]));
                        }
                    }
                    p.push((t.a0 - 1, e.p_to));
                    (p, fwd_dir)
                }
                Kind::Back => {
                    let ls = lane_row(s.rank + 1, ei);
                    let lt = lane_row(t.rank, ei);
                    (
                        vec![
                            (s.a0 + s.na, e.p_from),
                            (ls, e.p_from),
                            (ls, e.corridor),
                            (lt, e.corridor),
                            (lt, e.p_to),
                            (t.a0 - 1, e.p_to),
                        ],
                        fwd_dir,
                    )
                }
                Kind::Same => {
                    let l = lane_row(s.rank + 1, ei);
                    (
                        vec![
                            (s.a0 + s.na, e.p_from),
                            (l, e.p_from),
                            (l, e.p_to),
                            (t.a0 + t.na, e.p_to),
                        ],
                        back_dir,
                    )
                }
            };
            polys.push(simplify(pts));
            arrows.push(dir);
        }

        // real coordinates
        let real = |a: i32, b: i32| -> (i32, i32) {
            if lr {
                (a, b)
            } else {
                (b, a)
            }
        };
        let node_rect = |n: &N| -> IRect {
            if lr {
                IRect::new(n.a0, n.b0, n.na, n.nb)
            } else {
                IRect::new(n.b0, n.a0, n.nb, n.na)
            }
        };
        let panel_rect = |n: &N| -> Option<IRect> {
            n.panel.as_ref()?;
            let pb0 = n.b0 + n.nb + PANEL_GAP;
            Some(if lr {
                IRect::new(n.pa0, pb0, n.pa, n.pb)
            } else {
                IRect::new(pb0, n.pa0, n.pb, n.pa)
            })
        };
        let mut real_polys: Vec<Vec<(i32, i32)>> = polys
            .iter()
            .map(|p| p.iter().map(|&(a, b)| real(a, b)).collect())
            .collect();
        let mut loop_dirs: Vec<Dir> = arrows.clone();
        // self loops: drawn in real coordinates beside the node
        for (ei, e) in edges.iter().enumerate() {
            if e.kind != Kind::Loop {
                continue;
            }
            let r = node_rect(&nodes[e.s]);
            let left = !lr && nodes[e.s].panel.is_some();
            let (y1, y2) = (r.y + 1, r.y + r.h - 1);
            let pts = if left {
                let x = r.x - 1;
                vec![(x, y1), (x - 3, y1), (x - 3, y2), (x, y2)]
            } else {
                let x = r.right();
                vec![(x, y1), (x + 3, y1), (x + 3, y2), (x, y2)]
            };
            real_polys[ei] = pts;
            loop_dirs[ei] = if left { Dir::Right } else { Dir::Left };
        }

        // label placement
        let mut rects: Vec<IRect> = Vec::new();
        for n in &nodes {
            rects.push(node_rect(n));
            if let Some(p) = panel_rect(n) {
                rects.push(p);
            }
        }
        let mut line_cells: HashSet<(i32, i32)> = HashSet::new();
        for p in &real_polys {
            for w in p.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (dx, dy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
                let mut c = a;
                line_cells.insert(c);
                while c != b {
                    c = (c.0 + dx, c.1 + dy);
                    line_cells.insert(c);
                }
            }
        }
        let mut label_rects: Vec<Option<(IRect, Vec<String>)>> = vec![None; f.edges.len()];
        let mut placed: Vec<IRect> = Vec::new();
        let mut all_ok = true;
        for (ei, e) in edges.iter().enumerate() {
            let Some(text) = &f.edges[e.spec].label else {
                continue;
            };
            let lines = wrap(text, opts.max_edge_label_width as usize);
            let (w, h) = (max_width(&lines) as i32, lines.len() as i32);
            let pts = &real_polys[ei];
            let mut order: Vec<usize> = Vec::new();
            let nseg = pts.len().saturating_sub(1);
            if nseg > 0 {
                order.push(0);
                if nseg > 1 {
                    order.push(nseg - 1);
                }
                order.extend(1..nseg.saturating_sub(1));
            }
            let mut cands: Vec<IRect> = Vec::new();
            for &k in &order {
                let (a, b) = (pts[k], pts[k + 1]);
                if a.0 == b.0 {
                    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                    for y in y0..=(y1 - (h - 1)).max(y0) {
                        cands.push(IRect::new(a.0 + 1, y, w, h));
                        cands.push(IRect::new(a.0 - w, y, w, h));
                    }
                } else {
                    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                    if x1 - x0 + 1 >= w {
                        for x in x0..=(x1 - w + 1) {
                            cands.push(IRect::new(x, a.1 - h, w, h));
                            cands.push(IRect::new(x, a.1 + 1, w, h));
                        }
                    }
                }
            }
            let ok = |c: &IRect, placed: &[IRect], strict: bool| -> bool {
                if rects.iter().any(|r| r.intersects(c)) {
                    return false;
                }
                if placed
                    .iter()
                    .any(|r| r.intersects(&IRect::new(c.x - 1, c.y, c.w + 2, c.h)))
                {
                    return false;
                }
                if strict {
                    for y in c.y..c.bottom() {
                        for x in c.x..c.right() {
                            if line_cells.contains(&(x, y)) {
                                return false;
                            }
                        }
                    }
                }
                true
            };
            let pick = cands
                .iter()
                .find(|c| ok(c, &placed, true))
                .copied()
                .or_else(|| {
                    all_ok = false;
                    cands.iter().find(|c| ok(c, &placed, false)).copied()
                })
                .or_else(|| {
                    all_ok = false;
                    pts.first().map(|p| IRect::new(p.0 + 1, p.1, w, h))
                });
            if let Some(r) = pick {
                placed.push(r);
                label_rects[ei] = Some((r, lines));
            }
        }
        if !all_ok && has_labels && attempt < 6 {
            attempt += 1;
            extra += if lr { 2 } else { 1 };
            continue;
        }

        // ------------------------------------------------------------ build output
        let mut objects: Vec<RawObject> = Vec::new();
        for n in &nodes {
            let r = node_rect(n);
            let mut o = RawObject::new(ObjectId::from(n.id.as_str()), RawKind::Node(n.shape), r);
            o.label = n.lines.clone();
            o.label_rect = Some(IRect::new(
                r.x + 4,
                r.y + 1,
                n.label_w,
                n.lines.len() as i32,
            ));
            o.details_count = n.details;
            o.has_code = n.has_code;
            o.expanded = n.panel.is_some();
            o.decor = box_decor(r, r.y + 1, n.details, n.has_code, true);
            objects.push(o);
        }
        for n in &nodes {
            if let (Some(p), Some(r)) = (&n.panel, panel_rect(n)) {
                objects.push(panel_object(&n.id, p, r.x, r.y, None));
            }
        }
        let mut out_edges: Vec<RawEdge> = Vec::new();
        for (ei, e) in edges.iter().enumerate() {
            let se = &f.edges[e.spec];
            let eid = ObjectId(format!("edge:{}", e.spec));
            let label_id = label_rects[ei]
                .as_ref()
                .map(|_| ObjectId(format!("edge:{}:label", e.spec)));
            if let (Some(id), Some((r, lines))) = (&label_id, &label_rects[ei]) {
                let mut o = RawObject::new(id.clone(), RawKind::EdgeLabel, *r);
                o.label = lines.clone();
                o.label_rect = Some(*r);
                o.owner = Some(eid.clone());
                objects.push(o);
            }
            out_edges.push(RawEdge {
                id: eid,
                from: ObjectId::from(nodes[e.s].id.as_str()),
                to: ObjectId::from(nodes[e.t].id.as_str()),
                points: real_polys[ei].clone(),
                arrow: loop_dirs[ei],
                style: se.style.unwrap_or(EdgeStyle::Solid),
                msg_kind: None,
                back: e.kind == Kind::Back,
                label: label_id,
                label_rect: label_rects[ei].as_ref().map(|(r, _)| *r),
            });
        }
        let nav = build_nav(&nodes, &rows, &edges);
        let mut l = super::finish(
            crate::diagram::DiagramKind::Flowchart,
            objects,
            out_edges,
            Vec::new(),
            Nav::Flow(nav),
            (0, 0),
        )?;
        // `finish` shifts into the non-negative quadrant; a flowchart's nav only holds ids.
        l.build_index();
        return Ok(l);
    }
}

/// Distinct, ordered port cells on a node side. `b0`/`nb` is the node's cross extent.
fn port_cells(b0: i32, nb: i32, n: usize) -> Vec<i32> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![b0 + (nb - 1) / 2];
    }
    let inner_len = (nb - 2).max(1);
    let (start, len) = if n as i32 <= inner_len {
        (b0 + 1.min(nb - 1), inner_len)
    } else {
        (b0, nb)
    };
    (0..n)
        .map(|i| {
            let idx = ((2 * i + 1) as i32 * len) / (2 * n as i32);
            start + idx.min(len - 1)
        })
        .collect()
}

/// Crossings made inside one channel when segment `i` runs on a lane above segment `j`:
/// `j`'s top-anchored verticals pass through `i`'s lane, and `i`'s bottom-anchored verticals
/// pass through `j`'s lane. A vertical on an end column of the other run is a corner, not a
/// crossing (shared columns are ordered by [`must_above`]).
fn cross_if_above(i: &Seg, j: &Seg) -> i64 {
    let inside = |c: i32, s: &Seg| c > s.lo() && c < s.hi();
    let mut n = 0;
    for (c, a) in j.ends() {
        if a == Anchor::Top && inside(c, i) {
            n += 1;
        }
    }
    for (c, a) in i.ends() {
        if a == Anchor::Bottom && inside(c, j) {
            n += 1;
        }
    }
    n
}

/// `i` must lie above `j` when a Top-anchored end of `i` shares a column with a Bottom-anchored
/// end of `j` (their verticals would otherwise overlap).
fn must_above(i: &Seg, j: &Seg) -> bool {
    i.ends().iter().any(|&(ci, ai)| {
        ai == Anchor::Top
            && j.ends()
                .iter()
                .any(|&(cj, aj)| aj == Anchor::Bottom && cj == ci)
    })
}

/// Cost of an ordering violating a hard constraint (dwarfs any crossing count).
const VIOLATION: i64 = 10_000;

/// Lane assignment for one channel (see module docs). Returns the lane (>= 1) of every segment
/// and the highest lane used.
///
/// Segments are first put in a top-to-bottom order that minimises the crossings between their
/// horizontal runs and each other's verticals ([`cross_if_above`]) while honouring the hard
/// "shares a column" constraints ([`must_above`]): greedy construction, then insertion moves
/// until no move helps. Lanes are then handed out in that order; runs that do not overlap may
/// share a lane.
fn assign_lanes(segs: &[Seg]) -> (Vec<i32>, i32) {
    let n = segs.len();
    if n == 0 {
        return (Vec::new(), 0);
    }
    // cost[i][j]: price of putting i above j
    let mut cost = vec![vec![0i64; n]; n];
    for i in 0..n {
        for j in 0..n {
            if i != j {
                cost[i][j] = cross_if_above(&segs[i], &segs[j])
                    + if must_above(&segs[j], &segs[i]) { VIOLATION } else { 0 };
            }
        }
    }
    // greedy: repeatedly take the segment that is cheapest to put above everything left
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut left: Vec<usize> = (0..n).collect();
    while !left.is_empty() {
        let (pos, _) = left
            .iter()
            .enumerate()
            .min_by_key(|&(_, &k)| {
                let net: i64 = left
                    .iter()
                    .filter(|&&o| o != k)
                    .map(|&o| cost[k][o] - cost[o][k])
                    .sum();
                (net, segs[k].lo(), k)
            })
            .unwrap();
        order.push(left.remove(pos));
    }
    // insertion moves
    for _ in 0..(2 * n + 4) {
        let mut improved = false;
        for k in 0..n {
            let cur = order.iter().position(|&x| x == k).unwrap();
            let rest: Vec<usize> = order.iter().copied().filter(|&x| x != k).collect();
            // price of k at insertion point p (0..=rest.len()): everything before is above k
            let mut after: i64 = rest.iter().map(|&o| cost[k][o]).sum();
            let mut before = 0i64;
            let mut best = (after, 0usize);
            for (p, &o) in rest.iter().enumerate() {
                before += cost[o][k];
                after -= cost[k][o];
                if before + after < best.0 {
                    best = (before + after, p + 1);
                }
            }
            let cur_price = {
                let before: i64 = rest[..cur].iter().map(|&o| cost[o][k]).sum();
                let after: i64 = rest[cur..].iter().map(|&o| cost[k][o]).sum();
                before + after
            };
            if best.0 < cur_price {
                let mut next = rest;
                next.insert(best.1, k);
                order = next;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    // lanes in that order
    let mut lane = vec![0i32; n];
    let mut placed: Vec<usize> = Vec::with_capacity(n);
    let mut maxl = 0;
    for &k in &order {
        let mut l = 1;
        for &o in &placed {
            let overlap = segs[o].lo() <= segs[k].hi() + 1 && segs[k].lo() <= segs[o].hi() + 1;
            if overlap || must_above(&segs[o], &segs[k]) {
                l = l.max(lane[o] + 1);
            }
        }
        lane[k] = l;
        placed.push(k);
        maxl = maxl.max(l);
    }
    (lane, maxl)
}

/// Drop repeated points and merge collinear runs.
fn simplify(pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let same_row = a.0 == b.0 && b.0 == p.0;
            let same_col = a.1 == b.1 && b.1 == p.1;
            if same_row || same_col {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    out
}

fn build_nav(nodes: &[N], rows: &[Vec<usize>], edges: &[E]) -> FlowNav {
    let id = |i: usize| ObjectId::from(nodes[i].id.as_str());
    let mut nav = FlowNav::default();
    for r in rows {
        // `rows` is already sorted by cross position
        nav.ranks.push(r.iter().map(|&i| id(i)).collect());
    }
    for (ri, r) in rows.iter().enumerate() {
        for &i in r {
            nav.rank.insert(id(i), ri);
            nav.order.push(id(i));
        }
    }
    let pos = |i: usize| (nodes[i].nc(), i);
    let mut ch: Vec<Vec<(bool, (i32, usize), usize)>> = vec![Vec::new(); nodes.len()];
    let mut pa: Vec<Vec<(bool, (i32, usize), usize)>> = vec![Vec::new(); nodes.len()];
    for e in edges {
        if e.kind == Kind::Loop {
            continue;
        }
        let back = e.kind == Kind::Back;
        ch[e.s].push((back, pos(e.t), e.t));
        pa[e.t].push((back, pos(e.s), e.s));
    }
    let order_ids = |mut v: Vec<(bool, (i32, usize), usize)>| -> Vec<ObjectId> {
        v.sort();
        let mut seen = HashSet::new();
        v.into_iter()
            .filter(|x| seen.insert(x.2))
            .map(|x| id(x.2))
            .collect()
    };
    for i in 0..nodes.len() {
        nav.children.insert(id(i), order_ids(ch[i].clone()));
        nav.parents.insert(id(i), order_ids(pa[i].clone()));
    }
    for r in rows {
        for &i in r {
            nav.siblings.insert(
                id(i),
                r.iter().filter(|&&j| j != i).map(|&j| id(j)).collect(),
            );
        }
    }
    // branch siblings: other children of any parent
    for i in 0..nodes.len() {
        let mut v: Vec<(i32, usize)> = Vec::new();
        for (back, _, p) in &pa[i] {
            if *back {
                continue;
            }
            for (b2, pp, c) in &ch[*p] {
                if !*b2 && *c != i {
                    v.push(*pp);
                }
            }
        }
        v.sort();
        v.dedup();
        nav.branch_siblings
            .insert(id(i), v.into_iter().map(|(_, j)| id(j)).collect());
    }
    for r in rows {
        for &i in r {
            if pa[i].iter().all(|(back, _, _)| *back) {
                nav.roots.push(id(i));
            }
        }
    }
    if nav.roots.is_empty() {
        if let Some(f) = nav.order.first() {
            nav.roots.push(f.clone());
        }
    }
    nav
}
