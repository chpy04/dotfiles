//! Browser tests: key handling (effects) and TestBackend renders. No terminal, no store.

use std::path::PathBuf;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use tether_core::config::Config;
use tether_core::diagram::{validate, DiagramInfo, DiagramKind};
use tether_core::theme::Theme;

use super::app::{preview_key, Browser, Effect, Mode, StatusKind};
use super::draw::{draw, fit};
use super::preview;
use crate::view::export::buffer_to_text;
use crate::view::icons::Icons;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;

fn row(id: &str, title: &str, kind: DiagramKind, starred: bool, branch: Option<&str>, age_days: u64) -> DiagramInfo {
    DiagramInfo {
        id: id.into(),
        title: title.into(),
        kind,
        created: NOW - age_days * DAY,
        updated: NOW - age_days * DAY,
        last_opened: None,
        starred,
        worktree: "proj".into(),
        branch: branch.map(String::from),
        note: None,
        repo: "proj".into(),
        repo_key: "proj-aaaaaaaa".into(),
    }
}

fn rows() -> Vec<DiagramInfo> {
    vec![
        row("session-refresh-aaa111", "Session refresh", DiagramKind::Flowchart, true, Some("feat-login"), 2),
        row("login-request-bbb222", "Login request", DiagramKind::Sequence, false, Some("main"), 9),
        row("cache-layout-ccc333", "Cache layout", DiagramKind::Flowchart, false, None, 40),
    ]
}

fn browser(rows: Vec<DiagramInfo>) -> Browser {
    let mut b = Browser::new(rows, "proj", Theme::from_config(&Config::default().theme).unwrap(), Icons::PLAIN, 30);
    b.now = NOW;
    b
}

fn k(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn type_str(b: &mut Browser, s: &str) {
    for c in s.chars() {
        b.handle_key(k(c));
    }
}

fn fixture(name: &str) -> tether_core::diagram::Diagram {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tether-core/tests/fixtures/diagrams").join(format!("{name}.json"));
    validate(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Render the real viewer preview for the selected row and cache it, like the runner does.
fn with_preview(b: &mut Browser, fixture_name: &str) {
    let req = b.need_preview().expect("selection needs a preview");
    let out = preview::render(fixture(fixture_name), &req.id, std::env::temp_dir(), &Config::default());
    b.set_preview(req.key, out);
}

fn render(b: &Browser, w: u16, h: u16) -> (Buffer, String) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| {
        let area = f.area();
        draw(b, f.buffer_mut(), area)
    })
    .unwrap();
    let buf = t.backend().buffer().clone();
    let text = buffer_to_text(&buf);
    if std::env::var_os("TETHER_DUMP").is_some() {
        println!("{text}\n----");
    }
    (buf, text)
}

fn id(repo: &str, id: &str) -> (String, String) {
    (repo.to_string(), id.to_string())
}

// ------------------------------------------------------------------------------------ keys

#[test]
fn move_keys_clamp_and_selection_follows() {
    let mut b = browser(rows());
    assert_eq!(b.selected().unwrap().id, "session-refresh-aaa111");
    b.handle_key(k('k'));
    assert_eq!(b.sel, 0);
    b.handle_key(k('j'));
    b.handle_key(code(KeyCode::Down));
    b.handle_key(k('j'));
    assert_eq!(b.selected().unwrap().id, "cache-layout-ccc333");
    b.handle_key(k('g'));
    assert_eq!(b.sel, 0);
    b.handle_key(k('G'));
    assert_eq!(b.sel, 2);
    b.handle_key(code(KeyCode::Up));
    assert_eq!(b.sel, 1);
}

#[test]
fn enter_t_e_act_on_the_selection() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    let (rk, rid) = id("proj-aaaaaaaa", "login-request-bbb222");
    assert_eq!(b.handle_key(code(KeyCode::Enter)), vec![Effect::OpenHere { repo_key: rk.clone(), id: rid.clone() }]);
    assert_eq!(b.handle_key(k('t')), vec![Effect::OpenTab { repo_key: rk.clone(), id: rid.clone() }]);
    assert_eq!(b.handle_key(k('e')), vec![Effect::CopyMermaid { repo_key: rk, id: rid }]);
    assert!(matches!(b.mode, Mode::Normal));
}

