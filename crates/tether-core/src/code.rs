//! `tether code open` (F1.3, key flow 2): jump a tether-registered Neovim in the target tab to
//! `file:line`, or split a pane and start one. Never opens a second Neovim in a tab that has one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::config::Placement;
use crate::error::{Result, TetherError};
use crate::herdr::{Client, Pane, PaneSplit};
use crate::nvim;
use crate::panes::{self, Scope};
use crate::send::{self, CodeRef};
use crate::tabs;
use crate::workspace::StateStore;

/// What `code open` needs from Neovim, so the decision logic is testable without one.
pub trait Editor {
    /// If a tether-registered Neovim lives in `pane_id` and answers: its cursor (`path:line`).
    fn probe(&self, pane_id: &str) -> Option<String>;
    /// Jump that Neovim; returns `path:line` as Neovim reports it.
    fn jump(&self, pane_id: &str, file: &Path, line: u32, end_line: Option<u32>) -> Result<String>;
    /// Wait until the Neovim of `pane_id` answers.
    fn wait_ready(&self, pane_id: &str, timeout: Duration) -> bool;
}

/// The real thing: sockets under `<runtime>/tether/`.
pub struct SocketEditor {
    pub runtime: PathBuf,
}

impl Editor for SocketEditor {
    fn probe(&self, pane_id: &str) -> Option<String> {
        nvim::probe(&nvim::socket_path_in(&self.runtime, pane_id))
    }

    fn jump(&self, pane_id: &str, file: &Path, line: u32, end_line: Option<u32>) -> Result<String> {
        let sock = nvim::socket_path_in(&self.runtime, pane_id);
        nvim::jump(&sock, file, line, end_line)
            .map_err(|e| TetherError::Other(format!("could not jump Neovim in pane {pane_id}: {e}")))
    }

