//! `~/.config/tether/config.toml` (override the path with `TETHER_CONFIG`).
//! A missing file means defaults. Unknown keys are an error (typos should not be silent).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TetherError};

/// Default payload template (F2.4). Placeholders: `{path} {line} {range} {worktree} {code} {text}`.
pub const DEFAULT_TEMPLATE: &str = "[comment] {path}:{range}  (worktree: {worktree})\n{code}\n{text}";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keys: Keys,
    pub comment: CommentConfig,
    pub placement: Placement,
    pub nvim: NvimConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// Neovim, normal + visual mode: comment on the line/range to the primary agent.
    pub comment: String,
    /// Neovim: pick the target agent first.
    pub comment_pick: String,
    // Viewer keys (Phase 3+).
    pub view_down: String,
    pub view_up: String,
    pub view_left: String,
    pub view_right: String,
    pub view_open: String,
    pub view_details: String,
    pub view_search: String,
    pub view_search_next: String,
    pub view_quit: String,
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            comment: "<leader>ac".into(),
            comment_pick: "<leader>aC".into(),
            view_down: "j".into(),
            view_up: "k".into(),
            view_left: "h".into(),
            view_right: "l".into(),
            view_open: "<CR>".into(),
            view_details: "<Tab>".into(),
            view_search: "/".into(),
            view_search_next: "n".into(),
            view_quit: "q".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommentConfig {
    pub template: String,
    /// Longest code excerpt (in lines) put into a comment; longer ranges are truncated.
    pub max_code_lines: usize,
}

