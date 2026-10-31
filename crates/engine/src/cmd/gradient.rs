//! The Gradient panel and the Gradient tool: in-place gradient edits and the gradient vector.

use serde_json::{Value, json};
use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientMethod, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::appearance::stroke_paint_bounds;
use vectorcraft_doc::{Document, Node, NodeKind, StrokeGradientMode};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_tools::params::color_json;

use super::appearance::{ItemTarget, edit_items, edits_stroke, item_target};
use super::edit::{outermost, selected_roots};
use super::paint::swatch_solid;
use super::{
    CommandSpec, NodeId, Result, Session, always, bad, bool_or, check_numbers, check_switches, cmd, color_value, f64_or, has_doc, index,
    leaf_targets, point_param, str_param,
};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.editGradient",
            "Gradient",
            ["Window", "Gradient"],
            None,
            "{stroke?: bool (default: the targeted item's kind, else the active proxy), kind?: linear|radial|freeform (freeform places points on each object, coloured along the stops), mode?: points|lines (freeform: how the Gradient tool adds points), method?: classic|perceptual (how the colours between the stops mix: in display RGB, or perceptually (OKLab) so the steps look even, which SVG and PDF exports draw as an image; freeform gradients mix their own way), stops?: [{offset 0..1, color? (needed without swatch), opacity? 0..1 (or 0..100), midpoint? 0.13..0.87, swatch?: colour swatch name (a global or spot colour or tint swatch links the stop, so swatch edits recolour it and a spot stop prints on its plate; a process colour just gives its colour), tint?: 0..100 (% of the linked swatch; default 100, or a tint swatch's own)}] (at least 2; a freeform gradient's points are recoloured along them), keepModel?: bool (as paint.setFill; a stop color the gradient already has keeps its model), angle?: deg, aspect?: %, reverse?: bool, dither?: bool (fills only: a linear or radial gradient fill is drawn with fine noise that hides banding, and as pixels in SVG and PDF exports), item?: fill/stroke item index|null (omitted: the Appearance panel's active item when it is of the edited kind), ids?|id?, strokeMode?: within|along|across (strokes only: the gradient lies on the page and shows through the stroke, runs from the start of each subpath to its end, or runs from the stroke's left edge to its right all along it; with nothing selected, for the next object drawn; type characters' own strokes always paint within), target?: \"object\"|\"contents\"} edit the gradient in place (keeps its placement); solid/none paints become the default gradient",
            has_doc,
            edit_gradient
        ),
        cmd!(
            "paint.setGradientGeom",
            "Gradient Vector",
            [],
            None,
            "{start?: [x,y], end?: [x,y] (document coordinates; both or neither: omitted, the vector stays), aspect?: % (radial: the extent ellipse's height / width; default: kept), focal?: [x,y] (document coordinates) | null (radial: the focal point, where the first stop sits, pulled inside the extent ellipse; null centres it; default: it keeps its place in the ellipse), ids?|id?, stroke?: bool (default: the targeted item's kind, else the active proxy), item?: fill/stroke item index|null (alias: index; omitted: the Appearance panel's active item when it is of the edited kind), target?: \"object\"|\"contents\", index?: appearance item index (as item)} set the gradient vector, aspect ratio and focal point (solid paints become the default gradient; type objects set it on their runs, in text space)",
            has_doc,
            set_gradient_geom
        ),
        cmd!(
            "gradient.selectStop",
            "Select Gradient Stop",
            [],
            None,
            "{index: stop index (0 = the start) | null to clear} select a stop of the gradient behind the active proxy (the first selected object's, else the default paint): the stop the Gradient tool's annotator, the Gradient and Color panels and Delete/arrow keys act on → {index}",
            always,
            select_stop
        ),
    ]
}

/// The gradient behind the active proxy ([`Session::proxy_paints`]).
pub(crate) fn active_gradient(s: &Session) -> Option<GradientPaint> {
    let (fill, stroke) = s.proxy_paints();
    match if s.fill_active { fill } else { stroke } {
        Paint::Gradient(g) => Some(*g),
        _ => None,
    }
}

