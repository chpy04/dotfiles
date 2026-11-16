//! Tab operations (F1.1, F1.5, F1.6): resolve `<tab>` (index, id or label), open/rename/focus/close
//! within one workspace, and record every tab tether creates so that closing or renaming a tab
//! tether did not create is refused unless forced.

use serde::Serialize;

use crate::error::{Result, TetherError};
use crate::herdr::{Client, HerdrError, Tab, TabCreate};
use crate::panes::{self, Scope};
use crate::workspace::StateStore;

/// Resolve `spec` among `tabs` (one workspace): exact tab id, then 1-based index (digits), then
/// exact label, then case-insensitive label. Ambiguous labels are an error.
pub fn resolve_tab<'a>(tabs: &'a [Tab], spec: &str) -> Result<&'a Tab> {
    if let Some(t) = tabs.iter().find(|t| t.tab_id == spec) {
        return Ok(t);
    }
    if !spec.is_empty() && spec.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = spec.parse::<u32>() {
            if let Some(t) = tabs.iter().find(|t| t.number == n) {
                return Ok(t);
            }
        }
    }
    for exact in [true, false] {
        let hits: Vec<&Tab> = tabs
            .iter()
            .filter(|t| if exact { t.label == spec } else { t.label.eq_ignore_ascii_case(spec) })
            .collect();
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0]),
            _ => {
                return Err(TetherError::Target(format!(
                    "tab label `{spec}` is ambiguous ({}); use an index or tab id",
                    hits.iter().map(|t| format!("{} {}", t.number, t.tab_id)).collect::<Vec<_>>().join(", ")
                )))
            }
        }
    }
    let known = tabs.iter().map(|t| format!("{} \"{}\"", t.number, t.label)).collect::<Vec<_>>().join(", ");
    Err(TetherError::Target(format!("no tab `{spec}` in this workspace (tabs: {known}); see `tether layout`")))
}

/// List the scoped workspace's tabs and drop state records of tabs that are gone.
pub fn scoped_tabs(client: &Client, scope: &Scope, store: &StateStore) -> Result<Vec<Tab>> {
    let mut tabs = client.tab_list(Some(&scope.workspace_id))?;
    tabs.sort_by_key(|t| t.number);
    let live: Vec<String> = tabs.iter().map(|t| t.tab_id.clone()).collect();
    store.prune_owned_tabs(&scope.workspace_id, &live)?;
    Ok(tabs)
}

