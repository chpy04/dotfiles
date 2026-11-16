//! Layout engine tests (PRD F3.5: no overlapping lines at ~30 nodes, expansion re-layout).
//!
//! Snapshots live in `tests/fixtures/layout/*.txt`; regenerate with `UPDATE_GOLDEN=1`.
//! To eyeball any fixture: `DUMP=session_refresh_30 EXPAND=check,lock [ENGINE=naive]
//! cargo test -p tether-core --test layout dump -- --nocapture`.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use tether_core::diagram::{validate, Diagram, EdgeStyle, Item, MessageKind, Sequence};
use tether_core::layout::*;

// ------------------------------------------------------------------ helpers

fn diagrams_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagrams")
}

fn load(name: &str) -> Diagram {
    let src = std::fs::read_to_string(diagrams_dir().join(format!("{name}.json"))).unwrap();
    validate(&src).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn load_src(src: &str) -> Diagram {
    validate(src).unwrap_or_else(|e| panic!("{e:?}"))
}

fn ids(v: &[&str]) -> HashSet<ObjectId> {
    v.iter().map(|s| ObjectId::from(*s)).collect()
}

fn run(d: &Diagram, expanded: &[&str], engine: Engine) -> Layout {
    let ex = ids(expanded);
    let mut input = LayoutInput::new(d, &ex);
    input.opts.engine = engine;
    layout(&input).expect("layout")
}

fn golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/layout")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {name} (run with UPDATE_GOLDEN=1)"));
    assert_eq!(
        expected, actual,
        "golden {name} differs\n--- actual ---\n{actual}"
    );
}

fn is_panel_or_node(o: &Object) -> bool {
    matches!(o.kind, ObjectKind::Node { .. } | ObjectKind::Details)
}

/// Bounds, ids, and sanity common to every layout.
fn check_geometry(l: &Layout) {
    let (w, h) = (l.width, l.height);
    let mut seen = HashSet::new();
    for o in &l.objects {
        assert!(seen.insert(o.id.clone()), "duplicate object id {}", o.id);
        assert!(o.rect.w > 0 && o.rect.h > 0, "{} is empty", o.id);
        assert!(
            o.rect.right() <= w && o.rect.bottom() <= h,
            "{} outside canvas",
            o.id
        );
        if let Some(r) = o.label_rect {
            assert!(
                o.rect.contains_rect(&r) || matches!(o.kind, ObjectKind::Group { .. }),
                "{} label outside rect",
                o.id
            );
        }
        assert_eq!(l.object(o.id.as_str()).unwrap().id, o.id);
    }
    for e in &l.edges {
        assert!(
            e.points.len() >= 2,
            "{} has {} points",
            e.id,
            e.points.len()
        );
        for p in &e.points {
            assert!(
                p.x < w && p.y < h,
                "{} point {:?} outside canvas {}x{}",
                e.id,
                p,
                w,
                h
            );
        }
        for pair in e.points.windows(2) {
            assert!(
                pair[0].x == pair[1].x || pair[0].y == pair[1].y,
                "{} has a diagonal segment",
                e.id
            );
        }
        if let Some(r) = e.label_rect {
            assert!(
                r.right() <= w && r.bottom() <= h,
                "{} label outside canvas",
                e.id
            );
        }
        assert!(l.object(e.from.as_str()).is_some() && l.object(e.to.as_str()).is_some());
    }
}

/// Cells shared by two edges in the same direction (the lines would be drawn on top of each
/// other), split into vertical and horizontal. Also counts crossings (perpendicular sharing).
#[derive(Debug, Default, Clone, Copy)]
struct Metrics {
    vertical_overlap: usize,
    horizontal_overlap: usize,
    crossings: usize,
}

fn metrics(l: &Layout) -> Metrics {
    let mut cells: BTreeMap<(u16, u16), Vec<u8>> = BTreeMap::new();
    for e in &l.edges {
        for (p, bits) in polyline_cells(&e.points) {
            cells.entry((p.x, p.y)).or_default().push(bits);
        }
    }
    let mut m = Metrics::default();
    for list in cells.values() {
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let shared = list[i] & list[j];
                if shared & (UP | DOWN) != 0 {
                    m.vertical_overlap += 1;
                } else if shared & (LEFT | RIGHT) != 0 {
                    m.horizontal_overlap += 1;
                } else {
                    let (a, b) = (list[i], list[j]);
                    let a_straight_v = a & (LEFT | RIGHT) == 0 && a != 0;
                    let b_straight_h = b & (UP | DOWN) == 0 && b != 0;
                    let a_straight_h = a & (UP | DOWN) == 0 && a != 0;
                    let b_straight_v = b & (LEFT | RIGHT) == 0 && b != 0;
                    if (a_straight_v && b_straight_h) || (a_straight_h && b_straight_v) {
                        m.crossings += 1;
                    }
                }
            }
        }
    }
    m
}

