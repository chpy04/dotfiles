//! The per-repo diagram library through the CLI: list/open/star/rename/rm/prune/path, worktree
//! sharing, the one-time import of `.tether/diagrams/`. Every command runs with `TETHER_DATA`
//! pointing at a temp dir; nothing here touches the real library.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tether_core::testing::FakeHerdr;

struct Env {
    root: tempfile::TempDir,
    repo: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

impl Env {
    fn new() -> Env {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        Env { root, repo }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn cmd_in(&self, cwd: &Path, herdr: Option<&FakeHerdr>) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tether"));
        c.env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_WORKSPACE_ID")
            .env_remove("HERDR_TAB_ID")
            .env_remove("HERDR_PANE_ID")
            .env("TETHER_DATA", self.data())
            .env("TETHER_STATE", self.root.path().join("state.json"))
            .env("TETHER_CONFIG", self.root.path().join("config.toml"))
            .current_dir(cwd);
        if let Some(h) = herdr {
            c.env("HERDR_SOCKET_PATH", h.path()).env("HERDR_WORKSPACE_ID", "w1").env("HERDR_TAB_ID", "w1:t1").env("HERDR_PANE_ID", "w1:p9");
        }
        c
    }

    fn cmd(&self) -> Command {
        self.cmd_in(&self.repo, None)
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    fn spec(&self, name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tether-core/tests/fixtures/diagrams").join(format!("{name}.json")).to_string_lossy().into_owned()
    }

    fn create(&self, name: &str) -> String {
        let o = self.run(&["diagram", "create", &self.spec(name), "--open", "none"]);
        assert!(o.status.success(), "{}", err(&o));
        out(&o).trim().to_string()
    }

    fn list_json(&self, extra: &[&str]) -> Vec<Value> {
        let mut args = vec!["diagram", "list", "--json"];
        args.extend_from_slice(extra);
        let o = self.run(&args);
        assert!(o.status.success(), "{}", err(&o));
        serde_json::from_str::<Value>(&out(&o)).unwrap().as_array().unwrap().clone()
    }

    fn folder(&self) -> PathBuf {
        PathBuf::from(out(&self.run(&["diagram", "path"])).trim())
    }

    fn index_path(&self) -> PathBuf {
        self.folder().parent().unwrap().join("index.json")
    }

    fn edit_index(&self, f: impl FnOnce(&mut Value)) {
        let p = self.index_path();
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(p, v.to_string()).unwrap();
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn create_stores_under_the_data_dir_and_never_touches_the_repo() {
    let env = Env::new();
    let o = env.run(&["diagram", "create", &env.spec("prd_flowchart"), "--open", "none", "--json"]);
    assert!(o.status.success(), "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let id = v["id"].as_str().unwrap();
    let path = PathBuf::from(v["path"].as_str().unwrap());
    assert!(path.starts_with(env.data().join("repos")), "{path:?}");
    assert!(path.is_file());
    assert_eq!(v["starred"], false);
    assert!(!env.repo.join(".tether").exists());
    assert!(!env.repo.join(".gitignore").exists(), "no more .gitignore edits");
    let folder = env.folder();
    assert_eq!(folder, path.parent().unwrap());
    assert!(folder.parent().unwrap().join("repo.json").is_file());
    let row = &env.list_json(&[])[0];
    assert_eq!(row["id"], id);
    assert_eq!(row["branch"], "main");
    assert_eq!(row["worktree"], "proj");
    assert_eq!(row["starred"], false);
}

#[test]
fn create_star_and_the_list_table() {
    let env = Env::new();
    let a = env.create("prd_flowchart");
    let o = env.run(&["diagram", "create", &env.spec("login_sequence"), "--open", "none", "--star"]);
    let b = out(&o).trim().to_string();
    let t = out(&env.run(&["diagram", "list"]));
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].contains("ID") && lines[0].contains("TYPE") && lines[0].contains("BRANCH") && lines[0].contains("AGE") && lines[0].contains("TITLE"), "{t}");
    let star = lines.iter().find(|l| l.contains(&b)).unwrap();
    assert!(star.starts_with('★') && star.contains("sequence") && star.contains("main") && star.contains("now") && star.contains("Login request"), "{star}");
    let plain = lines.iter().find(|l| l.contains(&a)).unwrap();
    assert!(!plain.contains('★'));

    let only = env.list_json(&["--starred"]);
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["id"], b.as_str());
    let t = out(&env.run(&["diagram", "list", "--starred"]));
    assert!(t.contains(&b) && !t.contains(&a));
}

#[test]
fn star_unstar_rename_rm_and_path() {
    let env = Env::new();
    let a = env.create("prd_flowchart");
    let b = env.create("login_sequence");

    let o = env.run(&["diagram", "star", &a]);
    assert!(o.status.success() && out(&o).contains("starred"), "{}", err(&o));
    assert!(out(&env.run(&["diagram", "star", &a])).contains("already starred"));
    assert_eq!(env.list_json(&["--starred"]).len(), 1);
    assert!(out(&env.run(&["diagram", "unstar", &a])).contains("unstarred"));
    assert!(env.list_json(&["--starred"]).is_empty());
    // by title words too
    assert!(env.run(&["diagram", "star", "login"]).status.success());
    assert_eq!(env.list_json(&["--starred"])[0]["id"], b.as_str());
    assert_eq!(env.run(&["diagram", "star", "ghost-000000"]).status.code(), Some(2));

    // rename: the title in the spec changes too, the id stays
    let o = env.run(&["diagram", "rename", &a, "Brand", "new", "title"]);
    assert!(o.status.success(), "{}", err(&o));
    let shown: Value = serde_json::from_str(&out(&env.run(&["diagram", "show", &a]))).unwrap();
    assert_eq!(shown["title"], "Brand new title");
    let rows = env.list_json(&[]);
    assert!(rows.iter().any(|r| r["id"] == a.as_str() && r["title"] == "Brand new title"));
    assert!(env.folder().join(format!("{a}.json")).is_file());
    assert!(std::fs::read_to_string(env.folder().join(format!("{a}.json"))).unwrap().contains("Brand new title"));
    assert_eq!(env.run(&["diagram", "rename", &a, "  "]).status.code(), Some(2));

    // rm several; a bad id is reported but the good one is removed; `delete` still works
    let o = env.run(&["diagram", "rm", &a, "nope-123456"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(out(&o).contains(&format!("deleted {a}")) && err(&o).contains("nope-123456"));
    assert_eq!(env.list_json(&[]).len(), 1);
    assert!(env.run(&["diagram", "delete", &b]).status.success());
    assert!(env.list_json(&[]).is_empty());
    assert!(out(&env.run(&["diagram", "list"])).contains("no diagrams saved"));

    // path prints the folder
    assert!(out(&env.run(&["diagram", "path"])).trim().starts_with(env.data().join("repos").to_str().unwrap()));
}

#[test]
fn worktrees_share_one_library() {
    let env = Env::new();
    let wt = env.root.path().join("proj-feature");
    git(&env.repo, &["worktree", "add", "-q", "-b", "feat-x", wt.to_str().unwrap()]);
    let id = env.create("prd_flowchart");
    let o = env.cmd_in(&wt, None).args(["diagram", "list", "--json"]).output().unwrap();
    let rows: Vec<Value> = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id.as_str());
    let o = env.cmd_in(&wt, None).args(["diagram", "create", &env.spec("login_sequence"), "--open", "none"]).output().unwrap();
    assert!(o.status.success());
    let rows = env.list_json(&[]);
    assert_eq!(rows.len(), 2, "created in the worktree, visible from the main checkout");
    let seq = rows.iter().find(|r| r["type"] == "sequence").unwrap();
    assert_eq!((seq["branch"].as_str(), seq["worktree"].as_str()), (Some("feat-x"), Some("proj-feature")));
    assert_eq!(out(&env.cmd_in(&wt, None).args(["diagram", "path"]).output().unwrap()), out(&env.run(&["diagram", "path"])));
}

#[test]
fn list_all_repos_includes_other_repos() {
    let env = Env::new();
    env.create("prd_flowchart");
    let other = env.root.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "trunk"]);
    let o = env.cmd_in(&other, None).args(["diagram", "create", &env.spec("login_sequence"), "--open", "none"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(env.list_json(&[]).len(), 1, "default: this repo only");
    let all = env.list_json(&["--all-repos"]);
    assert_eq!(all.len(), 2);
    let mut repos: Vec<_> = all.iter().map(|r| r["repo"].as_str().unwrap().to_string()).collect();
    repos.sort();
    assert_eq!(repos, vec!["other".to_string(), "proj".to_string()]);
    let t = out(&env.run(&["diagram", "list", "--all-repos"]));
    assert!(t.contains("REPO") && t.contains("other") && t.contains("trunk"), "{t}");
}

#[test]
fn open_matches_titles_fuzzily_and_reports_ambiguity() {
    let env = Env::new();
    let a = env.create("prd_flowchart"); // "Session refresh"
    let b = env.create("login_sequence"); // "Login request"
    let c = env.create("session_refresh_30"); // "Auth session refresh flow"

    // ambiguous: lists the candidates, exit 2, opens nothing
    let o = env.run(&["diagram", "open", "SESSION", "--placement", "here"]);
    assert_eq!(o.status.code(), Some(2));
    let e = err(&o);
    assert!(e.contains("matches 2 diagrams") && e.contains(&a) && e.contains(&c) && !e.contains(&b), "{e}");
    // no match
    let o = env.run(&["diagram", "open", "kubernetes"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("no saved diagram matches"), "{}", err(&o));

    // a unique substring opens a tab named after the title (fake Herdr), case-insensitively
    let h = FakeHerdr::start(|method, params| match method {
        "tab.list" => Ok(json!({"type":"tab_list","tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"}]})),
        "tab.create" => Ok(json!({"type":"tab_created",
            "tab":{"tab_id":"w1:t7","workspace_id":"w1","number":2,"label":params["label"]},
            "root_pane":{"pane_id":"w1:p7","workspace_id":"w1","tab_id":"w1:t7"}})),
        "pane.get" => Ok(json!({"type":"pane_info","pane":{"pane_id":params["pane_id"],"workspace_id":"w1","tab_id":"w1:t1"}})),
        "pane.send_input" => Ok(json!({"type":"ok"})),
        _ => Err(("invalid_request".into(), format!("unexpected {method}"))),
    });
    let o = env.cmd_in(&env.repo, Some(&h)).args(["diagram", "open", "LOGIN", "req", "--placement", "tab"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("opened tab \"Login request\""), "{}", out(&o));
    assert_eq!(h.requests_for("tab.create")[0].params["label"], "Login request");
    let sent = h.requests_for("pane.send_input");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].params["pane_id"], "w1:p7");
    let text = sent[0].params["text"].as_str().unwrap();
    assert!(text.contains(&format!("view {b}")) && text.contains("TETHER_DATA"), "the viewer pane gets the data dir: {text}");

