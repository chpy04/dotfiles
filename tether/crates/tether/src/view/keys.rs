//! Key notation (`<leader>ac`, `<CR>`, `<C-d>`), chords and the viewer keymap.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tether_core::config::Keys;
use tether_core::diagram::Direction;

/// One key press, normalized: shift is folded into the character, alt is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub code: KeyCode,
    pub ctrl: bool,
}

impl Chord {
    pub fn ch(c: char) -> Chord {
        Chord { code: KeyCode::Char(c), ctrl: false }
    }

    /// `None` for key releases and keys we do not bind.
    pub fn from_event(e: &KeyEvent) -> Option<Chord> {
        if e.kind == KeyEventKind::Release {
            return None;
        }
        let ctrl = e.modifiers.contains(KeyModifiers::CONTROL);
        let code = match e.code {
            KeyCode::Char(c) if ctrl => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        Some(Chord { code, ctrl })
    }

    /// How the chord reads in a hint (`␣`, `⏎`, `^d`).
    pub fn display(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "␣".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "⏎".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Backspace => "⌫".into(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.ctrl {
            format!("^{base}")
        } else {
            base
        }
    }
}

fn named(tok: &str) -> Option<Chord> {
    let lower = tok.to_ascii_lowercase();
    let plain = |code| Some(Chord { code, ctrl: false });
    match lower.as_str() {
        "cr" | "enter" | "return" => plain(KeyCode::Enter),
        "tab" => plain(KeyCode::Tab),
        "esc" => plain(KeyCode::Esc),
        "space" => plain(KeyCode::Char(' ')),
        "bs" | "backspace" => plain(KeyCode::Backspace),
        "up" => plain(KeyCode::Up),
        "down" => plain(KeyCode::Down),
        "left" => plain(KeyCode::Left),
        "right" => plain(KeyCode::Right),
        "lt" => plain(KeyCode::Char('<')),
        _ => {
            let rest = lower.strip_prefix("c-")?;
            let mut it = rest.chars();
            let c = it.next()?;
            it.next().is_none().then_some(Chord { code: KeyCode::Char(c), ctrl: true })
        }
    }
}

/// Parse a key sequence like `<leader>ac` or `<C-d>`. `leader` is itself key notation (`" "`,
/// `","`, `"<Space>"`).
pub fn parse_keyseq(s: &str, leader: &str) -> Result<Vec<Chord>, String> {
    fn go(s: &str, leader: Option<&str>, depth: u8) -> Result<Vec<Chord>, String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if c == '<' {
                if let Some(end) = rest.find('>') {
                    let tok = &rest[1..end];
                    if tok.eq_ignore_ascii_case("leader") {
                        let l = leader.ok_or_else(|| "<leader> inside the leader key".to_string())?;
                        if depth > 0 {
                            return Err("<leader> inside the leader key".into());
                        }
                        out.extend(go(l, None, depth + 1)?);
                        rest = &rest[end + 1..];
                        continue;
                    }
                    if let Some(ch) = named(tok) {
                        out.push(ch);
                        rest = &rest[end + 1..];
                        continue;
                    }
                    return Err(format!("unknown key `<{tok}>` in `{s}`"));
                }
            }
            out.push(Chord::ch(c));
            rest = &rest[c.len_utf8()..];
        }
        Ok(out)
    }
    let v = go(s, Some(leader), 0)?;
    if v.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(v)
}

/// Things a key can do in normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    Left,
    Right,
    /// Flowchart: step forward along the highlighted outgoing edge.
    StepForward,
    /// Flowchart: step back along the edge we came by (else the first parent).
    StepBack,
    /// Flowchart: switch to the previous / next sibling branch.
    BranchPrev,
    BranchNext,
    /// Jump list back / forward.
    JumpBack,
    JumpForward,
    First,
    Last,
    HalfDown,
    HalfUp,
    Open,
    Details,
    Search,
    SearchNext,
    SearchPrev,
    Quit,
    Help,
    /// Star / unstar the diagram.
    Star,
    Comment,
    CommentPick,
    Clear,
}

/// Result of looking a pending sequence up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    Action(Action),
    /// A longer binding starts with this sequence.
    Prefix,
    None,
}

/// Normal-mode keymap built from `[keys]` plus the fixed extras (arrows, `g`/`G`, `N`, `^d`/`^u`).
#[derive(Debug, Clone)]
pub struct Keymap {
    binds: Vec<(Vec<Chord>, Action)>,
}

