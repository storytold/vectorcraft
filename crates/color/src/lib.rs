//! Colour, paint and blend-mode types for VectorCraft.
//!
//! Colours keep the model the user picked them in (RGB, CMYK, Gray, HSB is a UI view of RGB), so
//! documents don't drift when converting back and forth. Rendering asks for [`Color::to_rgba`].
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blend;
pub mod cms;
pub mod gradient;
pub mod harmony;
pub mod swatch;

pub use blend::BlendMode;
pub use gradient::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
pub use swatch::{Swatch, SwatchGroup, default_swatches};

use serde::{Deserialize, Serialize};

/// A colour in its authoring model. Components are 0..=1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "lowercase")]
pub enum Color {
    Rgb { r: f32, g: f32, b: f32 },
    Cmyk { c: f32, m: f32, y: f32, k: f32 },
    Gray { k: f32 },
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}

impl Color {
    pub const BLACK: Color = Color::Rgb { r: 0.0, g: 0.0, b: 0.0 };
    pub const WHITE: Color = Color::Rgb { r: 1.0, g: 1.0, b: 1.0 };

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color::Rgb { r, g, b }
    }
    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::Rgb { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0 }
    }
    pub fn cmyk(c: f32, m: f32, y: f32, k: f32) -> Self {
        Color::Cmyk { c, m, y, k }
    }
    pub fn gray(k: f32) -> Self {
        Color::Gray { k }
    }

    /// Display (sRGB) colour through the active colour settings ([`cms::active`]): RGB is
    /// converted from the working RGB space, CMYK through the working CMYK profile (relative
    /// colorimetric). Grey is ink percentage (0 = white).
    pub fn to_rgb(&self) -> [f32; 3] {
        match *self {
            Color::Rgb { r, g, b } if cms::rgb_is_srgb() => [r, g, b],
            Color::Cmyk { c, m, y, k } if cms::cmyk_is_device() => cms::naive_cmyk_to_rgb([c, m, y, k]),
            // Illustrator's Gray is ink percentage: 0 = white, 1 = black.
            Color::Gray { k } => [1.0 - k; 3],
            _ => cms::active().display_rgb(self),
        }
    }
    /// Profile-free display RGB (`(1−c)(1−k)` …), the pre-colour-management formula.
    pub fn to_rgb_uncalibrated(&self) -> [f32; 3] {
        match *self {
            Color::Rgb { r, g, b } => [r, g, b],
            Color::Cmyk { c, m, y, k } => cms::naive_cmyk_to_rgb([c, m, y, k]),
            Color::Gray { k } => [1.0 - k; 3],
        }
    }
    /// CIE Lab (D50) through the active colour settings.
    pub fn to_lab(&self) -> cms::Lab {
        cms::active().lab(self)
    }
    /// Colour-managed CMYK in the working CMYK space with `intent` (see [`Color::to_cmyk`] for the
    /// profile-free formula).
    pub fn to_cmyk_managed(&self, intent: cms::Intent) -> [f32; 4] {
        cms::active().to_cmyk(self, intent)
    }
    /// Gamut warning: true when this colour can't be reproduced in the working CMYK space.
    pub fn out_of_gamut(&self) -> bool {
        cms::active().out_of_gamut(self)
    }
    pub fn to_rgba8(&self, alpha: f32) -> [u8; 4] {
        let [r, g, b] = self.to_rgb();
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(r), q(g), q(b), q(alpha)]
    }
    /// Profile-free CMYK (`k = 1 − max(r, g, b)` …), kept for exact legacy numbers; colour-managed
    /// separations use [`Color::to_cmyk_managed`].
    pub fn to_cmyk(&self) -> [f32; 4] {
        match *self {
            Color::Cmyk { c, m, y, k } => [c, m, y, k],
            _ => {
                let [r, g, b] = self.to_rgb();
                let k = 1.0 - r.max(g).max(b);
                if k >= 1.0 {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                [(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k]
            }
        }
    }
    /// HSB with hue in degrees 0..360, saturation and brightness 0..1.
    pub fn to_hsb(&self) -> [f32; 3] {
        let [r, g, b] = self.to_rgb();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d <= 0.0 {
            0.0
        } else if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let s = if max <= 0.0 { 0.0 } else { d / max };
        [h, s, max]
    }
    pub fn from_hsb(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        Color::Rgb { r: r + m, g: g + m, b: b + m }
    }
    /// `#rrggbb` (display RGB).
    pub fn to_hex(&self) -> String {
        let [r, g, b, _] = self.to_rgba8(1.0);
        format!("#{r:02x}{g:02x}{b:02x}")
    }
    /// Parse `#rgb`, `#rrggbb` or a few CSS names.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "black" => return Some(Color::BLACK),
            "white" => return Some(Color::WHITE),
            "red" => return Some(Color::rgb(1.0, 0.0, 0.0)),
            "green" => return Some(Color::rgb8(0, 128, 0)),
            "blue" => return Some(Color::rgb(0.0, 0.0, 1.0)),
            _ => {}
        }
        let h = s.strip_prefix('#').unwrap_or(s);
        let p = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
        match h.len() {
            3 => Some(Color::rgb8(p(0, 1)? * 17, p(1, 1)? * 17, p(2, 1)? * 17)),
            6 | 8 => Some(Color::rgb8(p(0, 2)?, p(2, 2)?, p(4, 2)?)),
            _ => None,
        }
    }
    /// Complement (Illustrator's Edit → Edit Colors → Invert is 1 - rgb; this is hue + 180).
    pub fn complement(&self) -> Self {
        let [h, s, v] = self.to_hsb();
        Color::from_hsb(h + 180.0, s, v)
    }
    pub fn invert(&self) -> Self {
        let [r, g, b] = self.to_rgb();
        Color::rgb(1.0 - r, 1.0 - g, 1.0 - b)
    }
    /// The RGB inverse expressed in the colour's own model (CMYK through the profile-free
    /// [`Color::to_cmyk`]; a grey's ink is inverted).
    pub fn invert_keep_model(&self) -> Self {
        match *self {
            Color::Rgb { .. } => self.invert(),
            Color::Cmyk { .. } => {
                let [c, m, y, k] = self.invert().to_cmyk();
                Color::Cmyk { c, m, y, k }
            }
            Color::Gray { k } => Color::Gray { k: 1.0 - k },
        }
    }
    /// Complement in the colour's own model: each component becomes (highest + lowest) − itself,
    /// over R, G, B or over C, M, Y (K kept). For RGB this is the hue turned by 180°; a grey is its
    /// own complement.
    pub fn complement_keep_model(&self) -> Self {
        let flip = |v: [f32; 3]| {
            let s = v[0].max(v[1]).max(v[2]) + v[0].min(v[1]).min(v[2]);
            v.map(|x| (s - x).clamp(0.0, 1.0))
        };
        match *self {
            Color::Rgb { r, g, b } => {
                let [r, g, b] = flip([r, g, b]);
                Color::Rgb { r, g, b }
            }
            Color::Cmyk { c, m, y, k } => {
                let [c, m, y] = flip([c, m, y]);
                Color::Cmyk { c, m, y, k }
            }
            Color::Gray { .. } => *self,
        }
    }
    /// Linear interpolation in display RGB.
    pub fn lerp(&self, other: &Color, t: f32) -> Color {
        let a = self.to_rgb();
        let b = other.to_rgb();
        Color::rgb(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t)
    }
    /// The colour model this colour is authored in.
    pub fn model(&self) -> cms::Model {
        match self {
            Color::Rgb { .. } => cms::Model::Rgb,
            Color::Cmyk { .. } => cms::Model::Cmyk,
            Color::Gray { .. } => cms::Model::Gray,
        }
    }
    /// This colour expressed in `model` with the profile-free formulas Edit Colors uses (RGB from
    /// the display colour, CMYK by [`Color::to_cmyk`], Gray as ink from luminance); unchanged when it
    /// is in `model` already.
    pub fn in_model(self, model: cms::Model) -> Color {
        if self.model() == model {
            return self;
        }
        match model {
            cms::Model::Rgb => {
                let [r, g, b] = self.to_rgb();
                Color::rgb(r, g, b)
            }
            cms::Model::Cmyk => {
                let [c, m, y, k] = self.to_cmyk();
                Color::cmyk(c, m, y, k)
            }
            cms::Model::Gray => {
                let [r, g, b] = self.to_rgb();
                Color::gray((1.0 - (0.299 * r + 0.587 * g + 0.114 * b)).clamp(0.0, 1.0))
            }
        }
    }
    pub fn model_name(&self) -> &'static str {
        match self {
            Color::Rgb { .. } => "RGB",
            Color::Cmyk { .. } => "CMYK",
            Color::Gray { .. } => "Grayscale",
        }
    }
}