#[test]
fn quit_keys() {
    let mut b = browser(rows());
    assert_eq!(b.handle_key(k('q')), vec![Effect::Quit]);
    assert_eq!(b.handle_key(ctrl('c')), vec![Effect::Quit]);
    assert_eq!(b.handle_key(code(KeyCode::Esc)), vec![Effect::Quit]);
    // esc first clears an active filter
    type_str(&mut b, "/login");
    b.handle_key(code(KeyCode::Enter));
    assert_eq!(b.filter, "login");
    assert!(b.handle_key(code(KeyCode::Esc)).is_empty());
    assert_eq!(b.filter, "");
    assert_eq!(b.handle_key(code(KeyCode::Esc)), vec![Effect::Quit]);
}

#[test]
fn star_toggles_from_the_current_state() {
    let mut b = browser(rows());
    assert_eq!(
        b.handle_key(k('s')),
        vec![Effect::SetStar { repo_key: "proj-aaaaaaaa".into(), id: "session-refresh-aaa111".into(), on: false }],
        "starred -> unstar"
    );
    b.handle_key(k('j'));
    assert_eq!(
        b.handle_key(k('s')),
        vec![Effect::SetStar { repo_key: "proj-aaaaaaaa".into(), id: "login-request-bbb222".into(), on: true }]
    );
}

#[test]
fn rename_flow_prefills_edits_and_refuses_empty() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    b.handle_key(k('r'));
    let Mode::Rename { input, .. } = &b.mode else { panic!("{:?}", b.mode) };
    assert_eq!(input.text, "Login request");
    // edit: remove "request", type "flow"
    for _ in 0..7 {
        b.handle_key(code(KeyCode::Backspace));
    }
    type_str(&mut b, "flow");
    assert_eq!(
        b.handle_key(code(KeyCode::Enter)),
        vec![Effect::Rename { repo_key: "proj-aaaaaaaa".into(), id: "login-request-bbb222".into(), title: "Login flow".into() }]
    );
    assert!(matches!(b.mode, Mode::Normal));

    // empty title stays in the editor with an error
    b.handle_key(k('r'));
    b.handle_key(ctrl('u'));
    assert!(b.handle_key(code(KeyCode::Enter)).is_empty());
    assert!(matches!(b.mode, Mode::Rename { .. }));
    assert_eq!(b.status.as_ref().unwrap().0, StatusKind::Error);
    // unchanged title and esc do nothing
    b.handle_key(code(KeyCode::Esc));
    b.handle_key(k('r'));
    assert!(b.handle_key(code(KeyCode::Enter)).is_empty());
    assert!(matches!(b.mode, Mode::Normal));
    b.handle_key(k('r'));
    type_str(&mut b, "zzz");
    assert!(b.handle_key(code(KeyCode::Esc)).is_empty());
    assert!(matches!(b.mode, Mode::Normal));
}

#[test]
fn rename_editor_moves_the_cursor() {
    let mut b = browser(rows());
    b.handle_key(k('r'));
    b.handle_key(code(KeyCode::Home));
    type_str(&mut b, ">> ");
    b.handle_key(code(KeyCode::End));
    b.handle_key(code(KeyCode::Left));
    b.handle_key(code(KeyCode::Delete));
    b.handle_key(ctrl('w'));
    let Mode::Rename { input, .. } = &b.mode else { panic!() };
    assert_eq!(input.text, ">> Session ");
}

#[test]
fn delete_needs_y() {
    let mut b = browser(rows());
    b.handle_key(k('d'));
    assert!(matches!(b.mode, Mode::ConfirmDelete { .. }));
    for no in [k('n'), code(KeyCode::Esc), code(KeyCode::Enter), k('x')] {
        assert!(b.handle_key(no).is_empty());
        assert!(matches!(b.mode, Mode::Normal), "any other key cancels");
        b.handle_key(k('d'));
    }
    assert_eq!(
        b.handle_key(k('y')),
        vec![Effect::Delete { repo_key: "proj-aaaaaaaa".into(), id: "session-refresh-aaa111".into() }]
    );
    assert!(matches!(b.mode, Mode::Normal));
}

