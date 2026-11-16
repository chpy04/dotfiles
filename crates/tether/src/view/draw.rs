//! Drawing: the canvas (flowchart or sequence diagram) plus header, footer and overlays, all
//! rendered from [`App`] into a ratatui [`Buffer`]. Pure: nothing here mutates the model.
//!
//! Look (F3.5): rounded boxes `╭╮╰╯`, the focused object gets a heavy border `┏┓┗┛━┃` and the
//! accent fill; objects connected to the focus stay bright and everything else dims toward the
//! background. Edges use box-drawing junctions chosen from [`edge_cells`] masks, dashed edges
//! `┄┆`, arrowheads `▼▲◀▶`.

use std::collections::{BTreeMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect as RRect;
use ratatui::style::{Color, Modifier, Style};
use tether_core::diagram::{EdgeStyle, MessageKind, Resolved};
use tether_core::layout::{
    char_width, edge_cells, polyline_cells, str_width, Dir, EdgeGeom, Object, ObjectId, ObjectKind, Rect, DOWN, LEFT,
    RIGHT, UP,
};
use tether_core::theme::Rgb;

use super::app::{App, Mode, StatusKind, Tier, FOOTER_ROWS, HEADER_ROWS};
use super::keys::Action;

/// Dim factors: how far toward the background a non-focused object fades.
pub(super) const DIM_NEAR: f32 = 0.08;
pub(super) const DIM_FAR: f32 = 0.42;

fn col(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn st(fg: Rgb, bg: Rgb) -> Style {
    Style::default().fg(col(fg)).bg(col(bg))
}

// ------------------------------------------------------------------------------------ painter

/// Draws canvas coordinates into the body area of a buffer, clipping to it.
struct Painter<'a> {
    buf: &'a mut Buffer,
    body: RRect,
    ox: i32,
    oy: i32,
}

impl Painter<'_> {
    fn screen(&self, x: i32, y: i32) -> Option<(u16, u16)> {
        let (sx, sy) = (x + self.ox, y + self.oy);
        if sx < 0 || sy < 0 || sx >= self.body.width as i32 || sy >= self.body.height as i32 {
            return None;
        }
        Some((self.body.x + sx as u16, self.body.y + sy as u16))
    }

    fn put(&mut self, x: i32, y: i32, sym: &str, style: Style) {
        if let Some(p) = self.screen(x, y) {
            if let Some(c) = self.buf.cell_mut(p) {
                c.reset();
                c.set_symbol(sym);
                c.set_style(style);
            }
        }
    }

    /// Draw `s` starting at `(x, y)`; returns the width in cells.
    fn text(&mut self, x: i32, y: i32, s: &str, style: Style) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            let w = char_width(ch) as i32;
            if w == 0 {
                continue;
            }
            let mut b = [0u8; 4];
            self.put(cx, y, ch.encode_utf8(&mut b), style);
            if w == 2 {
                self.put(cx + 1, y, "", style);
            }
            cx += w;
        }
        cx - x
    }

    /// Rounded frame without touching the interior (group frames sit over lifelines and arrows).
    fn outline(&mut self, r: Rect, border: Style) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        let (x0, y0) = (r.x as i32, r.y as i32);
        let (x1, y1) = (r.right() as i32 - 1, r.bottom() as i32 - 1);
        for x in x0 + 1..x1 {
            self.put(x, y0, "─", border);
            self.put(x, y1, "─", border);
        }
        for y in y0 + 1..y1 {
            self.put(x0, y, "│", border);
            self.put(x1, y, "│", border);
        }
        self.put(x0, y0, "╭", border);
        self.put(x1, y0, "╮", border);
        self.put(x0, y1, "╰", border);
        self.put(x1, y1, "╯", border);
    }

    /// Rounded (or heavy) frame; the interior is filled with `fill`, border cells use `border`.
    fn frame(&mut self, r: Rect, heavy: bool, border: Style, fill: Style) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        let (tl, tr, bl, br, h, v) = if heavy { ("┏", "┓", "┗", "┛", "━", "┃") } else { ("╭", "╮", "╰", "╯", "─", "│") };
        let (x0, y0) = (r.x as i32, r.y as i32);
        let (x1, y1) = (r.right() as i32 - 1, r.bottom() as i32 - 1);
        for y in y0 + 1..y1 {
            for x in x0 + 1..x1 {
                self.put(x, y, " ", fill);
            }
        }
        for x in x0 + 1..x1 {
            self.put(x, y0, h, border);
            self.put(x, y1, h, border);
        }
        for y in y0 + 1..y1 {
            self.put(x0, y, v, border);
            self.put(x1, y, v, border);
        }
        self.put(x0, y0, tl, border);
        self.put(x1, y0, tr, border);
        self.put(x0, y1, bl, border);
        self.put(x1, y1, br, border);
    }
}

// ------------------------------------------------------------------------------------ colors

struct Look {
    fg: Rgb,
    fill: Rgb,
    border: Rgb,
    heavy: bool,
    focus: bool,
    bold: bool,
}

fn dim_factor(t: Tier) -> f32 {
    match t {
        Tier::Focus => 0.0,
        Tier::Near => DIM_NEAR,
        Tier::Far => DIM_FAR,
    }
}