fn assert_flow_quality(l: &Layout, what: &str) -> Metrics {
    check_geometry(l);
    // no two object rects overlap
    for (i, a) in l.objects.iter().enumerate() {
        for b in &l.objects[i + 1..] {
            assert!(
                !a.rect.intersects(&b.rect),
                "{what}: {} {:?} overlaps {} {:?}",
                a.id,
                a.rect,
                b.id,
                b.rect
            );
        }
    }
    // no edge cell inside any node, panel or label (the polyline starts/ends just outside)
    for e in &l.edges {
        for (p, _) in polyline_cells(&e.points) {
            for o in &l.objects {
                let blocks = is_panel_or_node(o) || matches!(o.kind, ObjectKind::EdgeLabel);
                assert!(
                    !(blocks && o.rect.contains(p)),
                    "{what}: edge {} passes through {} at {:?}",
                    e.id,
                    o.id,
                    p
                );
            }
        }
        // endpoints touch their node: first point is adjacent to the source border, last to
        // the target border
        let from = l.object(e.from.as_str()).unwrap().rect;
        let to = l.object(e.to.as_str()).unwrap().rect;
        let first = e.points[0];
        let last = *e.points.last().unwrap();
        assert!(
            adjacent(&from, first),
            "{what}: {} does not start at {}",
            e.id,
            e.from
        );
        assert!(
            adjacent(&to, last),
            "{what}: {} does not end at {}",
            e.id,
            e.to
        );
    }
    let m = metrics(l);
    assert_eq!(
        m.vertical_overlap, 0,
        "{what}: vertical collinear overlaps: {m:?}"
    );
    assert_eq!(
        m.horizontal_overlap, 0,
        "{what}: horizontal collinear overlaps: {m:?}"
    );
    // edge labels sit beside their edge
    for e in &l.edges {
        if let (Some(id), Some(r)) = (&e.label, e.label_rect) {
            let o = l.object(id.as_str()).expect("label object");
            assert_eq!(o.rect, r);
            assert_eq!(o.owner.as_ref(), Some(&e.id));
        }
    }
    m
}

fn adjacent(r: &Rect, p: Point) -> bool {
    // the cell touches the rect's border from outside (side adjacency, no diagonals)
    let (x, y) = (p.x as i32, p.y as i32);
    let (rx0, rx1) = (r.x as i32, r.right() as i32 - 1);
    let (ry0, ry1) = (r.y as i32, r.bottom() as i32 - 1);
    let in_x = x >= rx0 && x <= rx1;
    let in_y = y >= ry0 && y <= ry1;
    (in_x && (y == ry0 - 1 || y == ry1 + 1)) || (in_y && (x == rx0 - 1 || x == rx1 + 1))
}

// ------------------------------------------------------------------ flowchart

const EXPANDABLE: [&str; 6] = [
    "verify_sig",
    "load_session",
    "check",
    "lock",
    "audit",
    "serve_stale",
];

#[test]
fn flowchart_30_collapsed_has_clean_geometry() {
    let d = load("session_refresh_30");
    let l = run(&d, &[], Engine::Dot);
    let m = assert_flow_quality(&l, "collapsed");
    eprintln!(
        "session_refresh_30 collapsed: {}x{} {m:?}",
        l.width, l.height
    );
    assert_eq!(
        l.objects
            .iter()
            .filter(|o| matches!(o.kind, ObjectKind::Node { .. }))
            .count(),
        30
    );
    assert_eq!(l.edges.len(), 36);
}

#[test]
fn flowchart_30_expanded_has_clean_geometry() {
    let d = load("session_refresh_30");
    for set in [
        vec!["check"],
        vec!["check", "load_session"],
        vec!["verify_sig", "lock", "audit"],
        EXPANDABLE.to_vec(),
        // nodes without details have nothing to expand
        vec!["start", "anon", "check"],
    ] {
        let l = run(&d, &set, Engine::Dot);
        let m = assert_flow_quality(&l, &format!("expanded {set:?}"));
        eprintln!(
            "session_refresh_30 expanded {set:?}: {}x{} {m:?}",
            l.width, l.height
        );
    }
}

