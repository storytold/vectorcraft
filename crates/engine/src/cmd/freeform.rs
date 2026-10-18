//! Freeform gradients: the points and lines of the freeform gradient behind the active proxy (the
//! Gradient panel's freeform section, the Color panel and the Gradient tool edit them through
//! these). Points are given in document coordinates; type runs keep them in text space.

use serde_json::{Value, json};
use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{Freeform, FreeformMode, FreeformPoint, GradientKind, Paint};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Affine, Rect};

use super::appearance::{edit_items, edits_stroke, index_param, item_target};
use super::gradient::{StopOwner, item_paint_bounds, run_paint_mut};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.freeform.addPoint",
            "Add Freeform Point",
            [],
            None,
            "{at: [x,y] (document coordinates), color? (default: the gradient's colour there), keepModel?: bool (as paint.setFill), opacity?: 0..1 (or 0..100), spread?: 0..1 (or 0..100 %; the radius of pure colour around the point, a fraction of half the object's larger side; default 0), line?: true (join it to the selected point: extends the line ending there, else starts one), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\"} add a point to the freeform gradient (an unplaced one keeps its automatic points first) and select it → {index}",
            has_doc,
            add_point
        ),
        cmd!(
            "paint.freeform.setPoint",
            "Edit Freeform Point",
            [],
            None,
            "{index? (default: the selected point), at?: [x,y] (document coordinates), color?, keepModel?: bool (as paint.setFill), opacity?: 0..1 (or 0..100), spread?: 0..1 (or 0..100 %), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\"} move or recolour a point of the freeform gradient → {index}",
            has_doc,
            set_point
        ),
        cmd!(
            "paint.freeform.deletePoint",
            "Delete Freeform Point",
            [],
            None,
            "{index? (default: the selected point), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\"} remove a point (never the last one); lines through it close up around it and lines left with one point go → {index}",
            has_doc,
            delete_point
        ),
        cmd!(
            "paint.freeform.addLine",
            "Add Freeform Line",
            [],
            None,
            "{points: [index, …] (at least two), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\"} join points with a smooth line through them, in order → {line}",
            has_doc,
            add_line
        ),
        cmd!(
            "paint.freeform.splitLine",
            "Split Freeform Line",
            [],
            None,
            "{line: line index, segment: the segment from the line's point `segment` to the next, t?: 0..1 along it (default 0.5), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\"} add a point on a line there (colour, opacity and spread between the segment's ends) and select it → {index}",
            has_doc,
            split_line
        ),
        cmd!(
            "paint.freeform.selectPoint",
            "Select Freeform Point",
            [],
            None,
            "{index: point index | null to clear} select a point of the freeform gradient behind the active proxy (the first selected object's): the point the Gradient tool, the Gradient and Color panels and Delete act on → {index}",
            has_doc,
            select_point
        ),
        cmd!(
            query "paint.freeform.get",
            "Freeform Gradient",
            [],
            None,
            "{stroke?: bool (default: the active proxy)} the freeform gradient behind the proxy of the first selected object → {mode: points|lines, points: [{at: [x,y] (document coordinates), color, opacity, spread}], lines: [[index, …]], selected: index|null}",
            has_doc,
            get
        ),
    ]
}

const NOT_FREEFORM: &str = "the paint is not a freeform gradient (apply one with paint.editGradient {kind: \"freeform\"})";

impl Session {
    /// The selected freeform point (`paint.freeform.selectPoint`), while the gradient it was
    /// selected on is still the one behind the active proxy. Callers check it against the point
    /// count (an undo can remove points).
    pub fn selected_freeform_point(&self) -> Option<usize> {
        self.freeform_point.filter(|(_, owner)| *owner == StopOwner::of(self)).map(|(i, _)| i)
    }

