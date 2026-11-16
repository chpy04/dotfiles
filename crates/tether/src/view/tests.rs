//! Viewer tests: rendering (TestBackend buffers and golden files), navigation, expansion,
//! search, stale links, the comment flow and overlays. No terminal, no subprocesses.
//!
//! Goldens live in `crates/tether/tests/golden/`; regenerate with `UPDATE_GOLDEN=1 cargo test`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use tether_core::config::Config;
use tether_core::diagram::{validate, Diagram, Resolved};
use tether_core::layout::{edge_cells, ObjectId, ObjectKind, Point, Rect};
use tether_core::theme::{mocha, Rgb, Theme};
use tether_core::workspace::AgentRow;

use super::app::{App, Effect, Mode, Msg, StatusKind, HEADER_ROWS, PAD_Y};
use super::draw::{draw, DIM_FAR, DIM_NEAR};
use super::export::{buffer_to_ansi, buffer_to_text, render, render_full};
use super::icons::Icons;
use super::{build_app, RenderOpts};

// ------------------------------------------------------------------------------------ helpers

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tether-core/tests/fixtures/diagrams")
}

fn fixture(name: &str) -> Diagram {
    let src = std::fs::read_to_string(fixtures().join(format!("{name}.json"))).unwrap();
    validate(&src).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn app_for(d: Diagram, worktree: &Path, icons: bool, term: (u16, u16)) -> App {
    build_app(d, "dg-1", worktree.to_path_buf(), &Config::default(), Some(icons), term).unwrap()
}

/// A worktree holding the source files the fixtures link to.
fn worktree() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let put = |path: &str, special: &[(usize, &str)]| {
        let full = d.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        let body: Vec<String> = (1..=200)
            .map(|n| special.iter().find(|(l, _)| *l == n).map(|(_, t)| t.to_string()).unwrap_or_else(|| format!("// line {n}")))
            .collect();
        std::fs::write(full, body.join("\n")).unwrap();
    };
    put("src/auth/session.ts", &[(142, "if (token.expiresAt < Date.now()) {")]);
    put("src/auth/refresh.ts", &[]);
    put("src/api/login.ts", &[(14, "router.post('/login', login)")]);
    put("src/auth/verify.ts", &[(31, "export async function verify(email, pw) {")]);
    d
}

fn key_event(tok: &str) -> KeyEvent {
    let none = KeyModifiers::NONE;
    match tok {
        "<cr>" => KeyEvent::new(KeyCode::Enter, none),
        "<tab>" => KeyEvent::new(KeyCode::Tab, none),
        "<esc>" => KeyEvent::new(KeyCode::Esc, none),
        "<bs>" => KeyEvent::new(KeyCode::Backspace, none),
        "<up>" => KeyEvent::new(KeyCode::Up, none),
        "<down>" => KeyEvent::new(KeyCode::Down, none),
        "<c-j>" => KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
        "<c-d>" => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        "<c-u>" => KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        "<s-cr>" => KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        "<space>" => KeyEvent::new(KeyCode::Char(' '), none),
        t if t.chars().count() == 1 => KeyEvent::new(KeyCode::Char(t.chars().next().unwrap()), none),
        t => panic!("bad key token {t}"),
    }
}

fn tokens(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = script;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            if let Some(end) = rest.find('>') {
                out.push(rest[..=end].to_string());
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push(c.to_string());
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Feed a key script, collecting all effects.
fn keys(app: &mut App, script: &str) -> Vec<Effect> {
    let mut all = Vec::new();
    for t in tokens(script) {
        all.extend(app.handle_key(key_event(&t)));
    }
    all
}

fn focus(app: &App) -> &str {
    app.focus.as_str()
}

fn id(s: &str) -> ObjectId {
    ObjectId::from(s)
}

fn golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden {name} (UPDATE_GOLDEN=1)"));
    assert_eq!(expected, actual, "golden {name} differs\n--- actual ---\n{actual}");
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("not a truecolor color: {other:?}"),
    }
}

fn r3(c: Rgb) -> (u8, u8, u8) {
    (c.0, c.1, c.2)
}

/// Render the whole canvas of an app.
fn full(app: &mut App) -> Buffer {
    render_full(app, 60, true)
}

/// Screen position of a canvas cell in the current frame.
fn screen_of(app: &App, x: u16, y: u16) -> (u16, u16) {
    let (ox, oy) = app.origin();
    ((x as i32 + ox) as u16, (y as i32 + oy) as u16 + HEADER_ROWS)
}

// ------------------------------------------------------------------------------------ render: goldens

#[test]
fn golden_prd_flowchart_with_check_focused() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    app.set_focus(id("check"));
    let text = buffer_to_text(&full(&mut app));
    golden("prd_flowchart_check.txt", &text);
    assert!(text.contains("┏━") && text.contains("┗━"));
}

#[test]
fn golden_30_node_flowchart() {
    let wt = worktree();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    app.set_focus(id("check"));
    let text = buffer_to_text(&full(&mut app));
    golden("session_refresh_30_check.txt", &text);
}

#[test]
fn golden_login_sequence_with_m2_expanded() {
    let wt = worktree();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (100, 30));
    app.set_focus(id("m2"));
    app.set_expanded(&[id("m2")]).unwrap();
    let text = buffer_to_text(&full(&mut app));
    golden("login_sequence_m2_expanded.txt", &text);
    assert!(text.contains("DATA MODEL") && text.contains("src/auth/verify.ts:31"));
}

#[test]
fn render_through_a_real_test_backend_terminal() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), true, (90, 24));
    app.prepare_frame(Instant::now());
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            draw(&app, f.buffer_mut(), area)
        })
        .unwrap();
    let text = buffer_to_text(terminal.backend().buffer());
    assert!(text.contains("Session refresh") && text.contains("Token expired?") && text.contains("Call /token/refresh"));
    assert!(text.contains("hjkl move") && text.contains("␣ac ask"), "{text}");
}

// ------------------------------------------------------------------------------------ render: style

