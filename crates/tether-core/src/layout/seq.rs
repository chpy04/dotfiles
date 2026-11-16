//! Sequence diagram layout: a fixed grid. One column per participant, one row band per
//! message, notes over their participants, group frames around their rows.
//!
//! Row bands (top to bottom):
//! * participant boxes, then one blank row;
//! * message: label rows, then the arrow row, then a blank row;
//! * note: box, then a blank row;
//! * group: top border (with the `alt [label]` header), optional branch rows, the items, bottom
//!   border, a blank row;
//! * a details panel sits **directly under** its owner's last row and adds exactly its own
//!   height to every later row.

use std::collections::{HashMap, HashSet};

use crate::diagram::{DiagramKind, EdgeStyle, GroupKind, Item, MessageKind, Sequence};

use super::common::{adorn_width, badge_len, box_decor, build_panel, panel_object};
use super::text::{max_width, str_width, wrap};
use super::{
    Dir, IRect, Layout, LayoutError, LayoutOptions, Nav, ObjectId, RawBranch, RawDecor, RawEdge,
    RawKind, RawLifeline, RawObject, SeqNav,
};

/// Width of a self-message loop in cells.
const LOOP_W: i32 = 4;

struct Ctx<'a> {
    cx: Vec<i32>,
    pidx: HashMap<&'a str, usize>,
    cursor: i32,
    groups: Vec<RawObject>,
    items: Vec<RawObject>,
    panels: Vec<RawObject>,
    edges: Vec<RawEdge>,
    nav: SeqNav,
    gcount: usize,
    expanded: &'a HashSet<ObjectId>,
    opts: &'a LayoutOptions,
    pids: &'a [String],
}

#[derive(Clone, Copy)]
struct Span {
    lo: usize,
    hi: usize,
    need: i32,
}

struct Extent {
    lo: i32,
    hi: i32,
}

impl Extent {
    fn new() -> Self {
        Extent {
            lo: i32::MAX,
            hi: i32::MIN,
        }
    }
    fn add(&mut self, lo: i32, hi: i32) {
        self.lo = self.lo.min(lo);
        self.hi = self.hi.max(hi);
    }
    fn merge(&mut self, o: &Extent) {
        if o.lo <= o.hi {
            self.add(o.lo, o.hi);
        }
    }
}

fn kind_word(k: GroupKind) -> &'static str {
    match k {
        GroupKind::Alt => "alt",
        GroupKind::Opt => "opt",
        GroupKind::Loop => "loop",
        GroupKind::Par => "par",
    }
}