    /// The freeform gradient behind the Fill (`stroke` false) or Stroke proxy, its points in
    /// document coordinates, and the length its spreads are fractions of there (see
    /// [`Node::proxy_freeform`]).
    pub fn proxy_freeform(&self, stroke: bool) -> Option<(Freeform, f64)> {
        let (n, item) = self.proxy_source(stroke)?;
        n.proxy_freeform(stroke, item)
    }

    fn select_freeform_point(&mut self, i: Option<usize>) {
        self.freeform_point = i.map(|i| (i, StopOwner::of(self)));
    }
}

/// Where an edited paint lives: the map from the document to its space and the length its spreads
/// are fractions of.
#[derive(Clone, Copy)]
struct Space {
    to_paint: Affine,
    scale: f64,
}

/// The `opacity` / `spread` convention: 0..1, or a percentage above 1.
fn fraction(v: &Value, key: &str) -> std::result::Result<f32, String> {
    let x = v.as_f64().ok_or_else(|| format!("`{key}` must be a number"))?;
    Ok((if x > 1.0 { x / 100.0 } else { x }).clamp(0.0, 1.0) as f32)
}

/// Apply the `at` (mapped by `to_paint`), `color`, `opacity` and `spread` fields of `v` to `pt`.
fn point_fields(v: &Value, pt: &mut FreeformPoint, to_paint: Affine) -> std::result::Result<(), String> {
    if v.get("at").is_some() {
        pt.at = to_paint * point_param(v, "at").ok_or("`at` must be [x, y]")?;
    }
    if let Some(c) = v.get("color") {
        pt.color = color_value(c).ok_or("`color` must be a colour")?;
    }
    if let Some(o) = v.get("opacity") {
        pt.opacity = fraction(o, "opacity")?;
    }
    if let Some(s) = v.get("spread") {
        pt.spread = fraction(s, "spread")?;
    }
    Ok(())
}

/// The `color` of `p` as the document stores it: an RGB color in a CMYK document becomes CMYK
/// through the color settings ([`super::paint::new_color`]). `None` without a valid one.
fn given_color(s: &Session, p: &Value) -> Option<vectorcraft_color::Color> {
    p.get("color").and_then(color_value).map(|c| super::paint::new_color(s, p, c))
}

/// Parse the `freeform` param of a gradient paint (`{points: [{at, color, opacity?, spread?}],
/// lines?: [[index, …]], mode?}`; points in the paint's space). Lossless for
/// `vectorcraft_tools::params::freeform_json`.
pub(crate) fn parse_freeform(v: &Value) -> std::result::Result<Freeform, String> {
    let points = v.get("points").and_then(Value::as_array).ok_or("`freeform.points` must be an array")?;
    let mut f = Freeform::default();
    for (i, pv) in points.iter().enumerate() {
        let (Some(at), Some(color)) = (point_param(pv, "at"), pv.get("color").and_then(color_value)) else {
            return Err(format!("freeform point {i} needs `at` [x, y] and a valid `color`"));
        };
        let mut pt = FreeformPoint::new(at, color);
        point_fields(pv, &mut pt, Affine::IDENTITY).map_err(|e| format!("freeform point {i}: {e}"))?;
        f.points.push(pt);
    }
    if let Some(lines) = v.get("lines") {
        let lines = lines.as_array().ok_or("`freeform.lines` must be an array")?;
        for (i, l) in lines.iter().enumerate() {
            let ix = point_indices(l).ok_or_else(|| format!("freeform line {i} must be an array of point indices"))?;
            f.add_line(ix).map_err(|e| format!("freeform line {i}: {e}"))?;
        }
    }
    if let Some(m) = str_param(v, "mode") {
        f.mode = parse_mode(m)?;
    }
    Ok(f)
}

pub(crate) fn parse_mode(m: &str) -> std::result::Result<FreeformMode, String> {
    FreeformMode::parse(m).ok_or_else(|| format!("unknown freeform mode `{m}` (points, lines)"))
}