impl Default for CommentConfig {
    fn default() -> Self {
        CommentConfig { template: DEFAULT_TEMPLATE.into(), max_code_lines: 20 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Placement {
    /// Where `tether view` opens a diagram: `tab` | `split` | `here`.
    pub diagram: String,
    /// Direction in which code opens next to a visual: `right` | `down`.
    pub code_direction: String,
    /// Fraction of the space kept by the existing pane when splitting for code.
    pub code_ratio: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Placement { diagram: "tab".into(), code_direction: "right".into(), code_ratio: 0.5 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NvimConfig {
    /// Command used to launch Neovim (word-split by `bin/tether-nvim`; may contain flags).
    pub command: String,
}

impl Default for NvimConfig {
    fn default() -> Self {
        NvimConfig { command: "nvim".into() }
    }
}

/// Commented default config written by `tether config init`.
pub const DEFAULT_FILE: &str = r#"# tether configuration. Every key is optional; the values shown are the defaults.

[keys]
# Neovim: comment on the current line (normal mode) or the selected range (visual mode).
comment = "<leader>ac"
# Neovim: pick which agent receives the comment first.
comment_pick = "<leader>aC"
# Diagram viewer keys (used from Phase 3 on).
view_down = "j"
view_up = "k"
view_left = "h"
view_right = "l"
view_open = "<CR>"
view_details = "<Tab>"
view_search = "/"
view_search_next = "n"
view_quit = "q"

[comment]
# Payload sent to the agent. Placeholders:
#   {path}      path relative to the worktree root
#   {line}      first line number
#   {range}     "142" or "142-148" (visual mode)
#   {worktree}  basename of `git rev-parse --show-toplevel`
#   {code}      the code lines, formatted "  142 │ ..." (the line is dropped if unavailable)
#   {text}      your comment (required)
# The final message must not start with "/" or "!" (no slash commands); the default starts with "[comment]".
template = '''
[comment] {path}:{range}  (worktree: {worktree})
{code}
{text}'''
# Longest code excerpt (lines) included in a comment.
max_code_lines = 20

[placement]
# Where a diagram opens: "tab" | "split" | "here".
diagram = "tab"
# Where code opens relative to a visual: "right" | "down".
code_direction = "right"
# Fraction of the space the existing pane keeps when splitting.
code_ratio = 0.5

[nvim]
# Command used to launch Neovim from `tether-nvim` and `tether code open`.
command = "nvim"
"#;

/// The config path: `$TETHER_CONFIG`, else `$XDG_CONFIG_HOME/tether/config.toml`,
/// else `~/.config/tether/config.toml`.
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TETHER_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".config"),
    };
    base.join("tether/config.toml")
}

impl Config {
    /// Parse a TOML string and validate it.
    pub fn parse(s: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(s).map_err(|e| TetherError::Config(format!("invalid config: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.comment.template.contains("{text}") {
            return Err(TetherError::Config("invalid config: [comment] template must contain {text}".into()));
        }
        if self.comment.max_code_lines == 0 {
            return Err(TetherError::Config("invalid config: [comment] max_code_lines must be at least 1".into()));
        }
        Ok(())
    }

    /// Load from `config_path()`; a missing file yields defaults.
    pub fn load() -> Result<Config> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &std::path::Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| TetherError::Config(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(TetherError::Config(format!("cannot read {}: {e}", path.display()))),
        }
    }

    /// Dotted-key lookup for `tether config get` (e.g. `nvim.command`, `keys.comment`).
    pub fn get(&self, dotted: &str) -> Option<serde_json::Value> {
        let v = serde_json::to_value(self).ok()?;
        let mut cur = &v;
        for part in dotted.split('.') {
            cur = cur.get(part)?;
        }
        Some(cur.clone())
    }
}

/// Write the commented default file; refuses to overwrite an existing one.
pub fn init_file(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        return Err(TetherError::Config(format!("{} already exists; refusing to overwrite", path.display())));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| TetherError::Config(format!("cannot create {}: {e}", dir.display())))?;
    }
    std::fs::write(path, DEFAULT_FILE).map_err(|e| TetherError::Config(format!("cannot write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.keys.comment, "<leader>ac");
        assert_eq!(c.keys.comment_pick, "<leader>aC");
        assert_eq!(c.comment.max_code_lines, 20);
        assert_eq!(c.comment.template, DEFAULT_TEMPLATE);
        assert_eq!(c.nvim.command, "nvim");
        assert_eq!(c.placement.diagram, "tab");
    }

    #[test]
    fn shipped_default_file_equals_defaults() {
        let c = Config::parse(DEFAULT_FILE).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let c = Config::parse("[keys]\ncomment = \"<leader>cc\"\n[comment]\nmax_code_lines = 5\n").unwrap();
        assert_eq!(c.keys.comment, "<leader>cc");
        assert_eq!(c.keys.comment_pick, "<leader>aC");
        assert_eq!(c.comment.max_code_lines, 5);
        assert_eq!(c.comment.template, DEFAULT_TEMPLATE);
    }

    #[test]
    fn unknown_key_and_missing_text_placeholder_rejected() {
        assert!(Config::parse("[keys]\ncomnent = \"x\"\n").is_err());
        let e = Config::parse("[comment]\ntemplate = \"nothing here\"\n").unwrap_err();
        assert!(e.to_string().contains("{text}"));
        assert!(Config::parse("[comment]\nmax_code_lines = 0\n").is_err());
    }

    #[test]
    fn missing_file_is_defaults_and_init_refuses_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/config.toml");
        assert_eq!(Config::load_from(&p).unwrap(), Config::default());
        init_file(&p).unwrap();
        assert_eq!(Config::load_from(&p).unwrap(), Config::default());
        let e = init_file(&p).unwrap_err();
        assert!(e.to_string().contains("refusing to overwrite"));
    }

    #[test]
    fn broken_file_reports_path() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "[keys\n").unwrap();
        let e = Config::load_from(&p).unwrap_err();
        assert!(e.to_string().contains("config.toml"), "{e}");
    }

    #[test]
    fn dotted_get() {
        let c = Config::default();
        assert_eq!(c.get("nvim.command").unwrap(), serde_json::json!("nvim"));
        assert_eq!(c.get("keys.comment").unwrap(), serde_json::json!("<leader>ac"));
        assert!(c.get("nope.x").is_none());
    }
}
