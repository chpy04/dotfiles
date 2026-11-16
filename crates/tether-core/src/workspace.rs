//! Current-workspace context, the agent listing, and primary-agent tagging.
//!
//! Primary agent (PRD "Implementation defaults"): the first agent tether sees in a workspace is
//! tagged and persisted in a state file (Herdr has no storage API). The tag moves only when the
//! tagged pane is gone from the workspace (or nothing is tagged): then the first `claude` agent in
//! tab order (any agent if there is no claude) is chosen. A pane that merely has no agent entry for
//! a moment keeps its tag, and the fallback used meanwhile is not persisted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, TetherError};
use crate::herdr::{Agent, Client, HerdrError};

// ---------------------------------------------------------------------------------------------
// Context from env (F2.1)
// ---------------------------------------------------------------------------------------------

/// Where tether was invoked: the Herdr workspace/tab/pane in `HERDR_*`.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub workspace_id: String,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
}

impl Context {
    /// Resolve from the process environment. Errors clearly when not inside Herdr.
    pub fn from_env() -> Result<Context> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Context> {
        let nonempty = |k: &str| get(k).filter(|v| !v.is_empty());
        match nonempty("HERDR_WORKSPACE_ID") {
            Some(workspace_id) => Ok(Context {
                workspace_id,
                tab_id: nonempty("HERDR_TAB_ID"),
                pane_id: nonempty("HERDR_PANE_ID"),
            }),
            None => Err(TetherError::NotInHerdr(
                "not inside a Herdr workspace: HERDR_WORKSPACE_ID is not set (run tether from a pane in Herdr)".into(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Persisted state
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    workspaces: BTreeMap<String, WorkspaceState>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct WorkspaceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primary_pane: Option<String>,
    /// Tabs created by `tether tab open` (so `tab close` can refuse hand-made ones).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tabs: Vec<OwnedTab>,
}

/// A tab tether created, recorded in the state file under its workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedTab {
    pub tab_id: String,
    /// Label at creation time (informational; renames by tether keep it in sync).
    pub label: String,
    /// Unix seconds.
    #[serde(default)]
    pub created_at: u64,
}

/// `$TETHER_STATE`, else `$XDG_STATE_HOME/tether/state.json`, else `~/.local/state/tether/state.json`.
/// (Sockets stay in the runtime dir; state must survive `$TMPDIR` purges.)
pub fn state_path() -> PathBuf {
    state_path_from(|k| std::env::var_os(k))
}

pub fn state_path_from(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let nonempty = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(p) = nonempty("TETHER_STATE") {
        return PathBuf::from(p);
    }
    let base = match nonempty("XDG_STATE_HOME") {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(nonempty("HOME").unwrap_or_else(|| "/tmp".into())).join(".local/state"),
    };
    base.join("tether").join("state.json")
}

/// Records younger than this are never pruned: the tab list they are compared with may predate them.
const PRUNE_GRACE_SECS: u64 = 10;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        StateStore { path: path.into() }
    }

    pub fn from_env() -> Self {
        Self::new(state_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the state. A missing file is "nothing recorded"; a corrupt one is backed up to
    /// `<name>.corrupt-<ts>` and reported on stderr (never silently dropped).
    fn load(&self) -> StateFile {
        let Ok(body) = std::fs::read_to_string(&self.path) else { return StateFile::default() };
        match serde_json::from_str(&body) {
            Ok(st) => st,
            Err(e) => {
                let mut name = self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                name.push(format!(".corrupt-{}", now_secs()));
                let backup = self.path.with_file_name(name);
                let saved = std::fs::copy(&self.path, &backup).is_ok();
                eprintln!(
                    "tether: warning: state file {} is corrupt ({e}); {} and starting empty",
                    self.path.display(),
                    if saved { format!("backed it up to {}", backup.display()) } else { "could not back it up".to_string() }
                );
                StateFile::default()
            }
        }
    }

    /// Exclusive advisory lock on the sidecar `<state>.lock`, held until the guard drops.
    fn lock(&self) -> Result<std::fs::File> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| TetherError::Other(format!("cannot create {}: {e}", dir.display())))?;
        }
        let mut name = self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".lock");
        let lock_path = self.path.with_file_name(name);
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| TetherError::Other(format!("cannot open {}: {e}", lock_path.display())))?;
        f.lock().map_err(|e| TetherError::Other(format!("cannot lock {}: {e}", lock_path.display())))?;
        Ok(f)
    }

    pub fn primary_pane(&self, workspace_id: &str) -> Option<String> {
        self.load().workspaces.get(workspace_id).and_then(|w| w.primary_pane.clone())
    }

    pub fn set_primary_pane(&self, workspace_id: &str, pane_id: &str) -> Result<()> {
        self.update(|st| st.workspaces.entry(workspace_id.to_string()).or_default().primary_pane = Some(pane_id.to_string()))
    }

    /// Tabs tether created in this workspace.
    pub fn owned_tabs(&self, workspace_id: &str) -> Vec<OwnedTab> {
        self.load().workspaces.get(workspace_id).map(|w| w.tabs.clone()).unwrap_or_default()
    }

    pub fn is_owned_tab(&self, workspace_id: &str, tab_id: &str) -> bool {
        self.owned_tabs(workspace_id).iter().any(|t| t.tab_id == tab_id)
    }

    /// Record a tab tether just created (replaces a stale record with the same id).
    pub fn add_owned_tab(&self, workspace_id: &str, tab_id: &str, label: &str) -> Result<()> {
        self.add_owned_tab_at(workspace_id, tab_id, label, now_secs())
    }

    pub(crate) fn add_owned_tab_at(&self, workspace_id: &str, tab_id: &str, label: &str, created_at: u64) -> Result<()> {
        self.update(|st| {
            let w = st.workspaces.entry(workspace_id.to_string()).or_default();
            w.tabs.retain(|t| t.tab_id != tab_id);
            w.tabs.push(OwnedTab { tab_id: tab_id.to_string(), label: label.to_string(), created_at });
        })
    }

    pub fn rename_owned_tab(&self, workspace_id: &str, tab_id: &str, label: &str) -> Result<()> {
        self.update(|st| {
            if let Some(t) = st.workspaces.entry(workspace_id.to_string()).or_default().tabs.iter_mut().find(|t| t.tab_id == tab_id) {
                t.label = label.to_string();
            }
        })
    }

    pub fn remove_owned_tab(&self, workspace_id: &str, tab_id: &str) -> Result<()> {
        self.update(|st| {
            if let Some(w) = st.workspaces.get_mut(workspace_id) {
                w.tabs.retain(|t| t.tab_id != tab_id);
            }
        })
    }

    /// Drop records of tabs that no longer exist (`live` = current tab ids of the workspace), so a
    /// reused tab id can never inherit "tether made this". A record is dropped only when its tab id
    /// is absent from `live` **and** it is older than ~10 s (a tab created after `live` was fetched
    /// must survive); the check is repeated inside the state lock. Writes only when something changed.
    pub fn prune_owned_tabs(&self, workspace_id: &str, live: &[String]) -> Result<()> {
        self.prune_owned_tabs_at(workspace_id, live, now_secs())
    }

    fn prune_owned_tabs_at(&self, workspace_id: &str, live: &[String], now: u64) -> Result<()> {
        let prunable = |t: &OwnedTab| !live.contains(&t.tab_id) && now.saturating_sub(t.created_at) > PRUNE_GRACE_SECS;
        if !self.owned_tabs(workspace_id).iter().any(prunable) {
            return Ok(());
        }
        self.update(|st| {
            if let Some(w) = st.workspaces.get_mut(workspace_id) {
                w.tabs.retain(|t| !prunable(t));
            }
        })
    }

    /// Locked read-modify-write: concurrent tether processes cannot lose each other's records.
    fn update(&self, f: impl FnOnce(&mut StateFile)) -> Result<()> {
        let _guard = self.lock()?;
        let mut st = self.load();
        f(&mut st);
        let tmp = self.path.with_extension(format!("tmp{}", std::process::id()));
        let body = serde_json::to_string_pretty(&st).map_err(|e| TetherError::Other(e.to_string()))?;
        std::fs::write(&tmp, body).map_err(|e| TetherError::Other(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| TetherError::Other(format!("cannot write {}: {e}", self.path.display())))
    }
}

