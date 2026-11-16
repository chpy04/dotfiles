//! Text measuring and wrapping in terminal cells.

/// Display width of one char in cells (0, 1 or 2). A small table, no external crate.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u == 0 || (0x300..=0x36F).contains(&u) || (0x200B..=0x200F).contains(&u) || u == 0xFE0F {
        return 0;
    }
    if u < 0x20 || (0x7F..0xA0).contains(&u) {
        return 0;
    }
    let wide = (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE6F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1F64F).contains(&u)
        || (0x1F900..=0x1F9FF).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

/// Display width of a string in cells.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Greedy word wrap to `max` cells. Explicit `\n` starts a new line; words longer than
/// `max` are split. Never returns an empty vec (at least one, possibly empty, line).
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut lw = 0usize;
        for word in para.split_whitespace() {
            let ww = str_width(word);
            if ww > max {
                // flush, then hard-split
                if lw > 0 {
                    out.push(std::mem::take(&mut line));
                    lw = 0;
                }
                for ch in word.chars() {
                    let cw = char_width(ch);
                    if lw + cw > max {
                        out.push(std::mem::take(&mut line));
                        lw = 0;
                    }
                    line.push(ch);
                    lw += cw;
                }
                continue;
            }
            let need = if lw == 0 { ww } else { lw + 1 + ww };
            if need > max {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
                lw = ww;
            } else {
                if lw > 0 {
                    line.push(' ');
                }
                line.push_str(word);
                lw = need;
            }
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Width of the widest line.
pub fn max_width(lines: &[String]) -> usize {
    lines.iter().map(|l| str_width(l)).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words() {
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("a", 7), vec!["a"]);
    }

    #[test]
    fn splits_long_words_and_keeps_paragraphs() {
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
    }

    #[test]
    fn widths() {
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("日本"), 4);
        assert_eq!(wrap("", 4), vec![""]);
    }
}