#[test]
fn filter_is_incremental_by_title_or_branch_and_keeps_the_selection() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    b.handle_key(k('/'));
    assert!(matches!(b.mode, Mode::Filter(_)));
    type_str(&mut b, "LAY");
    assert_eq!(b.visible().len(), 1, "title match is case-insensitive");
    assert_eq!(b.selected().unwrap().id, "cache-layout-ccc333");
    b.handle_key(ctrl('u'));
    type_str(&mut b, "feat-");
    assert_eq!(b.visible().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["session-refresh-aaa111"], "branch match");
    for _ in 0..5 {
        b.handle_key(code(KeyCode::Backspace));
    }
    assert_eq!(b.visible().len(), 3);
    assert_eq!(b.selected().unwrap().id, "session-refresh-aaa111", "selection falls back to the first row");
    type_str(&mut b, "zzz");
    assert!(b.visible().is_empty());
    assert!(b.selected().is_none());
    // no selection: act keys do nothing
    b.handle_key(code(KeyCode::Enter)); // keeps the filter
    for key in ['t', 's', 'r', 'd', 'e'] {
        assert!(b.handle_key(k(key)).is_empty());
    }
    assert!(matches!(b.mode, Mode::Normal));
    // esc inside the filter prompt clears it
    b.handle_key(k('/'));
    assert!(b.handle_key(code(KeyCode::Esc)).is_empty());
    assert_eq!(b.filter, "");
    assert_eq!(b.visible().len(), 3);
    // arrow keys move while typing
    b.handle_key(k('/'));
    b.handle_key(code(KeyCode::Down));
    b.handle_key(ctrl('n'));
    assert_eq!(b.sel, 2);
    b.handle_key(ctrl('p'));
    assert_eq!(b.sel, 1);
}

#[test]
fn starred_only_and_all_repos_toggles() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    b.handle_key(k('S'));
    assert!(b.starred_only);
    assert_eq!(b.visible().len(), 1);
    assert_eq!(b.selected().unwrap().id, "session-refresh-aaa111");
    b.handle_key(k('S'));
    assert!(!b.starred_only && b.visible().len() == 3);
    assert_eq!(b.handle_key(k('a')), vec![Effect::Reload]);
    assert!(b.all_repos);
    assert_eq!(b.handle_key(k('a')), vec![Effect::Reload]);
    assert!(!b.all_repos);
}

#[test]
fn set_rows_keeps_selection_by_id_and_drops_stale_previews() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    with_preview(&mut b, "login_sequence");
    assert!(b.need_preview().is_none());
    let key = preview_key(b.selected().unwrap());
    // a new row on top: the selection stays on the same diagram
    let mut fresh = rows();
    fresh.insert(0, row("new-ddd444", "New one", DiagramKind::Flowchart, false, None, 0));
    b.set_rows(fresh, None);
    assert_eq!(b.selected().unwrap().id, "login-request-bbb222");
    assert!(b.previews.contains_key(&key), "unchanged diagram keeps its preview");
    // an update invalidates it, a delete clamps the selection
    let mut changed = rows();
    changed[1].updated += 5;
    b.set_rows(changed, None);
    assert!(!b.previews.contains_key(&key));
    assert!(b.need_preview().is_some());
    b.set_rows(vec![row("only-eee555", "Only", DiagramKind::Flowchart, false, None, 1)], None);
    assert_eq!(b.sel, 0);
    b.set_rows(vec![], None);
    assert!(b.selected().is_none());
}

#[test]
fn preview_scroll_is_clamped_to_the_canvas() {
    let mut b = browser(rows());
    with_preview(&mut b, "session_refresh_30");
    let (cw, ch) = b.canvas_size().unwrap();
    assert!(ch > 20, "the 30-node fixture is taller than a pane: {ch}");
    b.view.set((60, 12));
    b.handle_key(ctrl('u'));
    assert_eq!(b.scroll_y, 0);
    b.handle_key(ctrl('d'));
    assert_eq!(b.scroll_y, 6, "half the pane");
    for _ in 0..100 {
        b.handle_key(ctrl('d'));
    }
    assert_eq!(b.scroll_y, ch - 12);
    b.handle_key(ctrl('u'));
    assert_eq!(b.scroll_y, ch - 12 - 6);
    b.handle_key(k('>'));
    assert_eq!(b.scroll_x, 8.min(cw.saturating_sub(60)));
    // moving to another diagram resets the scroll
    b.handle_key(k('j'));
    assert_eq!((b.scroll_y, b.scroll_x), (0, 0));
}

