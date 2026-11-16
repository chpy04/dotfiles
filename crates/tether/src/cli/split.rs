use clap::Args;
use serde_json::json;
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::panes::{self, Scope, SplitOpts};

#[derive(Args)]
pub struct SplitArgs {
    /// `right` or `down`
    pub direction: String,
    /// Share of the space kept by the existing pane (0 < r < 1)
    #[arg(long, default_value_t = 0.5)]
    pub ratio: f32,
    /// Pane to split (default: the current pane, $HERDR_PANE_ID)
    #[arg(long, value_name = "ID")]
    pub pane: Option<String>,
    /// Move focus to the new pane (default: do not steal focus)
    #[arg(long)]
    pub focus: bool,
    /// Act on this workspace instead of $HERDR_WORKSPACE_ID
    #[arg(long, value_name = "ID")]
    pub workspace: Option<String>,
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
    /// Command to run in the new pane, after `--`
    #[arg(last = true)]
    pub cmd: Vec<String>,
}

pub fn run(a: SplitArgs) -> Result<()> {
    let scope = Scope::from_env(a.workspace.as_deref())?;
    let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
    let opts = SplitOpts { direction: &a.direction, ratio: Some(a.ratio), pane: a.pane.as_deref(), cwd: cwd.as_deref(), focus: a.focus, cmd: &a.cmd };
    let s = panes::split(&Client::from_env(), &scope, &opts)?;
    if a.json {
        let mut v = serde_json::to_value(&s).map_err(|e| TetherError::Other(e.to_string()))?;
        v["ok"] = true.into();
        v["workspace_id"] = json!(scope.workspace_id);
        println!("{v}");
    } else {
        println!("{}", s.pane_id);
    }
    Ok(())
}
