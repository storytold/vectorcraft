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
/// Scribble's angle is also stored above -180° and up to 180° (270° becomes -90°), and a Warp's
/// map `version` given as a whole float is stored as an integer.
pub fn upgrade_params(id: &str, params: &Value) -> Value {
    match params {
        Value::Object(m) => {
            let mut m = m.clone();
            upgrade_perspective(id, &mut m);
            upgrade_light(id, &mut m);
            upgrade_scribble(id, &mut m);
            upgrade_warp_version(id, &mut m);
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

/// A Warp's map `version` given as a whole float (`1.0`) is stored as the integer.
fn upgrade_warp_version(id: &str, m: &mut Map<String, Value>) {
    if !id.starts_with("warp.") {
        return;
    }
    if let Some(v) = m.get_mut("version")
        && !v.is_u64()
        && let Some(n) = crate::warp::warp_version(v)
    {
        *v = json!(n);
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
    let mut params = merged_params(id, params);
    // A new Warp takes its style's Bézier net unless a map is given; the catalog's defaults carry
    // no version, so warps saved without one keep the earlier map.
    if id.starts_with("warp.")
        && let Value::Object(m) = &mut params
    {
        m.entry("version").or_insert(json!(crate::warp::WARP_NET));
    }
    Some(Effect { id: id.to_string(), params, visible: true })
}

/// Checks an effect record that a painted live-effect plug-in returns on a piece, or on one of its
/// fills or strokes (`row`) ([`vectorcraft_plugins::painted::CheckEffect`]): a built-in effect other
/// than a 3D effect, a raster effect only on a fill or stroke, and parameters that effect takes. A
/// parameter is one its defaults or its parameter documentation name, of the type its default has,
/// within the range its documentation gives ([`crate::catalog::any_documented_range`]); a texture is one as `effect.list`
/// gives it, and an SVG filter's XML one drawable filter. A plug-in effect, the plug-in itself
/// included, is refused, so the effects a plug-in returns never run a plug-in.
pub fn check_returned_effect(e: &Effect, row: bool) -> Result<(), String> {
    if e.id.starts_with(vectorcraft_plugins::effect::PREFIX) {
        return Err("plug-in effects are not allowed in returned effects".into());
    }
    let Some(info) = catalog_index().get(e.id.as_str()) else {
        return Err(format!("unknown effect {:?}", e.id));
    };
    if is_three_d(&e.id) {
        return Err("3D effects are not allowed in returned effects".into());
    }
    if row && !is_raster(&e.id) {
        return Err(format!("{:?} is not a raster effect; a fill or stroke takes raster effects only", e.id));
    }
    let given = match &e.params {
        Value::Null => return Ok(()),
        Value::Object(m) => m,
        _ => return Err("`params` must be an object".into()),
    };
    let defaults = info.defaults.as_object();
    for (key, v) in given {
        let default = defaults.and_then(|d| d.get(key));
        if default.is_none() && !documents_key(info.params, key) {
            return Err(format!("`{key}` is not a parameter of {:?}", e.id));
        }
        let kind = |v: &Value| std::mem::discriminant(v);
        if let Some(d) = default
            && !d.is_null()
            && kind(d) != kind(v)
        {
            return Err(format!("`{key}` must be of the type its default {d} has"));
        }
        let range = crate::catalog::any_documented_range(&e.id, key);
        if let (Some(r), Some(x)) = (range, v.as_f64())
            && !r.contains(&x)
        {
            return Err(format!("`{key}` must be from {} to {}", r.start(), r.end()));
        }
    }
    if let Some(t) = given.get(crate::pixel::TEXTURE_IMAGE)
        && t.as_str().is_none_or(|t| crate::pixel::texture(t).is_none())
    {
        return Err("`textureImage` must be a texture as effect.list gives it".into());
    }
    check_returned_values(&e.id, given)
}

/// The values in `given`, the parameters of built-in effect `id` that a painted live-effect plug-in
/// returned, that `effect.apply` also checks: an SVG filter's XML, `scale`, `origin`, `axes` and
/// `name`; a Warp's `version`; a `color` the effect can draw; a Levels or Curves `channel`; a Radial
/// Blur's center; Pathfinder Options; and the type of each parameter without a default.
fn check_returned_values(id: &str, given: &Map<String, Value>) -> Result<(), String> {
    let number = |key: &str| given.get(key).is_some_and(|v| !v.is_number());
    let switch = |key: &str| given.get(key).is_some_and(|v| !v.is_boolean());
    if id == crate::SVG_FILTER {
        if let Some(xml) = given.get("xml") {
            crate::svg_filter::check(xml.as_str().ok_or("`xml` must be a string")?)?;
        }
        if given.get("scale").is_some_and(|s| !s.as_f64().is_some_and(|k| (0.001..=1000.0).contains(&k))) {
            return Err("`scale` must be a number from 0.001 to 1000".into());
        }
        if given
            .get("origin")
            .is_some_and(|o| !o.as_array().is_some_and(|o| o.len() == 2 && o.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))))
        {
            return Err("`origin` must be [x, y] in points".into());
        }
        if given.get("axes").is_some_and(|a| crate::svg_filter::valid_axes(a).is_none()) {
            return Err("`axes` must be [a, b, c, d], each from -1000 to 1000, with a·d - b·c at least 0.001 from 0".into());
        }
        if given.contains_key("name") && !given.contains_key("xml") {
            return Err("an SVG filter is named by its id: give `xml` with `name`".into());
        }
    }
    if id.starts_with("warp.") && given.get("version").is_some_and(|v| crate::warp_version(v).is_none()) {
        return Err(format!("`version` must be 0 or {}", crate::WARP_NET));
    }
    let takes_color = catalog_index().get(id).is_some_and(|info| info.defaults.get("color").is_some_and(Value::is_string));
    if takes_color && given.get("color").is_some_and(|v| !is_color(v)) {
        return Err("`color` must be a #rrggbb color".into());
    }
    if matches!(id, "adjust.levels" | "adjust.curves") && given.get("channel").is_some_and(|v| !v.as_str().is_some_and(crate::is_channel)) {
        return Err(format!("`channel` must be one of {}", crate::CHANNELS.join(", ")));
    }
    if id == "blur.radial" {
        for key in ["centerX", "centerY"] {
            if given.get(key).is_some_and(|v| !v.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x))) {
                return Err(format!("`{key}` must be a number from 0 to 1"));
            }
        }
    }
    if crate::takes_pathfinder_options(id) {
        if given.get("precision").is_some_and(|v| !v.as_f64().is_some_and(|p| crate::PATHFINDER_EFFECT_PRECISION.contains(&p))) {
            return Err("precision must be from 0.001 to 10 pt".into());
        }
        if let Some(key) = ["removeRedundantPoints", "removeUnpainted"].into_iter().find(|k| switch(k)) {
            return Err(format!("`{key}` must be true or false"));
        }
    }
    let numbers: &[&str] = match id {
        "stylize.scribble" => &["variation"],
        "path.outlineStroke" => &["width"],
        _ => &[],
    };
    if let Some(key) = numbers.iter().find(|k| number(k)) {
        return Err(format!("`{key}` must be a number"));
    }
    if id == "blur.gaussian" && switch("linear") {
        return Err("`linear` must be true or false".into());
    }
    Ok(())
}