impl Keymap {
    /// The keymap of a flowchart (`flow` is its direction) or of a sequence diagram (`None`).
    /// They share everything but the movement keys: a flowchart steps along arrows
    /// (`view_flow_*`), a sequence diagram moves between messages and participants (`view_down`
    /// & co). All keys are validated either way.
    ///
    /// The `view_flow_*` step/branch keys are named for a left-to-right (`LR`) chart, where the
    /// arrows run along `h`/`l` and sibling branches stack along `j`/`k`. A top-to-bottom (`TB`)
    /// chart transposes them: the `view_flow_branch_next`/`_prev` keys (`j`/`k`) step forward /
    /// back along the arrows and the `view_flow_forward`/`_back` keys (`l`/`h`) switch to the
    /// next / previous branch. So the keys always follow the screen: along the flow steps, across
    /// it switches branch.
    pub fn from_keys(k: &Keys, flow: Option<Direction>) -> Result<Keymap, String> {
        let mut binds: Vec<(Vec<Chord>, Action)> = Vec::new();
        let tb = flow == Some(Direction::TB);
        let (fwd, back, next, prev) = if tb {
            (Action::BranchNext, Action::BranchPrev, Action::StepForward, Action::StepBack)
        } else {
            (Action::StepForward, Action::StepBack, Action::BranchNext, Action::BranchPrev)
        };
        let movement = [
            (&k.view_down, Action::Down, "view_down", false),
            (&k.view_up, Action::Up, "view_up", false),
            (&k.view_left, Action::Left, "view_left", false),
            (&k.view_right, Action::Right, "view_right", false),
            (&k.view_flow_forward, fwd, "view_flow_forward", true),
            (&k.view_flow_back, back, "view_flow_back", true),
            (&k.view_flow_branch_next, next, "view_flow_branch_next", true),
            (&k.view_flow_branch_prev, prev, "view_flow_branch_prev", true),
            (&k.view_flow_left, Action::Left, "view_flow_left", true),
            (&k.view_flow_down, Action::Down, "view_flow_down", true),
            (&k.view_flow_up, Action::Up, "view_flow_up", true),
            (&k.view_flow_right, Action::Right, "view_flow_right", true),
        ];
        for (s, a, name, is_flow) in movement {
            let seq = parse_keyseq(s, &k.leader).map_err(|e| format!("keys.{name}: {e}"))?;
            if is_flow == flow.is_some() {
                binds.push((seq, a));
            }
        }
        let configured = [
            (&k.view_jump_back, Action::JumpBack, "view_jump_back"),
            (&k.view_jump_forward, Action::JumpForward, "view_jump_forward"),
            (&k.view_open, Action::Open, "view_open"),
            (&k.view_details, Action::Details, "view_details"),
            (&k.view_search, Action::Search, "view_search"),
            (&k.view_search_next, Action::SearchNext, "view_search_next"),
            (&k.view_quit, Action::Quit, "view_quit"),
            (&k.view_star, Action::Star, "view_star"),
            (&k.comment, Action::Comment, "comment"),
            (&k.comment_pick, Action::CommentPick, "comment_pick"),
        ];
        for (s, a, name) in configured {
            let seq = parse_keyseq(s, &k.leader).map_err(|e| format!("keys.{name}: {e}"))?;
            binds.push((seq, a));
        }
        let plain = |code| vec![Chord { code, ctrl: false }];
        let ctrl = |c: char| vec![Chord { code: KeyCode::Char(c), ctrl: true }];
        // fixed extras (configured bindings win: they come first in the list)
        binds.push((plain(KeyCode::Down), Action::Down));
        binds.push((plain(KeyCode::Up), Action::Up));
        binds.push((plain(KeyCode::Left), Action::Left));
        binds.push((plain(KeyCode::Right), Action::Right));
        binds.push((plain(KeyCode::Enter), Action::Open));
        binds.push((vec![Chord::ch('g')], Action::First));
        binds.push((vec![Chord::ch('G')], Action::Last));
        binds.push((plain(KeyCode::Home), Action::First));
        binds.push((plain(KeyCode::End), Action::Last));
        binds.push((vec![Chord::ch('N')], Action::SearchPrev));
        binds.push((ctrl('d'), Action::HalfDown));
        binds.push((ctrl('u'), Action::HalfUp));
        binds.push((plain(KeyCode::PageDown), Action::HalfDown));
        binds.push((plain(KeyCode::PageUp), Action::HalfUp));
        binds.push((vec![Chord::ch('?')], Action::Help));
        binds.push((plain(KeyCode::Esc), Action::Clear));
        binds.push((ctrl('c'), Action::Quit));
        Ok(Keymap { binds })
    }

    pub fn lookup(&self, seq: &[Chord]) -> Match {
        if let Some((_, a)) = self.binds.iter().find(|(s, _)| s.as_slice() == seq) {
            return Match::Action(*a);
        }
        if self.binds.iter().any(|(s, _)| s.len() > seq.len() && s[..seq.len()] == *seq) {
            return Match::Prefix;
        }
        Match::None
    }

    /// The key(s) bound to `a` as they read in a hint (the first configured binding).
    pub fn hint(&self, a: Action) -> String {
        self.binds
            .iter()
            .find(|(_, x)| *x == a)
            .map(|(s, _)| s.iter().map(Chord::display).collect::<String>())
            .unwrap_or_default()
    }