    // an id works as well, and an exact title wins over longer matches
    let o = env.cmd_in(&env.repo, Some(&h)).args(["diagram", "open", &a, "--placement", "tab"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    env.run(&["diagram", "rename", &c, "Session"]);
    let o = env.cmd_in(&env.repo, Some(&h)).args(["diagram", "open", "session", "--placement", "tab"]).output().unwrap();
    assert!(o.status.success(), "exact title \"Session\" beats \"Session refresh\": {}", err(&o));
}

#[test]
fn viewing_records_last_opened() {
    let env = Env::new();
    let id = env.create("prd_flowchart");
    assert!(env.list_json(&[])[0]["last_opened"].is_null());
    // no terminal here, so the viewer itself refuses, but opening was recorded first
    let o = env.run(&["view", &id]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("interactive terminal"), "{}", err(&o));
    assert!(env.list_json(&[])[0]["last_opened"].as_u64().unwrap() > 0);
}

#[test]
fn prune_dry_run_real_stars_and_keep_days() {
    let env = Env::new();
    let old = env.create("prd_flowchart");
    let old_starred = env.create("login_sequence");
    let fresh = env.create("session_refresh_30");
    env.run(&["diagram", "star", &old_starred]);
    let long_ago = 1_000_000_000u64; // 2001
    env.edit_index(|v| {
        for d in v["diagrams"].as_array_mut().unwrap() {
            if d["id"] == old.as_str() || d["id"] == old_starred.as_str() {
                d["updated"] = json!(long_ago);
                d["created"] = json!(long_ago);
            }
        }
        v["last_prune"] = json!(u64::MAX / 2); // far in the future: automatic pruning stays out of the way
    });

    let o = env.run(&["diagram", "prune", "--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("would remove") && out(&o).contains(&old) && !out(&o).contains(&old_starred) && !out(&o).contains(&fresh), "{}", out(&o));
    assert_eq!(env.list_json(&[]).len(), 3, "dry run removes nothing");

    let o = env.run(&["diagram", "prune"]);
    assert!(out(&o).contains(&format!("removed {old}")), "{}", out(&o));
    let ids: Vec<String> = env.list_json(&[]).iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    assert!(!ids.contains(&old) && ids.contains(&old_starred) && ids.contains(&fresh), "{ids:?}");
    assert!(!env.folder().join(format!("{old}.json")).exists());
    assert!(out(&env.run(&["diagram", "prune"])).contains("nothing to prune"));

    // keep_days = 0 turns pruning off
    std::fs::write(env.root.path().join("config.toml"), "[diagrams]\nkeep_days = 0\n").unwrap();
    let o = env.run(&["diagram", "prune"]);
    assert!(out(&o).contains("pruning is off"), "{}", out(&o));
    assert_eq!(env.list_json(&[]).len(), 2);
}

#[test]
fn list_prunes_automatically_once_a_day_and_says_so() {
    let env = Env::new();
    let old = env.create("prd_flowchart");
    let keep = env.create("login_sequence");
    env.edit_index(|v| {
        for d in v["diagrams"].as_array_mut().unwrap() {
            if d["id"] == old.as_str() {
                d["updated"] = json!(1_000_000_000u64);
            }
        }
        v["last_prune"] = json!(0);
    });
    let o = env.run(&["diagram", "list", "--json"]);
    let rows: Vec<Value> = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], keep.as_str());
    assert!(err(&o).contains("pruned 1 unstarred diagram"), "{}", err(&o));
    assert!(env.list_json(&[])[0]["id"] == keep.as_str());
    // keep_days from the config is honoured: with 100000 days nothing is old
    let env2 = Env::new();
    let id = env2.create("prd_flowchart");
    std::fs::write(env2.root.path().join("config.toml"), "[diagrams]\nkeep_days = 100000\n").unwrap();
    env2.edit_index(|v| {
        v["diagrams"][0]["updated"] = json!(1_000_000_000u64);
        v["last_prune"] = json!(0);
    });
    assert_eq!(env2.list_json(&[])[0]["id"], id.as_str());
}

#[test]
fn legacy_worktree_diagrams_are_imported_once_with_a_stderr_note() {
    let env = Env::new();
    // an old-style store inside the worktree
    let legacy = env.repo.join(".tether/diagrams");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::copy(env.spec("prd_flowchart"), legacy.join("old-one-abc123.json")).unwrap();
    std::fs::copy(env.spec("login_sequence"), legacy.join("old-two-def456.json")).unwrap();

    let o = env.run(&["diagram", "list", "--json"]);
    let rows: Vec<Value> = serde_json::from_str(&out(&o)).unwrap();
    let mut ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, vec!["old-one-abc123", "old-two-def456"], "ids are kept");
    let e = err(&o);
    assert!(e.contains("imported 2 diagrams") && e.contains("the old folder can be deleted"), "{e}");
    assert_eq!(e.lines().filter(|l| l.contains("imported")).count(), 1, "one line: {e}");
    assert!(rows.iter().all(|r| r["updated"].as_u64().unwrap() > 1_000_000_000), "updated comes from the file mtime");
    assert!(legacy.join("old-one-abc123.json").is_file(), "old files stay");

    // once only
    env.run(&["diagram", "rm", "old-one-abc123"]);
    let o = env.run(&["diagram", "list", "--json"]);
    assert!(!err(&o).contains("imported"), "{}", err(&o));
    assert_eq!(serde_json::from_str::<Vec<Value>>(&out(&o)).unwrap().len(), 1);
    // the viewer path imports too (a fresh data dir, `show` is the first command)
    let env2 = Env::new();
    let l2 = env2.repo.join(".tether/diagrams");
    std::fs::create_dir_all(&l2).unwrap();
    std::fs::copy(env2.spec("prd_flowchart"), l2.join("first-000001.json")).unwrap();
    let o = env2.run(&["diagram", "show", "first-000001"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("imported 1 diagram "), "{}", err(&o));
}

#[test]
fn outside_git_the_cwd_is_the_repo() {
    let env = Env::new();
    let plain = env.root.path().join("not-a-repo");
    std::fs::create_dir_all(&plain).unwrap();
    let o = env.cmd_in(&plain, None).args(["diagram", "create", &env.spec("prd_flowchart"), "--open", "none"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let o = env.cmd_in(&plain, None).args(["diagram", "list", "--json"]).output().unwrap();
    let rows: Vec<Value> = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["branch"].is_null());
    assert_eq!(rows[0]["repo"], "not-a-repo");
    assert!(env.list_json(&[]).is_empty(), "the git repo has its own library");
}

#[test]
fn diagrams_browser_needs_a_terminal() {
    let env = Env::new();
    for args in [vec!["diagrams"], vec!["diagram", "browse"]] {
        let o = env.run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
        assert!(err(&o).contains("interactive terminal"), "{}", err(&o));
    }
}
