//! Phase 2 (layout tools) end-to-end tests of the `tether` binary against a fake Herdr socket.

use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tether_core::testing::FakeHerdr;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env { dir: tempfile::tempdir().unwrap() }
    }
    fn cmd(&self, h: &FakeHerdr) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tether"));
        c.env("HERDR_SOCKET_PATH", h.path())
            .env("HERDR_WORKSPACE_ID", "w1")
            .env("HERDR_TAB_ID", "w1:t1")
            .env("HERDR_PANE_ID", "w1:p1")
            .env("TETHER_STATE", self.dir.path().join("state.json"))
            // never touch the real diagram library
            .env("TETHER_DATA", self.dir.path().join("tether-data"))
            .env("TETHER_CONFIG", self.dir.path().join("config.toml"))
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .current_dir(self.dir.path());
        c
    }
    fn state(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.dir.path().join("state.json")).unwrap()).unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Workspace w1 with a hand-made tab 2; tab.create/close mutate the list.
fn fake() -> FakeHerdr {
    let tabs = Arc::new(Mutex::new(vec![
        json!({"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1","focused":true}),
        json!({"tab_id":"w1:t2","workspace_id":"w1","number":2,"label":"my notes"}),
    ]));
    FakeHerdr::start(move |method, params| match method {
        "tab.list" => Ok(json!({"type":"tab_list","tabs":tabs.lock().unwrap().clone()})),
        "tab.get" => Err(("tab_not_found".into(), "no".into())),
        "tab.create" => {
            let mut t = tabs.lock().unwrap();
            let n = t.len() + 1;
            let id = format!("w1:t{}", n + 10);
            let tab = json!({"tab_id":id,"workspace_id":"w1","number":n,"label":params["label"]});
            t.push(tab.clone());
            Ok(json!({"type":"tab_created","tab":tab,"root_pane":{"pane_id":format!("w1:p{}", n + 10),"workspace_id":"w1","tab_id":id}}))
        }
        "tab.close" => {
            let id = params["tab_id"].clone();
            tabs.lock().unwrap().retain(|t| t["tab_id"] != id);
            Ok(json!({"type":"ok"}))
        }
        "session.snapshot" => Ok(json!({"type":"session_snapshot","snapshot":{
            "workspaces":[{"workspace_id":"w1","label":"task"}],
            "tabs":tabs.lock().unwrap().clone(),
            "panes":[
                {"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","focused":true,"agent":"claude","agent_status":"working"},
                {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t2","focused":false},
                {"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","focused":false}],
            "agents":[{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"working"}],
            "layouts":[]}})),
        "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{"pane_id":params["pane_id"],"foreground_processes":[{"pid":1,"name":"zsh","argv":["zsh"]}]}})),
        "pane.get" => match params["pane_id"].as_str().unwrap() {
            "w2:p1" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1"}})),
            id => Ok(json!({"type":"pane_info","pane":{"pane_id":id,"workspace_id":"w1","tab_id":"w1:t1"}})),
        },
        "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p30","workspace_id":"w1","tab_id":"w1:t1"}})),
        "pane.send_input" => Ok(json!({"type":"ok"})),
        _ => Err(("invalid_request".into(), method.into())),
    })
}

