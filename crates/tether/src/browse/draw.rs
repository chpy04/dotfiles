//! Renders the browser into a ratatui buffer: header, list on the left, the viewer's own drawing
//! of the selected diagram on the right, a prompt/status line and key hints.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use tether_core::diagram::{format_age, DiagramInfo, DiagramKind};
use tether_core::layout::{char_width, str_width};
use tether_core::theme::Rgb;

use super::app::{Browser, LineInput, Mode, StatusKind};
use crate::view::app::{FOOTER_ROWS, HEADER_ROWS};

fn col(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn st(fg: Rgb, bg: Rgb) -> Style {
    Style::default().fg(col(fg)).bg(col(bg))
}

/// Draw `s` at `(x, y)` clipped to `max_x` (exclusive); returns the x after the text.
fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max_x: u16) -> u16 {
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

fn fill(buf: &mut Buffer, r: Rect, style: Style) {
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(style);
            }
        }
    }
}

/// `s` cut to `w` cells, ending in `…` when it was cut.
pub fn fit(s: &str, w: usize) -> String {
    if str_width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = char_width(ch);
        if used + cw + 1 > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

fn pad(s: &str, w: usize) -> String {
    let n = str_width(s);
    format!("{s}{}", " ".repeat(w.saturating_sub(n)))
}

fn lpad(s: &str, w: usize) -> String {
    let n = str_width(s);
    format!("{}{s}", " ".repeat(w.saturating_sub(n)))
}

/// Render the whole browser into `area`.
pub fn draw(b: &Browser, buf: &mut Buffer, area: Rect) {
    let th = &b.theme;
    fill(buf, area, st(th.fg, th.bg));
    if area.height < 6 || area.width < 30 {
        put(buf, area.x, area.y, "terminal too small", st(th.warn, th.bg), area.x + area.width);
        return;
    }
    let right = area.x + area.width;
    let body = Rect { x: area.x, y: area.y + HEADER_ROWS, width: area.width, height: area.height - HEADER_ROWS - FOOTER_ROWS };
    draw_header(b, buf, area);
    if b.rows.is_empty() {
        draw_empty(b, buf, body);
    } else {
        let list_w = if area.width < 80 { area.width / 2 } else { (area.width * 2 / 5).clamp(40, 64) };
        let list = Rect { width: list_w, ..body };
        let prev = Rect { x: body.x + list_w, width: body.width - list_w, ..body };
        draw_list(b, buf, list);
        draw_preview(b, buf, prev);
    }
    draw_footer(b, buf, area);
    if matches!(b.mode, Mode::Help) {
        draw_help(b, buf, body, right);
    }
}

fn draw_header(b: &Browser, buf: &mut Buffer, area: Rect) {
    let th = &b.theme;
    let right = area.x + area.width;
    let bar = st(th.fg, th.bar);
    fill(buf, Rect { height: 1, ..area }, bar);
    let mut x = area.x + 1;
    x = put(buf, x, area.y, b.icons.diagram(true), st(th.accent, th.bar), right);
    x = put(buf, x, area.y, " tether diagrams", bar.add_modifier(Modifier::BOLD), right);
    let scope = if b.all_repos { "all repos".to_string() } else { b.repo.clone() };
    x = put(buf, x, area.y, "  ·  ", st(th.dim, th.bar), right);
    x = put(buf, x, area.y, &scope, st(th.link, th.bar), right);
    let total = b.visible().len();
    let count = if total == b.rows.len() { format!("{total} diagram{}", if total == 1 { "" } else { "s" }) } else { format!("{total} of {}", b.rows.len()) };
    x = put(buf, x, area.y, "  ·  ", st(th.dim, th.bar), right);
    x = put(buf, x, area.y, &count, st(th.dim, th.bar), right);
    if b.starred_only {
        x = put(buf, x, area.y, "  ★ only", st(th.warn, th.bar), right);
    }
    if !b.filter.is_empty() {
        put(buf, x, area.y, &format!("  / {}", b.filter), st(th.accent, th.bar), right);
    }
}

fn draw_empty(b: &Browser, buf: &mut Buffer, body: Rect) {
    let th = &b.theme;
    let right = body.x + body.width;
    let days = if b.keep_days == 0 { "kept until you delete them".to_string() } else { format!("kept for {} days unless starred", b.keep_days) };
    let scope = if b.all_repos { "in any repo yet".to_string() } else { format!("for {} yet", b.repo) };
    let lines: Vec<(String, Style)> = vec![
        (format!("No saved diagrams {scope}."), st(th.fg, th.bg).add_modifier(Modifier::BOLD)),
        (String::new(), st(th.fg, th.bg)),
        ("Ask Claude to draw one (\"diagram the login flow\"), or run".into(), st(th.dim, th.bg)),
        ("    tether diagram create spec.json".into(), st(th.link, th.bg)),
        (String::new(), st(th.fg, th.bg)),
        ("Diagrams are saved per repo, shared by all its worktrees,".to_string(), st(th.dim, th.bg)),
        (format!("and {days}."), st(th.dim, th.bg)),
        (String::new(), st(th.fg, th.bg)),
        ("a  all repos      ?  help      q  quit".into(), st(th.dim, th.bg)),
    ];
    let top = body.y + body.height.saturating_sub(lines.len() as u16) / 2;
    for (i, (l, style)) in lines.iter().enumerate() {
        let w = str_width(l) as u16;
        let x = body.x + body.width.saturating_sub(w) / 2;
        put(buf, x, top + i as u16, l, *style, right);
    }
}

fn kind_icon(b: &Browser, k: DiagramKind) -> &'static str {
    b.icons.diagram(k == DiagramKind::Flowchart)
}

