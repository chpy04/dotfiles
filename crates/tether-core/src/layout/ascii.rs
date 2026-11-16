//! Plain-text rendering of a [`Layout`] for snapshot tests and eyeballing.
//!
//! Boxes use `+-|`, edges `-|` (`+` at corners and crossings), arrowheads `v ^ < >`. Dashed
//! edges use `.` and `:`, lifelines `:`. Icons: `*` decision, `o` terminal, `/` io, `#`
//! subroutine, `=` database; `@` marks a code link; `+N` is the details badge.

use std::collections::{BTreeMap, HashSet};

use crate::diagram::{EdgeStyle, ParticipantKind, Shape};

use super::{
    edge_cells, polyline_cells, Dir, Layout, Object, ObjectKind, Rect, DOWN, LEFT, RIGHT, UP,
};

struct Grid {
    w: usize,
    h: usize,
    cells: Vec<Vec<char>>,
}

impl Grid {
    fn new(w: u16, h: u16) -> Self {
        Grid {
            w: w as usize,
            h: h as usize,
            cells: vec![vec![' '; w as usize]; h as usize],
        }
    }
    fn put(&mut self, x: u16, y: u16, c: char) {
        let (x, y) = (x as usize, y as usize);
        if x < self.w && y < self.h {
            self.cells[y][x] = c;
        }
    }
    fn text(&mut self, x: u16, y: u16, s: &str) {
        let mut cx = x;
        for ch in s.chars() {
            self.put(cx, y, ch);
            cx += super::text::char_width(ch).max(1) as u16;
        }
    }
    fn clear(&mut self, r: Rect) {
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.put(x, y, ' ');
            }
        }
    }
    fn frame(&mut self, r: Rect) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        for x in r.x..r.right() {
            self.put(x, r.y, '-');
            self.put(x, r.bottom() - 1, '-');
        }
        for y in r.y..r.bottom() {
            self.put(r.x, y, '|');
            self.put(r.right() - 1, y, '|');
        }
        for (x, y) in [
            (r.x, r.y),
            (r.right() - 1, r.y),
            (r.x, r.bottom() - 1),
            (r.right() - 1, r.bottom() - 1),
        ] {
            self.put(x, y, '+');
        }
    }
}

fn shape_icon(s: Shape) -> char {
    match s {
        Shape::Process => ' ',
        Shape::Decision => '*',
        Shape::Terminal => 'o',
        Shape::Io => '/',
        Shape::Subroutine => '#',
        Shape::Database => '=',
    }
}

fn participant_icon(k: Option<ParticipantKind>) -> char {
    match k {
        Some(ParticipantKind::Actor) => 'o',
        Some(ParticipantKind::Service) => '#',
        Some(ParticipantKind::Database) => '=',
        Some(ParticipantKind::Queue) => '~',
        Some(ParticipantKind::External) => '&',
        None => ' ',
    }
}

/// Render the geometry as text (one `\n` terminated line per row, trailing spaces trimmed).
pub fn ascii_debug(layout: &Layout) -> String {
    let mut g = Grid::new(layout.width, layout.height);

    // lifelines
    for l in &layout.lifelines {
        for y in l.y0..=l.y1 {
            if !l.breaks.iter().any(|&(a, b)| y >= a && y <= b) {
                g.put(l.x, y, ':');
            }
        }
    }
    // group frames
    for o in &layout.objects {
        if let ObjectKind::Group { branches, .. } = &o.kind {
            let r = o.rect;
            g.frame(r);
            if let Some(lr) = o.label_rect {
                g.text(lr.x, lr.y, &o.label.join(" "));
            }
            for b in branches {
                if b.separator {
                    g.put(r.x, b.y, '|');
                    for x in r.x + 1..r.right() - 1 {
                        g.put(x, b.y, if (x - r.x) % 2 == 0 { '-' } else { ' ' });
                    }
                    g.put(r.right() - 1, b.y, '|');
                }
                if let Some(l) = &b.label {
                    g.text(r.x + 2, b.y, &format!("[{l}]"));
                }
            }
        }
    }
    // boxes
    for o in &layout.objects {
        match &o.kind {
            ObjectKind::Node { shape } => draw_box(&mut g, o, shape_icon(*shape)),
            ObjectKind::Participant { kind } => draw_box(&mut g, o, participant_icon(*kind)),
            ObjectKind::Note { .. } => draw_box(&mut g, o, ' '),
            ObjectKind::Details => {
                g.clear(o.rect);
                g.frame(o.rect);
                for s in &o.sections {
                    g.text(o.rect.x + 2, s.y, &s.title.to_uppercase());
                    for (i, b) in s.body.iter().enumerate() {
                        g.text(o.rect.x + 2, s.y + 1 + i as u16, b);
                    }
                }
            }
            _ => {}
        }
    }
    // edges
    let mask = edge_cells(layout);
    let mut solid: HashSet<(u16, u16)> = HashSet::new();
    let mut dashed: HashSet<(u16, u16)> = HashSet::new();
    for e in &layout.edges {
        for (p, _) in polyline_cells(&e.points) {
            if e.style == EdgeStyle::Dashed {
                dashed.insert((p.x, p.y));
            } else {
                solid.insert((p.x, p.y));
            }
        }
    }
    for (&(x, y), &bits) in &mask {
        let vert = bits & (UP | DOWN) != 0;
        let horiz = bits & (LEFT | RIGHT) != 0;
        let only_dashed = dashed.contains(&(x, y)) && !solid.contains(&(x, y));
        let c = match (vert, horiz) {
            (true, false) => {
                if only_dashed {
                    ':'
                } else {
                    '|'
                }
            }
            (false, true) => {
                if only_dashed {
                    '.'
                } else {
                    '-'
                }
            }
            (false, false) => ' ',
            _ => '+',
        };
        g.put(x, y, c);
    }
    for e in &layout.edges {
        if let Some(last) = e.points.last() {
            let c = match e.arrow {
                Dir::Up => '^',
                Dir::Down => 'v',
                Dir::Left => '<',
                Dir::Right => '>',
            };
            g.put(last.x, last.y, c);
        }
    }
    // text on top
    let mut by_row: BTreeMap<u16, ()> = BTreeMap::new();
    for o in &layout.objects {
        match &o.kind {
            ObjectKind::EdgeLabel | ObjectKind::Message { .. } => {
                if let Some(r) = o.label_rect {
                    for (i, l) in o.label.iter().enumerate() {
                        g.text(r.x, r.y + i as u16, l);
                    }
                }
                decorate(&mut g, o);
                by_row.insert(o.rect.y, ());
            }
            _ => {}
        }
    }
    let mut s = String::new();
    for row in &g.cells {
        let line: String = row.iter().collect();
        s.push_str(line.trim_end());
        s.push('\n');
    }
    s
}

fn draw_box(g: &mut Grid, o: &Object, icon: char) {
    g.clear(o.rect);
    g.frame(o.rect);
    if let Some(r) = o.label_rect {
        for (i, l) in o.label.iter().enumerate() {
            g.text(r.x, r.y + i as u16, l);
        }
    }
    if let Some(p) = o.decor.icon {
        g.put(p.x, p.y, icon);
    }
    decorate(g, o);
}

fn decorate(g: &mut Grid, o: &Object) {
    if let Some(p) = o.decor.code {
        g.put(p.x, p.y, '@');
    }
    if let Some(p) = o.decor.badge {
        g.text(p.x, p.y, &format!("+{}", o.details_count));
    }
}
