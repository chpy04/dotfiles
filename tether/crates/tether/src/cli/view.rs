use clap::Args;
use tether_core::diagram::DiagramStore;
use tether_core::error::{Result, TetherError};

use super::diagram::{open_diagram, open_store, parse_placement, Placement};
use crate::view;

#[derive(Args)]
pub struct ViewArgs {
    /// Diagram id (see `tether diagram list`)
    pub id: String,
    /// Where to show it: `tab`, `split` or `here`. Without it the viewer runs in this pane
    #[arg(long, value_name = "tab|split|here")]
    pub placement: Option<String>,
    /// Pane to split for `--placement split` (default: the current pane)
    #[arg(long, value_name = "PANE")]
    pub pane: Option<String>,
    /// Switch to the new tab/pane (default: do not steal focus)
    #[arg(long)]
    pub focus: bool,
}

pub fn run(a: ViewArgs) -> Result<()> {
    open_stored(&open_store()?, &a.id, a.placement.as_deref(), a.pane.as_deref(), a.focus)
}

/// Show the stored diagram `id` of `store` per `placement` (default: the viewer runs in this pane).
/// Running the viewer (here, or in the tab/pane it opens) records `last_opened`.
pub fn open_stored(store: &DiagramStore, id: &str, placement: Option<&str>, pane: Option<&str>, focus: bool) -> Result<()> {
    let placement = match placement {
        None => Placement::Here,
        Some(p) => match parse_placement(p)? {
            Placement::None => return Err(TetherError::Input("--placement none makes no sense for `tether view`".into())),
            p => p,
        },
    };
    if placement == Placement::Here {
        // a missing id is reported by the viewer; the touch just records that it was opened
        let _ = store.touch_opened(id);
        return view::run_in(store, id);
    }
    let d = store.get(id).map_err(|e| TetherError::Input(e.to_string()))?;
    let o = open_diagram(id, d.title(), placement, pane, focus, store.worktree_root())?;
    match o.mode {
        "tab" => println!("opened tab \"{}\" ({})", o.label.unwrap_or_default(), o.tab_id.unwrap_or_default()),
        _ => println!("opened pane {}", o.pane_id.unwrap_or_default()),
    }
    Ok(())
}