    /// Continuations of a pending prefix, for the which-key line: `(rest of the keys, action)`.
    pub fn continuations(&self, prefix: &[Chord]) -> Vec<(String, Action)> {
        self.binds
            .iter()
            .filter(|(s, _)| s.len() > prefix.len() && s[..prefix.len()] == *prefix)
            .map(|(s, a)| (s[prefix.len()..].iter().map(Chord::display).collect::<String>(), *a))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_leader_sequences_and_named_keys() {
        let v = parse_keyseq("<leader>ac", " ").unwrap();
        assert_eq!(v, vec![Chord::ch(' '), Chord::ch('a'), Chord::ch('c')]);
        assert_eq!(parse_keyseq("<leader>aC", ",").unwrap()[0], Chord::ch(','));
        assert_eq!(parse_keyseq("<leader>x", "<Space>").unwrap()[0], Chord::ch(' '));
        assert_eq!(parse_keyseq("<CR>", " ").unwrap(), vec![Chord { code: KeyCode::Enter, ctrl: false }]);
        assert_eq!(parse_keyseq("<C-d>", " ").unwrap(), vec![Chord { code: KeyCode::Char('d'), ctrl: true }]);
        assert!(parse_keyseq("<bogus>", " ").is_err());
        assert!(parse_keyseq("", " ").is_err());
    }

    #[test]
    fn default_keymap_matches_and_prefixes() {
        let km = Keymap::from_keys(&Keys::default(), Some(Direction::LR)).unwrap();
        let seq = Keymap::from_keys(&Keys::default(), None).unwrap();
        assert_eq!(seq.lookup(&[Chord::ch('j')]), Match::Action(Action::Down));
        assert_eq!(seq.lookup(&[Chord::ch('l')]), Match::Action(Action::Right));
        assert_eq!(seq.lookup(&[Chord::ch('J')]), Match::None, "flowchart keys are not bound in sequence diagrams");
        assert_eq!(km.lookup(&[Chord::ch('j')]), Match::Action(Action::BranchNext));
        assert_eq!(km.lookup(&[Chord::ch(' ')]), Match::Prefix);
        assert_eq!(km.lookup(&[Chord::ch(' '), Chord::ch('a')]), Match::Prefix);
        assert_eq!(km.lookup(&[Chord::ch(' '), Chord::ch('a'), Chord::ch('c')]), Match::Action(Action::Comment));
        assert_eq!(km.lookup(&[Chord::ch(' '), Chord::ch('a'), Chord::ch('C')]), Match::Action(Action::CommentPick));
        assert_eq!(km.lookup(&[Chord::ch(' '), Chord::ch('z')]), Match::None);
        assert_eq!(km.lookup(&[Chord::ch('G')]), Match::Action(Action::Last));
        assert_eq!(km.lookup(&[Chord::ch('l')]), Match::Action(Action::StepForward));
        assert_eq!(km.lookup(&[Chord::ch('h')]), Match::Action(Action::StepBack));
        assert_eq!(km.lookup(&[Chord::ch('k')]), Match::Action(Action::BranchPrev));
        assert_eq!(km.lookup(&[Chord::ch('H')]), Match::Action(Action::Left));
        assert_eq!(km.lookup(&[Chord::ch('J')]), Match::Action(Action::Down));
        assert_eq!(km.lookup(&[Chord::ch('K')]), Match::Action(Action::Up));
        assert_eq!(km.lookup(&[Chord::ch('L')]), Match::Action(Action::Right));
        let ctrl = |c| Chord { code: KeyCode::Char(c), ctrl: true };
        assert_eq!(km.lookup(&[ctrl('o')]), Match::Action(Action::JumpBack));
        assert_eq!(km.lookup(&[ctrl('i')]), Match::Action(Action::JumpForward));
        assert_eq!(km.hint(Action::Comment), "␣ac");
        let c = km.continuations(&[Chord::ch(' '), Chord::ch('a')]);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn top_to_bottom_flowcharts_transpose_step_and_branch_keys() {
        let tb = Keymap::from_keys(&Keys::default(), Some(Direction::TB)).unwrap();
        assert_eq!(tb.lookup(&[Chord::ch('j')]), Match::Action(Action::StepForward));
        assert_eq!(tb.lookup(&[Chord::ch('k')]), Match::Action(Action::StepBack));
        assert_eq!(tb.lookup(&[Chord::ch('l')]), Match::Action(Action::BranchNext));
        assert_eq!(tb.lookup(&[Chord::ch('h')]), Match::Action(Action::BranchPrev));
        // on-screen moves do not depend on the direction
        assert_eq!(tb.lookup(&[Chord::ch('J')]), Match::Action(Action::Down));
        assert_eq!(tb.lookup(&[Chord::ch('L')]), Match::Action(Action::Right));
        assert_eq!(tb.hint(Action::StepForward), "j");
        assert_eq!(tb.hint(Action::BranchPrev), "h");
    }

    #[test]
    fn custom_leader_and_bad_config() {
        let k = Keys { leader: ",".into(), ..Keys::default() };
        let km = Keymap::from_keys(&k, Some(Direction::TB)).unwrap();
        assert_eq!(km.lookup(&[Chord::ch(','), Chord::ch('a'), Chord::ch('c')]), Match::Action(Action::Comment));
        let k = Keys { view_down: "<nope>".into(), ..Keys::default() };
        assert!(Keymap::from_keys(&k, Some(Direction::TB)).unwrap_err().contains("keys.view_down"));
    }
}