fn select_stop(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gradient.selectStop";
    let index = match p.get("index") {
        None => return Err(bad(C, "missing `index` (a stop index, or null to clear)")),
        Some(Value::Null) => None,
        Some(v) => {
            let i = v.as_u64().map(index).ok_or_else(|| bad(C, "`index` must be a whole number or null"))?;
            let n = active_gradient(s).ok_or_else(|| bad(C, "the active paint is not a gradient"))?.gradient.stops.len();
            if i >= n {
                return Err(bad(C, format!("no stop {i} (the gradient has {n})")));
            }
            Some(i)
        }
    };
    s.gradient_stop = index.map(|i| (i, StopOwner::of(s)));
    Ok(json!({ "index": index }))
}

/// Whose gradient a selected stop belongs to: the active document, its first selected object
/// (None: the default paint), the proxy in front and the Appearance panel's active item.
/// Selecting other art, toggling the proxy or picking another item leaves no stop selected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StopOwner {
    doc: Option<usize>,
    object: Option<NodeId>,
    fill: bool,
    item: Option<usize>,
}

impl StopOwner {
    pub(crate) fn of(s: &Session) -> Self {
        Self { doc: s.active, object: s.active().and_then(|d| d.selection.objects.first().copied()), fill: s.fill_active, item: s.appearance_item() }
    }
}

impl Session {
    /// The selected gradient stop (`gradient.selectStop`), while the gradient it was selected on
    /// is still the one behind the active proxy. Callers check it against the stop count (an undo
    /// can remove stops).
    pub fn selected_stop(&self) -> Option<usize> {
        self.gradient_stop.filter(|(_, owner)| *owner == StopOwner::of(self)).map(|(i, _)| i)
    }
}

type Parsed<T> = std::result::Result<T, String>;

/// Parse a kind name.
fn parse_kind(k: &str) -> Parsed<GradientKind> {
    GradientKind::parse(k).ok_or_else(|| format!("unknown gradient kind `{k}` (linear, radial, freeform)"))
}

/// The `method` param of `p`, if given: how the colours between the stops mix.
fn method_param(p: &Value) -> Parsed<Option<GradientMethod>> {
    match p.get("method") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(GradientMethod::parse).map(Some).ok_or_else(|| "`method` must be classic or perceptual".into()),
    }
}

/// `v` (params holding `stops`) with each stop that names a `swatch` resolved through `d`
/// ([`swatch_solid`]): a global or spot colour or a tint swatch gives the stop its colour, link and
/// tint, a process colour just its colour. A stop linked to a swatch that no longer exists keeps
/// the colour it gives.
pub(crate) fn link_stops(d: &Document, v: &Value) -> Parsed<Value> {
    let mut v = v.clone();
    let Some(stops) = v.get_mut("stops").and_then(Value::as_array_mut) else { return Ok(v) };
    for (i, st) in stops.iter_mut().enumerate() {
        let Some(name) = st.get("swatch").and_then(Value::as_str).map(str::to_string) else { continue };
        if d.swatch(&name).is_none() && st.get("color").is_some() {
            continue;
        }
        let tint = st
            .get("tint")
            .filter(|v| !v.is_null())
            .map(|v| v.as_f64().ok_or_else(|| format!("stop {i}: `{k}` must be a number", k = "tint")))
            .transpose()?;
        let tint = tint.map(|t| (t / 100.0).clamp(0.0, 1.0) as f32);
        let Paint::Solid { color, swatch, tint } = swatch_solid(d, &name, tint).map_err(|e| format!("stop {i}: {e}"))? else {
            continue;
        };
        let Some(o) = st.as_object_mut() else { continue };
        o.insert("color".into(), color_json(&color));
        match swatch {
            Some(n) => {
                o.insert("swatch".into(), json!(n));
                o.insert("tint".into(), json!(tint * 100.0));
            }
            None => {
                o.remove("swatch");
                o.remove("tint");
            }
        }
    }
    Ok(v)
}

/// Stop colors to store, one per stop of `g` (params holding `stops`, before [`link_stops`]): the
/// color a stop gives and the color the document stores it as when it is new to the gradient, an
/// RGB color in a CMYK document becoming CMYK through the color settings
/// ([`super::paint::new_color`], with `p`'s `keepModel`). `None` for a stop that names a swatch,
/// which gives its color in the swatch's model, and for a color stored as given.
pub(crate) type StopColors = Vec<Option<(vectorcraft_color::Color, vectorcraft_color::Color)>>;