fn box_look(app: &App, tier: Tier, fill: Rgb, hue: Rgb, searched: bool) -> Look {
    let th = &app.theme;
    if tier == Tier::Focus {
        return Look { fg: th.accent_fg, fill: th.accent, border: th.focus_border, heavy: true, focus: true, bold: true };
    }
    let f = dim_factor(tier);
    let border = if searched { th.search } else { hue.mix(th.bg, f) };
    Look { fg: th.fg.mix(th.bg, f), fill: fill.mix(th.bg, f * 0.7), border, heavy: false, focus: false, bold: true }
}

// ------------------------------------------------------------------------------------ entry

/// Render the whole screen into `area` of `buf`.
pub fn draw(app: &App, buf: &mut Buffer, area: RRect) {
    let th = &app.theme;
    // canvas background everywhere first
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(st(th.fg, th.bg));
            }
        }
    }
    if area.height < HEADER_ROWS + FOOTER_ROWS + 1 || area.width < 10 {
        return;
    }
    let body = RRect {
        x: area.x,
        y: area.y + HEADER_ROWS,
        width: area.width,
        height: area.height - HEADER_ROWS - FOOTER_ROWS,
    };
    let (ox, oy) = app.origin();
    {
        let mut p = Painter { buf, body, ox, oy };
        if app.is_flowchart() {
            draw_flow(app, &mut p);
        } else {
            draw_seq(app, &mut p);
        }
    }
    draw_scrollbars(app, buf, body);
    draw_header(app, buf, area);
    draw_footer(app, buf, area);
    match &app.mode {
        Mode::Comment(c) => draw_comment(app, buf, body, c),
        Mode::Picker(p) => draw_picker(app, buf, body, p),
        Mode::Help => draw_help(app, buf, body),
        _ => {}
    }
}

// ------------------------------------------------------------------------------------ flowchart

fn line_glyph(mask: u8, dashed: bool) -> &'static str {
    let (v, h) = (mask & (UP | DOWN) != 0, mask & (LEFT | RIGHT) != 0);
    match (v, h) {
        (false, false) => " ",
        (true, false) => {
            if dashed {
                "┆"
            } else {
                "│"
            }
        }
        (false, true) => {
            if dashed {
                "┄"
            } else {
                "─"
            }
        }
        _ => match mask {
            m if m == DOWN | RIGHT => "╭",
            m if m == DOWN | LEFT => "╮",
            m if m == UP | RIGHT => "╰",
            m if m == UP | LEFT => "╯",
            m if m == UP | DOWN | RIGHT => "├",
            m if m == UP | DOWN | LEFT => "┤",
            m if m == LEFT | RIGHT | DOWN => "┬",
            m if m == LEFT | RIGHT | UP => "┴",
            _ => "┼",
        },
    }
}

fn arrow_glyph(d: Dir, kind: Option<MessageKind>) -> &'static str {
    let open = matches!(kind, Some(MessageKind::Async) | Some(MessageKind::Return));
    match (d, open) {
        (Dir::Down, _) => "▼",
        (Dir::Up, _) => "▲",
        (Dir::Left, false) => "◀",
        (Dir::Right, false) => "▶",
        (Dir::Left, true) => "◁",
        (Dir::Right, true) => "▷",
    }
}

#[derive(Default, Clone, Copy)]
struct CellInfo {
    solid: bool,
    dashed: bool,
    /// 0 plain, 1 touches the focus, 2 is the chosen branch.
    tier: u8,
}

fn edge_tier(app: &App, e: &EdgeGeom) -> u8 {
    if e.from != app.focus && e.to != app.focus {
        return 0;
    }
    if e.from == app.focus && app.branch.get(&app.focus).is_some_and(|b| *b == e.to) {
        2
    } else {
        1
    }
}

fn edge_color(app: &App, tier: u8) -> (Rgb, bool) {
    let th = &app.theme;
    match tier {
        2 => (th.accent, true),
        1 => (th.accent.mix(th.edge, 0.45), false),
        _ => (th.edge, false),
    }
}