#[test]
fn focused_node_has_heavy_border_and_accent_fill_others_dim() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    app.set_focus(id("check"));
    let buf = full(&mut app);

    let r = app.layout.object("check").unwrap().rect;
    let sym = |x: u16, y: u16| {
        let (sx, sy) = screen_of(&app, x, y);
        buf[(sx, sy)].symbol().to_string()
    };
    assert_eq!(sym(r.x, r.y), "┏");
    assert_eq!(sym(r.right() - 1, r.y), "┓");
    assert_eq!(sym(r.x, r.bottom() - 1), "┗");
    assert_eq!(sym(r.right() - 1, r.bottom() - 1), "┛");
    assert_eq!(sym(r.x + 3, r.y), "━");
    assert_eq!(sym(r.x, r.y + 1), "┃");
    // accent fill under the label, accent_fg text, focus border color
    let lr = app.layout.object("check").unwrap().label_rect.unwrap();
    let (lx, ly) = screen_of(&app, lr.x, lr.y);
    assert_eq!(rgb(buf[(lx, ly)].bg), r3(theme.accent));
    assert_eq!(rgb(buf[(lx, ly)].fg), r3(theme.accent_fg));
    assert!(buf[(lx, ly)].modifier.contains(Modifier::BOLD), "labels are bold");
    let (bx, by) = screen_of(&app, r.x + 3, r.y);
    assert_eq!(rgb(buf[(bx, by)].fg), r3(theme.focus_border));

    // every other node uses rounded borders and is dimmed toward the background
    let nav = app.layout.flow_nav().unwrap().clone();
    let near: HashSet<&ObjectId> = nav.children[&id("check")].iter().chain(nav.parents[&id("check")].iter()).collect();
    let (mut far_seen, mut near_seen) = (0, 0);
    for o in &app.layout.objects {
        let ObjectKind::Node { shape } = &o.kind else { continue };
        if o.id == id("check") {
            continue;
        }
        let (x, y) = screen_of(&app, o.rect.x, o.rect.y);
        assert_eq!(buf[(x, y)].symbol(), "╭", "{} has a rounded border", o.id);
        let hue = theme.shape(*shape).border;
        let (hx, hy) = screen_of(&app, o.rect.x + 2, o.rect.y);
        let got = rgb(buf[(hx, hy)].fg);
        if near.contains(&o.id) {
            near_seen += 1;
            assert_eq!(got, r3(hue.mix(theme.bg, DIM_NEAR)), "{} near-dimmed", o.id);
        } else {
            far_seen += 1;
            assert_eq!(got, r3(hue.mix(theme.bg, DIM_FAR)), "{} far-dimmed", o.id);
            assert_ne!(got, r3(hue), "{} must differ from the undimmed color", o.id);
        }
        let (ix, iy) = screen_of(&app, o.rect.x + 1, o.rect.y + 1);
        assert!(matches!(buf[(ix, iy)].bg, Color::Rgb(..)), "true-color fill");
    }
    assert!(far_seen > 20 && near_seen >= 2, "far {far_seen} near {near_seen}");
    // fills differ per shape type
    let fill_of = |node: &str| {
        let o = app.layout.object(node).unwrap();
        let (x, y) = screen_of(&app, o.rect.x + 1, o.rect.y + 1);
        rgb(buf[(x, y)].bg)
    };
    assert_ne!(fill_of("anon"), fill_of("start"));
}

#[test]
fn incident_edges_are_accent_and_the_chosen_branch_is_bold() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    app.set_focus(id("check"));
    keys(&mut app, "l"); // pick the second branch
    let buf = full(&mut app);
    let e0 = app.layout.edge("edge:0").unwrap().clone(); // check -> refresh (not chosen)
    let e1 = app.layout.edge("edge:1").unwrap().clone(); // check -> ok (chosen)
    let at = |p: &Point| {
        let (x, y) = screen_of(&app, p.x, p.y);
        buf[(x, y)].clone()
    };
    let c1 = at(&e1.points[e1.points.len() / 2]);
    assert_eq!(rgb(c1.fg), r3(theme.accent));
    assert!(c1.modifier.contains(Modifier::BOLD));
    let c0 = at(&e0.points[e0.points.len() - 1]);
    assert_eq!(rgb(c0.fg), r3(theme.accent.mix(theme.edge, 0.45)), "other incident edge: softer accent");
    assert!(!c0.modifier.contains(Modifier::BOLD));
}

#[test]
fn no_edge_glyph_inside_any_node_or_panel_and_edges_use_box_drawing() {
    let wt = worktree();
    for (name, expand) in [("session_refresh_30", vec!["check", "lock"]), ("prd_flowchart", vec!["check"])] {
        let mut app = app_for(fixture(name), wt.path(), false, (100, 30));
        let ids: Vec<ObjectId> = expand.iter().map(|s| id(s)).collect();
        app.set_expanded(&ids).unwrap();
        app.set_focus(id("check"));
        let buf = full(&mut app);
        let masks = edge_cells(&app.layout);
        assert!(!masks.is_empty());
        let boxes: Vec<(ObjectId, Rect)> = app
            .layout
            .objects
            .iter()
            .filter(|o| matches!(o.kind, ObjectKind::Node { .. } | ObjectKind::Details))
            .map(|o| (o.id.clone(), o.rect))
            .collect();
        let line_glyphs = ["─", "│", "╭", "╮", "╰", "╯", "├", "┤", "┬", "┴", "┼", "┄", "┆", "▼", "▲", "◀", "▶"];
        for &(x, y) in masks.keys() {
            for (bid, r) in &boxes {
                assert!(!r.contains(Point { x, y }), "{name}: edge cell ({x},{y}) lies inside {bid}");
            }
            let (sx, sy) = screen_of(&app, x, y);
            let g = buf[(sx, sy)].symbol();
            assert!(line_glyphs.contains(&g), "{name}: edge cell ({x},{y}) drew {g:?}");
        }
        // and nothing but the node's own border/label/decor is drawn inside a node rect
        for (bid, r) in &boxes {
            for y in r.y + 1..r.bottom() - 1 {
                for x in r.x + 1..r.right() - 1 {
                    let (sx, sy) = screen_of(&app, x, y);
                    let g = buf[(sx, sy)].symbol();
                    let is_bar = g == "│" && (x == r.x + 1 || x == r.right() - 2); // subroutine side bars
                    assert!(!["╭", "╮", "╰", "╯", "├", "┤", "┬", "┴", "┼", "▼", "▲", "◀", "▶", "┄", "┆"].contains(&g) && (g != "│" || is_bar), "{name}: {g:?} inside {bid} at ({x},{y})");
                }
            }
        }
    }
}