// ---------------------------------------------------------------------------------------------
// Agent rows
// ---------------------------------------------------------------------------------------------

/// One agent in a workspace, annotated with its tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRow {
    /// 1-based tab index.
    pub tab_index: u32,
    pub tab_label: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub pane_id: String,
    /// `claude`, `codex`, ...
    pub kind: String,
    /// `idle | working | blocked | done | unknown`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_primary: bool,
}

impl AgentRow {
    /// `tab 1 · claude — idle` (the picker label).
    pub fn display(&self) -> String {
        format!("tab {} · {} — {}", self.tab_index, self.kind, self.state)
    }
}

/// Outcome of primary selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    /// Index into `rows` of the primary.
    pub index: usize,
    /// Whether the choice should be written as the new tag.
    pub persist: bool,
}

/// Pure primary selection. `rows` must be in tab order. `tagged_pane_exists` says whether the
/// tagged pane is still a pane of the workspace (agent or not).
/// * the tagged pane has an agent row: it is the primary, nothing to write;
/// * the tagged pane exists but has no agent right now (Herdr hiccup): keep the tag; use the
///   fallback for this call only (`persist = false`);
/// * the tagged pane is gone, or nothing is tagged: fallback, persisted.
///
/// Fallback = first `claude` agent in tab order, else the first agent.
pub fn pick_primary(rows: &[AgentRow], tagged: Option<&str>, tagged_pane_exists: bool) -> Option<Pick> {
    if let Some(t) = tagged {
        if let Some(i) = rows.iter().position(|r| r.pane_id == t) {
            return Some(Pick { index: i, persist: false });
        }
    }
    let index = rows.iter().position(|r| r.kind == "claude").or(if rows.is_empty() { None } else { Some(0) })?;
    Some(Pick { index, persist: !(tagged.is_some() && tagged_pane_exists) })
}

