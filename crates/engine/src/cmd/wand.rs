//! Magic Wand: select objects with attributes similar to a reference object. The settings are the
//! Magic Wand panel's (session state, kept across tool switches); the tool runs
//! `select.magicWand` on click.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{ColorMode, Document, Node, NodeId, NodeKind};

use super::select::selectable_ids;
use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, has_doc, object_param, str_param, switch_or};

/// Magic Wand panel settings. Tolerances: colors as a Euclidean distance, 0–255 between RGB
/// values in an RGB document and 0–100 between CMYK percentages in a CMYK document; stroke
/// weight in points; opacity in percent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WandSettings {
    pub fill_color: bool,
    pub fill_tolerance: f64,
    pub stroke_color: bool,
    pub stroke_tolerance: f64,
    pub stroke_weight: bool,
    pub weight_tolerance: f64,
    pub opacity: bool,
    pub opacity_tolerance: f64,
    pub blending_mode: bool,
}

impl Default for WandSettings {
    fn default() -> Self {
        Self {
            fill_color: true,
            fill_tolerance: 32.0,
            stroke_color: false,
            stroke_tolerance: 32.0,
            stroke_weight: false,
            weight_tolerance: 5.0,
            opacity: false,
            opacity_tolerance: 5.0,
            blending_mode: false,
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "select.magicWand",
            "Magic Wand",
            [],
            None,
            "{id, mode?: set|add|subtract} select objects similar to `id` by the Magic Wand settings → {count}",
            has_doc,
            magic_wand
        ),
        cmd!(
            query "magicWand.set",
            "Magic Wand Options",
            ["Window", "Magic Wand"],
            None,
            "{fillColor?, fillTolerance?: 0-255 in an RGB document or 0-100 in a CMYK one, strokeColor?, strokeTolerance?: as fillTolerance, strokeWeight?, weightTolerance?: 0-1000 pt, opacity?, opacityTolerance?: 0-100 %, blendingMode?, reset?: bool} → the settings; a value out of range is an error and changes nothing, and an omitted one keeps its setting",
            always,
            set
        ),
        cmd!(
            query "magicWand.options",
            "Magic Wand Settings",
            [],
            None,
            "{} → the Magic Wand settings (color tolerances at most 100 in a CMYK document)",
            always,
            |s, _| Ok(json!(s.menu.wand.effective(active_mode(s))))
        ),
    ]
}

/// The active document's color mode, RGB with no document open.
fn active_mode(s: &Session) -> ColorMode {
    s.doc().map(|st| st.doc.color_mode).unwrap_or_default()
}