#[test]
fn flow_edge_junctions_dashes_and_arrowheads_are_drawn() {
    let wt = worktree();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    let text = buffer_to_text(&full(&mut app));
    for glyph in ["╭", "╮", "╰", "╯", "│", "─", "▼", "┄", "┆"] {
        assert!(text.contains(glyph), "missing {glyph}");
    }
    assert!(["├", "┤", "┬", "┴", "┼"].iter().any(|g| text.contains(g)), "a T or cross junction somewhere in 30 nodes");
}

#[test]
fn nerd_icons_and_the_plain_fallback() {
    let wt = worktree();
    let mut nerd = app_for(fixture("prd_flowchart"), wt.path(), true, (100, 30));
    let t = buffer_to_text(&full(&mut nerd));
    assert!(t.contains('\u{f059}'), "decision icon");
    assert!(t.contains('\u{f121}'), "code marker");
    let mut plain = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let t = buffer_to_text(&full(&mut plain));
    assert!(t.contains('◆') && t.contains('§'));
    assert!(!t.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)), "no private-use glyphs in the fallback");
    assert_eq!(Icons::resolve(true, Some("0")), Icons::PLAIN);
}

#[test]
fn stale_link_gets_a_red_chip_badge() {
    let wt = worktree();
    std::fs::write(wt.path().join("src/auth/session.ts"), "nothing relevant here\n").unwrap();
    let theme = Theme::default();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let buf = full(&mut app);
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Stale));
    assert_eq!(app.link_state(&id("refresh")), Some(Resolved::Exact(18)));
    let c = app.layout.object("check").unwrap().decor.code.unwrap();
    let (x, y) = screen_of(&app, c.x, c.y);
    assert_eq!(buf[(x, y)].symbol(), "!");
    assert_eq!(rgb(buf[(x, y)].bg), r3(theme.error));
}

#[test]
fn sequence_rendering_participants_lifeline_breaks_and_focus() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (100, 30));
    app.set_focus(id("m2"));
    app.set_expanded(&[id("m2")]).unwrap();
    let buf = full(&mut app);
    // participants: rounded boxes with kind fills
    let browser = app.layout.object("browser").unwrap().rect;
    let db = app.layout.object("db").unwrap().rect;
    let (bx, by) = screen_of(&app, browser.x, browser.y);
    assert_eq!(buf[(bx, by)].symbol(), "╭");
    let fill = |r: Rect| {
        let (x, y) = screen_of(&app, r.x + 1, r.y + 1);
        rgb(buf[(x, y)].bg)
    };
    assert_ne!(fill(browser), fill(db));
    // focused message: accent arrow
    let e = app.layout.edge("m2").unwrap().clone();
    let mid = (e.points[0].x + e.points[1].x) / 2;
    let (mx, my) = screen_of(&app, mid, e.points[0].y);
    assert_eq!(rgb(buf[(mx, my)].fg), r3(theme.accent));
    assert_eq!(buf[(mx, my)].symbol(), "─");
    let (ax, ay) = screen_of(&app, e.points[1].x, e.points[1].y);
    assert_eq!(buf[(ax, ay)].symbol(), "▶", "sync arrowhead");
    // dashed return arrow
    let ret = app.layout.edge("db_ret").unwrap().clone();
    let (rx, ry) = screen_of(&app, (ret.points[0].x + ret.points[1].x) / 2, ret.points[0].y);
    assert_eq!(buf[(rx, ry)].symbol(), "┄");
    let (hx, hy) = screen_of(&app, ret.points[1].x, ret.points[1].y);
    assert_eq!(buf[(hx, hy)].symbol(), "◁", "return arrows are dashed with an open head");
    // the lifeline is interrupted under the details panel
    let panel = app.layout.details_of("m2").unwrap().rect;
    let ll = app.layout.lifelines.iter().find(|l| l.participant == id("api")).unwrap();
    assert!(panel.x <= ll.x && ll.x < panel.right());
    let (lx, ly) = screen_of(&app, ll.x, panel.y + 2);
    assert_ne!(buf[(lx, ly)].symbol(), "┆", "lifeline broken inside the panel");
    let (lx, ly) = screen_of(&app, ll.x, panel.bottom() + 1);
    assert_eq!(buf[(lx, ly)].symbol(), "┆");
    // `⏎ code` hint on the focused message
    assert!(buffer_to_text(&buf).contains("⏎ code"));
    // group frame has a kind tab: " alt " in accent_fg on the group color
    let g = app.layout.object("group:1").unwrap();
    let lr = g.label_rect.unwrap();
    let (gx, gy) = screen_of(&app, lr.x, lr.y);
    assert_eq!(rgb(buf[(gx, gy)].fg), r3(theme.accent_fg));
}

#[test]
fn ansi_export_contains_truecolor_sequences() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let ansi = buffer_to_ansi(&full(&mut app));
    assert!(ansi.contains("\x1b[38;2;") && ansi.contains("\x1b[48;2;"), "truecolor fg/bg");
    assert!(ansi.contains("\x1b[1m"), "bold");
    let (text, ansi2) = super::render_static(
        fixture("prd_flowchart"),
        wt.path().to_path_buf(),
        &Config::default(),
        &RenderOpts { focus: Some("check".into()), expand: vec!["check".into()], icons: Some(false), min_width: 60, resolve_links: true },
    )
    .unwrap();
    assert!(text.contains("EDGE CASE"), "{text}");
    assert!(ansi2.contains("\x1b[48;2;137;180;250m"), "accent fill present");
}

// ------------------------------------------------------------------------------------ navigation: flowchart

#[test]
fn prd_example_j_l_k() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    assert_eq!(focus(&app), "check", "starts at the root");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "refresh", "j follows the first outgoing edge");
    keys(&mut app, "l");
    assert_eq!(focus(&app), "ok", "l switches to the sibling branch");
    keys(&mut app, "k");
    assert_eq!(focus(&app), "check", "k goes back to the parent");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "ok", "j follows the last chosen branch");
    keys(&mut app, "h");
    assert_eq!(focus(&app), "refresh");
    keys(&mut app, "kk");
    assert_eq!(focus(&app), "check");
    assert!(app.status.is_some(), "k at the root explains itself");
}

