use clap::{Args, Subcommand};
use serde_json::json;
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::panes::Scope;
use tether_core::tabs;
use tether_core::workspace::StateStore;

#[derive(Args)]
pub struct Common {
    /// Act on this workspace instead of $HERDR_WORKSPACE_ID
    #[arg(long, value_name = "ID")]
    pub workspace: Option<String>,
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum TabCmd {
    /// Create a tab named <name> (recorded as tether-made) and optionally run a command in it
    Open {
        /// Tab label, named after the subject (e.g. "session refresh")
        name: String,
        /// Switch to the new tab (default: do not steal focus)
        #[arg(long, conflicts_with = "no_focus")]
        focus: bool,
        #[arg(long)]
        no_focus: bool,
        #[command(flatten)]
        common: Common,
        /// Command to run in the new tab's pane, after `--`
        #[arg(last = true)]
        cmd: Vec<String>,
    },
    /// Rename a tab (only tabs tether created, unless --force)
    Rename {
        /// Tab index, id or label
        tab: String,
        name: String,
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        common: Common,
    },
    /// Switch to a tab (any tab)
    Focus {
        tab: String,
        #[command(flatten)]
        common: Common,
    },
    /// Close a tab (only tabs tether created, unless --force)
    Close {
        tab: String,
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        common: Common,
    },
}

impl TabCmd {
    pub fn wants_json(&self) -> bool {
        match self {
            TabCmd::Open { common, .. } | TabCmd::Rename { common, .. } | TabCmd::Focus { common, .. } | TabCmd::Close { common, .. } => {
                common.json
            }
        }
    }
}

pub fn run(c: TabCmd) -> Result<()> {
    let client = Client::from_env();
    let store = StateStore::from_env();
    match c {
        TabCmd::Open { name, focus, no_focus: _, common, cmd } => {
            let scope = Scope::from_env(common.workspace.as_deref())?;
            let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
            let o = tabs::open_tab(&client, &store, &scope, &name, cwd.as_deref(), focus, &cmd)?;
            if o.duplicate_label {
                eprintln!("tether: warning: another tab is already labeled \"{}\"; use the tab id {} to address this one", o.label, o.tab_id);
            }
            if common.json {
                let mut v = serde_json::to_value(&o).map_err(|e| TetherError::Other(e.to_string()))?;
                v["ok"] = true.into();
                v["workspace_id"] = json!(scope.workspace_id);
                println!("{v}");
            } else {
                println!("opened tab {} \"{}\" ({}) pane {}{}", o.index, o.label, o.tab_id, o.pane_id, if o.focused { " [focused]" } else { "" });
            }
        }
        TabCmd::Rename { tab, name, force, common } => {
            let scope = Scope::from_env(common.workspace.as_deref())?;
            let r = tabs::rename_tab(&client, &store, &scope, &tab, &name, force)?;
            report(&common, "renamed", &r, &scope)?;
        }
        TabCmd::Focus { tab, common } => {
            let scope = Scope::from_env(common.workspace.as_deref())?;
            let r = tabs::focus_tab(&client, &store, &scope, &tab)?;
            report(&common, "focused", &r, &scope)?;
        }
        TabCmd::Close { tab, force, common } => {
            let scope = Scope::from_env(common.workspace.as_deref())?;
            let r = tabs::close_tab(&client, &store, &scope, &tab, force)?;
            report(&common, "closed", &r, &scope)?;
        }
    }
    Ok(())
}

fn report(common: &Common, verb: &str, r: &tabs::TabChange, scope: &Scope) -> Result<()> {
    if common.json {
        let mut v = serde_json::to_value(r).map_err(|e| TetherError::Other(e.to_string()))?;
        v["ok"] = true.into();
        v["action"] = json!(verb);
        v["workspace_id"] = json!(scope.workspace_id);
        println!("{v}");
    } else {
        println!("{verb} tab {} \"{}\" ({})", r.index, r.label, r.tab_id);
    }
    Ok(())
}