/// The [`StopColors`] of `g`'s stops.
pub(crate) fn stop_colors(s: &Session, p: &Value, g: &Value) -> StopColors {
    let stops = g.get("stops").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let stored = |st: &Value| {
        let c = st.get("color").and_then(color_value).filter(|_| str_param(st, "swatch").is_none())?;
        let to = super::paint::new_color(s, p, c);
        (to != c).then_some((c, to))
    };
    stops.iter().map(stored).collect()
}

/// `g` (params holding `stops`, resolved by [`link_stops`] in their order) for an edit of paint
/// `old`, with each stop's color replaced by the one `stored` ([`stop_colors`]) gives for it,
/// unless `old`'s stops already have that color. A freeform gradient's points are colored from the
/// stored colors, and a stop list sent back with only its positions changed keeps its colors.
pub(crate) fn with_stored_stops<'a>(g: &'a Value, old: &Paint, stored: &StopColors) -> std::borrow::Cow<'a, Value> {
    use std::borrow::Cow;
    if stored.iter().all(Option::is_none) {
        return Cow::Borrowed(g);
    }
    let had = |c: &vectorcraft_color::Color| matches!(old, Paint::Gradient(o) if o.gradient.stops.iter().any(|st| st.color == *c));
    let mut v = g.clone();
    let Some(stops) = v.get_mut("stops").and_then(Value::as_array_mut) else { return Cow::Borrowed(g) };
    for (st, sc) in stops.iter_mut().zip(stored) {
        let Some((from, to)) = sc.filter(|(from, _)| !had(from)) else { continue };
        if st.get("color").and_then(color_value) == Some(from)
            && let Some(o) = st.as_object_mut()
        {
            o.insert("color".into(), color_json(&to));
        }
    }
    Cow::Owned(v)
}

/// Parse `stops` params: at least two, each with a numeric `offset` (clamped to 0..1) and a valid
/// `color`; `opacity` is 0..1 (values above 1 are percentages) and `midpoint` is clamped to the
/// diamond's range; `swatch` and `tint` (%) keep a stop's link as given (resolve them first with
/// [`link_stops`]). The result is sorted by offset.
pub(crate) fn parse_stops(v: &Value) -> Parsed<Vec<GradientStop>> {
    let arr = v.as_array().ok_or("`stops` must be an array")?;
    if arr.len() < 2 {
        return Err("a gradient needs at least two stops".into());
    }
    let mut out = arr
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let num = |k: &str| st.get(k).map(|v| v.as_f64().ok_or_else(|| format!("stop {i}: `{k}` must be a number"))).transpose();
            let offset = num("offset")?.ok_or_else(|| format!("stop {i} needs `offset`"))?;
            let color = st.get("color").and_then(color_value).ok_or_else(|| format!("stop {i} needs a valid `color`"))?;
            let opacity = num("opacity")?.map(|o| if o > 1.0 { o / 100.0 } else { o }).unwrap_or(1.0);
            let swatch = str_param(st, "swatch").map(str::to_string);
            let tint = num("tint")?.filter(|_| swatch.is_some()).map_or(1.0, |t| (t / 100.0).clamp(0.0, 1.0) as f32);
            Ok(GradientStop {
                offset: offset.clamp(0.0, 1.0) as f32,
                color,
                opacity: opacity.clamp(0.0, 1.0) as f32,
                midpoint: num("midpoint")?.unwrap_or(0.5).clamp(0.13, 0.87) as f32,
                swatch,
                tint,
            })
        })
        .collect::<Parsed<Vec<_>>>()?;
    out.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    Ok(out)
}

