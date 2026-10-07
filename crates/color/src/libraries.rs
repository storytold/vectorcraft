//! Swatch libraries: read-only sets of swatches the library panel opens and adds from. Every
//! built-in library is computed here (names and palettes are our own; nothing is copied from any
//! other application or colour system), so the colours are exact and the same on every run.
//!
//! Colours are designed in OKLCH (perceptual lightness, chroma and hue), mapped into sRGB by
//! lowering chroma until they fit, and rounded to 8-bit RGB.

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint, Swatch, SwatchGroup};

/// A library: its name, its ungrouped swatches and its colour groups.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SwatchLibrary {
    pub name: String,
    #[serde(default)]
    pub swatches: Vec<Swatch>,
    #[serde(default)]
    pub groups: Vec<SwatchGroup>,
}

impl SwatchLibrary {
    /// Every swatch: the ungrouped ones first, then each group's in order.
    pub fn iter(&self) -> impl Iterator<Item = &Swatch> {
        self.swatches.iter().chain(self.groups.iter().flat_map(|g| g.swatches.iter()))
    }
    /// The number of swatches.
    pub fn len(&self) -> usize {
        self.swatches.len() + self.groups.iter().map(|g| g.swatches.len()).sum::<usize>()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn swatch(&self, name: &str) -> Option<&Swatch> {
        self.iter().find(|s| s.name == name)
    }
    pub fn group(&self, name: &str) -> Option<&SwatchGroup> {
        self.groups.iter().find(|g| g.name == name)
    }
}

/// A built-in library: a stable id, its menu name and the function that computes it.
pub struct BuiltinLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn() -> (Vec<Swatch>, Vec<SwatchGroup>),
}

macro_rules! lib {
    ($id:literal, $name:literal, $make:expr) => {
        BuiltinLibrary { id: $id, name: $name, make: $make }
    };
}

/// The built-in swatch libraries, in menu order.
pub const SWATCH_LIBRARIES: &[BuiltinLibrary] = &[
    lib!("web-safe-216", "Web Safe 216", web_safe),
    lib!("grays-neutrals", "Grays and Neutrals", grays_neutrals),
    lib!("earth-tones", "Earth Tones", earth_tones),
    lib!("skin-tone-ramps", "Skin Tone Ramps", skin_tones),
    lib!("pastels", "Pastels", pastels),
    lib!("brights", "Brights", brights),
    lib!("metallic-gradients", "Metallic Gradients", metallic),
    lib!("perceptual-scales", "Perceptual Scales", perceptual),
    lib!("harmony-sets", "Harmony Sets", harmony_sets),
];

/// The built-in gradient libraries (Window → Swatch Libraries → Gradients), in menu order.
pub const GRADIENT_LIBRARIES: &[BuiltinLibrary] = &[
    lib!("spectrum-gradients", "Spectrum Gradients", spectrum_gradients),
    lib!("sky-gradients", "Sky Gradients", sky_gradients),
    lib!("neutral-gradients", "Neutral Gradients", neutral_gradients),
    lib!("duotone-gradients", "Duotone Gradients", duotone_gradients),
    lib!("radial-glows", "Radial Glows", radial_glows),
];

/// Every built-in library: the swatch libraries, then the gradient libraries.
fn builtins() -> impl Iterator<Item = &'static BuiltinLibrary> {
    SWATCH_LIBRARIES.iter().chain(GRADIENT_LIBRARIES)
}

/// Built-in library `id` (swatch or gradient library), computed once.
pub fn builtin_library(id: &str) -> Option<Arc<SwatchLibrary>> {
    static LIBS: OnceLock<Vec<Arc<SwatchLibrary>>> = OnceLock::new();
    let libs = LIBS.get_or_init(|| {
        builtins()
            .map(|b| {
                let (swatches, groups) = (b.make)();
                Arc::new(SwatchLibrary { name: b.name.into(), swatches, groups })
            })
            .collect()
    });
    builtins().position(|b| b.id == id).map(|i| libs[i].clone())
}

// ---------- colour construction ----------

/// sRGB (0..=1, gamma encoded) of OKLab `(l, a, b)`, unclamped.
fn oklab_to_srgb(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let lin = [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3,
    ];
    lin.map(|v| if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 })
}

