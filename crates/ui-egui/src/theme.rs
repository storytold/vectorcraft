//! The user's interface choices (brightness, accent, colour scheme) and how they select the root theme.

use craft::theme::{AppSizes, ModeChoice, Rgba, ThemeId};
use craft::ui::runtime::env::RootEnv;
use egui::{FontFamily, FontId};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use vectorcraft_engine::Prefs;

mod proposal;

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

/// Modern remains the default. Classic, Flat and Quiet are opt-in schemes; the proposal palettes
/// and corner scale use the same shared framework as the built-in themes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    #[default]
    Modern,
    Classic,
    Flat,
    Quiet,
}

impl Scheme {
    pub const ALL: [Scheme; 4] = [Scheme::Modern, Scheme::Classic, Scheme::Flat, Scheme::Quiet];
    pub fn id(self) -> &'static str {
        match self {
            Scheme::Modern => "modern",
            Scheme::Classic => "classic",
            Scheme::Flat => "flat",
            Scheme::Quiet => "quiet",
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

/// The root theme the saved interface preferences select: its id, fixed mode and accent seed.
/// Classic, Flat and Quiet keep their own blue; Modern keeps the user's accent.
pub fn root_theme(prefs: &Prefs) -> (ThemeId, ModeChoice, Option<Rgba>) {
    let brightness = Brightness::parse(&prefs.ui_brightness).unwrap_or_default();
    let accent = Accent::parse(&prefs.ui_accent).unwrap_or_default();
    let scheme = Scheme::parse(&prefs.ui_color_scheme).unwrap_or_default();
    let dark = matches!(brightness, Brightness::Dark | Brightness::MediumDark);
    let mode = if dark { ModeChoice::Dark } else { ModeChoice::Light };
    match scheme {
        Scheme::Flat | Scheme::Quiet => (proposal::definition(scheme, brightness).id.clone(), mode, None),
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

/// Supplies app-defined themes through the framework's library-theme contract. Built-in schemes
/// keep the current production root, sizes and control settings unchanged.
pub(crate) fn apply_scheme(root: RootEnv, prefs: &Prefs) -> RootEnv {
    let scheme = Scheme::parse(&prefs.ui_color_scheme).unwrap_or_default();
    match scheme {
        Scheme::Modern | Scheme::Classic => root,
        Scheme::Flat | Scheme::Quiet => {
            let brightness = Brightness::parse(&prefs.ui_brightness).unwrap_or_default();
            let mut root = root.library_theme(proposal::definition(scheme, brightness));
            root.sizes = proposal::sizes(scheme);
            root
        }
    }
}

/// Production sizes remain shared by identity, as required by the framework's token cache.
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
        "typography.body.size": 13,
        "typography.body.line_height": 17,
        "typography.body_strong.size": 13,
        "typography.body_strong.line_height": 17,
        "typography.label.size": 12.5,
        "typography.label.line_height": 17,
        "typography.caption.size": 11,
        "typography.caption.line_height": 15
    }
}"#;

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

    fn resolved(root: RootEnv) -> Arc<craft::ui::style::UiTokens> {
        use craft::ui::runtime::{
            env::Env,
            frame::{FrameInput, set_input},
        };
        let ctx = crate::testing::ui_context();
        set_input(&ctx, FrameInput::new(root));
        let mut tokens = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| tokens = Some(Arc::clone(&Env::of(ui).tokens)));
        out.textures_delta.clear();
        tokens.expect("the frame resolves its root theme")
    }

    /// Quiet must not change a new install or a saved Modern selection at any brightness.
    #[test]
    fn the_default_look_has_no_quiet_lines_or_corners() {
        assert_eq!(Scheme::default(), Scheme::Modern);
        assert_eq!(Prefs::default().ui_color_scheme, "modern");
        for b in Brightness::ALL {
            let prefs = Prefs { ui_brightness: b.id().into(), ..Default::default() };
            let (id, mode) = match b {
                Brightness::Dark => ("craft.vectorcraft-modern", ModeChoice::Dark),
                Brightness::MediumDark => ("craft.vectorcraft-modern-medium", ModeChoice::Dark),
                Brightness::MediumLight => ("craft.vectorcraft-modern-medium", ModeChoice::Light),
                Brightness::Light => ("craft.vectorcraft-modern", ModeChoice::Light),
            };
            let actual = resolved(crate::app::root_env(&prefs, 1.0));
            let mut production = RootEnv::new().theme(theme_id(id)).mode(mode);
            production.sizes = app_sizes();
            let expected = resolved(production);
            assert_eq!(actual.color, expected.color, "{b:?}: production colours");
            assert_eq!(actual.shape, expected.shape, "{b:?}: production corners and edges");
        }
    }

    /// Confirming Quiet in Preferences changes the theme the real frame resolves, not just the
    /// stored choice or a private palette helper.
    #[test]
    fn choosing_quiet_in_the_preferences_applies_it() {
        let mut app = crate::testing::windowed(crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
        crate::prefs_dialog::open(&mut app, None);
        crate::prefs_dialog::set_field(&mut app, "uiColorScheme", serde_json::json!("quiet")).unwrap();
        crate::prefs_dialog::confirm(&mut app).unwrap();
        assert_eq!(app.ui.engine_prefs["uiColorScheme"], "quiet");
        for (b, panel, field) in [
            (Brightness::Dark, 0x151618, 0x0f1012),
            (Brightness::MediumDark, 0x1d1e21, 0x16171a),
            (Brightness::MediumLight, 0xe5e7ea, 0xdadde1),
            (Brightness::Light, 0xf3f4f6, 0xe9ebee),
        ] {
            app.session.prefs.ui_brightness = b.id().into();
            let actual = resolved(crate::app::root_env(&app.session.prefs, 1.0));
            let rgb = |v: u32| Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            assert_eq!(actual.color.surface.panel, rgb(panel), "{b:?}");
            assert_eq!(actual.color.surface.chrome, rgb(panel), "{b:?}: one chrome surface");
            assert_eq!(actual.color.surface.sunken, rgb(field), "{b:?}: inset fields");
            let r = actual.shape.radius;
            assert_eq!((r.sm, r.control, r.surface, r.window), (4.0, 8.0, 14.0, 14.0), "{b:?}");
        }
    }
}