#[test]
fn at_a_decision_h_l_pick_the_branch_and_j_follows_it() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "l");
    assert_eq!(focus(&app), "check", "l at a decision picks, it does not move");
    assert!(app.status.as_ref().unwrap().text.contains("no → Continue request"), "{:?}", app.status);
    keys(&mut app, "j");
    assert_eq!(focus(&app), "ok");
    keys(&mut app, "kh");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "refresh");
}

#[test]
fn arrows_g_and_capital_g() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "<down>");
    assert_eq!(focus(&app), "refresh");
    keys(&mut app, "<up>");
    assert_eq!(focus(&app), "check");
    keys(&mut app, "G");
    let last = app.layout.flow_nav().unwrap().order.last().unwrap().to_string();
    assert_eq!(focus(&app), last);
    keys(&mut app, "g");
    assert_eq!(focus(&app), "check");
}

#[test]
fn a_scripted_key_sequence_reaches_every_node_of_the_30_node_flowchart() {
    let wt = worktree();
    let start = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    let all: Vec<String> = start.layout.flow_nav().unwrap().order.iter().map(|o| o.to_string()).collect();
    assert_eq!(all.len(), 30);
    // BFS over key presses (state = focus + remembered branches) finds a script per node
    let state_key = |a: &App| {
        let mut b: Vec<(String, String)> = a.branch.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        b.sort();
        (a.focus.to_string(), b)
    };
    let mut seen: HashMap<String, String> = HashMap::new();
    seen.insert(focus(&start).to_string(), String::new());
    let mut visited = HashSet::new();
    visited.insert(state_key(&start));
    let mut q: VecDeque<(App, String)> = VecDeque::new();
    q.push_back((start.clone(), String::new()));
    'bfs: while let Some((app, script)) = q.pop_front() {
        for k in ['j', 'k', 'h', 'l'] {
            let mut next = app.clone();
            keys(&mut next, &k.to_string());
            if !visited.insert(state_key(&next)) {
                continue;
            }
            let s = format!("{script}{k}");
            seen.entry(focus(&next).to_string()).or_insert_with(|| s.clone());
            if seen.len() == all.len() {
                break 'bfs;
            }
            q.push_back((next, s));
        }
    }
    let missing: Vec<&String> = all.iter().filter(|n| !seen.contains_key(*n)).collect();
    assert!(missing.is_empty(), "unreachable by hjkl: {missing:?}");
    // replay: each recorded script, typed into a fresh viewer, lands on its node
    for (node, script) in &seen {
        let mut app = start.clone();
        keys(&mut app, script);
        assert_eq!(focus(&app), node, "script {script:?}");
    }
}

#[test]
fn k_returns_to_the_parent_we_came_from() {
    let wt = worktree();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    let mut path = vec![focus(&app).to_string()];
    for _ in 0..8 {
        keys(&mut app, "j");
        if path.last().map(String::as_str) != Some(focus(&app)) {
            path.push(focus(&app).to_string());
        }
    }
    assert!(path.len() >= 4, "{path:?}");
    for expect in path.iter().rev().skip(1) {
        keys(&mut app, "k");
        assert_eq!(focus(&app), expect);
    }
}

// ------------------------------------------------------------------------------------ navigation: sequence

#[test]
fn sequence_j_k_h_l() {
    let wt = worktree();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (100, 30));
    assert_eq!(focus(&app), "m1");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "m2");
    keys(&mut app, "jj");
    assert_eq!(focus(&app), "db_ret");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "n1", "notes are focusable rows");
    keys(&mut app, "kk");
    assert_eq!(focus(&app), "m3");
    // h/l: step onto the message's lifeline, then walk participants
    keys(&mut app, "k"); // m2: api -> auth
    keys(&mut app, "l");
    assert_eq!(focus(&app), "auth", "l = right-most participant of the message");
    keys(&mut app, "l");
    assert_eq!(focus(&app), "db");
    keys(&mut app, "l");
    assert_eq!(focus(&app), "db", "no participant further right");
    keys(&mut app, "hh");
    assert_eq!(focus(&app), "api");
    keys(&mut app, "h");
    assert_eq!(focus(&app), "browser");
    // j on a lifeline walks that participant's items after the last visited one (m2)
    keys(&mut app, "ll");
    assert_eq!(focus(&app), "auth");
    keys(&mut app, "j");
    assert_eq!(focus(&app), "m3");
    keys(&mut app, "gk");
    assert_eq!(focus(&app), "m1");
    keys(&mut app, "G");
    assert_eq!(app.layout.seq_nav().unwrap().items.last().unwrap().as_str(), focus(&app));
    keys(&mut app, "j");
    assert_eq!(app.status.as_ref().unwrap().text, "last message");
}

// ------------------------------------------------------------------------------------ expansion

#[test]
fn tab_expands_relayouts_and_later_objects_move() {
    let wt = worktree();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (100, 40));
    keys(&mut app, "j"); // m2
    let before = app.layout.object("m3").unwrap().rect.y;
    let h_before = app.layout.height;
    keys(&mut app, "<tab>");
    assert!(app.expanded.contains(&id("m2")));
    let panel = app.layout.details_of("m2").expect("details panel appears").rect;
    let after = app.layout.object("m3").unwrap().rect.y;
    assert_eq!(after - before, panel.h, "later rows shift down by exactly the panel height");
    assert!(app.layout.height > h_before);
    keys(&mut app, "<tab>");
    assert!(!app.expanded.contains(&id("m2")));
    assert_eq!(app.layout.object("m3").unwrap().rect.y, before, "collapse restores the layout");
    // an element without details says so
    keys(&mut app, "g<tab>");
    assert!(app.status.as_ref().unwrap().text.contains("no details"));
}

#[test]
fn flowchart_expansion_adds_a_panel_beside_the_node() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    assert!(app.layout.details_of("check").is_none());
    keys(&mut app, "<tab>");
    let n = app.layout.object("check").unwrap().rect;
    let p = app.layout.details_of("check").expect("panel").rect;
    assert!(!n.intersects(&p));
    let text = buffer_to_text(&full(&mut app));
    assert!(text.contains("EDGE CASE") && text.contains("Uses client clock"), "{text}");
}