/// OKLab `[l, a, b]` of a colour's display RGB.
pub fn to_oklab(c: &Color) -> [f64; 3] {
    let [r, g, b] = c.to_rgb_uncalibrated().map(|v| {
        let v = f64::from(v);
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    });
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Interpolation of `a` → `b` at `t` in OKLab, a perceptually uniform space (gradients with
/// perceptual interpolation): mid-tones stay as light and as saturated as the eye expects, where
/// mixing in RGB goes muddy and dark. The result is an RGB colour clamped to sRGB.
pub fn mix_perceptual(a: &Color, b: &Color, t: f32) -> Color {
    let (p, q) = (to_oklab(a), to_oklab(b));
    let t = f64::from(t);
    let [r, g, bl] = oklab_to_srgb(p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t);
    let c = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 };
    Color::rgb(c(r), c(g), c(bl))
}

/// The 8-bit sRGB colour of OKLCH `(l, c, h°)`, with the chroma lowered until it fits in sRGB.
fn oklch(l: f64, c: f64, h: f64) -> Color {
    let (sin, cos) = h.to_radians().sin_cos();
    let at = |c: f64| oklab_to_srgb(l, c * cos, c * sin);
    let fits = |rgb: [f64; 3]| rgb.iter().all(|v| (-1e-4..=1.0001).contains(v));
    let rgb = if fits(at(c)) {
        at(c)
    } else {
        // Bisect the largest chroma in gamut.
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if fits(at(mid)) { lo = mid } else { hi = mid }
        }
        at(lo)
    };
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::rgb8(q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

/// OKLab interpolation of `a` → `b` at `t`, as an 8-bit sRGB colour.
fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> Color {
    let p = [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
    let (c, h) = (p[1].hypot(p[2]), p[2].atan2(p[1]).to_degrees());
    oklch(p[0], c, h)
}

/// OKLab `[l, a, b]` of OKLCH `(l, c, h°)`.
fn lab(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (sin, cos) = h.to_radians().sin_cos();
    [l, c * cos, c * sin]
}

fn solid(name: impl Into<String>, color: Color) -> Swatch {
    Swatch { name: name.into(), paint: Paint::solid(color), global: false, spot: false }
}

fn group(name: &str, swatches: Vec<Swatch>) -> SwatchGroup {
    SwatchGroup { name: name.into(), swatches }
}

/// A gradient swatch through `stops` of (offset, colour, opacity).
pub(crate) fn gradient(name: &str, kind: GradientKind, stops: &[(f32, Color, f32)]) -> Swatch {
    let stops = stops.iter().map(|&(offset, color, opacity)| GradientStop { opacity, ..GradientStop::new(offset, color) }).collect();
    Swatch { name: name.into(), paint: Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(kind, stops)))), global: false, spot: false }
}

/// `n` steps of OKLCH lightness from `l0` to `l1`, chroma `c(t)` and hue `h(t)` (`t` in 0..=1),
/// named "`name` 1"…
fn ramp(name: &str, n: usize, (l0, l1): (f64, f64), c: impl Fn(f64) -> f64, h: impl Fn(f64) -> f64) -> SwatchGroup {
    let swatches = (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1).max(1) as f64;
            solid(format!("{name} {}", i + 1), oklch(l0 + (l1 - l0) * t, c(t), h(t)))
        })
        .collect();
    group(name, swatches)
}

/// Twelve named OKLCH hues around the wheel.
const HUES: [(&str, f64); 12] = [
    ("Red", 25.0),
    ("Orange", 55.0),
    ("Amber", 75.0),
    ("Yellow", 100.0),
    ("Lime", 125.0),
    ("Green", 145.0),
    ("Teal", 180.0),
    ("Cyan", 205.0),
    ("Azure", 235.0),
    ("Blue", 265.0),
    ("Violet", 295.0),
    ("Magenta", 330.0),
];

/// One group of the twelve hues at OKLCH lightness `l` and chroma `c`, named "`prefix` Red"…
fn hue_row(prefix: &str, l: f64, c: f64) -> SwatchGroup {
    group(prefix, HUES.iter().map(|(n, h)| solid(format!("{prefix} {n}"), oklch(l, c, *h))).collect())
}

// ---------- the libraries ----------

