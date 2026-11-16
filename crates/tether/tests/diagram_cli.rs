//! `tether diagram …`, `tether view` placement and `tether send --diagram` against a fake Herdr.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use tether_core::testing::FakeHerdr;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let e = Env { dir: tempfile::tempdir().unwrap() };
        // a worktree with the source files the fixture links to
        let src = e.dir.path().join("src/auth");
        std::fs::create_dir_all(&src).unwrap();
        let body: Vec<String> =
            (1..=200).map(|n| if n == 142 { "if (token.expiresAt < Date.now()) {".to_string() } else { format!("// line {n}") }).collect();
        std::fs::write(src.join("session.ts"), body.join("\n")).unwrap();
        e
    }

    fn cmd(&self, herdr: Option<&FakeHerdr>) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tether"));
        c.env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_WORKSPACE_ID")
            .env_remove("HERDR_TAB_ID")
            .env_remove("HERDR_PANE_ID")
            .env("TETHER_STATE", self.dir.path().join("state.json"))
            .env("TETHER_CONFIG", self.dir.path().join("config.toml"))
            .current_dir(self.dir.path());
        if let Some(h) = herdr {
            c.env("HERDR_SOCKET_PATH", h.path())
                .env("HERDR_WORKSPACE_ID", "w1")
                .env("HERDR_TAB_ID", "w1:t1")
                .env("HERDR_PANE_ID", "w1:p9");
        }
        c
    }

    fn spec(&self, name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tether-core/tests/fixtures/diagrams").join(format!("{name}.json"));
        p.to_string_lossy().into_owned()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn create(env: &Env, name: &str) -> String {
    let o = env.cmd(None).args(["diagram", "create", &env.spec(name), "--open", "none"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    out(&o).trim().to_string()
}

#[test]
fn bad_specs_give_readable_errors_and_exit_2() {
    let env = Env::new();
    for name in ["invalid_flowchart_unknown_edge", "invalid_flowchart_bad_shape", "invalid_sequence_unknown_participant", "invalid_flowchart_duplicate_ids"] {
        let o = env.cmd(None).args(["diagram", "create", &env.spec(name), "--open", "none"]).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{name}: {}", err(&o));
        let e = err(&o);
        assert!(e.contains("tether: error: invalid diagram spec"), "{name}: {e}");
        assert!(e.contains("- /"), "{name}: errors are located by JSON pointer: {e}");
        assert!(e.contains("schema/flowchart.schema.json"), "{name}: points at the schemas: {e}");
    }
    // nothing was stored
    let o = env.cmd(None).args(["diagram", "list"]).output().unwrap();
    assert!(out(&o).contains("no diagrams"), "{}", out(&o));
    // not JSON at all, from stdin
    let mut child = env.cmd(None).args(["diagram", "create", "-", "--open", "none"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"{ nope").unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("invalid JSON"), "{}", err(&o));
    // missing file
    let o = env.cmd(None).args(["diagram", "create", "/nonexistent/spec.json"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    // bad placement
    let o = env.cmd(None).args(["diagram", "create", &env.spec("prd_flowchart"), "--open", "sideways"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("expected tab, split, here or none"));
}

#[test]
fn create_list_show_export_update_delete_round_trip() {
    let env = Env::new();
    let id = create(&env, "prd_flowchart");
    assert!(id.starts_with("session-refresh-"), "{id}");
    let id2 = create(&env, "login_sequence");

    // list (table + json)
    let o = env.cmd(None).args(["diagram", "list"]).output().unwrap();
    let t = out(&o);
    assert!(t.contains(&id) && t.contains("flowchart") && t.contains("Session refresh") && t.contains(&id2) && t.contains("sequence"), "{t}");
    let o = env.cmd(None).args(["diagram", "list", "--json"]).output().unwrap();
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(v.as_array().unwrap().iter().any(|r| r["id"] == json!(id) && r["type"] == "flowchart" && r["title"] == "Session refresh"));

    // --json create output
    let o = env.cmd(None).args(["diagram", "create", &env.spec("prd_flowchart"), "--open", "none", "--json"]).output().unwrap();
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["type"], "flowchart");
    assert_eq!(v["opened"]["mode"], "none");
    assert!(Path::new(v["path"].as_str().unwrap()).exists());

    // show = the stored spec
    let o = env.cmd(None).args(["diagram", "show", &id]).output().unwrap();
    let shown: Value = serde_json::from_str(&out(&o)).unwrap();
    let original: Value = serde_json::from_str(&std::fs::read_to_string(env.spec("prd_flowchart")).unwrap()).unwrap();
    assert_eq!(shown["title"], original["title"]);
    assert_eq!(shown["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(shown["nodes"][0]["code"]["anchor"], "token.expiresAt <");

    // export mermaid
    let o = env.cmd(None).args(["diagram", "export", &id, "--mermaid"]).output().unwrap();
    let m = out(&o);
    assert!(m.starts_with("flowchart TD") && m.contains("Token expired?") && m.contains("yes"), "{m}");
    let o = env.cmd(None).args(["diagram", "export", &id2, "--mermaid"]).output().unwrap();
    assert!(out(&o).starts_with("sequenceDiagram"), "{}", out(&o));

    // update: new title, same id
    let mut changed = original.clone();
    changed["title"] = json!("Session refresh v2");
    let mut child = env.cmd(None).args(["diagram", "update", &id, "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(changed.to_string().as_bytes()).unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    let o = env.cmd(None).args(["diagram", "list", "--json"]).output().unwrap();
    assert!(out(&o).contains("Session refresh v2"));
    // an invalid update is refused and leaves the stored one alone
    let o = env.cmd(None).args(["diagram", "update", &id, &env.spec("invalid_flowchart_bad_shape")]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(out(&env.cmd(None).args(["diagram", "show", &id]).output().unwrap()).contains("Session refresh v2"));

    // delete
    let o = env.cmd(None).args(["diagram", "delete", &id]).output().unwrap();
    assert!(o.status.success());
    let o = env.cmd(None).args(["diagram", "show", &id]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("diagram not found"), "{}", err(&o));
    // stored under .tether/diagrams
    assert!(env.dir.path().join(".tether/diagrams").is_dir());
}

#[test]
fn render_prints_the_diagram_without_a_terminal() {
    let env = Env::new();
    let id = create(&env, "prd_flowchart");
    let o = env.cmd(None).args(["diagram", "render", &id, "--focus", "check", "--no-icons"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&o);
    assert!(t.contains("┏━") && t.contains("Token expired?") && t.contains("Call /token/refresh"), "{t}");
    let o = env.cmd(None).args(["diagram", "render", &env.spec("login_sequence"), "--expand", "m2", "--ansi"]).output().unwrap();
    assert!(out(&o).contains("\x1b[38;2;") && out(&o).contains("DATA MODEL"), "{}", err(&o));
    let o = env.cmd(None).args(["diagram", "render", &id, "--focus", "nope"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

fn fake_herdr() -> FakeHerdr {
    FakeHerdr::start(|method, params| match method {
        "tab.list" => Ok(json!({"type":"tab_list","tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"}]})),
        "tab.create" => Ok(json!({"type":"tab_created",
            "tab":{"tab_id":"w1:t7","workspace_id":"w1","number":2,"label":params["label"]},
            "root_pane":{"pane_id":"w1:p7","workspace_id":"w1","tab_id":"w1:t7"}})),
        "pane.get" => Ok(json!({"type":"pane_info","pane":{"pane_id":params["pane_id"],"workspace_id":"w1","tab_id":"w1:t1"}})),
        "pane.split" => Ok(json!({"type":"pane_created","pane":{"pane_id":"w1:p8","workspace_id":"w1","tab_id":"w1:t1"}})),
        "pane.send_input" => Ok(json!({"type":"ok"})),
        "agent.list" => Ok(json!({"type":"agent_list","agents":[
            {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"}]})),
        "agent.prompt" => Ok(json!({"type":"agent_prompted","agent":{"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"}})),
        _ => Err(("invalid_request".into(), format!("unexpected {method}"))),
    })
}

#[test]
fn create_open_tab_names_the_tab_after_the_title_and_runs_the_viewer() {
    let env = Env::new();
    let h = fake_herdr();
    let o = env.cmd(Some(&h)).args(["diagram", "create", &env.spec("prd_flowchart")]).output().unwrap(); // default placement: tab
    assert!(o.status.success(), "{}", err(&o));
    let id = out(&o).trim().to_string();
    let create = &h.requests_for("tab.create")[0].params;
    assert_eq!(create["label"], "Session refresh");
    assert_eq!(create["workspace_id"], "w1");
    assert_eq!(create["focus"], false, "does not steal focus by default");
    let sent = h.requests_for("pane.send_input");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].params["pane_id"], "w1:p7", "only the pane we created receives input");
    let text = sent[0].params["text"].as_str().unwrap();
    assert!(text.starts_with("exec "), "{text}");
    assert!(text.contains("env TETHER_CONFIG=") && text.contains("TETHER_STATE="), "env is carried into the pane: {text}");
    assert!(text.ends_with(&format!("tether view {id}")) || text.ends_with(&format!(" view {id}")), "{text}");
    // the tab is recorded as tether-made (state file written by the layout helpers)
    let st = std::fs::read_to_string(env.dir.path().join("state.json")).unwrap();
    assert!(st.contains("w1:t7"), "{st}");
    assert!(err(&o).contains("opened tab \"Session refresh\""), "{}", err(&o));

    // json summary
    let o = env.cmd(Some(&h)).args(["diagram", "create", &env.spec("login_sequence"), "--json", "--focus"]).output().unwrap();
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["opened"]["mode"], "tab");
    assert_eq!(v["opened"]["tab_id"], "w1:t7");
    assert_eq!(h.requests_for("tab.create")[1].params["focus"], true);
}

#[test]
fn create_open_split_and_view_placement() {
    let env = Env::new();
    let h = fake_herdr();
    let o = env.cmd(Some(&h)).args(["diagram", "create", &env.spec("prd_flowchart"), "--open", "split", "--pane", "w1:p3"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let id = out(&o).trim().to_string();
    let split = &h.requests_for("pane.split")[0].params;
    assert_eq!((split["target_pane_id"].as_str(), split["direction"].as_str()), (Some("w1:p3"), Some("right")));
    assert_eq!(h.requests_for("pane.send_input")[0].params["pane_id"], "w1:p8");
    // `tether view <id> --placement tab`
    let o = env.cmd(Some(&h)).args(["view", &id, "--placement", "tab"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("opened tab \"Session refresh\""), "{}", out(&o));
    // unknown id
    let o = env.cmd(Some(&h)).args(["view", "nope-123", "--placement", "tab"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("diagram not found"));
    // outside Herdr: the diagram is still saved, with a pointer to reopen it
    let o = env.cmd(None).args(["diagram", "create", &env.spec("prd_flowchart")]).output().unwrap();
    assert_eq!(o.status.code(), Some(3));
    assert!(err(&o).contains("diagram saved as") && err(&o).contains("tether view"), "{}", err(&o));
}

#[test]
fn placement_comes_from_the_config_when_open_is_omitted() {
    let env = Env::new();
    std::fs::write(env.dir.path().join("config.toml"), "[placement]\ndiagram = \"none\"\n").unwrap();
    let h = fake_herdr();
    let o = env.cmd(Some(&h)).args(["diagram", "create", &env.spec("prd_flowchart")]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(h.requests().is_empty(), "placement none touches nothing: {:?}", h.requests());
}

#[test]
fn send_diagram_payload_matches_the_prd_example() {
    let env = Env::new();
    let id = create(&env, "prd_flowchart");
    let h = fake_herdr();
    let o = env
        .cmd(Some(&h))
        .args(["send", "--to", "primary", "--diagram", &id, "--node", "check", "--text", "Why client clock instead of server time here?"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let text = h.requests_for("agent.prompt")[0].params["text"].as_str().unwrap().to_string();
    let wt = std::fs::canonicalize(env.dir.path()).unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        text,
        format!("[comment] diagram \"Session refresh\" › node \"Token expired?\"  src/auth/session.ts:142  (worktree: {wt})\n  142 │ if (token.expiresAt < Date.now()) {{\nWhy client clock instead of server time here?")
    );
    // a node without code: no ref, no code line
    let o = env.cmd(Some(&h)).args(["send", "--diagram", &id, "--node", "ok", "--text", "fine?", "--json"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["ref"]["diagram"], json!(id));
    assert_eq!(v["ref"]["node"], "ok");
    let text = h.requests_for("agent.prompt")[1].params["text"].as_str().unwrap().to_string();
    assert_eq!(text, format!("[comment] diagram \"Session refresh\" › node \"Continue request\"  (worktree: {wt})\nfine?"));
    // stale link: marker, no code line
    std::fs::write(env.dir.path().join("src/auth/session.ts"), "nothing\n").unwrap();
    env.cmd(Some(&h)).args(["send", "--diagram", &id, "--node", "check", "--text", "still there?"]).output().unwrap();
    let text = h.requests_for("agent.prompt")[2].params["text"].as_str().unwrap().to_string();
    assert!(text.contains("src/auth/session.ts:142  [stale link]  (worktree:") && !text.contains(" │ "), "{text}");
}

#[test]
fn send_diagram_sequence_messages_and_bad_ids() {
    let env = Env::new();
    let id = create(&env, "login_sequence");
    let h = fake_herdr();
    let o = env.cmd(Some(&h)).args(["send", "--diagram", &id, "--node", "m2", "--text", "why 423?"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let text = h.requests_for("agent.prompt")[0].params["text"].as_str().unwrap().to_string();
    assert!(text.starts_with("[comment] diagram \"Login request\" › message \"verify(email, pw)\"  src/auth/verify.ts:31"), "{text}");
    assert!(text.ends_with("why 423?"));
    // unknown node lists the valid ids, unknown diagram too; neither touches Herdr
    let n = h.requests().len();
    let o = env.cmd(Some(&h)).args(["send", "--diagram", &id, "--node", "zzz", "--text", "x"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("no element `zzz`") && err(&o).contains("m1"), "{}", err(&o));
    let o = env.cmd(Some(&h)).args(["send", "--diagram", "gone-000000", "--node", "m1", "--text", "x"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(h.requests().len(), n);
    // --diagram needs --node, and conflicts with --ref
    let o = env.cmd(Some(&h)).args(["send", "--diagram", &id, "--text", "x"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let o = env.cmd(Some(&h)).args(["send", "--diagram", &id, "--node", "m1", "--ref", "a:1", "--text", "x"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn view_without_a_terminal_is_a_clear_usage_error() {
    let env = Env::new();
    let id = create(&env, "prd_flowchart");
    let o = env.cmd(None).args(["view", &id]).stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("interactive terminal"), "{}", err(&o));
}
