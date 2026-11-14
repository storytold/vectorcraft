//! Preferences › Performance › Use Monitor Profile for Canvas: the canvas's art, its artboards'
//! paper and the transparency grid are converted from sRGB into the ICC profile of the display
//! that shows most of the main window, as the last step before they are drawn.
//!
//! The desktop app reads the display's profile on a background thread and hands the reading to
//! [`DisplayProfile::report`], which checks the profile and builds its conversion table there.
//! Where the system converts the window to the display itself (macOS, Windows with Advanced
//! Color, a Wayland session), or the display has no usable profile, nothing is converted and the
//! canvas stays sRGB. Exports, `ui.render`, proofs and the document are not affected: a proof is
//! drawn first and then converted like any other frame.

use std::sync::{Arc, RwLock};

use serde_json::{Value, json};
use vectorcraft_color::cms::display::DisplayTransform;
pub use vectorcraft_color::cms::display::MAX_PROFILE_BYTES;

/// What the desktop app read for the display showing the main window.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The system's name for the display (Windows: `\\.\DISPLAY1`; X11: the output's name).
    pub display: Option<String>,
    pub found: Found,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// The display's ICC profile, and where it was read from.
    Profile { icc: Vec<u8>, from: String },
    /// The system converts the window to the display: VectorCraft converts nothing. The reason.
    System(String),
    /// The display has no profile VectorCraft can read: the canvas stays sRGB. The reason.
    NoProfile(String),
}

/// What the canvas does with the reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Nothing has been read: the web build, tests, or a platform without a reader.
    Unsupported,
    /// The first reading has not finished.
    Pending,
    /// The profile is converted into.
    Applied,
    /// The profile shows sRGB unchanged.
    Identity,
    /// The system converts the window.
    System,
    /// No profile, or one that can't be used.
    NoProfile,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Pending => "pending",
            Self::Applied => "profile",
            Self::Identity => "srgbProfile",
            Self::System => "system",
            Self::NoProfile => "none",
        }
    }
}

#[derive(Debug)]
struct State {
    /// Changes whenever the conversion changes, so frames rendered for the old one are rendered again.
    generation: u64,
    kind: Kind,
    display: Option<String>,
    /// The profile's description and where it was read from.
    profile: Option<(String, String)>,
    reason: Option<String>,
    transform: Option<Arc<DisplayTransform>>,
    /// The bytes last reported, so a reading of the same profile builds nothing.
    reported: Option<(Option<String>, Vec<u8>)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            generation: 0,
            kind: Kind::Unsupported,
            display: None,
            profile: None,
            reason: Some("this build doesn't read display profiles".into()),
            transform: None,
            reported: None,
        }
    }
}

/// The display profile of the main window, shared with the thread that reads it.
#[derive(Clone, Debug, Default)]
pub struct DisplayProfile(Arc<RwLock<State>>);

impl DisplayProfile {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The desktop app has started reading the display's profile.
    pub fn pending(&self) {
        let mut s = self.write();
        if s.kind == Kind::Unsupported {
            s.kind = Kind::Pending;
            s.reason = Some("the display profile hasn't been read yet".into());
        }
    }

    /// Takes `reading` in. A profile is checked, and its conversion built, on the calling thread;
    /// a profile that can't be used leaves the canvas sRGB, with the reason in [`Self::status`].
    /// The same profile reported again for the same display changes nothing. Returns whether the
    /// canvas's conversion changed.
    pub fn report(&self, reading: Reading) -> bool {
        if let Found::Profile { icc, .. } = &reading.found
            && self.read().reported.as_ref().is_some_and(|(d, b)| *d == reading.display && b == icc)
        {
            return false;
        }
        let (kind, profile, reason, transform, reported) = match reading.found {
            Found::Profile { icc, from } => {
                let reported = Some((reading.display.clone(), icc.clone()));
                match DisplayTransform::from_icc(&icc) {
                    Ok(t) if t.is_identity() => (Kind::Identity, Some((t.name().to_string(), from)), None, None, reported),
                    Ok(t) => (Kind::Applied, Some((t.name().to_string(), from)), None, Some(Arc::new(t)), reported),
                    Err(e) => (Kind::NoProfile, None, Some(format!("{from}: {e}")), None, reported),
                }
            }
            Found::System(r) => (Kind::System, None, Some(r), None, None),
            Found::NoProfile(r) => (Kind::NoProfile, None, Some(r), None, None),
        };
        let mut s = self.write();
        let changed = s.kind != kind || s.display != reading.display || s.profile != profile || s.reason != reason;
        let converts_differently = match (&s.transform, &transform) {
            (None, None) => false,
            (Some(a), Some(b)) => a.fingerprint() != b.fingerprint(),
            _ => true,
        };
        if converts_differently {
            s.generation += 1;
        }
        (s.kind, s.display, s.profile, s.reason, s.transform, s.reported) = (kind, reading.display, profile, reason, transform, reported);
        changed || converts_differently
    }