/// Crossing counts (perpendicular sharing of a cell by two edges) per fixture and expansion
/// set, with the Graphviz engine. The first number of each row is the count before the port and
/// lane ordering work (review fix 4, F3.5); the count may not get worse, and the small
/// documented examples must reach 0.
fn crossing_cases() -> Vec<(&'static str, &'static str, Vec<&'static str>, usize)> {
    vec![
        ("session_refresh_30", "collapsed", vec![], CROSS_SESSION_COLLAPSED),
        ("session_refresh_30", "check", vec!["check"], CROSS_SESSION_CHECK),
        (
            "session_refresh_30",
            "check+load_session",
            vec!["check", "load_session"],
            CROSS_SESSION_CHECK_LOAD,
        ),
        (
            "session_refresh_30",
            "verify_sig+lock+audit",
            vec!["verify_sig", "lock", "audit"],
            CROSS_SESSION_THREE,
        ),
        (
            "session_refresh_30",
            "all expandable",
            EXPANDABLE.to_vec(),
            CROSS_SESSION_ALL,
        ),
        ("prd_flowchart", "collapsed", vec![], CROSS_PRD),
        ("skill_example", "collapsed", vec![], 0),
        ("skill_example", "check", vec!["check"], 0),
    ]
}

// Upper bounds. Counts before the lane/port ordering fix (review fix 4): collapsed 3, check 5,
// check+load_session 5, verify_sig+lock+audit 7, all expandable 11, prd 0, skill example 0 / 2
// (check expanded). Now 3/3/3/3/3/0/0/0; the bounds hold the new values.
const CROSS_SESSION_COLLAPSED: usize = 3;
const CROSS_SESSION_CHECK: usize = 3;
const CROSS_SESSION_CHECK_LOAD: usize = 3;
const CROSS_SESSION_THREE: usize = 3;
const CROSS_SESSION_ALL: usize = 3;
const CROSS_PRD: usize = 0;
const CROSS_LR: usize = 4;
const CROSS_LR_EXP: usize = 4;
const CROSS_NAIVE: usize = 10;

#[test]
fn crossing_report() {
    for (name, what, set, max) in crossing_cases() {
        let d = load(name);
        let l = run(&d, &set, Engine::Dot);
        let m = assert_flow_quality(&l, &format!("{name} {what}"));
        eprintln!("CROSSINGS {name} [{what}]: {}", m.crossings);
        assert!(
            m.crossings <= max,
            "{name} [{what}]: {} crossings, expected at most {max}",
            m.crossings
        );
    }
}

/// Same measurement for left-to-right flowcharts and the naive engine.
#[test]
fn crossing_report_lr_and_naive() {
    let src = std::fs::read_to_string(diagrams_dir().join("session_refresh_30.json")).unwrap();
    let lr = load_src(&src.replace("\"direction\": \"TB\"", "\"direction\": \"LR\""));
    let tb = load("session_refresh_30");
    let rows: Vec<(&str, &Diagram, Vec<&str>, Engine, usize)> = vec![
        ("LR collapsed", &lr, vec![], Engine::Dot, CROSS_LR),
        ("LR check+lock", &lr, vec!["check", "lock"], Engine::Dot, CROSS_LR_EXP),
        ("naive collapsed", &tb, vec![], Engine::Naive, CROSS_NAIVE),
    ];
    for (what, d, set, engine, max) in rows {
        let l = run(d, &set, engine);
        let m = assert_flow_quality(&l, what);
        eprintln!("CROSSINGS {what}: {}", m.crossings);
        assert!(m.crossings <= max, "{what}: {} crossings, expected at most {max}", m.crossings);
    }
}

#[test]
fn expansion_attaches_panel_beside_owner_and_makes_room() {
    let d = load("session_refresh_30");
    let collapsed = run(&d, &[], Engine::Dot);
    let l = run(&d, &["check", "load_session"], Engine::Dot);
    for id in ["check", "load_session"] {
        let n = l.object(id).unwrap();
        let p = l.details_of(id).expect("panel");
        assert_eq!(p.owner.as_ref().map(|o| o.as_str()), Some(id));
        assert_eq!(p.id.as_str(), format!("{id}#details"));
        assert!(matches!(p.kind, ObjectKind::Details));
        assert!(n.expanded);
        assert!(
            p.rect.x >= n.rect.right(),
            "panel is right of its node (TB)"
        );
        assert!(
            p.rect.y < n.rect.bottom() && n.rect.y < p.rect.bottom(),
            "panel shares the node's rows"
        );
        assert!(!p.sections.is_empty());
        for s in &p.sections {
            assert!(p.rect.contains(Point {
                x: p.rect.x + 2,
                y: s.y
            }));
        }
        assert_eq!(n.details_count, 1);
    }
    // collapsed layout has no panels
    assert!(collapsed.object("check#details").is_none());
    assert!(!collapsed.object("check").unwrap().expanded);
    // the diagram grew to make room
    assert!(l.width > collapsed.width || l.height > collapsed.height);
    // a node without details never gets a panel
    let l2 = run(&d, &["start"], Engine::Dot);
    assert!(l2.details_of("start").is_none());
    assert_eq!(l2, collapsed);
    // panel text carries the detail and the code ref
    let p = l.details_of("check").unwrap();
    assert_eq!(p.sections[0].title, "Edge case");
    assert_eq!(p.sections.last().unwrap().title, "CODE");
    assert_eq!(
        p.sections.last().unwrap().body,
        vec!["src/auth/session.ts:142"]
    );
}