// ------------------------------------------------------------------------------------ search

#[test]
fn incremental_search_n_and_escape() {
    let wt = worktree();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 30));
    let origin = focus(&app).to_string();
    keys(&mut app, "/");
    assert!(matches!(app.mode, Mode::Search(_)));
    keys(&mut app, "tok");
    assert!(app.matches.len() >= 2, "{:?}", app.matches);
    assert_eq!(focus(&app), app.matches[0].as_str(), "incremental: focus is already on the first match");
    keys(&mut app, "<bs><bs><bs>");
    assert!(app.matches.is_empty());
    assert_eq!(focus(&app), origin, "an empty query goes back to where the search began");
    keys(&mut app, "refresh");
    let first = focus(&app).to_string();
    keys(&mut app, "<cr>");
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(focus(&app), first);
    assert!(app.status.as_ref().unwrap().text.contains("match 1/"), "{:?}", app.status);
    keys(&mut app, "n");
    assert_ne!(focus(&app), first);
    keys(&mut app, "N");
    assert_eq!(focus(&app), first);
    keys(&mut app, "N");
    assert_eq!(focus(&app), app.matches.last().unwrap().as_str(), "N wraps around");
    keys(&mut app, "g");
    let here = focus(&app).to_string();
    keys(&mut app, "/TOKEN<esc>");
    assert_eq!(focus(&app), here, "esc restores the focus");
    assert!(app.matches.is_empty());
    keys(&mut app, "/zzzz<cr>");
    assert!(app.status.as_ref().unwrap().text.contains("no match"));
    keys(&mut app, "<esc>n");
    assert!(app.status.as_ref().unwrap().text.contains("press /"));
}

#[test]
fn search_finds_sequence_labels_including_notes_and_participants() {
    let wt = worktree();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (100, 30));
    keys(&mut app, "/bcrypt<cr>");
    assert_eq!(focus(&app), "n1");
    keys(&mut app, "/auth<cr>");
    assert!(app.matches.iter().any(|m| m.as_str() == "auth"));
}

// ------------------------------------------------------------------------------------ code links

#[test]
fn enter_opens_exact_and_moved_links_and_refuses_stale_ones() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    app.pane_id = Some("w9:p3".into());

    // exact
    let fx = keys(&mut app, "<cr>");
    assert_eq!(fx.len(), 1);
    let Effect::OpenCode { abs_path, rel_path, line, from_pane } = &fx[0] else { panic!("{fx:?}") };
    assert_eq!((rel_path.as_str(), *line, from_pane.as_deref()), ("src/auth/session.ts", 142, Some("w9:p3")));
    assert_eq!(abs_path, &wt.path().join("src/auth/session.ts"));
    assert_eq!(
        fx[0].argv().unwrap(),
        vec![
            "code".to_string(),
            "open".into(),
            format!("{}:142", wt.path().join("src/auth/session.ts").display()),
            "--from".into(),
            "w9:p3".into()
        ]
    );

    // moved: put 7 extra lines on top; the anchor line is now 149
    let src = std::fs::read_to_string(wt.path().join("src/auth/session.ts")).unwrap();
    std::fs::write(wt.path().join("src/auth/session.ts"), format!("{}{src}", "// new\n".repeat(7))).unwrap();
    let fx = keys(&mut app, "<cr>");
    let Effect::OpenCode { line, .. } = &fx[0] else { panic!("{fx:?}") };
    assert_eq!(*line, 149);
    let s = app.status.as_ref().unwrap();
    assert_eq!(s.kind, StatusKind::Warn);
    assert!(s.text.contains("moved from L142 → L149"), "{}", s.text);
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Moved(149)));

    // stale: the anchor is gone -> no effect, error status
    std::fs::write(wt.path().join("src/auth/session.ts"), "gone\n").unwrap();
    let fx = keys(&mut app, "<cr>");
    assert!(fx.is_empty(), "stale links must not jump: {fx:?}");
    let s = app.status.as_ref().unwrap();
    assert_eq!(s.kind, StatusKind::Error);
    assert!(s.text.contains("stale link") && s.text.contains("not jumping"), "{}", s.text);
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Stale));

    // missing file
    std::fs::remove_file(wt.path().join("src/auth/session.ts")).unwrap();
    assert!(keys(&mut app, "<cr>").is_empty());
    assert!(app.status.as_ref().unwrap().text.contains("cannot read src/auth/session.ts"));

    // no code link on the node
    keys(&mut app, "jl");
    assert_eq!(focus(&app), "ok");
    assert!(keys(&mut app, "<cr>").is_empty());
    assert!(app.status.as_ref().unwrap().text.contains("no code link"));
}

#[test]
fn links_are_resolved_lazily_on_draw_and_cached() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    assert!(app.links.is_empty(), "nothing resolved before the first frame");
    let now = Instant::now();
    app.prepare_frame(now);
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Exact(142)));
    assert_eq!(app.link_state(&id("refresh")), Some(Resolved::Exact(18)));
    assert!(app.link_state(&id("ok")).is_none(), "no code link, nothing to resolve");
    // cached: a changed file is not re-read until the TTL passes
    std::fs::write(wt.path().join("src/auth/session.ts"), "x\n").unwrap();
    app.prepare_frame(now + Duration::from_secs(1));
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Exact(142)));
    app.prepare_frame(now + Duration::from_secs(6));
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Stale));
}

// ------------------------------------------------------------------------------------ comments

fn agent(pane: &str, tab: u32, kind: &str, state: &str, primary: bool) -> AgentRow {
    AgentRow {
        tab_index: tab,
        tab_label: tab.to_string(),
        tab_id: format!("w1:t{tab}"),
        workspace_id: "w1".into(),
        pane_id: pane.into(),
        kind: kind.into(),
        state: state.into(),
        name: None,
        is_primary: primary,
    }
}

