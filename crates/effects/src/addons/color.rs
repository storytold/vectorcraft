//! Color Effects (a built-in add-on): live Exposure, Desaturate, Duotone and Invert Colors, in
//! Effect › Color Effects. They are colour adjustments ([`crate::is_adjustment`]): they recolour what
//! an object paints as the others do, in stack order with them.

use serde_json::{Value, json};

use crate::catalog::{EffectInfo, Lengths};
use crate::util::num;

/// Exposure: brighten or darken in stops of light, with an offset and a gamma.
pub const EXPOSURE: &str = "addon.color.exposure";
/// Desaturate: every colour to the grey of its lightness.
pub const DESATURATE: &str = "addon.color.desaturate";
/// Invert Colors: every colour to its RGB inverse.
pub const INVERT: &str = "addon.color.invert";
pub use super::duotone::DUOTONE;

/// The add-on's effects, in menu order.
pub const EFFECTS: [&str; 4] = [DESATURATE, DUOTONE, EXPOSURE, INVERT];

/// Is `id` one of the add-on's effects?
pub fn is_color_effect(id: &str) -> bool {
    EFFECTS.contains(&id)
}

/// The add-on's catalogue entries.
pub(crate) fn catalog() -> Vec<EffectInfo> {
    const MENU: &[&str] = &["Effect", "Color Effects"];
    let e = |id, label, params, defaults| EffectInfo { id, label, menu: MENU, params, defaults, raster: false, lengths: Lengths::default() };
    vec![
        e(DESATURATE, "Desaturate", "{} turns every colour the object paints to the grey of its lightness (each colour keeps its model)", json!({})),
        e(DUOTONE, "Duotone…", super::duotone::PARAMS, super::duotone::defaults()),
        e(
            EXPOSURE,
            "Exposure…",
            "{exposure: stops -20..20 (0; mostly the highlights: each stop doubles or halves the light), offset: -0.5..0.5 (0; lifts or sinks the shadows and midtones), gamma: 0.1..10 (1; above 1 lightens the midtones)} recolours as Brightness/Contrast does, in linear light",
            json!({"exposure": 0.0, "offset": 0.0, "gamma": 1.0}),
        ),
        e(INVERT, "Invert Colors", "{} turns every colour the object paints to its RGB inverse (each colour keeps its model)", json!({})),
    ]
}

/// One of the add-on's adjustments, its parameters normalised.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Adjust {
    /// Linear light times `gain`, plus `offset`, to the power 1 / `gamma`.
    Exposure {
        gain: f32,
        offset: f32,
        gamma: f32,
    },
    Desaturate,
    Invert,
    /// Duotone ([`super::duotone`]).
    Duotone(super::duotone::Duotone),
}

impl Adjust {
    pub(crate) fn parse(id: &str, p: &Value) -> Option<Self> {
        Some(match id {
            EXPOSURE => Adjust::Exposure {
                gain: 2f64.powf(num(p, "exposure", 0.0).clamp(-20.0, 20.0)) as f32,
                offset: num(p, "offset", 0.0).clamp(-0.5, 0.5) as f32,
                gamma: num(p, "gamma", 1.0).clamp(0.1, 10.0) as f32,
            },
            DESATURATE => Adjust::Desaturate,
            INVERT => Adjust::Invert,
            DUOTONE => Adjust::Duotone(super::duotone::Duotone::parse(p)?),
            _ => return None,
        })
    }

    pub(crate) fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Adjust::Exposure { gain, offset, gamma } => rgb.map(|v| to_srgb((to_linear(v) * gain + offset).max(0.0).powf(1.0 / gamma))),
            Adjust::Desaturate => {
                let [r, g, b] = rgb;
                [(r.max(g).max(b) + r.min(g).min(b)) / 2.0; 3]
            }
            Adjust::Invert => rgb.map(|v| 1.0 - v),
            Adjust::Duotone(d) => d.apply_rgb(rgb),
        }
    }
}

fn to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_map;
    use vectorcraft_doc::Effect;

    fn map(id: &str, params: Value) -> crate::ColorMap {
        color_map(&[Effect { id: id.into(), params, visible: true }]).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn the_effects_are_colour_adjustments_in_their_own_menu() {
        for id in EFFECTS {
            assert!(crate::is_adjustment(id), "{id}");
            assert_eq!(crate::effect_info(id).unwrap().menu, ["Effect", "Color Effects"]);
        }
    }

    #[test]
    fn exposure_works_in_stops_of_linear_light() {
        let grey = [0.5; 3];
        assert!(close(map(EXPOSURE, json!({})).apply_rgb(grey), grey), "neutral by default");
        // One stop doubles the light: sRGB 0.5 (linear 0.214) becomes linear 0.428, sRGB 0.686.
        assert!(close(map(EXPOSURE, json!({"exposure": 1})).apply_rgb(grey), [0.686; 3]));
        assert!(close(map(EXPOSURE, json!({"exposure": -20})).apply_rgb([1.0; 3]), [0.0; 3]));
        assert!(map(EXPOSURE, json!({"offset": 0.1})).apply_rgb([0.0; 3])[0] > 0.3, "the offset lifts the shadows");
        assert!(map(EXPOSURE, json!({"gamma": 2})).apply_rgb(grey)[0] > 0.6);
        assert!(close(map(EXPOSURE, json!({"exposure": 1e300, "gamma": 0})).apply_rgb(grey), [1.0; 3]), "out of range values are clamped");
    }

    #[test]
    fn desaturate_and_invert() {
        assert!(close(map(DESATURATE, json!({})).apply_rgb([1.0, 0.0, 0.0]), [0.5; 3]));
        assert!(close(map(DESATURATE, json!({})).apply_rgb([1.0, 1.0, 0.0]), [0.5; 3]), "lightness is kept, not luminance");
        assert!(close(map(INVERT, json!({})).apply_rgb([1.0, 0.25, 0.0]), [0.0, 0.75, 1.0]));
        let both = map(INVERT, json!({})).then(&map(INVERT, json!({})));
        assert!(close(both.apply_rgb([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]));
    }
}
