use std::io::Read;

use clap::Args;
use serde_json::json;
use tether_core::config::Config;
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::send::{self, CodeRef};
use tether_core::workspace::{self, Context, StateStore};

#[derive(Args)]
pub struct SendArgs {
    /// Target: `primary` (default), a pane id (`w8:p2`), or an agent name
    #[arg(long, default_value = "primary")]
    pub to: String,

    /// Code reference `path:line` or `path:line-endLine` (relative paths resolve against the cwd)
    #[arg(long = "ref", value_name = "PATH:LINE[-END]", conflicts_with_all = ["diagram", "node"])]
    pub reference: Option<String>,

    /// Diagram id (comments on diagram nodes arrive in Phase 3)
    #[arg(long, requires = "node", conflicts_with = "reference")]
    pub diagram: Option<String>,

    /// Node id inside --diagram
    #[arg(long, requires = "diagram")]
    pub node: Option<String>,

    /// Comment text (plain text only; must not start with `/`)
    #[arg(long, allow_hyphen_values = true, conflicts_with = "text_file", required_unless_present = "text_file")]
    pub text: Option<String>,

    /// Read the comment text from this file (`-` = stdin)
    #[arg(long, value_name = "FILE", conflicts_with = "text", required_unless_present = "text")]
    pub text_file: Option<String>,

    /// Machine-readable result on stdout
    #[arg(long)]
    pub json: bool,
}

fn read_text(a: &SendArgs) -> Result<String> {
    if let Some(t) = &a.text {
        return Ok(t.clone());
    }
    let f = a.text_file.as_deref().unwrap_or("-");
    let mut s = String::new();
    if f == "-" {
        std::io::stdin().read_to_string(&mut s).map_err(|e| TetherError::Input(format!("cannot read stdin: {e}")))?;
    } else {
        s = std::fs::read_to_string(f).map_err(|e| TetherError::Input(format!("cannot read {f}: {e}")))?;
    }
    Ok(s)
}

pub fn run(a: SendArgs) -> Result<()> {
    // 1. input validation first: nothing is sent on bad input
    if a.diagram.is_some() {
        return Err(TetherError::Input(
            "--diagram/--node comments are not implemented yet (they arrive with the diagram viewer in Phase 3); use --ref path:line".into(),
        ));
    }
    let ref_str = a
        .reference
        .as_deref()
        .ok_or_else(|| TetherError::Input("one of --ref path:line[-end] or --diagram <id> --node <id> is required".into()))?;
    let code_ref: CodeRef = ref_str.parse()?;
    let text = read_text(&a)?;
    send::clean_comment_text(&text)?;

    // 2. environment
    let cfg = Config::load()?;
    let ctx = Context::from_env()?;
    let client = Client::from_env();
    let store = StateStore::from_env();

    // 3. resolve, format, deliver
    let target = workspace::resolve_target(&client, &ctx, &store, &a.to)?;
    let cwd = std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))?;
    let prepared = send::prepare_comment(&cfg.comment, &code_ref, &text, &cwd)?;
    let delivery = send::deliver(&client, &target, &prepared.payload)?;

    for w in &delivery.warnings {
        eprintln!("tether: warning: {w}");
    }
    if a.json {
        let v = json!({
            "ok": true,
            "to": a.to,
            "pane_id": target.pane_id,
            "tab_index": target.tab_index,
            "tab_label": target.tab_label,
            "kind": target.kind,
            "is_primary": target.is_primary,
            "state_before": delivery.state_before,
            "warnings": delivery.warnings,
            "ref": {"path": prepared.rel_path, "line": code_ref.line, "end_line": code_ref.end_line, "worktree": prepared.worktree},
            "bytes": delivery.bytes,
        });
        println!("{v}");
    } else {
        println!(
            "sent to {} · tab {} ({}, {})",
            target.kind, target.tab_index, target.pane_id, delivery.state_before
        );
    }
    Ok(())
}