#[test]
fn flowchart_layout_is_deterministic() {
    let d = load("session_refresh_30");
    for set in [vec![], vec!["check", "lock"]] {
        let a = run(&d, &set, Engine::Dot);
        let b = run(&d, &set, Engine::Dot);
        assert_eq!(a, b);
        assert_eq!(ascii_debug(&a), ascii_debug(&b));
    }
}

#[test]
fn flowchart_navigation_graph() {
    let d = load("session_refresh_30");
    let l = run(&d, &[], Engine::Dot);
    let nav = l.flow_nav().unwrap();
    let id = |s: &str| ObjectId::from(s);
    assert_eq!(nav.roots, vec![id("start")]);
    assert_eq!(nav.children.len(), 30);
    assert_eq!(nav.parents.len(), 30);
    // children in left-to-right order, forward before back edges
    for (n, kids) in &nav.children {
        let xs: Vec<u16> = kids
            .iter()
            .filter(|k| nav.rank[*k] > nav.rank[n])
            .map(|k| l.object(k.as_str()).unwrap().rect.center().x)
            .collect();
        assert!(
            xs.windows(2).all(|w| w[0] <= w[1]),
            "{n}: children not left to right: {xs:?}"
        );
    }
    // spec edges are all present
    let Diagram::Flowchart(f) = &d else {
        unreachable!()
    };
    for e in &f.edges {
        assert!(
            nav.children[&id(&e.from)].contains(&id(&e.to)),
            "{} -> {}",
            e.from,
            e.to
        );
        assert!(nav.parents[&id(&e.to)].contains(&id(&e.from)));
    }
    // back edges are last
    let back = nav.children[&id("backoff")].clone();
    assert_eq!(back, vec![id("refresh")]);
    assert!(l.edges.iter().any(|e| e.back));
    // same-rank siblings are ordered by x and agree with ranks
    for row in &nav.ranks {
        let xs: Vec<u16> = row
            .iter()
            .map(|k| l.object(k.as_str()).unwrap().rect.center().x)
            .collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        for n in row {
            let mut want: Vec<ObjectId> = row.iter().filter(|o| *o != n).cloned().collect();
            let mut got = nav.siblings[n].clone();
            want.sort();
            got.sort();
            assert_eq!(want, got);
        }
    }
    let check_row = &nav.ranks[nav.rank[&id("check")]];
    let pos = check_row.iter().position(|o| *o == id("check")).unwrap();
    assert_eq!(nav.sibling("check", 1), check_row.get(pos + 1));
    assert_eq!(
        nav.sibling("check", -1),
        pos.checked_sub(1).and_then(|p| check_row.get(p))
    );
    // a decision's branches: yes goes to the left or right of no, but both are children
    // `lock` is a child of both `check` and `near_expiry`
    let mut b = nav.branch_siblings[&id("lock")].clone();
    b.sort();
    assert_eq!(b, vec![id("attach"), id("near_expiry")]);
    assert_eq!(nav.branch_siblings[&id("near_expiry")], vec![id("lock")]);
    assert_eq!(nav.branch_siblings[&id("wait_lock")], vec![id("refresh")]);
    // order covers every node once
    assert_eq!(nav.order.len(), 30);
    // looks up by id
    assert!(l.edge("edge:0").is_some());
}

#[test]
fn edge_styles_labels_and_arrows() {
    let d = load("prd_flowchart");
    let l = run(&d, &[], Engine::Dot);
    assert_flow_quality(&l, "prd");
    assert_eq!(l.edges.len(), 2);
    for e in &l.edges {
        assert_eq!(e.arrow, Dir::Down);
        assert_eq!(e.style, EdgeStyle::Solid);
        assert!(e.label.is_some());
    }
    let yes = l.object("edge:0:label").unwrap();
    assert_eq!(yes.label, vec!["yes"]);
    let n = l.object("check").unwrap();
    assert_eq!(n.details_count, 1);
    assert!(n.has_code);
    assert!(n.decor.badge.is_some() && n.decor.code.is_some() && n.decor.icon.is_some());
}

#[test]
fn naive_engine_meets_the_same_gates() {
    let d = load("session_refresh_30");
    for set in [vec![], vec!["check", "load_session"]] {
        let l = run(&d, &set, Engine::Naive);
        let m = assert_flow_quality(&l, "naive");
        eprintln!("naive {set:?}: {}x{} {m:?}", l.width, l.height);
    }
    let l = run(&load("prd_flowchart"), &[], Engine::Naive);
    assert_flow_quality(&l, "naive prd");
}