#[test]
fn comment_flow_produces_the_send_effect() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "j"); // refresh
    assert!(keys(&mut app, "<space>ac").is_empty());
    let Mode::Comment(c) = &app.mode else { panic!("comment float should be open") };
    assert_eq!((c.to.as_str(), c.node.as_str()), ("primary", "refresh"));
    keys(&mut app, "Why POST?");
    keys(&mut app, "<c-j>");
    keys(&mut app, "second line");
    let fx = keys(&mut app, "<cr>");
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(fx.len(), 1);
    assert_eq!(
        fx[0].argv().unwrap(),
        ["send", "--to", "primary", "--diagram", "dg-1", "--node", "refresh", "--text-file", "-"].map(String::from).to_vec()
    );
    assert_eq!(fx[0].stdin(), Some("Why POST?\nsecond line"));
    assert!(app.status.as_ref().unwrap().text.starts_with("sending comment on \"Call /token/refresh\""));
}

#[test]
fn comment_float_edge_cases() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "<space>ac");
    keys(&mut app, "hello");
    assert!(keys(&mut app, "<esc>").is_empty(), "esc cancels, nothing is sent");
    assert!(matches!(app.mode, Mode::Normal));
    keys(&mut app, "<space>ac");
    assert!(keys(&mut app, "<cr>").is_empty());
    assert!(matches!(app.mode, Mode::Comment(_)), "an empty comment keeps the float open");
    assert_eq!(app.status.as_ref().unwrap().kind, StatusKind::Error);
    keys(&mut app, "/clear");
    assert!(keys(&mut app, "<cr>").is_empty());
    assert!(matches!(app.mode, Mode::Comment(_)));
    assert!(app.status.as_ref().unwrap().text.contains("must not start with `/`"));
    keys(&mut app, "<esc><space>ac");
    keys(&mut app, "a<s-cr>b");
    let fx = keys(&mut app, "<cr>");
    assert_eq!(fx[0].stdin(), Some("a\nb"), "shift-enter adds a newline");
    // a failed send brings the draft back when the comment is reopened on the same node
    app.on_msg(Msg::SendDone { result: Err("agent_blocked".into()), node: id("check"), to: "primary".into(), text: "a\nb".into() });
    assert_eq!(app.status.as_ref().unwrap().kind, StatusKind::Error);
    keys(&mut app, "<space>ac");
    let Mode::Comment(c) = &app.mode else { panic!() };
    assert_eq!(c.input.text(), "a\nb");
    app.on_msg(Msg::SendDone { result: Ok("sent to claude · tab 1".into()), node: id("check"), to: "primary".into(), text: String::new() });
    assert_eq!(app.status.as_ref().unwrap().kind, StatusKind::Ok);
}

#[test]
fn leader_sequences_time_out_after_a_second() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let t0 = Instant::now();
    app.handle_key_at(key_event("<space>"), t0);
    assert_eq!(app.pending_hint().as_deref(), Some("␣"));
    app.handle_key_at(key_event("a"), t0 + Duration::from_millis(300));
    assert_eq!(app.pending_hint().as_deref(), Some("␣a"));
    let mut quick = app.clone();
    quick.handle_key_at(key_event("c"), t0 + Duration::from_millis(900));
    assert!(matches!(quick.mode, Mode::Comment(_)), "within the window the sequence completes");
    let mut ticked = app.clone();
    assert!(ticked.tick(t0 + Duration::from_millis(1500)));
    assert!(ticked.pending_hint().is_none(), "tick drops stale pending keys");
    app.handle_key_at(key_event("c"), t0 + Duration::from_millis(1500));
    assert!(matches!(app.mode, Mode::Normal), "a late `c` is not the end of <leader>ac");
    let mut app2 = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    app2.handle_key_at(key_event("<space>"), t0);
    app2.handle_key_at(key_event("j"), t0 + Duration::from_millis(100));
    assert_eq!(focus(&app2), "refresh", "an unrelated key abandons the sequence and acts alone");
}

#[test]
fn picker_shows_agents_with_state_colors_and_sends_to_the_pick() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let fx = keys(&mut app, "<space>aC");
    assert_eq!(fx, vec![Effect::FetchAgents]);
    assert_eq!(fx[0].argv().unwrap(), ["agents", "--json"].map(String::from).to_vec());
    assert!(buffer_to_text(&render(&app)).contains("loading agents…"));
    app.on_msg(Msg::Agents(Ok(vec![
        agent("w1:p2", 1, "claude", "idle", true),
        agent("w1:p5", 3, "claude", "working", false),
        agent("w1:p7", 5, "codex", "blocked", false),
    ])));
    let buf = render(&app);
    let text = buffer_to_text(&buf);
    assert!(text.contains("Send comment to"), "{text}");
    assert!(
        text.contains("tab 1 · claude — idle") && text.contains("tab 3 · claude — working") && text.contains("tab 5 · codex — blocked"),
        "{text}"
    );
    let color_of = |needle: &str| {
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
            if let Some(i) = line.find(needle) {
                let col = line[..i].chars().count() as u16;
                return rgb(buf[(col, y)].fg);
            }
        }
        panic!("{needle} not found");
    };
    assert_eq!(color_of("idle"), r3(theme.ok));
    assert_eq!(color_of("working"), r3(mocha::YELLOW));
    assert_eq!(color_of("blocked"), r3(theme.error));
    keys(&mut app, "jj<cr>");
    let Mode::Comment(c) = &app.mode else { panic!("{:?}", app.mode) };
    assert_eq!((c.to.as_str(), c.title.as_str()), ("w1:p7", "→ codex · tab 5"));
    keys(&mut app, "hi");
    let fx = keys(&mut app, "<cr>");
    assert_eq!(
        fx[0].argv().unwrap(),
        ["send", "--to", "w1:p7", "--diagram", "dg-1", "--node", "check", "--text-file", "-"].map(String::from).to_vec()
    );
}

