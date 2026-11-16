//! `tether diagram create|list|show|export|update|delete|render` and the placement logic shared
//! with `tether view` (F3.1, F3.6).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use clap::{Args, Subcommand};
use serde_json::json;
use tether_core::config::Config;
use tether_core::diagram::{self, format_errors, to_mermaid_opts, Diagram, DiagramStore, StoreError};
use tether_core::error::{Result, TetherError};
use tether_core::herdr::Client;
use tether_core::panes::{self, Scope, SplitOpts};
use tether_core::tabs;
use tether_core::workspace::StateStore;

use crate::view;

#[derive(Subcommand)]
pub enum DiagramCmd {
    /// Validate a spec (JSON file or `-` for stdin), store it, print its id and open the viewer
    Create(CreateArgs),
    /// List the diagrams stored in this worktree (.tether/diagrams/)
    List(ListArgs),
    /// Print the stored spec of a diagram as JSON
    Show { id: String },
    /// Export a diagram to another format
    Export(ExportArgs),
    /// Replace the spec stored under an id (validated first)
    Update {
        id: String,
        /// New spec: JSON file or `-` for stdin
        spec: String,
    },
    /// Delete a stored diagram
    Delete { id: String },
    /// Print a static render of a diagram (stored id or spec file) without a terminal
    Render(RenderArgs),
}

impl DiagramCmd {
    pub fn wants_json(&self) -> bool {
        match self {
            DiagramCmd::Create(a) => a.json,
            DiagramCmd::List(a) => a.json,
            _ => false,
        }
    }
}

#[derive(Args)]
pub struct CreateArgs {
    /// Spec file (`-` reads stdin). See schema/flowchart.schema.json and schema/sequence.schema.json
    pub spec: String,
    /// Where to show it: `tab` (new tab named after the title), `split` (right of the current or
    /// --pane), `here` (run the viewer in this pane), `none`. Default: [placement] diagram, `tab`
    #[arg(long, value_name = "tab|split|here|none")]
    pub open: Option<String>,
    /// Pane to split for `--open split` (default: the current pane)
    #[arg(long, value_name = "PANE")]
    pub pane: Option<String>,
    /// Switch to the new tab (default: do not steal focus)
    #[arg(long)]
    pub focus: bool,
    /// Machine-readable result
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// JSON array: id, type, title, modified (unix seconds)
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ExportArgs {
    pub id: String,
    /// Mermaid source (the only format; the flag is accepted for explicitness)
    #[arg(long)]
    pub mermaid: bool,
    /// Emit plain `participant` lines (no `@{ "type": ... }`) so Mermaid versions before 11 can parse it
    #[arg(long)]
    pub compat: bool,
}

#[derive(Args)]
pub struct RenderArgs {
    /// Stored diagram id, or a path to a spec file
    pub target: String,
    /// Object id to focus
    #[arg(long)]
    pub focus: Option<String>,
    /// Comma separated object ids to expand
    #[arg(long, value_delimiter = ',')]
    pub expand: Vec<String>,
    /// ANSI truecolor output instead of plain glyphs
    #[arg(long)]
    pub ansi: bool,
    /// Plain Unicode icons instead of Nerd Font ones
    #[arg(long)]
    pub no_icons: bool,
    /// Minimum width of the render in columns
    #[arg(long, default_value_t = 60)]
    pub width: u16,
    /// Do not check code links against the files (no stale badges)
    #[arg(long)]
    pub no_resolve: bool,
}

// ------------------------------------------------------------------------------------ helpers

fn store_err(e: StoreError) -> TetherError {
    match e {
        StoreError::Invalid(_) | StoreError::NotFound(_) | StoreError::BadId(_) => TetherError::Input(e.to_string()),
        other => TetherError::Other(other.to_string()),
    }
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))
}

fn read_spec_text(spec: &str) -> Result<String> {
    if spec == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| TetherError::Input(format!("cannot read stdin: {e}")))?;
        Ok(s)
    } else {
        std::fs::read_to_string(spec).map_err(|e| TetherError::Input(format!("cannot read spec file {spec}: {e}")))
    }
}

