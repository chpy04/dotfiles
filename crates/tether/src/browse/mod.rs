//! `tether diagrams` (alias `tether diagram browse`): a full-screen browser for the saved
//! diagrams of this repo (or all repos): a list on the left, the viewer's own drawing of the
//! selection on the right.
//!
//! * [`app`]: state and key handling; keys in, [`app::Effect`]s out, no I/O.
//! * [`draw`]: renders the state into a ratatui buffer. [`preview`]: renders one diagram with
//!   the viewer's code (`view::export::render_full`).
//! * this file: the terminal loop and the effect runner (store writes, the viewer, the clipboard,
//!   `tether view --placement tab`).

pub mod app;
pub mod draw;
pub mod preview;

#[cfg(test)]
mod tests;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Args;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use tether_core::config::Config;
use tether_core::diagram::{data_dir, to_mermaid_opts, DiagramInfo, DiagramStore};
use tether_core::error::{Result, TetherError};
use tether_core::theme::Theme;

use crate::view::icons::Icons;
use app::{Browser, Effect, StatusKind};

#[derive(Args, Default)]
pub struct BrowseArgs {
    /// Start with the diagrams of every repo instead of this one
    #[arg(long)]
    pub all_repos: bool,
    /// Start with starred diagrams only
    #[arg(long)]
    pub starred: bool,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Load the rows of the current repo, or of every repo (newest activity first). Never prunes.
pub fn load_rows(store: &DiagramStore, all: bool) -> std::result::Result<Vec<DiagramInfo>, String> {
    let mut rows = store.rows().map_err(|e| e.to_string())?;
    if all {
        for other in DiagramStore::all_repos(&data_dir()) {
            if other.repo_key() != store.repo_key() {
                rows.extend(other.rows().map_err(|e| e.to_string())?);
            }
        }
        rows.sort_by(|a, b| b.activity().cmp(&a.activity()).then_with(|| a.id.cmp(&b.id)));
    }
    Ok(rows)
}

// ------------------------------------------------------------------------------------ clipboard

/// Put `text` on the system clipboard: `$TETHER_CLIPBOARD` (a shell command reading stdin) if set,
/// else the first of pbcopy, wl-copy, xclip, xsel that exists.
pub fn copy_to_clipboard(text: &str) -> std::result::Result<(), String> {
    let override_cmd = std::env::var("TETHER_CLIPBOARD").ok().filter(|s| !s.is_empty());
    let candidates: Vec<(String, Vec<String>)> = match override_cmd {
        Some(c) => vec![("sh".into(), vec!["-c".into(), c])],
        None => [("pbcopy", vec![]), ("wl-copy", vec![]), ("xclip", vec!["-selection", "clipboard"]), ("xsel", vec!["--clipboard", "--input"])]
            .into_iter()
            .map(|(c, a)| (c.to_string(), a.into_iter().map(String::from).collect()))
            .collect(),
    };
    for (cmd, args) in candidates {
        let Ok(mut child) = Command::new(&cmd).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            continue;
        };
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(text.as_bytes());
        }
        return match child.wait() {
            Ok(st) if st.success() => Ok(()),
            Ok(st) => Err(format!("{cmd} failed ({st})")),
            Err(e) => Err(format!("{cmd}: {e}")),
        };
    }
    Err("no clipboard tool found (pbcopy, wl-copy, xclip or xsel)".into())
}

// ------------------------------------------------------------------------------------ terminal

static ACTIVE: AtomicBool = AtomicBool::new(false);