    /// The conversion the canvas draws through: `None` when `enabled` (the preference) is off, or
    /// when nothing is to be converted.
    pub fn transform(&self, enabled: bool) -> Option<Arc<DisplayTransform>> {
        if !enabled {
            return None;
        }
        self.read().transform.clone()
    }

    /// A number that changes whenever [`Self::transform`] does, 0 while it is `None`: the canvas's
    /// frames depend on it ([`crate::CacheKey::display`]).
    pub fn key(&self, enabled: bool) -> u64 {
        let s = self.read();
        if enabled && s.transform.is_some() { s.generation } else { 0 }
    }

    /// The status `ui.inspect` reports as `displayProfile`: the display, what was found
    /// (`profile`, `srgbProfile`, `system`, `none`, `pending` or `unsupported`), the profile's
    /// description and where it was read from, whether the canvas is converted, why not, and
    /// whether the system converts the window.
    pub fn status(&self, enabled: bool) -> Value {
        let s = self.read();
        json!({
            "enabled": enabled,
            "display": s.display,
            "source": s.kind.id(),
            "profile": s.profile.as_ref().map(|p| &p.0),
            "from": s.profile.as_ref().map(|p| &p.1),
            "applied": enabled && s.transform.is_some(),
            "reason": s.reason,
            "osManaged": s.kind == Kind::System,
        })
    }

    /// The line the Preferences dialog shows under the checkbox, translated.
    pub(crate) fn note(&self, enabled: bool) -> String {
        let s = self.read();
        match (enabled, s.kind, &s.profile) {
            (false, ..) => tl!("The canvas is drawn in sRGB.").to_string(),
            (true, Kind::Applied | Kind::Identity, Some((name, _))) => format!("{} {name}", tl!("Canvas display profile:")),
            (true, Kind::System, _) => tl!("The system matches the window to the display's colors.").to_string(),
            _ => tl!("No usable display profile was found: the canvas is drawn in sRGB.").to_string(),
        }
    }
}

/// Why `icc` can't be used as a display profile, or `Ok` when [`DisplayProfile::report`] would
/// convert through it (or find that it shows sRGB unchanged). Builds no conversion table.
pub fn check_profile(icc: &[u8]) -> Result<(), String> {
    DisplayTransform::check(icc).map_err(|e| e.to_string())
}

/// `pixels` (premultiplied RGBA8) through `t`, using every core for a large frame.
pub(crate) fn convert(t: Option<&DisplayTransform>, pixels: &mut [u8]) {
    let Some(t) = t else { return };
    #[cfg(not(target_arch = "wasm32"))]
    {
        const PIECE: usize = 256 * 1024 * 4;
        let threads = std::thread::available_parallelism().map_or(1, usize::from).min(16);
        if threads > 1 && pixels.len() > PIECE {
            let per = pixels.len().div_ceil(threads).div_ceil(4) * 4;
            std::thread::scope(|scope| {
                for piece in pixels.chunks_mut(per) {
                    scope.spawn(move || t.apply_premultiplied(piece));
                }
            });
            return;
        }
    }
    t.apply_premultiplied(pixels);
}

