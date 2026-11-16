//! Integration tests for the diagram module (PRD F3.4, F3.6 and the spec/schema contract).

use std::path::PathBuf;
use tether_core::diagram::*;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/diagrams")
        .join(name)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name)).expect("fixture")
}

fn errors(name: &str) -> Vec<ValidationError> {
    validate(&fixture(name)).expect_err("fixture should be invalid")
}

fn has(errs: &[ValidationError], path: &str, needle: &str) -> bool {
    errs.iter()
        .any(|e| e.path == path && e.message.contains(needle))
}

fn golden(name: &str, actual: &str) {
    let path = fixture_path(&format!("golden/{name}"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).expect("golden file (run with UPDATE_GOLDEN=1)");
    assert_eq!(expected, actual, "golden {name} differs");
}

// ---------------------------------------------------------------- valid

#[test]
fn prd_example_parses_exactly_as_written() {
    // The PRD JSON verbatim, including its inline layout.
    let src = r#"{
  "type": "flowchart",
  "title": "Session refresh",
  "nodes": [
    { "id": "check", "label": "Token expired?", "shape": "decision",
      "code": { "path": "src/auth/session.ts", "line": 142, "anchor": "token.expiresAt <" },
      "details": [{ "title": "Edge case", "body": "Uses client clock; skew > 30s causes early refresh" }] },
    { "id": "refresh", "label": "Call /token/refresh",
      "code": { "path": "src/auth/refresh.ts", "line": 18 } },
    { "id": "ok", "label": "Continue request" }
  ],
  "edges": [
    { "from": "check", "to": "refresh", "label": "yes" },
    { "from": "check", "to": "ok", "label": "no" }
  ]
}"#;
    let Diagram::Flowchart(f) = validate(src).unwrap() else {
        panic!("not a flowchart")
    };
    assert_eq!(f.title, "Session refresh");
    assert_eq!(f.direction(), Direction::TB);
    assert_eq!(f.nodes.len(), 3);
    assert_eq!(f.nodes[0].shape(), Shape::Decision);
    assert_eq!(f.nodes[1].shape(), Shape::Process);
    assert_eq!(
        f.nodes[0].code.as_ref().unwrap().anchor.as_deref(),
        Some("token.expiresAt <")
    );
    assert_eq!(f.nodes[0].details[0].title, "Edge case");
    assert_eq!(f.edges[0].label.as_deref(), Some("yes"));
    assert_eq!(f.edges[0].style(), EdgeStyle::Solid);
    assert!(validate(&fixture("prd_flowchart.json")).is_ok());
}

#[test]
fn big_flowchart_fixture_is_valid() {
    let Diagram::Flowchart(f) = validate(&fixture("session_refresh_30.json")).unwrap() else {
        panic!()
    };
    assert_eq!(f.nodes.len(), 30);
    assert!(f.edges.len() >= 30);
    for shape in [
        Shape::Process,
        Shape::Decision,
        Shape::Terminal,
        Shape::Io,
        Shape::Subroutine,
        Shape::Database,
    ] {
        assert!(
            f.nodes.iter().any(|n| n.shape() == shape),
            "{shape:?} unused"
        );
    }
    assert!(f.edges.iter().any(|e| e.style() == EdgeStyle::Dashed));
}

#[test]
fn sequence_fixture_valid_with_auto_ids() {
    let Diagram::Sequence(s) = validate(&fixture("login_sequence.json")).unwrap() else {
        panic!()
    };
    let mut ids = Vec::new();
    s.walk(&mut |it| match it {
        Item::Message(m) => ids.push(m.id.clone()),
        Item::Note(n) => ids.push(n.id.clone()),
        Item::Group(_) => {}
    });
    assert_eq!(
        ids,
        ["m1", "m2", "m3", "db_ret", "n1", "m4", "m5", "m6", "m7", "m8", "m9", "m10", "m11"]
    );
    let Item::Message(m2) = &s.items[1] else {
        panic!()
    };
    assert_eq!(m2.details.len(), 2);
    assert_eq!(m2.code.as_ref().unwrap().line, 31);
}

