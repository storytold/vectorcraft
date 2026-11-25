//! The user's interface choices (brightness, accent, colour scheme) and how they select the root theme.

use craft::theme::{AppSizes, ModeChoice, Rgba, ThemeId};
use egui::{FontFamily, FontId};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use vectorcraft_engine::Prefs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Brightness {
    Dark,
    #[default]
    MediumDark,
    MediumLight,
    Light,
}

impl Brightness {
    pub const ALL: [Brightness; 4] = [Brightness::Dark, Brightness::MediumDark, Brightness::MediumLight, Brightness::Light];
    pub fn label(self) -> &'static str {
        match self {
            Brightness::Dark => "Dark",
            Brightness::MediumDark => "Medium Dark",
            Brightness::MediumLight => "Medium Light",
            Brightness::Light => "Light",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Brightness::Dark => "dark",
            Brightness::MediumDark => "mediumDark",
            Brightness::MediumLight => "mediumLight",
            Brightness::Light => "light",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id().eq_ignore_ascii_case(s) || b.label().eq_ignore_ascii_case(s))
    }
}

/// User Interface › Accent Color: the one colour for what is on, chosen or focused. Coral, the
/// default, is the app icon's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accent {
    #[default]
    Coral,
    Blue,
    Violet,
    Green,
    Amber,
    Pink,
}

impl Accent {
    pub const ALL: [Accent; 6] = [Accent::Coral, Accent::Blue, Accent::Violet, Accent::Green, Accent::Amber, Accent::Pink];
    pub fn id(self) -> &'static str {
        match self {
            Accent::Coral => "coral",
            Accent::Blue => "blue",
            Accent::Violet => "violet",
            Accent::Green => "green",
            Accent::Amber => "amber",
            Accent::Pink => "pink",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(s))
    }
    /// The accent's seed colour (`0xRRGGBB`) on dark panels and on light ones.
    fn seed_hex(self, dark: bool) -> u32 {
        match (self, dark) {
            (Accent::Coral, true) => 0xf06a52,
            (Accent::Coral, false) => 0xd9472f,
            (Accent::Blue, true) => 0x5b96ff,
            (Accent::Blue, false) => 0x2563eb,
            (Accent::Violet, true) => 0xa98bff,
            (Accent::Violet, false) => 0x7146dc,
            (Accent::Green, true) => 0x3fcf8e,
            (Accent::Green, false) => 0x16955a,
            (Accent::Amber, true) => 0xf5b544,
            (Accent::Amber, false) => 0xb06d06,
            (Accent::Pink, true) => 0xf36aa3,
            (Accent::Pink, false) => 0xcc3672,
        }
    }
}

/// User Interface › Color Scheme: Modern (the default: slate greys and the chosen accent) or
/// Classic (the neutral greys and blue accent of earlier versions). Both keep the same layout,
/// cards and corners; only the colours differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    #[default]
    Modern,
    Classic,
}

impl Scheme {
    pub const ALL: [Scheme; 2] = [Scheme::Modern, Scheme::Classic];
    pub fn id(self) -> &'static str {
        match self {
            Scheme::Modern => "modern",
            Scheme::Classic => "classic",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s))
    }
}

/// A built-in theme's id; the ids are constants, so the fallback never shows.
fn theme_id(id: &str) -> ThemeId {
    ThemeId::parse(id).unwrap_or_else(|_| ThemeId::suite_default())
}

