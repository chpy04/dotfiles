//! `tether code open` (F1.3, key flow 2): jump a tether-registered Neovim in the target tab to
//! `file:line`, or split a pane and start one. Never opens a second Neovim in a tab that has one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::config::Placement;
use crate::error::{Result, TetherError};
use crate::herdr::{Client, Pane, PaneSplit, Tab};
use crate::nvim;
use crate::panes::{self, Scope};
use crate::send::{self, CodeRef};
use crate::tabs;
use crate::workspace::{self, StateStore};

/// What `code open` needs from Neovim, so the decision logic is testable without one.
pub trait Editor {
    /// If a tether-registered Neovim lives in `pane_id` and answers: its cursor (`path:line`).
    fn probe(&self, pane_id: &str) -> Option<String>;
    /// Jump that Neovim; returns `path:line` as Neovim reports it. `diff`: also show a diff
    /// against that revision (and close an earlier one; `None` leaves no tether diff open).
    fn jump(&self, pane_id: &str, file: &Path, line: u32, end_line: Option<u32>, diff: Option<&nvim::DiffBase>) -> Result<String>;
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

    fn jump(&self, pane_id: &str, file: &Path, line: u32, end_line: Option<u32>, diff: Option<&nvim::DiffBase>) -> Result<String> {
        let sock = nvim::socket_path_in(&self.runtime, pane_id);
        nvim::jump(&sock, file, line, end_line, diff)
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

/// Run git in `dir`; stdout on success, `None` on failure.
fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Resolve `rev` (a branch, tag, commit, `HEAD~3`, ...) in the repo of `file` and read the file as
/// it is in that commit. Fails when `file` is not in a git repo or `rev` is not a commit; a file
/// that does not exist in that commit gives `text: None` (the diff then shows it as all new).
pub fn diff_base(file: &Path, rev: &str) -> Result<nvim::DiffBase> {
    let rev = rev.trim();
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(TetherError::Input(format!(
            "--diff \"{rev}\" is not a git revision (give a branch, tag or commit, e.g. main or HEAD~1)"
        )));
    }
    let top = send::git_toplevel(file)
        .ok_or_else(|| TetherError::Input(format!("--diff needs a git repo, and {} is not in one", file.display())))?;
    let top = top.canonicalize().unwrap_or(top);
    let rel = file
        .strip_prefix(&top)
        .map_err(|_| TetherError::Input(format!("{} is outside its git worktree {}", file.display(), top.display())))?
        .to_string_lossy()
        .into_owned();
    let sha = git(&top, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{rev}^{{commit}}")])
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            TetherError::Input(format!("--diff {rev}: no such commit, branch or tag in {} (see `git log`, `git branch -a`)", top.display()))
        })?;
    let short = git(&top, &["rev-parse", "--short", &sha])
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .unwrap_or_else(|| sha.chars().take(7).collect());
    let text = git(&top, &["show", &format!("{sha}:{rel}")]).map(|o| String::from_utf8_lossy(&o).into_owned());
    Ok(nvim::DiffBase { rev: rev.to_string(), sha, short, rel, text })
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
    /// The pane that was split (only when not reused and no tab was created).
    pub split_from: Option<String>,
    /// A new tab was created for this Neovim (default placement away from the primary agent's tab).
    pub tab_created: bool,
    /// Label of that new tab.
    pub tab_label: Option<String>,
    /// `--diff`: the revision the file is shown against, as `<rev> (<short sha>)`.
    pub diff: Option<String>,
    /// What was done and why, in one line (printed by the CLI).
    pub summary: String,
    pub warnings: Vec<String>,
}