#[test]
fn missing_dot_is_a_clear_error_and_auto_falls_back() {
    let d = load("prd_flowchart");
    let ex = HashSet::new();
    let mut input = LayoutInput::new(&d, &ex);
    input.opts.dot_path = "/nonexistent/definitely-not-dot".to_string();
    let err = layout(&input).unwrap_err();
    assert!(matches!(err, LayoutError::DotMissing(..)), "{err}");
    assert!(err.to_string().contains("graphviz"), "{err}");
    input.opts.engine = Engine::Auto;
    let l = layout(&input).expect("auto falls back to the naive engine");
    assert_flow_quality(&l, "auto fallback");
    // a dot that fails
    input.opts.engine = Engine::Dot;
    input.opts.dot_path = "false".to_string();
    assert!(matches!(
        layout(&input).unwrap_err(),
        LayoutError::DotFailed { .. }
    ));
}

#[test]
fn left_to_right_flowchart_is_clean() {
    let src = std::fs::read_to_string(diagrams_dir().join("session_refresh_30.json")).unwrap();
    assert!(src.contains("\"direction\": \"TB\""));
    let d = load_src(&src.replace("\"direction\": \"TB\"", "\"direction\": \"LR\""));
    for set in [vec![], vec!["check", "lock"]] {
        let l = run(&d, &set, Engine::Dot);
        let m = assert_flow_quality(&l, &format!("LR {set:?}"));
        eprintln!("LR {set:?}: {}x{} {m:?}", l.width, l.height);
        if !set.is_empty() {
            let n = l.object("check").unwrap();
            let p = l.details_of("check").unwrap();
            assert!(p.rect.y >= n.rect.bottom(), "LR panel sits below its node");
        }
        // flow runs left to right
        let a = l.object("start").unwrap().rect.x;
        let b = l.object("continue").unwrap().rect.x;
        assert!(a < b);
    }
    golden(
        "session_refresh_30_lr.txt",
        &ascii_debug(&run(&d, &[], Engine::Dot)),
    );
}

#[test]
fn back_same_rank_and_self_edges() {
    let d = load_src(
        r#"{"type":"flowchart","title":"odd","nodes":[
            {"id":"a","label":"A"},{"id":"b","label":"B"},{"id":"c","label":"C"},
            {"id":"d","label":"D","details":[{"title":"T","body":"some body text"}]}],
          "edges":[
            {"from":"a","to":"b","label":"go"},{"from":"b","to":"c"},{"from":"c","to":"a","label":"loop back","style":"dashed"},
            {"from":"b","to":"b","label":"self"},{"from":"a","to":"d"},{"from":"d","to":"c"},{"from":"b","to":"a"}]}"#,
    );
    for engine in [Engine::Dot, Engine::Naive] {
        for set in [vec![], vec!["d"], vec!["d", "b"]] {
            let l = run(&d, &set, engine);
            assert_flow_quality(&l, &format!("{engine:?} {set:?}"));
            // self loop exists and has an arrow back into the node
            let e = l.edge("edge:3").unwrap();
            assert_eq!(e.from, e.to);
            assert!(!l.flow_nav().unwrap().children[&ObjectId::from("b")]
                .contains(&ObjectId::from("b")));
        }
    }
    let l = run(&d, &[], Engine::Dot);
    assert!(l.edge("edge:2").unwrap().back);
    assert_eq!(l.edge("edge:2").unwrap().style, EdgeStyle::Dashed);
}

#[test]
fn unknown_and_duplicate_ids_are_errors() {
    use tether_core::diagram::{Edge, Flowchart, Node};
    let node = |id: &str| Node {
        id: id.into(),
        label: id.into(),
        shape: None,
        code: None,
        details: vec![],
    };
    let mut f = Flowchart {
        title: "t".into(),
        direction: None,
        nodes: vec![node("a"), node("b")],
        edges: vec![Edge {
            from: "a".into(),
            to: "zz".into(),
            label: None,
            style: None,
        }],
    };
    let ex = HashSet::new();
    let d = Diagram::Flowchart(f.clone());
    assert!(matches!(
        layout(&LayoutInput::new(&d, &ex)),
        Err(LayoutError::UnknownId(_))
    ));
    f.edges.clear();
    f.nodes.push(node("a"));
    let d = Diagram::Flowchart(f);
    assert!(matches!(
        layout(&LayoutInput::new(&d, &ex)),
        Err(LayoutError::DuplicateId(_))
    ));
}