fn draw_flow(app: &App, p: &mut Painter) {
    let th = &app.theme;
    let l = &app.layout;

    // ---- edges
    let masks = edge_cells(l);
    let mut info: BTreeMap<(u16, u16), CellInfo> = BTreeMap::new();
    for e in &l.edges {
        let tier = edge_tier(app, e);
        for (pt, _) in polyline_cells(&e.points) {
            let c = info.entry((pt.x, pt.y)).or_default();
            if e.style == EdgeStyle::Dashed {
                c.dashed = true;
            } else {
                c.solid = true;
            }
            c.tier = c.tier.max(tier);
        }
    }
    for (&(x, y), &mask) in &masks {
        let ci = info.get(&(x, y)).copied().unwrap_or_default();
        let (color, bold) = edge_color(app, ci.tier);
        let g = line_glyph(mask, ci.dashed && !ci.solid);
        let mut style = st(color, th.bg);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        p.put(x as i32, y as i32, g, style);
    }
    for e in &l.edges {
        if let Some(last) = e.points.last() {
            let (color, bold) = edge_color(app, edge_tier(app, e));
            let mut style = st(color, th.bg);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            p.put(last.x as i32, last.y as i32, arrow_glyph(e.arrow, None), style);
        }
    }

    // ---- nodes, then panels, then edge labels
    let searched: HashSet<&ObjectId> = app.matches.iter().collect();
    for o in &l.objects {
        if let ObjectKind::Node { shape } = &o.kind {
            let colors = th.shape(*shape);
            let tier = app.tier(&o.id);
            let look = box_look(app, tier, colors.fill, colors.border, searched.contains(&o.id));
            draw_box(app, p, o, &look, app.icons.shape(*shape), colors.border, tier);
            if *shape == tether_core::diagram::Shape::Subroutine && o.rect.h >= 3 {
                let bar = look.border;
                for y in o.rect.y + 1..o.rect.bottom() - 1 {
                    p.put(o.rect.x as i32 + 1, y as i32, "│", st(bar, look.fill));
                    p.put(o.rect.right() as i32 - 2, y as i32, "│", st(bar, look.fill));
                }
            }
        }
    }
    draw_panels(app, p);
    for o in &l.objects {
        if o.kind == ObjectKind::EdgeLabel {
            let Some(r) = o.label_rect else { continue };
            let incident = o
                .owner
                .as_ref()
                .and_then(|id| l.edge(id.as_str()))
                .map(|e| edge_tier(app, e))
                .unwrap_or(0);
            let (color, bold) = match incident {
                0 => (th.dim.mix(th.bg, 0.25), false),
                2 => (th.accent, true),
                _ => (th.accent.mix(th.dim, 0.35), true),
            };
            let mut style = st(color, th.bg);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            for (i, line) in o.label.iter().enumerate() {
                p.text(r.x as i32, (r.y + i as u16) as i32, line, style);
            }
        }
    }
}

