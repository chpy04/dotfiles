//! clap definitions, dispatch and error reporting.
//!
//! Exit codes (see `tether_core::error::exit`): 0 ok, 1 failure, 2 bad input/usage,
//! 3 not inside Herdr, 4 Herdr unavailable, 5 no such target, 6 refused (delivery, or a layout action
//! on something tether does not own), 7 config error.
//! Errors go to stderr as `tether: error: <message>` (+ `tether: hint: ...`); with `--json`
//! the same error is also printed to stdout as `{"ok":false,"error":{"kind","message"}}`.

mod agents;
mod code;
mod config_cmd;
mod diagram;
mod layout;
mod send;
mod split;
mod tab;
mod view;

use clap::{Parser, Subcommand};
use tether_core::error::{exit, TetherError};

#[derive(Parser)]
#[command(name = "tether", version, about = "Herdr + Neovim + Claude Code glue: comments, layout, diagrams")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a comment (text + code ref) to an agent in this Herdr workspace
    Send(send::SendArgs),
    /// List the agents in this workspace (tab, pane, kind, state, primary)
    Agents(agents::AgentsArgs),
    /// Show, locate or create the config file
    #[command(subcommand)]
    Config(config_cmd::ConfigCmd),
    /// Print this workspace's tabs and panes (commands, agents, Neovims). Run this before changing anything
    Layout(layout::LayoutArgs),
    /// Create, rename, focus or close tabs in this workspace
    #[command(subcommand)]
    Tab(tab::TabCmd),
    /// Split a pane right or down and optionally run a command in the new pane
    Split(split::SplitArgs),
    /// Open code in Neovim
    #[command(subcommand)]
    Code(code::CodeCmd),
    /// Create, list, open, star, rename, delete, prune, show, export or update diagrams (flowcharts, sequence diagrams)
    #[command(subcommand)]
    Diagram(diagram::DiagramCmd),
    /// Show a stored diagram in the viewer (in this pane, or --placement tab|split)
    View(view::ViewArgs),
    /// Browse the saved diagrams: list, live preview, star, rename, delete, open
    Diagrams(crate::browse::BrowseArgs),
}

/// Whether the user asked for JSON output (decides how errors are reported).
fn wants_json(cmd: &Command) -> bool {
    match cmd {
        Command::Send(a) => a.json,
        Command::Agents(a) => a.json,
        Command::Config(c) => c.wants_json(),
        Command::Layout(a) => a.json,
        Command::Tab(c) => c.wants_json(),
        Command::Split(a) => a.json,
        Command::Code(c) => c.wants_json(),
        Command::Diagram(c) => c.wants_json(),
        Command::View(_) | Command::Diagrams(_) => false,
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let json = wants_json(&cli.command);
    let result = match cli.command {
        Command::Send(a) => send::run(a),
        Command::Agents(a) => agents::run(a),
        Command::Config(c) => config_cmd::run(c),
        Command::Layout(a) => layout::run(a),
        Command::Tab(c) => tab::run(c),
        Command::Split(a) => split::run(a),
        Command::Code(c) => code::run(c),
        Command::Diagram(c) => diagram::run(c),
        Command::View(a) => view::run(a),
        Command::Diagrams(a) => crate::browse::run(a),
    };
    match result {
        Ok(()) => exit::OK,
        Err(e) => report(&e, json),
    }
}

fn report(e: &TetherError, json: bool) -> i32 {
    eprintln!("tether: error: {e}");
    if let Some(h) = e.hint() {
        eprintln!("tether: hint: {h}");
    }
    if json {
        let v = serde_json::json!({"ok": false, "error": {"kind": e.kind(), "message": e.to_string(), "exit_code": e.exit_code()}});
        println!("{v}");
    }
    e.exit_code()
}
