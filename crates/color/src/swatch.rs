//! Swatches and the default swatch set (our own palette, not Adobe's).

use serde::{Deserialize, Serialize};

use crate::cms::Model;
use crate::{Color, Gradient, GradientKind, GradientStop, Paint};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub name: String,
    pub paint: Paint,
    /// Global swatches update every object that uses them.
    #[serde(default)]
    pub global: bool,
    /// Spot colour (prints on its own plate).
    #[serde(default)]
    pub spot: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwatchGroup {
    pub name: String,
    pub swatches: Vec<Swatch>,
}

/// A built-in hex literal (all of them parse; black is only a fallback).
fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::BLACK)
}

fn solid(name: &str, s: &str) -> Swatch {
    Swatch { name: name.into(), paint: Paint::solid(hex(s)), global: false, spot: false }
}

/// The default swatches of a document in colour `model`: [`base_swatches`] with their RGB colours
/// (gradient stops too) as whole-percent CMYK in a CMYK document. Greys stay Gray.
pub fn default_swatches(model: Model) -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let (mut s, mut g) = base_swatches();
    if model == Model::Cmyk {
        let cmyk = |c: &mut Color| {
            if let Color::Rgb { .. } = c {
                let pct = |v: f32| (v.clamp(0.0, 1.0) * 100.0).round() / 100.0;
                let [cc, m, y, k] = c.to_cmyk();
                *c = Color::cmyk(pct(cc), pct(m), pct(y), pct(k));
            }
        };
        for w in s.iter_mut().chain(g.iter_mut().flat_map(|g| g.swatches.iter_mut())) {
            match &mut w.paint {
                Paint::Solid { color, .. } => cmyk(color),
                Paint::Gradient(gp) => gp.gradient.stops.iter_mut().for_each(|st| cmyk(&mut st.color)),
                _ => {}
            }
        }
    }
    (s, g)
}

/// The default document swatches: None, Registration-like black, white, black, a spectrum, greys,
/// gradients. (The composition is ours; the reference app's layout grammar — None first, then
/// specials — is kept.)
fn base_swatches() -> (Vec<Swatch>, Vec<SwatchGroup>) {
    let mut s =
        vec![Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false }, solid("White", "#ffffff"), solid("Black", "#000000")];
    let spectrum = [
        ("Red", "#ed1c24"),
        ("Orange Red", "#f15a24"),
        ("Orange", "#f7931e"),
        ("Amber", "#fbb03b"),
        ("Yellow", "#fcee21"),
        ("Yellow Green", "#d9e021"),
        ("Lime", "#8cc63f"),
        ("Green", "#39b54a"),
        ("Emerald", "#009245"),
        ("Teal", "#006837"),
        ("Aqua", "#22b573"),
        ("Cyan Green", "#00a99d"),
        ("Cyan", "#29abe2"),
        ("Azure", "#0071bc"),
        ("Blue", "#2e3192"),
        ("Indigo", "#1b1464"),
        ("Violet", "#662d91"),
        ("Purple", "#93278f"),
        ("Magenta", "#9e005d"),
        ("Rose", "#d4145a"),
        ("Pink", "#ed1e79"),
        ("Brown", "#c7b299"),
        ("Tan", "#998675"),
        ("Coffee", "#736357"),
        ("Chocolate", "#534741"),
        ("Sand", "#c69c6d"),
        ("Copper", "#a67c52"),
        ("Rust", "#8c6239"),
        ("Walnut", "#754c24"),
        ("Espresso", "#603813"),
    ];
    s.extend(spectrum.iter().map(|(n, h)| solid(n, h)));
    let grays: Vec<Swatch> = (1..=9)
        .rev()
        .map(|i| {
            let k = i as f32 / 10.0;
            let c = Color::gray(k);
            Swatch { name: format!("K={}", (k * 100.0) as u32), paint: Paint::solid(c), global: false, spot: false }
        })
        .collect();
    let grad = |name: &str, a: &str, b: &str, kind: GradientKind| Swatch {
        name: name.into(),
        paint: Paint::Gradient(Box::new(crate::GradientPaint::new(Gradient {
            kind,
            stops: vec![
                GradientStop { offset: 0.0, color: hex(a), opacity: 1.0, midpoint: 0.5 },
                GradientStop { offset: 1.0, color: hex(b), opacity: 1.0, midpoint: 0.5 },
            ],
        }))),
        global: false,
        spot: false,
    };
    s.push(grad("White, Black", "#ffffff", "#000000", GradientKind::Linear));
    s.push(grad("Radial White, Black", "#ffffff", "#000000", GradientKind::Radial));
    s.push(grad("Sunset", "#fbb03b", "#d4145a", GradientKind::Linear));
    s.push(grad("Ocean", "#29abe2", "#2e3192", GradientKind::Linear));
    let groups = vec![
        SwatchGroup { name: "Grays".into(), swatches: grays },
        SwatchGroup {
            name: "Brights".into(),
            swatches: vec![
                solid("Bright Red", "#ff1d25"),
                solid("Bright Yellow", "#ffe600"),
                solid("Bright Green", "#00e676"),
                solid("Bright Blue", "#2979ff"),
                solid("Bright Violet", "#d500f9"),
            ],
        },
    ];
    (s, groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_start_with_none() {
        let (s, g) = default_swatches(Model::Rgb);
        assert!(s[0].paint.is_none());
        assert!(s.len() > 30);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].swatches.len(), 9);
    }

    #[test]
    fn cmyk_defaults_are_cmyk_with_the_same_names() {
        let (rgb, _) = default_swatches(Model::Rgb);
        let (s, g) = default_swatches(Model::Cmyk);
        assert_eq!(s.iter().map(|w| &w.name).collect::<Vec<_>>(), rgb.iter().map(|w| &w.name).collect::<Vec<_>>());
        let colors = s.iter().chain(g.iter().flat_map(|g| &g.swatches)).flat_map(|w| match &w.paint {
            Paint::Solid { color, .. } => vec![*color],
            Paint::Gradient(gp) => gp.gradient.stops.iter().map(|st| st.color).collect(),
            _ => vec![],
        });
        assert!(colors.clone().all(|c| !matches!(c, Color::Rgb { .. })));
        assert!(colors.clone().any(|c| matches!(c, Color::Gray { .. })), "greys stay Gray");
        let white = s.iter().find(|w| w.name == "White").and_then(|w| w.paint.color());
        assert_eq!(white, Some(Color::cmyk(0.0, 0.0, 0.0, 0.0)));
        let black = s.iter().find(|w| w.name == "Black").and_then(|w| w.paint.color());
        assert_eq!(black, Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    }
}
