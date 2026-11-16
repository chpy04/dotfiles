//! Colors for the diagram viewer (PRD F3.5: "colors that follow my theme").
//!
//! The default palette is **Catppuccin Mocha** (the user's Ghostty and Neovim theme). Everything
//! is overridable from the `[theme]` section of the config file; see [`ThemeConfig`]. The viewer
//! only ever asks this module for colors, so a different theme is a config change.
//!
//! This module is UI-framework free: colors are plain [`Rgb`] triples.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diagram::{GroupKind, ParticipantKind, Shape};

/// A 24-bit color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `#rrggbb` or `rrggbb`.
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 || !h.is_ascii() {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Rgb(p(0)?, p(2)?, p(4)?))
    }

    /// `#rrggbb`.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Linear blend: `t = 0` is `self`, `t = 1` is `other`.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// Per-kind color overrides (hex strings). Used twice: `[theme.fills]` (box backgrounds) and
/// `[theme.borders]` (box borders, icons, arrows).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KindColors {
    pub process: Option<String>,
    pub decision: Option<String>,
    pub terminal: Option<String>,
    pub io: Option<String>,
    pub subroutine: Option<String>,
    pub database: Option<String>,
    pub actor: Option<String>,
    pub service: Option<String>,
    pub queue: Option<String>,
    pub external: Option<String>,
    pub note: Option<String>,
    pub alt: Option<String>,
    pub opt: Option<String>,
    pub r#loop: Option<String>,
    pub par: Option<String>,
}

/// The `[theme]` config section. Every key is optional; colors are `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// Palette name. Only `catppuccin-mocha` ships.
    pub name: String,
    /// Canvas background.
    pub bg: Option<String>,
    /// Bars (header, footer).
    pub bar: Option<String>,
    /// Normal text.
    pub fg: Option<String>,
    /// Secondary text (hints, edge labels).
    pub dim: Option<String>,
    /// Focus color: focused fill, incident edges, the comment float border.
    pub accent: Option<String>,
    /// Text drawn on the accent fill.
    pub accent_fg: Option<String>,
    /// Border of the focused object.
    pub focus_border: Option<String>,
    /// Edge lines and lifelines.
    pub edge: Option<String>,
    /// Details panels and the comment float background.
    pub panel: Option<String>,
    /// Badge and warning color (`+N`, moved links).
    pub warn: Option<String>,
    /// Stale links and errors.
    pub error: Option<String>,
    /// Success messages, idle/done agents.
    pub ok: Option<String>,
    /// Code links and the CODE section.
    pub link: Option<String>,
    /// Search matches.
    pub search: Option<String>,
    /// Fill overrides per shape/participant/note/group.
    pub fills: KindColors,
    /// Border overrides per shape/participant/note/group.
    pub borders: KindColors,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            name: "catppuccin-mocha".into(),
            bg: None,
            bar: None,
            fg: None,
            dim: None,
            accent: None,
            accent_fg: None,
            focus_border: None,
            edge: None,
            panel: None,
            warn: None,
            error: None,
            ok: None,
            link: None,
            search: None,
            fills: KindColors::default(),
            borders: KindColors::default(),
        }
    }
}

/// Fill and border color of one kind of box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxColors {
    /// Background of the box interior.
    pub fill: Rgb,
    /// Border, icon and arrow color.
    pub border: Rgb,
}

/// Resolved colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub bg: Rgb,
    pub bar: Rgb,
    pub fg: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub accent_fg: Rgb,
    pub focus_border: Rgb,
    pub edge: Rgb,
    pub panel: Rgb,
    pub warn: Rgb,
    pub error: Rgb,
    pub ok: Rgb,
    pub link: Rgb,
    pub search: Rgb,
    kinds: BTreeMap<&'static str, BoxColors>,
}

/// Catppuccin Mocha palette.
pub mod mocha {
    use super::Rgb;
    pub const ROSEWATER: Rgb = Rgb(0xf5, 0xe0, 0xdc);
    pub const FLAMINGO: Rgb = Rgb(0xf2, 0xcd, 0xcd);
    pub const PINK: Rgb = Rgb(0xf5, 0xc2, 0xe7);
    pub const MAUVE: Rgb = Rgb(0xcb, 0xa6, 0xf7);
    pub const RED: Rgb = Rgb(0xf3, 0x8b, 0xa8);
    pub const MAROON: Rgb = Rgb(0xeb, 0xa0, 0xac);
    pub const PEACH: Rgb = Rgb(0xfa, 0xb3, 0x87);
    pub const YELLOW: Rgb = Rgb(0xf9, 0xe2, 0xaf);
    pub const GREEN: Rgb = Rgb(0xa6, 0xe3, 0xa1);
    pub const TEAL: Rgb = Rgb(0x94, 0xe2, 0xd5);
    pub const SKY: Rgb = Rgb(0x89, 0xdc, 0xeb);
    pub const SAPPHIRE: Rgb = Rgb(0x74, 0xc7, 0xec);
    pub const BLUE: Rgb = Rgb(0x89, 0xb4, 0xfa);
    pub const LAVENDER: Rgb = Rgb(0xb4, 0xbe, 0xfe);
    pub const TEXT: Rgb = Rgb(0xcd, 0xd6, 0xf4);
    pub const SUBTEXT0: Rgb = Rgb(0xa6, 0xad, 0xc8);
    pub const OVERLAY0: Rgb = Rgb(0x6c, 0x70, 0x86);
    pub const SURFACE0: Rgb = Rgb(0x31, 0x32, 0x44);
    pub const SURFACE1: Rgb = Rgb(0x45, 0x47, 0x5a);
    pub const SURFACE2: Rgb = Rgb(0x58, 0x5b, 0x70);
    pub const BASE: Rgb = Rgb(0x1e, 0x1e, 0x2e);
    pub const MANTLE: Rgb = Rgb(0x18, 0x18, 0x25);
    pub const CRUST: Rgb = Rgb(0x11, 0x11, 0x1b);
}