/// Run `f` on the freeform gradient behind the proxy of each target (the selection, or `ids`),
/// as one undo step; returns what it returned for the first. An unplaced gradient gets the
/// automatic points it shows first, and the stops follow the points' colours.
fn edit<R>(s: &mut Session, p: &Value, cmd: &str, mut f: impl FnMut(&mut Freeform, Space) -> std::result::Result<R, String>) -> Result<R> {
    let item = item_target(s, p, cmd)?;
    let stroke = edits_stroke(s, p, item, !s.fill_active)?;
    let item = item.of_kind(s, !stroke);
    let ids = item.targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(cmd, "select an object with a freeform gradient"));
    }
    let mut first = None;
    let mut apply = |paint: &mut Paint, b: Rect, to_doc: Affine| -> Result<()> {
        let Paint::Gradient(g) = paint else { return Err(bad(cmd, NOT_FREEFORM)) };
        if g.gradient.kind != GradientKind::Freeform {
            return Err(bad(cmd, NOT_FREEFORM));
        }
        let to_paint = if to_doc == Affine::IDENTITY { to_doc } else { to_doc.inverse() };
        let mut ff = g.freeform_on(b).into_owned();
        let r = f(&mut ff, Space { to_paint, scale: spread_scale(b) }).map_err(|e| bad(cmd, e))?;
        g.set_freeform(ff);
        g.swatch = None;
        first.get_or_insert(r);
        Ok(())
    };
    edit_items(s, &ids, item, cmd, "Gradient", !stroke, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            let (xf, lb) = (t.xf, t.local_bounds());
            if xf.determinant().abs() < 1e-12 {
                return Err(bad(cmd, "the type object is flattened to nothing"));
            }
            for r in &mut t.runs {
                let (paint, b) = run_paint_mut(r, stroke, lb);
                apply(paint, b, xf)?;
            }
            return Ok(());
        }
        let b = item_paint_bounds(n, index, !stroke).ok_or_else(|| bad(cmd, NOT_FREEFORM))?;
        let paint =
            if stroke { n.appearance.stroke_at_mut(index).map(|l| &mut l.paint) } else { n.appearance.fill_at_mut(index).map(|l| &mut l.paint) };
        apply(paint.ok_or_else(|| bad(cmd, NOT_FREEFORM))?, b, Affine::IDENTITY)
    })?;
    let r = first.ok_or_else(|| bad(cmd, NOT_FREEFORM))?;
    // The edited gradient is the last one used (`paint.lastGradient`).
    let shown = s.doc()?.doc.node(ids[0]).map(|n| super::paint::proxy_paint(n, stroke, item.resolve(&n.appearance, !stroke, cmd).ok().flatten()));
    if let Some(p) = shown {
        s.remember_paint(&p);
    }
    Ok(r)
}

/// The `index` param, else the selected point.
fn point_index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    match p.get("index") {
        Some(v) => v.as_u64().map(|i| i as usize).ok_or_else(|| bad(cmd, "`index` must be a point index")),
        None => s.selected_freeform_point().ok_or_else(|| bad(cmd, "no point selected (give `index`)")),
    }
}

fn check_index(f: &Freeform, i: usize) -> std::result::Result<(), String> {
    if i < f.points.len() { Ok(()) } else { Err(format!("no point {i} (the gradient has {})", f.points.len())) }
}

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.addPoint";
    let at = point_param(p, "at").ok_or_else(|| bad(C, "missing `at` [x, y]"))?;
    let join = match p.get("line") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => s.selected_freeform_point(),
        Some(_) => return Err(bad(C, "`line` must be true or false")),
    };
    let given = given_color(s, p);
    let index = edit(s, p, C, |f, sp| {
        let q = sp.to_paint * at;
        let (color, opacity) = f.sample(q, sp.scale);
        let mut pt = FreeformPoint { opacity, ..FreeformPoint::new(q, color) };
        point_fields(p, &mut pt, sp.to_paint)?;
        if let Some(c) = given {
            pt.color = c;
        }
        let i = f.add_point(pt);
        if let Some(from) = join {
            f.connect(from, i);
        }
        Ok(i)
    })?;
    s.select_freeform_point(Some(index));
    Ok(json!({ "index": index }))
}