/// The 216 colours whose channels are multiples of 0x33, named by their hex value.
fn web_safe() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let mut s = Vec::with_capacity(216);
    for r in 0..6u8 {
        for g in 0..6u8 {
            for b in 0..6u8 {
                let c = Color::rgb8(r * 0x33, g * 0x33, b * 0x33);
                s.push(solid(c.to_hex().to_uppercase(), c));
            }
        }
    }
    (s, vec![])
}

fn grays_neutrals() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let grays = (0..=20).map(|i| solid(format!("K={}", i * 5), Color::gray(i as f32 / 20.0))).collect();
    let tinted = |name: &str, c: f64, h: f64| ramp(name, 9, (0.95, 0.25), |_| c, |_| h);
    let neutrals = [
        ("Ivory", 0.96, 0.02, 95.0),
        ("Linen", 0.92, 0.02, 70.0),
        ("Oat", 0.86, 0.035, 80.0),
        ("Greige", 0.72, 0.02, 75.0),
        ("Stone", 0.66, 0.015, 95.0),
        ("Taupe", 0.56, 0.03, 50.0),
        ("Slate", 0.48, 0.025, 250.0),
        ("Charcoal", 0.30, 0.01, 260.0),
    ];
    let neutrals = neutrals.iter().map(|&(n, l, c, h)| solid(n, oklch(l, c, h))).collect();
    let groups = vec![group("Grays", grays), tinted("Warm Gray", 0.012, 70.0), tinted("Cool Gray", 0.015, 250.0), group("Neutrals", neutrals)];
    (vec![], groups)
}

fn earth_tones() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let families = [
        ("Clay", 0.10, 40.0),
        ("Terracotta", 0.13, 35.0),
        ("Ochre", 0.11, 80.0),
        ("Umber", 0.06, 60.0),
        ("Sand", 0.05, 85.0),
        ("Olive", 0.08, 115.0),
        ("Moss", 0.07, 130.0),
    ];
    // Chroma peaks mid-ramp, as earth pigments do.
    let groups = families.iter().map(|&(n, c, h)| ramp(n, 6, (0.86, 0.34), |t| c * (0.6 + 0.4 * (1.0 - (2.0 * t - 1.0).powi(2))), |_| h)).collect();
    (vec![], groups)
}

fn skin_tones() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let ramps = [("Rosy", 0.075, 35.0), ("Neutral", 0.07, 55.0), ("Golden", 0.08, 70.0), ("Olive", 0.06, 85.0)];
    // Light tones are paler; deep tones keep warmth but lose a little chroma.
    let groups = ramps.iter().map(|&(n, c, h)| ramp(n, 8, (0.93, 0.32), |t| c * (0.45 + 0.75 * t - 0.35 * t * t), |t| h - 8.0 * t)).collect();
    (vec![], groups)
}

fn pastels() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    (vec![], vec![hue_row("Pastel", 0.91, 0.055), hue_row("Soft", 0.84, 0.085)])
}

fn brights() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    // Chroma beyond sRGB: each hue lands on the gamut edge at that lightness.
    (vec![], vec![hue_row("Vivid", 0.72, 0.4), hue_row("Deep", 0.52, 0.4)])
}

fn metallic() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let metals: [(&str, f64, f64, f64); 10] = [
        ("Gold", 0.78, 0.12, 85.0),
        ("Silver", 0.80, 0.005, 250.0),
        ("Bronze", 0.62, 0.09, 65.0),
        ("Copper", 0.64, 0.12, 45.0),
        ("Brass", 0.76, 0.10, 95.0),
        ("Chrome", 0.72, 0.012, 230.0),
        ("Rose Gold", 0.76, 0.07, 30.0),
        ("Gunmetal", 0.45, 0.015, 250.0),
        ("Platinum", 0.86, 0.008, 90.0),
        ("Pewter", 0.60, 0.01, 200.0),
    ];
    // Light, base, shadow, highlight, base: the banding of polished metal.
    let bands: [(f32, f64); 5] = [(0.0, 0.16), (0.3, 0.0), (0.5, -0.2), (0.72, 0.12), (1.0, -0.06)];
    let s = metals
        .iter()
        .map(|&(n, l, c, h)| {
            let stops: Vec<(f32, Color, f32)> = bands.iter().map(|&(at, dl)| (at, oklch((l + dl).clamp(0.05, 0.98), c, h), 1.0)).collect();
            gradient(n, GradientKind::Linear, &stops)
        })
        .collect();
    (s, vec![])
}