/// Parse a gradient paint object (the `gradient` param of `paint.setFill`). Lossless for everything
/// `vectorcraft_tools::params::gradient_params` writes.
pub(crate) fn parse_gradient(g: &Value) -> Parsed<GradientPaint> {
    if !g.is_object() {
        return Err("`gradient` must be an object".into());
    }
    let kind = str_param(g, "kind").map(parse_kind).transpose()?.unwrap_or_default();
    let stops = g.get("stops").map(parse_stops).transpose()?.unwrap_or_else(|| Gradient::default().stops);
    let mut gp = GradientPaint::new(Gradient::new(kind, stops));
    gp.gradient.dither = match g.get("dither") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or("`dither` must be true or false")?,
    };
    gp.gradient.method = method_param(g)?.unwrap_or_default();
    gp.angle = f64_or(g, "angle", 0.0);
    gp.swatch = str_param(g, "swatch").map(str::to_string);
    let geom = GeomEdit::parse(g)?;
    if let Some((start, end)) = geom.vector {
        let mut g = GradientGeom { start, end, aspect: geom.aspect.unwrap_or(1.0), focal: None };
        g.set_focal(geom.focal.flatten());
        gp.angle = g.angle_deg();
        gp.geom = Some(g);
    }
    if let Some(f) = g.get("freeform") {
        gp.freeform = Some(super::freeform::parse_freeform(f)?);
    }
    Ok(gp)
}

/// The `aspect` param (a percentage) as a ratio.
fn aspect_param(p: &Value) -> Parsed<Option<f64>> {
    p.get("aspect")
        .map(|v| v.as_f64().map(|a| (a / 100.0).clamp(0.005, 327.67)).ok_or_else(|| "`aspect` must be a number (%)".to_string()))
        .transpose()
}

/// `paint` without the placement it had on the art it came from (a swatch, or the default paint
/// for new art): a placed gradient keeps its angle and fits each object it lands on (freeform
/// points are placed afresh, coloured along the stops, which follow the points' colours).
pub(crate) fn unplaced(paint: &Paint) -> Paint {
    match paint {
        Paint::Gradient(g) if g.geom.is_some() || g.freeform.is_some() => {
            Paint::Gradient(Box::new(GradientPaint { geom: None, freeform: None, ..(**g).clone() }))
        }
        _ => paint.clone(),
    }
}

/// Is `p` applying a swatch (whose gradient placement belongs to the art it was saved from)?
fn applies_swatch(p: &Value) -> bool {
    p.get("swatch").is_some()
}

/// `paint` as applied to an object with `bounds`: a gradient given an `aspect` but no vector is
/// placed on the bounds so the aspect sticks, and a gradient swatch fits the object (keeping its
/// aspect) instead of the art it was saved from.
pub(crate) fn place_paint(paint: &Paint, p: &Value, bounds: Option<Rect>) -> Paint {
    let Paint::Gradient(g) = paint else { return paint.clone() };
    let aspect = match g.geom {
        Some(geom) if applies_swatch(p) => (geom.aspect != 1.0).then(|| json!(geom.aspect * 100.0)),
        // Freeform points given with the paint stay where they were put.
        _ if g.freeform.is_some() && !applies_swatch(p) => return paint.clone(),
        None => p.get("gradient").and_then(|g| g.get("aspect")).cloned(),
        Some(_) => return paint.clone(),
    };
    let fitted = unplaced(paint);
    match aspect {
        Some(a) => apply_gradient_edit(&fitted, &json!({ "aspect": a }), bounds).unwrap_or(fitted),
        None => fitted,
    }
}

/// Apply the gradient edits in `p` to `paint` (pure; unit-tested).
pub(crate) fn apply_gradient_edit(paint: &Paint, p: &Value, bounds: Option<Rect>) -> std::result::Result<Paint, String> {
    apply_gradient_edit_in(paint, p, bounds, &|_| true)
}

