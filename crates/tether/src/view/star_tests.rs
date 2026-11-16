//! Viewer `s`: star / unstar the diagram (effect, optimistic header state, failure rollback).

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tether_core::config::Config;
use tether_core::diagram::validate;

use super::app::{App, Effect, Msg, StatusKind};
use super::build_app;
use super::export::{buffer_to_text, render};

fn app_with(cfg: &Config) -> App {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tether-core/tests/fixtures/diagrams/prd_flowchart.json");
    let d = validate(&std::fs::read_to_string(p).unwrap()).unwrap();
    build_app(d, "dg-1", std::env::temp_dir(), cfg, Some(false), (100, 30)).unwrap()
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn header(app: &App) -> String {
    buffer_to_text(&render(app)).lines().next().unwrap().to_string()
}

#[test]
fn s_toggles_the_star_and_asks_the_cli() {
    let mut app = app_with(&Config::default());
    assert!(!app.starred);
    assert!(!header(&app).contains('★'));

    let fx = app.handle_key(key('s'));
    assert_eq!(fx, vec![Effect::Star { diagram: "dg-1".into(), on: true }]);
    assert_eq!(fx[0].argv().unwrap(), vec!["diagram", "star", "dg-1"]);
    assert!(app.starred, "optimistic");
    assert!(header(&app).contains("★"), "header shows the star: {}", header(&app));
    assert!(header(&app).contains("Session refresh"));

    let fx = app.handle_key(key('s'));
    assert_eq!(fx, vec![Effect::Star { diagram: "dg-1".into(), on: false }]);
    assert_eq!(fx[0].argv().unwrap(), vec!["diagram", "unstar", "dg-1"]);
    assert!(!app.starred);
    assert!(!header(&app).contains('★'));
}

#[test]
fn star_result_updates_the_status_and_a_failure_rolls_back() {
    let mut app = app_with(&Config::default());
    app.handle_key(key('s'));
    app.on_msg(Msg::StarDone { result: Ok("starred x".into()), on: true });
    assert!(app.starred);
    let st = app.status.clone().unwrap();
    assert_eq!(st.kind, StatusKind::Ok);
    assert!(st.text.contains("starred"));

    app.handle_key(key('s'));
    assert!(!app.starred);
    app.on_msg(Msg::StarDone { result: Err("diagram not found: dg-1".into()), on: false });
    assert!(app.starred, "a failed unstar puts the star back");
    let st = app.status.clone().unwrap();
    assert_eq!(st.kind, StatusKind::Error);
    assert!(st.text.contains("unstar failed") && st.text.contains("not found"), "{}", st.text);
}

#[test]
fn the_star_key_is_configurable_and_listed_in_help() {
    let cfg = Config::parse("[keys]\nview_star = \"x\"\n").unwrap();
    let mut app = app_with(&cfg);
    assert!(app.handle_key(key('s')).is_empty(), "the default key is free again");
    assert_eq!(app.handle_key(key('x')), vec![Effect::Star { diagram: "dg-1".into(), on: true }]);
    app.handle_key(key('?'));
    let help = buffer_to_text(&render(&app));
    assert!(help.contains("star / unstar this diagram"), "{help}");
}