/// OKLCH `(l, c, h°)` anchors a scale passes through.
type Anchors = &'static [(f64, f64, f64)];

/// A linear gradient through evenly spaced, opaque `colors`.
fn even(name: &str, kind: GradientKind, colors: &[Color]) -> Swatch {
    let last = (colors.len() - 1).max(1) as f32;
    let stops: Vec<(f32, Color, f32)> = colors.iter().enumerate().map(|(i, c)| (i as f32 / last, *c, 1.0)).collect();
    gradient(name, kind, &stops)
}

/// OKLCH `(l, c, h°)` colours.
fn lch(points: &[(f64, f64, f64)]) -> Vec<Color> {
    points.iter().map(|&(l, c, h)| oklch(l, c, h)).collect()
}

/// Full-hue sweeps and hue-neighbour blends.
fn spectrum_gradients() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let wheel: Vec<Color> = (0..=6).map(|i| oklch(0.72, 0.4, 25.0 + i as f64 * 60.0)).collect();
    let mut s = vec![
        even("Spectrum", GradientKind::Linear, &wheel),
        even("Soft Spectrum", GradientKind::Linear, &lch(&[(0.88, 0.08, 25.0), (0.9, 0.08, 145.0), (0.86, 0.08, 265.0), (0.88, 0.08, 25.0)])),
    ];
    // Each hue to the one two steps round the wheel.
    s.extend((0..HUES.len()).map(|i| {
        let ((a, ha), (b, hb)) = (HUES[i], HUES[(i + 2) % HUES.len()]);
        even(&format!("{a} to {b}"), GradientKind::Linear, &[oklch(0.7, 0.4, ha), oklch(0.7, 0.4, hb)])
    }));
    (s, vec![])
}

fn sky_gradients() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let skies: [(&str, Anchors); 6] = [
        ("Dawn", &[(0.45, 0.08, 280.0), (0.72, 0.12, 350.0), (0.9, 0.09, 75.0)]),
        ("Noon", &[(0.55, 0.13, 250.0), (0.78, 0.09, 235.0), (0.95, 0.03, 220.0)]),
        ("Sunset", &[(0.35, 0.09, 300.0), (0.62, 0.18, 20.0), (0.85, 0.15, 70.0)]),
        ("Dusk", &[(0.25, 0.07, 275.0), (0.48, 0.1, 310.0), (0.7, 0.1, 30.0)]),
        ("Night", &[(0.15, 0.04, 270.0), (0.3, 0.07, 265.0)]),
        ("Overcast", &[(0.62, 0.015, 240.0), (0.86, 0.01, 230.0)]),
    ];
    (skies.iter().map(|(n, p)| even(n, GradientKind::Linear, &lch(p))).collect(), vec![])
}

fn neutral_gradients() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let (white, black) = (Color::gray(0.0), Color::gray(1.0));
    let s = vec![
        even("White to Black", GradientKind::Linear, &[white, black]),
        even("Light Grays", GradientKind::Linear, &[white, Color::gray(0.4)]),
        even("Dark Grays", GradientKind::Linear, &[Color::gray(0.6), black]),
        even("Warm Grays", GradientKind::Linear, &lch(&[(0.95, 0.012, 70.0), (0.3, 0.012, 70.0)])),
        even("Cool Grays", GradientKind::Linear, &lch(&[(0.95, 0.015, 250.0), (0.3, 0.015, 250.0)])),
        gradient("Fade to Transparent", GradientKind::Linear, &[(0.0, black, 1.0), (1.0, black, 0.0)]),
        gradient("White Fade", GradientKind::Linear, &[(0.0, white, 1.0), (1.0, white, 0.0)]),
        gradient("Vignette", GradientKind::Radial, &[(0.0, black, 0.0), (0.6, black, 0.0), (1.0, black, 0.7)]),
    ];
    (s, vec![])
}