fn draw_list(b: &Browser, buf: &mut Buffer, r: Rect) {
    let th = &b.theme;
    let right = r.x + r.width;
    let rows = b.visible();
    if rows.is_empty() {
        let msg = if b.starred_only && b.filter.is_empty() { "No starred diagrams.".to_string() } else { format!("No diagrams match \"{}\".", b.filter) };
        put(buf, r.x + 2, r.y + 1, &msg, st(th.dim, th.bg), right);
        put(buf, r.x + 2, r.y + 3, "Esc clears the filter, S toggles starred only.", st(th.dim, th.bg), right);
        return;
    }
    let h = r.height as usize;
    let top = b.sel.saturating_sub(h / 2).min(rows.len().saturating_sub(h));
    // columns: "★ " title … "<icon> branch age"
    let age_w = 4usize;
    let branch_w = ((r.width as usize).saturating_sub(24) * if b.all_repos { 2 } else { 1 } / if b.all_repos { 5 } else { 3 }).clamp(6, 24);
    let right_w = 1 + 1 + branch_w + 1 + age_w + 1;
    let title_w = (r.width as usize).saturating_sub(2 + right_w + 1).max(8);
    for (i, row) in rows.iter().enumerate().skip(top).take(h) {
        let y = r.y + (i - top) as u16;
        let selected = i == b.sel;
        let (fg, bg) = if selected { (th.accent_fg, th.accent) } else { (th.fg, th.bg) };
        let base = st(fg, bg);
        fill(buf, Rect { x: r.x, y, width: r.width, height: 1 }, base);
        let star_style = if selected { st(th.accent_fg, bg) } else { st(th.warn, bg) };
        put(buf, r.x + 1, y, if row.starred { "★" } else { " " }, star_style, right);
        let title = fit(&row.title, title_w);
        put(buf, r.x + 3, y, &title, if selected { base.add_modifier(Modifier::BOLD) } else { base }, right);
        let dim = if selected { st(th.accent_fg, bg) } else { st(th.dim, bg) };
        let mut x = right.saturating_sub(right_w as u16);
        x = put(buf, x, y, kind_icon(b, row.kind), if selected { dim } else { st(th.link, bg) }, right);
        x = put(buf, x + 1, y, &pad(&fit(&branch_label(b, row), branch_w), branch_w), dim, right);
        put(buf, x + 1, y, &lpad(&format_age(b.now, row.activity()), age_w), dim, right);
    }
}

fn branch_label(b: &Browser, r: &DiagramInfo) -> String {
    let branch = r.branch.clone().unwrap_or_else(|| "-".into());
    if b.all_repos {
        format!("{}:{branch}", r.repo)
    } else {
        branch
    }
}