/// The opaque or translucent color `c` through `t`.
pub(crate) fn color32(t: Option<&DisplayTransform>, c: egui::Color32) -> egui::Color32 {
    let Some(t) = t else { return c };
    let mut px = c.to_array();
    t.apply_premultiplied(&mut px);
    egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Display P3's ICC file as a display profile.
    pub(crate) fn p3() -> Vec<u8> {
        let mut b = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::DISPLAY_P3).unwrap().to_vec();
        b[12..16].copy_from_slice(b"mntr");
        b
    }

    fn reading(found: Found) -> Reading {
        Reading { display: Some("DP-1".into()), found }
    }

    #[test]
    fn a_display_profile_is_converted_into_while_the_preference_is_on() {
        let d = DisplayProfile::default();
        assert_eq!(d.status(true)["source"], "unsupported");
        assert!(d.transform(true).is_none() && d.key(true) == 0);
        d.pending();
        assert_eq!(d.status(true)["source"], "pending");
        assert!(d.report(reading(Found::Profile { icc: p3(), from: "colord".into() })));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool(), st["profile"].as_str()), (Some("profile"), Some(true), Some("Display P3")));
        assert_eq!((st["display"].as_str(), st["from"].as_str(), st["osManaged"].as_bool()), (Some("DP-1"), Some("colord"), Some(false)));
        assert!(d.transform(true).is_some());
        let key = d.key(true);
        assert_ne!(key, 0);
        // Off: nothing is converted, and frames don't depend on the profile.
        assert!(d.transform(false).is_none());
        assert_eq!((d.key(false), d.status(false)["applied"].as_bool()), (0, Some(false)));
        // The same profile again changes nothing.
        assert!(!d.report(reading(Found::Profile { icc: p3(), from: "colord".into() })));
        assert_eq!(d.key(true), key);
        // The system takes over: nothing is converted.
        assert!(d.report(reading(Found::System("Windows manages this display's colors".into()))));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool(), st["osManaged"].as_bool()), (Some("system"), Some(false), Some(true)));
        assert_eq!(d.key(true), 0);
        // The profile back: a new key, so frames drawn before are drawn again.
        d.report(reading(Found::Profile { icc: p3(), from: "colord".into() }));
        assert!(d.key(true) > key);
    }

    #[test]
    fn a_profile_that_can_t_be_used_leaves_the_canvas_srgb_and_says_why() {
        let d = DisplayProfile::default();
        let cmyk = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::GENERIC_CMYK).unwrap().to_vec();
        assert!(d.report(reading(Found::Profile { icc: cmyk, from: "printer.icc".into() })));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool()), (Some("none"), Some(false)));
        assert!(st["reason"].as_str().is_some_and(|r| r.starts_with("printer.icc: ") && r.contains("not a display profile")), "{st}");
        assert!(d.transform(true).is_none());
        d.report(reading(Found::Profile { icc: vec![1, 2, 3], from: "broken.icc".into() }));
        assert!(d.status(true)["reason"].as_str().is_some_and(|r| r.contains("not an ICC profile")));
        // An sRGB display profile converts nothing.
        let mut srgb = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::SRGB).unwrap().to_vec();
        srgb[12..16].copy_from_slice(b"mntr");
        d.report(reading(Found::Profile { icc: srgb, from: "sRGB.icc".into() }));
        assert_eq!((d.status(true)["source"].as_str(), d.status(true)["applied"].as_bool()), (Some("srgbProfile"), Some(false)));
        assert!(d.transform(true).is_none());
    }

    #[test]
    fn large_frames_convert_the_same_on_several_threads() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        let mut big: Vec<u8> = (0..(600 * 1024 * 4)).map(|i| (i * 7 % 251) as u8).collect();
        for px in big.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let mut one = big.clone();
        t.apply_premultiplied(&mut one);
        convert(Some(&t), &mut big);
        assert_eq!(big, one);
        assert_eq!(color32(None, egui::Color32::RED), egui::Color32::RED);
        let [r, g, b] = t.apply8([255, 0, 0]);
        assert_eq!(color32(Some(&t), egui::Color32::RED), egui::Color32::from_rgb(r, g, b));
    }
}
