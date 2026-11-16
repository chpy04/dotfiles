//! Glyph sets: Nerd Font icons (default) and a plain Unicode fallback (`TETHER_ICONS=0` or
//! `[view] icons = false`). Nerd Font codepoints are Font Awesome ones (U+F0xx..U+F2xx), which
//! every Nerd Font patched family carries.

use tether_core::diagram::{ParticipantKind, Shape};

/// A glyph set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icons {
    nerd: bool,
}

impl Icons {
    /// Nerd Font icons.
    pub const NERD: Icons = Icons { nerd: true };
    /// Plain Unicode glyphs.
    pub const PLAIN: Icons = Icons { nerd: false };

    /// `nerd = false` selects the fallback.
    pub fn new(nerd: bool) -> Icons {
        Icons { nerd }
    }

    /// Resolve from the config value and the `TETHER_ICONS` environment variable (`0`, `false`,
    /// `off` or `no` disable icons; any other non-empty value enables them).
    pub fn resolve(cfg_icons: bool, env: Option<&str>) -> Icons {
        match env.map(|v| v.trim().to_ascii_lowercase()) {
            Some(v) if matches!(v.as_str(), "0" | "false" | "off" | "no") => Icons::PLAIN,
            Some(v) if !v.is_empty() => Icons::NERD,
            _ => Icons::new(cfg_icons),
        }
    }

    /// Icon of a flowchart shape.
    pub fn shape(&self, s: Shape) -> &'static str {
        match (self.nerd, s) {
            (true, Shape::Process) => "\u{f013}",    // cog
            (true, Shape::Decision) => "\u{f059}",   // question-circle
            (true, Shape::Terminal) => "\u{f11e}",   // flag-checkered
            (true, Shape::Io) => "\u{f0ec}",         // exchange
            (true, Shape::Subroutine) => "\u{f1b2}", // cube
            (true, Shape::Database) => "\u{f1c0}",   // database
            (false, Shape::Process) => "▪",
            (false, Shape::Decision) => "◆",
            (false, Shape::Terminal) => "●",
            (false, Shape::Io) => "▰",
            (false, Shape::Subroutine) => "▣",
            (false, Shape::Database) => "≡",
        }
    }

    /// Icon of a participant kind.
    pub fn participant(&self, k: Option<ParticipantKind>) -> &'static str {
        match (self.nerd, k) {
            (true, Some(ParticipantKind::Actor)) => "\u{f007}",    // user
            (true, Some(ParticipantKind::Service) | None) => "\u{f233}", // server
            (true, Some(ParticipantKind::Database)) => "\u{f1c0}", // database
            (true, Some(ParticipantKind::Queue)) => "\u{f03a}",    // list
            (true, Some(ParticipantKind::External)) => "\u{f0ac}", // globe
            (false, Some(ParticipantKind::Actor)) => "☺",
            (false, Some(ParticipantKind::Service) | None) => "▣",
            (false, Some(ParticipantKind::Database)) => "≡",
            (false, Some(ParticipantKind::Queue)) => "≋",
            (false, Some(ParticipantKind::External)) => "↗",
        }
    }

    /// Marker for "this element has a code link".
    pub fn code(&self) -> &'static str {
        if self.nerd {
            "\u{f121}" // code
        } else {
            "§"
        }
    }

    /// Marker for a link that no longer resolves.
    pub fn stale(&self) -> &'static str {
        if self.nerd {
            "\u{f071}" // warning
        } else {
            "!"
        }
    }

    /// Prefix of the comment float title and the header.
    pub fn arrow(&self) -> &'static str {
        "→"
    }

    /// Header glyph per diagram type.
    pub fn diagram(&self, flowchart: bool) -> &'static str {
        match (self.nerd, flowchart) {
            (true, true) => "\u{f0e8}",  // sitemap
            (true, false) => "\u{f0ec}", // exchange
            (false, true) => "◈",
            (false, false) => "⇄",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_overrides_config() {
        assert_eq!(Icons::resolve(true, Some("0")), Icons::PLAIN);
        assert_eq!(Icons::resolve(true, Some("off")), Icons::PLAIN);
        assert_eq!(Icons::resolve(false, Some("1")), Icons::NERD);
        assert_eq!(Icons::resolve(false, None), Icons::PLAIN);
        assert_eq!(Icons::resolve(true, Some("")), Icons::NERD);
    }

    #[test]
    fn plain_icons_have_no_private_use_codepoints() {
        for s in [Shape::Process, Shape::Decision, Shape::Terminal, Shape::Io, Shape::Subroutine, Shape::Database] {
            assert!(!Icons::PLAIN.shape(s).chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)));
            assert!(Icons::NERD.shape(s).chars().all(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)));
        }
        assert_eq!(Icons::PLAIN.shape(Shape::Decision), "◆");
    }
}