fn draw_preview(b: &Browser, buf: &mut Buffer, r: Rect) {
    let th = &b.theme;
    let right = r.x + r.width;
    // separator
    for y in r.y..r.y + r.height {
        if let Some(c) = buf.cell_mut((r.x, y)) {
            c.set_symbol("│");
            c.set_style(st(th.edge, th.bg));
        }
    }
    let inner = Rect { x: r.x + 1, y: r.y, width: r.width.saturating_sub(1), height: r.height };
    let Some(row) = b.selected() else {
        b.view.set((inner.width, inner.height.saturating_sub(2)));
        return;
    };
    // title strip
    fill(buf, Rect { height: 1, ..inner }, st(th.fg, th.panel));
    let mut x = put(buf, inner.x + 1, inner.y, kind_icon(b, row.kind), st(th.link, th.panel), right);
    x = put(buf, x + 1, inner.y, &fit(&row.title, (inner.width as usize).saturating_sub(24).max(10)), st(th.fg, th.panel).add_modifier(Modifier::BOLD), right);
    let mut meta = vec![row.kind.as_str().to_string()];
    if let Some(br) = &row.branch {
        meta.push(br.clone());
    }
    if b.all_repos {
        meta.push(row.repo.clone());
    }
    put(buf, x + 2, inner.y, &meta.join(" · "), st(th.dim, th.panel), right);

    let canvas = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: inner.height.saturating_sub(2) };
    b.view.set((canvas.width, canvas.height));
    let foot_y = inner.y + inner.height - 1;
    match b.previews.get(&super::app::preview_key(row)) {
        None => {
            put(buf, canvas.x + 2, canvas.y + 1, "rendering…", st(th.dim, th.bg), right);
        }
        Some(Err(e)) => {
            put(buf, canvas.x + 2, canvas.y + 1, "cannot render this diagram:", st(th.error, th.bg), right);
            put(buf, canvas.x + 2, canvas.y + 2, &fit(e, (canvas.width as usize).saturating_sub(4)), st(th.dim, th.bg), right);
        }
        Some(Ok(src)) => {
            // the viewer's canvas without its own header and footer rows, cropped to the pane
            let (cw, ch) = (src.area.width, src.area.height.saturating_sub(HEADER_ROWS + FOOTER_ROWS));
            let sy = b.scroll_y.min(ch.saturating_sub(canvas.height));
            let sx = b.scroll_x.min(cw.saturating_sub(canvas.width));
            for dy in 0..canvas.height.min(ch) {
                for dx in 0..canvas.width.min(cw) {
                    let from = (src.area.x + sx + dx, src.area.y + HEADER_ROWS + sy + dy);
                    if let (Some(s), Some(d)) = (src.cell(from), buf.cell_mut((canvas.x + dx, canvas.y + dy))) {
                        *d = s.clone();
                    }
                }
            }
            if ch > canvas.height || cw > canvas.width {
                let mut s = String::new();
                if ch > canvas.height {
                    s.push_str(&format!(" rows {}-{}/{}  ^d ^u scroll", sy + 1, (sy + canvas.height).min(ch), ch));
                }
                if cw > canvas.width {
                    s.push_str(&format!(" cols {}-{}/{}  < > pan", sx + 1, (sx + canvas.width).min(cw), cw));
                }
                fill(buf, Rect { x: inner.x, y: foot_y, width: inner.width, height: 1 }, st(th.dim, th.panel));
                put(buf, inner.x, foot_y, &s, st(th.dim, th.panel), right);
            }
        }
    }
}

fn input_line(buf: &mut Buffer, x: u16, y: u16, label: &str, input: &LineInput, style: Style, max_x: u16) {
    let x = put(buf, x, y, label, style.add_modifier(Modifier::BOLD), max_x);
    let before: String = input.text.chars().take(input.cursor).collect();
    let after: String = input.text.chars().skip(input.cursor).collect();
    let cx = put(buf, x, y, &before, style, max_x);
    let cur = after.chars().next().map(|c| c.to_string()).unwrap_or_else(|| " ".into());
    let cx = put(buf, cx, y, &cur, style.add_modifier(Modifier::REVERSED), max_x);
    put(buf, cx, y, &after.chars().skip(1).collect::<String>(), style, max_x);
}