#[test]
fn help_opens_and_any_key_closes() {
    let mut b = browser(rows());
    b.handle_key(k('?'));
    assert!(matches!(b.mode, Mode::Help));
    assert!(b.handle_key(k('q')).is_empty(), "q only closes help");
    assert!(matches!(b.mode, Mode::Normal));
}

#[test]
fn fit_truncates_with_an_ellipsis() {
    assert_eq!(fit("short", 10), "short");
    assert_eq!(fit("a longer title here", 8), "a longe…");
    assert_eq!(fit("x", 0), "");
}

// ------------------------------------------------------------------------------------ renders

#[test]
fn list_and_live_preview_render() {
    let mut b = browser(rows());
    with_preview(&mut b, "session_refresh_30");
    let (buf, text) = render(&b, 140, 36);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("tether diagrams") && lines[0].contains("proj") && lines[0].contains("3 diagrams"), "{}", lines[0]);
    // list: star, title, kind glyph, branch, age
    assert!(lines[1].contains("★ Session refresh") && lines[1].contains("feat-login") && lines[1].contains("2d"), "{}", lines[1]);
    assert!(lines[2].contains("Login request") && lines[2].contains("main") && lines[2].contains("9d"), "{}", lines[2]);
    assert!(lines[3].contains("Cache layout") && lines[3].contains("5w"), "{}", lines[3]);
    // the selected row is drawn with the theme's accent fill
    let th = &b.theme;
    let sel_cell = buf.cell((5, 1)).unwrap();
    assert_eq!(sel_cell.bg, ratatui::style::Color::Rgb(th.accent.0, th.accent.1, th.accent.2));
    let other = buf.cell((5, 2)).unwrap();
    assert_eq!(other.bg, ratatui::style::Color::Rgb(th.bg.0, th.bg.1, th.bg.2));
    // preview: title strip plus the viewer's drawing of the diagram (box borders)
    assert!(lines[1].contains("│") && lines[1].contains("Session refresh"));
    assert!(text.contains("╭") && text.contains("─"), "boxes from the viewer's draw code");
    assert!(text.contains("flowchart"), "kind in the title strip");
    // hints
    assert!(lines.last().unwrap().contains("↵ open") && lines.last().unwrap().contains("? help"));
    // no leakage of the viewer's own footer hints
    assert!(!text.contains("q quit  ") || lines.last().unwrap().contains("q quit"));
}

#[test]
fn tall_diagrams_are_cropped_with_a_scroll_hint_and_scroll() {
    let mut b = browser(rows());
    with_preview(&mut b, "session_refresh_30");
    let (_, top) = render(&b, 120, 24);
    assert!(top.contains("rows 1-"), "{top}");
    assert!(top.contains("^d ^u"));
    b.handle_key(ctrl('d'));
    b.handle_key(ctrl('d'));
    let (_, scrolled) = render(&b, 120, 24);
    assert!(!scrolled.contains("rows 1-"), "{scrolled}");
    assert_ne!(top, scrolled);
}

#[test]
fn sequence_preview_renders() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    with_preview(&mut b, "login_sequence");
    let (_, text) = render(&b, 140, 40);
    assert!(text.contains("sequence"));
    assert!(text.contains("Browser") && text.contains("╭"), "{text}");
}

#[test]
fn preview_error_is_shown_in_the_pane() {
    let mut b = browser(rows());
    let req = b.need_preview().unwrap();
    b.set_preview(req.key, Err("stored diagram x is corrupt".into()));
    let (_, text) = render(&b, 120, 24);
    assert!(text.contains("cannot render this diagram") && text.contains("corrupt"), "{text}");
}

