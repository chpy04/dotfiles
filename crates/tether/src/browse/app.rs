//! The browser's state and key handling. Pure: keys go in, [`Effect`]s come out; the runner in
//! `mod.rs` does the I/O (store writes, opening the viewer, the clipboard) and feeds the new rows back.

use std::cell::Cell;
use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tether_core::diagram::DiagramInfo;
use tether_core::theme::Theme;

use crate::view::app::{FOOTER_ROWS, HEADER_ROWS};
use crate::view::icons::Icons;

/// Something the runner must do on the browser's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Leave the browser.
    Quit,
    /// Open in the full viewer in this pane; the browser resumes when the viewer quits.
    OpenHere { repo_key: String, id: String },
    /// `tether view <id> --placement tab`; the browser stays.
    OpenTab { repo_key: String, id: String },
    /// Star (`on`) or unstar.
    SetStar { repo_key: String, id: String, on: bool },
    /// Change the title.
    Rename { repo_key: String, id: String, title: String },
    /// Delete the diagram (already confirmed).
    Delete { repo_key: String, id: String },
    /// Put the Mermaid export on the clipboard.
    CopyMermaid { repo_key: String, id: String },
    /// Reload the rows (the scope changed).
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Ok,
    Error,
}

/// A one-line editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    pub text: String,
    /// Cursor position in chars.
    pub cursor: usize,
}

impl LineInput {
    pub fn new(text: &str) -> LineInput {
        LineInput { text: text.to_string(), cursor: text.chars().count() }
    }

    fn byte(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(self.text.len())
    }