/// `resolve_tab` plus the workspace refusal: a tab id that belongs to another workspace is refused
/// with a pointer to `--workspace`.
pub fn resolve_scoped(client: &Client, scope: &Scope, store: &StateStore, spec: &str) -> Result<Tab> {
    let tabs = scoped_tabs(client, scope, store)?;
    match resolve_tab(&tabs, spec) {
        Ok(t) => Ok(t.clone()),
        Err(not_found) => {
            if spec.contains(':') {
                match client.tab_get(spec) {
                    Ok(t) if t.workspace_id != scope.workspace_id => {
                        return Err(scope.refuse_foreign(&format!("tab {spec}"), &t.workspace_id))
                    }
                    Ok(t) => return Ok(t),
                    Err(HerdrError::Api { .. }) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Err(not_found)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabOpened {
    pub tab_id: String,
    pub index: u32,
    pub label: String,
    pub pane_id: String,
    pub focused: bool,
    pub ran: Option<String>,
    /// Another tab already had this label (refer to the new one by id/index).
    pub duplicate_label: bool,
}

/// `tab open`: create a tab named `name`, record it as tether-made, optionally run `cmd`.
pub fn open_tab(
    client: &Client,
    store: &StateStore,
    scope: &Scope,
    name: &str,
    cwd: Option<&str>,
    focus: bool,
    cmd: &[String],
) -> Result<TabOpened> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TetherError::Input("tab name must not be empty".into()));
    }
    let existing = scoped_tabs(client, scope, store)?;
    let duplicate_label = existing.iter().any(|t| t.label == name);
    let (tab, root) = client.tab_create(&TabCreate { workspace_id: Some(&scope.workspace_id), label: Some(name), cwd, focus })?;
    // record before running anything: a crash after creation must still leave the tab marked as ours
    store.add_owned_tab(&scope.workspace_id, &tab.tab_id, name)?;
    if !cmd.is_empty() {
        panes::run_in_pane(client, &root.pane_id, cmd)?;
    }
    Ok(TabOpened {
        tab_id: tab.tab_id,
        index: tab.number,
        label: tab.label,
        pane_id: root.pane_id,
        focused: focus,
        ran: (!cmd.is_empty()).then(|| panes::command_line(cmd)),
        duplicate_label,
    })
}

fn refuse_foreign_tab(action: &str, tab: &Tab) -> TetherError {
    TetherError::Refused(format!(
        "refusing to {action} tab {} \"{}\" ({}): tether did not create it, so it is yours (F1.6); pass --force to override",
        tab.number, tab.label, tab.tab_id
    ))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabChange {
    pub tab_id: String,
    pub index: u32,
    pub label: String,
    pub created_by_tether: bool,
}

/// `tab close`: refuses tabs tether did not create unless `force`.
pub fn close_tab(client: &Client, store: &StateStore, scope: &Scope, spec: &str, force: bool) -> Result<TabChange> {
    let tab = resolve_scoped(client, scope, store, spec)?;
    let owned = store.is_owned_tab(&scope.workspace_id, &tab.tab_id);
    if !owned && !force {
        return Err(refuse_foreign_tab("close", &tab));
    }
    client.tab_close(&tab.tab_id)?;
    store.remove_owned_tab(&scope.workspace_id, &tab.tab_id)?;
    Ok(TabChange { tab_id: tab.tab_id, index: tab.number, label: tab.label, created_by_tether: owned })
}

/// `tab rename`: same ownership rule as close (renaming a hand-made tab changes the user's setup).
pub fn rename_tab(client: &Client, store: &StateStore, scope: &Scope, spec: &str, name: &str, force: bool) -> Result<TabChange> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TetherError::Input("tab name must not be empty".into()));
    }
    let tab = resolve_scoped(client, scope, store, spec)?;
    let owned = store.is_owned_tab(&scope.workspace_id, &tab.tab_id);
    if !owned && !force {
        return Err(refuse_foreign_tab("rename", &tab));
    }
    client.tab_rename(&tab.tab_id, name)?;
    if owned {
        store.rename_owned_tab(&scope.workspace_id, &tab.tab_id, name)?;
    }
    Ok(TabChange { tab_id: tab.tab_id, index: tab.number, label: name.to_string(), created_by_tether: owned })
}

