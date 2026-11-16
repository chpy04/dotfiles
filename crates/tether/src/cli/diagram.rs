//! `tether diagram create|list|open|star|unstar|rename|rm|prune|path|browse|show|export|update|render`
//! and the placement logic shared with `tether view` (F3.1, F3.6).
//!
//! Diagrams are stored per repo under the data dir (see `tether_core::diagram::store`).

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde_json::json;
use tether_core::config::Config;
use tether_core::diagram::{
    self, data_dir, format_age, format_errors, match_query, to_mermaid_opts, CreateOpts, Diagram, DiagramInfo, DiagramStore, StoreError,
    DEFAULT_KEEP_DAYS,
};
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
    /// List the saved diagrams of this repo (newest first); `--all-repos` for every repo
    List(ListArgs),
    /// Open a saved diagram in the viewer, by id or by a (partial, case-insensitive) title
    Open(OpenArgs),
    /// Star a diagram, by id or title words (starred diagrams are never pruned)
    Star { id: String },
    /// Remove the star of a diagram
    Unstar { id: String },
    /// Change a diagram's title (the id stays)
    Rename {
        /// Diagram id, or words of its title
        id: String,
        /// The new title
        #[arg(required = true, num_args = 1..)]
        title: Vec<String>,
    },
    /// Delete saved diagrams by id
    #[command(alias = "delete")]
    Rm {
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
    },
    /// Remove unstarred diagrams not opened or changed for `[diagrams] keep_days` days
    Prune {
        /// Only list what would be removed
        #[arg(long)]
        dry_run: bool,
    },
    /// Print the folder this repo's diagrams are saved in
    Path,
    /// Browse the saved diagrams in a full-screen list with a live preview (same as `tether diagrams`)
    Browse(crate::browse::BrowseArgs),
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
    /// Star the diagram so it is kept (never pruned)
    #[arg(long)]
    pub star: bool,
    /// Machine-readable result
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// JSON array: id, type, title, starred, branch, worktree, repo, created/updated/last_opened/activity (unix seconds)
    #[arg(long)]
    pub json: bool,
    /// Include the diagrams of every repo, not just this one
    #[arg(long)]
    pub all_repos: bool,
    /// Only starred diagrams
    #[arg(long)]
    pub starred: bool,
}

#[derive(Args)]
pub struct OpenArgs {
    /// Diagram id, or words of its title (case-insensitive substring; must match exactly one diagram)
    #[arg(required = true, num_args = 1..)]
    pub query: Vec<String>,
    /// Where to show it: `tab`, `split` or `here` (default: the viewer runs in this pane)
    #[arg(long, value_name = "tab|split|here")]
    pub placement: Option<String>,
    /// Pane to split for `--placement split` (default: the current pane)
    #[arg(long, value_name = "PANE")]
    pub pane: Option<String>,
    /// Switch to the new tab/pane (default: do not steal focus)
    #[arg(long)]
    pub focus: bool,
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
const FORWARDED_ENV: [&str; 10] = [
    "TETHER_CONFIG",
    "TETHER_STATE",
    "TETHER_DATA",
    "TETHER_DOT",
    "TETHER_ICONS",
    "TETHER_NVIM",
    "TETHER_NVIM_WRAPPER",
    "TETHER_BIN",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
];

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
            let d = open_store()?.get(&id).map_err(store_err)?;
            println!("{}", serde_json::to_string_pretty(&d).map_err(|e| TetherError::Other(e.to_string()))?);
            Ok(())
        }
        DiagramCmd::Export(a) => {
            let _ = a.mermaid;
            let d = open_store()?.get(&a.id).map_err(store_err)?;
            print!("{}", to_mermaid_opts(&d, a.compat));
            Ok(())
        }
        DiagramCmd::Update { id, spec } => {
            let d = parse_and_check(&read_spec_text(&spec)?)?;
            open_store()?.update(&id, &d).map_err(store_err)?;
            println!("updated {id}");
            Ok(())
        }
        DiagramCmd::Open(a) => open(a),
        DiagramCmd::Star { id } => set_star(&id, true),
        DiagramCmd::Unstar { id } => set_star(&id, false),
        DiagramCmd::Rename { id, title } => {
            let title = title.join(" ");
            let store = open_store()?;
            let id = resolve_query(&store, &id)?.id;
            store.rename(&id, &title).map_err(store_err)?;
            println!("renamed {id} to \"{}\"", title.trim());
            Ok(())
        }
        DiagramCmd::Rm { ids } => rm(&ids),
        DiagramCmd::Prune { dry_run } => prune(dry_run),
        DiagramCmd::Path => {
            println!("{}", open_store()?.dir().display());
            Ok(())
        }
        DiagramCmd::Browse(a) => crate::browse::run(a),
        DiagramCmd::Render(a) => render(a),
    }
}