#[test]
fn flowchart_snapshots() {
    let d = load("session_refresh_30");
    golden(
        "session_refresh_30_collapsed.txt",
        &ascii_debug(&run(&d, &[], Engine::Dot)),
    );
    golden(
        "session_refresh_30_expanded.txt",
        &ascii_debug(&run(&d, &["check", "load_session"], Engine::Dot)),
    );
    golden(
        "prd_flowchart.txt",
        &ascii_debug(&run(&load("prd_flowchart"), &[], Engine::Dot)),
    );
}

// ------------------------------------------------------------------ sequence

fn seq_of(d: &Diagram) -> &Sequence {
    match d {
        Diagram::Sequence(s) => s,
        _ => panic!("not a sequence"),
    }
}

fn spec_item_ids(d: &Diagram) -> Vec<String> {
    let mut s = seq_of(d).clone();
    s.assign_ids();
    let mut out = Vec::new();
    s.walk(&mut |it| match it {
        Item::Message(m) => out.push(m.id.clone()),
        Item::Note(n) => out.push(n.id.clone()),
        Item::Group(_) => {}
    });
    out
}

fn assert_seq_quality(l: &Layout, d: &Diagram, what: &str) {
    check_geometry(l);
    let nav = l.seq_nav().unwrap();
    // spec order == row order, rows strictly increasing
    let want = spec_item_ids(d);
    let got: Vec<String> = nav.items.iter().map(|o| o.0.clone()).collect();
    assert_eq!(want, got, "{what}: nav order");
    let ys: Vec<u16> = nav
        .items
        .iter()
        .map(|i| l.object(i.as_str()).unwrap().rect.y)
        .collect();
    assert!(
        ys.windows(2).all(|w| w[0] < w[1]),
        "{what}: rows not monotonic {ys:?}"
    );
    // no overlaps among leaf objects
    let leaves: Vec<&Object> = l
        .objects
        .iter()
        .filter(|o| !matches!(o.kind, ObjectKind::Group { .. }))
        .collect();
    for (i, a) in leaves.iter().enumerate() {
        for b in &leaves[i + 1..] {
            assert!(
                !a.rect.intersects(&b.rect),
                "{what}: {} {:?} overlaps {} {:?}",
                a.id,
                a.rect,
                b.id,
                b.rect
            );
        }
    }
    // frames: each encloses its children, and frames are nested or disjoint
    let groups: Vec<&Object> = l
        .objects
        .iter()
        .filter(|o| matches!(o.kind, ObjectKind::Group { .. }))
        .collect();
    for g in &groups {
        for o in l.objects.iter().filter(|o| o.group.as_ref() == Some(&g.id)) {
            assert!(
                g.rect.contains_rect(&o.rect),
                "{what}: {} escapes {}",
                o.id,
                g.id
            );
        }
    }
    for (i, a) in groups.iter().enumerate() {
        for b in &groups[i + 1..] {
            let nested = a.rect.contains_rect(&b.rect) || b.rect.contains_rect(&a.rect);
            assert!(
                nested || !a.rect.intersects(&b.rect),
                "{what}: frames {} and {} partially overlap",
                a.id,
                b.id
            );
        }
    }
    // participants left to right with their lifelines
    let xs: Vec<u16> = nav
        .participants
        .iter()
        .map(|p| l.object(p.as_str()).unwrap().rect.x)
        .collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(l.lifelines.len(), nav.participants.len());
    for (ll, p) in l.lifelines.iter().zip(&nav.participants) {
        assert_eq!(&ll.participant, p);
        let r = l.object(p.as_str()).unwrap().rect;
        assert_eq!(ll.x, r.center().x);
        assert_eq!(ll.y0, r.bottom());
        assert!(ll.y1 < l.height);
        for &(a, b) in &ll.breaks {
            assert!(a <= b && a >= ll.y0 && b <= ll.y1);
        }
    }
    // every message arrow starts and ends next to its lifelines
    for e in &l.edges {
        let from = l
            .lifelines
            .iter()
            .find(|x| x.participant == e.from)
            .unwrap()
            .x as i32;
        let to = l
            .lifelines
            .iter()
            .find(|x| x.participant == e.to)
            .unwrap()
            .x as i32;
        let (a, z) = (e.points[0], *e.points.last().unwrap());
        assert_eq!((a.x as i32 - from).abs(), 1, "{what}: {} start", e.id);
        assert_eq!((z.x as i32 - to).abs(), 1, "{what}: {} end", e.id);
        let msg = l.object(e.id.as_str()).unwrap();
        assert!(msg.rect.contains(a) && msg.rect.contains(z));
    }
}

