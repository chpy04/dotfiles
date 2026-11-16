//! A small multi-line text editor for the comment float and the search prompt.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Result of feeding a key to the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Text or cursor changed (or nothing happened).
    Changed,
    /// Enter without newline modifiers: the caller should submit.
    Submit,
    /// Esc: the caller should cancel.
    Cancel,
}

/// Lines of text plus a cursor `(row, column in chars)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
}

impl Default for Input {
    fn default() -> Self {
        Input { lines: vec![String::new()], row: 0, col: 0 }
    }
}

fn byte_at(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(i, _)| i).unwrap_or(s.len())
}

impl Input {
    pub fn from_text(t: &str) -> Input {
        let mut lines: Vec<String> = t.split('\n').map(str::to_string).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let row = lines.len() - 1;
        let col = lines[row].chars().count();
        Input { lines, row, col }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    #[cfg(test)]
    pub fn is_blank(&self) -> bool {
        self.lines.iter().all(|l| l.trim().is_empty())
    }

    pub fn insert_char(&mut self, c: char) {
        let l = &mut self.lines[self.row];
        let b = byte_at(l, self.col);
        l.insert(b, c);
        self.col += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.replace("\r\n", "\n").replace('\r', "\n").chars() {
            if c == '\n' {
                self.newline();
            } else if !c.is_control() || c == '\t' {
                self.insert_char(c);
            }
        }
    }

    pub fn newline(&mut self) {
        let b = byte_at(&self.lines[self.row], self.col);
        let rest = self.lines[self.row].split_off(b);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            let l = &mut self.lines[self.row];
            let b = byte_at(l, self.col - 1);
            l.remove(b);
            self.col -= 1;
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
        }
    }

    pub fn delete(&mut self) {
        let len = self.lines[self.row].chars().count();
        if self.col < len {
            let l = &mut self.lines[self.row];
            let b = byte_at(l, self.col);
            l.remove(b);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn delete_word_back(&mut self) {
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut i = self.col;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        if i == self.col {
            return self.backspace();
        }
        let s: String = chars[..i].iter().chain(chars[self.col..].iter()).collect();
        self.lines[self.row] = s;
        self.col = i;
    }

    fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
        }
    }

    fn right(&mut self) {
        if self.col < self.lines[self.row].chars().count() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn vertical(&mut self, d: i32) {
        let r = self.row as i32 + d;
        if r >= 0 && (r as usize) < self.lines.len() {
            self.row = r as usize;
            self.col = self.col.min(self.lines[self.row].chars().count());
        }
    }

    /// Apply a key. Enter submits; Shift-Enter, Alt-Enter and Ctrl-J insert a newline.
    pub fn key(&mut self, k: &KeyEvent) -> Edit {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc => return Edit::Cancel,
            KeyCode::Enter if shift || alt || ctrl => self.newline(),
            KeyCode::Enter => return Edit::Submit,
            KeyCode::Char('j') if ctrl => self.newline(),
            KeyCode::Char('m') if ctrl => return Edit::Submit,
            KeyCode::Char('c') if ctrl => return Edit::Cancel,
            KeyCode::Char('u') if ctrl => {
                let b = byte_at(&self.lines[self.row], self.col);
                self.lines[self.row] = self.lines[self.row][b..].to_string();
                self.col = 0;
            }
            KeyCode::Char('w') if ctrl => self.delete_word_back(),
            KeyCode::Char('a') if ctrl => self.col = 0,
            KeyCode::Char('e') if ctrl => self.col = self.lines[self.row].chars().count(),
            KeyCode::Char('b') if ctrl => self.left(),
            KeyCode::Char('f') if ctrl => self.right(),
            KeyCode::Char('h') if ctrl => self.backspace(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert_char(c),
            KeyCode::Tab => self.insert_str("  "),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Up => self.vertical(-1),
            KeyCode::Down => self.vertical(1),
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.lines[self.row].chars().count(),
            _ => {}
        }
        Edit::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn type_str(i: &mut Input, s: &str) {
        for c in s.chars() {
            i.key(&k(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn typing_newlines_and_submit() {
        let mut i = Input::default();
        type_str(&mut i, "héllo");
        assert_eq!(i.key(&k(KeyCode::Enter, KeyModifiers::SHIFT)), Edit::Changed);
        type_str(&mut i, "wörld");
        assert_eq!(i.key(&k(KeyCode::Char('j'), KeyModifiers::CONTROL)), Edit::Changed);
        type_str(&mut i, "x");
        assert_eq!(i.text(), "héllo\nwörld\nx");
        assert_eq!(i.key(&k(KeyCode::Enter, KeyModifiers::NONE)), Edit::Submit);
        assert_eq!(i.key(&k(KeyCode::Esc, KeyModifiers::NONE)), Edit::Cancel);
    }

    #[test]
    fn editing_in_the_middle() {
        let mut i = Input::from_text("abc\ndef");
        assert_eq!((i.row, i.col), (1, 3));
        i.key(&k(KeyCode::Up, KeyModifiers::NONE));
        i.key(&k(KeyCode::Home, KeyModifiers::NONE));
        i.key(&k(KeyCode::Right, KeyModifiers::NONE));
        type_str(&mut i, "Z");
        assert_eq!(i.text(), "aZbc\ndef");
        i.key(&k(KeyCode::End, KeyModifiers::NONE));
        i.key(&k(KeyCode::Right, KeyModifiers::NONE)); // wraps to next line start
        i.key(&k(KeyCode::Backspace, KeyModifiers::NONE)); // joins lines
        assert_eq!(i.text(), "aZbcdef");
        i.key(&k(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.text(), "def");
        i.key(&k(KeyCode::End, KeyModifiers::NONE));
        i.key(&k(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.text(), "");
        assert!(i.is_blank());
    }

    #[test]
    fn paste_and_word_delete() {
        let mut i = Input::default();
        i.insert_str("one two\r\nthree\x1b");
        assert_eq!(i.text(), "one two\nthree");
        i.key(&k(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.text(), "one two\n");
        i.key(&k(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(i.lines.len(), 2);
    }
}