#[test]
fn auto_ids_skip_explicit_collisions_and_are_deterministic() {
    let src = r#"{"type":"sequence","title":"S","participants":[{"id":"a","label":"A"}],
      "items":[{"type":"message","from":"a","to":"a","label":"x"},
               {"type":"message","id":"m1","from":"a","to":"a","label":"y"},
               {"type":"note","over":["a"],"text":"t"}]}"#;
    let a = validate(src).unwrap();
    let b = validate(src).unwrap();
    assert_eq!(a, b);
    let Diagram::Sequence(s) = a else { panic!() };
    let Item::Message(first) = &s.items[0] else {
        panic!()
    };
    assert_eq!(first.id, "m2");
}

#[test]
fn diagrams_round_trip_through_json() {
    for f in [
        "prd_flowchart.json",
        "session_refresh_30.json",
        "login_sequence.json",
    ] {
        let d = validate(&fixture(f)).unwrap();
        let text = serde_json::to_string(&d).unwrap();
        assert_eq!(validate(&text).unwrap(), d, "{f}");
    }
}

// -------------------------------------------------------------- invalid

#[test]
fn rejects_bad_json_and_bad_type() {
    let e = validate("{nope").unwrap_err();
    assert!(e[0].message.contains("invalid JSON at line 1"));
    let e = validate(r#"{"title":"x"}"#).unwrap_err();
    assert!(has(&e, "/type", "missing"));
    let e = validate(r#"{"type":"gantt"}"#).unwrap_err();
    assert!(has(&e, "/type", "unknown diagram type"));
}

#[test]
fn schema_errors_carry_json_pointers() {
    let e = errors("invalid_flowchart_bad_shape.json");
    assert!(has(&e, "/nodes/0/shape", "hexagon"), "{e:?}");
    assert!(
        e[0].message.contains("process"),
        "should list allowed shapes: {e:?}"
    );
    let e = errors("invalid_flowchart_extra_field.json");
    assert!(e.iter().any(|x| x.message.contains("colour")), "{e:?}");
    let e = errors("invalid_sequence_line_zero.json");
    assert!(e.iter().any(|x| x.path == "/items/0/code/line"), "{e:?}");
    let e = validate(r#"{"type":"flowchart","title":"T","nodes":[]}"#).unwrap_err();
    assert!(e.iter().any(|x| x.path == "/nodes"), "{e:?}");
}

#[test]
fn flowchart_semantic_errors() {
    let e = errors("invalid_flowchart_unknown_edge.json");
    assert!(has(&e, "/edges/1/to", "unknown node \"zzz\""));
    assert!(e[0].message.contains("known node ids: a, b"));
    let e = errors("invalid_flowchart_duplicate_ids.json");
    assert!(has(&e, "/nodes/2/id", "duplicate node id \"a\""));
    assert!(e[0].message.contains("/nodes/0/id"));
    let e = errors("invalid_flowchart_empty_label.json");
    assert!(has(&e, "/nodes/0/label", "must not be empty"));
    let e = errors("invalid_flowchart_bad_lines.json");
    assert!(has(&e, "/nodes/0/code/end_line", "must be >= line"));
    let e = errors("invalid_flowchart_abs_path.json");
    assert!(has(&e, "/nodes/0/code/path", "relative"));
}

#[test]
fn sequence_semantic_errors() {
    let e = errors("invalid_sequence_unknown_participant.json");
    assert!(has(&e, "/items/1/to", "unknown participant \"ghost\""));
    let e = errors("invalid_sequence_note_over.json");
    assert!(has(&e, "/items/1/over/1", "unknown participant \"nope\""));
    let e = errors("invalid_sequence_dup_ids.json");
    assert!(has(&e, "/items/2/id", "duplicate"));
    let e = errors("invalid_sequence_alt_without_branches.json");
    assert!(has(&e, "/items/1/branches", "alt group needs"));
}

#[test]
fn nested_errors_report_full_path() {
    let src = r#"{"type":"sequence","title":"S","participants":[{"id":"a","label":"A"}],
      "items":[{"type":"group","kind":"alt","label":"g","branches":[
        {"items":[{"type":"message","from":"a","to":"a","label":"ok"}]},
        {"label":"x","items":[{"type":"message","from":"a","to":"q","label":"bad"}]}]}]}"#;
    let e = validate(src).unwrap_err();
    assert!(
        has(
            &e,
            "/items/0/branches/1/items/0/to",
            "unknown participant \"q\""
        ),
        "{e:?}"
    );
}

#[test]
fn duplicate_participants_and_blank_title() {
    let src = r#"{"type":"sequence","title":" ","participants":[{"id":"a","label":"A"},{"id":"a","label":"B"}],
      "items":[{"type":"message","from":"a","to":"a","label":"x"}]}"#;
    let e = validate(src).unwrap_err();
    assert!(has(&e, "/title", "must not be empty"));
    assert!(has(&e, "/participants/1/id", "duplicate participant id"));
}

#[test]
fn error_display_includes_path() {
    let e = errors("invalid_flowchart_unknown_edge.json");
    assert!(e[0].to_string().starts_with("/edges/1/to: "));
    assert!(format_errors(&e).starts_with("- /edges/1/to"));
}

// -------------------------------------------------------------- mermaid

#[test]
fn mermaid_flowchart_shapes_and_styles() {
    let d = validate(
        r#"{"type":"flowchart","title":"T","direction":"LR","nodes":[
        {"id":"p","label":"Proc"},{"id":"d","label":"Dec?","shape":"decision"},
        {"id":"t","label":"End","shape":"terminal"},{"id":"i","label":"In","shape":"io"},
        {"id":"s","label":"Sub","shape":"subroutine"},{"id":"db","label":"Store","shape":"database"}],
        "edges":[{"from":"p","to":"d"},{"from":"d","to":"t","label":"yes","style":"dashed"}]}"#,
    )
    .unwrap();
    let expected = "\
flowchart LR
    p[\"Proc\"]
    d{\"Dec?\"}
    t([\"End\"])
    i[/\"In\"/]
    s[[\"Sub\"]]
    db[(\"Store\")]
    p --> d
    d -.->|\"yes\"| t
";
    assert_eq!(to_mermaid(&d), expected);
}

#[test]
fn mermaid_escapes_labels_and_ids() {
    let d = validate(
        r#"{"type":"flowchart","title":"T","nodes":[
        {"id":"end","label":"say \"hi\" #1 <b> & more; ok\nline2"},{"id":"a-b","label":"x"},{"id":"a_b","label":"y"},{"id":"1st","label":"z"}],
        "edges":[{"from":"end","to":"a-b","label":"a|b"},{"from":"a_b","to":"1st"}]}"#,
    )
    .unwrap();
    let m = to_mermaid(&d);
    assert!(m.starts_with("flowchart TD\n"));
    assert!(
        m.contains(r#"end_["say #quot;hi#quot; #35;1 #lt;b#gt; #amp; more#59; ok<br/>line2"]"#),
        "{m}"
    );
    assert!(m.contains(r#"end_ -->|"a|b"| a_b"#), "{m}");
    assert!(m.contains("a_b_2[\"y\"]"), "{m}");
    assert!(m.contains("n1st[\"z\"]"), "{m}");
}

#[test]
fn mermaid_sequence_arrows_notes_and_groups() {
    let d = validate(
        r#"{"type":"sequence","title":"S","participants":[
          {"id":"u","label":"User","kind":"actor"},{"id":"api","label":"API"},
          {"id":"db","label":"DB","kind":"database"},{"id":"q","label":"Jobs","kind":"queue"}],
        "items":[
          {"type":"message","from":"u","to":"api","label":"go"},
          {"type":"message","from":"api","to":"q","label":"enqueue","kind":"async"},
          {"type":"note","over":["api","db"],"text":"a; b"},
          {"type":"group","kind":"alt","label":"ok","branches":[
            {"items":[{"type":"message","from":"db","to":"api","label":"row","kind":"return"}]},
            {"label":"missing","items":[{"type":"note","over":["db"],"text":"none"}]}]},
          {"type":"group","kind":"par","label":"fan out","branches":[
            {"items":[{"type":"message","from":"api","to":"db","label":"w1"}]},
            {"label":"second","items":[{"type":"message","from":"api","to":"q","label":"w2"}]}]},
          {"type":"group","kind":"opt","label":"cache","items":[{"type":"message","from":"api","to":"api","label":"self"}]},
          {"type":"group","kind":"loop","label":"retry","items":[{"type":"message","from":"api","to":"db","label":"again"}]}]}"#,
    )
    .unwrap();
    let expected = r#"sequenceDiagram
    actor u as User
    participant api as API
    participant db@{ "type": "database" } as DB
    participant q@{ "type": "queue" } as Jobs
    u->>api: go
    api-)q: enqueue
    Note over api,db: a#59; b
    alt ok
        db-->>api: row
    else missing
        Note over db: none
    end
    par fan out
        api->>db: w1
    and second
        api->>q: w2
    end
    opt cache
        api->>api: self
    end
    loop retry
        api->>db: again
    end
"#;
    assert_eq!(to_mermaid(&d), expected);
}

#[test]
fn mermaid_goldens_for_fixtures() {
    for (f, g) in [
        ("prd_flowchart.json", "prd_flowchart.mmd"),
        ("session_refresh_30.json", "session_refresh_30.mmd"),
        ("login_sequence.json", "login_sequence.mmd"),
    ] {
        golden(g, &to_mermaid(&validate(&fixture(f)).unwrap()));
    }
}

// ---------------------------------------------------------------- store

#[test]
fn store_round_trip_with_fixtures_in_temp_git_repo() {
    let repo = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    let store = DiagramStore::open(repo.path());
    let mut ids = Vec::new();
    for f in [
        "prd_flowchart.json",
        "session_refresh_30.json",
        "login_sequence.json",
    ] {
        let d = validate(&fixture(f)).unwrap();
        let id = store.create(&d).unwrap();
        assert_eq!(store.get(&id).unwrap(), d, "{f}");
        ids.push(id);
    }
    let list = store.list().unwrap();
    assert_eq!(list.len(), 3);
    let kinds: Vec<_> = list
        .iter()
        .filter(|i| i.kind == DiagramKind::Sequence)
        .collect();
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].title, "Login request");
    for id in ids {
        store.delete(&id).unwrap();
    }
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn item_schema_errors_are_specific() {
    let head =
        r#"{"type":"sequence","title":"S","participants":[{"id":"a","label":"A"}],"items":["#;
    let e = validate(&format!(
        r#"{head}{{"type":"message","from":"a","label":"x"}}]}}"#
    ))
    .unwrap_err();
    assert!(
        has(&e, "/items/0", "\"to\" is a required property"),
        "{e:?}"
    );
    let e = validate(&format!(r#"{head}{{"type":"msg"}}]}}"#)).unwrap_err();
    assert!(
        has(&e, "/items/0/type", "unknown item type \"msg\""),
        "{e:?}"
    );
    let e = validate(&format!(r#"{head}{{"from":"a"}}]}}"#)).unwrap_err();
    assert!(has(&e, "/items/0", "missing \"type\""), "{e:?}");
    let e = validate(&format!(
        r#"{head}{{"type":"group","kind":"loop","label":"l","items":[{{"type":"message","from":"a","to":"a","label":"x","kind":"shout"}}]}}]}}"#
    ))
    .unwrap_err();
    assert!(has(&e, "/items/0/items/0/kind", "shout"), "{e:?}");
}

// ---------------------------------------------------------------- id namespace (review fix 1)

fn seq_with(extra_items: &str, participants: &str) -> String {
    format!(
        r#"{{"type":"sequence","title":"t","participants":{participants},"items":[{extra_items}]}}"#
    )
}

#[test]
fn auto_ids_never_collide_with_participant_ids() {
    let src = seq_with(
        r#"{"type":"message","from":"m1","to":"n1","label":"hi"},{"type":"note","over":["m1"],"text":"x"}"#,
        r#"[{"id":"m1","label":"M"},{"id":"n1","label":"N"}]"#,
    );
    let Diagram::Sequence(s) = validate(&src).expect("valid") else {
        panic!()
    };
    let mut ids: Vec<String> = s.participants.iter().map(|p| p.id.clone()).collect();
    s.walk(&mut |it| match it {
        Item::Message(m) => ids.push(m.id.clone()),
        Item::Note(n) => ids.push(n.id.clone()),
        _ => {}
    });
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "ids must be unique: {ids:?}");
}

#[test]
fn message_id_may_not_reuse_a_participant_id() {
    let src = seq_with(
        r#"{"type":"message","id":"a","from":"a","to":"b","label":"hi"}"#,
        r#"[{"id":"a","label":"A"},{"id":"b","label":"B"}]"#,
    );
    let errs = validate(&src).expect_err("shared namespace");
    assert!(has(&errs, "/items/0/id", "duplicate"), "{errs:?}");
    assert!(has(&errs, "/items/0/id", "/participants/0/id"), "{errs:?}");
}

#[test]
fn reserved_generated_id_shapes_are_rejected() {
    for bad in ["group:1", "x#details", "edge:0", "a:label", "has space"] {
        let seq = seq_with(
            &format!(r#"{{"type":"message","id":"{bad}","from":"a","to":"b","label":"hi"}}"#),
            r#"[{"id":"a","label":"A"},{"id":"b","label":"B"}]"#,
        );
        assert!(validate(&seq).is_err(), "sequence message id {bad:?} accepted");
        let part = seq_with(
            r#"{"type":"message","from":"a","to":"b","label":"hi"}"#,
            &format!(r#"[{{"id":"a","label":"A"}},{{"id":"{bad}","label":"B"}}]"#),
        );
        assert!(validate(&part).is_err(), "participant id {bad:?} accepted");
        let flow = format!(
            r#"{{"type":"flowchart","title":"t","nodes":[{{"id":"{bad}","label":"x"}}]}}"#
        );
        let errs = validate(&flow).expect_err(bad);
        assert!(errs.iter().any(|e| e.path == "/nodes/0/id"), "{bad}: {errs:?}");
    }
    // the allowed alphabet still works
    let ok = r#"{"type":"flowchart","title":"t","nodes":[{"id":"a-b_c.9","label":"x"}]}"#;
    assert!(validate(ok).is_ok());
}

#[test]
fn code_line_is_bounded() {
    let src = r#"{"type":"flowchart","title":"t","nodes":[{"id":"a","label":"x","code":{"path":"f","line":4294967295}}]}"#;
    let errs = validate(src).expect_err("huge line");
    assert!(errs.iter().any(|e| e.path == "/nodes/0/code/line"), "{errs:?}");
    let src = r#"{"type":"flowchart","title":"t","nodes":[{"id":"a","label":"x","code":{"path":"f","line":1,"end_line":4294967295}}]}"#;
    assert!(validate(src).is_err());
}

#[test]
fn mermaid_compat_drops_participant_types() {
    let d = validate(&fixture("login_sequence.json")).unwrap();
    let new = to_mermaid_opts(&d, false);
    let old = to_mermaid_opts(&d, true);
    assert!(new.contains("@{"), "fixture should have a database/queue participant:\n{new}");
    assert!(!old.contains("@{"), "{old}");
    assert!(old.contains("participant "), "{old}");
    assert_eq!(to_mermaid(&d), new);
}
