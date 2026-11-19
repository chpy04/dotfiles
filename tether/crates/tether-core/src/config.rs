//! `~/.config/tether/config.toml` (override the path with `TETHER_CONFIG`).
//! A missing file means defaults. Unknown keys are an error (typos should not be silent).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TetherError};

/// Default payload template (F2.4). Placeholders: `{path} {line} {range} {worktree} {code} {text}`.
pub const DEFAULT_TEMPLATE: &str = "[comment] {path}:{range}  (worktree: {worktree})\n{code}\n{text}";

/// Default for `[comment] max_code_line_width`.
pub const DEFAULT_MAX_CODE_LINE_WIDTH: usize = 200;
/// Smallest accepted `[comment] max_code_line_width`.
pub const MIN_CODE_LINE_WIDTH: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keys: Keys,
    pub comment: CommentConfig,
    pub placement: Placement,
    pub nvim: NvimConfig,
    /// Diagram viewer behaviour (`[view]`).
    pub view: ViewConfig,
    /// Diagram viewer colors (`[theme]`).
    pub theme: crate::theme::ThemeConfig,
    /// Saved-diagram library (`[diagrams]`).
    pub diagrams: DiagramsConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiagramsConfig {
    /// Unstarred diagrams not opened or changed for this many days are pruned automatically
    /// (checked at most once a day, on `create` and `list`). 0 keeps everything.
    pub keep_days: u32,
}