fn restore_terminal() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = std::io::stdout();
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// The raw-mode alternate screen; restored when dropped, also while panicking.
struct Tui {
    term: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl Tui {
    fn enter() -> std::io::Result<Tui> {
        ACTIVE.store(true, Ordering::SeqCst);
        let guard_err = |e| {
            restore_terminal();
            e
        };
        enable_raw_mode().map_err(guard_err)?;
        execute!(std::io::stdout(), EnterAlternateScreen).map_err(guard_err)?;
        let term = Terminal::new(CrosstermBackend::new(std::io::stdout())).map_err(guard_err)?;
        Ok(Tui { term })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore_terminal();
    }
}

// ------------------------------------------------------------------------------------ runner

enum Msg {
    TabOpened(std::result::Result<String, String>),
}

struct Runner {
    store: DiagramStore,
    cfg: Config,
    exe: PathBuf,
    tx: mpsc::Sender<Msg>,
}

impl Runner {
    fn store_for(&self, repo_key: &str) -> DiagramStore {
        if repo_key == self.store.repo_key() {
            self.store.clone()
        } else {
            DiagramStore::for_key(&data_dir(), repo_key).with_keep_days(self.cfg.diagrams.keep_days)
        }
    }

    /// Render the preview the browser asked for.
    fn preview(&self, b: &mut Browser) {
        let Some(req) = b.need_preview() else { return };
        let store = self.store_for(&req.repo_key);
        let out = store
            .get(&req.id)
            .map_err(|e| e.to_string())
            .and_then(|d| preview::render(d, &req.id, store.worktree_root().to_path_buf(), &self.cfg));
        b.set_preview(req.key, out);
    }

    fn reload(&self, b: &mut Browser) {
        match load_rows(&self.store, b.all_repos) {
            Ok(rows) => b.set_rows(rows, None),
            Err(e) => b.set_status(StatusKind::Error, e),
        }
    }

    fn title_of(b: &Browser, repo_key: &str, id: &str) -> String {
        b.rows.iter().find(|r| r.id == id && r.repo_key == repo_key).map(|r| r.title.clone()).unwrap_or_else(|| id.to_string())
    }

    /// Perform one effect. Returns true to quit. `tui` is the live terminal, given up while the
    /// full viewer runs.
    fn perform(&self, e: Effect, b: &mut Browser, tui: &mut Option<Tui>) -> bool {
        match e {
            Effect::Quit => return true,
            Effect::Reload => self.reload(b),
            Effect::SetStar { repo_key, id, on } => {
                let title = Self::title_of(b, &repo_key, &id);
                match self.store_for(&repo_key).set_starred(&id, on) {
                    Ok(_) => {
                        self.reload(b);
                        b.set_status(StatusKind::Ok, format!("{} \"{title}\"", if on { "starred" } else { "unstarred" }));
                    }
                    Err(e) => b.set_status(StatusKind::Error, e.to_string()),
                }
            }
            Effect::Rename { repo_key, id, title } => match self.store_for(&repo_key).rename(&id, &title) {
                Ok(()) => {
                    self.reload(b);
                    b.set_status(StatusKind::Ok, format!("renamed to \"{title}\""));
                }
                Err(e) => b.set_status(StatusKind::Error, e.to_string()),
            },
            Effect::Delete { repo_key, id } => {
                let title = Self::title_of(b, &repo_key, &id);
                match self.store_for(&repo_key).delete(&id) {
                    Ok(()) => {
                        self.reload(b);
                        b.set_status(StatusKind::Ok, format!("deleted \"{title}\""));
                    }
                    Err(e) => b.set_status(StatusKind::Error, e.to_string()),
                }
            }
            Effect::CopyMermaid { repo_key, id } => {
                let title = Self::title_of(b, &repo_key, &id);
                let out = self
                    .store_for(&repo_key)
                    .get(&id)
                    .map_err(|e| e.to_string())
                    .and_then(|d| {
                        let text = to_mermaid_opts(&d, false);
                        copy_to_clipboard(&text).map(|()| text.lines().count())
                    });
                match out {
                    Ok(n) => b.set_status(StatusKind::Ok, format!("copied the Mermaid source of \"{title}\" ({n} lines) to the clipboard")),
                    Err(e) => b.set_status(StatusKind::Error, format!("copy failed: {e}")),
                }
            }
            Effect::OpenTab { repo_key, id } => {
                let store = self.store_for(&repo_key);
                let title = Self::title_of(b, &repo_key, &id);
                b.set_status(StatusKind::Info, format!("opening \"{title}\" in a new tab…"));
                let (exe, tx) = (self.exe.clone(), self.tx.clone());
                let cwd = if repo_key == self.store.repo_key() { std::env::current_dir().ok() } else { Some(store.worktree_root().to_path_buf()) };
                std::thread::spawn(move || {
                    let mut cmd = Command::new(exe);
                    cmd.args(["view", &id, "--placement", "tab"]).stdin(Stdio::null());
                    if let Some(c) = cwd.filter(|c| c.is_dir()) {
                        cmd.current_dir(c);
                    }
                    let res = match cmd.output() {
                        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                        Ok(o) => Err(first_error_line(&String::from_utf8_lossy(&o.stderr))),
                        Err(e) => Err(format!("cannot run tether: {e}")),
                    };
                    let _ = tx.send(Msg::TabOpened(res));
                });
            }
            Effect::OpenHere { repo_key, id } => {
                let store = self.store_for(&repo_key);
                let _ = store.touch_opened(&id);
                // hand the terminal to the viewer; it restores the normal screen when it quits
                *tui = None;
                let res = crate::view::run_in(&store, &id);
                match Tui::enter() {
                    Ok(mut t) => {
                        let _ = t.term.clear();
                        *tui = Some(t);
                    }
                    Err(_) => return true,
                }
                self.reload(b);
                if let Err(e) = res {
                    b.set_status(StatusKind::Error, e.to_string());
                }
            }
        }
        false
    }
}

fn first_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(|l| l.trim().trim_start_matches("tether: error: ").trim_start_matches("error: "))
        .find(|l| !l.is_empty())
        .unwrap_or("failed")
        .to_string()
}