pub struct CodeOpen<'a> {
    pub reference: &'a CodeRef,
    pub file: PathBuf,
    /// `--in`: tab index, id or label. Default: the `--from` pane's tab, else the current tab; when
    /// that is the primary agent's tab (implicit only), a tether tab with a Neovim, else a new tab.
    pub in_tab: Option<&'a str>,
    pub from: Option<&'a str>,
    pub focus: bool,
    pub ready_timeout: Duration,
    pub wrapper: String,
    pub placement: &'a Placement,
    /// `--diff`: show the file side by side against this git revision, for this open only.
    pub diff: Option<&'a str>,
}

/// The tab holding the primary agent (read-only lookup; `None` when there is none or Herdr cannot say).
fn primary_agent_tab(client: &Client, store: &StateStore, workspace_id: &str) -> Option<String> {
    let rows = workspace::list_agents_in(client, workspace_id, store, false).ok()?;
    rows.into_iter().find(|r| r.is_primary).map(|r| r.tab_id)
}

/// Neovim panes in the most recently created tether tab (newest first) that has a live one.
fn recent_tether_nvim(
    owned: &[workspace::OwnedTab],
    exclude_tab: &str,
    live_tabs: &[crate::herdr::Tab],
    all_panes: &[Pane],
    alive: &dyn Fn(&str) -> bool,
) -> Option<(crate::herdr::Tab, String)> {
    let mut order: Vec<&workspace::OwnedTab> = owned.iter().collect();
    // newest first; equal timestamps: the one recorded later wins
    order.reverse();
    order.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    for t in order {
        if t.tab_id == exclude_tab {
            continue;
        }
        let Some(tab) = live_tabs.iter().find(|x| x.tab_id == t.tab_id) else { continue };
        if let Some(p) = all_panes.iter().find(|p| p.tab_id == tab.tab_id && alive(&p.pane_id)) {
            return Some((tab.clone(), p.pane_id.clone()));
        }
    }
    None
}