fn create(a: CreateArgs) -> Result<()> {
    // validate everything that can be wrong with the input before touching the store or Herdr
    let d = parse_and_check(&read_spec_text(&a.spec)?)?;
    let cfg = Config::load()?;
    let placement = parse_placement(a.open.as_deref().unwrap_or(&cfg.placement.diagram))?;
    let store = open_store_with(&cfg)?;
    let id = store.create_with(&d, &CreateOpts { starred: a.star, note: None }).map_err(store_err)?;
    let path = store.dir().join(format!("{id}.json"));
    let mut summary = json!({"ok": true, "id": id, "title": d.title(), "type": d.kind().as_str(), "starred": a.star, "path": path});
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

/// The store of the repo containing the cwd, with `[diagrams] keep_days` applied. A broken config
/// does not stop a command that only reads or edits diagrams (the default retention applies).
pub fn open_store() -> Result<DiagramStore> {
    let keep = Config::load().map(|c| c.diagrams.keep_days).unwrap_or(DEFAULT_KEEP_DAYS);
    Ok(DiagramStore::open(&cwd()?).with_keep_days(keep))
}

fn open_store_with(cfg: &Config) -> Result<DiagramStore> {
    Ok(DiagramStore::open(&cwd()?).with_keep_days(cfg.diagrams.keep_days))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn info_json(r: &DiagramInfo) -> serde_json::Value {
    json!({
        "id": r.id, "type": r.kind.as_str(), "title": r.title, "starred": r.starred,
        "branch": r.branch, "worktree": r.worktree, "note": r.note,
        "repo": r.repo, "repo_key": r.repo_key,
        "created": r.created, "updated": r.updated, "last_opened": r.last_opened,
        "activity": r.activity(),
        "modified": r.updated,
    })
}

fn list(a: ListArgs) -> Result<()> {
    let store = open_store()?;
    // `list` also prunes (at most daily) and imports an old `.tether/diagrams/`; both report on stderr
    let mut rows = store.list().map_err(store_err)?;
    if a.all_repos {
        for other in DiagramStore::all_repos(&data_dir()) {
            if other.repo_key() != store.repo_key() {
                rows.extend(other.rows().map_err(store_err)?);
            }
        }
        rows.sort_by(|x, y| y.activity().cmp(&x.activity()).then_with(|| x.id.cmp(&y.id)));
    }
    if a.starred {
        rows.retain(|r| r.starred);
    }
    if a.json {
        println!("{}", serde_json::Value::Array(rows.iter().map(info_json).collect()));
        return Ok(());
    }
    if rows.is_empty() {
        let what = if a.starred { "no starred diagrams" } else { "no diagrams saved" };
        if a.all_repos {
            println!("{what} in any repo yet");
        } else {
            println!("{what} for {} (folder: {})", store.repo_name(), store.dir().display());
        }
        return Ok(());
    }
    let now = now_secs();
    let w = rows.iter().map(|r| r.id.len()).max().unwrap_or(2).max(2);
    let bw = rows.iter().map(|r| r.branch.as_deref().unwrap_or("-").chars().count()).max().unwrap_or(6).clamp(6, 24);
    let rw = if a.all_repos { rows.iter().map(|r| r.repo.chars().count()).max().unwrap_or(4).max(4) } else { 0 };
    let repo_head = if a.all_repos { format!("{:<rw$}  ", "REPO") } else { String::new() };
    println!("  {repo_head}{:<w$}  {:<9}  {:<bw$}  {:>4}  TITLE", "ID", "TYPE", "BRANCH", "AGE");
    for r in rows {
        let star = if r.starred { "★" } else { " " };
        let repo = if a.all_repos { format!("{:<rw$}  ", r.repo) } else { String::new() };
        let branch: String = r.branch.as_deref().unwrap_or("-").chars().take(bw).collect();
        println!("{star} {repo}{:<w$}  {:<9}  {:<bw$}  {:>4}  {}", r.id, r.kind.as_str(), branch, format_age(now, r.activity()), r.title);
    }
    Ok(())
}

/// Resolve `<id|title words>` against this repo's diagrams: exactly one match, or an exit-2 error that
/// lists the candidates.
pub fn resolve_query(store: &DiagramStore, query: &str) -> Result<DiagramInfo> {
    let rows = store.rows().map_err(store_err)?;
    let found = match_query(&rows, query);
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(TetherError::Input(format!(
            "no saved diagram matches \"{query}\" in {}; run `tether diagram list` to see them",
            store.repo_name()
        ))),
        many => {
            let now = now_secs();
            let mut msg = format!("\"{query}\" matches {} diagrams in {}; use the id or a longer title:", many.len(), store.repo_name());
            for r in many {
                msg.push_str(&format!(
                    "\n  {}{}  {}  ({}, {})",
                    if r.starred { "★ " } else { "" },
                    r.id,
                    r.title,
                    r.kind.as_str(),
                    format_age(now, r.activity())
                ));
            }
            Err(TetherError::Input(msg))
        }
    }
}

fn open(a: OpenArgs) -> Result<()> {
    let store = open_store()?;
    let hit = resolve_query(&store, &a.query.join(" "))?;
    super::view::open_stored(&store, &hit.id, a.placement.as_deref(), a.pane.as_deref(), a.focus)
}

fn set_star(id: &str, on: bool) -> Result<()> {
    let store = open_store()?;
    let id = resolve_query(&store, id)?.id;
    let id = id.as_str();
    let changed = store.set_starred(id, on).map_err(store_err)?;
    let title = store.info(id).map(|i| i.title).unwrap_or_default();
    match (on, changed) {
        (true, true) => println!("starred {id}  {title}"),
        (true, false) => println!("{id} was already starred"),
        (false, true) => println!("unstarred {id}  {title}"),
        (false, false) => println!("{id} was not starred"),
    }
    Ok(())
}

fn rm(ids: &[String]) -> Result<()> {
    let store = open_store()?;
    let mut failed: Vec<String> = Vec::new();
    for id in ids {
        match store.delete(id) {
            Ok(()) => println!("deleted {id}"),
            Err(e) => {
                eprintln!("tether: error: {e}");
                failed.push(id.clone());
            }
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(TetherError::Input(format!("{} of {} not deleted: {}", failed.len(), ids.len(), failed.join(", "))))
    }
}

fn prune(dry_run: bool) -> Result<()> {
    let store = open_store()?;
    let days = Config::load().map(|c| c.diagrams.keep_days).unwrap_or(DEFAULT_KEEP_DAYS);
    if days == 0 {
        println!("pruning is off ([diagrams] keep_days = 0); nothing removed");
        return Ok(());
    }
    let gone = store.prune(dry_run).map_err(store_err)?;
    if gone.is_empty() {
        println!("nothing to prune (unstarred diagrams are kept {days} days from their last open or change)");
        return Ok(());
    }
    let now = now_secs();
    for r in &gone {
        println!("{} {}  {}  (last active {} ago)", if dry_run { "would remove" } else { "removed" }, r.id, r.title, format_age(now, r.activity()));
    }
    if dry_run {
        println!("{} diagram(s) would be removed; star the ones to keep with `tether diagram star <id>`", gone.len());
    }
    Ok(())
}

fn render(a: RenderArgs) -> Result<()> {
    let store = open_store()?;
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