// ------------------------------------------------------------------------------------ entry

/// Run the browser.
pub fn run(args: BrowseArgs) -> Result<()> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(TetherError::Input(
            "the diagram browser needs an interactive terminal; use `tether diagram list` to see the saved diagrams without one".into(),
        ));
    }
    let cfg = Config::load()?;
    let cwd = std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))?;
    let store = DiagramStore::open(&cwd).with_keep_days(cfg.diagrams.keep_days);
    // `list` imports an old `.tether/diagrams/` and prunes (once a day); it reports on stderr, so
    // do it before the screen is taken over
    store.list().map_err(|e| TetherError::Other(e.to_string()))?;
    let rows = load_rows(&store, args.all_repos).map_err(TetherError::Other)?;
    let theme = Theme::from_config(&cfg.theme).map_err(TetherError::Config)?;
    let icons = Icons::resolve(cfg.view.icons, std::env::var("TETHER_ICONS").ok().as_deref());
    let mut b = Browser::new(rows, store.repo_name(), theme, icons, cfg.diagrams.keep_days);
    b.all_repos = args.all_repos;
    b.starred_only = args.starred;
    b.set_rows(b.rows.clone(), None);
    let exe = std::env::current_exe().map_err(|e| TetherError::Other(format!("cannot locate the tether binary: {e}")))?;
    let (tx, rx) = mpsc::channel();
    let runner = Runner { store, cfg, exe, tx };
    event_loop(&runner, &mut b, &rx).map_err(|e| TetherError::Other(format!("terminal error: {e}")))
}

fn event_loop(runner: &Runner, b: &mut Browser, rx: &mpsc::Receiver<Msg>) -> std::io::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev_hook(info);
    }));
    let mut tui = Some(Tui::enter()?);
    let mut dirty = true;
    loop {
        let Some(t) = tui.as_mut() else { return Ok(()) };
        b.now = now_secs();
        runner.preview(b);
        if dirty {
            t.term.draw(|f| {
                let area = f.area();
                draw::draw(b, f.buffer_mut(), area);
            })?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    dirty = true;
                    for e in b.handle_key(k) {
                        if runner.perform(e, b, &mut tui) {
                            return Ok(());
                        }
                    }
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }
        while let Ok(Msg::TabOpened(res)) = rx.try_recv() {
            match res {
                Ok(line) => b.set_status(StatusKind::Ok, if line.is_empty() { "opened a tab".to_string() } else { line }),
                Err(e) => b.set_status(StatusKind::Error, e),
            }
            dirty = true;
        }
    }
}
