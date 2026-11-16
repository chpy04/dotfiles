use clap::Args;
use tether_core::error::Result;
use tether_core::herdr::Client;
use tether_core::workspace::{self, Context, StateStore};

#[derive(Args)]
pub struct AgentsArgs {
    /// JSON array: tab_index, tab_label, tab_id, pane_id, kind, state, name?, is_primary
    #[arg(long)]
    pub json: bool,
}

pub fn run(a: AgentsArgs) -> Result<()> {
    let ctx = Context::from_env()?;
    let rows = workspace::list_agents(&Client::from_env(), &ctx, &StateStore::from_env())?;
    if a.json {
        println!("{}", serde_json::to_string(&rows).map_err(|e| tether_core::error::TetherError::Other(e.to_string()))?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("no agents in workspace {}", ctx.workspace_id);
        return Ok(());
    }
    println!("{:<4} {:<12} {:<8} {:<10} {:<9} PRIMARY", "TAB", "LABEL", "PANE", "AGENT", "STATE");
    for r in &rows {
        println!(
            "{:<4} {:<12} {:<8} {:<10} {:<9} {}",
            r.tab_index,
            r.tab_label,
            r.pane_id,
            r.kind,
            r.state,
            if r.is_primary { "*" } else { "" }
        );
    }
    Ok(())
}