/// [`apply_gradient_edit`] on a shape that contains the points `inside` does: a gradient turned
/// freeform gets its first points there.
pub(crate) fn apply_gradient_edit_in(
    paint: &Paint,
    p: &Value,
    bounds: Option<Rect>,
    inside: &dyn Fn(Point) -> bool,
) -> std::result::Result<Paint, String> {
    let mut gp = match paint {
        Paint::Gradient(g) => (**g).clone(),
        _ => GradientPaint::new(Gradient::default()),
    };
    let mut switched = false;
    if let Some(k) = str_param(p, "kind") {
        let kind = parse_kind(k)?;
        if kind != gp.gradient.kind {
            gp.gradient.kind = kind;
            gp.geom = None;
            gp.freeform = None;
            switched = true;
        }
    }
    let recolor = p.get("stops").is_some() || bool_or(p, "reverse", false);
    if let Some(stops) = p.get("stops") {
        gp.gradient.stops = parse_stops(stops)?;
        gp.swatch = None;
    }
    if bool_or(p, "reverse", false) {
        // Midpoints mirrored with their segments.
        gp.gradient.reverse();
    }
    if let Some(d) = p.get("dither").and_then(Value::as_bool) {
        gp.gradient.dither = d;
    }
    if let Some(m) = method_param(p)? {
        gp.gradient.method = m;
    }
    // Angle and aspect edits keep the focal point in its place in the extent ellipse.
    let before = gp.geom;
    if let Some(a) = p.get("angle").and_then(Value::as_f64) {
        let a = ((a + 180.0).rem_euclid(360.0)) - 180.0;
        gp.angle = a;
        if let Some(g) = &mut gp.geom {
            let r = a.to_radians();
            let dir = vectorcraft_geom::Vec2::new(r.cos(), -r.sin());
            if gp.gradient.kind == GradientKind::Radial {
                g.end = g.start + dir * g.length();
            } else {
                let c = g.start.midpoint(g.end);
                let half = g.length() / 2.0;
                g.start = c - dir * half;
                g.end = c + dir * half;
            }
        }
    }
    if let Some(asp) = aspect_param(p)? {
        if gp.geom.is_none()
            && let Some(b) = bounds
        {
            gp.geom = Some(gp.resolve(b));
        }
        if let Some(g) = &mut gp.geom {
            g.aspect = asp;
        }
    }
    if let (Some(b), Some(g)) = (before, &mut gp.geom) {
        g.keep_focal_from(&b);
    }
    if gp.gradient.kind == GradientKind::Freeform {
        // Points already shown (automatic ones fitted to the box) stay put; a fresh freeform
        // gradient gets its points inside the shape.
        if let Some(b) = bounds {
            gp.seed_freeform(b, if switched { inside } else { &|_| true });
        }
        if recolor
            && !switched
            && let Some(f) = &mut gp.freeform
        {
            f.recolor(&gp.gradient);
            gp.gradient.stops = f.stops();
        }
    }
    if let Some(m) = str_param(p, "mode") {
        let mode = super::freeform::parse_mode(m)?;
        if gp.gradient.kind != GradientKind::Freeform {
            return Err("`mode` applies to freeform gradients".into());
        }
        gp.freeform.get_or_insert_with(Default::default).mode = mode;
    }
    Ok(Paint::Gradient(Box::new(gp)))
}