/// The root theme the saved interface preferences select: its id, its fixed mode (the four
/// brightness levels decide light or dark, never the system) and the accent seed. Classic keeps its
/// own blue and Modern's coral is in its theme files, so both return no seed there.
pub fn root_theme(prefs: &Prefs) -> (ThemeId, ModeChoice, Option<Rgba>) {
    let brightness = Brightness::parse(&prefs.ui_brightness).unwrap_or_default();
    let accent = Accent::parse(&prefs.ui_accent).unwrap_or_default();
    let scheme = Scheme::parse(&prefs.ui_color_scheme).unwrap_or_default();
    let dark = matches!(brightness, Brightness::Dark | Brightness::MediumDark);
    let mode = if dark { ModeChoice::Dark } else { ModeChoice::Light };
    match scheme {
        Scheme::Classic => {
            let id = match brightness {
                Brightness::MediumDark => "craft.artcraft",
                Brightness::MediumLight => "craft.pro-light-gray",
                Brightness::Dark | Brightness::Light => "craft.pro",
            };
            (theme_id(id), mode, None)
        }
        Scheme::Modern => {
            let id = match brightness {
                Brightness::Dark | Brightness::Light => "craft.vectorcraft-modern",
                Brightness::MediumDark | Brightness::MediumLight => "craft.vectorcraft-modern-medium",
            };
            let seed = (accent != Accent::Coral).then(|| {
                let h = accent.seed_hex(dark);
                Rgba::rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
            });
            (theme_id(id), mode, seed)
        }
    }
}

/// The app's sizes over the framework's size sets (`sizes.json`): push buttons have the app's 4-point corners
/// where the sets give pills, the Home title is 26-point under 60 points of page margin with a 36-point call to
/// action, and the compact set takes the reference app's metrics:
/// 26-point fields and spinners, 24-point push buttons, 10 points inside panels, 5 between rows, 16-point glyphs in
/// icon buttons, 5-point control corners, 8-point menus and 10-point windows, 33-point panel tabs, 310-point dock
/// columns with 14-point column bars and 38-point icon columns, 29-point menu rows, and dialogs 22 points inside their
/// edge with 28-point buttons at least 72 points wide.
/// Its type is the size the reference app draws: it set Inter at 92% of its nominal 13-point body, 12.5-point labels
/// and 11.5-point captions, so they read at 12, 11.5 and 10.5 points, on the rows those nominal sizes gave.
const SIZES: &[u8] = br#"{
    "all": {
        "radius.button": 4,
        "typography.display.size": 26,
        "typography.display.line_height": 32,
        "size.page.margin": 60,
        "size.page.action": 36
    },
    "compact": {
        "size.control.md": 26,
        "size.button": 24,
        "size.icon.glyph_md": 16,
        "size.rail.width": 38,
        "size.dock.column": 310,
        "size.bar.column": 14,
        "size.panel_header": 33,
        "size.bar.tab": 35,
        "size.bar.tab_large": 44,
        "space.pad.regular": 10,
        "space.gap.row": 5,
        "radius.control": 5,
        "radius.surface": 8,
        "radius.window": 10,
        "size.dialog.button": 72,
        "size.dialog.button_height": 28,
        "size.row.menu": 29,
        "space.pad.dialog": 22,
        "typography.body.size": 12,
        "typography.body.line_height": 17,
        "typography.body_strong.size": 12,
        "typography.body_strong.line_height": 17,
        "typography.label.size": 11.5,
        "typography.label.line_height": 17,
        "typography.caption.size": 10.5,
        "typography.caption.line_height": 15
    }
}"#;

/// The app's sizes ([`SIZES`]) as one shared value: the token cache tells a change of sizes by its pointer, so a
/// new value each frame would resolve the tokens again each frame.
pub fn app_sizes() -> Arc<AppSizes> {
    static SHARED: LazyLock<Arc<AppSizes>> = LazyLock::new(|| Arc::new(AppSizes::parse_or_default(SIZES)));
    Arc::clone(&SHARED)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(craft::ui::fonts::SEMIBOLD.into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saved strings are what Preferences and old settings files hold.
    #[test]
    fn brightness_roundtrip() {
        for b in Brightness::ALL {
            assert_eq!(Brightness::parse(b.id()), Some(b));
            assert_eq!(Brightness::parse(b.label()), Some(b));
        }
    }

    /// A size `app_sizes` cannot read is only logged and dropped, which brings the pill buttons back unnoticed.
    #[test]
    fn the_app_sizes_are_read_whole() {
        assert!(AppSizes::parse(SIZES).is_ok_and(|sizes| !sizes.is_empty()));
    }
}