/// Parse and validate a spec, failing with agent-readable messages (exit code 2).
pub fn parse_spec(text: &str) -> Result<Diagram> {
    diagram::validate(text).map_err(|errs| {
        TetherError::Input(format!(
            "invalid diagram spec ({} problem{}):\n{}\nschemas: schema/flowchart.schema.json, schema/sequence.schema.json (see the Diagrams section of skill/SKILL.md)",
            errs.len(),
            if errs.len() == 1 { "" } else { "s" },
            format_errors(&errs)
        ))
    })
}

/// Parse and validate a spec, then dry-run the layout (all details expanded, so panels are
/// checked too) so a spec that validates but cannot be laid out fails here with an exit-2
/// message instead of after it was stored. Uses the `auto` engine: a missing Graphviz is not
/// an input error.
pub fn parse_and_check(text: &str) -> Result<Diagram> {
    let d = parse_spec(text)?;
    check_layout(&d)?;
    Ok(d)
}

/// Dry-run [`tether_core::layout::layout`] on `d`.
pub fn check_layout(d: &Diagram) -> Result<()> {
    use std::collections::HashSet;
    use tether_core::layout::{layout, Engine, LayoutInput, ObjectId};
    let mut expanded: HashSet<ObjectId> = HashSet::new();
    match d {
        Diagram::Flowchart(f) => expanded.extend(f.nodes.iter().map(|n| ObjectId::from(n.id.as_str()))),
        Diagram::Sequence(sq) => {
            sq.walk(&mut |it| match it {
                tether_core::diagram::Item::Message(m) => {
                    expanded.insert(ObjectId::from(m.id.as_str()));
                }
                tether_core::diagram::Item::Note(n) => {
                    expanded.insert(ObjectId::from(n.id.as_str()));
                }
                _ => {}
            });
        }
    }
    let mut input = LayoutInput::new(d, &expanded);
    input.opts.engine = Engine::Auto;
    layout(&input).map(|_| ()).map_err(|e| {
        TetherError::Input(format!("the spec is valid but cannot be laid out: {e}; fix the ids/labels or simplify the diagram"))
    })
}

/// Where a diagram is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Tab,
    Split,
    Here,
    None,
}

pub fn parse_placement(s: &str) -> Result<Placement> {
    match s {
        "tab" => Ok(Placement::Tab),
        "split" => Ok(Placement::Split),
        "here" => Ok(Placement::Here),
        "none" => Ok(Placement::None),
        other => Err(TetherError::Input(format!("invalid placement `{other}`: expected tab, split, here or none"))),
    }
}

/// Environment variables worth carrying into a pane we start (the new shell does not inherit our
/// environment, only Herdr's).
const FORWARDED_ENV: [&str; 8] =
    ["TETHER_CONFIG", "TETHER_STATE", "TETHER_DOT", "TETHER_ICONS", "TETHER_NVIM", "TETHER_NVIM_WRAPPER", "TETHER_BIN", "XDG_RUNTIME_DIR"];

/// Script run by `sh -c` around the viewer: a failing viewer prints its error (already on stderr)
/// and waits for enter, so the message is not lost when the pane closes. A normal quit (`q`,
/// exit 0) closes the pane at once. The viewer and its arguments arrive as `"$@"`.
const VIEWER_SCRIPT: &str = r#""$@" || { printf '\n[tether view failed; press enter to close] '; read -r _; }"#;