/// A node/participant/note box: frame, icon, label, code marker and `+N` badge.
fn draw_box(app: &App, p: &mut Painter, o: &Object, look: &Look, icon: &str, icon_color: Rgb, tier: Tier) {
    let th = &app.theme;
    let border = st(look.border, th.bg);
    let fill = st(look.fg, look.fill);
    p.frame(o.rect, look.heavy, if look.heavy { border.add_modifier(Modifier::BOLD) } else { border }, fill);
    let mut label_style = fill;
    if look.bold {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    if let Some(r) = o.label_rect {
        for (i, line) in o.label.iter().enumerate() {
            p.text(r.x as i32, (r.y + i as u16) as i32, line, label_style);
        }
    }
    if let Some(ic) = o.decor.icon {
        if !icon.is_empty() {
            let c = if look.focus { th.accent_fg } else { icon_color.mix(th.bg, dim_factor(tier)) };
            p.put(ic.x as i32, ic.y as i32, icon, st(c, look.fill).add_modifier(Modifier::BOLD));
        }
    }
    draw_decor(app, p, o, look.fill, look.focus, tier);
}

/// Code marker (with stale/moved state) and `+N` badge of an object.
fn draw_decor(app: &App, p: &mut Painter, o: &Object, fill: Rgb, focus: bool, tier: Tier) {
    let th = &app.theme;
    if let Some(c) = o.decor.code {
        let state = app.link_state(&o.id);
        let style = match state {
            Some(Resolved::Stale) | Some(Resolved::MissingFile) => {
                st(th.accent_fg, th.error).add_modifier(Modifier::BOLD)
            }
            Some(Resolved::Moved(_)) => st(th.warn, fill).add_modifier(Modifier::BOLD),
            _ => {
                let color = if focus { th.accent_fg } else { th.link.mix(th.bg, dim_factor(tier)) };
                st(color, fill)
            }
        };
        let glyph = match state {
            Some(Resolved::Stale) | Some(Resolved::MissingFile) => app.icons.stale(),
            _ => app.icons.code(),
        };
        p.put(c.x as i32, c.y as i32, glyph, style);
    }
    if let Some(b) = o.decor.badge {
        let color = if focus { th.accent_fg } else { th.warn.mix(th.bg, dim_factor(tier)) };
        let txt = format!("+{}", o.details_count);
        p.text(b.x as i32, b.y as i32, &txt, st(color, fill).add_modifier(Modifier::BOLD));
    }
}

fn draw_panels(app: &App, p: &mut Painter) {
    let th = &app.theme;
    for o in &app.layout.objects {
        if o.kind != ObjectKind::Details {
            continue;
        }
        let owner_focus = o.owner.as_ref().is_some_and(|id| *id == app.focus);
        let border = if owner_focus { th.accent } else { th.edge.mix(th.dim, 0.4) };
        let mut bs = st(border, th.bg);
        if owner_focus {
            bs = bs.add_modifier(Modifier::BOLD);
        }
        p.frame(o.rect, false, bs, st(th.fg, th.panel));
        for s in &o.sections {
            let is_code = s.title == "CODE";
            let title_color = if is_code { th.link } else { th.warn };
            p.text(
                o.rect.x as i32 + 2,
                s.y as i32,
                &s.title.to_uppercase(),
                st(title_color, th.panel).add_modifier(Modifier::BOLD),
            );
            for (i, line) in s.body.iter().enumerate() {
                let style = if is_code { st(th.link, th.panel).add_modifier(Modifier::UNDERLINED) } else { st(th.fg, th.panel) };
                p.text(o.rect.x as i32 + 2, (s.y + 1 + i as u16) as i32, line, style);
            }
        }
    }
}

// ------------------------------------------------------------------------------------ sequence

fn msg_color(app: &App, tier: Tier) -> Rgb {
    let th = &app.theme;
    match tier {
        Tier::Focus => th.accent,
        Tier::Near => th.dim,
        Tier::Far => th.dim.mix(th.bg, 0.45),
    }
}

fn draw_seq(app: &App, p: &mut Painter) {
    let th = &app.theme;
    let l = &app.layout;
    let focus_group: Option<&ObjectId> = l.object(app.focus.as_str()).and_then(|o| o.group.as_ref());

    // ---- lifelines
    for ll in &l.lifelines {
        let tier = app.tier(&ll.participant);
        let color = match tier {
            Tier::Focus => th.accent,
            Tier::Near => th.edge.mix(th.dim, 0.5),
            Tier::Far => th.edge,
        };
        for y in ll.y0..=ll.y1 {
            if !ll.breaks.iter().any(|&(a, b)| y >= a && y <= b) {
                p.put(ll.x as i32, y as i32, "┆", st(color, th.bg));
            }
        }
    }

    // ---- group frames (paint order: outer first)
    for o in &l.objects {
        let ObjectKind::Group { kind, branches, .. } = &o.kind else { continue };
        let hue = th.group(*kind).border;
        let active = focus_group.is_some_and(|g| *g == o.id)
            || l.object(app.focus.as_str()).is_some_and(|f| f.group.as_ref() == Some(&o.id));
        let color = if active { hue } else { hue.mix(th.bg, 0.5) };
        let r = o.rect;
        p.outline(r, st(color, th.bg));
        for b in branches {
            if b.separator {
                p.put(r.x as i32, b.y as i32, "├", st(color, th.bg));
                p.put(r.right() as i32 - 1, b.y as i32, "┤", st(color, th.bg));
                for x in r.x + 1..r.right() - 1 {
                    p.put(x as i32, b.y as i32, "┄", st(color, th.bg));
                }
            }
            if let Some(label) = &b.label {
                p.text(r.x as i32 + 2, b.y as i32, &format!("[{label}]"), st(color, th.bg).add_modifier(Modifier::BOLD));
            }
        }
        // kind tab + label on the top border
        if let (Some(lr), Some(header)) = (o.label_rect, o.label.first()) {
            let (word, rest) = header.split_once(' ').unwrap_or((header.as_str(), ""));
            let wl = str_width(word) as i32;
            let tab = format!(" {word} ");
            p.text(lr.x as i32 - 1, lr.y as i32, &tab, st(th.accent_fg, color).add_modifier(Modifier::BOLD));
            let _ = wl;
            p.text(lr.x as i32 + wl + 1, lr.y as i32, rest, st(th.fg.mix(th.bg, if active { 0.0 } else { 0.3 }), th.bg).add_modifier(Modifier::BOLD));
        }
    }

    // ---- message arrows
    let lifeline_x: HashSet<u16> = l.lifelines.iter().map(|ll| ll.x).collect();
    let masks = edge_cells(l);
    let mut cell_tier: BTreeMap<(u16, u16), (Tier, bool)> = BTreeMap::new();
    for e in &l.edges {
        let tier = app.tier(&e.id);
        for (pt, _) in polyline_cells(&e.points) {
            let entry = cell_tier.entry((pt.x, pt.y)).or_insert((Tier::Far, true));
            let rank = |t: Tier| match t {
                Tier::Focus => 2,
                Tier::Near => 1,
                Tier::Far => 0,
            };
            if rank(tier) >= rank(entry.0) {
                entry.0 = tier;
            }
            if e.style != EdgeStyle::Dashed {
                entry.1 = false;
            }
        }
    }
    for (&(x, y), &mask) in &masks {
        let (tier, dashed) = cell_tier.get(&(x, y)).copied().unwrap_or((Tier::Far, false));
        let color = msg_color(app, tier);
        let mut style = st(color, th.bg);
        if tier == Tier::Focus {
            style = style.add_modifier(Modifier::BOLD);
        }
        p.put(x as i32, y as i32, line_glyph(mask, dashed), style);
    }
    for e in &l.edges {
        let tier = app.tier(&e.id);
        let mut style = st(msg_color(app, tier), th.bg);
        if tier == Tier::Focus {
            style = style.add_modifier(Modifier::BOLD);
        }
        if let (Some(first), Some(last)) = (e.points.first(), e.points.last()) {
            p.put(last.x as i32, last.y as i32, arrow_glyph(e.arrow, e.msg_kind), style);
            // tee glyphs where the arrow meets the lifelines
            let dir_first_right = e.points.get(1).is_some_and(|n| n.x > first.x);
            let (sx, sg) = if dir_first_right { (first.x as i32 - 1, "├") } else { (first.x as i32 + 1, "┤") };
            let (ex, eg) = match e.arrow {
                Dir::Right => (last.x as i32 + 1, "┤"),
                _ => (last.x as i32 - 1, "├"),
            };
            for (x, g, y) in [(sx, sg, first.y), (ex, eg, last.y)] {
                if x >= 0 && lifeline_x.contains(&(x as u16)) {
                    p.put(x, y as i32, g, style);
                }
            }
        }
    }

    // ---- participants and notes
    let searched: HashSet<&ObjectId> = app.matches.iter().collect();
    for o in &l.objects {
        match &o.kind {
            ObjectKind::Participant { kind } => {
                let c = th.participant(*kind);
                let tier = app.tier(&o.id);
                let look = box_look(app, tier, c.fill, c.border, searched.contains(&o.id));
                draw_box(app, p, o, &look, app.icons.participant(*kind), c.border, tier);
            }
            ObjectKind::Note { .. } => {
                let c = th.note();
                let tier = app.tier(&o.id);
                let mut look = box_look(app, tier, c.fill, c.border, searched.contains(&o.id));
                if !look.focus {
                    look.bold = false;
                }
                draw_box(app, p, o, &look, "", c.border, tier);
            }
            _ => {}
        }
    }

    // ---- message labels
    for o in &l.objects {
        let ObjectKind::Message { .. } = &o.kind else { continue };
        let tier = app.tier(&o.id);
        let mut color = match tier {
            Tier::Focus => th.accent,
            Tier::Near => th.fg.mix(th.bg, 0.1),
            Tier::Far => th.fg.mix(th.bg, 0.45),
        };
        if searched.contains(&o.id) && tier != Tier::Focus {
            color = th.search;
        }
        let mut style = st(color, th.bg);
        if tier == Tier::Focus {
            style = style.add_modifier(Modifier::BOLD);
        }
        if let Some(r) = o.label_rect {
            for (i, line) in o.label.iter().enumerate() {
                p.text(r.x as i32, (r.y + i as u16) as i32, line, style);
            }
        }
        draw_decor(app, p, o, th.bg, false, tier);
        if tier == Tier::Focus && o.has_code {
            // `⏎ code` hint after the label/decor, when it fits between the lifelines
            let r = o.label_rect.unwrap_or(o.rect);
            let mut end = r.x as i32 + o.label.iter().map(|s| str_width(s)).max().unwrap_or(0) as i32;
            if let Some(c) = o.decor.code {
                end = end.max(c.x as i32 + 2);
            }
            if let Some(b) = o.decor.badge {
                end = end.max(b.x as i32 + 1 + o.details_count.to_string().len() as i32);
            }
            let stale = matches!(app.link_state(&o.id), Some(Resolved::Stale) | Some(Resolved::MissingFile));
            let (hint, hc) = if stale { (" stale link".to_string(), th.error) } else { (" ⏎ code".to_string(), th.dim) };
            if end + str_width(&hint) as i32 <= o.rect.right() as i32 {
                p.text(end, r.y as i32, &hint, st(hc, th.bg).add_modifier(Modifier::ITALIC));
            }
        }
    }
    draw_panels(app, p);
}

// ------------------------------------------------------------------------------------ chrome

fn put_str(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max_x: u16) -> u16 {
    let mut cx = x;
    for ch in s.chars() {
        let w = char_width(ch) as u16;
        if w == 0 {
            continue;
        }
        if cx + w > max_x {
            break;
        }
        if let Some(c) = buf.cell_mut((cx, y)) {
            let mut b = [0u8; 4];
            c.set_symbol(ch.encode_utf8(&mut b));
            c.set_style(style);
        }
        if w == 2 {
            if let Some(c) = buf.cell_mut((cx + 1, y)) {
                c.set_symbol("");
                c.set_style(style);
            }
        }
        cx += w;
    }
    cx
}

fn fill_row(buf: &mut Buffer, area: RRect, y: u16, style: Style) {
    for x in area.x..area.x + area.width {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.reset();
            c.set_style(style);
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = char_width(ch);
        if w + cw + 1 > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

fn draw_header(app: &App, buf: &mut Buffer, area: RRect) {
    let th = &app.theme;
    let bar = st(th.fg, th.bar);
    fill_row(buf, area, area.y, bar);
    let flow = app.is_flowchart();
    let title = app.diagram.title().to_string();
    let counts = match &app.diagram {
        tether_core::diagram::Diagram::Flowchart(f) => format!("flowchart · {} nodes · {} edges", f.nodes.len(), f.edges.len()),
        tether_core::diagram::Diagram::Sequence(s) => {
            let msgs = app.layout.seq_nav().map_or(0, |n| n.messages.len());
            format!("sequence · {} participants · {} messages", s.participants.len(), msgs)
        }
    };
    let right_edge = area.x + area.width;
    let mut x = put_str(buf, area.x + 1, area.y, app.icons.diagram(flow), st(th.accent, th.bar).add_modifier(Modifier::BOLD), right_edge);
    x = put_str(buf, x + 1, area.y, &truncate(&title, (area.width as usize).saturating_sub(8)), bar.add_modifier(Modifier::BOLD), right_edge);
    x = put_str(buf, x + 2, area.y, &counts, st(th.dim.mix(th.bar, 0.3), th.bar), right_edge);
    // focused element, right aligned
    if let Some(el) = app.focused_element() {
        let crumb = format!("{}: {}", el.kind.word(), el.label.replace('\n', " "));
        let room = right_edge.saturating_sub(x + 3) as usize;
        if room > 8 {
            let crumb = truncate(&crumb, room.min(48));
            let w = str_width(&crumb) as u16;
            put_str(buf, right_edge.saturating_sub(w + 1), area.y, &crumb, st(th.dim, th.bar), right_edge);
        }
    }
}

fn hint_segments(app: &App) -> Vec<(String, &'static str)> {
    let km = &app.keymap;
    let mut v: Vec<(String, &'static str)> = if app.is_flowchart() {
        vec![("hjkl".into(), "move")]
    } else {
        vec![("j/k".into(), "message"), ("h/l".into(), "participant")]
    };
    v.push((km.hint(Action::Open), "code"));
    v.push((km.hint(Action::Details), "details"));
    v.push((km.hint(Action::Comment), "ask"));
    v.push((km.hint(Action::Search), "search"));
    v.push((km.hint(Action::Help), "help"));
    v.push((km.hint(Action::Quit), "quit"));
    v
}

fn draw_footer(app: &App, buf: &mut Buffer, area: RRect) {
    let th = &app.theme;
    let status_y = area.y + area.height - 2;
    let hint_y = area.y + area.height - 1;
    let right = area.x + area.width;
    fill_row(buf, area, status_y, st(th.fg, th.bg));
    fill_row(buf, area, hint_y, st(th.fg, th.bar));

    // ---- status line: search prompt > pending leader > status message
    if let Mode::Search(s) = &app.mode {
        let q = s.input.lines.first().cloned().unwrap_or_default();
        let x = put_str(buf, area.x + 1, status_y, "/", st(th.accent, th.bg).add_modifier(Modifier::BOLD), right);
        let x = put_str(buf, x, status_y, &q, st(th.fg, th.bg), right);
        put_str(buf, x, status_y, "█", st(th.accent, th.bg), right);
        let info = if q.is_empty() {
            "type to search labels".to_string()
        } else if app.matches.is_empty() {
            "no match".to_string()
        } else {
            let i = app.matches.iter().position(|m| *m == app.focus).map(|i| i + 1).unwrap_or(0);
            format!("{i}/{}", app.matches.len())
        };
        let w = str_width(&info) as u16;
        put_str(buf, right.saturating_sub(w + 1), status_y, &info, st(th.dim, th.bg), right);
    } else if let Some(pending) = app.pending_hint() {
        let x = put_str(buf, area.x + 1, status_y, &pending, st(th.accent, th.bg).add_modifier(Modifier::BOLD), right);
        let pend = app.pending.clone();
        let conts: Vec<String> = app
            .keymap
            .continuations(&pend)
            .into_iter()
            .map(|(k, a)| {
                let name = match a {
                    Action::Comment => "comment primary agent",
                    Action::CommentPick => "comment, pick agent",
                    _ => "",
                };
                format!("{k} {name}")
            })
            .collect();
        put_str(buf, x + 2, status_y, &conts.join(" · "), st(th.dim, th.bg), right);
    } else if let Some(s) = &app.status {
        let color = match s.kind {
            StatusKind::Info => th.dim,
            StatusKind::Ok => th.ok,
            StatusKind::Warn => th.warn,
            StatusKind::Error => th.error,
        };
        put_str(buf, area.x + 1, status_y, &truncate(&s.text, (area.width as usize).saturating_sub(2)), st(color, th.bg), right);
    }

    // ---- hint line
    let mut x = area.x + 1;
    let sep = st(th.edge, th.bar);
    for (i, (key, desc)) in hint_segments(app).into_iter().enumerate() {
        let w = str_width(&key) + 1 + str_width(desc) + if i > 0 { 3 } else { 0 };
        if x as usize + w >= right as usize {
            break;
        }
        if i > 0 {
            x = put_str(buf, x, hint_y, " · ", sep, right);
        }
        x = put_str(buf, x, hint_y, &key, st(th.accent, th.bar).add_modifier(Modifier::BOLD), right);
        x = put_str(buf, x + 1, hint_y, desc, st(th.dim, th.bar), right);
    }
}

fn draw_scrollbars(app: &App, buf: &mut Buffer, body: RRect) {
    let th = &app.theme;
    let (bw, bh) = app.body_size();
    let (cw, ch) = (app.layout.width + 2 * super::app::PAD_X, app.layout.height + 2 * super::app::PAD_Y);
    if ch > bh && bh > 2 {
        let thumb = ((bh as u32 * bh as u32) / ch as u32).max(1) as u16;
        let track = bh.saturating_sub(thumb);
        let top = if ch > bh { (app.scroll_y as u32 * track as u32 / (ch - bh) as u32) as u16 } else { 0 };
        for i in 0..thumb {
            if let Some(c) = buf.cell_mut((body.x + body.width - 1, body.y + top + i)) {
                c.set_symbol("┃");
                c.set_style(st(th.edge, th.bg));
            }
        }
    }
    if cw > bw && bw > 2 {
        let thumb = ((bw as u32 * bw as u32) / cw as u32).max(2) as u16;
        let track = bw.saturating_sub(thumb);
        let left = (app.scroll_x as u32 * track as u32 / (cw - bw) as u32) as u16;
        for i in 0..thumb {
            if let Some(c) = buf.cell_mut((body.x + left + i, body.y + body.height - 1)) {
                c.set_symbol("━");
                c.set_style(st(th.edge, th.bg));
            }
        }
    }
}

// ------------------------------------------------------------------------------------ overlays

/// A bordered box with a title on the top border and a footer on the bottom border.
fn overlay_frame(app: &App, buf: &mut Buffer, r: RRect, title: &str, footer: &str) {
    let th = &app.theme;
    let bg = th.panel;
    let border = st(th.accent, bg).add_modifier(Modifier::BOLD);
    let (x0, y0) = (r.x, r.y);
    let (x1, y1) = (r.x + r.width - 1, r.y + r.height - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_symbol(" ");
                c.set_style(st(th.fg, bg));
            }
        }
    }
    for x in x0 + 1..x1 {
        put_str(buf, x, y0, "─", border, x1 + 1);
        put_str(buf, x, y1, "─", border, x1 + 1);
    }
    for y in y0 + 1..y1 {
        put_str(buf, x0, y, "│", border, x1 + 1);
        put_str(buf, x1, y, "│", border, x1 + 1);
    }
    put_str(buf, x0, y0, "╭", border, x1 + 1);
    put_str(buf, x1, y0, "╮", border, x1 + 1);
    put_str(buf, x0, y1, "╰", border, x1 + 1);
    put_str(buf, x1, y1, "╯", border, x1 + 1);
    if !title.is_empty() {
        let t = format!(" {} ", truncate(title, (r.width as usize).saturating_sub(6)));
        put_str(buf, x0 + 2, y0, &t, st(th.accent, bg).add_modifier(Modifier::BOLD), x1);
    }
    if !footer.is_empty() {
        let f = format!(" {footer} ");
        let w = str_width(&f) as u16;
        if w + 4 <= r.width {
            put_str(buf, x1.saturating_sub(w + 1), y1, &f, st(th.dim, bg), x1);
        }
    }
}

/// Soft-wrap `lines` to `width` columns; returns rows and the `(row, col)` of the cursor.
fn wrap_input(lines: &[String], width: usize, cur: (usize, usize)) -> (Vec<String>, (usize, usize)) {
    let mut rows: Vec<String> = Vec::new();
    let mut cursor = (0usize, 0usize);
    for (li, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let start = rows.len();
        if chars.is_empty() {
            rows.push(String::new());
        } else {
            for chunk in chars.chunks(width.max(1)) {
                rows.push(chunk.iter().collect());
            }
        }
        if li == cur.0 {
            let w = width.max(1);
            let mut r = start + cur.1 / w;
            let mut c = cur.1 % w;
            if cur.1 > 0 && cur.1 == chars.len() && c == 0 {
                // cursor at the very end of a full row: stay on that row, past its last cell
                r -= 1;
                c = w;
            }
            if c >= w {
                rows.push(String::new());
                r += 1;
                c = 0;
            }
            cursor = (r, c);
        }
    }
    (rows, cursor)
}

fn draw_comment(app: &App, buf: &mut Buffer, body: RRect, c: &super::app::CommentState) {
    let th = &app.theme;
    let width = 50u16.min(body.width.saturating_sub(2)).max(20.min(body.width));
    let inner_w = width.saturating_sub(4) as usize;
    let (rows, cur) = wrap_input(&c.input.lines, inner_w, (c.input.row, c.input.col));
    let max_rows = 8usize.min(body.height.saturating_sub(4) as usize).max(1);
    let shown = rows.len().clamp(1, max_rows);
    let first = if cur.0 >= shown { cur.0 + 1 - shown } else { 0 };
    let height = shown as u16 + 2;
    // anchor under the focused object (above it when there is no room)
    let (ox, oy) = app.origin();
    let (ax, ay, ab) = match app.layout.object(app.focus.as_str()) {
        Some(o) => (o.rect.x as i32 + ox, o.rect.y as i32 + oy, o.rect.bottom() as i32 + oy),
        None => (0, 0, 1),
    };
    let x = ax.clamp(0, body.width.saturating_sub(width) as i32) as u16;
    let mut y = ab;
    if y + height as i32 > body.height as i32 {
        y = ay - height as i32;
    }
    let y = y.clamp(0, body.height.saturating_sub(height) as i32) as u16;
    let r = RRect { x: body.x + x, y: body.y + y, width, height };
    overlay_frame(app, buf, r, &c.title, "⏎ send · ^j newline · esc cancel");
    let text_style = st(th.fg, th.panel);
    for (i, row) in rows.iter().skip(first).take(shown).enumerate() {
        put_str(buf, r.x + 2, r.y + 1 + i as u16, row, text_style, r.x + r.width - 1);
    }
    // caret
    let (cr, cc) = (cur.0 - first, cur.1);
    let (cx, cy) = (r.x + 2 + cc as u16, r.y + 1 + cr as u16);
    if cx < r.x + r.width - 1 {
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            let sym = cell.symbol().to_string();
            cell.set_symbol(if sym.trim().is_empty() { " " } else { &sym });
            cell.set_style(st(th.accent_fg, th.accent));
        }
    }
}