#[test]
fn picker_cancel_and_agent_errors() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "<space>aC");
    app.on_msg(Msg::Agents(Err("not inside Herdr".into())));
    let text = buffer_to_text(&render(&app));
    assert!(text.contains("not inside Herdr"), "{text}");
    assert!(keys(&mut app, "<esc>").is_empty());
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn comment_float_is_anchored_under_the_focus_and_titled_with_the_primary_agent() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (100, 40));
    app.on_msg(Msg::Agents(Ok(vec![agent("w1:p2", 1, "claude", "idle", true)])));
    keys(&mut app, "jj");
    keys(&mut app, "<space>ac");
    keys(&mut app, "typed text");
    let buf = render(&app);
    let text = buffer_to_text(&buf);
    assert!(text.contains("→ claude · tab 1"), "{text}");
    assert!(text.contains("⏎ send · ^j newline · esc cancel"), "{text}");
    assert!(text.contains("typed text"));
    let o = app.layout.object(focus(&app)).unwrap().rect;
    let (nx, ny) = screen_of(&app, o.x, o.bottom() - 1);
    let panel = Color::Rgb(theme.panel.0, theme.panel.1, theme.panel.2);
    let mut corner = None;
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if buf[(x, y)].symbol() == "╭" && rgb(buf[(x, y)].fg) == r3(theme.accent) && buf[(x, y)].bg == panel {
                corner = Some((x, y));
            }
        }
    }
    let (cx, cy) = corner.expect("float corner");
    assert_eq!(cy, ny + 1, "directly under the focused object");
    assert!(cx.abs_diff(nx) <= o.w, "starts at the object's left edge");
}

// ------------------------------------------------------------------------------------ viewport, mouse, misc

#[test]
fn viewport_follows_focus_and_scrolls() {
    let wt = worktree();
    let mut app = app_for(fixture("session_refresh_30"), wt.path(), false, (80, 24));
    assert_eq!(app.scroll_y, 0);
    keys(&mut app, "G");
    assert!(app.scroll_y > 0, "scrolled down to the last node");
    let r = app.layout.object(focus(&app)).unwrap().rect;
    let (_, bh) = app.body_size();
    let top = app.scroll_y;
    assert!(r.y + PAD_Y >= top && r.bottom() + PAD_Y <= top + bh, "focus rect {r:?} inside the viewport at scroll {top}");
    keys(&mut app, "g");
    assert_eq!(app.scroll_y, 0);
    let f = focus(&app).to_string();
    keys(&mut app, "<c-d>");
    assert!(app.scroll_y > 0);
    assert_eq!(focus(&app), f, "ctrl-d scrolls the viewport only");
    keys(&mut app, "<c-u>");
    assert_eq!(app.scroll_y, 0);
    let wheel = |kind| MouseEvent { kind, column: 5, row: 5, modifiers: KeyModifiers::NONE };
    app.handle_mouse(wheel(MouseEventKind::ScrollDown));
    assert_eq!(app.scroll_y, 3);
    app.handle_mouse(wheel(MouseEventKind::ScrollUp));
    assert_eq!(app.scroll_y, 0);
    keys(&mut app, "G");
    app.resize(60, 14);
    let r = app.layout.object(focus(&app)).unwrap().rect;
    let (_, bh) = app.body_size();
    assert!(r.bottom() < app.scroll_y + bh, "resize keeps the focus visible");
    assert!(buffer_to_text(&render(&app)).contains('┃'), "scrollbar thumb");
}

#[test]
fn clicking_focuses_the_object_under_the_mouse() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let o = app.layout.object("ok").unwrap().rect;
    let (sx, sy) = screen_of(&app, o.x + 2, o.y + 1);
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: sx, row: sy, modifiers: KeyModifiers::NONE });
    assert_eq!(focus(&app), "ok");
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 0, row: 3, modifiers: KeyModifiers::NONE });
    assert_eq!(focus(&app), "ok", "a click on empty canvas changes nothing");
}

#[test]
fn quit_help_and_clear() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "?");
    assert!(matches!(app.mode, Mode::Help));
    let text = buffer_to_text(&render(&app));
    assert!(text.contains("Keys") && text.contains("follow an edge") && text.contains("comment to the primary agent"), "{text}");
    keys(&mut app, "x");
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(keys(&mut app, "q"), vec![Effect::Quit]);
}

#[test]
fn custom_keys_and_leader_from_config() {
    let wt = worktree();
    let mut cfg = Config::default();
    cfg.keys.leader = ",".into();
    cfg.keys.view_down = "s".into();
    let mut app = build_app(fixture("prd_flowchart"), "dg-1", wt.path().to_path_buf(), &cfg, Some(false), (100, 30)).unwrap();
    keys(&mut app, "s");
    assert_eq!(focus(&app), "refresh");
    keys(&mut app, ",ac");
    assert!(matches!(app.mode, Mode::Comment(_)));
    keys(&mut app, "<esc>");
    assert!(buffer_to_text(&render(&app)).contains(",ac ask"));
}

#[test]
fn footer_hints_follow_the_diagram_type() {
    let wt = worktree();
    let flow = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    let seq = app_for(fixture("login_sequence"), wt.path(), false, (110, 30));
    let ft = buffer_to_text(&render(&flow));
    let st = buffer_to_text(&render(&seq));
    assert!(ft.contains("hjkl move · ⏎ code · tab details · ␣ac ask"), "{ft}");
    assert!(st.contains("j/k message · h/l participant · ⏎ code · tab details · ␣ac ask"), "{st}");
}

// ------------------------------------------------------------------------------------ phase 4: sequence polish

fn cell_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<Vec<_>>().join("")
}

#[test]
fn focused_message_has_accent_arrow_bold_label_and_code_hint() {
    let wt = worktree();
    let theme = Theme::default();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (110, 40));
    app.set_focus(id("m2"));
    let buf = full(&mut app);
    let o = app.layout.object("m2").unwrap().clone();
    let lr = o.label_rect.unwrap();
    let (x, y) = screen_of(&app, lr.x, lr.y);
    assert_eq!(rgb(buf[(x, y)].fg), r3(theme.accent), "label is accent colored");
    assert!(buf[(x, y)].modifier.contains(Modifier::BOLD), "label is bold");
    assert!(cell_text(&buf, y).contains("⏎ code"), "{}", cell_text(&buf, y));
    // an unfocused linked message has neither the hint nor the accent
    let o1 = app.layout.object("m1").unwrap().clone();
    let lr1 = o1.label_rect.unwrap();
    let (x1, y1) = screen_of(&app, lr1.x, lr1.y);
    assert!(!cell_text(&buf, y1).contains("⏎ code"));
    assert_ne!(rgb(buf[(x1, y1)].fg), r3(theme.accent));
    assert!(!buf[(x1, y1)].modifier.contains(Modifier::BOLD));
    // a message without a code link gets no hint even when focused
    app.set_focus(id("db_ret"));
    let buf = full(&mut app);
    let o = app.layout.object("db_ret").unwrap().clone();
    let (_, y) = screen_of(&app, o.label_rect.unwrap().x, o.label_rect.unwrap().y);
    assert!(!cell_text(&buf, y).contains("⏎ code"));
}