fn set_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.setPoint";
    let i = point_index(s, p, C)?;
    let given = given_color(s, p);
    edit(s, p, C, |f, sp| {
        check_index(f, i)?;
        point_fields(p, &mut f.points[i], sp.to_paint)?;
        if let Some(c) = given {
            f.points[i].color = c;
        }
        Ok(())
    })?;
    Ok(json!({ "index": i }))
}

fn delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.deletePoint";
    let i = point_index(s, p, C)?;
    edit(s, p, C, |f, _| {
        check_index(f, i)?;
        if f.remove_point(i) { Ok(()) } else { Err("a freeform gradient keeps at least one point".into()) }
    })?;
    // The points after it moved down one.
    let sel = s.selected_freeform_point().and_then(|j| match j.cmp(&i) {
        std::cmp::Ordering::Less => Some(j),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(j - 1),
    });
    s.select_freeform_point(sel);
    Ok(json!({ "index": i }))
}

fn add_line(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.addLine";
    let points = p.get("points").and_then(point_indices).ok_or_else(|| bad(C, "`points` must be an array of point indices"))?;
    let line = edit(s, p, C, |f, _| f.add_line(points.clone()))?;
    Ok(json!({ "line": line }))
}

fn split_line(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.splitLine";
    let line = index_param(p, "line", C)?;
    let segment = index_param(p, "segment", C)?;
    let t = f64_or(p, "t", 0.5);
    let index = edit(s, p, C, |f, _| f.split_line(line, segment, t).ok_or_else(|| format!("no segment {segment} on line {line}")))?;
    s.select_freeform_point(Some(index));
    Ok(json!({ "index": index }))
}

fn select_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.selectPoint";
    let index = match p.get("index") {
        None => return Err(bad(C, "missing `index` (a point index, or null to clear)")),
        Some(Value::Null) => None,
        Some(v) => {
            let i = v.as_u64().ok_or_else(|| bad(C, "`index` must be a whole number or null"))? as usize;
            let (f, _) = s.proxy_freeform(!s.fill_active).ok_or_else(|| bad(C, NOT_FREEFORM))?;
            check_index(&f, i).map_err(|e| bad(C, e))?;
            Some(i)
        }
    };
    s.select_freeform_point(index);
    Ok(json!({ "index": index }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.freeform.get";
    let stroke = bool_or(p, "stroke", !s.fill_active);
    let (f, _) = s.proxy_freeform(stroke).ok_or_else(|| bad(C, NOT_FREEFORM))?;
    let selected = if stroke != s.fill_active { s.selected_freeform_point().filter(|i| *i < f.points.len()) } else { None };
    let mut v = vectorcraft_tools::params::freeform_json(&f);
    v["selected"] = json!(selected);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line with an index that isn't a point's is refused, not shortened to the indices that are.
    #[test]
    fn malformed_freeform_lines_are_not_silently_shortened() {
        let points = json!([
            {"at": [0, 0], "color": "#ff0000"},
            {"at": [50, 0], "color": "#00ff00"},
            {"at": [100, 0], "color": "#0000ff"}
        ]);
        for lines in [json!([[0, "broken", 2]]), json!([[0, -1, 2]]), json!([[0, 1.5, 2]]), json!([{"points": [0, 1]}]), json!("not an array")] {
            let value = json!({"points": points, "lines": lines, "mode": "lines"});
            assert!(parse_freeform(&value).is_err(), "invalid lines accepted: {value}");
        }
        let good = parse_freeform(&json!({"points": points, "lines": [[0, 1, 2]], "mode": "lines"})).unwrap();
        assert_eq!(good.lines, vec![vec![0, 1, 2]]);
    }
}
