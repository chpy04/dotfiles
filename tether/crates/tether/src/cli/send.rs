use std::io::Read;

use clap::Args;
use serde_json::json;
use tether_core::config::Config;
use tether_core::diagram::{self, DiagramStore};
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

    /// Diagram id: comment on an element of a stored diagram (needs --node)
    #[arg(long, requires = "node", conflicts_with = "reference")]
    pub diagram: Option<String>,

    /// Element id inside --diagram: a flowchart node, or a sequence message/note/participant id
    #[arg(long, requires = "diagram")]
    pub node: Option<String>,

    /// Allow `--to <pane-id>` to name a pane of this other workspace (default: refused, exit 6)
    #[arg(long, value_name = "ID")]
    pub workspace: Option<String>,

    /// Allow `--to <pane-id>` to name a pane of any other workspace
    #[arg(long)]
    pub allow_foreign: bool,

    /// Comment text (plain text only; `/` first is refused only if the [comment] template starts with {text})
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
    let text = read_text(&a)?;
    let cwd = std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))?;
    let source = match (&a.diagram, a.reference.as_deref()) {
        (Some(id), _) => Source::Diagram { id: id.clone(), node: a.node.clone().unwrap_or_default() },
        (None, Some(r)) => Source::Ref(r.parse()?),
        (None, None) => {
            return Err(TetherError::Input("one of --ref path:line[-end] or --diagram <id> --node <id> is required".into()))
        }
    };
    // diagram comments are built before touching Herdr: a bad id/node is a usage error
    let cfg = Config::load()?;
    send::clean_comment_text_for(&cfg.comment.template, &text)?; // leading `/` only matters if the template puts {text} first
    let diagram_prepared = match &source {
        Source::Diagram { id, node } => Some(prepare_diagram(&cfg, id, node, &text, &cwd)?),
        Source::Ref(_) => None,
    };

    // 2. environment
    let ctx = Context::from_env()?;
    let client = Client::from_env();
    let store = StateStore::from_env();

    // 3. resolve, format, deliver
    let target = workspace::resolve_target_opts(&client, &ctx, &store, &a.to, a.workspace.as_deref(), a.allow_foreign)?;
    let prepared = match (&source, diagram_prepared) {
        (Source::Ref(code_ref), _) => send::prepare_comment(&cfg.comment, code_ref, &text, &cwd)?,
        (Source::Diagram { .. }, Some(p)) => p,
        (Source::Diagram { .. }, None) => unreachable!("diagram comments are prepared above"),
    };
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
            "ref": match &source {
                Source::Ref(code_ref) => json!({"path": prepared.rel_path, "line": code_ref.line, "end_line": code_ref.end_line, "worktree": prepared.worktree}),
                Source::Diagram { id, node } => json!({"diagram": id, "node": node, "path": prepared.rel_path, "worktree": prepared.worktree}),
            },
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

enum Source {
    Ref(CodeRef),
    Diagram { id: String, node: String },
}

/// Build the payload for `--diagram <id> --node <obj-id>` (F3.3): diagram title, element label and
/// its code ref with the live code line.
fn prepare_diagram(cfg: &Config, id: &str, node: &str, text: &str, cwd: &std::path::Path) -> Result<send::Prepared> {
    let store = DiagramStore::open(cwd);
    let diagram = store.get(id).map_err(|e| TetherError::Input(e.to_string()))?;
    let el = diagram::find_element(&diagram, node).ok_or_else(|| {
        let ids: Vec<String> = diagram::elements(&diagram).into_iter().map(|e| e.id).collect();
        TetherError::Input(format!("diagram `{id}` has no element `{node}` (ids: {})", ids.join(", ")))
    })?;
    let target = send::DiagramTarget { diagram_title: diagram.title(), kind: el.kind.word(), label: &el.label, code: el.code.as_ref() };
    send::prepare_diagram_comment(&cfg.comment, &target, text, store.worktree_root())
}