fn draw_picker(app: &App, buf: &mut Buffer, body: RRect, p: &super::app::PickerState) {
    let th = &app.theme;
    let lines: Vec<(String, String, String, bool)> = p
        .rows
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    let left = format!("tab {} · {}", r.tab_index, r.kind);
                    (left, r.state.clone(), if r.is_primary { "primary".to_string() } else { String::new() }, true)
                })
                .collect()
        })
        .unwrap_or_default();
    let content_w = lines.iter().map(|(l, s, pr, _)| str_width(l) + 3 + str_width(s) + if pr.is_empty() { 0 } else { 3 + str_width(pr) }).max().unwrap_or(0);
    let width = ((content_w + 8).max(34) as u16).min(body.width.saturating_sub(2));
    let n = lines.len().max(1) as u16;
    let height = (n + 2).min(body.height.saturating_sub(2)).max(3);
    let r = RRect {
        x: body.x + body.width.saturating_sub(width) / 2,
        y: body.y + body.height.saturating_sub(height) / 2,
        width,
        height,
    };
    overlay_frame(app, buf, r, "Send comment to", "⏎ pick · esc cancel");
    let right = r.x + r.width - 1;
    if p.rows.is_none() {
        put_str(buf, r.x + 3, r.y + 1, "loading agents…", st(th.dim, th.panel), right);
        return;
    }
    if lines.is_empty() {
        let msg = p.error.clone().unwrap_or_else(|| "no agents in this workspace".to_string());
        put_str(buf, r.x + 3, r.y + 1, &truncate(&msg, width.saturating_sub(6) as usize), st(th.error, th.panel), right);
        return;
    }
    for (i, (left, state, primary, _)) in lines.iter().enumerate().take(height.saturating_sub(2) as usize) {
        let y = r.y + 1 + i as u16;
        let selected = i == p.sel;
        let bg = if selected { th.edge } else { th.panel };
        for x in r.x + 1..right {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ");
                c.set_style(st(th.fg, bg));
            }
        }
        if selected {
            put_str(buf, r.x + 1, y, "▌", st(th.accent, bg), right);
        }
        let mut name = st(th.fg, bg);
        if selected {
            name = name.add_modifier(Modifier::BOLD);
        }
        let x = put_str(buf, r.x + 3, y, left, name, right);
        let x = put_str(buf, x, y, " — ", st(th.dim, bg), right);
        let x = put_str(buf, x, y, state, st(th.agent_state(state), bg).add_modifier(Modifier::BOLD), right);
        if !primary.is_empty() {
            put_str(buf, x + 1, y, &format!("({primary})"), st(th.dim, bg), right);
        }
    }
}

