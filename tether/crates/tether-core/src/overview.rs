//! `tether layout` (F1.4): a compact picture of this workspace -- tabs in order, panes with their
//! foreground command, agent kind/state, primary flag and registered Neovim.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::herdr::{Client, ForegroundProcess, Snapshot};
use crate::nvim;
use crate::panes::Scope;
use crate::workspace::{self, StateStore};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaneView {
    pub pane_id: String,
    pub focused: bool,
    /// The pane tether was invoked from (`HERDR_PANE_ID`).
    pub current: bool,
    /// Foreground command from `pane.process_info` (e.g. `zsh`, `nvim src/a.ts`).
    pub command: Option<String>,
    pub agent: Option<AgentView>,
    pub primary: bool,
    /// A tether-registered Neovim that answers: where its cursor is (`path:line`).
    pub nvim: Option<NvimView>,
    /// `{x,y,width,height}` in cells, when Herdr reported a layout for the tab.
    pub rect: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentView {
    pub kind: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NvimView {
    pub socket: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabView {
    pub index: u32,
    pub tab_id: String,
    pub label: String,
    pub focused: bool,
    pub created_by_tether: bool,
    /// The caller's own tab.
    pub current: bool,
    pub panes: Vec<PaneView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutReport {
    pub workspace_id: String,
    pub workspace_label: Option<String>,
    pub tabs: Vec<TabView>,
}

/// One-line description of a foreground process: `name args...` without `--listen <path>` noise,
/// capped at `max` chars.
pub fn describe_process(fp: &ForegroundProcess, max: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut it = fp.argv.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--listen" || a == "--server" {
            it.next();
            continue;
        }
        parts.push(a.clone());
    }
    let mut s = if fp.name.is_empty() { fp.argv.first().cloned().unwrap_or_default() } else { fp.name.clone() };
    for p in parts {
        s.push(' ');
        s.push_str(&p);
    }
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        s = format!("{cut}…");
    }
    s
}

/// Pure assembly of the report from one snapshot. `command_of` and `nvim_of` are injected so the
/// logic is testable without processes or sockets.
pub fn build_report(
    snap: &Snapshot,
    scope: &Scope,
    owned_tab_ids: &[String],
    primary_pane: Option<&str>,
    command_of: &dyn Fn(&str) -> Option<String>,
    nvim_of: &dyn Fn(&str) -> Option<NvimView>,
) -> LayoutReport {
    let ws = &scope.workspace_id;
    let mut tabs: Vec<&crate::herdr::Tab> = snap.tabs.iter().filter(|t| &t.workspace_id == ws).collect();
    tabs.sort_by_key(|t| t.number);
    let views = tabs
        .into_iter()
        .map(|t| {
            let layout = snap.layouts.iter().find(|l| l["tab_id"] == t.tab_id.as_str());
            let rect_of = |pane_id: &str| -> Option<Value> {
                layout?.get("panes")?.as_array()?.iter().find(|p| p["pane_id"] == pane_id).map(|p| p["rect"].clone())
            };
            let mut panes: Vec<PaneView> = snap
                .panes
                .iter()
                .filter(|p| p.tab_id == t.tab_id)
                .map(|p| {
                    let agent = snap.agents.iter().find(|a| a.pane_id == p.pane_id);
                    let kind = agent.map(|a| a.agent.clone()).or_else(|| p.agent.clone());
                    PaneView {
                        pane_id: p.pane_id.clone(),
                        focused: p.focused,
                        current: scope.pane_id.as_deref() == Some(p.pane_id.as_str()),
                        command: command_of(&p.pane_id),
                        agent: kind.map(|kind| AgentView {
                            kind,
                            state: agent.map(|a| a.agent_status.clone()).unwrap_or_else(|| p.agent_status.clone()),
                        }),
                        primary: primary_pane == Some(p.pane_id.as_str()),
                        nvim: nvim_of(&p.pane_id),
                        rect: rect_of(&p.pane_id),
                    }
                })
                .collect();
            // reading order (top-to-bottom, left-to-right) when geometry is known
            panes.sort_by_key(|p| {
                p.rect.as_ref().map(|r| (r["y"].as_u64().unwrap_or(0), r["x"].as_u64().unwrap_or(0))).unwrap_or((0, 0))
            });
            TabView {
                index: t.number,
                tab_id: t.tab_id.clone(),
                label: t.label.clone(),
                focused: t.focused,
                created_by_tether: owned_tab_ids.contains(&t.tab_id),
                current: scope.tab_id.as_deref() == Some(t.tab_id.as_str()),
                panes,
            }
        })
        .collect();
    LayoutReport {
        workspace_id: ws.clone(),
        workspace_label: snap.workspaces.iter().find(|w| &w.workspace_id == ws).map(|w| w.label.clone()),
        tabs: views,
    }
}

/// Gather and build the report (one `session.snapshot`, one `pane.process_info` per pane, one
/// socket probe per pane that has a tether socket file).
pub fn layout(client: &Client, scope: &Scope, store: &StateStore, runtime: &Path) -> Result<LayoutReport> {
    let snap = client.session_snapshot()?;
    let live: Vec<String> = snap.tabs.iter().filter(|t| t.workspace_id == scope.workspace_id).map(|t| t.tab_id.clone()).collect();
    store.prune_owned_tabs(&scope.workspace_id, &live)?;
    let owned: Vec<String> = store.owned_tabs(&scope.workspace_id).into_iter().map(|t| t.tab_id).collect();

    // primary agent: tagged pane, else first agent in tab order (same rule as `tether agents`)
    let in_ws: Vec<_> = snap.agents.iter().filter(|a| a.workspace_id == scope.workspace_id).cloned().collect();
    let ws_tabs: Vec<_> = snap.tabs.iter().filter(|t| t.workspace_id == scope.workspace_id).cloned().collect();
    let mut rows = workspace::build_rows(&in_ws, &ws_tabs, &scope.workspace_id);
    let pane_ids: Vec<String> = snap.panes.iter().filter(|p| p.workspace_id == scope.workspace_id).map(|p| p.pane_id.clone()).collect();
    workspace::apply_primary(&mut rows, &scope.workspace_id, store, Some(&pane_ids), false)?; // read-only: never writes the tag
    let primary = rows.iter().find(|r| r.is_primary).map(|r| r.pane_id.clone());

    let command_of = |pane: &str| -> Option<String> {
        let info = client.pane_process_info(pane).ok()?;
        let fp = info.foreground_processes.first()?;
        Some(describe_process(fp, 48))
    };
    let nvim_of = |pane: &str| -> Option<NvimView> {
        let sock = nvim::socket_path_in(runtime, pane);
        nvim::probe(&sock).map(|at| NvimView { socket: sock.to_string_lossy().into_owned(), at })
    };
    Ok(build_report(&snap, scope, &owned, primary.as_deref(), &command_of, &nvim_of))
}

/// Compact text form (what Claude reads before acting).
pub fn render(r: &LayoutReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("workspace {}", r.workspace_id));
    if let Some(l) = &r.workspace_label {
        out.push_str(&format!(" \"{l}\""));
    }
    out.push('\n');
    for t in &r.tabs {
        out.push_str(&format!("tab {} \"{}\" {}", t.index, t.label, t.tab_id));
        let mut flags = Vec::new();
        if t.focused {
            flags.push("focused");
        }
        if t.current {
            flags.push("current");
        }
        flags.push(if t.created_by_tether { "tether-made" } else { "yours" });
        out.push_str(&format!("  [{}]\n", flags.join(", ")));
        for p in &t.panes {
            out.push_str(&format!("  {}", p.pane_id));
            let mut bits: Vec<String> = Vec::new();
            if let Some(a) = &p.agent {
                bits.push(format!("agent {}/{}", a.kind, a.state));
            }
            if p.primary {
                bits.push("primary".into());
            }
            if let Some(n) = &p.nvim {
                bits.push(format!("nvim@{}", n.at));
            } else if let Some(c) = &p.command {
                bits.push(c.clone());
            }
            if p.focused {
                bits.push("focused".into());
            }
            if p.current {
                bits.push("<- you".into());
            }
            out.push_str(&format!("  {}\n", bits.join("  ·  ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::{Agent, Pane, Tab, Workspace};

    fn snap() -> Snapshot {
        let pane = |id: &str, tab: &str, focused: bool, agent: Option<&str>| Pane {
            pane_id: id.into(),
            workspace_id: if id.starts_with("w2") { "w2".into() } else { "w1".into() },
            tab_id: tab.into(),
            focused,
            agent: agent.map(str::to_string),
            agent_status: agent.map(|_| "working".to_string()).unwrap_or_else(|| "unknown".into()),
            ..Default::default()
        };
        Snapshot {
            workspaces: vec![Workspace { workspace_id: "w1".into(), label: "LIN-482".into(), ..Default::default() }],
            tabs: vec![
                Tab { tab_id: "w1:t5".into(), workspace_id: "w1".into(), number: 2, label: "session refresh".into(), ..Default::default() },
                Tab { tab_id: "w1:t1".into(), workspace_id: "w1".into(), number: 1, label: "1".into(), focused: true, ..Default::default() },
                Tab { tab_id: "w2:t1".into(), workspace_id: "w2".into(), number: 1, label: "other".into(), ..Default::default() },
            ],
            panes: vec![
                pane("w1:p1", "w1:t1", true, Some("claude")),
                pane("w1:p6", "w1:t5", false, None),
                pane("w1:p7", "w1:t5", false, None),
                pane("w2:p1", "w2:t1", false, Some("claude")),
            ],
            agents: vec![Agent { pane_id: "w1:p1".into(), workspace_id: "w1".into(), agent: "claude".into(), agent_status: "working".into(), ..Default::default() }],
            layouts: vec![serde_json::json!({"tab_id":"w1:t5","panes":[
                {"pane_id":"w1:p7","rect":{"x":100,"y":0,"width":90,"height":50}},
                {"pane_id":"w1:p6","rect":{"x":0,"y":0,"width":100,"height":50}}]})],
            ..Default::default()
        }
    }

    #[test]
    fn report_is_scoped_ordered_and_annotated() {
        let scope = Scope { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()), explicit_other: false };
        let cmd = |p: &str| Some(if p == "w1:p6" { "nvim a.ts".to_string() } else { "zsh".to_string() });
        let nv = |p: &str| (p == "w1:p6").then(|| NvimView { socket: "/r/tether/w1-p6.sock".into(), at: "/w/a.ts:42".into() });
        let r = build_report(&snap(), &scope, &["w1:t5".to_string()], Some("w1:p1"), &cmd, &nv);
        assert_eq!(r.workspace_label.as_deref(), Some("LIN-482"));
        assert_eq!(r.tabs.len(), 2, "other workspace excluded");
        assert_eq!(r.tabs.iter().map(|t| t.index).collect::<Vec<_>>(), vec![1, 2]);
        let t1 = &r.tabs[0];
        assert!(t1.focused && t1.current && !t1.created_by_tether);
        let a = &t1.panes[0];
        assert!(a.primary && a.current && a.focused);
        assert_eq!(a.agent.as_ref().unwrap().state, "working");
        let t2 = &r.tabs[1];
        assert!(t2.created_by_tether);
        // reading order by rect: p6 (x=0) before p7 (x=100) although snapshot order has p6 first anyway
        assert_eq!(t2.panes.iter().map(|p| p.pane_id.as_str()).collect::<Vec<_>>(), vec!["w1:p6", "w1:p7"]);
        assert_eq!(t2.panes[0].nvim.as_ref().unwrap().at, "/w/a.ts:42");
        assert!(t2.panes[1].nvim.is_none() && t2.panes[1].agent.is_none());

        let text = render(&r);
        assert!(text.contains("tab 2 \"session refresh\" w1:t5  [tether-made]"), "{text}");
        assert!(text.contains("tab 1 \"1\" w1:t1  [focused, current, yours]"), "{text}");
        assert!(text.contains("w1:p1  agent claude/working  ·  primary  ·  zsh  ·  focused  ·  <- you"), "{text}");
        assert!(text.contains("w1:p6  nvim@/w/a.ts:42"), "{text}");
        assert!(!text.contains("w2:"), "{text}");
        // JSON is serializable and carries the flags
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["tabs"][1]["created_by_tether"], true);
    }

    #[test]
    fn describe_process_drops_listen_noise_and_truncates() {
        let fp = ForegroundProcess {
            name: "nvim".into(),
            argv: vec!["nvim".into(), "--listen".into(), "/var/very/long/tether/w1-p6.sock".into(), "+42".into(), "src/a.ts".into()],
            ..Default::default()
        };
        assert_eq!(describe_process(&fp, 48), "nvim +42 src/a.ts");
        assert_eq!(describe_process(&fp, 8), "nvim +4…");
        let z = ForegroundProcess { name: "zsh".into(), argv: vec!["-zsh".into()], ..Default::default() };
        assert_eq!(describe_process(&z, 48), "zsh");
    }
}