/// Pairs of hues from opposite sides of the wheel, at different lightness.
fn duotone_gradients() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let pairs = [
        ("Coral", 0.7, 30.0, "Navy", 0.32, 265.0),
        ("Plum", 0.38, 330.0, "Lime", 0.88, 125.0),
        ("Teal", 0.5, 190.0, "Gold", 0.85, 90.0),
        ("Indigo", 0.3, 280.0, "Peach", 0.86, 55.0),
        ("Forest", 0.35, 150.0, "Rose", 0.8, 0.0),
    ];
    let s = pairs
        .iter()
        .map(|&(a, la, ha, b, lb, hb)| even(&format!("{a} and {b}"), GradientKind::Linear, &[oklch(la, 0.15, ha), oklch(lb, 0.15, hb)]))
        .collect();
    (s, vec![])
}

/// A bright centre fading to a transparent edge, per hue.
fn radial_glows() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let s = HUES
        .iter()
        .map(|(n, h)| {
            let c = oklch(0.78, 0.2, *h);
            gradient(&format!("{n} Glow"), GradientKind::Radial, &[(0.0, Color::WHITE, 1.0), (0.25, c, 1.0), (1.0, c, 0.0)])
        })
        .collect();
    (s, vec![])
}

/// The sequential scales of [`perceptual`]: name and anchors of rising lightness.
const SEQUENTIAL: [(&str, Anchors); 5] = [
    ("Ember", &[(0.22, 0.10, 300.0), (0.55, 0.18, 30.0), (0.93, 0.15, 100.0)]),
    ("Lagoon", &[(0.25, 0.07, 265.0), (0.60, 0.12, 200.0), (0.93, 0.12, 130.0)]),
    ("Dusk", &[(0.20, 0.08, 280.0), (0.60, 0.15, 330.0), (0.92, 0.07, 60.0)]),
    ("Moss", &[(0.25, 0.05, 150.0), (0.92, 0.07, 110.0)]),
    ("Ink", &[(0.20, 0.02, 260.0), (0.96, 0.01, 260.0)]),
];

/// `n` colours along OKLab segments between `anchors`.
fn scale(name: &str, n: usize, anchors: &[(f64, f64, f64)]) -> SwatchGroup {
    let pts: Vec<[f64; 3]> = anchors.iter().map(|&(l, c, h)| lab(l, c, h)).collect();
    let segs = (pts.len() - 1) as f64;
    let swatches = (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64 * segs;
            let k = (t.floor() as usize).min(pts.len() - 2);
            solid(format!("{name} {}", i + 1), mix(pts[k], pts[k + 1], t - k as f64))
        })
        .collect();
    group(name, swatches)
}

/// Scales whose lightness rises evenly (sequential) plus two diverging ones.
fn perceptual() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let mut groups: Vec<SwatchGroup> = SEQUENTIAL.iter().map(|(n, a)| scale(n, 9, a)).collect();
    groups.push(scale("Clay to Teal", 9, &[(0.45, 0.12, 40.0), (0.96, 0.01, 80.0), (0.45, 0.09, 200.0)]));
    groups.push(scale("Plum to Olive", 9, &[(0.40, 0.11, 330.0), (0.96, 0.01, 90.0), (0.45, 0.09, 120.0)]));
    (vec![], groups)
}