/// All kinds a [`Theme`] knows colors for.
const KIND_NAMES: [&str; 15] = [
    "process",
    "decision",
    "terminal",
    "io",
    "subroutine",
    "database",
    "actor",
    "service",
    "queue",
    "external",
    "note",
    "alt",
    "opt",
    "loop",
    "par",
];

fn kind_hue(name: &str) -> Rgb {
    use mocha::*;
    match name {
        "process" => BLUE,
        "decision" => MAUVE,
        "terminal" => GREEN,
        "io" => PEACH,
        "subroutine" => TEAL,
        "database" => YELLOW,
        "actor" => GREEN,
        "service" => BLUE,
        "queue" => PEACH,
        "external" => MAROON,
        "note" => YELLOW,
        "alt" => MAUVE,
        "opt" => TEAL,
        "loop" => PEACH,
        "par" => SKY,
        _ => TEXT,
    }
}

impl KindColors {
    fn get(&self, name: &str) -> Option<&String> {
        match name {
            "process" => self.process.as_ref(),
            "decision" => self.decision.as_ref(),
            "terminal" => self.terminal.as_ref(),
            "io" => self.io.as_ref(),
            "subroutine" => self.subroutine.as_ref(),
            "database" => self.database.as_ref(),
            "actor" => self.actor.as_ref(),
            "service" => self.service.as_ref(),
            "queue" => self.queue.as_ref(),
            "external" => self.external.as_ref(),
            "note" => self.note.as_ref(),
            "alt" => self.alt.as_ref(),
            "opt" => self.opt.as_ref(),
            "loop" => self.r#loop.as_ref(),
            "par" => self.par.as_ref(),
            _ => None,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::catppuccin_mocha()
    }
}

impl Theme {
    /// The built-in default.
    pub fn catppuccin_mocha() -> Theme {
        use mocha::*;
        let mut kinds = BTreeMap::new();
        for k in KIND_NAMES {
            let hue = kind_hue(k);
            kinds.insert(k, BoxColors { fill: BASE.mix(hue, 0.17), border: hue });
        }
        Theme {
            bg: BASE,
            bar: MANTLE,
            fg: TEXT,
            dim: SUBTEXT0,
            accent: BLUE,
            accent_fg: CRUST,
            focus_border: LAVENDER,
            edge: SURFACE2,
            panel: SURFACE0,
            warn: PEACH,
            error: RED,
            ok: GREEN,
            link: SAPPHIRE,
            search: YELLOW,
            kinds,
        }
    }

    /// Apply the `[theme]` config section over the default palette.
    pub fn from_config(c: &ThemeConfig) -> Result<Theme, String> {
        if c.name != "catppuccin-mocha" {
            return Err(format!("unknown theme name `{}` (available: catppuccin-mocha)", c.name));
        }
        let mut t = Theme::catppuccin_mocha();
        let pick = |key: &str, v: &Option<String>, slot: &mut Rgb| -> Result<(), String> {
            if let Some(s) = v {
                *slot = Rgb::from_hex(s).ok_or_else(|| format!("theme.{key}: `{s}` is not a #rrggbb color"))?;
            }
            Ok(())
        };
        pick("bg", &c.bg, &mut t.bg)?;
        pick("bar", &c.bar, &mut t.bar)?;
        pick("fg", &c.fg, &mut t.fg)?;
        pick("dim", &c.dim, &mut t.dim)?;
        pick("accent", &c.accent, &mut t.accent)?;
        pick("accent_fg", &c.accent_fg, &mut t.accent_fg)?;
        pick("focus_border", &c.focus_border, &mut t.focus_border)?;
        pick("edge", &c.edge, &mut t.edge)?;
        pick("panel", &c.panel, &mut t.panel)?;
        pick("warn", &c.warn, &mut t.warn)?;
        pick("error", &c.error, &mut t.error)?;
        pick("ok", &c.ok, &mut t.ok)?;
        pick("link", &c.link, &mut t.link)?;
        pick("search", &c.search, &mut t.search)?;
        for k in KIND_NAMES {
            let hue_override = c.borders.get(k);
            let fill_override = c.fills.get(k);
            let cur = t.kinds[k];
            let border = match hue_override {
                Some(s) => Rgb::from_hex(s).ok_or_else(|| format!("theme.borders.{k}: `{s}` is not a #rrggbb color"))?,
                None => cur.border,
            };
            let fill = match fill_override {
                Some(s) => Rgb::from_hex(s).ok_or_else(|| format!("theme.fills.{k}: `{s}` is not a #rrggbb color"))?,
                // the default fill follows the (possibly overridden) border and background
                None => t.bg.mix(border, 0.17),
            };
            t.kinds.insert(k, BoxColors { fill, border });
        }
        Ok(t)
    }