pub fn open(client: &Client, store: &StateStore, scope: &Scope, editor: &dyn Editor, o: &CodeOpen) -> Result<CodeOpened> {
    let implicit = o.in_tab.is_none() && o.from.is_none();
    // 0. a bad --diff fails before any pane or tab is touched
    let diff = o.diff.map(|rev| diff_base(&o.file, rev)).transpose()?;
    // 1. which tab
    let mut tab = match (o.in_tab, o.from) {
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
    let all_panes: Vec<Pane> = client.pane_list(Some(&scope.workspace_id))?;
    let panes_of = |tab_id: &str| -> Vec<Pane> { all_panes.iter().filter(|p| p.tab_id == tab_id).cloned().collect() };
    let mut tab_panes = panes_of(&tab.tab_id);
    let mut warnings = Vec::new();
    let mut summary = String::new();
    let mut new_tab: Option<(String, String)> = None; // (label, root pane)
    let mut reuse_pane: Option<String> = None;

    // 1b. default placement: never split the primary agent's tab on an implicit target. Reuse the
    // Neovim in the most recently tether-made tab, else open a tab named after the file.
    if implicit
        && !tab_panes.iter().any(|p| editor.probe(&p.pane_id).is_some())
        && primary_agent_tab(client, store, &scope.workspace_id).as_deref() == Some(tab.tab_id.as_str())
    {
        let live_tabs = tabs::scoped_tabs(client, scope, store)?;
        let owned = store.owned_tabs(&scope.workspace_id);
        if let Some((t, pane)) =
            recent_tether_nvim(&owned, &tab.tab_id, &live_tabs, &all_panes, &|id| editor.probe(id).is_some())
        {
            summary = format!(
                "tab {} \"{}\" is the primary agent's tab, so it was not split; reused the Neovim in tether tab {} \"{}\"",
                tab.number, tab.label, t.number, t.label
            );
            tab_panes = panes_of(&t.tab_id);
            tab = t;
            reuse_pane = Some(pane);
        } else {
            let name = o.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "code".into());
            let cwd = send::git_toplevel(&o.file)
                .or_else(|| o.file.parent().map(Path::to_path_buf))
                .map(|p| p.to_string_lossy().into_owned());
            let args = vec![o.wrapper.clone(), format!("+{}", o.reference.line), o.file.to_string_lossy().into_owned()];
            let opened = tabs::open_tab(client, store, scope, &name, cwd.as_deref(), false, &args)?;
            summary = format!(
                "tab {} \"{}\" is the primary agent's tab, so it was not split; opened new tab {} \"{}\" for Neovim",
                tab.number, tab.label, opened.index, opened.label
            );
            new_tab = Some((opened.label.clone(), opened.pane_id.clone()));
            tab = Tab { tab_id: opened.tab_id, number: opened.index, label: opened.label, ..tab };
            tab_panes = vec![Pane { pane_id: opened.pane_id.clone(), tab_id: tab.tab_id.clone(), ..Default::default() }];
        }
    }

    // 2. reuse or split (or: Neovim was just started in the new tab's root pane)
    let r = o.reference;
    let (pane_id, reused, split_from) = if let Some((_, root)) = new_tab.clone() {
        if !editor.wait_ready(&root, o.ready_timeout) {
            return Err(TetherError::Other(format!(
                "Neovim in new tab pane {root} did not answer within {:?}; the tab was left open (is `{}` runnable there?)",
                o.ready_timeout, o.wrapper
            )));
        }
        (root, false, None)
    } else if let Some(p) = reuse_pane {
        (p, true, None)
    } else {
        match decide(&tab_panes, &|id| editor.probe(id).is_some(), o.from, scope.pane_id.as_deref())? {
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
        }
    };

    // 3. jump (also after a fresh start: confirms the position and applies the range highlight)
    let at = editor.jump(&pane_id, &o.file, r.line, r.end_line, diff.as_ref())?;
    if o.focus {
        // the Neovim pane itself, reused or newly split; a pane in another tab needs that tab shown first
        if scope.tab_id.as_deref() != Some(tab.tab_id.as_str()) {
            client.tab_focus(&tab.tab_id)?;
        }
        client.pane_focus(&pane_id)?;
    }
    if summary.is_empty() {
        summary = match (&split_from, reused) {
            (_, true) => format!("reused the Neovim in pane {pane_id} (tab {} \"{}\")", tab.number, tab.label),
            (Some(f), false) => format!("no Neovim in tab {} \"{}\"; split pane {f} and started one", tab.number, tab.label),
            (None, false) => format!("started Neovim in pane {pane_id}"),
        };
    }
    let diff_label = diff.as_ref().map(|d| format!("{} ({})", d.rev, d.short));
    if let Some(d) = &diff {
        summary.push_str(&format!("; diffing against {} ({})", d.rev, d.short));
        if d.text.is_none() {
            warnings.push(format!("{} does not exist in {}; the diff shows the whole file as new", d.rel, d.rev));
        }
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
        tab_created: new_tab.is_some(),
        tab_label: new_tab.map(|(l, _)| l),
        diff: diff_label,
        summary,
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

    /// A repo with `a.txt` committed twice (tag `v1` on the first) and `new.txt` only in the worktree.
    fn diff_repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let g = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
        };
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        g(&["add", "a.txt"]);
        g(&["commit", "-qm", "1"]);
        g(&["tag", "v1"]);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        g(&["commit", "-qam", "2"]);
        std::fs::write(root.join("a.txt"), "three\n").unwrap();
        std::fs::write(root.join("new.txt"), "fresh\n").unwrap();
        (d, root)
    }

    #[test]
    fn diff_base_reads_the_file_at_a_revision() {
        let (_d, root) = diff_repo();
        let b = diff_base(&root.join("a.txt"), "v1").unwrap();
        assert_eq!((b.rev.as_str(), b.rel.as_str(), b.text.as_deref()), ("v1", "a.txt", Some("one\n")));
        assert_eq!(b.sha.len(), 40);
        assert!(b.sha.starts_with(&b.short));
        assert_eq!(diff_base(&root.join("a.txt"), "HEAD").unwrap().text.as_deref(), Some("two\n"));
        assert_eq!(diff_base(&root.join("a.txt"), "main~1").unwrap().sha, b.sha);
        // a file the revision does not have: diffed against nothing
        assert_eq!(diff_base(&root.join("new.txt"), "main").unwrap().text, None);
    }

    #[test]
    fn diff_base_refuses_bad_revisions_and_non_repos() {
        let (_d, root) = diff_repo();
        let f = root.join("a.txt");
        for bad in ["", "  ", "--output=/tmp/x", "-p", "a b", "nope-branch"] {
            assert!(matches!(diff_base(&f, bad).unwrap_err(), TetherError::Input(_)), "{bad:?}");
        }
        let plain = tempfile::tempdir().unwrap();
        let pf = plain.path().canonicalize().unwrap().join("x.txt");
        std::fs::write(&pf, "x").unwrap();
        let e = diff_base(&pf, "main").unwrap_err();
        assert!(e.to_string().contains("git repo"), "{e}");
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
        fn jump(&self, pane_id: &str, file: &Path, line: u32, end: Option<u32>, _diff: Option<&nvim::DiffBase>) -> Result<String> {
            self.jumps.borrow_mut().push((pane_id.to_string(), line, end));
            Ok(format!("{}:{line}", file.display()))
        }
        fn wait_ready(&self, _: &str, _: Duration) -> bool {
            self.ready_after_split
        }
    }

    fn fake() -> FakeHerdr {
        fake_ex(false)
    }

    /// `primary`: the agent list shows a Claude in tab w1:t1 (the caller's own tab).
    fn fake_ex(primary: bool) -> FakeHerdr {
        FakeHerdr::start(move |method, params| match method {
            "agent.list" if primary => Ok(json!({"type":"agent_list","agents":[
                {"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle"}]})),
            "tab.create" => Ok(json!({"type":"tab_created",
                "tab":{"tab_id":"w1:t9","workspace_id":"w1","number":3,"label":params["label"]},
                "root_pane":{"pane_id":"w1:p20","workspace_id":"w1","tab_id":"w1:t9"}})),
            "tab.list" => Ok(json!({"type":"tab_list","tabs":[
                {"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"},
                {"tab_id":"w1:t4","workspace_id":"w1","number":2,"label":"diagram"}]})),
            "pane.list" => Ok(json!({"type":"pane_list","panes":[
                {"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","focused":true},
                {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t4","focused":false},
                {"pane_id":"w1:p6","workspace_id":"w1","tab_id":"w1:t4","focused":false}]})),
            "pane.get" => {
                let tab = if params["pane_id"] == "w1:p1" { "w1:t1" } else { "w1:t4" };
                Ok(json!({"type":"pane_info","pane":{"pane_id":params["pane_id"],"workspace_id":"w1","tab_id":tab}}))
            }
            "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p9","workspace_id":"w1","tab_id":"w1:t4"}})),
            "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{"pane_id":params["pane_id"],"foreground_processes":[{"pid":1,"name":"zsh","argv":["zsh"]}]}})),
            "pane.send_input" | "pane.focus" | "tab.focus" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), method.to_string())),
        })
    }

    fn scope() -> Scope {
        Scope { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()), explicit_other: false }
    }

    fn run(f: &FakeHerdr, ed: &MockEditor, in_tab: Option<&str>, from: Option<&str>, r: &str) -> Result<CodeOpened> {
        run_seeded(f, ed, in_tab, from, r, &[]).0
    }

    /// `seed`: (tab id, label, created_at) tether-made tabs recorded before the call. Also returns
    /// the tether-made tab ids recorded afterwards.
    fn run_seeded(
        f: &FakeHerdr,
        ed: &MockEditor,
        in_tab: Option<&str>,
        from: Option<&str>,
        r: &str,
        seed: &[(&str, &str, u64)],
    ) -> (Result<CodeOpened>, Vec<String>) {
        run_seeded_focus(f, ed, in_tab, from, r, seed, false)
    }

    fn run_seeded_focus(
        f: &FakeHerdr,
        ed: &MockEditor,
        in_tab: Option<&str>,
        from: Option<&str>,
        r: &str,
        seed: &[(&str, &str, u64)],
        focus: bool,
    ) -> (Result<CodeOpened>, Vec<String>) {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("a.ts");
        std::fs::write(&file, "x\n").unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        for (id, label, at) in seed {
            store.add_owned_tab_at("w1", id, label, *at).unwrap();
        }
        let reference: CodeRef = r.parse().unwrap();
        let placement = Placement { diagram: "tab".into(), code_direction: "right".into(), code_ratio: 0.4 };
        let res = open(
            &Client::new(f.path()),
            &store,
            &scope(),
            ed,
            &CodeOpen { reference: &reference, file, in_tab, from, focus, ready_timeout: Duration::from_millis(50), wrapper: "/repo/bin/tether-nvim".into(), placement: &placement, diff: None },
        );
        let owned = store.owned_tabs("w1").into_iter().map(|t| t.tab_id).collect();
        (res, owned)
    }

    // ---- finding 7: the primary agent's tab is not split by default ----

    #[test]
    fn primary_tab_default_opens_a_new_tab_named_after_the_file() {
        let f = fake_ex(true);
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        let (r, owned) = run_seeded(&f, &ed, None, None, "a.ts:7", &[]);
        let r = r.unwrap();
        assert!(f.requests_for("pane.split").is_empty(), "the chat tab is not split");
        let create = &f.requests_for("tab.create")[0].params;
        assert_eq!((create["label"].as_str(), create["focus"].as_bool()), (Some("a.ts"), Some(false)));
        assert!(r.tab_created && !r.reused);
        assert_eq!((r.tab_id.as_str(), r.pane_id.as_str(), r.tab_label.as_deref()), ("w1:t9", "w1:p20", Some("a.ts")));
        assert_eq!(owned, ["w1:t9"], "the new tab is recorded as tether-made");
        let text = f.requests_for("pane.send_input")[0].params["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("/repo/bin/tether-nvim +7 ") && text.ends_with("a.ts"), "{text}");
        assert_eq!(ed.jumps.borrow()[0].0, "w1:p20");
        assert!(r.summary.contains("primary agent's tab") && r.summary.contains("new tab 3 \"a.ts\""), "{}", r.summary);
    }

    #[test]
    fn primary_tab_default_reuses_neovim_in_the_most_recent_tether_tab() {
        let f = fake_ex(true);
        let ed = MockEditor { alive: ["w1:p5".to_string()].into(), ready_after_split: true, ..Default::default() };
        let (r, _) = run_seeded(&f, &ed, None, None, "a.ts:7", &[("w1:t4", "diagram", 100)]);
        let r = r.unwrap();
        assert!(r.reused && !r.tab_created);
        assert_eq!((r.pane_id.as_str(), r.tab_id.as_str()), ("w1:p5", "w1:t4"));
        assert!(f.requests_for("tab.create").is_empty() && f.requests_for("pane.split").is_empty());
        assert!(r.summary.contains("reused the Neovim in tether tab 2"), "{}", r.summary);
        // a tether tab without a Neovim does not count: new tab
        let f = fake_ex(true);
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        let (r, _) = run_seeded(&f, &ed, None, None, "a.ts:7", &[("w1:t4", "diagram", 100)]);
        assert!(r.unwrap().tab_created);
    }

    // ---- --focus puts focus on the Neovim pane itself, in every case ----

    fn focused(f: &FakeHerdr) -> Vec<String> {
        f.requests_for("pane.focus").iter().map(|r| r.params["pane_id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn focus_goes_to_the_neovim_pane_when_it_is_reused_or_newly_split_or_in_a_new_tab() {
        // reused (same tab as the caller's pane is not needed: target tab "diagram")
        let f = fake();
        let ed = MockEditor { alive: ["w1:p5".to_string()].into(), ..Default::default() };
        run_seeded_focus(&f, &ed, Some("diagram"), None, "a.ts:7", &[], true).0.unwrap();
        assert_eq!(focused(&f), ["w1:p5"], "reuse: the Neovim pane gets focus");
        assert_eq!(f.requests_for("tab.focus").len(), 1, "it lives in another tab than the caller: that tab is shown");
        // newly split
        let f = fake();
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        let r = run_seeded_focus(&f, &ed, None, Some("w1:p1"), "a.ts:7", &[], true).0.unwrap();
        assert!(!r.reused);
        assert_eq!(focused(&f), ["w1:p9"], "split: the new Neovim pane gets focus, not the pane it was split from");
        assert!(f.requests_for("tab.focus").is_empty(), "same tab as the caller");
        // new tab (primary agent's tab is never split)
        let f = fake_ex(true);
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        let r = run_seeded_focus(&f, &ed, None, None, "a.ts:7", &[], true).0.unwrap();
        assert!(r.tab_created);
        assert_eq!(focused(&f), ["w1:p20"]);
        // without --focus nothing is focused
        let f = fake();
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        run_seeded_focus(&f, &ed, None, Some("w1:p1"), "a.ts:7", &[], false).0.unwrap();
        assert!(focused(&f).is_empty() && f.requests_for("tab.focus").is_empty());
    }

    #[test]
    fn explicit_in_or_from_still_splits_next_to_the_chat() {
        for (in_tab, from) in [(Some("1"), None), (None, Some("w1:p1"))] {
            let f = fake_ex(true);
            let ed = MockEditor { ready_after_split: true, ..Default::default() };
            let r = run(&f, &ed, in_tab, from, "a.ts:7").unwrap();
            assert!(!r.tab_created && !r.reused, "{in_tab:?} {from:?}");
            assert_eq!(f.requests_for("pane.split")[0].params["target_pane_id"], "w1:p1");
            assert!(f.requests_for("tab.create").is_empty());
        }
    }

    #[test]
    fn a_live_neovim_in_the_primary_tab_is_reused_in_place() {
        let f = fake_ex(true);
        let ed = MockEditor { alive: ["w1:p1".to_string()].into(), ..Default::default() };
        let r = run(&f, &ed, None, None, "a.ts:7").unwrap();
        assert!(r.reused && r.pane_id == "w1:p1" && !r.tab_created);
        assert!(f.requests_for("tab.create").is_empty() && f.requests_for("pane.split").is_empty());
    }

    #[test]
    fn non_primary_current_tab_still_splits_by_default() {
        // the primary lives in w1:t4; the caller is in w1:t1
        let f = FakeHerdr::start(|method, params| match method {
            "agent.list" => Ok(json!({"type":"agent_list","agents":[
                {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t4","agent":"claude","agent_status":"idle"}]})),
            "tab.list" => Ok(json!({"type":"tab_list","tabs":[
                {"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"},
                {"tab_id":"w1:t4","workspace_id":"w1","number":2,"label":"chat"}]})),
            "pane.list" => Ok(json!({"type":"pane_list","panes":[
                {"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","focused":true},
                {"pane_id":"w1:p5","workspace_id":"w1","tab_id":"w1:t4"}]})),
            "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p9","workspace_id":"w1","tab_id":"w1:t1"}})),
            "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{"pane_id":params["pane_id"],"foreground_processes":[]}})),
            "pane.send_input" | "pane.focus" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), method.to_string())),
        });
        let ed = MockEditor { ready_after_split: true, ..Default::default() };
        let r = run(&f, &ed, None, None, "a.ts:7").unwrap();
        assert!(!r.tab_created);
        assert_eq!(f.requests_for("pane.split")[0].params["target_pane_id"], "w1:p1");
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