/// Colour-wheel harmonies of a few base colours (the Color Guide's rules).
fn harmony_sets() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    use crate::harmony::Harmony;
    let bases = [("Coral", Color::from_hsb(8.0, 0.68, 0.94)), ("Teal", Color::from_hsb(176.0, 0.72, 0.62))];
    let rules = [Harmony::Complementary, Harmony::SplitComplementary, Harmony::Analogous, Harmony::Triad, Harmony::Tetrad, Harmony::Pentagram];
    let mut groups = vec![];
    for (bn, base) in bases {
        for rule in rules {
            let name = format!("{} {bn}", rule.label());
            // Rounded to 8 bits like the other libraries.
            let swatches = rule
                .apply(base)
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    let [r, g, b, _] = c.to_rgba8(1.0);
                    solid(format!("{name} {}", i + 1), Color::rgb8(r, g, b))
                })
                .collect();
            groups.push(SwatchGroup { name, swatches });
        }
    }
    (vec![], groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Arc<SwatchLibrary>> {
        builtins().map(|b| builtin_library(b.id).unwrap()).collect()
    }

    /// Every component of every colour (solid or gradient stop).
    fn components(lib: &SwatchLibrary) -> Vec<f32> {
        let comps = |c: &Color| match *c {
            Color::Rgb { r, g, b } => vec![r, g, b],
            Color::Cmyk { c, m, y, k } => vec![c, m, y, k],
            Color::Gray { k } => vec![k],
            // Normalized to 0..1 like the others.
            Color::Lab { l, a, b } => vec![l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0],
        };
        lib.iter()
            .flat_map(|s| match &s.paint {
                Paint::Solid { color, .. } => comps(color),
                Paint::Gradient(g) => g.gradient.stops.iter().flat_map(|st| comps(&st.color)).collect(),
                _ => vec![],
            })
            .collect()
    }

    #[test]
    fn libraries_are_deterministic_unique_and_in_range() {
        let mut ids = std::collections::HashSet::new();
        for (b, lib) in builtins().zip(all()) {
            assert!(ids.insert(b.id), "duplicate id {}", b.id);
            assert_eq!(lib.name, b.name);
            assert!(lib.len() >= 5, "{} has {}", b.id, lib.len());
            // Recomputing gives the same library.
            let (s, g) = (b.make)();
            assert_eq!((s, g), (lib.swatches.clone(), lib.groups.clone()), "{} is deterministic", b.id);
            // Swatch and group names share one namespace, as in a document.
            let mut names = std::collections::HashSet::new();
            for n in lib.iter().map(|s| &s.name).chain(lib.groups.iter().map(|g| &g.name)) {
                assert!(names.insert(n.clone()), "{}: duplicate name {n}", b.id);
            }
            assert!(components(&lib).iter().all(|v| (0.0..=1.0).contains(v)), "{} out of range", b.id);
            assert!(lib.iter().all(|s| !s.paint.is_none() && !s.global && !s.spot));
        }
    }

    #[test]
    fn web_safe_has_the_216_web_colours() {
        let lib = builtin_library("web-safe-216").unwrap();
        assert_eq!(lib.len(), 216);
        let mut hex: Vec<String> = lib.iter().map(|s| s.paint.color().unwrap().to_hex()).collect();
        hex.dedup();
        assert_eq!(hex.len(), 216);
        assert!(lib.iter().all(|s| s.paint.color().unwrap().to_rgba8(1.0)[..3].iter().all(|v| v % 0x33 == 0)));
        assert_eq!(lib.swatches[0].name, "#000000");
        assert_eq!(lib.swatches[215].name, "#FFFFFF");
    }

    #[test]
    fn sequential_scales_rise_in_lightness() {
        let lib = builtin_library("perceptual-scales").unwrap();
        for (name, _) in SEQUENTIAL {
            let l: Vec<f64> = lib.group(name).unwrap().swatches.iter().map(|s| to_oklab(&s.paint.color().unwrap())[0]).collect();
            assert!(l.windows(2).all(|w| w[1] > w[0] + 0.03), "{name}: {l:?}");
        }
    }

    #[test]
    fn oklch_round_trips_in_gamut_colours() {
        let c = oklch(0.7, 0.1, 150.0);
        let [l, a, b] = to_oklab(&c);
        assert!((l - 0.7).abs() < 0.01 && (a.hypot(b) - 0.1).abs() < 0.01, "{l} {a} {b}");
        // Out-of-gamut chroma is clipped to the gamut edge, keeping lightness.
        let v = oklch(0.72, 0.4, 25.0);
        assert!((to_oklab(&v)[0] - 0.72).abs() < 0.02);
    }

    #[test]
    fn gradient_libraries_have_sorted_stops() {
        for b in GRADIENT_LIBRARIES {
            let lib = builtin_library(b.id).unwrap();
            assert!(lib.iter().all(|s| matches!(s.paint, Paint::Gradient(_))), "{} holds gradients only", b.id);
        }
        for lib in all() {
            for s in lib.iter() {
                if let Paint::Gradient(g) = &s.paint {
                    let st = &g.gradient.stops;
                    assert!(st.len() >= 2 && st.windows(2).all(|w| w[0].offset <= w[1].offset), "{}", s.name);
                    assert!(st.iter().all(|s| (0.0..=1.0).contains(&s.offset) && (0.0..=1.0).contains(&s.opacity)));
                }
            }
        }
    }
}