/// Build rows from raw Herdr data (no primary flags yet), ordered by tab index.
pub fn build_rows(agents: &[Agent], tabs: &[crate::herdr::Tab], workspace_id: &str) -> Vec<AgentRow> {
    let mut rows: Vec<(usize, AgentRow)> = agents
        .iter()
        .filter(|a| a.workspace_id == workspace_id)
        .enumerate()
        .map(|(i, a)| {
            let tab = tabs.iter().find(|t| t.tab_id == a.tab_id);
            (
                i,
                AgentRow {
                    tab_index: tab.map(|t| t.number).unwrap_or(0),
                    tab_label: tab.map(|t| t.label.clone()).unwrap_or_default(),
                    tab_id: a.tab_id.clone(),
                    workspace_id: a.workspace_id.clone(),
                    pane_id: a.pane_id.clone(),
                    kind: a.agent.clone(),
                    state: a.agent_status.clone(),
                    name: a.name.clone(),
                    is_primary: false,
                },
            )
        })
        .collect();
    // tab numbers are 1-based; 0 means "tab unknown" and sorts last
    rows.sort_by_key(|(i, r)| (if r.tab_index == 0 { u32::MAX } else { r.tab_index }, *i));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// Apply primary tagging: reads the tag, picks, flags the row. With `persist_tag` a changed tag is
/// written; read-only callers (`agents`, `layout`) pass `false` and never write.
/// `pane_ids` are all panes of the workspace (agent or not); `None` = unknown, which counts as
/// "the tagged pane still exists" so nothing is retagged on a guess.
pub fn apply_primary(
    rows: &mut [AgentRow],
    workspace_id: &str,
    store: &StateStore,
    pane_ids: Option<&[String]>,
    persist_tag: bool,
) -> Result<()> {
    let tagged = store.primary_pane(workspace_id);
    let exists = tagged.as_deref().is_some_and(|t| pane_ids.is_none_or(|ids| ids.iter().any(|p| p == t)));
    if let Some(pick) = pick_primary(rows, tagged.as_deref(), exists) {
        rows[pick.index].is_primary = true;
        if persist_tag && pick.persist && tagged.as_deref() != Some(rows[pick.index].pane_id.as_str()) {
            store.set_primary_pane(workspace_id, &rows[pick.index].pane_id)?;
        }
    }
    Ok(())
}

fn workspace_pane_ids(client: &Client, workspace_id: &str) -> Option<Vec<String>> {
    client.pane_list(Some(workspace_id)).ok().map(|p| p.into_iter().map(|p| p.pane_id).collect())
}

/// Every agent in the context's workspace (tab order), with the primary flagged. Read-only: never
/// writes the tag (sending does, see `resolve_target`).
pub fn list_agents(client: &Client, ctx: &Context, store: &StateStore) -> Result<Vec<AgentRow>> {
    list_agents_in(client, &ctx.workspace_id, store, false)
}

/// `list_agents` for a workspace id; `persist_tag` allows writing a changed primary tag.
pub fn list_agents_in(client: &Client, workspace_id: &str, store: &StateStore, persist_tag: bool) -> Result<Vec<AgentRow>> {
    let agents = client.agent_list()?;
    let tabs = client.tab_list(Some(workspace_id))?;
    let mut rows = build_rows(&agents, &tabs, workspace_id);
    let panes = workspace_pane_ids(client, workspace_id);
    apply_primary(&mut rows, workspace_id, store, panes.as_deref(), persist_tag)?;
    Ok(rows)
}

/// Resolve `--to`: `primary`, a pane id (`w8:p2`), or an agent name (also: a unique agent kind).
/// A pane id outside the current workspace is refused (see `resolve_target_opts`).
pub fn resolve_target(client: &Client, ctx: &Context, store: &StateStore, to: &str) -> Result<AgentRow> {
    resolve_target_opts(client, ctx, store, to, None, false)
}

/// `resolve_target` with the foreign-pane policy: an explicit pane id that belongs to another
/// workspace is refused (`TetherError::Refused`, exit 6) unless `workspace` (`--workspace <id>`)
/// names that workspace or `allow_foreign` (`--allow-foreign`) is set.
pub fn resolve_target_opts(
    client: &Client,
    ctx: &Context,
    store: &StateStore,
    to: &str,
    workspace: Option<&str>,
    allow_foreign: bool,
) -> Result<AgentRow> {
    let rows = list_agents_in(client, &ctx.workspace_id, store, true)?;
    let row = resolve_in_rows(client, ctx, &rows, to)?;
    if row.workspace_id != ctx.workspace_id && !allow_foreign && workspace != Some(row.workspace_id.as_str()) {
        return Err(TetherError::Refused(format!(
            "pane {} belongs to workspace {}, but tether is scoped to workspace {}; pass --workspace {} \
             (or --allow-foreign) to send there deliberately",
            row.pane_id, row.workspace_id, ctx.workspace_id, row.workspace_id
        )));
    }
    Ok(row)
}

fn resolve_in_rows(client: &Client, ctx: &Context, rows: &[AgentRow], to: &str) -> Result<AgentRow> {
    if to == "primary" {
        return rows.iter().find(|r| r.is_primary).cloned().ok_or_else(|| {
            TetherError::Target(format!(
                "no agent found in workspace {} (start one, e.g. `claude`, in a pane of this workspace)",
                ctx.workspace_id
            ))
        });
    }
    if to.contains(':') {
        if let Some(r) = rows.iter().find(|r| r.pane_id == to) {
            return Ok(r.clone());
        }
        // Explicit pane id outside this workspace (or not yet in the list): ask Herdr.
        return match client.agent_get(to) {
            Ok(a) => {
                let tabs = client.tab_list(Some(&a.workspace_id)).unwrap_or_default();
                let mut r = build_rows(std::slice::from_ref(&a), &tabs, &a.workspace_id);
                Ok(r.remove(0))
            }
            Err(HerdrError::Api { code, .. }) if code == "agent_not_found" || code == "pane_not_found" => {
                Err(TetherError::Target(format!("pane {to} is not an agent pane (or does not exist)")))
            }
            Err(e) => Err(e.into()),
        };
    }
    let by_name: Vec<&AgentRow> = rows.iter().filter(|r| r.name.as_deref() == Some(to)).collect();
    let matches = if by_name.is_empty() { rows.iter().filter(|r| r.kind == to).collect() } else { by_name };
    match matches.len() {
        0 => Err(TetherError::Target(format!(
            "no agent named `{to}` in workspace {} (use `tether agents` to list them)",
            ctx.workspace_id
        ))),
        1 => Ok(matches[0].clone()),
        _ => Err(TetherError::Target(format!(
            "`{to}` matches {} agents ({}); use a pane id",
            matches.len(),
            matches.iter().map(|r| r.pane_id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHerdr;
    use serde_json::json;

    fn row(tab: u32, pane: &str, kind: &str) -> AgentRow {
        AgentRow {
            tab_index: tab,
            tab_label: tab.to_string(),
            tab_id: format!("w1:t{tab}"),
            workspace_id: "w1".into(),
            pane_id: pane.into(),
            kind: kind.into(),
            state: "idle".into(),
            name: None,
            is_primary: false,
        }
    }

    #[test]
    fn context_requires_workspace() {
        let none = Context::from_lookup(|_| None).unwrap_err();
        assert!(matches!(none, TetherError::NotInHerdr(_)));
        assert!(none.to_string().contains("not inside a Herdr"));
        assert_eq!(none.exit_code(), crate::error::exit::NOT_IN_HERDR);
        let empty = Context::from_lookup(|k| (k == "HERDR_WORKSPACE_ID").then(String::new));
        assert!(empty.is_err());
        let ok = Context::from_lookup(|k| match k {
            "HERDR_WORKSPACE_ID" => Some("w3".into()),
            "HERDR_PANE_ID" => Some("w3:p1".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(ok, Context { workspace_id: "w3".into(), tab_id: None, pane_id: Some("w3:p1".into()) });
    }

    fn panes(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pick_primary_rules() {
        let rows = vec![row(1, "w1:p1", "claude"), row(3, "w1:p5", "codex")];
        let pick = |t: Option<&str>, exists: bool| pick_primary(&rows, t, exists);
        assert_eq!(pick(None, false), Some(Pick { index: 0, persist: true })); // first seen = tab order
        assert_eq!(pick(Some("w1:p5"), true), Some(Pick { index: 1, persist: false })); // tag wins, nothing to write
        assert_eq!(pick(Some("w1:gone"), false), Some(Pick { index: 0, persist: true })); // pane vanished -> retag
        assert_eq!(pick_primary(&[], Some("w1:p1"), true), None);
    }

    #[test]
    fn pane_without_agent_entry_keeps_its_tag_and_fallback_is_not_persisted() {
        // tagged pane exists in the workspace but is momentarily missing from agent.list
        let rows = vec![row(1, "w1:p2", "claude"), row(2, "w1:p3", "claude")];
        assert_eq!(pick_primary(&rows, Some("w1:p1"), true), Some(Pick { index: 0, persist: false }));
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        store.set_primary_pane("w1", "w1:p1").unwrap();
        let mut r = rows.clone();
        apply_primary(&mut r, "w1", &store, Some(&panes(&["w1:p1", "w1:p2", "w1:p3"])), true).unwrap();
        assert!(r[0].is_primary, "fallback used for this call");
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"), "tag kept");
        // pane list unknown: also never retag on a guess
        let mut r = rows.clone();
        apply_primary(&mut r, "w1", &store, None, true).unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        // once the pane is really gone, the tag moves
        let mut r = rows.clone();
        apply_primary(&mut r, "w1", &store, Some(&panes(&["w1:p2", "w1:p3"])), true).unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
    }

    #[test]
    fn read_only_callers_never_write_a_tag() {
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let mut r = vec![row(1, "w1:p2", "claude")];
        apply_primary(&mut r, "w1", &store, Some(&panes(&["w1:p2"])), false).unwrap();
        assert!(r[0].is_primary);
        assert_eq!(store.primary_pane("w1"), None);
        assert!(!store.path().exists(), "no state file written");
        // ...but a tagged pane that is gone is not rewritten either
        store.set_primary_pane("w1", "w1:old").unwrap();
        apply_primary(&mut r, "w1", &store, Some(&panes(&["w1:p2"])), false).unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:old"));
    }

    #[test]
    fn fallback_prefers_claude_and_unknown_tabs_sort_last() {
        let rows = vec![row(1, "w1:p1", "codex"), row(2, "w1:p2", "claude"), row(3, "w1:p3", "claude")];
        assert_eq!(pick_primary(&rows, None, false).unwrap().index, 1);
        let no_claude = vec![row(1, "w1:p1", "codex"), row(2, "w1:p2", "gemini")];
        assert_eq!(pick_primary(&no_claude, None, false).unwrap().index, 0);
        // build_rows: an agent whose tab is not in the tab list (tab_index 0) goes last
        let ag = |pane: &str, tab: &str| Agent {
            pane_id: pane.into(),
            workspace_id: "w1".into(),
            tab_id: tab.into(),
            agent: "claude".into(),
            agent_status: "idle".into(),
            ..Default::default()
        };
        let tabs = vec![
            crate::herdr::Tab { tab_id: "w1:t1".into(), workspace_id: "w1".into(), number: 1, label: "1".into(), ..Default::default() },
            crate::herdr::Tab { tab_id: "w1:t2".into(), workspace_id: "w1".into(), number: 2, label: "2".into(), ..Default::default() },
        ];
        let rows = build_rows(&[ag("w1:pX", "w1:tunknown"), ag("w1:p2", "w1:t2"), ag("w1:p1", "w1:t1")], &tabs, "w1");
        assert_eq!(rows.iter().map(|r| r.pane_id.as_str()).collect::<Vec<_>>(), ["w1:p1", "w1:p2", "w1:pX"]);
        assert_eq!(pick_primary(&rows, None, false).unwrap().index, 0);
    }

    #[test]
    fn tagging_persists_and_retags_when_pane_disappears() {
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("t/state.json"));
        // first sight: tab 1 agent is tagged
        let mut rows = vec![row(1, "w1:p1", "claude"), row(2, "w1:p2", "claude")];
        apply_primary(&mut rows, "w1", &store, Some(&panes(&["w1:p1", "w1:p2"])), true).unwrap();
        assert!(rows[0].is_primary && !rows[1].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        // tabs reordered: tag sticks to pane p1 although it is now tab 2
        let mut rows = vec![row(1, "w1:p2", "claude"), row(2, "w1:p1", "claude")];
        apply_primary(&mut rows, "w1", &store, Some(&panes(&["w1:p1", "w1:p2"])), true).unwrap();
        assert!(rows[1].is_primary && !rows[0].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        // p1 closes: fall back to the first claude in tab order and retag
        let mut rows = vec![row(1, "w1:p2", "claude"), row(4, "w1:p9", "codex")];
        apply_primary(&mut rows, "w1", &store, Some(&panes(&["w1:p2", "w1:p9"])), true).unwrap();
        assert!(rows[0].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
        // other workspaces are independent; no agents -> no tag change
        assert_eq!(store.primary_pane("w2"), None);
        let mut none: Vec<AgentRow> = vec![];
        apply_primary(&mut none, "w1", &store, Some(&[]), true).unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
    }

    #[test]
    fn corrupt_state_file_is_backed_up_not_dropped() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        std::fs::write(&p, "{ not json").unwrap();
        let store = StateStore::new(&p);
        assert_eq!(store.primary_pane("w1"), None);
        store.set_primary_pane("w1", "w1:p1").unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        let backups: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("state.json.corrupt-"))
            .collect();
        assert!(!backups.is_empty(), "corrupt file kept as state.json.corrupt-<ts>");
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), "{ not json");
    }

    #[test]
    fn state_path_locations() {
        use std::ffi::OsString;
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| -> Option<OsString> { pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(*v)) }
        };
        assert_eq!(state_path_from(env(&[("TETHER_STATE", "/x/s.json"), ("HOME", "/h")])), PathBuf::from("/x/s.json"));
        assert_eq!(state_path_from(env(&[("XDG_STATE_HOME", "/xs"), ("HOME", "/h")])), PathBuf::from("/xs/tether/state.json"));
        assert_eq!(state_path_from(env(&[("HOME", "/h")])), PathBuf::from("/h/.local/state/tether/state.json"));
        assert_eq!(state_path_from(env(&[("XDG_STATE_HOME", ""), ("HOME", "/h")])), PathBuf::from("/h/.local/state/tether/state.json"));
    }

    #[test]
    fn concurrent_updates_lose_no_records() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s/state.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    // each thread has its own store (own file handles), like separate processes
                    let store = StateStore::new(path);
                    for i in 0..10 {
                        store.add_owned_tab("w1", &format!("w1:t{t}-{i}"), "x").unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let store = StateStore::new(&path);
        assert_eq!(store.owned_tabs("w1").len(), 80, "no lost records");
    }

    #[test]
    fn prune_keeps_live_and_fresh_records_only_drops_old_absent_ones() {
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        store.add_owned_tab("w1", "w1:t1", "live").unwrap();
        store.add_owned_tab("w1", "w1:t2", "gone-but-fresh").unwrap();
        let now = now_secs();
        // pretend t3 and t1 are old
        store
            .update(|st| {
                let w = st.workspaces.get_mut("w1").unwrap();
                w.tabs.push(OwnedTab { tab_id: "w1:t3".into(), label: "gone-old".into(), created_at: now - 60 });
                w.tabs.iter_mut().find(|t| t.tab_id == "w1:t1").unwrap().created_at = now - 60;
            })
            .unwrap();
        let live = vec!["w1:t1".to_string()];
        store.prune_owned_tabs("w1", &live).unwrap();
        let ids: Vec<String> = store.owned_tabs("w1").into_iter().map(|t| t.tab_id).collect();
        assert_eq!(ids, ["w1:t1", "w1:t2"], "live kept, absent+old dropped, absent+fresh (created after the tab list) kept");
        // once the fresh one ages past the grace period it goes
        store.prune_owned_tabs_at("w1", &live, now + 30).unwrap();
        assert_eq!(store.owned_tabs("w1").len(), 1);
    }

    fn fake_workspace() -> FakeHerdr {
        FakeHerdr::start(|method, params| match method {
            "agent.list" => Ok(json!({"type":"agent_list","agents":[
                {"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","agent":"claude","agent_status":"idle"},
                {"pane_id":"w1:p4","workspace_id":"w1","tab_id":"w1:t3","agent":"codex","agent_status":"blocked"},
                {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"done"},
                {"pane_id":"w1:p7","workspace_id":"w1","tab_id":"w1:t3","agent":"claude","agent_status":"working","name":"reviewer"}]})),
            "tab.list" => {
                assert_eq!(params["workspace_id"], "w1");
                Ok(json!({"type":"tab_list","tabs":[
                    {"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"},
                    {"tab_id":"w1:t3","workspace_id":"w1","number":2,"label":"review"}]}))
            }
            "pane.list" => Ok(json!({"type":"pane_list","panes":[
                {"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t1"},
                {"pane_id":"w1:p4","workspace_id":"w1","tab_id":"w1:t3"},
                {"pane_id":"w1:p7","workspace_id":"w1","tab_id":"w1:t3"}]})),
            "agent.get" => match params["target"].as_str().unwrap() {
                "w2:p1" => Ok(json!({"type":"agent_info","agent":{"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","agent":"claude","agent_status":"idle"}})),
                _ => Err(("agent_not_found".into(), "no such agent".into())),
            },
            _ => Err(("invalid_request".into(), "?".into())),
        })
    }

    fn ctx() -> Context {
        Context { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p9".into()) }
    }

    #[test]
    fn list_agents_filters_workspace_orders_by_tab_and_tags() {
        let fake = fake_workspace();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(fake.path());
        let rows = list_agents(&c, &ctx(), &store).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].pane_id, "w1:p2");
        assert_eq!((rows[0].tab_index, rows[0].tab_label.as_str(), rows[0].state.as_str()), (1, "1", "done"));
        assert!(rows[0].is_primary);
        assert_eq!(rows[1].tab_label, "review");
        assert_eq!(rows.iter().filter(|r| r.is_primary).count(), 1);
        assert_eq!(rows[0].display(), "tab 1 · claude — done");
        assert_eq!(store.primary_pane("w1"), None, "`agents` is read-only: no tag written");
        // sending (resolve_target) does tag on first sight
        assert_eq!(resolve_target(&c, &ctx(), &store, "primary").unwrap().pane_id, "w1:p2");
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
    }

    #[test]
    fn resolve_target_variants() {
        let fake = fake_workspace();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(fake.path());
        assert_eq!(resolve_target(&c, &ctx(), &store, "primary").unwrap().pane_id, "w1:p2");
        assert_eq!(resolve_target(&c, &ctx(), &store, "w1:p4").unwrap().kind, "codex");
        assert_eq!(resolve_target(&c, &ctx(), &store, "reviewer").unwrap().pane_id, "w1:p7");
        assert_eq!(resolve_target(&c, &ctx(), &store, "codex").unwrap().pane_id, "w1:p4");
        // `claude` matches two agents in w1
        let e = resolve_target(&c, &ctx(), &store, "claude").unwrap_err();
        assert!(e.to_string().contains("matches 2 agents"), "{e}");
        // explicit pane in another workspace: refused unless --workspace matches or --allow-foreign
        let e = resolve_target(&c, &ctx(), &store, "w2:p1").unwrap_err();
        assert!(matches!(e, TetherError::Refused(_)), "{e}");
        assert_eq!(e.exit_code(), crate::error::exit::REFUSED);
        assert!(e.to_string().contains("--workspace w2") && e.to_string().contains("--allow-foreign"), "{e}");
        assert!(matches!(resolve_target_opts(&c, &ctx(), &store, "w2:p1", Some("w3"), false), Err(TetherError::Refused(_))));
        assert_eq!(resolve_target_opts(&c, &ctx(), &store, "w2:p1", Some("w2"), false).unwrap().workspace_id, "w2");
        assert_eq!(resolve_target_opts(&c, &ctx(), &store, "w2:p1", None, true).unwrap().workspace_id, "w2");
        // unknown
        assert!(matches!(resolve_target(&c, &ctx(), &store, "w1:p99").unwrap_err(), TetherError::Target(_)));
        assert!(matches!(resolve_target(&c, &ctx(), &store, "nobody").unwrap_err(), TetherError::Target(_)));
    }

    #[test]
    fn primary_with_no_agents_is_a_clear_error() {
        let fake = FakeHerdr::start(|method, _| match method {
            "agent.list" => Ok(json!({"type":"agent_list","agents":[]})),
            "tab.list" => Ok(json!({"type":"tab_list","tabs":[]})),
            _ => Err(("invalid_request".into(), "?".into())),
        });
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let e = resolve_target(&Client::new(fake.path()), &ctx(), &store, "primary").unwrap_err();
        assert!(e.to_string().contains("no agent found in workspace w1"));
        assert_eq!(e.exit_code(), crate::error::exit::NO_TARGET);
    }
}
