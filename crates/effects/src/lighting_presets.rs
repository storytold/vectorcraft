//! Original lighting rigs for the publicly documented 3D preset names.
//! Adobe publishes the names, not numeric settings. These values are our own; Standard
//! retains the existing default. A preset replaces the light rig and ambient only.

use serde_json::{Value, json};
use vectorcraft_three_d::{DEFAULT_LIGHT, Light};

#[derive(Clone, Copy, Debug)]
pub struct LightingPreset {
    pub id: &'static str,
    pub label: &'static str,
    rotation: f64,
    height: f64,
    softness: f64,
    ambient: f64,
}

pub const LIGHTING_PRESETS: [LightingPreset; 4] = [
    LightingPreset { id: "standard", label: "Standard", rotation: DEFAULT_LIGHT.0, height: DEFAULT_LIGHT.1, softness: 0.0, ambient: 25.0 },
    LightingPreset { id: "diffuse", label: "Diffuse", rotation: DEFAULT_LIGHT.0, height: DEFAULT_LIGHT.1, softness: 100.0, ambient: 35.0 },
    LightingPreset { id: "topLeft", label: "Top Left", rotation: 135.0, height: 45.0, softness: 0.0, ambient: 25.0 },
    LightingPreset { id: "right", label: "Right", rotation: 0.0, height: 30.0, softness: 0.0, ambient: 25.0 },
];

impl LightingPreset {
    /// The same patch is used by commands and the dialog's preview transaction. Null
    /// `lights` replaces an explicit multi-light rig with this single white light.
    pub fn params(self) -> Value {
        json!({
            "lights": null, "lightColor": "#ffffff", "lightIntensity": 80.0,
            "lightRotation": self.rotation, "lightHeight": self.height,
            "lightSoftness": self.softness, "lightBehind": false, "ambient": self.ambient,
        })
    }

    fn matches(self, light: Light, ambient: f64) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        !light.behind
            && light.color.iter().all(|c| close(*c, 1.0))
            && close(light.intensity, 0.8)
            && close(light.rotation, self.rotation)
            && close(light.height, self.height)
            && close(light.softness, self.softness / 100.0)
            && close(ambient, self.ambient / 100.0)
    }
}

pub fn lighting_preset_params(id: &str) -> Option<Value> {
    LIGHTING_PRESETS.iter().find(|p| p.id == id).map(|p| p.params())
}

/// Identify the effective rig, including an equivalent explicit single light. Names
/// are UI state derived from settings, so edits and older files cannot leave stale names.
pub fn matching_lighting_preset(params: &Value) -> Option<usize> {
    let upgraded = crate::upgrade_params(crate::REVOLVE, params);
    let o = crate::revolve_options(&upgraded);
    let light = if let Some(lights) = &o.lights {
        let mut entries = lights.iter();
        let light = *entries.next()?;
        if entries.next().is_some() {
            return None;
        }
        light
    } else {
        Light {
            rotation: o.light_rotation,
            height: o.light_height,
            behind: o.light_behind,
            color: o.light_color,
            intensity: o.light_intensity,
            softness: o.light_softness,
        }
    };
    LIGHTING_PRESETS.iter().position(|preset| preset.matches(light, o.ambient))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_presets_are_valid_unique_and_standard_preserves_the_default() {
        for (i, preset) in LIGHTING_PRESETS.iter().enumerate() {
            let patch = preset.params();
            crate::lighting::validate(&patch).unwrap();
            assert_eq!(lighting_preset_params(preset.id), Some(patch.clone()));
            assert_eq!(matching_lighting_preset(&patch), Some(i));
            assert!(!patch.as_object().unwrap().contains_key("shade"));
            assert!(!patch.as_object().unwrap().contains_key("rotationX"));
        }
        assert_eq!(matching_lighting_preset(&json!({})), Some(0));
        assert!(lighting_preset_params("unknown").is_none());
        assert_eq!(crate::revolve_options(&LIGHTING_PRESETS[0].params()).light_direction(), crate::revolve_options(&json!({})).light_direction());
    }

    #[test]
    fn lighting_preset_matching_uses_effective_lights_and_detects_custom_edits() {
        let mut p = LIGHTING_PRESETS[3].params();
        p["lightColor"] = json!("#ff0000");
        p["lights"] = json!([{"rotation":0,"height":30}]);
        assert_eq!(matching_lighting_preset(&p), Some(3), "explicit light overrides stale legacy color");
        p["lights"][0]["color"] = json!("#4080ff");
        assert_eq!(matching_lighting_preset(&p), None);
        p["lights"] = json!([{}, {}]);
        assert_eq!(matching_lighting_preset(&p), None);
        for (key, value) in [("ambient", json!(26)), ("lightIntensity", json!(79)), ("lightBehind", json!(true))] {
            let mut p = LIGHTING_PRESETS[0].params();
            p[key] = value;
            assert_eq!(matching_lighting_preset(&p), None, "{key}");
        }
    }
}
