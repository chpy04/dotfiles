use clap::Args;
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::nvim;
use tether_core::overview;
use tether_core::panes::Scope;
use tether_core::workspace::StateStore;

#[derive(Args)]
pub struct LayoutArgs {
    /// Act on this workspace instead of $HERDR_WORKSPACE_ID
    #[arg(long, value_name = "ID")]
    pub workspace: Option<String>,
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

pub fn run(a: LayoutArgs) -> Result<()> {
    let scope = Scope::from_env(a.workspace.as_deref())?;
    let report = overview::layout(&Client::from_env(), &scope, &StateStore::from_env(), &nvim::runtime_dir())?;
    if a.json {
        let mut v = serde_json::to_value(&report).map_err(|e| TetherError::Other(e.to_string()))?;
        v["ok"] = true.into();
        println!("{v}");
    } else {
        print!("{}", overview::render(&report));
    }
    Ok(())
}