/// Can an effect read `v` as its `color`: a hex color or name [`Color::from_hex`] reads, `[r, g, b]`
/// with three numbers first, or a color object?
///
/// [`Color::from_hex`]: vectorcraft_doc::color::Color::from_hex
fn is_color(v: &Value) -> bool {
    use vectorcraft_doc::color::Color;
    match v {
        Value::String(s) => Color::from_hex(s).is_some(),
        Value::Array(a) => a.len() >= 3 && a.iter().take(3).all(Value::is_number),
        Value::Object(_) => serde_json::from_value::<Color>(v.clone()).is_ok(),
        _ => false,
    }
}

/// Does parameter documentation `doc` (`{edgeWidth: pt 1..14 (2), …}`) name parameter `key`?
fn documents_key(doc: &str, key: &str) -> bool {
    doc.match_indices(key).any(|(at, _)| {
        let before = doc.get(..at).and_then(|s| s.chars().last());
        let after = doc.get(at + key.len()..).unwrap_or_default();
        matches!(before, Some('{' | ' ' | ',' | '|')) && (after.starts_with(':') || after.starts_with('?'))
    })
}

/// Makes [`check_returned_effect`] how this process checks the effect records painted live-effect
/// plug-ins return ([`vectorcraft_plugins::painted::set_effect_check`]).
pub(crate) fn check_returned_effects() {
    vectorcraft_plugins::painted::set_effect_check(check_returned_effect);
}