pub(super) fn layout_sequence(
    seq: &Sequence,
    expanded: &HashSet<ObjectId>,
    opts: &LayoutOptions,
) -> Result<Layout, LayoutError> {
    let mut seq = seq.clone();
    seq.assign_ids();

    // ---- ids
    let mut seen: HashSet<String> = HashSet::new();
    for p in &seq.participants {
        if !seen.insert(p.id.clone()) {
            return Err(LayoutError::DuplicateId(p.id.clone()));
        }
    }
    let mut dup = None;
    seq.walk(&mut |it| {
        let id = match it {
            Item::Message(m) => &m.id,
            Item::Note(n) => &n.id,
            Item::Group(_) => return,
        };
        if !seen.insert(id.clone()) && dup.is_none() {
            dup = Some(id.clone());
        }
    });
    if let Some(d) = dup {
        return Err(LayoutError::DuplicateId(d));
    }
    let pids: Vec<String> = seq.participants.iter().map(|p| p.id.clone()).collect();
    let pidx: HashMap<&str, usize> = pids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let np = pids.len();
    let find = |id: &str| -> Result<usize, LayoutError> {
        pidx.get(id)
            .copied()
            .ok_or_else(|| LayoutError::UnknownId(id.to_string()))
    };

    // ---- participant boxes
    let mut plines: Vec<Vec<String>> = Vec::new();
    let mut pw: Vec<i32> = Vec::new();
    for p in &seq.participants {
        let lines = wrap(&p.label, opts.max_label_width as usize);
        pw.push((max_width(&lines) as i32 + 6).max(10));
        plines.push(lines);
    }
    let ph = plines.iter().map(|l| l.len()).max().unwrap_or(1) as i32 + 2;

    // ---- column spacing
    let mut d: Vec<i32> = (0..np.saturating_sub(1))
        .map(|i| pw[i] / 2 + pw[i + 1] / 2 + 3)
        .collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut err: Option<LayoutError> = None;
    let max_msg = opts.max_message_label_width as usize;
    let note_wrap = opts.max_label_width as usize;
    seq.walk(&mut |it| match it {
        Item::Message(m) => {
            let (a, b) = match (find(&m.from), find(&m.to)) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => {
                    err.get_or_insert(e);
                    return;
                }
            };
            let lw = max_width(&wrap(&m.label, max_msg)) as i32
                + adorn_width(m.details.len(), m.code.is_some());
            if a == b {
                if a + 1 < np {
                    spans.push(Span {
                        lo: a,
                        hi: a + 1,
                        need: lw + LOOP_W + 4,
                    });
                }
            } else {
                spans.push(Span {
                    lo: a.min(b),
                    hi: a.max(b),
                    need: lw + 4,
                });
            }
        }
        Item::Note(n) => {
            let idx: Vec<usize> = n.over.iter().filter_map(|o| find(o).ok()).collect();
            if idx.len() != n.over.len() {
                for o in &n.over {
                    if let Err(e) = find(o) {
                        err.get_or_insert(e);
                    }
                }
                return;
            }
            let (lo, hi) = (*idx.iter().min().unwrap(), *idx.iter().max().unwrap());
            let w = max_width(&wrap(&n.text, note_wrap)) as i32
                + adorn_width(n.details.len(), false)
                + 4;
            if lo == hi {
                let half = (w + 1) / 2 + 1;
                if lo > 0 {
                    spans.push(Span {
                        lo: lo - 1,
                        hi: lo,
                        need: half,
                    });
                }
                if lo + 1 < np {
                    spans.push(Span {
                        lo,
                        hi: lo + 1,
                        need: half,
                    });
                }
            } else {
                spans.push(Span {
                    lo,
                    hi,
                    need: w - 4,
                });
            }
        }
        Item::Group(_) => {}
    });
    if let Some(e) = err {
        return Err(e);
    }
    spans.sort_by_key(|s| (s.hi - s.lo, s.lo));
    for s in &spans {
        let cur: i32 = d[s.lo..s.hi].iter().sum();
        if cur < s.need {
            let n = (s.hi - s.lo) as i32;
            let deficit = s.need - cur;
            let each = deficit / n;
            let rem = deficit % n;
            for (k, g) in d[s.lo..s.hi].iter_mut().enumerate() {
                *g += each + if (k as i32) >= n - rem { 1 } else { 0 };
            }
        }
    }
    let mut cx = vec![pw[0] / 2];
    for g in &d {
        cx.push(cx.last().unwrap() + g);
    }

    // ---- rows
    let mut ctx = Ctx {
        cx: cx.clone(),
        pidx,
        cursor: ph + 1,
        groups: Vec::new(),
        items: Vec::new(),
        panels: Vec::new(),
        edges: Vec::new(),
        nav: SeqNav::default(),
        gcount: 0,
        expanded,
        opts,
        pids: &pids,
    };
    place_items(&mut ctx, &seq.items, 0, None)?;
    let end = ctx.cursor;

    // ---- participants
    let mut objects: Vec<RawObject> = Vec::new();
    let mut groups = std::mem::take(&mut ctx.groups);
    groups.sort_by_key(|g| {
        g.id.as_str()
            .trim_start_matches("group:")
            .parse::<usize>()
            .unwrap_or(0)
    });
    objects.append(&mut groups);
    let mut participants = Vec::new();
    for (i, p) in seq.participants.iter().enumerate() {
        let x = cx[i] - (pw[i] - 1) / 2;
        let rect = IRect::new(x, 0, pw[i], ph);
        let mut o = RawObject::new(
            ObjectId::from(p.id.as_str()),
            RawKind::Participant(p.kind),
            rect,
        );
        o.label = plines[i].clone();
        o.label_rect = Some(IRect::new(
            x + 4,
            1,
            max_width(&plines[i]) as i32,
            plines[i].len() as i32,
        ));
        o.decor = box_decor(rect, 1, 0, false, true);
        participants.push(o);
        ctx.nav.participants.push(ObjectId::from(p.id.as_str()));
    }
    objects.append(&mut participants);
    objects.append(&mut ctx.items);
    let panels_start = objects.len();
    objects.append(&mut ctx.panels);

    // ---- lifelines
    let mut lifelines = Vec::new();
    for (i, p) in seq.participants.iter().enumerate() {
        let x = cx[i];
        let mut breaks: Vec<(i32, i32)> = Vec::new();
        for o in objects.iter() {
            let covers = matches!(o.kind, RawKind::Note(_) | RawKind::Details);
            if covers && o.rect.x <= x && x < o.rect.right() {
                breaks.push((o.rect.y, o.rect.bottom() - 1));
            }
        }
        breaks.sort();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for b in breaks {
            match merged.last_mut() {
                Some(l) if b.0 <= l.1 + 1 => l.1 = l.1.max(b.1),
                _ => merged.push(b),
            }
        }
        lifelines.push(RawLifeline {
            participant: ObjectId::from(p.id.as_str()),
            x,
            y0: ph,
            y1: (end - 1).max(ph),
            breaks: merged,
        });
    }
    let _ = panels_start;
    let nav = ctx.nav;
    super::finish(
        DiagramKind::Sequence,
        objects,
        ctx.edges,
        lifelines,
        Nav::Seq(nav),
        (0, end),
    )
}

