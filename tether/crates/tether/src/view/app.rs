//! The viewer's object model and update logic.
//!
//! [`App`] owns everything the screen is drawn from (diagram, layout, focus, expanded set,
//! viewport scroll, modes) and is driven by [`App::handle_key`] and friends, which return
//! [`Effect`]s for the runner to perform. Nothing in here spawns a process or touches the
//! terminal, so the whole flow is testable (`view/tests.rs`). The one side effect it does allow
//! itself is reading the linked source files to resolve anchors (F3.4).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use tether_core::config::Keys;
use tether_core::diagram::{self, Diagram, Element, Resolved};
use tether_core::layout::{self, Engine, Layout, LayoutInput, ObjectId, ObjectKind, Rect};
use tether_core::send;
use tether_core::theme::Theme;
use tether_core::workspace::AgentRow;

use super::icons::Icons;
use super::input::{Edit, Input};
use super::keys::{Action, Chord, Keymap, Match};

/// How long a half-typed leader sequence waits for its next key.
pub const LEADER_TIMEOUT: Duration = Duration::from_millis(1000);
/// How long an info/ok status stays; errors stay twice as long.
pub const STATUS_TTL: Duration = Duration::from_secs(6);
/// Links are re-resolved when their cached result is older than this.
pub const LINK_TTL: Duration = Duration::from_secs(5);
/// Blank cells kept around the canvas.
pub const PAD_X: u16 = 2;
pub const PAD_Y: u16 = 1;
/// Rows used by the header and by the status + hint lines.
pub const HEADER_ROWS: u16 = 1;
pub const FOOTER_ROWS: u16 = 2;
/// Longest jump list kept.
pub const JUMP_LIMIT: usize = 100;

// ------------------------------------------------------------------------------------ effects

/// Something the runner must do on the app's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// `tether code open <abs path>:<line> --from <pane>`.
    /// `focus`: also pass `--focus` so the Neovim pane gets focus.
    OpenCode { abs_path: PathBuf, rel_path: String, line: u32, from_pane: Option<String>, focus: bool },
    /// `tether send --to <to> --diagram <id> --node <node> --text-file -` with `text` on stdin.
    Send { to: String, diagram: String, node: String, text: String },
    /// `tether agents --json`.
    FetchAgents,
    /// `tether diagram star|unstar <diagram>`.
    Star { diagram: String, on: bool },
    /// Leave the viewer.
    Quit,
}

impl Effect {
    /// Arguments of the `tether` subprocess this effect runs (without the executable).
    pub fn argv(&self) -> Option<Vec<String>> {
        let s = |x: &str| x.to_string();
        match self {
            Effect::OpenCode { abs_path, line, from_pane, focus, .. } => {
                let mut v = vec![s("code"), s("open"), format!("{}:{}", abs_path.display(), line)];
                if let Some(p) = from_pane {
                    v.push(s("--from"));
                    v.push(p.clone());
                }
                if *focus {
                    v.push(s("--focus"));
                }
                Some(v)
            }
            Effect::Send { to, diagram, node, .. } => Some(vec![
                s("send"),
                s("--to"),
                to.clone(),
                s("--diagram"),
                diagram.clone(),
                s("--node"),
                node.clone(),
                s("--text-file"),
                s("-"),
            ]),
            Effect::FetchAgents => Some(vec![s("agents"), s("--json")]),
            Effect::Star { diagram, on } => Some(vec![s("diagram"), s(if *on { "star" } else { "unstar" }), diagram.clone()]),
            Effect::Quit => None,
        }
    }