#[test]
fn sequence_collapsed_geometry() {
    let d = load("login_sequence");
    let l = run(&d, &[], Engine::Dot);
    assert_seq_quality(&l, &d, "collapsed");
    let nav = l.seq_nav().unwrap();
    assert_eq!(nav.participants.len(), 4);
    assert_eq!(nav.messages.len(), 12);
    assert_eq!(nav.items.len(), 13); // plus one note
    assert_eq!(
        nav.groups,
        vec![
            ObjectId::from("group:1"),
            "group:2".into(),
            "group:3".into()
        ]
    );
    // return arrows are dashed, sync arrows solid, async keeps its kind
    for e in &l.edges {
        let want = if e.msg_kind == Some(MessageKind::Return) {
            EdgeStyle::Dashed
        } else {
            EdgeStyle::Solid
        };
        assert_eq!(e.style, want, "{}", e.id);
    }
    assert!(l
        .edges
        .iter()
        .any(|e| e.msg_kind == Some(MessageKind::Async)));
    // direction of arrowheads follows from -> to
    let post = l.edge("m1").unwrap();
    assert_eq!(post.arrow, Dir::Right);
    let ret = l.edge("db_ret").unwrap();
    assert_eq!(ret.arrow, Dir::Left);
    // no panels, no lifeline breaks except under the note
    assert!(l
        .objects
        .iter()
        .all(|o| !matches!(o.kind, ObjectKind::Details)));
    let note = l.object("n1").unwrap();
    let auth = l
        .lifelines
        .iter()
        .find(|x| x.participant.as_str() == "auth")
        .unwrap();
    assert_eq!(auth.breaks, vec![(note.rect.y, note.rect.bottom() - 1)]);
    // the alt group carries its three branch rows (first labelled, two separators)
    let g1 = l.object("group:1").unwrap();
    match &g1.kind {
        ObjectKind::Group {
            branches, depth, ..
        } => {
            assert_eq!(*depth, 0);
            assert_eq!(branches.len(), 3);
            assert!(!branches[0].separator && branches[1].separator && branches[2].separator);
            assert!(branches.windows(2).all(|w| w[0].y < w[1].y));
            assert!(branches
                .iter()
                .all(|b| g1.rect.y < b.y && b.y < g1.rect.bottom()));
        }
        _ => unreachable!(),
    }
    // the message label is wide enough for its column gap
    let m = l.object("m2").unwrap();
    assert_eq!(m.label, vec!["verify(email, pw)"]);
    assert!(m.rect.w as usize >= "verify(email, pw)".len() + 2);
    assert_eq!(m.details_count, 2);
    assert!(m.has_code);
}

#[test]
fn sequence_expansion_shifts_later_rows_by_the_panel_height() {
    let d = load("login_sequence");
    let before = run(&d, &[], Engine::Dot);
    for target in ["m2", "n1"] {
        let after = run(&d, &[target], Engine::Dot);
        assert_seq_quality(&after, &d, &format!("expanded {target}"));
        let panel = after.details_of(target).expect("panel");
        let ph = panel.rect.h;
        let owner_after = after.object(target).unwrap();
        assert!(owner_after.expanded);
        assert_eq!(panel.owner.as_ref().map(|o| o.as_str()), Some(target));
        // the panel sits directly under its owner
        assert_eq!(panel.rect.y, owner_after.rect.bottom());
        let nav = before.seq_nav().unwrap();
        let idx = nav.items.iter().position(|i| i.as_str() == target).unwrap();
        for (k, id) in nav.items.iter().enumerate() {
            let (b, a) = (
                before.object(id.as_str()).unwrap(),
                after.object(id.as_str()).unwrap(),
            );
            if k <= idx {
                assert_eq!(b.rect.y, a.rect.y, "{id} above/at the panel must not move");
            } else {
                assert_eq!(a.rect.y, b.rect.y + ph, "{id} must shift by exactly {ph}");
            }
        }
        assert_eq!(after.height, before.height + ph);
        // groups after the panel shift too; groups that enclose it grow
        for g in before
            .objects
            .iter()
            .filter(|o| matches!(o.kind, ObjectKind::Group { .. }))
        {
            let a = after.object(g.id.as_str()).unwrap();
            assert!(a.rect.y == g.rect.y || a.rect.y == g.rect.y + ph);
            assert!(a.rect.bottom() == g.rect.bottom() + ph);
        }
        // lifelines break around the panel for every lifeline it covers
        let mut covered = 0;
        for ll in &after.lifelines {
            let hit = panel.rect.x <= ll.x && ll.x < panel.rect.right();
            let range = (panel.rect.y, panel.rect.bottom() - 1);
            let broken = ll.breaks.iter().any(|&(a, b)| a <= range.0 && b >= range.1);
            assert_eq!(
                hit, broken,
                "lifeline {} (x={}) vs panel {:?}",
                ll.participant, ll.x, panel.rect
            );
            covered += hit as usize;
        }
        assert!(covered >= 1);
        // panel content
        assert!(!panel.sections.is_empty());
        if target == "m2" {
            let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
            assert_eq!(titles, vec!["Data model", "Edge case", "CODE"]);
        }
        eprintln!(
            "{target}: panel height {ph}, canvas {}x{} -> {}x{}",
            before.width, before.height, after.width, after.height
        );
    }
    // expanding two things shifts by the sum
    let two = run(&d, &["m2", "n1"], Engine::Dot);
    let p1 = run(&d, &["m2"], Engine::Dot)
        .details_of("m2")
        .unwrap()
        .rect
        .h;
    let p2 = run(&d, &["n1"], Engine::Dot)
        .details_of("n1")
        .unwrap()
        .rect
        .h;
    assert_eq!(two.height, before.height + p1 + p2);
    assert_seq_quality(&two, &d, "two expanded");
    // a message without details cannot expand
    assert_eq!(run(&d, &["m1"], Engine::Dot), before);
}