/// Left-to-right placement of the message label unit (label + code marker + badge).
fn adorn_decor(label_x: i32, y: i32, lw: i32, details: usize, has_code: bool) -> RawDecor {
    let mut d = RawDecor::default();
    let mut x = label_x + lw;
    if has_code {
        d.code = Some((x + 1, y));
        x += 2;
    }
    if details > 0 {
        d.badge = Some((x + 1, y));
    }
    let _ = badge_len;
    d
}

fn place_items(
    ctx: &mut Ctx<'_>,
    items: &[Item],
    depth: u16,
    group: Option<ObjectId>,
) -> Result<Extent, LayoutError> {
    let mut ext = Extent::new();
    for it in items {
        match it {
            Item::Message(m) => {
                let a = ctx.cx[*ctx
                    .pidx
                    .get(m.from.as_str())
                    .ok_or_else(|| LayoutError::UnknownId(m.from.clone()))?];
                let b = ctx.cx[*ctx
                    .pidx
                    .get(m.to.as_str())
                    .ok_or_else(|| LayoutError::UnknownId(m.to.clone()))?];
                let lines = wrap(&m.label, ctx.opts.max_message_label_width as usize);
                let lw = max_width(&lines) as i32;
                let nl = lines.len() as i32;
                let adorn = adorn_width(m.details.len(), m.code.is_some());
                let y = ctx.cursor;
                let id = ObjectId::from(m.id.as_str());
                let kind = m.kind();
                let (rect, label_rect, pts, arrow, left_x) = if a == b {
                    let lx = a + LOOP_W + 2;
                    (
                        IRect::new(a + 1, y, LOOP_W + 1 + lw + adorn, nl + 1),
                        IRect::new(lx, y, lw, nl),
                        vec![
                            (a + 1, y),
                            (a + LOOP_W, y),
                            (a + LOOP_W, y + nl),
                            (a + 1, y + nl),
                        ],
                        Dir::Left,
                        a.min(b),
                    )
                } else {
                    let (lo, hi) = (a.min(b), a.max(b));
                    let interior = hi - lo - 1;
                    let lx = lo + 1 + ((interior - (lw + adorn)) / 2).max(0);
                    let (pts, dir) = if a < b {
                        (vec![(a + 1, y + nl), (b - 1, y + nl)], Dir::Right)
                    } else {
                        (vec![(a - 1, y + nl), (b + 1, y + nl)], Dir::Left)
                    };
                    (
                        IRect::new(lo + 1, y, interior, nl + 1),
                        IRect::new(lx, y, lw, nl),
                        pts,
                        dir,
                        lo,
                    )
                };
                let mut o = RawObject::new(
                    id.clone(),
                    RawKind::Message(
                        kind,
                        ObjectId::from(m.from.as_str()),
                        ObjectId::from(m.to.as_str()),
                    ),
                    rect,
                );
                o.label = lines;
                o.label_rect = Some(label_rect);
                o.details_count = m.details.len();
                o.has_code = m.code.is_some();
                o.decor = adorn_decor(label_rect.x, y, lw, m.details.len(), m.code.is_some());
                o.group = group.clone();
                ctx.edges.push(RawEdge {
                    id: id.clone(),
                    from: ObjectId::from(m.from.as_str()),
                    to: ObjectId::from(m.to.as_str()),
                    points: pts,
                    arrow,
                    style: if kind == MessageKind::Return {
                        EdgeStyle::Dashed
                    } else {
                        EdgeStyle::Solid
                    },
                    msg_kind: Some(kind),
                    back: false,
                    label: None,
                    label_rect: None,
                });
                if a == b {
                    ext.add(a, rect.right());
                } else {
                    ext.add(a.min(b), a.max(b) + 1);
                }
                let _ = left_x;
                ctx.cursor = y + nl + 1;
                let panel = if ctx.expanded.contains(m.id.as_str()) {
                    build_panel(&m.details, m.code.as_ref(), ctx.opts.max_panel_width)
                } else {
                    None
                };
                if let Some(p) = panel {
                    let px = a.min(b) - 1;
                    let po = panel_object(&m.id, &p, px, ctx.cursor, group.clone());
                    ext.add(px, px + p.w);
                    ctx.cursor += p.h;
                    o.expanded = true;
                    ctx.panels.push(po);
                }
                ctx.cursor += 1;
                ctx.nav.messages.push(id.clone());
                ctx.nav.items.push(id);
                ctx.items.push(o);
            }
            Item::Note(n) => {
                let lines = wrap(&n.text, ctx.opts.max_label_width as usize);
                let tw = max_width(&lines) as i32;
                let adorn = adorn_width(n.details.len(), false);
                let w = (tw + adorn + 4).max(8);
                let idx: Vec<i32> = n
                    .over
                    .iter()
                    .map(|o| {
                        ctx.pidx
                            .get(o.as_str())
                            .map(|&i| ctx.cx[i])
                            .ok_or_else(|| LayoutError::UnknownId(o.clone()))
                    })
                    .collect::<Result<_, _>>()?;
                let (lo, hi) = (*idx.iter().min().unwrap(), *idx.iter().max().unwrap());
                let (x, w) = if lo == hi {
                    (lo - (w - 1) / 2, w)
                } else {
                    (lo - 2, (hi - lo + 4).max(w))
                };
                let y = ctx.cursor;
                let h = lines.len() as i32 + 2;
                let rect = IRect::new(x, y, w, h);
                let id = ObjectId::from(n.id.as_str());
                let over: Vec<ObjectId> =
                    n.over.iter().map(|s| ObjectId::from(s.as_str())).collect();
                let mut o = RawObject::new(id.clone(), RawKind::Note(over), rect);
                o.label = lines.clone();
                o.label_rect = Some(IRect::new(x + 2, y + 1, tw, lines.len() as i32));
                o.details_count = n.details.len();
                o.decor = adorn_decor(x + 2, y + 1, tw, n.details.len(), false);
                o.group = group.clone();
                ext.add(x, x + w);
                ctx.cursor = y + h;
                let panel = if ctx.expanded.contains(n.id.as_str()) {
                    build_panel(&n.details, None, ctx.opts.max_panel_width)
                } else {
                    None
                };
                if let Some(p) = panel {
                    let po = panel_object(&n.id, &p, x, ctx.cursor, group.clone());
                    ext.add(x, x + p.w);
                    ctx.cursor += p.h;
                    o.expanded = true;
                    ctx.panels.push(po);
                }
                ctx.cursor += 1;
                ctx.nav.items.push(id);
                ctx.items.push(o);
            }
            Item::Group(g) => {
                ctx.gcount += 1;
                let gid = ObjectId(format!("group:{}", ctx.gcount));
                ctx.nav.groups.push(gid.clone());
                let y0 = ctx.cursor;
                ctx.cursor += 1;
                let mut inner = Extent::new();
                let mut rows: Vec<RawBranch> = Vec::new();
                if g.branches.is_empty() {
                    inner.merge(&place_items(ctx, &g.items, depth + 1, Some(gid.clone()))?);
                } else {
                    for (bi, br) in g.branches.iter().enumerate() {
                        if bi > 0 || br.label.is_some() {
                            rows.push(RawBranch {
                                label: br.label.clone(),
                                y: ctx.cursor,
                                separator: bi > 0,
                            });
                            ctx.cursor += 1;
                        }
                        inner.merge(&place_items(ctx, &br.items, depth + 1, Some(gid.clone()))?);
                    }
                }
                let y1 = ctx.cursor;
                ctx.cursor += 2; // bottom border + blank row
                if inner.lo > inner.hi {
                    let c = ctx.cx[0];
                    inner.add(c, c + 1);
                }
                let header = format!("{} {}", kind_word(g.kind), g.label);
                let mut x0 = inner.lo - 2;
                let mut x1 = inner.hi + 2;
                let mut min_w = str_width(&header) as i32 + 4;
                for b in &rows {
                    if let Some(l) = &b.label {
                        min_w = min_w.max(str_width(l) as i32 + 8);
                    }
                }
                if x1 - x0 < min_w {
                    x1 = x0 + min_w;
                }
                while ctx.cx.contains(&x0) {
                    x0 -= 1;
                }
                while ctx.cx.contains(&(x1 - 1)) {
                    x1 += 1;
                }
                let rect = IRect::new(x0, y0, x1 - x0, y1 - y0 + 1);
                let mut o = RawObject::new(gid.clone(), RawKind::Group(g.kind, depth, rows), rect);
                o.label = vec![header.clone()];
                o.label_rect = Some(IRect::new(x0 + 2, y0, str_width(&header) as i32, 1));
                o.group = group.clone();
                ctx.groups.push(o);
                // the parent encloses this frame with one cell to spare
                ext.add(x0 + 1, x1 - 1);
            }
        }
    }
    let _ = ctx.pids;
    Ok(ext)
}