/// `new` expressed in the colour model of `orig` ([`Color::in_model`]): results of colour operations
/// (harmonies, blends, inversions, recolouring) keep the model of the colour they came from.
pub fn keep_model(orig: Color, new: Color) -> Color {
    new.in_model(orig.model())
}

/// What fills or strokes an object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Paint {
    #[default]
    None,
    Solid {
        color: Color,
        /// Name of the global swatch this colour is linked to (edits to the swatch update it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        swatch: Option<String>,
    },
    Gradient(Box<GradientPaint>),
    Pattern {
        pattern: String,
        /// Pattern space → document placement (Transform Patterns edits it; identity = tiles
        /// anchored at the document origin).
        #[serde(default, skip_serializing_if = "is_identity")]
        xf: kurbo::Affine,
    },
}

fn is_identity(a: &kurbo::Affine) -> bool {
    *a == kurbo::Affine::IDENTITY
}

impl Paint {
    pub fn solid(c: Color) -> Self {
        Paint::Solid { color: c, swatch: None }
    }
    pub fn is_none(&self) -> bool {
        matches!(self, Paint::None)
    }
    pub fn color(&self) -> Option<Color> {
        match self {
            Paint::Solid { color, .. } => Some(*color),
            _ => None,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Paint::None => "None".into(),
            Paint::Solid { color, swatch: Some(n) } => format!("{n} ({})", color.to_hex()),
            Paint::Solid { color, .. } => color.to_hex(),
            Paint::Gradient(g) => format!("{} gradient", g.gradient.kind.label()),
            Paint::Pattern { pattern, .. } => format!("pattern {pattern}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Color::from_hex("#ff8000").unwrap();
        assert_eq!(c.to_hex(), "#ff8000");
        assert_eq!(Color::from_hex("#fff").unwrap().to_hex(), "#ffffff");
        assert_eq!(Color::from_hex("zz"), None);
    }

    #[test]
    fn hsb_roundtrip() {
        for hex in ["#ff0000", "#00ff00", "#0000ff", "#336699", "#ffffff", "#000000", "#c0ffee"] {
            let c = Color::from_hex(hex).unwrap();
            let [h, s, v] = c.to_hsb();
            assert_eq!(Color::from_hsb(h, s, v).to_hex(), hex, "{hex}");
        }
    }

    #[test]
    fn cmyk_conversions() {
        assert_eq!(Color::cmyk(0.0, 0.0, 0.0, 1.0).to_rgb_uncalibrated(), [0.0, 0.0, 0.0]);
        assert_eq!(Color::cmyk(1.0, 0.0, 0.0, 0.0).to_rgb_uncalibrated(), [0.0, 1.0, 1.0]);
        // Managed display: paper is white, 100% cyan is a press cyan (not #00ffff).
        assert_eq!(Color::cmyk(0.0, 0.0, 0.0, 0.0).to_hex(), "#ffffff");
        let [r, g, b] = Color::cmyk(1.0, 0.0, 0.0, 0.0).to_rgb();
        assert!(r < 0.2 && g > 0.5 && b > 0.8, "{r} {g} {b}");
        let k = Color::rgb(1.0, 0.0, 0.0).to_cmyk();
        assert_eq!(k, [0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn gray_is_ink() {
        assert_eq!(Color::gray(0.0).to_hex(), "#ffffff");
        assert_eq!(Color::gray(1.0).to_hex(), "#000000");
    }

    #[test]
    fn complement_and_invert() {
        assert_eq!(Color::rgb(1.0, 0.0, 0.0).complement().to_hex(), "#00ffff");
        assert_eq!(Color::rgb(1.0, 0.0, 0.0).invert().to_hex(), "#00ffff");
    }

    #[test]
    fn complement_keeps_the_model() {
        assert_eq!(Color::rgb(1.0, 0.0, 0.0).complement_keep_model(), Color::rgb(0.0, 1.0, 1.0));
        // Same as turning the hue by 180° for RGB.
        let c = Color::rgb8(204, 102, 51);
        assert_eq!(c.complement_keep_model().to_hex(), c.complement().to_hex());
        assert_eq!(Color::cmyk(0.0, 1.0, 1.0, 0.2).complement_keep_model(), Color::cmyk(1.0, 0.0, 0.0, 0.2));
        assert_eq!(Color::gray(0.3).complement_keep_model(), Color::gray(0.3));
        assert_eq!(Color::gray(0.25).invert_keep_model(), Color::gray(0.75));
        assert!(matches!(Color::cmyk(0.0, 1.0, 1.0, 0.0).invert_keep_model(), Color::Cmyk { .. }));
        assert_eq!(Color::rgb(1.0, 1.0, 0.0).invert_keep_model(), Color::rgb(0.0, 0.0, 1.0));
    }

    #[test]
    fn colours_convert_between_models_and_keep_them() {
        use cms::Model;
        let red = Color::rgb(1.0, 0.0, 0.0);
        assert_eq!(red.in_model(Model::Cmyk), Color::cmyk(0.0, 1.0, 1.0, 0.0));
        assert_eq!(red.in_model(Model::Rgb), red, "already RGB: unchanged");
        assert_eq!(Color::gray(0.25).in_model(Model::Gray), Color::gray(0.25));
        assert_eq!(Color::WHITE.in_model(Model::Gray), Color::gray(0.0));
        assert_eq!(keep_model(Color::cmyk(0.1, 0.2, 0.3, 0.0), red).model(), Model::Cmyk);
        assert_eq!(keep_model(Color::gray(0.5), red).model(), Model::Gray);
        assert_eq!(keep_model(red, Color::rgb(0.0, 0.5, 1.0)), Color::rgb(0.0, 0.5, 1.0));
    }

    #[test]
    fn paint_serde() {
        let p = Paint::solid(Color::rgb8(10, 20, 30));
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"type\":\"solid\""));
        assert_eq!(serde_json::from_str::<Paint>(&s).unwrap(), p);
    }
}
