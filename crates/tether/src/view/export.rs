//! Static rendering: draw a diagram into a buffer without a terminal and convert the buffer to
//! plain text (`.txt`, golden tests) or ANSI truecolor (`.ans`, `cat` it in a terminal).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect as RRect;
use ratatui::style::{Color, Modifier};

use super::app::{App, FOOTER_ROWS, HEADER_ROWS, PAD_X, PAD_Y};
use super::draw::draw;

/// Size the terminal must have to show the whole canvas without scrolling.
pub fn full_size(app: &App, min_width: u16) -> (u16, u16) {
    let w = (app.layout.width + 2 * PAD_X).max(min_width);
    let h = app.layout.height + 2 * PAD_Y + HEADER_ROWS + FOOTER_ROWS;
    (w, h)
}

/// Render the whole canvas (resizing the app's terminal to fit it) into a fresh buffer.
pub fn render_full(app: &mut App, min_width: u16, resolve_links: bool) -> Buffer {
    let (w, h) = full_size(app, min_width);
    app.resize(w, h);
    app.scroll_x = 0;
    app.scroll_y = 0;
    if resolve_links {
        app.prepare_frame(std::time::Instant::now());
    }
    render(app)
}

/// Render the app at its current terminal size.
pub fn render(app: &App) -> Buffer {
    let area = RRect { x: 0, y: 0, width: app.term.0, height: app.term.1 };
    let mut buf = Buffer::empty(area);
    draw(app, &mut buf, area);
    buf
}

/// Glyphs only, one line per row, trailing spaces trimmed.
pub fn buffer_to_text(buf: &Buffer) -> String {
    let a = buf.area;
    let mut out = String::new();
    for y in a.y..a.y + a.height {
        let mut line = String::new();
        for x in a.x..a.x + a.width {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn sgr(c: Color, fg: bool) -> String {
    let base = if fg { 38 } else { 48 };
    match c {
        Color::Rgb(r, g, b) => format!("\x1b[{base};2;{r};{g};{b}m"),
        _ => format!("\x1b[{}m", if fg { 39 } else { 49 }),
    }
}

/// ANSI truecolor rendering (fills, bold/italic/underline) of the buffer.
pub fn buffer_to_ansi(buf: &Buffer) -> String {
    let a = buf.area;
    let mut out = String::new();
    for y in a.y..a.y + a.height {
        let mut cur: Option<(Color, Color, Modifier)> = None;
        for x in a.x..a.x + a.width {
            let c = &buf[(x, y)];
            let key = (c.fg, c.bg, c.modifier);
            if cur != Some(key) {
                out.push_str("\x1b[0m");
                out.push_str(&sgr(c.fg, true));
                out.push_str(&sgr(c.bg, false));
                if c.modifier.contains(Modifier::BOLD) {
                    out.push_str("\x1b[1m");
                }
                if c.modifier.contains(Modifier::ITALIC) {
                    out.push_str("\x1b[3m");
                }
                if c.modifier.contains(Modifier::UNDERLINED) {
                    out.push_str("\x1b[4m");
                }
                cur = Some(key);
            }
            let sym = c.symbol();
            out.push_str(if sym.is_empty() { "" } else { sym });
        }
        out.push_str("\x1b[0m\n");
    }
    out
}