/// The largest Fill Color and Stroke Color tolerance in a document of `mode`.
pub fn color_tolerance_max(mode: ColorMode) -> f64 {
    match mode {
        ColorMode::Rgb => 255.0,
        ColorMode::Cmyk => 100.0,
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "magicWand.set";
    let mode = active_mode(s);
    let color_max = color_tolerance_max(mode);
    let mut w = if switch_or(p, "reset", false, CMD)? { WandSettings::default() } else { s.menu.wand.clone() };
    for (key, flag) in [
        ("fillColor", &mut w.fill_color),
        ("strokeColor", &mut w.stroke_color),
        ("strokeWeight", &mut w.stroke_weight),
        ("opacity", &mut w.opacity),
        ("blendingMode", &mut w.blending_mode),
    ] {
        // Only a missing key keeps the setting; null, like any other value that isn't true or
        // false, is an error.
        if let Some(v) = p.get(key) {
            *flag = v.as_bool().ok_or_else(|| bad(CMD, format!("`{key}` must be true or false")))?;
        }
    }
    for (key, value, max) in [
        ("fillTolerance", &mut w.fill_tolerance, color_max),
        ("strokeTolerance", &mut w.stroke_tolerance, color_max),
        ("weightTolerance", &mut w.weight_tolerance, 1000.0),
        ("opacityTolerance", &mut w.opacity_tolerance, 100.0),
    ] {
        let Some(v) = p.get(key) else { continue };
        let n = v.as_f64().ok_or_else(|| bad(CMD, format!("`{key}` must be a number, not {v}")))?;
        if !(n.is_finite() && (0.0..=max).contains(&n)) {
            return Err(bad(CMD, format!("`{key}` must be from 0 to {max}, not {n}")));
        }
        *value = n;
    }
    s.menu.wand = w;
    Ok(json!(s.menu.wand.effective(mode)))
}

/// The fill an object shows: type's comes from its characters (the first run speaks for all; type
/// without any, the default character fill, as `document.inspect` reports it).
pub(crate) fn fill_of(n: &Node) -> Paint {
    match &n.kind {
        NodeKind::Text(t) => t.runs.first().map(|r| r.style.fill.clone()).unwrap_or_else(|| vectorcraft_doc::CharStyle::default().fill),
        _ => n.appearance.fill_paint(),
    }
}

/// The stroke paint and weight an object shows, type's from its first run as [`fill_of`].
pub(crate) fn stroke_of(n: &Node) -> (Paint, f64) {
    match &n.kind {
        NodeKind::Text(t) => t.runs.first().map(|r| (r.style.stroke.clone(), r.style.stroke_width)).unwrap_or_default(),
        _ => (n.appearance.stroke_paint(), n.appearance.stroke_width()),
    }
}

/// Are two paints within `tol`? In an RGB document `tol` is a 0–255 Euclidean distance between
/// the display RGB values; in a CMYK document it is a 0–100 Euclidean distance between the CMYK
/// percentages (a CMYK color's own values, any other color converted to CMYK). Non-solid paints
/// must match exactly.
pub fn similar_paint(a: &Paint, b: &Paint, tol: f64, mode: ColorMode) -> bool {
    match (a.color(), b.color()) {
        (Some(x), Some(y)) => {
            let (d, margin) = match mode {
                ColorMode::Rgb => {
                    let (x, y) = (x.to_rgb(), y.to_rgb());
                    ((0..3).map(|i| ((x[i] - y[i]) as f64 * 255.0).powi(2)).sum::<f64>().sqrt(), 1e-6)
                }
                // CMYK components are f32 fractions, so a distance of exactly `tol` points can come
                // out a few millionths over.
                ColorMode::Cmyk => {
                    let (x, y) = (x.to_cmyk(), y.to_cmyk());
                    ((0..4).map(|i| ((x[i] - y[i]) as f64 * 100.0).powi(2)).sum::<f64>().sqrt(), 1e-4)
                }
            };
            d <= tol + margin
        }
        _ => a == b,
    }
}

impl WandSettings {
    /// The settings in effect in a document of `mode`: the color tolerances are capped at that
    /// mode's maximum, so a larger value set in an RGB document applies as 100 in a CMYK one and
    /// comes back in an RGB one.
    pub fn effective(&self, mode: ColorMode) -> WandSettings {
        let max = color_tolerance_max(mode);
        WandSettings { fill_tolerance: self.fill_tolerance.min(max), stroke_tolerance: self.stroke_tolerance.min(max), ..self.clone() }
    }

    /// Does `n` match `reference` on every enabled attribute, comparing colors as in a document of
    /// `mode`?
    pub fn matches(&self, reference: &Node, n: &Node, mode: ColorMode) -> bool {
        let w = self.effective(mode);
        let (rs, rw) = stroke_of(reference);
        let (ns, nw) = stroke_of(n);
        (!w.fill_color || similar_paint(&fill_of(reference), &fill_of(n), w.fill_tolerance, mode))
            && (!w.stroke_color || similar_paint(&rs, &ns, w.stroke_tolerance, mode))
            && (!self.stroke_weight || (rw - nw).abs() <= self.weight_tolerance + 1e-9)
            && (!self.opacity || ((reference.opacity - n.opacity).abs() as f64 * 100.0) <= self.opacity_tolerance + 1e-6)
            && (!self.blending_mode || reference.blend == n.blend)
    }
}

/// Paintable leaves (paths, compounds, text, images) that are visible and editable.
fn paintable(doc: &Document) -> Vec<NodeId> {
    let mut out = vec![];
    let mut compounds: Vec<NodeId> = vec![];
    doc.walk(|n| {
        if matches!(n.kind, NodeKind::Compound { .. }) {
            compounds.push(n.id);
            out.push(n.id);
        } else if !n.is_container() && !n.is_ruler_guide() {
            out.push(n.id);
        }
    });
    let editable = doc.editable();
    out.retain(|id| {
        !doc.parent_of(*id).is_some_and(|p| compounds.contains(&p))
            && editable(*id)
            && doc.is_visible(*id)
            && doc.node(*id).is_some_and(|n| n.visible)
    });
    out
}

fn magic_wand(s: &mut Session, p: &Value) -> Result<Value> {
    let id = object_param(p, "id", "select.magicWand")?.ok_or_else(|| bad("select.magicWand", "missing id"))?;
    let st = s.doc()?;
    let reference = st.doc.node(id).ok_or(EngineError::NoNode(id))?.clone();
    let w = s.menu.wand.clone();
    let mode = st.doc.color_mode;
    // In isolation mode only the isolated group's contents can be picked, as for Select Same.
    let inside = st.isolation.map(|iso| selectable_ids(&st.doc, Some(iso)));
    let hits: Vec<NodeId> = paintable(&st.doc)
        .into_iter()
        .filter(|i| inside.as_ref().is_none_or(|set| set.contains(i)))
        .filter(|i| st.doc.node(*i).is_some_and(|n| w.matches(&reference, n, mode)))
        .collect();
    let ids: Vec<NodeId> = match str_param(p, "mode").unwrap_or("set") {
        "add" => st.selection.objects.iter().copied().chain(hits.iter().copied().filter(|h| !st.selection.objects.contains(h))).collect(),
        "subtract" => st.selection.objects.iter().copied().filter(|i| !hits.contains(i)).collect(),
        "set" => hits,
        other => return Err(bad("select.magicWand", format!("unknown mode `{other}` (set, add, subtract)"))),
    };
    let n = ids.len();
    s.select(|_, sel| sel.set(ids))?;
    Ok(json!({ "count": n }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(s: &mut Session, x: f64, fill: &str, stroke_w: f64) -> u64 {
        let id = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": fill})).unwrap();
        s.execute("stroke.set", &json!({"weight": stroke_w})).unwrap();
        id
    }

    fn selected(s: &Session) -> Vec<u64> {
        let st = s.doc().unwrap();
        st.selection.in_paint_order(&st.doc).iter().map(|i| i.0).collect()
    }

    #[test]
    fn fill_tolerance_stroke_weight_and_modes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let a = rect(&mut s, 0.0, "#ffffff", 1.0);
        let b = rect(&mut s, 20.0, "#fafafa", 4.0);
        let c = rect(&mut s, 40.0, "#ff0000", 1.0);
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        s.execute("magicWand.set", &json!({"fillTolerance": 0})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a]);
        s.execute("select.magicWand", &json!({"id": c, "mode": "add"})).unwrap();
        assert_eq!(selected(&s), [a, c]);
        // Stroke weight only (fill off): a and c share 1 pt.
        s.execute("magicWand.set", &json!({"fillColor": false, "strokeWeight": true, "weightTolerance": 0.5})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, c]);
        s.execute("select.magicWand", &json!({"id": a, "mode": "subtract"})).unwrap();
        assert!(selected(&s).is_empty());
        // Settings survive tool switches and reset to Illustrator's defaults.
        s.select_tool("pen", Default::default()).unwrap();
        assert_eq!(s.execute("magicWand.options", &json!({})).unwrap()["strokeWeight"], true);
        assert_eq!(s.execute("magicWand.set", &json!({"reset": true})).unwrap(), json!(WandSettings::default()));
    }

    #[test]
    fn opacity_and_blending_mode() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let a = rect(&mut s, 0.0, "#000000", 1.0);
        let b = rect(&mut s, 20.0, "#000000", 1.0);
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("transparency.set", &json!({"opacity": 50, "blend": "Multiply"})).unwrap();
        s.execute("magicWand.set", &json!({"opacity": true})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a]);
        s.execute("magicWand.set", &json!({"opacity": false, "blendingMode": true})).unwrap();
        s.execute("select.magicWand", &json!({"id": b})).unwrap();
        assert_eq!(selected(&s), [b]);
    }

    /// A 10 × 10 rectangle at `x` filled and stroked with the given colors (JSON color values).
    fn painted(s: &mut Session, x: f64, fill: Value, stroke: Value) -> u64 {
        let id = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({ "color": fill })).unwrap();
        s.execute("paint.setStroke", &json!({ "color": stroke })).unwrap();
        id
    }

    fn cmyk(c: f64, m: f64, y: f64, k: f64) -> Value {
        json!({"model": "cmyk", "c": c, "m": m, "y": y, "k": k})
    }

    #[test]
    fn cmyk_document_compares_fill_and_stroke_colors_in_cmyk() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200, "colorMode": "cmyk"})).unwrap();
        let white = cmyk(0.0, 0.0, 0.0, 0.0);
        let a = painted(&mut s, 0.0, cmyk(0.1, 0.0, 0.0, 0.0), white.clone());
        let b = painted(&mut s, 20.0, cmyk(0.0, 0.0, 0.0, 0.0), white.clone());
        let c = painted(&mut s, 40.0, cmyk(0.0, 0.1, 0.0, 0.0), white.clone());
        // a to b is 10 points of cyan; a to c is 10 points of cyan and 10 of magenta (14.1).
        s.execute("magicWand.set", &json!({"fillTolerance": 10})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        s.execute("magicWand.set", &json!({"fillTolerance": 9.9})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a]);
        s.execute("magicWand.set", &json!({"fillTolerance": 14.2})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b, c]);
        // Stroke Color alone, the same rule: d's stroke is 5 points of black from a's.
        let d = painted(&mut s, 60.0, cmyk(1.0, 1.0, 1.0, 1.0), cmyk(0.0, 0.0, 0.0, 0.05));
        s.execute("magicWand.set", &json!({"fillColor": false, "strokeColor": true, "strokeTolerance": 5})).unwrap();
        s.execute("select.magicWand", &json!({"id": d})).unwrap();
        assert_eq!(selected(&s), [a, b, c, d]);
        s.execute("magicWand.set", &json!({"strokeTolerance": 4.9})).unwrap();
        s.execute("select.magicWand", &json!({"id": d})).unwrap();
        assert_eq!(selected(&s), [d]);
    }

    #[test]
    fn cmyk_document_compares_gray_as_black_ink() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200, "colorMode": "cmyk"})).unwrap();
        let none = cmyk(0.0, 0.0, 0.0, 0.0);
        let a = painted(&mut s, 0.0, json!({"gray": 0.5}), none.clone());
        let b = painted(&mut s, 20.0, cmyk(0.0, 0.0, 0.0, 0.5), none.clone());
        let c = painted(&mut s, 40.0, cmyk(0.0, 0.0, 0.0, 0.6), none);
        s.execute("magicWand.set", &json!({"fillTolerance": 0})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        s.execute("magicWand.set", &json!({"fillTolerance": 10})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b, c]);
    }

    #[test]
    fn rgb_document_keeps_the_rgb_distance() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let a = rect(&mut s, 0.0, "#000000", 1.0);
        let b = rect(&mut s, 20.0, "#0a0000", 1.0);
        s.execute("magicWand.set", &json!({"fillTolerance": 10})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        s.execute("magicWand.set", &json!({"fillTolerance": 9.9})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a]);
        assert_eq!(s.execute("magicWand.set", &json!({"fillTolerance": 255})).unwrap()["fillTolerance"], 255.0);
    }

    #[test]
    fn set_rejects_tolerances_out_of_range_and_changes_nothing() {
        let mut s = Session::new();
        // With no document the color range is the RGB one.
        assert!(s.execute("magicWand.set", &json!({"fillTolerance": 256})).is_err());
        s.execute("magicWand.set", &json!({"fillTolerance": 200})).unwrap();
        s.execute("file.new", &json!({"width": 400, "height": 200, "colorMode": "cmyk"})).unwrap();
        s.execute("magicWand.set", &json!({"fillTolerance": 40, "strokeTolerance": 50})).unwrap();
        let before = s.execute("magicWand.options", &json!({})).unwrap();
        for bad_value in [
            json!({"fillTolerance": 101}),
            json!({"strokeTolerance": 100.5}),
            json!({"fillTolerance": -1}),
            json!({"fillTolerance": "40"}),
            json!({"weightTolerance": 1001}),
            json!({"opacityTolerance": -0.5}),
            json!({"fillColor": "yes"}),
            json!({"fillTolerance": null}),
            json!({"strokeColor": null}),
            json!({"reset": "yes", "fillColor": false}),
            json!({"reset": true, "fillTolerance": 101}),
            json!({"fillColor": false, "strokeTolerance": 101}),
        ] {
            let e = s.execute("magicWand.set", &bad_value).unwrap_err().to_string();
            assert!(e.contains("magicWand.set"), "{bad_value}: {e}");
            assert_eq!(s.execute("magicWand.options", &json!({})).unwrap(), before, "{bad_value} changed the settings");
        }
        assert!(s.execute("magicWand.set", &json!({"fillTolerance": 101})).unwrap_err().to_string().contains("from 0 to 100"));
        assert_eq!(s.execute("magicWand.set", &json!({"fillTolerance": 100})).unwrap()["fillTolerance"], 100.0);
        // An omitted value keeps its setting.
        assert_eq!(s.execute("magicWand.set", &json!({"strokeColor": true})).unwrap()["strokeTolerance"], 50.0);
    }

    #[test]
    fn a_larger_rgb_tolerance_applies_as_100_in_a_cmyk_document_and_comes_back() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        s.execute("magicWand.set", &json!({"fillTolerance": 200, "strokeTolerance": 60})).unwrap();
        s.execute("file.new", &json!({"width": 400, "height": 200, "colorMode": "cmyk"})).unwrap();
        let o = s.execute("magicWand.options", &json!({})).unwrap();
        assert_eq!((o["fillTolerance"].as_f64(), o["strokeTolerance"].as_f64()), (Some(100.0), Some(60.0)));
        // Setting another field does not overwrite the stored fill tolerance.
        assert_eq!(s.execute("magicWand.set", &json!({"opacity": true})).unwrap()["fillTolerance"], 100.0);
        // 100 applies: two colors 100 points apart in one channel match, 100 apart in two do not.
        let none = cmyk(0.0, 0.0, 0.0, 0.0);
        let a = painted(&mut s, 0.0, cmyk(0.0, 0.0, 0.0, 0.0), none.clone());
        let b = painted(&mut s, 20.0, cmyk(1.0, 0.0, 0.0, 0.0), none.clone());
        let c = painted(&mut s, 40.0, cmyk(1.0, 1.0, 0.0, 0.0), none);
        s.execute("magicWand.set", &json!({"opacity": false})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        assert!(!selected(&s).contains(&c));
        s.execute("document.activate", &json!({"index": 0})).unwrap();
        assert_eq!(s.execute("magicWand.options", &json!({})).unwrap()["fillTolerance"], 200.0);
        // Reset gives 32 in either mode.
        s.execute("document.activate", &json!({"index": 1})).unwrap();
        let o = s.execute("magicWand.set", &json!({"reset": true})).unwrap();
        assert_eq!((o["fillTolerance"].as_f64(), o["strokeTolerance"].as_f64()), (Some(32.0), Some(32.0)));
    }

    #[test]
    fn isolation_mode_limits_the_wand_to_the_isolated_group() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let a = rect(&mut s, 0.0, "#ff0000", 1.0);
        let b = rect(&mut s, 20.0, "#ff0000", 1.0);
        let outside = rect(&mut s, 40.0, "#ff0000", 1.0);
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.isolate", &json!({"id": g})).unwrap();
        s.execute("select.magicWand", &json!({"id": a})).unwrap();
        assert_eq!(selected(&s), [a, b]);
        assert!(!selected(&s).contains(&outside));
    }
}
