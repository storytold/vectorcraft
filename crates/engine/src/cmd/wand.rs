//! Magic Wand: select objects with attributes similar to a reference object. The settings are the
//! Magic Wand panel's (session state, kept across tool switches); the tool runs
//! `select.magicWand` on click.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::select::selectable_ids;
use super::*;

/// Magic Wand panel settings. Tolerances: colours as 0–255 RGB distance, stroke weight in
/// points, opacity in percent.
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
            "{fillColor?, fillTolerance?: 0-255, strokeColor?, strokeTolerance?: 0-255, strokeWeight?, weightTolerance?: pt, opacity?, opacityTolerance?: %, blendingMode?, reset?: bool} → the settings",
            always,
            set
        ),
        cmd!(query "magicWand.options", "Magic Wand Settings", [], None, "{} → the Magic Wand settings", always, |s, _| Ok(json!(s.menu.wand))),
    ]
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    if bool_or(p, "reset", false) {
        s.menu.wand = WandSettings::default();
    }
    let mut v = serde_json::to_value(&s.menu.wand).map_err(|e| EngineError::Other(e.to_string()))?;
    if let (Some(o), Some(src)) = (v.as_object_mut(), p.as_object()) {
        for (k, val) in src {
            if o.contains_key(k) {
                o.insert(k.clone(), val.clone());
            }
        }
    }
    let mut w: WandSettings = serde_json::from_value(v).map_err(|e| bad("magicWand.set", e.to_string()))?;
    w.fill_tolerance = w.fill_tolerance.clamp(0.0, 255.0);
    w.stroke_tolerance = w.stroke_tolerance.clamp(0.0, 255.0);
    w.weight_tolerance = w.weight_tolerance.clamp(0.0, 1000.0);
    w.opacity_tolerance = w.opacity_tolerance.clamp(0.0, 100.0);
    s.menu.wand = w;
    Ok(json!(s.menu.wand))
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

/// Are two paints within `tol` (0–255 Euclidean RGB distance)? Non-solid paints must match exactly.
pub fn similar_paint(a: &Paint, b: &Paint, tol: f64) -> bool {
    match (a.color(), b.color()) {
        (Some(x), Some(y)) => {
            let (x, y) = (x.to_rgb(), y.to_rgb());
            let d: f64 = (0..3).map(|i| ((x[i] - y[i]) as f64 * 255.0).powi(2)).sum::<f64>().sqrt();
            d <= tol + 1e-6
        }
        _ => a == b,
    }
}

impl WandSettings {
    /// Does `n` match `reference` on every enabled attribute?
    pub fn matches(&self, reference: &Node, n: &Node) -> bool {
        let (rs, rw) = stroke_of(reference);
        let (ns, nw) = stroke_of(n);
        (!self.fill_color || similar_paint(&fill_of(reference), &fill_of(n), self.fill_tolerance))
            && (!self.stroke_color || similar_paint(&rs, &ns, self.stroke_tolerance))
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
        } else if !n.is_container() {
            out.push(n.id);
        }
    });
    out.retain(|id| {
        !doc.parent_of(*id).is_some_and(|p| compounds.contains(&p))
            && doc.is_editable(*id)
            && doc.is_visible(*id)
            && doc.node(*id).is_some_and(|n| n.visible)
    });
    out
}

fn magic_wand(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("select.magicWand", "missing id"))?;
    let st = s.doc()?;
    let reference = st.doc.node(id).ok_or(EngineError::NoNode(id))?.clone();
    let w = s.menu.wand.clone();
    // In isolation mode only the isolated group's contents can be picked, as for Select Same.
    let inside = st.isolation.map(|iso| selectable_ids(&st.doc, Some(iso)));
    let hits: Vec<NodeId> = paintable(&st.doc)
        .into_iter()
        .filter(|i| inside.as_ref().is_none_or(|set| set.contains(i)))
        .filter(|i| st.doc.node(*i).is_some_and(|n| w.matches(&reference, n)))
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