impl Default for DiagramsConfig {
    fn default() -> Self {
        DiagramsConfig { keep_days: 30 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// Neovim, normal + visual mode: comment on the line/range to the primary agent.
    pub comment: String,
    /// Neovim: pick the target agent first.
    pub comment_pick: String,
    /// Neovim: browse and open the repo's saved diagrams.
    pub diagrams: String,
    // Viewer keys (Phase 3+).
    /// Sequence diagrams: next / previous message (`down`/`up`), previous / next participant
    /// (`left`/`right`). The arrow keys do the same in sequence diagrams.
    pub view_down: String,
    pub view_up: String,
    pub view_left: String,
    pub view_right: String,
    /// Flowcharts: step forward along the highlighted outgoing edge / back along the edge you came by.
    /// These four are named for left-to-right (`LR`) charts; a top-to-bottom (`TB`) chart swaps
    /// the pairs, so the branch keys step along the flow and the step keys switch branch.
    pub view_flow_forward: String,
    pub view_flow_back: String,
    /// Flowcharts: switch to the next / previous sibling branch (the other children of the parent).
    pub view_flow_branch_next: String,
    pub view_flow_branch_prev: String,
    /// Flowcharts: move on screen to the nearest node left / down / up / right (the arrow keys do too).
    pub view_flow_left: String,
    pub view_flow_down: String,
    pub view_flow_up: String,
    pub view_flow_right: String,
    pub view_open: String,
    pub view_details: String,
    pub view_search: String,
    pub view_search_next: String,
    pub view_quit: String,
    /// Viewer: star / unstar the diagram (starred diagrams are never pruned).
    pub view_star: String,
    /// Jump list: back / forward over the focus history.
    pub view_jump_back: String,
    pub view_jump_forward: String,
    /// What `<leader>` stands for in key sequences (viewer and Neovim). Default: space.
    pub leader: String,
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            comment: "<leader>ac".into(),
            comment_pick: "<leader>aC".into(),
            diagrams: "<leader>ad".into(),
            view_down: "j".into(),
            view_up: "k".into(),
            view_left: "h".into(),
            view_right: "l".into(),
            view_flow_forward: "l".into(),
            view_flow_back: "h".into(),
            view_flow_branch_next: "j".into(),
            view_flow_branch_prev: "k".into(),
            view_flow_left: "H".into(),
            view_flow_down: "J".into(),
            view_flow_up: "K".into(),
            view_flow_right: "L".into(),
            view_open: "<CR>".into(),
            view_details: "<Tab>".into(),
            view_search: "/".into(),
            view_search_next: "n".into(),
            view_quit: "q".into(),
            view_star: "s".into(),
            view_jump_back: "<C-o>".into(),
            view_jump_forward: "<C-i>".into(),
            leader: " ".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommentConfig {
    pub template: String,
    /// Longest code excerpt (in lines) put into a comment; longer ranges are truncated.
    pub max_code_lines: usize,
    /// Longest code line (characters) put into a comment; longer lines are cut and end with `…`.
    pub max_code_line_width: usize,
}

impl Default for CommentConfig {
    fn default() -> Self {
        CommentConfig { template: DEFAULT_TEMPLATE.into(), max_code_lines: 20, max_code_line_width: DEFAULT_MAX_CODE_LINE_WIDTH }
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
pub struct ViewConfig {
    /// Draw Nerd Font icons. `false` (or `TETHER_ICONS=0`) falls back to plain Unicode glyphs.
    pub icons: bool,
    /// Capture the mouse in the viewer (click to focus, wheel to scroll).
    pub mouse: bool,
    /// Enter (open code) also moves focus to the Neovim pane, whether it was reused or just split.
    pub focus_code: bool,
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig { icons: true, mouse: true, focus_code: true }
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
pub const DEFAULT_FILE: &str = r##"# tether configuration. Every key is optional; the values shown are the defaults.

[keys]
# Neovim: comment on the current line (normal mode) or the selected range (visual mode).
comment = "<leader>ac"
# Neovim: pick which agent receives the comment first.
comment_pick = "<leader>aC"
# Neovim: pick one of this repo's saved diagrams to open in a tab (:TetherDiagrams).
diagrams = "<leader>ad"
# Diagram viewer keys.
# Sequence diagrams: j/k next/previous message, h/l previous/next participant (the arrow keys too).
view_down = "j"
view_up = "k"
view_left = "h"
view_right = "l"
# Flowcharts, left to right (LR): h/l step back/forward along the arrows, j/k switch to the next/previous
# sibling branch. Top to bottom (TB) swaps the pairs: j/k step forward/back, l/h switch branch next/previous.
# H/J/K/L move on screen to the nearest node left/down/up/right (the arrow keys do too).
view_flow_back = "h"
view_flow_forward = "l"
view_flow_branch_next = "j"
view_flow_branch_prev = "k"
view_flow_left = "H"
view_flow_down = "J"
view_flow_up = "K"
view_flow_right = "L"
view_open = "<CR>"
view_details = "<Tab>"
view_search = "/"
view_search_next = "n"
view_quit = "q"
# Viewer: star / unstar the diagram (starred diagrams are never pruned).
view_star = "s"
# Jump list over the focus history. Terminals without the kitty keyboard protocol send <C-i> as <Tab>.
view_jump_back = "<C-o>"
view_jump_forward = "<C-i>"
# What <leader> means in key sequences such as "<leader>ac".
leader = " "

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
# Longest code line (characters) included in a comment; longer lines are cut with "…" (minimum 20).
max_code_line_width = 200

[placement]
# Where a diagram opens: "tab" | "split" | "here".
diagram = "tab"
# Where code opens relative to a visual: "right" | "down".
code_direction = "right"
# Fraction of the space the existing pane keeps when splitting.
code_ratio = 0.5

[view]
# Nerd Font icons in the diagram viewer (false = plain Unicode glyphs; env TETHER_ICONS=0 also does it).
icons = true
# Mouse support in the viewer (click to focus, wheel to scroll).
mouse = true
# Enter in the viewer also focuses the Neovim pane it opened or reused (false: leave focus on the viewer).
focus_code = true

# Viewer colors default to Catppuccin Mocha. Every key is optional ("#rrggbb"); uncomment to override.
[theme]
name = "catppuccin-mocha"
# accent = "#89b4fa"      # focus fill, incident edges, comment float
# accent_fg = "#11111b"   # text on the accent fill
# focus_border = "#b4befe"
# bg = "#1e1e2e"
# bar = "#181825"
# fg = "#cdd6f4"
# dim = "#a6adc8"
# edge = "#585b70"
# panel = "#313244"
# warn = "#fab387"
# error = "#f38ba8"
# ok = "#a6e3a1"
# link = "#74c7ec"
# search = "#f9e2af"
# [theme.fills]           # box backgrounds: process decision terminal io subroutine database
# decision = "#2b2840"    #   actor service queue external note alt opt loop par
# [theme.borders]         # box borders and icons, same keys
# decision = "#cba6f7"

[diagrams]
# Diagrams live in ~/.local/share/tether/repos/<repo>/ (override the root with $TETHER_DATA), shared
# by all worktrees of a clone. Unstarred diagrams neither opened nor changed for this many days are
# pruned automatically, at most once a day. 0 = never prune. Starred diagrams are never pruned.
keep_days = 30

[nvim]
# Command used to launch Neovim from `tether-nvim` and `tether code open`.
command = "nvim"
"##;

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
        if self.comment.max_code_line_width < MIN_CODE_LINE_WIDTH {
            return Err(TetherError::Config(format!(
                "invalid config: [comment] max_code_line_width must be at least {MIN_CODE_LINE_WIDTH}"
            )));
        }
        crate::theme::Theme::from_config(&self.theme).map_err(|e| TetherError::Config(format!("invalid config: {e}")))?;
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
        assert_eq!(c.keys.diagrams, "<leader>ad");
        assert_eq!(c.keys.view_star, "s");
        assert_eq!(c.diagrams.keep_days, 30);
    }

    #[test]
    fn diagrams_section() {
        let c = Config::parse("[diagrams]\nkeep_days = 0\n[keys]\ndiagrams = \"<leader>zz\"\n").unwrap();
        assert_eq!(c.diagrams.keep_days, 0);
        assert_eq!(c.keys.diagrams, "<leader>zz");
        assert_eq!(c.get("diagrams.keep_days").unwrap(), serde_json::json!(0));
        assert!(Config::parse("[diagrams]\nkeep = 3\n").is_err());
        assert!(Config::parse("[diagrams]\nkeep_days = -1\n").is_err());
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
    fn max_code_line_width_default_override_and_minimum() {
        assert_eq!(Config::default().comment.max_code_line_width, 200);
        assert_eq!(Config::parse(DEFAULT_FILE).unwrap().comment.max_code_line_width, 200);
        assert!(DEFAULT_FILE.contains("max_code_line_width = 200"));
        let c = Config::parse("[comment]\nmax_code_line_width = 80\n").unwrap();
        assert_eq!(c.comment.max_code_line_width, 80);
        assert_eq!(c.get("comment.max_code_line_width").unwrap(), serde_json::json!(80));
        assert!(Config::parse("[comment]\nmax_code_line_width = 20\n").is_ok());
        let e = Config::parse("[comment]\nmax_code_line_width = 19\n").unwrap_err();
        assert!(e.to_string().contains("max_code_line_width"), "{e}");
        assert!(Config::parse("[comment]\nmax_code_line_width = 0\n").is_err());
        assert!(Config::parse("[comment]\nmax_code_line_width = -5\n").is_err());
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

#[cfg(test)]
mod view_config_tests {
    use super::*;

    #[test]
    fn viewer_sections_parse_and_validate() {
        let c = Config::parse("[keys]\nleader = \",\"\n[view]\nicons = false\nmouse = false\n[theme]\naccent = \"#ff8800\"\n[theme.fills]\nio = \"#101010\"\n").unwrap();
        assert_eq!(c.keys.leader, ",");
        assert!(!c.view.icons && !c.view.mouse);
        assert_eq!(c.theme.accent.as_deref(), Some("#ff8800"));
        assert_eq!(c.get("theme.fills.io").unwrap(), serde_json::json!("#101010"));
        let d = Config::default();
        assert_eq!(d.keys.leader, " ");
        assert!(d.view.icons && d.view.mouse);
        assert_eq!(d.theme.name, "catppuccin-mocha");
    }

    #[test]
    fn bad_theme_colors_are_config_errors_naming_the_key() {
        let e = Config::parse("[theme]\naccent = \"blue\"\n").unwrap_err();
        assert!(e.to_string().contains("theme.accent"), "{e}");
        assert!(Config::parse("[theme.borders]\nnope = \"#000000\"\n").is_err());
        assert!(Config::parse("[view]\nicon = true\n").is_err(), "unknown keys stay an error");
    }
}
