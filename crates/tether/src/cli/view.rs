use clap::Args;
use tether_core::error::{Result, TetherError};

#[derive(Args)]
pub struct ViewArgs {
    /// Arguments are accepted and ignored until the viewer exists
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

pub fn run(_a: ViewArgs) -> Result<()> {
    Err(TetherError::Other(
        "`tether view` (the diagram viewer) arrives in Phase 3; until then use `tether tab open` / `tether code open`".into(),
    ))
}