fn draw_footer(b: &Browser, buf: &mut Buffer, area: Rect) {
    let th = &b.theme;
    let right = area.x + area.width;
    let y1 = area.y + area.height - 2;
    let y2 = area.y + area.height - 1;
    fill(buf, Rect { x: area.x, y: y1, width: area.width, height: 1 }, st(th.fg, th.bg));
    fill(buf, Rect { x: area.x, y: y2, width: area.width, height: 1 }, st(th.dim, th.bar));
    let line = st(th.fg, th.bg);
    match &b.mode {
        Mode::Filter(input) => input_line(buf, area.x + 1, y1, "/ ", input, st(th.accent, th.bg), right),
        Mode::Rename { input, .. } => input_line(buf, area.x + 1, y1, "rename: ", input, st(th.accent, th.bg), right),
        Mode::ConfirmDelete { title, .. } => {
            let x = put(buf, area.x + 1, y1, &format!("Delete \"{}\"? ", fit(title, (area.width as usize).saturating_sub(24))), st(th.warn, th.bg).add_modifier(Modifier::BOLD), right);
            put(buf, x, y1, "(y/n)", line, right);
        }
        _ => {
            if let Some((kind, text)) = &b.status {
                let fg = match kind {
                    StatusKind::Info => th.dim,
                    StatusKind::Ok => th.ok,
                    StatusKind::Error => th.error,
                };
                put(buf, area.x + 1, y1, text, st(fg, th.bg), right);
            } else if let Some(r) = b.selected() {
                if let Some(n) = &r.note {
                    put(buf, area.x + 1, y1, &fit(n, area.width as usize - 2), st(th.dim, th.bg), right);
                }
            }
        }
    }
    let hints = match &b.mode {
        Mode::Filter(_) => "type to filter by title or branch   ↵ keep   esc clear",
        Mode::Rename { .. } => "↵ rename   esc cancel",
        Mode::ConfirmDelete { .. } => "y delete   any other key cancels",
        Mode::Help => "any key closes help",
        Mode::Normal => "↵ open  t tab  s star  r rename  d delete  / filter  a all repos  S starred  e mermaid  ? help  q quit",
    };
    put(buf, area.x + 1, y2, hints, st(th.dim, th.bar), right);
}

fn draw_help(b: &Browser, buf: &mut Buffer, body: Rect, right: u16) {
    let th = &b.theme;
    let lines = [
        ("j / k  ↓ / ↑", "move"),
        ("g / G", "first / last"),
        ("↵", "open in the full viewer (q there returns here)"),
        ("t", "open in a new tether tab (stay here)"),
        ("s", "star / unstar (starred diagrams are never pruned)"),
        ("r", "rename (the id stays)"),
        ("d", "delete, asks y/n"),
        ("/", "filter by title or branch (↵ keeps, esc clears)"),
        ("a", "toggle all repos"),
        ("S", "toggle starred only"),
        ("^d / ^u", "scroll the preview (< > pan sideways)"),
        ("e", "copy the Mermaid source to the clipboard"),
        ("?", "this help"),
        ("q / esc", "quit"),
    ];
    let w = 66u16.min(body.width.saturating_sub(2));
    let h = (lines.len() as u16 + 4).min(body.height);
    let r = Rect { x: body.x + body.width.saturating_sub(w) / 2, y: body.y + body.height.saturating_sub(h) / 2, width: w, height: h };
    fill(buf, r, st(th.fg, th.panel));
    put(buf, r.x + 2, r.y + 1, "tether diagrams", st(th.accent, th.panel).add_modifier(Modifier::BOLD), right.min(r.x + r.width));
    for (i, (k, d)) in lines.iter().enumerate() {
        let y = r.y + 3 + i as u16;
        if y >= r.y + r.height {
            break;
        }
        let x = put(buf, r.x + 2, y, &pad(k, 14), st(th.link, th.panel), r.x + r.width);
        put(buf, x, y, d, st(th.fg, th.panel), r.x + r.width - 1);
    }
}