/// The shell command that runs the viewer in a fresh pane:
/// `exec sh -c '<VIEWER_SCRIPT>' tether-view [env K=V…] <tether> view <id>`.
/// `exec` makes the pane close when the viewer quits; each token is shell-quoted by the caller
/// (`panes::command_line`).
pub fn viewer_command(id: &str) -> Result<Vec<String>> {
    let exe = std::env::current_exe().map_err(|e| TetherError::Other(format!("cannot locate the tether binary: {e}")))?;
    let mut cmd: Vec<String> = ["exec", "sh", "-c", VIEWER_SCRIPT, "tether-view"].map(String::from).to_vec();
    let env: Vec<String> = FORWARDED_ENV
        .iter()
        .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).map(|v| format!("{k}={v}")))
        .collect();
    if !env.is_empty() {
        cmd.push("env".into());
        cmd.extend(env);
    }
    cmd.push(exe.to_string_lossy().into_owned());
    cmd.push("view".into());
    cmd.push(id.to_string());
    Ok(cmd)
}

/// What opening a viewer produced.
pub struct Opened {
    pub mode: &'static str,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub label: Option<String>,
}

/// Show diagram `id` per `placement` (F3.1). `Here` runs the viewer in this process and only
/// returns when it quits.
pub fn open_diagram(id: &str, title: &str, placement: Placement, pane: Option<&str>, focus: bool, root: &Path) -> Result<Opened> {
    let none = Opened { mode: "none", tab_id: None, pane_id: None, label: None };
    match placement {
        Placement::None => Ok(none),
        Placement::Here => {
            view::run(id)?;
            Ok(Opened { mode: "here", ..none })
        }
        Placement::Tab => {
            let scope = Scope::from_env(None)?;
            let cmd = viewer_command(id)?;
            let cwd = root.to_string_lossy().into_owned();
            let o = tabs::open_tab(&Client::from_env(), &StateStore::from_env(), &scope, title, Some(&cwd), focus, &cmd)?;
            Ok(Opened { mode: "tab", tab_id: Some(o.tab_id), pane_id: Some(o.pane_id), label: Some(o.label) })
        }
        Placement::Split => {
            let scope = Scope::from_env(None)?;
            let cmd = viewer_command(id)?;
            let cwd = root.to_string_lossy().into_owned();
            let opts = SplitOpts { direction: "right", ratio: Some(0.5), pane, cwd: Some(&cwd), focus, cmd: &cmd };
            let s = panes::split(&Client::from_env(), &scope, &opts)?;
            Ok(Opened { mode: "split", tab_id: Some(s.tab_id), pane_id: Some(s.pane_id), label: None })
        }
    }
}

// ------------------------------------------------------------------------------------ commands

pub fn run(c: DiagramCmd) -> Result<()> {
    match c {
        DiagramCmd::Create(a) => create(a),
        DiagramCmd::List(a) => list(a),
        DiagramCmd::Show { id } => {
            let d = DiagramStore::open(&cwd()?).get(&id).map_err(store_err)?;
            println!("{}", serde_json::to_string_pretty(&d).map_err(|e| TetherError::Other(e.to_string()))?);
            Ok(())
        }
        DiagramCmd::Export(a) => {
            let _ = a.mermaid;
            let d = DiagramStore::open(&cwd()?).get(&a.id).map_err(store_err)?;
            print!("{}", to_mermaid_opts(&d, a.compat));
            Ok(())
        }
        DiagramCmd::Update { id, spec } => {
            let d = parse_and_check(&read_spec_text(&spec)?)?;
            DiagramStore::open(&cwd()?).update(&id, &d).map_err(store_err)?;
            println!("updated {id}");
            Ok(())
        }
        DiagramCmd::Delete { id } => {
            DiagramStore::open(&cwd()?).delete(&id).map_err(store_err)?;
            println!("deleted {id}");
            Ok(())
        }
        DiagramCmd::Render(a) => render(a),
    }
}