/// `tab focus`: allowed on any tab of the workspace (it only changes which tab is shown).
pub fn focus_tab(client: &Client, store: &StateStore, scope: &Scope, spec: &str) -> Result<TabChange> {
    let tab = resolve_scoped(client, scope, store, spec)?;
    client.tab_focus(&tab.tab_id)?;
    let owned = store.is_owned_tab(&scope.workspace_id, &tab.tab_id);
    Ok(TabChange { tab_id: tab.tab_id, index: tab.number, label: tab.label, created_by_tether: owned })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHerdr;
    use serde_json::json;

    fn tab(id: &str, n: u32, label: &str) -> Tab {
        Tab { tab_id: id.into(), workspace_id: "w1".into(), number: n, label: label.into(), ..Default::default() }
    }

    fn tabs() -> Vec<Tab> {
        vec![tab("w1:t1", 1, "1"), tab("w1:t4", 2, "Tests"), tab("w1:t7", 3, "session refresh"), tab("w1:t9", 4, "dup"), tab("w1:t10", 5, "dup")]
    }

    #[test]
    fn resolves_by_id_index_and_label() {
        let t = tabs();
        assert_eq!(resolve_tab(&t, "w1:t7").unwrap().number, 3); // id
        assert_eq!(resolve_tab(&t, "2").unwrap().tab_id, "w1:t4"); // 1-based index, not id suffix
        assert_eq!(resolve_tab(&t, "session refresh").unwrap().tab_id, "w1:t7"); // label
        assert_eq!(resolve_tab(&t, "tests").unwrap().tab_id, "w1:t4"); // case-insensitive
        assert_eq!(resolve_tab(&t, "1").unwrap().number, 1);
        let amb = resolve_tab(&t, "dup").unwrap_err();
        assert!(amb.to_string().contains("ambiguous"), "{amb}");
        let none = resolve_tab(&t, "9").unwrap_err();
        assert!(matches!(none, TetherError::Target(_)));
        assert!(none.to_string().contains("no tab `9`"), "{none}");
        assert!(resolve_tab(&t, "nope").unwrap_err().to_string().contains("tabs: 1 \"1\""));
        // index wins over a label that is the same digits on another tab
        let t2 = vec![tab("w1:t1", 1, "2"), tab("w1:t2", 2, "x")];
        assert_eq!(resolve_tab(&t2, "2").unwrap().tab_id, "w1:t2");
    }

    /// Fake workspace w1 with a mutable tab list (so create/close are reflected).
    fn fake() -> FakeHerdr {
        let tabs = std::sync::Mutex::new(vec![
            json!({"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1"}),
            json!({"tab_id":"w1:t2","workspace_id":"w1","number":2,"label":"my notes"}),
        ]);
        FakeHerdr::start(move |method, params| match method {
            "tab.list" => {
                assert_eq!(params["workspace_id"], "w1", "tab.list must be scoped");
                Ok(json!({"type":"tab_list","tabs":tabs.lock().unwrap().clone()}))
            }
            "tab.get" => match params["tab_id"].as_str().unwrap() {
                "w2:t1" => Ok(json!({"type":"tab_info","tab":{"tab_id":"w2:t1","workspace_id":"w2","number":1,"label":"1"}})),
                _ => Err(("tab_not_found".into(), "no".into())),
            },
            "tab.create" => {
                assert_eq!(params["workspace_id"], "w1");
                let mut t = tabs.lock().unwrap();
                let n = t.len() + 1;
                let id = format!("w1:t{}", n + 10);
                let tab = json!({"tab_id":id,"workspace_id":"w1","number":n,"label":params["label"]});
                t.push(tab.clone());
                Ok(json!({"type":"tab_created","tab":tab,"root_pane":{"pane_id":format!("w1:p{}", n + 10),"workspace_id":"w1","tab_id":id}}))
            }
            "tab.close" => {
                let id = params["tab_id"].as_str().unwrap().to_string();
                tabs.lock().unwrap().retain(|t| t["tab_id"] != json!(id));
                Ok(json!({"type":"ok"}))
            }
            "tab.rename" | "tab.focus" | "pane.send_input" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), "?".into())),
        })
    }

    fn scope() -> Scope {
        Scope { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()), explicit_other: false }
    }

    #[test]
    fn open_records_and_runs_command_close_refuses_foreign_unless_forced() {
        let f = fake();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(f.path());

        let o = open_tab(&c, &store, &scope(), "session refresh", Some("/work"), false, &["tail".into(), "-f".into(), "x y".into()]).unwrap();
        assert_eq!((o.index, o.label.as_str(), o.tab_id.as_str()), (3, "session refresh", "w1:t13"));
        assert!(!o.duplicate_label);
        let create = &f.requests_for("tab.create")[0].params;
        assert_eq!(create["focus"], false);
        assert_eq!(create["cwd"], "/work");
        assert_eq!(f.requests_for("pane.send_input")[0].params["text"], "tail -f 'x y'");
        assert_eq!(store.owned_tabs("w1").len(), 1);
        assert!(store.is_owned_tab("w1", "w1:t13"));
        assert!(!store.is_owned_tab("w2", "w1:t13")); // keyed by workspace

        // the hand-made tab is refused, by index and by label, with the reason; nothing was closed
        for spec in ["2", "my notes", "w1:t2"] {
            let e = close_tab(&c, &store, &scope(), spec, false).unwrap_err();
            assert!(matches!(e, TetherError::Refused(_)), "{e}");
            assert!(e.to_string().contains("tether did not create it"), "{e}");
            assert!(e.to_string().contains("--force"), "{e}");
        }
        assert!(f.requests_for("tab.close").is_empty());
        // tab 1 (hand-made, the agent's own tab) too
        assert!(close_tab(&c, &store, &scope(), "1", false).is_err());

        // renaming a foreign tab is refused as well
        assert!(rename_tab(&c, &store, &scope(), "my notes", "x", false).is_err());
        assert!(f.requests_for("tab.rename").is_empty());

        // the tether-made tab closes (by label), and its record goes away
        let ch = close_tab(&c, &store, &scope(), "session refresh", false).unwrap();
        assert!(ch.created_by_tether);
        assert_eq!(f.requests_for("tab.close")[0].params["tab_id"], "w1:t13");
        assert!(store.owned_tabs("w1").is_empty());

        // --force closes a foreign tab
        let ch = close_tab(&c, &store, &scope(), "my notes", true).unwrap();
        assert!(!ch.created_by_tether);
        assert_eq!(f.requests_for("tab.close")[1].params["tab_id"], "w1:t2");
    }

    #[test]
    fn rename_keeps_record_in_sync_and_focus_is_allowed_everywhere() {
        let f = fake();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(f.path());
        open_tab(&c, &store, &scope(), "diagram", None, true, &[]).unwrap();
        assert_eq!(f.requests_for("tab.create")[0].params["focus"], true);
        let r = rename_tab(&c, &store, &scope(), "diagram", "flow", false).unwrap();
        assert_eq!(r.label, "flow");
        assert_eq!(f.requests_for("tab.rename")[0].params, json!({"tab_id":"w1:t13","label":"flow"}));
        assert_eq!(store.owned_tabs("w1")[0].label, "flow");
        // focusing a hand-made tab is fine
        let t = focus_tab(&c, &store, &scope(), "my notes").unwrap();
        assert!(!t.created_by_tether);
        assert_eq!(f.requests_for("tab.focus")[0].params["tab_id"], "w1:t2");
    }

    #[test]
    fn workspace_scoping_refuses_other_workspace_tabs() {
        let f = fake();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(f.path());
        // a tab id from workspace w2 is refused with a pointer to --workspace, even with --force
        for force in [false, true] {
            let e = close_tab(&c, &store, &scope(), "w2:t1", force).unwrap_err();
            assert!(matches!(e, TetherError::Refused(_)), "{e}");
            assert!(e.to_string().contains("workspace w2") && e.to_string().contains("--workspace w2"), "{e}");
        }
        assert!(f.requests_for("tab.close").is_empty());
        // every tab.list is scoped to w1 (asserted in the fake); unknown ids are plain not-found
        assert!(matches!(close_tab(&c, &store, &scope(), "w1:t99", true).unwrap_err(), TetherError::Target(_)));
    }

    #[test]
    fn stale_records_are_pruned_so_a_reused_id_is_not_ours() {
        let f = fake();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        // a record for a tab that no longer exists, and one for w1:t2 which does exist now
        store.add_owned_tab("w1", "w1:t55", "gone").unwrap();
        let c = Client::new(f.path());
        let _ = scoped_tabs(&c, &scope(), &store).unwrap();
        assert!(store.owned_tabs("w1").is_empty());
    }

    #[test]
    fn open_validates_name_and_flags_duplicates() {
        let f = fake();
        let d = tempfile::tempdir().unwrap();
        let store = StateStore::new(d.path().join("state.json"));
        let c = Client::new(f.path());
        assert!(matches!(open_tab(&c, &store, &scope(), "  ", None, false, &[]).unwrap_err(), TetherError::Input(_)));
        assert!(open_tab(&c, &store, &scope(), "my notes", None, false, &[]).unwrap().duplicate_label);
    }
}
