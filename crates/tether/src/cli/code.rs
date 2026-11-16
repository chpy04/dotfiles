use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::json;
use tether_core::code::{self, CodeOpen, SocketEditor};
use tether_core::config::Config;
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::nvim;
use tether_core::panes::Scope;
use tether_core::send::CodeRef;
use tether_core::workspace::StateStore;

#[derive(Subcommand)]
pub enum CodeCmd {
    /// Open path:line in the Neovim of a tab (reuse it, or split one to the right)
    Open(OpenArgs),
}

#[derive(Args)]
pub struct OpenArgs {
    /// `path:line` or `path:line-endLine` (relative paths resolve against the worktree root)
    #[arg(value_name = "PATH:LINE[-END]")]
    pub reference: String,
    /// Tab to open it in: index, id or label (default: the current tab)
    #[arg(long = "in", value_name = "TAB")]
    pub in_tab: Option<String>,
    /// Pane to split when no Neovim exists yet (default: the current pane)
    #[arg(long, value_name = "PANE")]
    pub from: Option<String>,
    /// Move focus to the Neovim pane (default: do not steal focus)
    #[arg(long)]
    pub focus: bool,
    /// Seconds to wait for a new Neovim to start listening
    #[arg(long, default_value_t = 10)]
    pub timeout: u64,
    /// Act on this workspace instead of $HERDR_WORKSPACE_ID
    #[arg(long, value_name = "ID")]
    pub workspace: Option<String>,
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

impl CodeCmd {
    pub fn wants_json(&self) -> bool {
        match self {
            CodeCmd::Open(a) => a.json,
        }
    }
}

pub fn run(c: CodeCmd) -> Result<()> {
    match c {
        CodeCmd::Open(a) => open(a),
    }
}

fn open(a: OpenArgs) -> Result<()> {
    let reference: CodeRef = a.reference.parse().map_err(|e: TetherError| TetherError::Input(e.to_string().replace("--ref", "reference")))?;
    let cwd = std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))?;
    let file = code::resolve_file(&reference.path, &cwd)?;
    let cfg = Config::load()?;
    let scope = Scope::from_env(a.workspace.as_deref())?;
    let editor = SocketEditor { runtime: nvim::runtime_dir() };
    let o = CodeOpen {
        reference: &reference,
        file,
        in_tab: a.in_tab.as_deref(),
        from: a.from.as_deref(),
        focus: a.focus,
        ready_timeout: Duration::from_secs(a.timeout),
        wrapper: code::locate_wrapper(),
        placement: &cfg.placement,
    };
    let r = code::open(&Client::from_env(), &StateStore::from_env(), &scope, &editor, &o)?;
    for w in &r.warnings {
        eprintln!("tether: warning: {w}");
    }
    if a.json {
        let mut v = serde_json::to_value(&r).map_err(|e| TetherError::Other(e.to_string()))?;
        v["ok"] = true.into();
        v["workspace_id"] = json!(scope.workspace_id);
        println!("{v}");
    } else {
        println!(
            "{} pane {} ({}) at {}",
            if r.reused { "reused" } else { "opened" },
            r.pane_id,
            match &r.split_from {
                Some(f) => format!("new split of {f}"),
                None => "existing Neovim".to_string(),
            },
            r.at
        );
    }
    Ok(())
}