/// Scale the distance parameters of `e` by `s` (Scale Strokes & Effects; see
/// [`EffectInfo::lengths`]), and its sizes in raster pixels ([`crate::raster_px_range`]), which
/// keep their share of the object so. A distance left at its default is written out scaled; one
/// the effect doesn't have (Outline Stroke's weight following its stroke) stays absent. A Pathfinder
/// effect without a `precision` gets the precision it evaluates with, scaled. Values are scaled
/// as stored, out of range or not, so a scale and its inverse restore them; evaluation applies
/// each effect's range.
pub fn scale_effect(e: &mut Effect, s: f64) {
    let Some(info) = catalog_index().get(e.id.as_str()) else { return };
    let merged = merged_params(&e.id, &e.params);
    let relative = util::flag(&merged, "relative", false);
    let mut keys: Vec<&str> = info.lengths.keys(relative).collect();
    keys.extend(merged.as_object().into_iter().flat_map(|m| m.keys()).map(String::as_str).filter(|k| crate::raster_px_range(&e.id, k).is_some()));
    for k in keys {
        let missing =
            if k == "precision" && crate::takes_pathfinder_options(&e.id) { crate::PathfinderEffectOptions::default().precision } else { f64::NAN };
        let v = util::num(&merged, k, missing);
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
        || id == crate::SVG_FILTER
}

/// Does `id` change geometry? (Crop Marks and Vector Halftone add art of their own instead,
/// [`crop_marks_art`] and [`crate::addons::halftone::halftone_art`]; colour adjustments recolour,
/// [`adjust`].) Plug-in effects do, installed or not (a missing plug-in
/// leaves the geometry as it is).
pub fn is_geometry(id: &str) -> bool {
    !is_raster(id)
        && !is_pathfinder(id)
        && !is_adjustment(id)
        && id != CROP_MARKS
        && id != crate::addons::halftone::HALFTONE
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

/// Does `n` have a visible live-effect plug-in effect, its own or a fill's or stroke's, whose
/// plug-in paints the pieces it returns? What such an effect draws also depends on the document's
/// swatches and patterns, which its pieces may name.
pub fn has_painted_plugin_effect(n: &Node) -> bool {
    let painted = |e: &Effect| e.visible && vectorcraft_plugins::effect::plugin_id(&e.id).is_some_and(vectorcraft_plugins::effect::paints_output);
    n.appearance.effects.iter().any(painted) || n.appearance.items.iter().any(|i| i.effects().iter().any(painted))
}

/// Is a live-effect plug-in that paints the pieces it returns installed?
pub fn any_plugin_paints_output() -> bool {
    vectorcraft_plugins::registry::list().iter().any(|p| vectorcraft_plugins::effect::paints_output(&p.manifest().id))
}

/// Changes whenever a plug-in is installed or removed (the registry's revision), and when deferred
/// live-effect plug-in stages give results ([`vectorcraft_plugins::deferred::revision`]).
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision() ^ vectorcraft_plugins::deferred::revision().rotate_left(32)
}