    /// Apply an editing key; false when the key is not an editing key.
    pub fn handle(&mut self, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => {
                let b = self.byte(self.cursor);
                self.text.drain(..b);
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let upto = self.cursor;
                let chars: Vec<char> = self.text.chars().collect();
                let mut i = upto;
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                let (a, b) = (self.byte(i), self.byte(upto));
                self.text.drain(a..b);
                self.cursor = i;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.text.chars().count(),
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                let b = self.byte(self.cursor);
                self.text.insert(b, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let (a, b) = (self.byte(self.cursor - 1), self.byte(self.cursor));
                    self.text.drain(a..b);
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.text.chars().count() {
                    let (a, b) = (self.byte(self.cursor), self.byte(self.cursor + 1));
                    self.text.drain(a..b);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing a filter (`/`); Enter keeps it, Esc clears it.
    Filter(LineInput),
    /// Renaming the diagram `id`.
    Rename { repo_key: String, id: String, input: LineInput },
    /// Waiting for `y` to delete.
    ConfirmDelete { repo_key: String, id: String, title: String },
    Help,
}

/// What the runner should render for the selected diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewReq {
    pub key: String,
    pub repo_key: String,
    pub id: String,
}

pub struct Browser {
    /// Every diagram of the current scope (this repo, or all repos), newest activity first.
    pub rows: Vec<DiagramInfo>,
    pub theme: Theme,
    pub icons: Icons,
    /// Name of the repo the browser was started in.
    pub repo: String,
    pub all_repos: bool,
    pub starred_only: bool,
    pub filter: String,
    pub mode: Mode,
    /// Index into [`Browser::visible`].
    pub sel: usize,
    pub scroll_y: u16,
    pub scroll_x: u16,
    pub status: Option<(StatusKind, String)>,
    /// Unix seconds used for the age column (set by the runner each frame).
    pub now: u64,
    /// Retention shown in the empty state.
    pub keep_days: u32,
    /// Rendered previews (the viewer's own drawing of the whole canvas), by [`preview_key`].
    pub previews: HashMap<String, Result<Buffer, String>>,
    /// Size of the preview pane as last drawn (for clamping the scroll).
    pub view: Cell<(u16, u16)>,
}

pub fn preview_key(r: &DiagramInfo) -> String {
    format!("{}/{}/{}", r.repo_key, r.id, r.updated)
}

impl Browser {
    pub fn new(rows: Vec<DiagramInfo>, repo: &str, theme: Theme, icons: Icons, keep_days: u32) -> Browser {
        Browser {
            rows,
            theme,
            icons,
            repo: repo.to_string(),
            all_repos: false,
            starred_only: false,
            filter: String::new(),
            mode: Mode::Normal,
            sel: 0,
            scroll_y: 0,
            scroll_x: 0,
            status: None,
            now: 0,
            keep_days,
            previews: HashMap::new(),
            view: Cell::new((0, 0)),
        }
    }

    /// The rows that pass the `S` and `/` filters.
    pub fn visible(&self) -> Vec<&DiagramInfo> {
        let q = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|r| !self.starred_only || r.starred)
            .filter(|r| {
                q.is_empty()
                    || r.title.to_lowercase().contains(&q)
                    || r.branch.as_deref().is_some_and(|b| b.to_lowercase().contains(&q))
                    || (self.all_repos && r.repo.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&DiagramInfo> {
        self.visible().get(self.sel).copied()
    }

    fn selected_ref(&self) -> Option<(String, String)> {
        self.selected().map(|r| (r.repo_key.clone(), r.id.clone()))
    }

    /// Swap in freshly loaded rows, keeping the selection on `keep` (or where it was).
    pub fn set_rows(&mut self, rows: Vec<DiagramInfo>, keep: Option<(String, String)>) {
        let before = keep.or_else(|| self.selected_ref());
        self.rows = rows;
        let live: Vec<String> = self.rows.iter().map(preview_key).collect();
        self.previews.retain(|k, _| live.contains(k));
        self.reselect(before);
    }

    fn reselect(&mut self, keep: Option<(String, String)>) {
        let pos = keep.and_then(|(k, id)| self.visible().iter().position(|r| r.repo_key == k && r.id == id));
        let n = self.visible().len();
        let old = self.sel;
        self.sel = pos.unwrap_or_else(|| old.min(n.saturating_sub(1)));
        if pos.is_none() || self.sel != old {
            self.scroll_y = 0;
            self.scroll_x = 0;
        }
    }

    pub fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = Some((kind, text.into()));
    }

    /// The preview the runner still has to render for the selected row.
    pub fn need_preview(&self) -> Option<PreviewReq> {
        let r = self.selected()?;
        let key = preview_key(r);
        if self.previews.contains_key(&key) {
            return None;
        }
        Some(PreviewReq { key, repo_key: r.repo_key.clone(), id: r.id.clone() })
    }

    pub fn set_preview(&mut self, key: String, p: Result<Buffer, String>) {
        self.previews.insert(key, p);
    }

    fn move_sel(&mut self, delta: isize) {
        let n = self.visible().len();
        if n == 0 {
            return;
        }
        let new = (self.sel as isize + delta).clamp(0, n as isize - 1) as usize;
        if new != self.sel {
            self.sel = new;
            self.scroll_y = 0;
            self.scroll_x = 0;
        }
    }

    /// Size of the canvas of the selected preview (without the viewer's header and footer rows).
    pub fn canvas_size(&self) -> Option<(u16, u16)> {
        let r = self.selected()?;
        let b = self.previews.get(&preview_key(r))?.as_ref().ok()?;
        Some((b.area.width, b.area.height.saturating_sub(HEADER_ROWS + FOOTER_ROWS)))
    }

    fn scroll_preview(&mut self, dy: i32, dx: i32) {
        let (vw, vh) = self.view.get();
        let (cw, ch) = self.canvas_size().unwrap_or((0, 0));
        let (max_y, max_x) = (ch.saturating_sub(vh) as i32, cw.saturating_sub(vw) as i32);
        self.scroll_y = (self.scroll_y as i32 + dy).clamp(0, max_y) as u16;
        self.scroll_x = (self.scroll_x as i32 + dx).clamp(0, max_x) as u16;
    }

    /// Rows a half page of the preview moves.
    fn half_page(&self) -> i32 {
        i32::from((self.view.get().1 / 2).max(1))
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        if k.kind == KeyEventKind::Release {
            return vec![];
        }
        self.status = None;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return vec![Effect::Quit];
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.normal_key(k),
            Mode::Filter(mut input) => {
                let keep = self.selected_ref();
                match k.code {
                    KeyCode::Enter => {}
                    KeyCode::Esc => {
                        self.filter.clear();
                        self.reselect(keep);
                    }
                    KeyCode::Down => {
                        self.move_sel(1);
                        self.mode = Mode::Filter(input);
                    }
                    KeyCode::Up => {
                        self.move_sel(-1);
                        self.mode = Mode::Filter(input);
                    }
                    KeyCode::Char('n') if ctrl => {
                        self.move_sel(1);
                        self.mode = Mode::Filter(input);
                    }
                    KeyCode::Char('p') if ctrl => {
                        self.move_sel(-1);
                        self.mode = Mode::Filter(input);
                    }
                    _ => {
                        input.handle(&k);
                        self.filter = input.text.clone();
                        self.mode = Mode::Filter(input);
                        self.reselect(keep);
                    }
                }
                vec![]
            }
            Mode::Rename { repo_key, id, mut input } => match k.code {
                KeyCode::Esc => vec![],
                KeyCode::Enter => {
                    let title = input.text.trim().to_string();
                    let unchanged = self.rows.iter().any(|r| r.id == id && r.repo_key == repo_key && r.title == title);
                    if title.is_empty() {
                        self.set_status(StatusKind::Error, "a title cannot be empty");
                        self.mode = Mode::Rename { repo_key, id, input };
                        vec![]
                    } else if unchanged {
                        vec![]
                    } else {
                        vec![Effect::Rename { repo_key, id, title }]
                    }
                }
                _ => {
                    input.handle(&k);
                    self.mode = Mode::Rename { repo_key, id, input };
                    vec![]
                }
            },
            Mode::ConfirmDelete { repo_key, id, .. } => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => vec![Effect::Delete { repo_key, id }],
                _ => {
                    self.set_status(StatusKind::Info, "delete cancelled");
                    vec![]
                }
            },
            Mode::Help => vec![],
        }
    }

    fn normal_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let sel = self.selected().cloned();
        let pair = sel.as_ref().map(|r| (r.repo_key.clone(), r.id.clone()));
        match k.code {
            KeyCode::Char('q') => return vec![Effect::Quit],
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    let keep = self.selected_ref();
                    self.filter.clear();
                    self.reselect(keep);
                } else {
                    return vec![Effect::Quit];
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_sel(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.move_sel(isize::MAX / 2),
            KeyCode::Char('d') if ctrl => self.scroll_preview(self.half_page(), 0),
            KeyCode::Char('u') if ctrl => self.scroll_preview(-self.half_page(), 0),
            KeyCode::PageDown => self.scroll_preview(self.half_page(), 0),
            KeyCode::PageUp => self.scroll_preview(-self.half_page(), 0),
            KeyCode::Char('>') => self.scroll_preview(0, 8),
            KeyCode::Char('<') => self.scroll_preview(0, -8),
            KeyCode::Char('/') => self.mode = Mode::Filter(LineInput::new(&self.filter)),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('a') => {
                self.all_repos = !self.all_repos;
                self.set_status(StatusKind::Info, if self.all_repos { "all repos" } else { "this repo only" });
                return vec![Effect::Reload];
            }
            KeyCode::Char('S') => {
                let keep = self.selected_ref();
                self.starred_only = !self.starred_only;
                self.reselect(keep);
                self.set_status(StatusKind::Info, if self.starred_only { "starred only" } else { "all diagrams" });
            }
            KeyCode::Enter => {
                if let Some((repo_key, id)) = pair {
                    return vec![Effect::OpenHere { repo_key, id }];
                }
            }
            KeyCode::Char('t') => {
                if let Some((repo_key, id)) = pair {
                    return vec![Effect::OpenTab { repo_key, id }];
                }
            }
            KeyCode::Char('s') => {
                if let (Some(r), Some((repo_key, id))) = (&sel, pair) {
                    return vec![Effect::SetStar { repo_key, id, on: !r.starred }];
                }
            }
            KeyCode::Char('r') => {
                if let Some(r) = &sel {
                    self.mode = Mode::Rename { repo_key: r.repo_key.clone(), id: r.id.clone(), input: LineInput::new(&r.title) };
                }
            }
            KeyCode::Char('d') => {
                if let Some(r) = &sel {
                    self.mode = Mode::ConfirmDelete { repo_key: r.repo_key.clone(), id: r.id.clone(), title: r.title.clone() };
                }
            }
            KeyCode::Char('e') => {
                if let Some((repo_key, id)) = pair {
                    return vec![Effect::CopyMermaid { repo_key, id }];
                }
            }
            _ => {}
        }
        vec![]
    }
}
