//! Node placement for flowcharts: ranks and cross-axis positions, from Graphviz
//! (`dot -Tplain`) or from a small built-in layered fallback.
//!
//! Only *placement* comes from here. Edge routing is done in `flow.rs`; Graphviz splines are
//! only sampled to find where long edges pass through intermediate ranks.

// Index-based graph code: indexing by rank/node/edge number is clearer than iterators here.
#![allow(clippy::needless_range_loop, clippy::type_complexity)]

use std::io::Write;
use std::process::{Command, Stdio};

use super::{Engine, LayoutError, LayoutOptions, CELL_H_IN, CELL_W_IN};

/// Result of placement.
pub(super) struct Placement {
    /// Rank (layer) per node.
    pub rank: Vec<usize>,
    /// Cross-axis centre per node, in cells (x for TB, y for LR). May be negative.
    pub cross: Vec<f64>,
    /// For each edge that spans 2+ ranks forwards: the cross coordinate at every intermediate
    /// rank (`rank[s]+1 ..= rank[t]-1`). Empty for all other edges.
    pub edge_cross: Vec<Vec<f64>>,
}

/// `sizes[i]` is the real (w, h) in cells of node `i`'s whole box (node plus any panel).
pub(super) fn place(
    sizes: &[(i32, i32)],
    edges: &[(usize, usize)],
    lr: bool,
    cross_gap: i32,
    opts: &LayoutOptions,
) -> Result<Placement, LayoutError> {
    match opts.engine {
        Engine::Dot => place_dot(sizes, edges, lr, cross_gap, opts),
        Engine::Naive => Ok(place_naive(sizes, edges, lr, cross_gap)),
        Engine::Auto => place_dot(sizes, edges, lr, cross_gap, opts)
            .or_else(|_| Ok(place_naive(sizes, edges, lr, cross_gap))),
    }
}

// ------------------------------------------------------------------------------- graphviz

/// Build the DOT source. Node `i` is called `n{i}`. Sizes are cells converted to inches with
/// [`CELL_W_IN`] x [`CELL_H_IN`]; `fixedsize` stops Graphviz from adding its own margins.
pub(super) fn build_dot(
    sizes: &[(i32, i32)],
    edges: &[(usize, usize)],
    lr: bool,
    cross_gap: i32,
) -> String {
    let nodesep = if lr {
        cross_gap as f64 * CELL_H_IN
    } else {
        cross_gap as f64 * CELL_W_IN
    };
    let mut s = String::new();
    s.push_str("digraph G {\n");
    s.push_str(&format!(
        "  rankdir={};\n  nodesep={:.3};\n  ranksep=0.5;\n  node [shape=box, fixedsize=true, label=\"\", margin=0];\n",
        if lr { "LR" } else { "TB" },
        nodesep
    ));
    for (i, (w, h)) in sizes.iter().enumerate() {
        s.push_str(&format!(
            "  n{i} [width={:.3}, height={:.3}];\n",
            *w as f64 * CELL_W_IN,
            *h as f64 * CELL_H_IN
        ));
    }
    for (a, b) in edges {
        s.push_str(&format!("  n{a} -> n{b};\n"));
    }
    s.push_str("}\n");
    s
}