#[test]
fn empty_state_explains_how_diagrams_get_created() {
    let b = browser(vec![]);
    let (_, text) = render(&b, 100, 24);
    assert!(text.contains("No saved diagrams for proj yet."), "{text}");
    assert!(text.contains("Ask Claude to draw one") && text.contains("tether diagram create spec.json"), "{text}");
    assert!(text.contains("kept for 30 days unless starred"), "{text}");
    assert!(text.lines().next().unwrap().contains("0 diagrams"));
    assert!(!text.contains('│'), "no preview pane when there is nothing to preview");
    let mut keep0 = browser(vec![]);
    keep0.keep_days = 0;
    assert!(render(&keep0, 100, 24).1.contains("kept until you delete them"));
}

#[test]
fn filter_prompt_header_indicator_and_no_match_message() {
    let mut b = browser(rows());
    b.handle_key(k('/'));
    type_str(&mut b, "request");
    let (_, text) = render(&b, 120, 24);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("/ request") && lines[0].contains("1 of 3"), "{}", lines[0]);
    assert!(lines[lines.len() - 2].trim_start().starts_with("/ request"), "{}", lines[lines.len() - 2]);
    assert!(lines[1].contains("Login request") && !text.contains("Cache layout"));
    type_str(&mut b, "zzz");
    let (_, none) = render(&b, 120, 24);
    assert!(none.contains("No diagrams match \"requestzzz\"."), "{none}");
}

#[test]
fn delete_confirmation_prompt_renders() {
    let mut b = browser(rows());
    b.handle_key(k('j'));
    b.handle_key(k('d'));
    let (_, text) = render(&b, 120, 24);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[lines.len() - 2].contains("Delete \"Login request\"? (y/n)"), "{}", lines[lines.len() - 2]);
    assert!(lines[lines.len() - 1].contains("y delete"));
}

#[test]
fn rename_prompt_status_starred_header_and_help_render() {
    let mut b = browser(rows());
    b.handle_key(k('r'));
    let (_, text) = render(&b, 120, 24);
    assert!(text.lines().nth(22).unwrap().trim_start().starts_with("rename: Session refresh"), "{text}");

    let mut b = browser(rows());
    b.handle_key(k('S'));
    b.set_status(StatusKind::Ok, "starred \"Session refresh\"");
    let (_, text) = render(&b, 120, 24);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("★ only") && lines[0].contains("1 of 3"), "{}", lines[0]);
    assert!(lines[22].contains("starred \"Session refresh\""), "{}", lines[22]);
    assert!(!text.contains("Login request"));

    let mut b = browser(rows());
    b.handle_key(k('?'));
    let (_, text) = render(&b, 120, 30);
    for want in ["star / unstar", "rename (the id stays)", "toggle all repos", "scroll the preview", "Mermaid"] {
        assert!(text.contains(want), "help lacks {want}: {text}");
    }
}

#[test]
fn all_repos_mode_shows_the_repo_next_to_the_branch() {
    let mut rs = rows();
    rs[1].repo = "other-repo".into();
    rs[1].repo_key = "other-repo-bbbbbbbb".into();
    let mut b = browser(rs);
    b.all_repos = true;
    let (_, text) = render(&b, 150, 24);
    assert!(text.lines().next().unwrap().contains("all repos"));
    assert!(text.lines().nth(2).unwrap().contains("other-repo:"), "{}", text.lines().nth(2).unwrap());
    // the filter also finds repo names
    b.handle_key(k('/'));
    type_str(&mut b, "other");
    assert_eq!(b.visible().len(), 1);
}

#[test]
fn tiny_terminal_degrades_to_a_message() {
    let b = browser(rows());
    assert!(render(&b, 20, 4).1.contains("terminal too small"));
    // narrow but usable
    let (_, text) = render(&b, 60, 14);
    assert!(text.contains("Session"), "{text}");
}

// ------------------------------------------------------------------------------------ clipboard

#[test]
fn clipboard_override_receives_the_text() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("clip.txt");
    std::env::set_var("TETHER_CLIPBOARD", format!("cat > {}", out.display()));
    super::copy_to_clipboard("flowchart TD\n  a --> b\n").unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "flowchart TD\n  a --> b\n");
    std::env::set_var("TETHER_CLIPBOARD", "exit 3");
    assert!(super::copy_to_clipboard("x").unwrap_err().contains("failed"));
    std::env::remove_var("TETHER_CLIPBOARD");
}