    fn wait_ready(&self, pane_id: &str, timeout: Duration) -> bool {
        nvim::wait_ready(&nvim::socket_path_in(&self.runtime, pane_id), timeout)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// A registered Neovim already lives in this pane: jump it.
    Reuse { pane_id: String },
    /// No Neovim in the tab: split `from` and start one.
    Split { from: String },
}

/// Pure decision. `tab_panes` are the panes of the target tab; `alive(pane)` says whether a
/// tether-registered Neovim answers there. `explicit_from` is `--from`; `current` is the caller's
/// own pane (used as the reference only when it is in the target tab).
pub fn decide(
    tab_panes: &[Pane],
    alive: &dyn Fn(&str) -> bool,
    explicit_from: Option<&str>,
    current: Option<&str>,
) -> Result<Decision> {
    if let Some(p) = tab_panes.iter().find(|p| alive(&p.pane_id)) {
        return Ok(Decision::Reuse { pane_id: p.pane_id.clone() });
    }
    let in_tab = |id: &str| tab_panes.iter().any(|p| p.pane_id == id);
    let from = match explicit_from {
        Some(f) if in_tab(f) => f.to_string(),
        Some(f) => {
            return Err(TetherError::Input(format!("--from {f} is not a pane of the target tab (see `tether layout`)")))
        }
        None => current
            .filter(|c| in_tab(c))
            .map(str::to_string)
            .or_else(|| tab_panes.iter().find(|p| p.focused).map(|p| p.pane_id.clone()))
            .or_else(|| tab_panes.first().map(|p| p.pane_id.clone()))
            .ok_or_else(|| TetherError::Target("the target tab has no panes".into()))?,
    };
    Ok(Decision::Split { from })
}

/// Path of the `tether-nvim` wrapper: `$TETHER_NVIM_WRAPPER`, else `bin/tether-nvim` next to the
/// repo the binary was built in, else `tether-nvim` from PATH.
pub fn locate_wrapper() -> String {
    locate_wrapper_from(std::env::var_os("TETHER_NVIM_WRAPPER"), std::env::current_exe().ok())
}

pub fn locate_wrapper_from(env: Option<std::ffi::OsString>, exe: Option<PathBuf>) -> String {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    if let Some(e) = env.filter(|e| !e.is_empty()) {
        return PathBuf::from(e).to_string_lossy().into_owned();
    }
    if let Some(exe) = exe {
        let exe = exe.canonicalize().unwrap_or(exe);
        for dir in exe.ancestors().skip(1).take(5) {
            for cand in [dir.join("bin/tether-nvim"), dir.join("tether-nvim")] {
                if is_exec(&cand) {
                    return cand.to_string_lossy().into_owned();
                }
            }
        }
    }
    "tether-nvim".to_string()
}

/// Resolve the reference path: absolute as is; relative against the git worktree root of `cwd`,
/// else against `cwd`. The file must exist.
pub fn resolve_file(path: &str, cwd: &Path) -> Result<PathBuf> {
    let p = Path::new(path);
    let mut tried = Vec::new();
    let mut cands = Vec::new();
    if p.is_absolute() {
        cands.push(p.to_path_buf());
    } else {
        if let Some(root) = send::git_toplevel(&cwd.join(".")) {
            cands.push(root.join(p));
        }
        cands.push(cwd.join(p));
    }
    for c in cands {
        if c.is_file() {
            return Ok(c.canonicalize().unwrap_or(c));
        }
        tried.push(c.display().to_string());
    }
    Err(TetherError::Input(format!("file not found: {path} (tried {})", tried.join(", "))))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeOpened {
    pub pane_id: String,
    /// `true`: an existing Neovim was jumped; `false`: a new split was created.
    pub reused: bool,
    pub tab_id: String,
    pub file: String,
    pub line: u32,
    pub end_line: Option<u32>,
    /// `path:line` as Neovim reports it after the jump.
    pub at: String,
    /// The pane that was split (only when not reused).
    pub split_from: Option<String>,
    pub warnings: Vec<String>,
}

pub struct CodeOpen<'a> {
    pub reference: &'a CodeRef,
    pub file: PathBuf,
    /// `--in`: tab index, id or label. Default: the `--from` pane's tab, else the current tab.
    pub in_tab: Option<&'a str>,
    pub from: Option<&'a str>,
    pub focus: bool,
    pub ready_timeout: Duration,
    pub wrapper: String,
    pub placement: &'a Placement,
}

pub fn open(client: &Client, store: &StateStore, scope: &Scope, editor: &dyn Editor, o: &CodeOpen) -> Result<CodeOpened> {
    // 1. which tab
    let tab = match (o.in_tab, o.from) {
        (Some(spec), _) => tabs::resolve_scoped(client, scope, store, spec)?,
        (None, Some(from)) => {
            let p = panes::pane_in_scope(client, scope, from)?;
            tabs::resolve_scoped(client, scope, store, &p.tab_id)?
        }
        (None, None) => match &scope.tab_id {
            Some(t) => tabs::resolve_scoped(client, scope, store, t)?,
            None => {
                return Err(TetherError::Input(
                    "no current tab (HERDR_TAB_ID is not set): pass --in <tab> (see `tether layout`)".into(),
                ))
            }
        },
    };
    let tab_panes: Vec<Pane> = client.pane_list(Some(&scope.workspace_id))?.into_iter().filter(|p| p.tab_id == tab.tab_id).collect();

    // 2. reuse or split
    let decision = decide(&tab_panes, &|id| editor.probe(id).is_some(), o.from, scope.pane_id.as_deref())?;
    let r = o.reference;
    let mut warnings = Vec::new();
    let (pane_id, reused, split_from) = match decision {
        Decision::Reuse { pane_id } => (pane_id, true, None),
        Decision::Split { from } => {
            // a Neovim that tether did not start cannot be driven: say so rather than hide it
            for p in &tab_panes {
                if let Ok(info) = client.pane_process_info(&p.pane_id) {
                    if info.foreground_processes.iter().any(|f| f.name == "nvim") {
                        warnings.push(format!(
                            "pane {} runs a Neovim without a tether socket (not started via tether-nvim); opened a new one",
                            p.pane_id
                        ));
                    }
                }
            }
            let cwd = send::git_toplevel(&o.file)
                .or_else(|| o.file.parent().map(Path::to_path_buf))
                .map(|p| p.to_string_lossy().into_owned());
            let direction = panes::parse_direction(&o.placement.code_direction)?;
            let ratio = panes::check_ratio(o.placement.code_ratio)?;
            let new = client.pane_split(&PaneSplit {
                target_pane_id: &from,
                direction,
                ratio: Some(ratio),
                cwd: cwd.as_deref(),
                focus: false,
            })?;
            let args = vec![o.wrapper.clone(), format!("+{}", r.line), o.file.to_string_lossy().into_owned()];
            panes::run_in_pane(client, &new.pane_id, &args)?;
            if !editor.wait_ready(&new.pane_id, o.ready_timeout) {
                return Err(TetherError::Other(format!(
                    "Neovim in new pane {} did not answer within {:?}; the pane was left open (is `{}` runnable there?)",
                    new.pane_id, o.ready_timeout, o.wrapper
                )));
            }
            (new.pane_id, false, Some(from))
        }
    };

    // 3. jump (also after a fresh start: confirms the position and applies the range highlight)
    let at = editor.jump(&pane_id, &o.file, r.line, r.end_line)?;
    if o.focus {
        client.pane_focus(&pane_id)?;
    }
    Ok(CodeOpened {
        pane_id,
        reused,
        tab_id: tab.tab_id,
        file: o.file.to_string_lossy().into_owned(),
        line: r.line,
        end_line: r.end_line,
        at,
        split_from,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHerdr;
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::HashSet;

    fn pane(id: &str, tab: &str, focused: bool) -> Pane {
        Pane { pane_id: id.into(), workspace_id: "w1".into(), tab_id: tab.into(), focused, ..Default::default() }
    }

    #[test]
    fn decide_reuses_a_live_neovim_and_never_splits_then() {
        let panes = vec![pane("w1:p1", "w1:t1", true), pane("w1:p2", "w1:t1", false), pane("w1:p3", "w1:t1", false)];
        let alive = |id: &str| id == "w1:p3";
        assert_eq!(decide(&panes, &alive, None, Some("w1:p1")).unwrap(), Decision::Reuse { pane_id: "w1:p3".into() });
        // even with an explicit --from, an existing Neovim wins
        assert_eq!(decide(&panes, &alive, Some("w1:p2"), None).unwrap(), Decision::Reuse { pane_id: "w1:p3".into() });
    }

    #[test]
    fn decide_splits_the_right_reference_pane() {
        let panes = vec![pane("w1:p1", "w1:t1", false), pane("w1:p2", "w1:t1", true)];
        let none = |_: &str| false;
        assert_eq!(decide(&panes, &none, Some("w1:p1"), Some("w1:p2")).unwrap(), Decision::Split { from: "w1:p1".into() }); // --from
        assert_eq!(decide(&panes, &none, None, Some("w1:p1")).unwrap(), Decision::Split { from: "w1:p1".into() }); // current
        assert_eq!(decide(&panes, &none, None, Some("w9:p9")).unwrap(), Decision::Split { from: "w1:p2".into() }); // current not in tab: focused
        assert_eq!(decide(&[pane("w1:p4", "w1:t1", false)], &none, None, None).unwrap(), Decision::Split { from: "w1:p4".into() }); // first
        assert!(matches!(decide(&panes, &none, Some("w1:p8"), None).unwrap_err(), TetherError::Input(_)));
        assert!(matches!(decide(&[], &none, None, None).unwrap_err(), TetherError::Target(_)));
    }

    #[derive(Default)]
    struct MockEditor {
        alive: HashSet<String>,
        jumps: RefCell<Vec<(String, u32, Option<u32>)>>,
        ready_after_split: bool,
    }

    impl Editor for MockEditor {
        fn probe(&self, pane_id: &str) -> Option<String> {
            self.alive.contains(pane_id).then(|| "/x/a.ts:1".to_string())
        }
        fn jump(&self, pane_id: &str, file: &Path, line: u32, end: Option<u32>) -> Result<String> {
            self.jumps.borrow_mut().push((pane_id.to_string(), line, end));
            Ok(format!("{}:{line}", file.display()))
        }
        fn wait_ready(&self, _: &str, _: Duration) -> bool {
            self.ready_after_split
        }
    }

    fn fake() -> FakeHerdr {
        FakeHerdr::start(|method, params| match method {
            "tab.list" => Ok(json!({"type":"tab_list","tabs":[
                {"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"},
                {"tab_id":"w1:t4","workspace_id":"w1","number":2,"label":"diagram"}]})),
            "pane.list" => Ok(json!({"type":"pane_list","panes":[
                {"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","focused":true},
                {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t4","focused":false},
                {"pane_id":"w1:p6","workspace_id":"w1","tab_id":"w1:t4","focused":false}]})),
            "pane.get" => Ok(json!({"type":"pane_info","pane":{"pane_id":params["pane_id"],"workspace_id":"w1","tab_id":"w1:t4"}})),
            "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p9","workspace_id":"w1","tab_id":"w1:t4"}})),
            "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{"pane_id":params["pane_id"],"foreground_processes":[{"pid":1,"name":"zsh","argv":["zsh"]}]}})),
            "pane.send_input" | "pane.focus" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), method.to_string())),
        })
    }

    fn scope() -> Scope {
        Scope { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()), explicit_other: false }
    }

    fn run(f: &FakeHerdr, ed: &MockEditor, in_tab: Option<&str>, from: Option<&str>, r: &str) -> Result<CodeOpened> {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("a.ts");
        std::fs::write(&file, "x\n").unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let reference: CodeRef = r.parse().unwrap();
        let placement = Placement { diagram: "tab".into(), code_direction: "right".into(), code_ratio: 0.4 };
        open(
            &Client::new(f.path()),
            &store,
            &scope(),
            ed,
            &CodeOpen { reference: &reference, file, in_tab, from, focus: false, ready_timeout: Duration::from_millis(50), wrapper: "/repo/bin/tether-nvim".into(), placement: &placement },
        )
    }

    #[test]
    fn reuses_the_neovim_in_the_target_tab_without_splitting() {
        let f = fake();
        let ed = MockEditor { alive: ["w1:p5".to_string()].into(), ..Default::default() };
        let r = run(&f, &ed, Some("diagram"), None, "a.ts:42-45").unwrap();
        assert!(r.reused);
        assert_eq!((r.pane_id.as_str(), r.tab_id.as_str(), r.split_from), ("w1:p5", "w1:t4", None));
        assert_eq!(ed.jumps.borrow()[0], ("w1:p5".to_string(), 42, Some(45)));
        assert!(f.requests_for("pane.split").is_empty(), "no second Neovim in a tab that has one");
        assert!(f.requests_for("pane.send_input").is_empty());
    }

    #[test]
    fn a_neovim_in_another_tab_does_not_count() {
        let f = fake();
        // p1 is in tab 1 and alive; the target is tab "diagram" which has none -> split
        let ed = MockEditor { alive: ["w1:p1".to_string()].into(), ready_after_split: true, ..Default::default() };
        let r = run(&f, &ed, Some("2"), Some("w1:p5"), "a.ts:7").unwrap();
        assert!(!r.reused);
        assert_eq!((r.pane_id.as_str(), r.split_from.as_deref()), ("w1:p9", Some("w1:p5")));
        let sp = &f.requests_for("pane.split")[0].params;
        assert_eq!(sp["target_pane_id"], "w1:p5");
        assert_eq!(sp["direction"], "right");
        assert!((sp["ratio"].as_f64().unwrap() - 0.4).abs() < 1e-6, "ratio comes from [placement]");
        assert_eq!(sp["focus"], false);
        let text = f.requests_for("pane.send_input")[0].params["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("/repo/bin/tether-nvim +7 "), "{text}");
        assert!(text.ends_with("a.ts"), "{text}");
        assert_eq!(ed.jumps.borrow()[0].0, "w1:p9");
    }

    #[test]
    fn startup_timeout_reports_the_pane_left_open() {
        let f = fake();
        let ed = MockEditor { ready_after_split: false, ..Default::default() };
        let e = run(&f, &ed, Some("diagram"), None, "a.ts:7").unwrap_err();
        assert!(e.to_string().contains("w1:p9") && e.to_string().contains("did not answer"), "{e}");
        assert!(ed.jumps.borrow().is_empty());
    }

    #[test]
    fn default_tab_is_the_current_one_and_other_workspace_is_refused() {
        let f = fake();
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        // no --in: current tab w1:t1 (pane p1 is the reference)
        let r = run(&f, &ed, None, None, "a.ts:3").unwrap();
        assert_eq!(r.tab_id, "w1:t1");
        // pane.list in the fake has p1 only in t1, so the split reference is the current pane
        assert_eq!(f.requests_for("pane.split")[0].params["target_pane_id"], "w1:p1");
        // a tab id of another workspace is refused (tab.get unknown in this fake -> not found)
        assert!(run(&f, &ed, Some("w2:t1"), None, "a.ts:3").is_err());
    }

    #[test]
    fn resolve_file_prefers_worktree_root_and_requires_existence() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::process::Command::new("git").arg("-C").arg(&root).args(["init", "-q"]).status().unwrap();
        std::fs::create_dir_all(root.join("src/sub")).unwrap();
        std::fs::write(root.join("src/a.ts"), "1\n").unwrap();
        std::fs::write(root.join("src/sub/b.ts"), "1\n").unwrap();
        // from a subdirectory, a root-relative path resolves against the worktree root
        assert_eq!(resolve_file("src/a.ts", &root.join("src/sub")).unwrap(), root.join("src/a.ts"));
        // a cwd-relative path works when it is not root-relative
        assert_eq!(resolve_file("b.ts", &root.join("src/sub")).unwrap(), root.join("src/sub/b.ts"));
        // absolute
        assert_eq!(resolve_file(root.join("src/a.ts").to_str().unwrap(), Path::new("/")).unwrap(), root.join("src/a.ts"));
        let e = resolve_file("nope.ts", &root).unwrap_err();
        assert!(matches!(e, TetherError::Input(_)) && e.to_string().contains("file not found"), "{e}");
    }

    #[test]
    fn wrapper_location() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        let w = root.join("bin/tether-nvim");
        std::fs::write(&w, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&w, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exe = root.join("target/debug/tether");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(locate_wrapper_from(None, Some(exe.clone())), w.to_string_lossy());
        assert_eq!(locate_wrapper_from(Some("/custom/w".into()), Some(exe)), "/custom/w");
        assert_eq!(locate_wrapper_from(None, Some(PathBuf::from("/nonexistent/x/tether"))), "tether-nvim");
        assert_eq!(locate_wrapper_from(None, None), "tether-nvim");
    }
}
