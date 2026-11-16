//! Error type shared by the tether CLI paths, with stable exit codes (the Neovim plugin shows
//! stderr and the CLI documents the codes).

use crate::herdr::HerdrError;

/// Exit codes of the `tether` binary.
pub mod exit {
    pub const OK: i32 = 0;
    /// Unclassified failure.
    pub const FAILURE: i32 = 1;
    /// Bad input: bad `--ref`, empty text, text that would trigger a slash command, ...
    pub const USAGE: i32 = 2;
    /// Not inside a Herdr pane/workspace.
    pub const NOT_IN_HERDR: i32 = 3;
    /// Herdr unreachable (socket missing, I/O error, timeout).
    pub const HERDR_UNAVAILABLE: i32 = 4;
    /// Target agent/pane not found, ambiguous, or no agent in the workspace.
    pub const NO_TARGET: i32 = 5;
    /// A refusal, nothing was done: Herdr refused delivery (`agent_not_ready`, `agent_blocked`, ...),
    /// or tether refused a layout action that would touch something it does not own (foreign tab,
    /// other workspace).
    pub const REFUSED: i32 = 6;
    /// Config file problem.
    pub const CONFIG: i32 = 7;
}

#[derive(Debug, thiserror::Error)]
pub enum TetherError {
    #[error("{0}")]
    NotInHerdr(String),
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Target(String),
    #[error("{0}")]
    Config(String),
    /// tether declined an unsafe action (closing a tab it did not create, leaving the workspace).
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, TetherError>;

impl TetherError {
    pub fn exit_code(&self) -> i32 {
        match self {
            TetherError::Input(_) => exit::USAGE,
            TetherError::NotInHerdr(_) => exit::NOT_IN_HERDR,
            TetherError::Target(_) => exit::NO_TARGET,
            TetherError::Config(_) => exit::CONFIG,
            TetherError::Refused(_) => exit::REFUSED,
            TetherError::Other(_) => exit::FAILURE,
            TetherError::Herdr(e) => match e {
                HerdrError::Connect { .. } | HerdrError::Io(_) | HerdrError::Timeout(_) | HerdrError::Protocol(_) => {
                    exit::HERDR_UNAVAILABLE
                }
                HerdrError::Api { code, .. } => match code.as_str() {
                    "agent_not_ready" | "agent_blocked" | "agent_prompt_stalled" => exit::REFUSED,
                    "agent_not_found" | "pane_not_found" | "tab_not_found" | "workspace_not_found" => exit::NO_TARGET,
                    _ => exit::FAILURE,
                },
            },
        }
    }

    /// Short machine-readable kind (also used in `--json` error output).
    pub fn kind(&self) -> String {
        match self {
            TetherError::Input(_) => "invalid_input".into(),
            TetherError::NotInHerdr(_) => "not_in_herdr".into(),
            TetherError::Target(_) => "no_target".into(),
            TetherError::Config(_) => "config".into(),
            TetherError::Refused(_) => "refused".into(),
            TetherError::Other(_) => "error".into(),
            TetherError::Herdr(HerdrError::Api { code, .. }) => code.clone(),
            TetherError::Herdr(_) => "herdr_unavailable".into(),
        }
    }

    /// Extra human advice for well-known refusals.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            TetherError::Herdr(HerdrError::Api { code, .. }) => match code.as_str() {
                "agent_blocked" => Some("the agent is waiting on an approval prompt; answer it in its pane first (nothing was sent)"),
                "agent_not_ready" => Some("the agent process is not the pane's foreground process (exited or suspended?) (nothing was sent)"),
                "agent_prompt_stalled" => Some("the agent did not accept the prompt"),
                _ => None,
            },
            _ => None,
        }
    }
}
