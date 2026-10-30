use super::catalog::{catalog_index, plugin_info};
use super::{CROP_MARKS, Effect, Map, Node, PIXEL_EFFECTS, RASTERIZE, Value, is_adjustment, is_pathfinder, is_three_d, json, util};

/// Dialog defaults for `id` (`None` for unknown effects).
pub fn default_params(id: &str) -> Option<Value> {
    match catalog_index().get(id) {
        Some(e) => Some(e.defaults.clone()),
        None => plugin_info(id).map(|e| e.defaults),
    }
}

/// `params` layered over the defaults of `id` (unknown keys are kept), in their current form (see
/// [`upgrade_params`]). The 3D effects' earlier `perspective` is read as `perspectiveAngle`, and
/// Scribble's earlier `variation` is read as `overlapVariation` and `spacingVariation`.
pub fn merged_params(id: &str, params: &Value) -> Value {
    let mut out = match catalog_index().get(id).map(|e| &e.defaults) {
        Some(Value::Object(m)) => m.clone(),
        Some(_) => Map::new(),
        None => vectorcraft_plugins::effect::installed(id).map(|p| p.manifest().defaults()).unwrap_or_default(),
    };
    // Upgraded before the defaults are under it: a default angle is no angle given.
    if let Value::Object(p) = upgrade_params(id, params) {
        out.extend(p);
    }
    Value::Object(out)
}

/// `params` for effect `id` in their current form. The 3D effects took Perspective as a 0–100 %
/// strength, `perspective`, before the documented 0°–160° angle, `perspectiveAngle`, and the light
/// as `lightAzimuth` and `lightElevation` before the documented Rotation and Height
/// (`lightRotation`, `lightHeight`, `lightBehind`). Scribble took one `variation` before its
/// separate `overlapVariation` and `spacingVariation`. Earlier values (from a file or a script of
/// then) become the current ones that show exactly the same, unless those are given too, which win.
/// Scribble's angle is also stored above -180° and up to 180° (270° becomes -90°).
pub fn upgrade_params(id: &str, params: &Value) -> Value {
    match params {
        Value::Object(m) => {
            let mut m = m.clone();
            upgrade_perspective(id, &mut m);
            upgrade_light(id, &mut m);
            upgrade_scribble(id, &mut m);
            Value::Object(m)
        }
        other => other.clone(),
    }
}

fn upgrade_light(id: &str, m: &mut Map<String, Value>) {
    if !is_three_d(id) {
        return;
    }
    let (azimuth, elevation) = (m.remove("lightAzimuth"), m.remove("lightElevation"));
    if azimuth.is_none() && elevation.is_none() || m.contains_key("lightRotation") || m.contains_key("lightHeight") {
        return;
    }
    let degrees = |v: Option<Value>, default: f64| v.and_then(|v| v.as_f64()).unwrap_or(default);
    let (rotation, height, behind) = vectorcraft_three_d::light_from_azimuth_elevation(degrees(azimuth, -45.0), degrees(elevation, 45.0));
    m.insert("lightRotation".into(), json!(rotation));
    m.insert("lightHeight".into(), json!(height));
    m.insert("lightBehind".into(), json!(behind));
}

fn upgrade_perspective(id: &str, m: &mut Map<String, Value>) {
    if !is_three_d(id) {
        return;
    }
    let Some(strength) = m.remove("perspective") else { return };
    if !m.contains_key("perspectiveAngle") {
        let angle = vectorcraft_three_d::perspective_from_strength(strength.as_f64().unwrap_or(0.0) / 100.0);
        m.insert("perspectiveAngle".into(), json!(angle));
    }
}

fn upgrade_scribble(id: &str, m: &mut Map<String, Value>) {
    if id != "stylize.scribble" {
        return;
    }
    if let Some(variation) = m.remove("variation") {
        for key in ["overlapVariation", "spacingVariation"] {
            m.entry(key).or_insert_with(|| variation.clone());
        }
    }
    if let Some(angle) = m.get("angle").and_then(Value::as_f64).filter(|a| a.is_finite() && (*a <= -180.0 || *a > 180.0)) {
        let wrapped = angle.rem_euclid(360.0);
        m.insert("angle".into(), json!(if wrapped > 180.0 { wrapped - 360.0 } else { wrapped }));
    }
}

/// A new effect record with default parameters (overridden by `params`).
pub fn new_effect(id: &str, params: &Value) -> Option<Effect> {
    if !catalog_index().contains_key(id) {
        vectorcraft_plugins::effect::installed(id)?;
    }
    Some(Effect { id: id.to_string(), params: merged_params(id, params), visible: true })
}

/// Scale the distance parameters of `e` by `s` (Scale Strokes & Effects; see
/// [`EffectInfo::lengths`]). A distance left at its default is written out scaled; one the
/// effect doesn't have (Outline Stroke's weight following its stroke) stays absent.
pub fn scale_effect(e: &mut Effect, s: f64) {
    let Some(info) = catalog_index().get(e.id.as_str()) else { return };
    let merged = merged_params(&e.id, &e.params);
    let relative = util::flag(&merged, "relative", false);
    for k in info.lengths.keys(relative) {
        let v = util::num(&merged, k, f64::NAN);
        if v.is_finite() {
            if !e.params.is_object() {
                e.params = Value::Object(Map::new());
            }
            e.params[k] = json!(v * s);
        }
    }
}

/// Is `id` a raster (painted) effect?
pub fn is_raster(id: &str) -> bool {
    matches!(id, "stylize.dropShadow" | "stylize.innerGlow" | "stylize.outerGlow" | "stylize.feather" | "blur.gaussian" | RASTERIZE)
        || PIXEL_EFFECTS.contains(&id)
}

/// Does `id` change geometry? (Crop Marks adds art of its own instead, [`crop_marks_art`]; colour
/// adjustments recolour, [`adjust`].) Plug-in effects do, installed or not (a missing plug-in
/// leaves the geometry as it is).
pub fn is_geometry(id: &str) -> bool {
    !is_raster(id)
        && !is_pathfinder(id)
        && !is_adjustment(id)
        && id != CROP_MARKS
        && !is_three_d(id)
        && (catalog_index().contains_key(id) || vectorcraft_plugins::effect::plugin_id(id).is_some())
}

/// Any visible geometry effect in the list?
pub fn has_geometry(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible && is_geometry(&e.id))
}

/// Does `n` have a visible plug-in effect, its own or a fill's or stroke's? Such an effect draws
/// what the plug-in installed under its id draws ([`plugin_revision`]).
pub fn has_plugin_effect(n: &Node) -> bool {
    let plugin = |e: &Effect| e.visible && vectorcraft_plugins::effect::plugin_id(&e.id).is_some();
    n.appearance.effects.iter().any(plugin) || n.appearance.items.iter().any(|i| i.effects().iter().any(plugin))
}

/// The plug-in registry's revision, which changes whenever a plug-in is installed or removed.
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision()
}