    /// Text piped to the subprocess's stdin.
    pub fn stdin(&self) -> Option<&str> {
        match self {
            Effect::Send { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// Results the runner feeds back (from worker threads).
#[derive(Debug, Clone)]
pub enum Msg {
    /// `code open` finished: Ok(one-line summary) or Err(message).
    OpenDone(Result<String, String>),
    /// `send` finished.
    SendDone { result: Result<String, String>, node: ObjectId, to: String, text: String },
    /// `agents --json` finished.
    Agents(Result<Vec<AgentRow>, String>),
    /// `diagram star|unstar` finished; `on` is the state that was asked for.
    StarDone { result: Result<String, String>, on: bool },
}

// ------------------------------------------------------------------------------------ modes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub kind: StatusKind,
    pub at: Instant,
}

#[derive(Debug, Clone)]
pub struct SearchState {
    pub input: Input,
    /// Focus before the search started (Esc restores it).
    pub origin: ObjectId,
}

#[derive(Debug, Clone)]
pub struct CommentState {
    pub input: Input,
    /// `--to` value: `primary` or a pane id.
    pub to: String,
    /// Float title, e.g. `→ claude · tab 1`.
    pub title: String,
    /// Element being commented on.
    pub node: ObjectId,
}

#[derive(Debug, Clone)]
pub struct PickerState {
    /// `None` while loading.
    pub rows: Option<Vec<AgentRow>>,
    pub sel: usize,
    pub error: Option<String>,
    pub node: ObjectId,
}

#[derive(Debug, Clone)]
pub enum Mode {
    Normal,
    Search(SearchState),
    Comment(CommentState),
    Picker(PickerState),
    Help,
}

/// How strongly an object stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Focus,
    /// Directly connected to the focus.
    Near,
    Far,
}

// ------------------------------------------------------------------------------------ app

/// Everything needed to start the viewer.
#[derive(Debug, Clone)]
pub struct AppInit {
    pub diagram: Diagram,
    pub diagram_id: String,
    pub worktree: PathBuf,
    pub keys: Keys,
    pub theme: Theme,
    pub icons: Icons,
    pub pane_id: Option<String>,
    /// Enter focuses the Neovim pane (`[view] focus_code`).
    pub focus_code: bool,
    /// Terminal size `(cols, rows)`.
    pub term: (u16, u16),
    pub engine: Engine,
}

#[derive(Debug, Clone)]
pub struct App {
    pub diagram: Diagram,
    pub diagram_id: String,
    pub worktree: PathBuf,
    pub theme: Theme,
    pub icons: Icons,
    pub keymap: Keymap,
    pub pane_id: Option<String>,
    pub layout: Layout,
    pub expanded: HashSet<ObjectId>,
    pub focus: ObjectId,
    pub elements: HashMap<ObjectId, Element>,
    pub came_from: HashMap<ObjectId, ObjectId>,
    pub branch: HashMap<ObjectId, ObjectId>,
    /// Flowchart `J`/`K` column memory (like vim's curswant): the x the vertical run started at.
    pub col_ref: Option<i32>,
    /// Focus history for the jump list (`Ctrl-o` / `Ctrl-i`): visited objects, oldest first.
    pub history: Vec<ObjectId>,
    /// Index of the current focus in `history`.
    pub hpos: usize,
    pub focus_code: bool,
    /// The diagram is starred (set by the runner from the library; `s` toggles it).
    pub starred: bool,
    pub(super) seq_cursor: usize,
    pub term: (u16, u16),
    pub scroll_x: u16,
    pub scroll_y: u16,
    pub mode: Mode,
    pub(super) pending: Vec<Chord>,
    pub(super) pending_at: Option<Instant>,
    pub status: Option<Status>,
    pub search_query: String,
    pub matches: Vec<ObjectId>,
    pub links: HashMap<ObjectId, (Resolved, Instant)>,
    pub agents: Option<Vec<AgentRow>>,
    failed_draft: Option<(ObjectId, String)>,
    engine: Engine,
}

fn compute_layout(d: &Diagram, expanded: &HashSet<ObjectId>, engine: Engine) -> Result<Layout, String> {
    let mut input = LayoutInput::new(d, expanded);
    input.opts.engine = engine;
    layout::layout(&input).map_err(|e| e.to_string())
}

impl App {
    pub fn new(init: AppInit) -> Result<App, String> {
        let expanded = HashSet::new();
        let layout = compute_layout(&init.diagram, &expanded, init.engine)?;
        let elements: HashMap<ObjectId, Element> =
            diagram::elements(&init.diagram).into_iter().map(|e| (ObjectId(e.id.clone()), e)).collect();
        let keymap = Keymap::from_keys(&init.keys, matches!(layout.nav, layout::Nav::Flow(_)))?;
        let focus = match &layout.nav {
            layout::Nav::Flow(n) => n.roots.first().or(n.order.first()).cloned(),
            layout::Nav::Seq(n) => n.items.first().or(n.participants.first()).cloned(),
        }
        .ok_or_else(|| "the diagram has nothing to focus".to_string())?;
        let mut app = App {
            diagram: init.diagram,
            diagram_id: init.diagram_id,
            worktree: init.worktree,
            theme: init.theme,
            icons: init.icons,
            keymap,
            pane_id: init.pane_id,
            layout,
            expanded,
            focus,
            elements,
            came_from: HashMap::new(),
            branch: HashMap::new(),
            col_ref: None,
            history: Vec::new(),
            hpos: 0,
            focus_code: init.focus_code,
            starred: false,
            seq_cursor: 0,
            term: init.term,
            scroll_x: 0,
            scroll_y: 0,
            mode: Mode::Normal,
            pending: Vec::new(),
            pending_at: None,
            status: None,
            search_query: String::new(),
            matches: Vec::new(),
            links: HashMap::new(),
            agents: None,
            failed_draft: None,
            engine: init.engine,
        };
        app.history.push(app.focus.clone());
        app.sync_cursor();
        app.ensure_visible_focus();
        Ok(app)
    }

    // ---------------------------------------------------------------- queries

    pub fn is_flowchart(&self) -> bool {
        matches!(self.layout.nav, layout::Nav::Flow(_))
    }

    pub fn focused_element(&self) -> Option<&Element> {
        self.elements.get(&self.focus)
    }

    /// Cached resolution of the focused/any object's code link (filled by [`App::prepare_frame`]).
    pub fn link_state(&self, id: &ObjectId) -> Option<Resolved> {
        self.links.get(id).map(|(r, _)| *r)
    }

    /// Body rectangle (below the header, above the footer) as `(w, h)`.
    pub fn body_size(&self) -> (u16, u16) {
        (self.term.0, self.term.1.saturating_sub(HEADER_ROWS + FOOTER_ROWS))
    }

    /// Canvas origin in body coordinates (may be negative when scrolled).
    pub fn origin(&self) -> (i32, i32) {
        let (bw, bh) = self.body_size();
        let ox = if self.layout.width + 2 * PAD_X <= bw {
            (bw - self.layout.width) as i32 / 2
        } else {
            PAD_X as i32 - self.scroll_x as i32
        };
        let oy = if self.layout.height + 2 * PAD_Y <= bh { PAD_Y as i32 } else { PAD_Y as i32 - self.scroll_y as i32 };
        (ox, oy)
    }

    fn max_scroll(&self) -> (u16, u16) {
        let (bw, bh) = self.body_size();
        (
            (self.layout.width + 2 * PAD_X).saturating_sub(bw),
            (self.layout.height + 2 * PAD_Y).saturating_sub(bh),
        )
    }

    /// Prominence of `id` for dimming.
    pub fn tier(&self, id: &ObjectId) -> Tier {
        if *id == self.focus {
            return Tier::Focus;
        }
        match &self.layout.nav {
            layout::Nav::Flow(n) => {
                let near = n.children.get(&self.focus).is_some_and(|c| c.contains(id))
                    || n.parents.get(&self.focus).is_some_and(|c| c.contains(id));
                if near {
                    Tier::Near
                } else {
                    Tier::Far
                }
            }
            layout::Nav::Seq(_) => {
                if self.seq_involves(&self.focus, id) {
                    Tier::Near
                } else {
                    Tier::Far
                }
            }
        }
    }

    /// Sequence: is participant `p` an endpoint of item `item` (or the other way round)?
    fn seq_involves(&self, a: &ObjectId, b: &ObjectId) -> bool {
        let ends = |x: &ObjectId| -> Vec<ObjectId> {
            match self.layout.object(x.as_str()).map(|o| &o.kind) {
                Some(ObjectKind::Message { from, to, .. }) => vec![from.clone(), to.clone()],
                Some(ObjectKind::Note { over }) => over.clone(),
                _ => vec![],
            }
        };
        ends(a).contains(b) || ends(b).contains(a)
    }

    // ---------------------------------------------------------------- status

    pub fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), kind, at: Instant::now() });
    }

    fn expire(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if let Some(s) = &self.status {
            let ttl = if s.kind == StatusKind::Error { STATUS_TTL * 2 } else { STATUS_TTL };
            if now.saturating_duration_since(s.at) > ttl {
                self.status = None;
                changed = true;
            }
        }
        if let Some(at) = self.pending_at {
            if now.saturating_duration_since(at) > LEADER_TIMEOUT {
                self.pending.clear();
                self.pending_at = None;
                changed = true;
            }
        }
        changed
    }

    /// Periodic housekeeping; returns true when the screen needs a redraw.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.expire(now)
    }

    /// Pending leader keys, for the footer (`␣a`).
    pub fn pending_hint(&self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.iter().map(Chord::display).collect())
        }
    }

    // ---------------------------------------------------------------- layout & viewport

    fn relayout(&mut self) -> Result<(), String> {
        self.layout = compute_layout(&self.diagram, &self.expanded, self.engine)?;
        self.clamp_scroll();
        Ok(())
    }

    fn clamp_scroll(&mut self) {
        let (mx, my) = self.max_scroll();
        self.scroll_x = self.scroll_x.min(mx);
        self.scroll_y = self.scroll_y.min(my);
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.term = (w, h);
        self.clamp_scroll();
        self.ensure_visible_focus();
    }

    /// Scroll the viewport (positive = right/down) without moving the focus.
    pub fn scroll_by(&mut self, dx: i32, dy: i32) {
        let (mx, my) = self.max_scroll();
        self.scroll_x = (self.scroll_x as i32 + dx).clamp(0, mx as i32) as u16;
        self.scroll_y = (self.scroll_y as i32 + dy).clamp(0, my as i32) as u16;
    }

    /// Rect to keep in view for the focused object: the object plus its details panel.
    fn focus_rect(&self) -> Option<Rect> {
        let o = self.layout.object(self.focus.as_str())?;
        let mut r = o.rect;
        if let Some(p) = self.layout.details_of(self.focus.as_str()) {
            let x = r.x.min(p.rect.x);
            let y = r.y.min(p.rect.y);
            let right = r.right().max(p.rect.right());
            let bottom = r.bottom().max(p.rect.bottom());
            r = Rect { x, y, w: right - x, h: bottom - y };
        }
        Some(r)
    }

    pub fn ensure_visible_focus(&mut self) {
        if let Some(r) = self.focus_rect() {
            self.ensure_visible(r);
        }
    }

    fn ensure_visible(&mut self, r: Rect) {
        const MX: i32 = 4;
        const MY: i32 = 2;
        let (bw, bh) = self.body_size();
        let fit = |lo: i32, hi: i32, scroll: i32, size: i32, margin: i32| -> i32 {
            let mut s = scroll;
            if hi + margin > s + size {
                s = hi + margin - size;
            }
            if lo - margin < s {
                s = lo - margin;
            }
            s.max(0)
        };
        let nx = fit(r.x as i32 + PAD_X as i32, r.right() as i32 + PAD_X as i32, self.scroll_x as i32, bw as i32, MX);
        let ny = fit(r.y as i32 + PAD_Y as i32, r.bottom() as i32 + PAD_Y as i32, self.scroll_y as i32, bh as i32, MY);
        self.scroll_x = nx.min(u16::MAX as i32) as u16;
        self.scroll_y = ny.min(u16::MAX as i32) as u16;
        self.clamp_scroll();
    }

    // ---------------------------------------------------------------- focus

    fn sync_cursor(&mut self) {
        if let layout::Nav::Seq(n) = &self.layout.nav {
            if let Some(i) = n.items.iter().position(|x| *x == self.focus) {
                self.seq_cursor = i;
            }
        }
    }

    /// Focus `id` (must be a focusable object), scroll it into view and record it in the jump
    /// list. Also forgets the `j`/`k` column memory (the vertical movement re-sets it).
    pub fn set_focus(&mut self, id: ObjectId) {
        if self.focus_raw(id) {
            self.col_ref = None;
            self.note_focus();
        }
    }

    /// Focus without touching the jump list or column memory; false when `id` is not focusable.
    pub(super) fn focus_raw(&mut self, id: ObjectId) -> bool {
        if self.layout.object(id.as_str()).is_none() {
            return false;
        }
        self.focus = id;
        self.sync_cursor();
        self.ensure_visible_focus();
        true
    }

    /// Push the current focus onto the jump list (dropping any forward entries), unless it is
    /// already the current entry. The list is capped at [`JUMP_LIMIT`].
    pub(super) fn note_focus(&mut self) {
        if self.history.get(self.hpos) == Some(&self.focus) {
            return;
        }
        self.history.truncate(self.hpos + 1);
        self.history.push(self.focus.clone());
        if self.history.len() > JUMP_LIMIT {
            let extra = self.history.len() - JUMP_LIMIT;
            self.history.drain(..extra);
        }
        self.hpos = self.history.len() - 1;
    }

    /// `Ctrl-o` (`back`) / `Ctrl-i`: walk the jump list without recording the visit.
    pub(super) fn jump_list(&mut self, back: bool) {
        // entries can go stale after a relayout; skip anything that is no longer focusable
        loop {
            let next = if back { self.hpos.checked_sub(1) } else { Some(self.hpos + 1).filter(|&i| i < self.history.len()) };
            let Some(i) = next else {
                self.set_status(StatusKind::Info, if back { "oldest position in the jump list" } else { "newest position in the jump list" });
                return;
            };
            self.hpos = i;
            let id = self.history[i].clone();
            if id == self.focus {
                continue;
            }
            if self.focus_raw(id) {
                self.col_ref = None;
                return;
            }
        }
    }

    // ---------------------------------------------------------------- lazy link resolution

    /// Resolve (and cache) the code links of the objects currently on screen. Called before each
    /// frame; the first draw of an object resolves it, later draws reuse the cache.
    pub fn prepare_frame(&mut self, now: Instant) {
        let (bw, bh) = self.body_size();
        let (ox, oy) = self.origin();
        let (cx0, cy0) = (-ox, -oy);
        let (cx1, cy1) = (cx0 + bw as i32, cy0 + bh as i32);
        let mut todo: Vec<ObjectId> = Vec::new();
        for o in &self.layout.objects {
            let Some(el) = self.elements.get(&o.id) else { continue };
            if el.code.is_none() {
                continue;
            }
            let r = o.rect;
            let visible = (r.right() as i32) > cx0 && (r.x as i32) < cx1 && (r.bottom() as i32) > cy0 && (r.y as i32) < cy1;
            if !visible {
                continue;
            }
            let fresh = self.links.get(&o.id).is_some_and(|(_, at)| now.saturating_duration_since(*at) < LINK_TTL);
            if !fresh {
                todo.push(o.id.clone());
            }
        }
        for id in todo {
            if let Some(code) = self.elements.get(&id).and_then(|e| e.code.clone()) {
                let r = diagram::resolve(&code, &self.worktree);
                self.links.insert(id, (r, now));
            }
        }
    }

    /// Expand exactly the given objects (used by static rendering and tests).
    pub fn set_expanded(&mut self, ids: &[ObjectId]) -> Result<(), String> {
        let old = std::mem::replace(&mut self.expanded, ids.iter().cloned().collect());
        if let Err(e) = self.relayout() {
            self.expanded = old;
            return Err(e);
        }
        self.ensure_visible_focus();
        Ok(())
    }

    // ---------------------------------------------------------------- input: keys

    pub fn handle_key(&mut self, ev: KeyEvent) -> Vec<Effect> {
        self.handle_key_at(ev, Instant::now())
    }

    pub fn handle_key_at(&mut self, ev: KeyEvent, now: Instant) -> Vec<Effect> {
        if ev.kind == KeyEventKind::Release {
            return vec![];
        }
        self.expire(now);
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.key_normal(ev, now),
            Mode::Search(st) => self.key_search(st, ev),
            Mode::Comment(st) => self.key_comment(st, ev),
            Mode::Picker(st) => self.key_picker(st, ev),
            Mode::Help => {
                // any key closes the help; `?` toggles
                vec![]
            }
        }
    }

    fn key_normal(&mut self, ev: KeyEvent, now: Instant) -> Vec<Effect> {
        let Some(chord) = Chord::from_event(&ev) else { return vec![] };
        self.pending.push(chord);
        self.pending_at = Some(now);
        let mut m = self.keymap.lookup(&self.pending);
        if m == Match::None && self.pending.len() > 1 {
            // abandoned sequence: the new key starts afresh
            self.pending.clear();
            self.pending.push(chord);
            m = self.keymap.lookup(&self.pending);
        }
        match m {
            Match::Prefix => vec![],
            Match::None => {
                self.pending.clear();
                self.pending_at = None;
                vec![]
            }
            Match::Action(a) => {
                self.pending.clear();
                self.pending_at = None;
                self.run_action(a)
            }
        }
    }

    fn run_action(&mut self, a: Action) -> Vec<Effect> {
        match a {
            Action::Down
            | Action::Up
            | Action::Left
            | Action::Right
            | Action::StepForward
            | Action::StepBack
            | Action::BranchPrev
            | Action::BranchNext => return self.navigate(a),
            Action::JumpBack | Action::JumpForward => self.jump_list(a == Action::JumpBack),
            Action::First | Action::Last => return self.jump_end(a == Action::Last),
            Action::HalfDown => {
                let h = (self.body_size().1 / 2).max(1) as i32;
                self.scroll_by(0, h);
            }
            Action::HalfUp => {
                let h = (self.body_size().1 / 2).max(1) as i32;
                self.scroll_by(0, -h);
            }
            Action::Open => return self.open_code(),
            Action::Details => self.toggle_details(),
            Action::Search => {
                self.mode = Mode::Search(SearchState { input: Input::default(), origin: self.focus.clone() });
            }
            Action::SearchNext => self.next_match(1),
            Action::SearchPrev => self.next_match(-1),
            Action::Quit => return vec![Effect::Quit],
            Action::Help => self.mode = Mode::Help,
            Action::Star => {
                // optimistic: the header shows the new state at once, a failure puts it back
                self.starred = !self.starred;
                return vec![Effect::Star { diagram: self.diagram_id.clone(), on: self.starred }];
            }
            Action::Comment => self.start_comment("primary".into()),
            Action::CommentPick => return self.start_picker(),
            Action::Clear => {
                self.matches.clear();
                self.search_query.clear();
                self.status = None;
            }
        }
        vec![]
    }

    // ---------------------------------------------------------------- details

    fn toggle_details(&mut self) {
        let Some(el) = self.focused_element().cloned() else { return };
        if el.details == 0 {
            self.set_status(StatusKind::Info, format!("no details on this {}", el.kind.word()));
            return;
        }
        let id = self.focus.clone();
        let was = self.expanded.contains(&id);
        if was {
            self.expanded.remove(&id);
        } else {
            self.expanded.insert(id.clone());
        }
        if let Err(e) = self.relayout() {
            if was {
                self.expanded.insert(id);
            } else {
                self.expanded.remove(&id);
            }
            self.set_status(StatusKind::Error, format!("layout failed: {e}"));
            return;
        }
        self.ensure_visible_focus();
    }

    // ---------------------------------------------------------------- code (F3.2)

    fn open_code(&mut self) -> Vec<Effect> {
        let Some(el) = self.focused_element().cloned() else { return vec![] };
        let Some(code) = el.code.clone() else {
            self.set_status(StatusKind::Info, format!("no code link on this {}", el.kind.word()));
            return vec![];
        };
        // always re-resolve on Enter: the file may have changed since the cached result
        let r = diagram::resolve(&code, &self.worktree);
        self.links.insert(self.focus.clone(), (r, Instant::now()));
        let open = |line: u32, app: &App| Effect::OpenCode {
            abs_path: app.worktree.join(&code.path),
            rel_path: code.path.clone(),
            line,
            from_pane: app.pane_id.clone(),
            focus: app.focus_code,
        };
        match r {
            Resolved::Exact(l) => {
                self.set_status(StatusKind::Info, format!("opening {}:{l}", code.path));
                vec![open(l, self)]
            }
            Resolved::Moved(l) => {
                self.set_status(StatusKind::Warn, format!("moved from L{} → L{l} · opening {}:{l}", code.line, code.path));
                vec![open(l, self)]
            }
            Resolved::Stale => {
                let what = code.anchor.as_deref().map(|a| format!("`{a}`")).unwrap_or_else(|| "the line".into());
                self.set_status(
                    StatusKind::Error,
                    format!("stale link: {what} not found in {}:{} · not jumping", code.path, code.line),
                );
                vec![]
            }
            Resolved::MissingFile => {
                self.set_status(StatusKind::Error, format!("link broken: cannot read {}", code.path));
                vec![]
            }
        }
    }

    // ---------------------------------------------------------------- search

    fn search_order(&self) -> Vec<ObjectId> {
        match &self.layout.nav {
            layout::Nav::Flow(n) => n.order.clone(),
            layout::Nav::Seq(n) => n.participants.iter().chain(n.items.iter()).cloned().collect(),
        }
    }

    fn recompute_matches(&mut self) {
        let q = self.search_query.to_lowercase();
        self.matches = if q.is_empty() {
            vec![]
        } else {
            self.search_order()
                .into_iter()
                .filter(|id| self.elements.get(id).is_some_and(|e| e.label.to_lowercase().contains(&q)))
                .collect()
        };
    }

    fn key_search(&mut self, mut st: SearchState, ev: KeyEvent) -> Vec<Effect> {
        match st.input.key(&ev) {
            Edit::Cancel => {
                self.matches.clear();
                self.search_query.clear();
                self.focus_raw(st.origin);
                self.col_ref = None;
                return vec![];
            }
            Edit::Submit => {
                self.col_ref = None;
                self.note_focus();
                if self.search_query.is_empty() {
                    self.matches.clear();
                } else if self.matches.is_empty() {
                    self.set_status(StatusKind::Warn, format!("no match for \"{}\"", self.search_query));
                } else {
                    let i = self.matches.iter().position(|m| *m == self.focus).map(|i| i + 1).unwrap_or(1);
                    self.set_status(StatusKind::Info, format!("match {i}/{} · n/N next/previous", self.matches.len()));
                }
                return vec![];
            }
            Edit::Changed => {}
        }
        st.input.lines.truncate(1);
        st.input.row = 0;
        self.search_query = st.input.lines[0].clone();
        self.recompute_matches();
        // incremental: first match at or after where the search began
        if !self.matches.is_empty() {
            let order = self.search_order();
            let pos = |id: &ObjectId| order.iter().position(|o| o == id).unwrap_or(0);
            let start = pos(&st.origin);
            let target = self
                .matches
                .iter()
                .find(|m| pos(m) >= start)
                .or_else(|| self.matches.first())
                .cloned();
            if let Some(t) = target {
                self.focus_raw(t);
            }
        } else {
            self.focus_raw(st.origin.clone());
        }
        self.mode = Mode::Search(st);
        vec![]
    }

    fn next_match(&mut self, dir: i32) {
        if self.matches.is_empty() {
            self.set_status(StatusKind::Info, "no search matches · press / to search");
            return;
        }
        let order = self.search_order();
        let pos = |id: &ObjectId| order.iter().position(|o| o == id).unwrap_or(0) as i32;
        let here = pos(&self.focus);
        let target = if dir >= 0 {
            self.matches.iter().find(|m| pos(m) > here).or_else(|| self.matches.first())
        } else {
            self.matches.iter().rev().find(|m| pos(m) < here).or_else(|| self.matches.last())
        }
        .cloned();
        if let Some(t) = target {
            let i = self.matches.iter().position(|m| *m == t).map(|i| i + 1).unwrap_or(1);
            self.set_focus(t);
            self.set_status(StatusKind::Info, format!("match {i}/{}", self.matches.len()));
        }
    }

    // ---------------------------------------------------------------- comments (F3.3)

    fn agent_title(&self, to: &str) -> String {
        let arrow = self.icons.arrow();
        let row = self.agents.as_ref().and_then(|rows| {
            if to == "primary" {
                rows.iter().find(|r| r.is_primary)
            } else {
                rows.iter().find(|r| r.pane_id == to)
            }
        });
        match row {
            Some(r) => format!("{arrow} {} · tab {}", r.kind, r.tab_index),
            None if to == "primary" => format!("{arrow} claude"),
            None => format!("{arrow} {to}"),
        }
    }

    fn start_comment(&mut self, to: String) {
        let title = self.agent_title(&to);
        let node = self.focus.clone();
        let input = match &self.failed_draft {
            Some((n, text)) if *n == node => Input::from_text(text),
            _ => Input::default(),
        };
        self.mode = Mode::Comment(CommentState { input, to, title, node });
    }

    fn key_comment(&mut self, mut st: CommentState, ev: KeyEvent) -> Vec<Effect> {
        match st.input.key(&ev) {
            Edit::Cancel => vec![],
            Edit::Changed => {
                self.mode = Mode::Comment(st);
                vec![]
            }
            Edit::Submit => match send::clean_comment_text(&st.input.text()) {
                Err(e) => {
                    self.set_status(StatusKind::Error, e.to_string());
                    self.mode = Mode::Comment(st);
                    vec![]
                }
                Ok(text) => {
                    self.failed_draft = None;
                    let label = self.elements.get(&st.node).map(|e| e.label.clone()).unwrap_or_default();
                    self.set_status(StatusKind::Info, format!("sending comment on \"{label}\" to {}…", st.title.trim_start_matches("→ ")));
                    vec![Effect::Send { to: st.to, diagram: self.diagram_id.clone(), node: st.node.to_string(), text }]
                }
            },
        }
    }

    fn start_picker(&mut self) -> Vec<Effect> {
        self.mode = Mode::Picker(PickerState { rows: self.agents.clone(), sel: 0, error: None, node: self.focus.clone() });
        vec![Effect::FetchAgents]
    }

    fn key_picker(&mut self, mut st: PickerState, ev: KeyEvent) -> Vec<Effect> {
        use ratatui::crossterm::event::KeyCode;
        let Some(chord) = Chord::from_event(&ev) else {
            self.mode = Mode::Picker(st);
            return vec![];
        };
        let n = st.rows.as_ref().map_or(0, Vec::len);
        match (chord.code, chord.ctrl) {
            (KeyCode::Esc, _) | (KeyCode::Char('q'), false) | (KeyCode::Char('c'), true) => return vec![],
            (KeyCode::Char('j'), false) | (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                if n > 0 {
                    st.sel = (st.sel + 1).min(n - 1);
                }
            }
            (KeyCode::Char('k'), false) | (KeyCode::Up, _) | (KeyCode::Char('p'), true) => st.sel = st.sel.saturating_sub(1),
            (KeyCode::Enter, _) => {
                if let Some(row) = st.rows.as_ref().and_then(|r| r.get(st.sel)).cloned() {
                    let title = format!("{} {} · tab {}", self.icons.arrow(), row.kind, row.tab_index);
                    let node = st.node.clone();
                    let input = match &self.failed_draft {
                        Some((nid, text)) if *nid == node => Input::from_text(text),
                        _ => Input::default(),
                    };
                    self.mode = Mode::Comment(CommentState { input, to: row.pane_id, title, node });
                    return vec![];
                }
            }
            _ => {}
        }
        self.mode = Mode::Picker(st);
        vec![]
    }

    // ---------------------------------------------------------------- async results

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::OpenDone(Ok(s)) => self.set_status(StatusKind::Ok, s),
            Msg::OpenDone(Err(e)) => self.set_status(StatusKind::Error, format!("code open failed: {e}")),
            Msg::SendDone { result: Ok(s), .. } => self.set_status(StatusKind::Ok, format!("✓ {s}")),
            Msg::SendDone { result: Err(e), node, to, text } => {
                self.failed_draft = Some((node, text));
                self.set_status(StatusKind::Error, format!("send to {to} failed: {e} · reopen the comment to get your text back"));
            }
            Msg::Agents(Ok(rows)) => {
                self.agents = Some(rows.clone());
                if let Mode::Picker(p) = &mut self.mode {
                    p.sel = p.sel.min(rows.len().saturating_sub(1));
                    p.rows = Some(rows);
                    p.error = None;
                }
            }
            Msg::StarDone { result: Ok(_), on } => {
                self.set_status(StatusKind::Ok, if on { "★ starred: kept until you unstar it" } else { "unstarred" })
            }
            Msg::StarDone { result: Err(e), on } => {
                self.starred = !on;
                self.set_status(StatusKind::Error, format!("{} failed: {e}", if on { "star" } else { "unstar" }));
            }
            Msg::Agents(Err(e)) => {
                if let Mode::Picker(p) = &mut self.mode {
                    p.rows = Some(vec![]);
                    p.error = Some(e.clone());
                }
                self.set_status(StatusKind::Error, format!("cannot list agents: {e}"));
            }
        }
    }

    // ---------------------------------------------------------------- input: paste & mouse

    pub fn handle_paste(&mut self, s: &str) {
        match &mut self.mode {
            Mode::Comment(c) => c.input.insert_str(s),
            Mode::Search(st) => {
                st.input.insert_str(&s.replace('\n', " "));
                self.search_query = st.input.lines[0].clone();
                self.recompute_matches();
            }
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent) -> Vec<Effect> {
        if !matches!(self.mode, Mode::Normal) {
            return vec![];
        }
        match ev.kind {
            MouseEventKind::ScrollDown => self.scroll_by(0, 3),
            MouseEventKind::ScrollUp => self.scroll_by(0, -3),
            MouseEventKind::ScrollRight => self.scroll_by(6, 0),
            MouseEventKind::ScrollLeft => self.scroll_by(-6, 0),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(id) = self.hit_test(ev.column, ev.row) {
                    self.set_focus(id);
                }
            }
            _ => {}
        }
        vec![]
    }

    /// The focusable object under a screen cell.
    pub fn hit_test(&self, col: u16, row: u16) -> Option<ObjectId> {
        if row < HEADER_ROWS || row >= self.term.1.saturating_sub(FOOTER_ROWS) {
            return None;
        }
        let (ox, oy) = self.origin();
        let cx = col as i32 - ox;
        let cy = (row - HEADER_ROWS) as i32 - oy;
        if cx < 0 || cy < 0 {
            return None;
        }
        let p = layout::Point { x: cx.min(u16::MAX as i32) as u16, y: cy.min(u16::MAX as i32) as u16 };
        self.layout.objects.iter().rev().find_map(|o| {
            if !o.rect.contains(p) {
                return None;
            }
            match &o.kind {
                ObjectKind::Details => o.owner.clone(),
                ObjectKind::Node { .. } | ObjectKind::Participant { .. } | ObjectKind::Message { .. } | ObjectKind::Note { .. } => {
                    Some(o.id.clone())
                }
                _ => None,
            }
        })
    }
}
