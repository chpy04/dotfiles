//! The preview: the viewer's own drawing of the whole canvas, rendered once per diagram version.
//! The browser crops and scrolls the result; nothing here forks the viewer's drawing code.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use tether_core::config::Config;
use tether_core::diagram::Diagram;

use crate::view::{build_app, export};

/// Render `diagram` with the viewer (`export::render_full`): the buffer holds the viewer's header
/// row, the canvas and its footer rows (the browser skips those).
pub fn render(diagram: Diagram, id: &str, worktree: PathBuf, cfg: &Config) -> Result<Buffer, String> {
    let mut app = build_app(diagram, id, worktree, cfg, None, (100, 30)).map_err(|e| e.to_string())?;
    Ok(export::render_full(&mut app, 20, false))
}