fn create(a: CreateArgs) -> Result<()> {
    // validate everything that can be wrong with the input before touching the store or Herdr
    let d = parse_and_check(&read_spec_text(&a.spec)?)?;
    let cfg = Config::load()?;
    let placement = parse_placement(a.open.as_deref().unwrap_or(&cfg.placement.diagram))?;
    let store = DiagramStore::open(&cwd()?);
    let id = store.create(&d).map_err(store_err)?;
    let path = store.dir().join(format!("{id}.json"));
    let mut summary = json!({"ok": true, "id": id, "title": d.title(), "type": d.kind().as_str(), "path": path});
    if placement == Placement::Here {
        if a.json {
            println!("{summary}");
        } else {
            println!("{id}");
        }
        open_diagram(&id, d.title(), placement, None, a.focus, store.worktree_root())?;
        return Ok(());
    }
    let opened = open_diagram(&id, d.title(), placement, a.pane.as_deref(), a.focus, store.worktree_root()).inspect_err(|_| {
        // the diagram is stored either way: tell the caller how to open it later
        eprintln!("tether: note: diagram saved as {id}; open it later with `tether view {id}`");
    })?;
    summary["opened"] = json!({"mode": opened.mode, "tab_id": opened.tab_id, "pane_id": opened.pane_id, "label": opened.label});
    if a.json {
        println!("{summary}");
    } else {
        println!("{id}");
        match opened.mode {
            "tab" => eprintln!("opened tab \"{}\" ({}) running the viewer", opened.label.unwrap_or_default(), opened.tab_id.unwrap_or_default()),
            "split" => eprintln!("opened a split ({}) running the viewer", opened.pane_id.unwrap_or_default()),
            _ => {}
        }
    }
    Ok(())
}

fn list(a: ListArgs) -> Result<()> {
    let rows = DiagramStore::open(&cwd()?).list().map_err(store_err)?;
    let secs = |t: std::time::SystemTime| t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if a.json {
        let v: Vec<_> = rows
            .iter()
            .map(|r| json!({"id": r.id, "type": r.kind.as_str(), "title": r.title, "modified": secs(r.mtime)}))
            .collect();
        println!("{}", serde_json::Value::Array(v));
        return Ok(());
    }
    if rows.is_empty() {
        println!("no diagrams in this worktree");
        return Ok(());
    }
    let w = rows.iter().map(|r| r.id.len()).max().unwrap_or(2).max(2);
    println!("{:<w$}  {:<9}  TITLE", "ID", "TYPE");
    for r in rows {
        println!("{:<w$}  {:<9}  {}", r.id, r.kind.as_str(), r.title);
    }
    Ok(())
}

fn render(a: RenderArgs) -> Result<()> {
    let cwd = cwd()?;
    let store = DiagramStore::open(&cwd);
    let d = if Path::new(&a.target).is_file() {
        parse_spec(&read_spec_text(&a.target)?)?
    } else {
        store.get(&a.target).map_err(store_err)?
    };
    let cfg = Config::load()?;
    let opts = view::RenderOpts { focus: a.focus, expand: a.expand, icons: a.no_icons.then_some(false), min_width: a.width, resolve_links: !a.no_resolve };
    let (text, ansi) = view::render_static(d, store.worktree_root().to_path_buf(), &cfg, &opts)?;
    print!("{}", if a.ansi { ansi } else { text });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn run_wrapper(args: &[&str], stdin: &str) -> (bool, String, String) {
        let mut cmd: Vec<String> = ["exec", "sh", "-c", VIEWER_SCRIPT, "tether-view"].map(String::from).to_vec();
        cmd.extend(args.iter().map(|a| a.to_string()));
        let line = panes::command_line(&cmd);
        let mut c = Command::new("sh")
            .args(["-c", &line])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let o = c.wait_with_output().unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
    }

    #[test]
    fn failing_viewer_prints_error_and_waits_for_enter() {
        let (ok, out, err) = run_wrapper(&["sh", "-c", "echo boom >&2; exit 3"], "\n");
        assert!(ok);
        assert!(err.contains("boom"), "{err}");
        assert!(out.contains("tether view failed; press enter to close"), "{out}");
    }

    #[test]
    fn clean_quit_closes_without_waiting() {
        let (ok, out, _) = run_wrapper(&["true"], "");
        assert!(ok);
        assert!(!out.contains("press enter"), "{out}");
    }
}