fn edit_gradient(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.editGradient";
    let item = item_target(s, p, C)?;
    let stroke = edits_stroke(s, p, item, !s.fill_active, C)?;
    check_switches(p, &["reverse", "dither"], C)?;
    check_numbers(p, &["angle"], C)?;
    let item = item.of_kind(s, !stroke);
    let ids = item.targets(s, p)?;
    // How the gradient lies on a stroke (the panel's Stroke buttons).
    let mode = match str_param(p, "strokeMode") {
        None => None,
        Some(_) if !stroke => return Err(bad(C, "`strokeMode` applies to strokes (stroke: true)")),
        Some(m) => Some(StrokeGradientMode::parse(m).ok_or_else(|| bad(C, format!("strokeMode must be within|along|across, got {m}")))?),
    };
    if p.get("dither").and_then(Value::as_bool).is_some() && stroke {
        return Err(bad(C, "`dither` applies to gradient fills (stroke: false)"));
    }
    super::paint::check_keep_model(p, C)?;
    // New RGB stop colors take the document's model; those of the swatches the stops name don't.
    let stored = stop_colors(s, p, p);
    // Stops naming swatches take their colours from them.
    let p = &link_stops(&s.doc()?.doc, p).map_err(|e| bad(C, e))?;
    // Validate against the defaults first so bad params fail without touching the document.
    let default_paint = if stroke { s.paint.stroke.clone() } else { s.paint.fill.clone() };
    let new_default = apply_gradient_edit(&default_paint, &with_stored_stops(p, &default_paint, &stored), None).map_err(|e| bad(C, e))?;
    let freeform = str_param(p, "kind").and_then(GradientKind::parse) == Some(GradientKind::Freeform);
    edit_items(s, &ids, item, C, "Gradient", !stroke, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            let lb = t.local_bounds();
            for r in &mut t.runs {
                let (cur, b) = run_paint_mut(r, stroke, lb);
                *cur = apply_gradient_edit(cur, &with_stored_stops(p, cur, &stored), Some(b)).map_err(|e| bad(C, e))?;
            }
            return Ok(());
        }
        let b = item_paint_bounds(n, index, !stroke);
        // Only a gradient turning freeform needs the shape (its first points go inside it).
        let inside = (freeform && b.is_some()).then(|| n.contains_fn());
        let inside: &dyn Fn(Point) -> bool = match &inside {
            Some(f) => f,
            None => &|_| true,
        };
        let old = n.appearance.paint_at(index, !stroke).unwrap_or(&Paint::None);
        let np = apply_gradient_edit_in(old, &with_stored_stops(p, old, &stored), b, inside).map_err(|e| bad(C, e))?;
        n.appearance.set_paint_at(index, !stroke, np);
        if let Some(m) = mode
            && let Some(st) = n.appearance.stroke_at_mut(index)
        {
            st.gradient_mode = m;
        }
        Ok(())
    })?;
    if let Some(m) = mode
        && ids.is_empty()
        && p.get("ids").is_none()
        && let Some(st) = s.new_art_stroke_mut()
    {
        st.gradient_mode = m;
    }
    // The last gradient is the one the first object now shows (the defaults' without objects).
    let shown = match ids.first() {
        Some(id) => s.doc()?.doc.node(*id).map(|n| super::paint::proxy_paint(n, stroke, item.resolve(&n.appearance, !stroke, C).ok().flatten())),
        None => Some(new_default.clone()),
    };
    if let Some(shown) = shown {
        s.remember_paint(&shown);
    }
    if stroke {
        s.paint.stroke = new_default;
    } else {
        s.paint.fill = new_default;
    }
    Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// A painted stroke on a type run without a weight gets 1 pt.
pub(crate) fn run_stroke_weight(style: &mut vectorcraft_doc::CharStyle) {
    if style.stroke_width == 0.0 {
        style.stroke_width = 1.0;
    }
}

/// A type run's fill or stroke paint and the box (text space) an unplaced gradient on it fits:
/// the layout bounds `lb`, grown by half the run's stroke weight for strokes.
pub(crate) fn run_paint_mut(r: &mut vectorcraft_doc::TextRun, stroke: bool, lb: Rect) -> (&mut Paint, Rect) {
    if stroke {
        let b = stroke_paint_bounds(lb, r.style.stroke_width);
        (&mut r.style.stroke, b)
    } else {
        (&mut r.style.fill, lb)
    }
}

/// The box an unplaced gradient on fill or stroke `index` of `n` fits (`None`: the topmost): the
/// geometric bounds, grown by half the weight for a stroke (`None` without that stroke).
pub(crate) fn item_paint_bounds(n: &Node, index: Option<usize>, fill: bool) -> Option<Rect> {
    let b = n.geometric_bounds()?;
    if fill { Some(b) } else { n.appearance.stroke_at(index).map(|st| st.paint_bounds(b)) }
}

/// `paint` as applied to a type run whose text space `xf` maps to the document: a vector given in
/// document coordinates moves into text space, and an aspect without a vector places the gradient
/// on the run's `bounds` (text space).
pub(crate) fn place_run_paint(paint: &Paint, p: &Value, xf: Affine, bounds: Rect) -> Paint {
    let mut out = place_paint(paint, p, Some(bounds));
    if let (Paint::Gradient(src), Paint::Gradient(g)) = (paint, &mut out)
        && (src.geom.is_some() || src.freeform.is_some())
        && !applies_swatch(p)
        && let Some(inv) = invert(xf)
    {
        g.transform(inv);
        g.angle = g.geom.map_or(g.angle, |geom| geom.angle_deg());
    }
    out
}

/// The inverse of `a`, if it has one.
fn invert(a: Affine) -> Option<Affine> {
    (a.determinant().abs() > 1e-12).then(|| a.inverse())
}