#[test]
fn layout_json_and_text_show_scoped_tabs_with_ownership() {
    let env = Env::new();
    let h = fake();
    let o = env.cmd(&h).args(["tab", "open", "session refresh", "--", "echo", "hi there"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("opened tab 3 \"session refresh\" (w1:t13) pane w1:p13"), "{}", out(&o));
    assert_eq!(h.requests_for("pane.send_input")[0].params["text"], "echo 'hi there'");
    // recorded in the state file, keyed by workspace
    assert_eq!(env.state()["workspaces"]["w1"]["tabs"][0]["tab_id"], "w1:t13");
    assert_eq!(env.state()["workspaces"]["w1"]["tabs"][0]["label"], "session refresh");

    let o = env.cmd(&h).args(["layout", "--json"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["workspace_id"], "w1");
    let tabs = v["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[0]["created_by_tether"], false);
    assert_eq!(tabs[1]["label"], "my notes");
    assert_eq!(tabs[1]["created_by_tether"], false);
    assert_eq!(tabs[2]["created_by_tether"], true);
    assert_eq!(tabs[0]["panes"][0]["agent"]["state"], "working");
    assert_eq!(tabs[0]["panes"][0]["primary"], true);
    assert_eq!(tabs[1]["panes"][0]["command"], "zsh");

    let o = env.cmd(&h).arg("layout").output().unwrap();
    let t = out(&o);
    assert!(t.contains("tab 2 \"my notes\" w1:t2  [yours]"), "{t}");
    assert!(t.contains("tab 3 \"session refresh\" w1:t13  [tether-made]"), "{t}");
    assert!(t.contains("agent claude/working  ·  primary"), "{t}");
}

#[test]
fn closing_a_hand_made_tab_is_refused_with_exit_6_and_a_reason() {
    let env = Env::new();
    let h = fake();
    let o = env.cmd(&h).args(["tab", "close", "my notes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(err(&o).contains("tether did not create it"), "{}", err(&o));
    assert!(h.requests_for("tab.close").is_empty());
    // --json error shape
    let o = env.cmd(&h).args(["tab", "close", "2", "--json"]).output().unwrap();
    assert_eq!(o.status.code(), Some(6));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!((v["ok"].clone(), v["error"]["kind"].clone()), (json!(false), json!("refused")));
    // tether-made tab closes; --force closes the hand-made one
    assert!(env.cmd(&h).args(["tab", "open", "diagram"]).output().unwrap().status.success());
    let o = env.cmd(&h).args(["tab", "close", "diagram"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(env.state()["workspaces"]["w1"].get("tabs"), None);
    let o = env.cmd(&h).args(["tab", "close", "my notes", "--force"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn split_prints_the_new_pane_id_and_refuses_other_workspaces() {
    let env = Env::new();
    let h = fake();
    let o = env.cmd(&h).args(["split", "down", "--ratio", "0.3", "--", "tail", "-f", "log"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o).trim(), "w1:p30");
    let sp = &h.requests_for("pane.split")[0].params;
    assert_eq!((sp["target_pane_id"].clone(), sp["direction"].clone()), (json!("w1:p1"), json!("down")));
    let o = env.cmd(&h).args(["split", "right", "--json"]).output().unwrap();
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!((v["pane_id"].clone(), v["split_from"].clone(), v["ok"].clone()), (json!("w1:p30"), json!("w1:p1"), json!(true)));
    // another workspace's pane: refused unless --workspace names it
    let n = h.requests_for("pane.split").len();
    let o = env.cmd(&h).args(["split", "right", "--pane", "w2:p1"]).output().unwrap();
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(err(&o).contains("--workspace w2"), "{}", err(&o));
    assert_eq!(h.requests_for("pane.split").len(), n);
    let o = env.cmd(&h).args(["split", "right", "--pane", "w2:p1", "--workspace", "w2"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    // bad input
    assert_eq!(env.cmd(&h).args(["split", "left"]).output().unwrap().status.code(), Some(2));
    assert_eq!(env.cmd(&h).args(["split", "right", "--ratio", "1.5"]).output().unwrap().status.code(), Some(2));
}

#[test]
fn view_unknown_diagram_is_usage_error_and_outside_herdr_is_exit_3() {
    let env = Env::new();
    let h = fake();
    let o = env.cmd(&h).args(["view", "some-id"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("diagram not found"), "{}", err(&o));
    let o = env.cmd(&h).env_remove("HERDR_WORKSPACE_ID").arg("layout").output().unwrap();
    assert_eq!(o.status.code(), Some(3));
    // an explicit --workspace works without HERDR_WORKSPACE_ID
    let o = env.cmd(&h).env_remove("HERDR_WORKSPACE_ID").args(["layout", "--workspace", "w1", "--json"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn code_open_reports_missing_file_and_bad_ref() {
    let env = Env::new();
    let h = fake();
    let o = env.cmd(&h).args(["code", "open", "nope.ts:3"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("file not found"), "{}", err(&o));
    let o = env.cmd(&h).args(["code", "open", "nonsense"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("expected path:line"), "{}", err(&o));
    assert!(h.requests_for("pane.split").is_empty());
}