#[test]
fn details_badges_on_messages_and_notes_and_tab_expands_notes() {
    let wt = worktree();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (110, 40));
    let buf = full(&mut app);
    let text = buffer_to_text(&buf);
    let m2 = app.layout.object("m2").unwrap();
    let b = m2.decor.badge.expect("message with details has a badge");
    let (bx, by) = screen_of(&app, b.x, b.y);
    assert_eq!(buf[(bx, by)].symbol(), "+");
    assert_eq!(buf[(bx + 1, by)].symbol(), "2");
    let n1 = app.layout.object("n1").unwrap();
    let nb = n1.decor.badge.expect("note with details has a badge");
    let (nx, ny) = screen_of(&app, nb.x, nb.y);
    assert_eq!((buf[(nx, ny)].symbol(), buf[(nx + 1, ny)].symbol()), ("+", "1"), "{text}");
    // m1 has no details -> no badge
    assert!(app.layout.object("m1").unwrap().decor.badge.is_none());
    // Tab on the note expands it (panel appears, note's badge stays), Tab again collapses
    app.set_focus(id("n1"));
    keys(&mut app, "<tab>");
    assert!(app.expanded.contains(&id("n1")));
    let panel = app.layout.details_of("n1").expect("note panel").rect;
    let note = app.layout.object("n1").unwrap().rect;
    assert!(!panel.intersects(&note));
    // lifelines break around the note's panel too
    let ll = app.layout.lifelines.iter().find(|l| l.breaks.iter().any(|&(a, b)| a <= panel.y && b >= panel.bottom().saturating_sub(1)));
    assert!(ll.is_some(), "some lifeline is broken across the panel rows");
    keys(&mut app, "<tab>");
    assert!(app.layout.details_of("n1").is_none());
}

#[test]
fn sequence_stale_links_are_marked_on_messages() {
    let wt = worktree();
    let theme = Theme::default();
    std::fs::write(wt.path().join("src/auth/verify.ts"), "nothing here\n").unwrap();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (110, 40));
    app.set_focus(id("m2"));
    let buf = full(&mut app);
    assert_eq!(app.link_state(&id("m2")), Some(Resolved::Stale));
    let c = app.layout.object("m2").unwrap().decor.code.unwrap();
    let (x, y) = screen_of(&app, c.x, c.y);
    assert_eq!(buf[(x, y)].symbol(), "!");
    assert_eq!(rgb(buf[(x, y)].bg), r3(theme.error));
    assert!(cell_text(&buf, y).contains("stale link"), "focused stale message says so: {}", cell_text(&buf, y));
    assert!(!cell_text(&buf, y).contains("⏎ code"));
    // m1 is still fine
    assert_eq!(app.link_state(&id("m1")), Some(Resolved::Exact(14)));
}

#[test]
fn moved_links_show_the_new_line_in_the_details_panel_code_section() {
    let wt = worktree();
    // flowchart: `check` has details and a code link at L142; shift the file by 7 lines
    let src = std::fs::read_to_string(wt.path().join("src/auth/session.ts")).unwrap();
    std::fs::write(wt.path().join("src/auth/session.ts"), format!("{}{src}", "// new\n".repeat(7))).unwrap();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    keys(&mut app, "<tab>");
    let text = buffer_to_text(&full(&mut app));
    assert_eq!(app.link_state(&id("check")), Some(Resolved::Moved(149)));
    assert!(text.contains("session.ts:149"), "{text}");
    assert!(text.contains("moved from L142"), "{text}");
    assert!(!text.contains("session.ts:142"), "the stale stored line is not shown\n{text}");

    // sequence: m2 (verify.ts:31) shifted by 3
    let src = std::fs::read_to_string(wt.path().join("src/auth/verify.ts")).unwrap();
    std::fs::write(wt.path().join("src/auth/verify.ts"), format!("{}{src}", "// new\n".repeat(3))).unwrap();
    let mut app = app_for(fixture("login_sequence"), wt.path(), false, (110, 40));
    app.set_focus(id("m2"));
    app.set_expanded(&[id("m2")]).unwrap();
    let text = buffer_to_text(&full(&mut app));
    assert_eq!(app.link_state(&id("m2")), Some(Resolved::Moved(34)));
    assert!(text.contains("verify.ts:34") && text.contains("moved from L31"), "{text}");

    // an exact link keeps the stored text and no note
    let wt2 = worktree();
    let mut app = app_for(fixture("login_sequence"), wt2.path(), false, (110, 40));
    app.set_focus(id("m2"));
    app.set_expanded(&[id("m2")]).unwrap();
    let text = buffer_to_text(&full(&mut app));
    assert!(text.contains("verify.ts:31") && !text.contains("moved from"), "{text}");
}

#[test]
fn moving_the_focus_never_retargets_neovim_only_enter_does() {
    let wt = worktree();
    let mut app = app_for(fixture("prd_flowchart"), wt.path(), false, (100, 30));
    for script in ["j", "k", "l", "h", "gG", "<tab>", "<tab>", "/ref<cr>", "<esc>"] {
        let fx = keys(&mut app, script);
        assert!(!fx.iter().any(|e| matches!(e, Effect::OpenCode { .. })), "{script:?} produced {fx:?}");
    }
    let mut seq = app_for(fixture("login_sequence"), wt.path(), false, (110, 40));
    for script in ["j", "j", "k", "l", "h", "<tab>"] {
        let fx = keys(&mut seq, script);
        assert!(!fx.iter().any(|e| matches!(e, Effect::OpenCode { .. })), "{script:?} produced {fx:?}");
    }
    let fx = keys(&mut app, "<cr>");
    assert!(fx.iter().any(|e| matches!(e, Effect::OpenCode { .. })) || app.link_state(&app.focus.clone()).is_none());
}