/// What `paint.setGradientGeom` sets, in document coordinates (None: kept).
struct GeomEdit {
    vector: Option<(Point, Point)>,
    aspect: Option<f64>,
    /// Some(None) centres the focal point.
    focal: Option<Option<Point>>,
}

impl GeomEdit {
    fn parse(p: &Value) -> std::result::Result<Self, String> {
        let point = |k: &str| p.get(k).map(|_| point_param(p, k).ok_or_else(|| format!("`{k}` must be [x, y]"))).transpose();
        let vector = match (point("start")?, point("end")?) {
            (Some(s), Some(e)) => Some((s, e)),
            (None, None) => None,
            _ => return Err("give both `start` and `end` [x, y] (or neither)".into()),
        };
        let focal = match p.get("focal") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(_) => Some(point("focal")?),
        };
        Ok(Self { vector, aspect: aspect_param(p)?, focal })
    }
}

/// `cur` with the vector, aspect and focal point of `e` (given in the document). `to_doc` maps
/// the paint's space (text space for type runs) to the document and `bounds` (in that space) fits
/// an unplaced gradient, so what the document shows carries over where `e` keeps it.
fn vector_paint(cur: &Paint, e: &GeomEdit, to_doc: Affine, bounds: Option<Rect>) -> Option<Paint> {
    let from_doc = invert(to_doc)?;
    let mut gp = match cur {
        Paint::Gradient(g) => (**g).clone(),
        _ => GradientPaint::new(Gradient::default()),
    };
    let kind = gp.gradient.kind;
    let map = |mut g: GradientGeom, a: Affine| {
        if a != Affine::IDENTITY {
            g.transform(a, kind);
        }
        g
    };
    let shown = gp.geom.or_else(|| bounds.map(|b| gp.resolve(b))).map(|g| map(g, to_doc));
    let (start, end) = e.vector.or_else(|| shown.map(|g| (g.start, g.end)))?;
    let aspect = e.aspect.or_else(|| shown.map(|g| g.aspect)).unwrap_or(1.0);
    let mut geom = GradientGeom { start, end, aspect, focal: None };
    match (e.focal, shown) {
        _ if kind != GradientKind::Radial => {}
        (Some(f), _) => geom.set_focal(f),
        (None, Some(b)) => geom.keep_focal_from(&b),
        (None, None) => {}
    }
    let geom = map(geom, from_doc);
    gp.angle = geom.angle_deg();
    gp.geom = Some(geom);
    Some(Paint::Gradient(Box::new(gp)))
}

fn set_gradient_geom(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.setGradientGeom";
    let edit = GeomEdit::parse(p).map_err(|e| bad(C, e))?;
    // `index` is an alias of `item`.
    let mut p = p.clone();
    if let (None, Some(i)) = (p.get("item"), p.get("index").cloned()) {
        p["item"] = i;
    }
    let p = &p;
    let item = item_target(s, p, C)?;
    let stroke = edits_stroke(s, p, item, !s.fill_active, C)?;
    let item = item.of_kind(s, !stroke);
    let targets = match item {
        ItemTarget::Top => {
            leaf_targets(s, &super::given_objects(s, p, C)?.map_or_else(|| selected_roots(s), |ids| Ok(outermost(&s.doc()?.doc, ids)))?)?
        }
        _ => item.targets(s, p)?,
    };
    if targets.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    edit_items(s, &targets, item, C, "Gradient", !stroke, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            let (xf, lb) = (t.xf, t.local_bounds());
            for r in &mut t.runs {
                if stroke {
                    run_stroke_weight(&mut r.style);
                }
                let (paint, b) = run_paint_mut(r, stroke, lb);
                if let Some(np) = vector_paint(paint, &edit, xf, Some(b)) {
                    *paint = np;
                }
            }
            return Ok(());
        }
        let b = item_paint_bounds(n, index, !stroke);
        if let Some(np) = vector_paint(n.appearance.paint_at(index, !stroke).unwrap_or(&Paint::None), &edit, Affine::IDENTITY, b) {
            n.appearance.set_paint_at(index, !stroke, np);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": targets.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
