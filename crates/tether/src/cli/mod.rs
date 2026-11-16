//! clap definitions, dispatch and error reporting.
//!
//! Exit codes (see `tether_core::error::exit`): 0 ok, 1 failure, 2 bad input/usage,
//! 3 not inside Herdr, 4 Herdr unavailable, 5 no such target, 6 delivery refused, 7 config error.
//! Errors go to stderr as `tether: error: <message>` (+ `tether: hint: ...`); with `--json`
//! the same error is also printed to stdout as `{"ok":false,"error":{"kind","message"}}`.

mod agents;
mod config_cmd;
mod send;

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
}

/// Whether the user asked for JSON output (decides how errors are reported).
fn wants_json(cmd: &Command) -> bool {
    match cmd {
        Command::Send(a) => a.json,
        Command::Agents(a) => a.json,
        Command::Config(c) => c.wants_json(),
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let json = wants_json(&cli.command);
    let result = match cli.command {
        Command::Send(a) => send::run(a),
        Command::Agents(a) => agents::run(a),
        Command::Config(c) => config_cmd::run(c),
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