fn draw_help(app: &App, buf: &mut Buffer, body: RRect) {
    let th = &app.theme;
    let km = &app.keymap;
    let flow = app.is_flowchart();
    let mut rows: Vec<(String, String)> = Vec::new();
    if flow {
        rows.push(("j / k".into(), "follow an edge / go back to the parent".into()));
        rows.push(("h / l".into(), "siblings; at a decision pick a branch".into()));
    } else {
        rows.push(("j / k".into(), "next / previous message".into()));
        rows.push(("h / l".into(), "previous / next participant".into()));
    }
    rows.push(("g / G".into(), "first / last".into()));
    rows.push((km.hint(Action::Open), "open the linked code in Neovim".into()));
    rows.push((km.hint(Action::Details), "expand / collapse details".into()));
    rows.push((km.hint(Action::Comment), "comment to the primary agent".into()));
    rows.push((km.hint(Action::CommentPick), "comment to a picked agent".into()));
    rows.push((format!("{} / n / N", km.hint(Action::Search)), "search labels / next / previous".into()));
    rows.push(("^d / ^u".into(), "scroll half a page (also the mouse wheel)".into()));
    rows.push((km.hint(Action::Help), "this help".into()));
    rows.push((km.hint(Action::Quit), "quit".into()));
    let kw = rows.iter().map(|(k, _)| str_width(k)).max().unwrap_or(0);
    let content = rows.iter().map(|(_, d)| kw + 3 + str_width(d)).max().unwrap_or(0);
    let width = ((content + 6) as u16).min(body.width.saturating_sub(2));
    let height = ((rows.len() + 2) as u16).min(body.height.saturating_sub(1));
    let r = RRect {
        x: body.x + body.width.saturating_sub(width) / 2,
        y: body.y + body.height.saturating_sub(height) / 2,
        width,
        height,
    };
    overlay_frame(app, buf, r, "Keys", "any key closes");
    let right = r.x + r.width - 1;
    for (i, (k, d)) in rows.iter().enumerate().take(height.saturating_sub(2) as usize) {
        let y = r.y + 1 + i as u16;
        put_str(buf, r.x + 2, y, k, st(th.accent, th.panel).add_modifier(Modifier::BOLD), right);
        put_str(buf, r.x + 2 + kw as u16 + 2, y, d, st(th.fg, th.panel), right);
    }
}