#[test]
fn sequence_is_deterministic_and_dot_independent() {
    let d = load("login_sequence");
    let ex = ids(&["m2"]);
    let mut input = LayoutInput::new(&d, &ex);
    input.opts.dot_path = "/nonexistent/dot".into();
    let a = layout(&input).expect("sequence layout never calls graphviz");
    let b = run(&d, &["m2"], Engine::Dot);
    assert_eq!(a, b);
}

#[test]
fn sequence_self_messages_notes_and_nesting() {
    let d = load_src(
        r#"{"type":"sequence","title":"s","participants":[
            {"id":"a","label":"Alpha"},{"id":"b","label":"Beta service"},{"id":"c","label":"C"}],
          "items":[
            {"type":"message","from":"a","to":"a","label":"think about it for a long while","details":[{"title":"Why","body":"because of the thing that happens when it is slow and long"}]},
            {"type":"note","over":["a","c"],"text":"spanning note across everything that is here"},
            {"type":"group","kind":"loop","label":"retry until the remote end says it is fine","items":[
              {"type":"message","from":"a","to":"c","label":"ping"},
              {"type":"group","kind":"alt","label":"result","branches":[
                {"label":"ok","items":[{"type":"message","from":"c","to":"a","label":"pong","kind":"return"}]},
                {"items":[{"type":"note","over":["b"],"text":"inner note","details":[{"title":"x","body":"y"}]},
                          {"type":"message","from":"c","to":"c","label":"log"}]}]}]},
            {"type":"message","from":"c","to":"a","label":"done","kind":"return"}]}"#,
    );
    for set in [vec![], vec!["m1"], vec!["n2", "m1"]] {
        let l = run(&d, &set, Engine::Dot);
        assert_seq_quality(&l, &d, &format!("{set:?}"));
    }
    let l = run(&d, &[], Engine::Dot);
    // self message loops back to its own lifeline
    let e = l.edge("m1").unwrap();
    assert_eq!(e.from, e.to);
    assert_eq!(e.points.len(), 4);
    assert_eq!(e.arrow, Dir::Left);
    // nested frame is indented inside its parent
    let outer = l.object("group:1").unwrap();
    let inner = l.object("group:2").unwrap();
    assert!(outer.rect.contains_rect(&inner.rect));
    assert!(inner.rect.x > outer.rect.x && inner.rect.right() < outer.rect.right());
    match (&outer.kind, &inner.kind) {
        (ObjectKind::Group { depth: d0, .. }, ObjectKind::Group { depth: d1, .. }) => {
            assert_eq!((*d0, *d1), (0, 1))
        }
        _ => unreachable!(),
    }
    // note spans its participants
    let n = l.object("n1").unwrap();
    let xa = l.lifelines[0].x;
    let xc = l.lifelines[2].x;
    assert!(n.rect.x < xa && n.rect.right() > xc);
}

#[test]
fn sequence_snapshots() {
    let d = load("login_sequence");
    golden(
        "login_sequence_collapsed.txt",
        &ascii_debug(&run(&d, &[], Engine::Dot)),
    );
    golden(
        "login_sequence_expanded.txt",
        &ascii_debug(&run(&d, &["m2"], Engine::Dot)),
    );
}

// ------------------------------------------------------------------ dev helper

#[test]
fn dump() {
    let Ok(name) = std::env::var("DUMP") else {
        return;
    };
    let d = load(&name);
    let ex: HashSet<ObjectId> = std::env::var("EXPAND")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(ObjectId::from)
        .collect();
    let mut input = LayoutInput::new(&d, &ex);
    if std::env::var("ENGINE").as_deref() == Ok("naive") {
        input.opts.engine = Engine::Naive;
    }
    let l = layout(&input).unwrap();
    println!("{}x{} {:?}", l.width, l.height, metrics(&l));
    println!("{}", ascii_debug(&l));
}
