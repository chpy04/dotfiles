//! End-to-end tests of the `tether` binary against a fake Herdr socket.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use tether_core::testing::FakeHerdr;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env { dir: tempfile::tempdir().unwrap() }
    }

    fn cmd(&self, herdr: Option<&FakeHerdr>) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tether"));
        c.env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_WORKSPACE_ID")
            .env_remove("HERDR_TAB_ID")
            .env_remove("HERDR_PANE_ID")
            .env("TETHER_STATE", self.dir.path().join("state.json"))
            // never touch the real diagram library
            .env("TETHER_DATA", self.dir.path().join("tether-data"))
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
}

fn run(c: &mut Command) -> Output {
    c.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn fake(prompt: Result<Value, (String, String)>) -> FakeHerdr {
    FakeHerdr::start(move |method, _| match method {
        "agent.list" => Ok(json!({"type":"agent_list","agents":[
            {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"},
            {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t2","agent":"codex","agent_status":"working"},
            {"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","agent":"claude","agent_status":"idle"}]})),
        "tab.list" => Ok(json!({"type":"tab_list","tabs":[
            {"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"},
            {"tab_id":"w1:t2","workspace_id":"w1","number":2,"label":"tests"}]})),
        "agent.prompt" => prompt.clone(),
        _ => Err(("invalid_request".into(), "?".into())),
    })
}

fn prompt_ok() -> Result<Value, (String, String)> {
    Ok(json!({"type":"agent_prompted","agent":{"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"}}))
}

#[test]
fn agents_json_lists_workspace_agents_with_primary() {
    let env = Env::new();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).args(["agents", "--json"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2); // w2 agent filtered out
    assert_eq!(rows[0]["pane_id"], "w1:p2");
    assert_eq!(rows[0]["tab_index"], 1);
    assert_eq!(rows[0]["tab_label"], "1");
    assert_eq!(rows[0]["kind"], "claude");
    assert_eq!(rows[0]["state"], "idle");
    assert_eq!(rows[0]["is_primary"], true);
    assert_eq!(rows[1]["is_primary"], false);
    assert_eq!(rows[1]["tab_label"], "tests");
    // `agents` is read-only: it must not write the primary tag
    assert!(!env.dir.path().join("state.json").exists());
    // sending tags the primary on first sight
    let o = run(env.cmd(Some(&h)).args(["send", "--ref", "x.txt:1", "--text", "hi"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let st: Value = serde_json::from_str(&std::fs::read_to_string(env.dir.path().join("state.json")).unwrap()).unwrap();
    assert_eq!(st["workspaces"]["w1"]["primary_pane"], "w1:p2");
}

#[test]
fn slash_body_is_fine_with_the_default_template_but_refused_when_text_comes_first() {
    let env = Env::new();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).args(["send", "--ref", "a.rs:3", "--text", "/clear the cache?"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let text = h.requests_for("agent.prompt")[0].params["text"].as_str().unwrap().to_string();
    assert!(text.starts_with("[comment] ") && text.ends_with("/clear the cache?"), "{text}");
    // a template that begins with {text} brings the body check back
    std::fs::write(env.dir.path().join("config.toml"), "[comment]\ntemplate = \"{text} ({path}:{line})\"\n").unwrap();
    let before = h.requests().len();
    let o = run(env.cmd(Some(&h)).args(["send", "--ref", "a.rs:3", "--text", "  /compact"]));
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert_eq!(h.requests().len(), before, "nothing sent");
}

#[test]
fn foreign_pane_is_refused_unless_workspace_matches_or_allow_foreign() {
    let env = Env::new();
    let h = FakeHerdr::start(|method, params| match method {
        "agent.list" => Ok(json!({"type":"agent_list","agents":[
            {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"}]})),
        "tab.list" => Ok(json!({"type":"tab_list","tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"}]})),
        "agent.get" => Ok(json!({"type":"agent_info","agent":{"pane_id":params["target"],"workspace_id":"w2","tab_id":"w2:t1","agent":"claude","agent_status":"idle"}})),
        "agent.prompt" => prompt_ok(),
        _ => Err(("invalid_request".into(), "?".into())),
    });
    let base = ["send", "--to", "w2:p1", "--ref", "x.txt:1", "--text", "hi"];
    let o = run(env.cmd(Some(&h)).args(base));
    assert_eq!(o.status.code(), Some(6), "{}", stderr(&o));
    assert!(stderr(&o).contains("--workspace w2"), "{}", stderr(&o));
    assert!(h.requests_for("agent.prompt").is_empty());
    let o = run(env.cmd(Some(&h)).args(base).args(["--workspace", "w3"]));
    assert_eq!(o.status.code(), Some(6));
    let o = run(env.cmd(Some(&h)).args(base).args(["--workspace", "w2"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(env.cmd(Some(&h)).args(base).arg("--allow-foreign"));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(h.requests_for("agent.prompt").len(), 2);
}

#[test]
fn agents_table_human_output() {
    let env = Env::new();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).arg("agents"));
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("PRIMARY") && out.contains("w1:p2") && out.contains("codex"), "{out}");
}

#[test]
fn send_delivers_formatted_payload_via_agent_prompt() {
    let env = Env::new();
    let file = env.dir.path().join("src/a.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "fn one() {}\nfn two() {}\nfn three() {}\n").unwrap();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).args(["send", "--to", "primary", "--ref", "src/a.rs:2-3", "--text", "Why two?\nAnd three?"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("sent to claude · tab 1"), "{}", stdout(&o));
    let reqs = h.requests_for("agent.prompt");
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].params["target"], "w1:p2");
    let text = reqs[0].params["text"].as_str().unwrap();
    let dirname = std::fs::canonicalize(env.dir.path()).unwrap();
    let wt = dirname.file_name().unwrap().to_string_lossy().into_owned();
    // tempdirs are not git repos (normally): worktree falls back to the cwd basename
    assert!(text.starts_with("[comment] src/a.rs:2-3  (worktree: "), "{text}");
    assert!(text.contains(&wt) || text.contains("(worktree: "), "{text}");
    assert!(text.contains("\n  2 │ fn two() {}\n  3 │ fn three() {}\n"), "{text}");
    assert!(text.ends_with("Why two?\nAnd three?"), "{text}");
    assert!(reqs[0].params.get("wait").is_none());
}

#[test]
fn send_json_output_and_pane_target() {
    let env = Env::new();
    std::fs::write(env.dir.path().join("x.txt"), "hello\n").unwrap();
    let h = FakeHerdr::start(|method, _| match method {
        "agent.list" => Ok(json!({"type":"agent_list","agents":[
            {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t2","agent":"codex","agent_status":"working"}]})),
        "tab.list" => Ok(json!({"type":"tab_list","tabs":[{"tab_id":"w1:t2","workspace_id":"w1","number":2,"label":"tests"}]})),
        "agent.prompt" => Ok(json!({"type":"agent_prompted","agent":{"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t2","agent":"codex","agent_status":"working"}})),
        _ => Err(("invalid_request".into(), "?".into())),
    });
    let o = run(env.cmd(Some(&h)).args(["send", "--to", "w1:p5", "--ref", "x.txt:1", "--text", "-leading dash ok", "--json"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["pane_id"], "w1:p5");
    assert_eq!(v["tab_index"], 2);
    assert_eq!(v["state_before"], "working");
    assert_eq!(h.requests_for("agent.prompt")[0].params["target"], "w1:p5");
}

#[test]
fn send_text_from_stdin() {
    let env = Env::new();
    std::fs::write(env.dir.path().join("x.txt"), "hello\n").unwrap();
    let h = fake(prompt_ok());
    let mut child = env
        .cmd(Some(&h))
        .args(["send", "--ref", "x.txt:1", "--text-file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all("from stdin\nline two\n".as_bytes()).unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let text = h.requests_for("agent.prompt")[0].params["text"].as_str().unwrap().to_string();
    assert!(text.ends_with("from stdin\nline two"), "{text}");
}

#[test]
fn bad_input_exits_2_and_never_touches_herdr() {
    let env = Env::new();
    let h = fake(prompt_ok());
    for args in [
        vec!["send", "--ref", "no-colon", "--text", "hi"],
        vec!["send", "--ref", "a.rs:0", "--text", "hi"],
        vec!["send", "--ref", "a.rs:3", "--text", "   "],
        vec!["send", "--diagram", "d1", "--node", "n1", "--text", "hi"],
    ] {
        let o = run(env.cmd(Some(&h)).args(&args));
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains("tether: error:"), "{args:?}: {}", stderr(&o));
    }
    assert!(h.requests().is_empty(), "no Herdr traffic on bad input: {:?}", h.requests());
    // clap-level: missing --ref/--diagram
    let o = run(env.cmd(Some(&h)).args(["send", "--text", "hi"]));
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--ref"));
    assert!(h.requests().is_empty());
}

#[test]
fn outside_herdr_is_exit_3_with_clear_message() {
    let env = Env::new();
    let o = run(env.cmd(None).args(["send", "--ref", "a.rs:1", "--text", "hi"]));
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("not inside a Herdr workspace"), "{}", stderr(&o));
    let o = run(env.cmd(None).args(["agents", "--json"]));
    assert_eq!(o.status.code(), Some(3));
    // --json also reports the error on stdout
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["kind"], "not_in_herdr");
}

#[test]
fn herdr_unreachable_is_exit_4() {
    let env = Env::new();
    let mut c = env.cmd(None);
    c.env("HERDR_SOCKET_PATH", "/nonexistent/herdr.sock").env("HERDR_WORKSPACE_ID", "w1");
    let o = run(c.args(["send", "--ref", "a.rs:1", "--text", "hi"]));
    assert_eq!(o.status.code(), Some(4), "{}", stderr(&o));
    assert!(stderr(&o).contains("cannot connect to Herdr socket"), "{}", stderr(&o));
}

#[test]
fn blocked_agent_refusal_is_exit_6_with_hint() {
    let env = Env::new();
    let h = fake(Err(("agent_blocked".into(), "agent w1:p2 is blocked".into())));
    let o = run(env.cmd(Some(&h)).args(["send", "--ref", "a.rs:1", "--text", "hi"]));
    assert_eq!(o.status.code(), Some(6));
    let e = stderr(&o);
    assert!(e.contains("agent_blocked") && e.contains("tether: hint:"), "{e}");
}

#[test]
fn unknown_target_is_exit_5() {
    let env = Env::new();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).args(["send", "--to", "nobody", "--ref", "a.rs:1", "--text", "hi"]));
    assert_eq!(o.status.code(), Some(5), "{}", stderr(&o));
    assert!(h.requests_for("agent.prompt").is_empty());
}

#[test]
fn config_commands() {
    let env = Env::new();
    // defaults without a file
    let o = run(env.cmd(None).args(["config", "show", "--json"]));
    assert!(o.status.success());
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["keys"]["comment"], "<leader>ac");
    assert_eq!(v["keys"]["comment_pick"], "<leader>aC");
    assert_eq!(v["comment"]["max_code_lines"], 20);
    let o = run(env.cmd(None).args(["config", "path"]));
    assert_eq!(stdout(&o).trim(), env.dir.path().join("config.toml").to_str().unwrap());
    // init writes, second init refuses
    let o = run(env.cmd(None).args(["config", "init"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(env.dir.path().join("config.toml").exists());
    let o = run(env.cmd(None).args(["config", "init"]));
    assert_eq!(o.status.code(), Some(7));
    assert!(stderr(&o).contains("refusing to overwrite"));
    // override via file
    std::fs::write(env.dir.path().join("config.toml"), "[keys]\ncomment = \"<leader>cc\"\n[nvim]\ncommand = \"nvim --clean\"\n").unwrap();
    let o = run(env.cmd(None).args(["config", "show", "--json"]));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["keys"]["comment"], "<leader>cc");
    assert_eq!(v["keys"]["comment_pick"], "<leader>aC");
    let o = run(env.cmd(None).args(["config", "get", "nvim.command"]));
    assert_eq!(stdout(&o).trim(), "nvim --clean");
    let o = run(env.cmd(None).args(["config", "show"]));
    assert!(stdout(&o).contains("[keys]") && stdout(&o).contains("<leader>cc"));
    // invalid config -> exit 7 mentioning the file
    std::fs::write(env.dir.path().join("config.toml"), "[comment]\ntemplate = \"no placeholder\"\n").unwrap();
    let o = run(env.cmd(None).args(["config", "show"]));
    assert_eq!(o.status.code(), Some(7));
    assert!(stderr(&o).contains("config.toml") && stderr(&o).contains("{text}"), "{}", stderr(&o));
}

#[test]
fn custom_template_from_config_is_used() {
    let env = Env::new();
    std::fs::write(env.dir.path().join("x.txt"), "alpha\n").unwrap();
    std::fs::write(
        env.dir.path().join("config.toml"),
        "[comment]\ntemplate = \"REVIEW {path}#{line}\\n{code}\\n>> {text}\"\n",
    )
    .unwrap();
    let h = fake(prompt_ok());
    let o = run(env.cmd(Some(&h)).args(["send", "--ref", "x.txt:1", "--text", "q?"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let text = h.requests_for("agent.prompt")[0].params["text"].as_str().unwrap().to_string();
    assert_eq!(text, "REVIEW x.txt#1\n  1 │ alpha\n>> q?");
}
