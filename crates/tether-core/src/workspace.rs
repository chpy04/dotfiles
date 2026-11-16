//! Current-workspace context, the agent listing, and primary-agent tagging.
//!
//! Primary agent (PRD "Implementation defaults"): the first agent tether sees in a workspace is
//! tagged and persisted in a state file (Herdr has no storage API). If the tagged pane is gone,
//! fall back to the agent in the first tab (lowest tab number), then retag.

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

/// `$TETHER_STATE`, else `<runtime>/tether/state.json`.
pub fn state_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TETHER_STATE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::nvim::socket_dir().join("state.json")
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

    fn load(&self) -> StateFile {
        // A missing or corrupt state file just means "nothing tagged yet".
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
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
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
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
    /// reused tab id can never inherit "tether made this". Writes only when something changed.
    pub fn prune_owned_tabs(&self, workspace_id: &str, live: &[String]) -> Result<()> {
        let stale = self.owned_tabs(workspace_id).iter().any(|t| !live.contains(&t.tab_id));
        if !stale {
            return Ok(());
        }
        self.update(|st| {
            if let Some(w) = st.workspaces.get_mut(workspace_id) {
                w.tabs.retain(|t| live.contains(&t.tab_id));
            }
        })
    }

    fn update(&self, f: impl FnOnce(&mut StateFile)) -> Result<()> {
        let mut st = self.load();
        f(&mut st);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| TetherError::Other(format!("cannot create {}: {e}", dir.display())))?;
        }
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

/// Pure primary selection. `rows` must be in tab order. Returns the index of the primary:
/// the tagged pane if it is still an agent, else the first agent in tab order.
pub fn pick_primary(rows: &[AgentRow], tagged: Option<&str>) -> Option<usize> {
    if let Some(t) = tagged {
        if let Some(i) = rows.iter().position(|r| r.pane_id == t) {
            return Some(i);
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(0)
    }
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
    rows.sort_by_key(|(i, r)| (r.tab_index, *i));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// Apply primary tagging: reads the tag, picks, flags the row, and persists a changed tag.
pub fn apply_primary(rows: &mut [AgentRow], workspace_id: &str, store: &StateStore) -> Result<()> {
    let tagged = store.primary_pane(workspace_id);
    if let Some(i) = pick_primary(rows, tagged.as_deref()) {
        rows[i].is_primary = true;
        if tagged.as_deref() != Some(rows[i].pane_id.as_str()) {
            store.set_primary_pane(workspace_id, &rows[i].pane_id)?;
        }
    }
    Ok(())
}

/// Every agent in the context's workspace (tab order), with the primary flagged (and tagged on
/// first sight).
pub fn list_agents(client: &Client, ctx: &Context, store: &StateStore) -> Result<Vec<AgentRow>> {
    let agents = client.agent_list()?;
    let tabs = client.tab_list(Some(&ctx.workspace_id))?;
    let mut rows = build_rows(&agents, &tabs, &ctx.workspace_id);
    apply_primary(&mut rows, &ctx.workspace_id, store)?;
    Ok(rows)
}

/// Resolve `--to`: `primary`, a pane id (`w8:p2`), or an agent name (also: a unique agent kind).
pub fn resolve_target(client: &Client, ctx: &Context, store: &StateStore, to: &str) -> Result<AgentRow> {
    let rows = list_agents(client, ctx, store)?;
    resolve_in_rows(client, ctx, &rows, to)
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

    #[test]
    fn pick_primary_rules() {
        let rows = vec![row(1, "w1:p1", "claude"), row(3, "w1:p5", "codex")];
        assert_eq!(pick_primary(&rows, None), Some(0)); // first seen = tab order
        assert_eq!(pick_primary(&rows, Some("w1:p5")), Some(1)); // tag wins, even if not tab 1
        assert_eq!(pick_primary(&rows, Some("w1:gone")), Some(0)); // tagged pane vanished -> first tab
        assert_eq!(pick_primary(&[], Some("w1:p1")), None);
    }

    #[test]
    fn tagging_persists_and_retags_when_pane_disappears() {
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("t/state.json"));
        // first sight: tab 1 agent is tagged
        let mut rows = vec![row(1, "w1:p1", "claude"), row(2, "w1:p2", "claude")];
        apply_primary(&mut rows, "w1", &store).unwrap();
        assert!(rows[0].is_primary && !rows[1].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        // tabs reordered: tag sticks to pane p1 although it is now tab 2
        let mut rows = vec![row(1, "w1:p2", "claude"), row(2, "w1:p1", "claude")];
        apply_primary(&mut rows, "w1", &store).unwrap();
        assert!(rows[1].is_primary && !rows[0].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
        // p1 closes: fall back to tab 1's agent and retag
        let mut rows = vec![row(1, "w1:p2", "claude"), row(4, "w1:p9", "codex")];
        apply_primary(&mut rows, "w1", &store).unwrap();
        assert!(rows[0].is_primary);
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
        // other workspaces are independent; no agents -> no tag change
        assert_eq!(store.primary_pane("w2"), None);
        let mut none: Vec<AgentRow> = vec![];
        apply_primary(&mut none, "w1", &store).unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p2"));
    }

    #[test]
    fn corrupt_state_file_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        std::fs::write(&p, "{ not json").unwrap();
        let store = StateStore::new(&p);
        assert_eq!(store.primary_pane("w1"), None);
        store.set_primary_pane("w1", "w1:p1").unwrap();
        assert_eq!(store.primary_pane("w1").as_deref(), Some("w1:p1"));
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
        // explicit pane in another workspace is allowed (asks Herdr)
        assert_eq!(resolve_target(&c, &ctx(), &store, "w2:p1").unwrap().workspace_id, "w2");
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