    fn kind(&self, name: &str) -> BoxColors {
        self.kinds.get(name).copied().unwrap_or(BoxColors { fill: self.panel, border: self.fg })
    }

    /// Colors of a flowchart node shape.
    pub fn shape(&self, s: Shape) -> BoxColors {
        self.kind(match s {
            Shape::Process => "process",
            Shape::Decision => "decision",
            Shape::Terminal => "terminal",
            Shape::Io => "io",
            Shape::Subroutine => "subroutine",
            Shape::Database => "database",
        })
    }

    /// Colors of a sequence participant box (`None` kind is drawn like a service).
    pub fn participant(&self, k: Option<ParticipantKind>) -> BoxColors {
        self.kind(match k {
            Some(ParticipantKind::Actor) => "actor",
            Some(ParticipantKind::Service) | None => "service",
            Some(ParticipantKind::Database) => "database",
            Some(ParticipantKind::Queue) => "queue",
            Some(ParticipantKind::External) => "external",
        })
    }

    /// Colors of a sequence note.
    pub fn note(&self) -> BoxColors {
        self.kind("note")
    }

    /// Frame color of a group (`border`; `fill` is unused by frames).
    pub fn group(&self, k: GroupKind) -> BoxColors {
        self.kind(match k {
            GroupKind::Alt => "alt",
            GroupKind::Opt => "opt",
            GroupKind::Loop => "loop",
            GroupKind::Par => "par",
        })
    }

    /// Color of an agent state as shown in the picker: idle/done green, working amber,
    /// blocked red, anything else dim.
    pub fn agent_state(&self, state: &str) -> Rgb {
        match state {
            "idle" | "done" => self.ok,
            "working" => mocha::YELLOW,
            "blocked" => self.error,
            _ => self.dim,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_and_mix() {
        let c = Rgb::from_hex("#89b4fa").unwrap();
        assert_eq!(c, mocha::BLUE);
        assert_eq!(c.to_hex(), "#89b4fa");
        assert_eq!(Rgb::from_hex("89B4FA"), Some(c));
        assert!(Rgb::from_hex("#89b4f").is_none());
        assert!(Rgb::from_hex("#zzzzzz").is_none());
        assert_eq!(Rgb(0, 0, 0).mix(Rgb(200, 100, 50), 0.5), Rgb(100, 50, 25));
        assert_eq!(Rgb(1, 2, 3).mix(Rgb(9, 9, 9), 0.0), Rgb(1, 2, 3));
    }

    #[test]
    fn default_config_resolves_to_mocha() {
        let t = Theme::from_config(&ThemeConfig::default()).unwrap();
        assert_eq!(t, Theme::catppuccin_mocha());
        assert_eq!(t.bg, Rgb(0x1e, 0x1e, 0x2e));
        assert_ne!(t.shape(Shape::Decision).fill, t.shape(Shape::Process).fill);
    }

    #[test]
    fn overrides_apply_and_derive_fills() {
        let mut c = ThemeConfig { accent: Some("#ff0000".into()), ..Default::default() };
        c.borders.process = Some("#00ff00".into());
        let t = Theme::from_config(&c).unwrap();
        assert_eq!(t.accent, Rgb(255, 0, 0));
        assert_eq!(t.shape(Shape::Process).border, Rgb(0, 255, 0));
        assert_eq!(t.shape(Shape::Process).fill, t.bg.mix(Rgb(0, 255, 0), 0.17));
        c.fills.process = Some("#101010".into());
        assert_eq!(Theme::from_config(&c).unwrap().shape(Shape::Process).fill, Rgb(16, 16, 16));
    }

    #[test]
    fn bad_values_are_reported_with_the_key() {
        let c = ThemeConfig { fg: Some("red".into()), ..Default::default() };
        assert!(Theme::from_config(&c).unwrap_err().contains("theme.fg"));
        let mut c = ThemeConfig::default();
        c.fills.io = Some("#12".into());
        assert!(Theme::from_config(&c).unwrap_err().contains("theme.fills.io"));
        let c = ThemeConfig { name: "nope".into(), ..Default::default() };
        assert!(Theme::from_config(&c).unwrap_err().contains("catppuccin-mocha"));
    }

    #[test]
    fn agent_state_colors() {
        let t = Theme::default();
        assert_eq!(t.agent_state("idle"), t.ok);
        assert_eq!(t.agent_state("done"), t.ok);
        assert_eq!(t.agent_state("blocked"), t.error);
        assert_eq!(t.agent_state("working"), mocha::YELLOW);
    }
}