fn run_dot(dot_path: &str, src: &str) -> Result<String, LayoutError> {
    let mut child = Command::new(dot_path)
        .arg("-Tplain")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| LayoutError::DotMissing(dot_path.to_string(), e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // dot reads its whole input before writing; the input is small.
        let _ = stdin.write_all(src.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| LayoutError::DotMissing(dot_path.to_string(), e.to_string()))?;
    if !out.status.success() {
        return Err(LayoutError::DotFailed {
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    String::from_utf8(out.stdout).map_err(|e| LayoutError::DotParse(e.to_string()))
}

struct Plain {
    height: f64,
    /// (x, y, w, h) inches per node index.
    nodes: Vec<Option<(f64, f64)>>,
    /// control points per edge in output order, keyed by (tail, head, ordinal).
    edges: Vec<(usize, usize, Vec<(f64, f64)>)>,
}

fn parse_plain(text: &str, n: usize) -> Result<Plain, LayoutError> {
    let bad = |m: &str| LayoutError::DotParse(m.to_string());
    let idx = |name: &str| -> Result<usize, LayoutError> {
        name.strip_prefix('n')
            .and_then(|r| r.parse::<usize>().ok())
            .filter(|&i| i < n)
            .ok_or_else(|| bad(&format!("unexpected node name `{name}`")))
    };
    let num = |s: Option<&str>| -> Result<f64, LayoutError> {
        s.and_then(|v| v.parse::<f64>().ok())
            .ok_or_else(|| bad("expected a number"))
    };
    let mut height = None;
    let mut nodes = vec![None; n];
    let mut edges = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("graph") => {
                let _scale = num(it.next())?;
                let _w = num(it.next())?;
                height = Some(num(it.next())?);
            }
            Some("node") => {
                let name = it.next().ok_or_else(|| bad("node without name"))?;
                let x = num(it.next())?;
                let y = num(it.next())?;
                nodes[idx(name)?] = Some((x, y));
            }
            Some("edge") => {
                let t = idx(it.next().ok_or_else(|| bad("edge without tail"))?)?;
                let h = idx(it.next().ok_or_else(|| bad("edge without head"))?)?;
                let cnt = num(it.next())? as usize;
                let mut pts = Vec::with_capacity(cnt);
                for _ in 0..cnt {
                    let x = num(it.next())?;
                    let y = num(it.next())?;
                    pts.push((x, y));
                }
                edges.push((t, h, pts));
            }
            _ => {}
        }
    }
    let height = height.ok_or_else(|| bad("missing `graph` line"))?;
    if nodes.iter().any(|n| n.is_none()) {
        return Err(bad("missing node line"));
    }
    Ok(Plain {
        height,
        nodes,
        edges,
    })
}

fn place_dot(
    sizes: &[(i32, i32)],
    edges: &[(usize, usize)],
    lr: bool,
    cross_gap: i32,
    opts: &LayoutOptions,
) -> Result<Placement, LayoutError> {
    let src = build_dot(sizes, edges, lr, cross_gap);
    let out = run_dot(&opts.dot_path, &src)?;
    let plain = parse_plain(&out, sizes.len())?;
    // unified (a ascending with rank, b in cells)
    let conv = |x: f64, y: f64| -> (f64, f64) {
        if lr {
            (x, (plain.height - y) / CELL_H_IN)
        } else {
            (plain.height - y, x / CELL_W_IN)
        }
    };
    let pos: Vec<(f64, f64)> = plain
        .nodes
        .iter()
        .map(|p| {
            let (x, y) = p.unwrap();
            conv(x, y)
        })
        .collect();
    let rank = cluster_ranks(&pos.iter().map(|p| p.0).collect::<Vec<_>>());
    let nranks = rank.iter().copied().max().map_or(0, |m| m + 1);
    let mut centre = vec![0.0f64; nranks];
    let mut cnt = vec![0usize; nranks];
    for (i, r) in rank.iter().enumerate() {
        centre[*r] += pos[i].0;
        cnt[*r] += 1;
    }
    for r in 0..nranks {
        centre[r] /= cnt[r].max(1) as f64;
    }
    // match dot's edges to ours: dot may emit edges in input order; key by (tail, head) queue
    let mut edge_pts: Vec<Option<Vec<(f64, f64)>>> = vec![None; edges.len()];
    let mut used = vec![false; plain.edges.len()];
    for (ei, &(s, t)) in edges.iter().enumerate() {
        // `dot` plain keeps the original tail/head orientation.
        if let Some(k) = plain
            .edges
            .iter()
            .enumerate()
            .position(|(k, (a, b, _))| !used[k] && *a == s && *b == t)
        {
            used[k] = true;
            edge_pts[ei] = Some(plain.edges[k].2.iter().map(|&(x, y)| conv(x, y)).collect());
        }
    }
    let mut edge_cross = Vec::with_capacity(edges.len());
    for (ei, &(s, t)) in edges.iter().enumerate() {
        let (rs, rt) = (rank[s], rank[t]);
        let mut v = Vec::new();
        if rt > rs + 1 {
            for r in rs + 1..rt {
                let b = edge_pts[ei]
                    .as_ref()
                    .and_then(|p| sample(p, centre[r]))
                    .unwrap_or_else(|| {
                        let f = (r - rs) as f64 / (rt - rs) as f64;
                        pos[s].1 + (pos[t].1 - pos[s].1) * f
                    });
                v.push(b);
            }
        }
        edge_cross.push(v);
    }
    Ok(Placement {
        rank,
        cross: pos.iter().map(|p| p.1).collect(),
        edge_cross,
    })
}

/// Group ascending-coordinate values into ranks (values within a small tolerance are equal).
fn cluster_ranks(a: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..a.len()).collect();
    order.sort_by(|&i, &j| a[i].partial_cmp(&a[j]).unwrap().then(i.cmp(&j)));
    let mut rank = vec![0usize; a.len()];
    let mut r = 0usize;
    let mut prev: Option<f64> = None;
    for &i in &order {
        if let Some(p) = prev {
            if a[i] - p > 0.03 {
                r += 1;
            }
        }
        rank[i] = r;
        prev = Some(a[i]);
    }
    rank
}

/// Piecewise-linear value of `b` where the polyline crosses `a`.
fn sample(pts: &[(f64, f64)], a: f64) -> Option<f64> {
    if pts.is_empty() {
        return None;
    }
    for w in pts.windows(2) {
        let ((a0, b0), (a1, b1)) = (w[0], w[1]);
        if (a0 - a) * (a1 - a) <= 0.0 {
            if (a1 - a0).abs() < 1e-9 {
                return Some(b0);
            }
            return Some(b0 + (b1 - b0) * (a - a0) / (a1 - a0));
        }
    }
    pts.iter()
        .min_by(|p, q| (p.0 - a).abs().partial_cmp(&(q.0 - a).abs()).unwrap())
        .map(|p| p.1)
}

// ------------------------------------------------------------------------------- naive

fn place_naive(
    sizes: &[(i32, i32)],
    edges: &[(usize, usize)],
    lr: bool,
    cross_gap: i32,
) -> Placement {
    let n = sizes.len();
    let cross_size = |i: usize| -> f64 {
        if lr {
            sizes[i].1 as f64
        } else {
            sizes[i].0 as f64
        }
    };
    // 1. break cycles with a DFS, then longest-path ranks
    let mut adj = vec![Vec::new(); n];
    for (ei, &(s, t)) in edges.iter().enumerate() {
        adj[s].push((t, ei));
    }
    let mut state = vec![0u8; n];
    let mut back = vec![false; edges.len()];
    fn dfs(u: usize, adj: &[Vec<(usize, usize)>], state: &mut [u8], back: &mut [bool]) {
        state[u] = 1;
        for &(v, ei) in &adj[u] {
            if state[v] == 1 {
                back[ei] = true;
            } else if state[v] == 0 {
                dfs(v, adj, state, back);
            }
        }
        state[u] = 2;
    }
    for u in 0..n {
        if state[u] == 0 {
            dfs(u, &adj, &mut state, &mut back);
        }
    }
    let mut indeg = vec![0usize; n];
    for (ei, &(_, t)) in edges.iter().enumerate() {
        if !back[ei] {
            indeg[t] += 1;
        }
    }
    let mut rank = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let u = queue[qi];
        qi += 1;
        for &(v, ei) in &adj[u] {
            if back[ei] {
                continue;
            }
            rank[v] = rank[v].max(rank[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    let nranks = rank.iter().copied().max().map_or(0, |m| m + 1);
    // 2. order within ranks (barycentre sweeps)
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); nranks];
    for i in 0..n {
        rows[rank[i]].push(i);
    }
    let neighbours = |i: usize| -> Vec<usize> {
        edges
            .iter()
            .filter_map(|&(s, t)| {
                if s == i {
                    Some(t)
                } else if t == i {
                    Some(s)
                } else {
                    None
                }
            })
            .collect()
    };
    let nb: Vec<Vec<usize>> = (0..n).map(neighbours).collect();
    let mut order_pos = vec![0.0f64; n];
    for row in &rows {
        for (k, &i) in row.iter().enumerate() {
            order_pos[i] = k as f64;
        }
    }
    for sweep in 0..6 {
        let range: Vec<usize> = if sweep % 2 == 0 {
            (1..nranks).collect()
        } else {
            (0..nranks.saturating_sub(1)).rev().collect()
        };
        for r in range {
            let adj_rank = if sweep % 2 == 0 { r - 1 } else { r + 1 };
            let mut keyed: Vec<(f64, usize)> = rows[r]
                .iter()
                .map(|&i| {
                    let ns: Vec<f64> = nb[i]
                        .iter()
                        .filter(|&&j| rank[j] == adj_rank)
                        .map(|&j| order_pos[j])
                        .collect();
                    let key = if ns.is_empty() {
                        order_pos[i]
                    } else {
                        ns.iter().sum::<f64>() / ns.len() as f64
                    };
                    (key, i)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
            rows[r] = keyed.iter().map(|k| k.1).collect();
            for (k, &i) in rows[r].iter().enumerate() {
                order_pos[i] = k as f64;
            }
        }
    }
    // 3. coordinates: pack, then pull towards neighbours while keeping the order
    let mut cross = vec![0.0f64; n];
    for row in &rows {
        let mut x = 0.0;
        for &i in row {
            cross[i] = x + cross_size(i) / 2.0;
            x += cross_size(i) + cross_gap as f64;
        }
    }
    for pass in 0..6 {
        let ranks: Vec<usize> = if pass % 2 == 0 {
            (0..nranks).collect()
        } else {
            (0..nranks).rev().collect()
        };
        for r in ranks {
            let mut x_prev_right = f64::NEG_INFINITY;
            for &i in &rows[r] {
                let ns: Vec<f64> = nb[i].iter().map(|&j| cross[j]).collect();
                let want = if ns.is_empty() {
                    cross[i]
                } else {
                    ns.iter().sum::<f64>() / ns.len() as f64
                };
                let min_c = x_prev_right + cross_gap as f64 + cross_size(i) / 2.0;
                let c = want.max(min_c);
                cross[i] = c;
                x_prev_right = c + cross_size(i) / 2.0;
            }
        }
    }
    let mut edge_cross = Vec::with_capacity(edges.len());
    for &(s, t) in edges {
        let (rs, rt) = (rank[s], rank[t]);
        let mut v = Vec::new();
        if rt > rs + 1 {
            for r in rs + 1..rt {
                let f = (r - rs) as f64 / (rt - rs) as f64;
                v.push(cross[s] + (cross[t] - cross[s]) * f);
            }
        }
        edge_cross.push(v);
    }
    Placement {
        rank,
        cross,
        edge_cross,
    }
}
