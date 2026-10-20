//! The object model plug-ins read and write: paths and compound paths as JSON.
//!
//! The host hands a plug-in `{"objects": [...]}` (see [`encode`]) and reads the same shape back
//! ([`decode`]). Geometry and paint use the native file format's forms (`PathData`, `Paint`), so
//! nothing is lost on the way through; everything a plug-in returns is validated (finite,
//! in-range coordinates, capped counts, clamped colours) before it reaches the document.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Point, Rect};

use crate::{Error, Result};

/// Coordinates a plug-in may return, in points (the document's own canvas limit).
pub const MAX_COORD: f64 = 4.0e6;
/// Most objects one output may hold.
pub const MAX_OBJECTS: usize = 100_000;
/// Most anchor points one output may hold, over all objects.
pub const MAX_ANCHORS: usize = 4_000_000;
/// Most fills or strokes per object, gradient stops per gradient and freeform points or lines
/// per freeform gradient.
pub const MAX_ITEMS: usize = 256;
/// Thickest stroke a plug-in may set, in points.
pub const MAX_STROKE_WIDTH: f64 = 1000.0;

/// One object returned by a plug-in. `None` fields keep the object's value (new objects take the
/// defaults: a path, non-zero rule, no fills or strokes, full opacity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutObject {
    /// The input object this updates; `None`: a new object.
    pub id: Option<NodeId>,
    /// `"compound"` (true) or `"path"` (false).
    pub compound: Option<bool>,
    /// `Some("")` clears the name.
    pub name: Option<String>,
    pub path: Option<PathData>,
    pub rule: Option<FillRule>,
    /// Fill paints in paint order (bottom first).
    pub fills: Option<Vec<Paint>>,
    pub strokes: Option<Vec<OutStroke>>,
    pub opacity: Option<f32>,
}

/// One stroke of an [`OutObject`].
#[derive(Clone, Debug, PartialEq)]
pub struct OutStroke {
    pub paint: Paint,
    /// `None`: keep the stroke's weight (1 pt for a new stroke).
    pub width: Option<f64>,
}

fn rule_name(r: FillRule) -> &'static str {
    match r {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

fn rect_json(r: Option<Rect>) -> Value {
    r.map_or(Value::Null, |r| json!([r.x0, r.y0, r.x1, r.y1]))
}

fn paint_json(p: &Paint) -> Value {
    match p {
        Paint::None => Value::Null,
        p => serde_json::to_value(p).unwrap_or(Value::Null),
    }
}

/// The geometry of a path or compound path (a compound's members' subpaths in order) and its fill
/// rule. `None` for other objects.
pub fn geometry(n: &Node) -> Option<(PathData, FillRule, bool)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule, false)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule, true))
        }
        _ => None,
    }
}

/// The fills and strokes of appearance `ap` as plug-ins see them, each in paint order: a fill is
/// its paint, a stroke `{paint, width}`. Filters and live effects are given the same.
pub(crate) fn paint_lists(ap: &Appearance) -> (Vec<Value>, Vec<Value>) {
    let fills = ap.items.iter().filter_map(|i| if let AppearanceItem::Fill(f) = i { Some(paint_json(&f.paint)) } else { None }).collect();
    let strokes = ap
        .items
        .iter()
        .filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(json!({"paint": paint_json(&s.paint), "width": s.width})) } else { None })
        .collect();
    (fills, strokes)
}

/// A path or compound path as plug-ins see it (`None` for other objects): `{id, type, name?, path,
/// fillRule, fills, strokes, opacity, bounds}`.
pub fn encode(n: &Node) -> Option<Value> {
    let (path, rule, compound) = geometry(n)?;
    let (fills, strokes) = paint_lists(&n.appearance);
    let mut o = json!({
        "id": n.id.0, "type": if compound { "compound" } else { "path" }, "path": path, "fillRule": rule_name(rule),
        "fills": fills, "strokes": strokes, "opacity": n.opacity, "bounds": rect_json(path.bounds()),
    });
    if let Some(name) = &n.name {
        o["name"] = json!(name);
    }
    Some(o)
}

/// The input document of a filter run: `{"objects": [...]}` for `ids` (paths and compound paths;
/// others are left out).
pub fn encode_input(doc: &Document, ids: &[NodeId]) -> Vec<u8> {
    let objects: Vec<Value> = ids.iter().filter_map(|id| doc.node(*id)).filter_map(encode).collect();
    serde_json::to_vec(&json!({ "objects": objects })).unwrap_or_default()
}

fn out_err(at: &str, msg: impl std::fmt::Display) -> Error {
    Error::Output(format!("{at}: {msg}"))
}

fn check_point(p: Point, at: &str) -> Result<()> {
    if p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD {
        Ok(())
    } else {
        Err(out_err(at, format!("coordinate out of range (limit ±{MAX_COORD} pt)")))
    }
}

/// A path from plug-in output: native `PathData` JSON, every coordinate finite and in range, empty
/// subpaths dropped; `anchors` counts towards [`MAX_ANCHORS`].
fn decode_path(v: &Value, at: &str, anchors: &mut usize) -> Result<PathData> {
    let mut path: PathData = serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))?;
    path.subpaths.retain(|sp| !sp.anchors.is_empty());
    *anchors = anchors.saturating_add(path.anchor_count());
    if *anchors > MAX_ANCHORS {
        return Err(out_err(at, format!("too many anchor points (limit {MAX_ANCHORS})")));
    }
    for (_, _, a) in path.anchors() {
        for p in [a.p, a.h_in, a.h_out] {
            check_point(p, at)?;
        }
    }
    if path.is_empty() {
        return Err(out_err(at, "the path has no anchor points"));
    }
    Ok(path)
}

fn unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// `c` with its components in range.
fn clamp_color(c: Color) -> Color {
    match c {
        Color::Rgb { r, g, b } => Color::Rgb { r: unit(r), g: unit(g), b: unit(b) },
        Color::Cmyk { c, m, y, k } => Color::Cmyk { c: unit(c), m: unit(m), y: unit(y), k: unit(k) },
        Color::Gray { k } => Color::Gray { k: unit(k) },
        Color::Lab { l, a, b } => Color::Lab { l: l.clamp(0.0, 100.0), a: a.clamp(-128.0, 127.0), b: b.clamp(-128.0, 127.0) },
    }
}

/// A paint from plug-in output (`null`: none), with colours, opacities and offsets clamped and
/// gradient and pattern placements checked.
fn decode_paint(v: &Value, at: &str) -> Result<Paint> {
    if v.is_null() {
        return Ok(Paint::None);
    }
    let mut p: Paint = serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))?;
    match &mut p {
        Paint::None => {}
        Paint::Solid { color, tint, .. } => {
            *color = clamp_color(*color);
            *tint = unit(*tint);
        }
        Paint::Gradient(g) => {
            let stops = &mut g.gradient.stops;
            if stops.is_empty() || stops.len() > MAX_ITEMS {
                return Err(out_err(at, format!("a gradient needs 1-{MAX_ITEMS} stops")));
            }
            for s in stops.iter_mut() {
                s.offset = unit(s.offset);
                s.opacity = unit(s.opacity);
                s.midpoint = s.midpoint.clamp(0.13, 0.87);
                s.tint = unit(s.tint);
                s.color = clamp_color(s.color);
            }
            stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            g.angle = g.angle.rem_euclid(360.0);
            if let Some(geom) = &mut g.geom {
                check_point(geom.start, at)?;
                check_point(geom.end, at)?;
                if let Some(f) = geom.focal {
                    check_point(f, at)?;
                }
                geom.aspect = if geom.aspect.is_finite() { geom.aspect.clamp(0.01, 100.0) } else { 1.0 };
            }
            if let Some(ff) = &mut g.freeform {
                if ff.points.len() > MAX_ITEMS || ff.lines.len() > MAX_ITEMS || ff.lines.iter().any(|l| l.len() > MAX_ITEMS) {
                    return Err(out_err(at, format!("a freeform gradient holds at most {MAX_ITEMS} points and lines")));
                }
                let n = ff.points.len();
                if ff.lines.iter().flatten().any(|i| *i >= n) {
                    return Err(out_err(at, "a freeform gradient line names a point it doesn't have"));
                }
                for pt in &mut ff.points {
                    check_point(pt.at, at)?;
                    pt.color = clamp_color(pt.color);
                    pt.opacity = unit(pt.opacity);
                    pt.spread = unit(pt.spread);
                }
            }
        }
        Paint::Pattern { xf, .. } => {
            if !xf.as_coeffs().iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD) {
                return Err(out_err(at, "pattern transform out of range"));
            }
        }
    }
    Ok(p)
}

fn opt<'a>(o: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

fn decode_object(v: &Value, at: &str, anchors: &mut usize) -> Result<OutObject> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    let mut out = OutObject::default();
    if let Some(id) = opt(o, "id") {
        out.id = Some(NodeId(id.as_u64().ok_or_else(|| out_err(at, "`id` must be the id of an input object"))?));
    }
    if let Some(t) = opt(o, "type") {
        out.compound = Some(match t.as_str() {
            Some("path") => false,
            Some("compound") => true,
            _ => return Err(out_err(at, "`type` must be \"path\" or \"compound\"")),
        });
    }
    if let Some(n) = o.get("name") {
        let name = match n {
            Value::Null => String::new(),
            n => n.as_str().ok_or_else(|| out_err(at, "`name` must be a string"))?.chars().filter(|c| !c.is_control()).take(256).collect(),
        };
        out.name = Some(name);
    }
    if let Some(p) = opt(o, "path") {
        out.path = Some(decode_path(p, &format!("{at}.path"), anchors)?);
    }
    if let Some(r) = opt(o, "fillRule") {
        out.rule = Some(match r.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("nonzero") => FillRule::NonZero,
            Some("evenodd") => FillRule::EvenOdd,
            _ => return Err(out_err(at, "`fillRule` must be \"nonzero\" or \"evenodd\"")),
        });
    }
    let list = |key: &str| -> Result<Option<&Vec<Value>>> {
        match opt(o, key) {
            None => Ok(None),
            Some(Value::Array(a)) if a.len() <= MAX_ITEMS => Ok(Some(a)),
            Some(_) => Err(out_err(at, format!("`{key}` must be an array of at most {MAX_ITEMS} entries"))),
        }
    };
    if let Some(fills) = list("fills")? {
        out.fills = Some(fills.iter().enumerate().map(|(i, f)| decode_paint(f, &format!("{at}.fills[{i}]"))).collect::<Result<_>>()?);
    }
    if let Some(strokes) = list("strokes")? {
        let mut v = Vec::with_capacity(strokes.len());
        for (i, s) in strokes.iter().enumerate() {
            let at = format!("{at}.strokes[{i}]");
            let s = s.as_object().ok_or_else(|| out_err(&at, "a stroke must be {\"paint\", \"width\"}"))?;
            let paint = decode_paint(s.get("paint").unwrap_or(&Value::Null), &at)?;
            let width = match opt(s, "width") {
                None => None,
                Some(w) => Some(w.as_f64().ok_or_else(|| out_err(&at, "`width` must be a number"))?.clamp(0.0, MAX_STROKE_WIDTH)),
            };
            v.push(OutStroke { paint, width });
        }
        out.strokes = Some(v);
    }
    if let Some(op) = opt(o, "opacity") {
        out.opacity = Some(unit(op.as_f64().ok_or_else(|| out_err(at, "`opacity` must be a number"))? as f32));
    }
    Ok(out)
}

/// Parses a plug-in's output: `{"objects": [...]}`, or `{"error": "message"}` when the plug-in
/// declined (the message reaches the user).
pub fn decode(bytes: &[u8]) -> Result<Vec<OutObject>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    let objects = v.get("objects").and_then(Value::as_array).ok_or_else(|| Error::Output("expected {\"objects\": [...]}".into()))?;
    if objects.len() > MAX_OBJECTS {
        return Err(Error::Output(format!("{} objects (limit {MAX_OBJECTS})", objects.len())));
    }
    let mut anchors = 0;
    objects.iter().enumerate().map(|(i, o)| decode_object(o, &format!("objects[{i}]"), &mut anchors)).collect()
}

/// The decline in a plug-in's output `v`, `{"error": "…"}`: its message as the user reads it,
/// without control characters and at most 300 of them.
pub(crate) fn declined(v: &Value) -> Option<Error> {
    let e = v.get("error").filter(|e| !e.is_null())?;
    Some(Error::Failed(printable(e, 300)))
}

/// A plug-in's text the user reads: `v` as a string (any other JSON as its text), without control
/// characters, at most `max` characters of it.
pub(crate) fn printable(v: &Value, max: usize) -> String {
    let s = v.as_str().map_or_else(|| v.to_string(), str::to_string);
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// The paints of `ap`'s fills (`fill`) or strokes, in paint order.
fn paints(ap: &Appearance, fill: bool) -> Vec<&Paint> {
    ap.items.iter().filter(|i| i.is_fill() == fill).map(AppearanceItem::paint).collect()
}

/// `p` without swatch links: a plug-in's colours are its own, so only paints it returned
/// unchanged keep theirs.
fn unlinked(mut p: Paint) -> Paint {
    match &mut p {
        Paint::Solid { swatch, tint, .. } => {
            *swatch = None;
            *tint = 1.0;
        }
        Paint::Gradient(g) => {
            g.swatch = None;
            for s in &mut g.gradient.stops {
                s.swatch = None;
                s.tint = 1.0;
            }
        }
        Paint::None | Paint::Pattern { .. } => {}
    }
    p
}

/// `p` as it may enter `doc`: unchanged from `was` (the paint in the same slot before), or
/// without swatch links; a pattern must exist in the document.
fn admit(doc: &Document, p: Paint, was: Option<&Paint>, at: &str) -> Result<Paint> {
    if let Paint::Pattern { pattern, .. } = &p
        && doc.pattern(pattern).is_none()
    {
        return Err(out_err(at, format!("the document has no pattern {pattern:?}")));
    }
    Ok(if was == Some(&p) { p } else { unlinked(p) })
}

/// Sets the paints of `ap`'s fills (`fill`) or strokes to `new`, in paint order: extra items are
/// removed (from the top), missing ones added above the last item of their kind (fills at the
/// bottom, strokes on top when the object has none).
fn set_items(ap: &mut Appearance, fill: bool, new: Vec<(Paint, Option<f64>)>) {
    let idx: Vec<usize> = ap.items.iter().enumerate().filter(|(_, i)| i.is_fill() == fill).map(|(k, _)| k).collect();
    for &i in idx.iter().skip(new.len()).rev() {
        ap.remove_item(i);
    }
    let kept = new.len().min(idx.len());
    let mut at = idx.get(..kept).and_then(<[usize]>::last).map_or(if fill { 0 } else { ap.items.len() }, |i| i + 1);
    for (k, (paint, width)) in new.into_iter().enumerate() {
        match idx.get(k).and_then(|&i| ap.items.get_mut(i)) {
            Some(AppearanceItem::Fill(f)) => f.paint = paint,
            Some(AppearanceItem::Stroke(s)) => {
                s.paint = paint;
                if let Some(w) = width {
                    s.width = w;
                }
            }
            None => {
                let item = if fill {
                    AppearanceItem::Fill(FillLayer::new(paint))
                } else {
                    AppearanceItem::Stroke(StrokeLayer::new(paint, width.unwrap_or(1.0)))
                };
                ap.insert_item(at, item);
                at += 1;
            }
        }
    }
}

/// A compound path's members: one path per subpath of `path`, keeping the ids of `old` members.
fn members(doc: &mut Document, old: &[Arc<Node>], path: PathData) -> Vec<Arc<Node>> {
    path.subpaths
        .into_iter()
        .enumerate()
        .map(|(i, sp)| {
            let id = old.get(i).map_or_else(|| doc.alloc_id(), |c| c.id);
            Arc::new(Node::path(id, PathData::single(sp), Appearance::default()))
        })
        .collect()
}

/// Writes `o` into `n` (an input object, or a fresh node for a new one).
fn write(doc: &mut Document, n: &mut Node, o: OutObject, at: &str) -> Result<()> {
    let (cur_path, cur_rule, cur_compound) = geometry(n).unwrap_or((PathData::default(), FillRule::NonZero, false));
    let compound = o.compound.unwrap_or(cur_compound);
    let rule = o.rule.unwrap_or(cur_rule);
    match (&mut n.kind, compound) {
        (NodeKind::Path { path, rule: r, live, .. }, false) => {
            if let Some(p) = o.path {
                *path = p;
                *live = None;
            }
            *r = rule;
        }
        (NodeKind::Compound { children, rule: r }, true) => {
            if let Some(p) = o.path {
                *children = members(doc, children, p);
            }
            *r = rule;
        }
        (NodeKind::Compound { .. }, false) => {
            n.kind = NodeKind::Path { path: o.path.unwrap_or(cur_path), rule, live: None, clipping: false, guide: false };
        }
        (_, true) => n.kind = NodeKind::Compound { children: members(doc, &[], o.path.unwrap_or(cur_path)), rule },
        (_, false) => n.kind = NodeKind::Path { path: o.path.unwrap_or(cur_path), rule, live: None, clipping: false, guide: false },
    }
    for (fill, new) in [
        (true, o.fills.map(|f| f.into_iter().map(|p| (p, None)).collect::<Vec<_>>())),
        (false, o.strokes.map(|s| s.into_iter().map(|s| (s.paint, s.width)).collect())),
    ] {
        let Some(new) = new else { continue };
        let was = paints(&n.appearance, fill);
        let admitted = new
            .into_iter()
            .enumerate()
            .map(|(i, (p, w))| Ok((admit(doc, p, was.get(i).copied(), &format!("{at}.{}[{i}]", if fill { "fills" } else { "strokes" }))?, w)))
            .collect::<Result<Vec<_>>>()?;
        set_items(&mut n.appearance, fill, admitted);
    }
    if let Some(op) = o.opacity {
        n.opacity = op;
    }
    if let Some(name) = o.name {
        n.name = Some(name).filter(|s| !s.trim().is_empty());
    }
    Ok(())
}

/// The paths and compound paths a filter reads from the selection `selected`, in paint order:
/// selected ones, and those inside selected groups and layers; hidden, locked, guide and clipping
/// paths are left out (so are paths inside compound paths, blends and other live objects).
pub fn filter_targets(doc: &Document, selected: &[NodeId]) -> Vec<NodeId> {
    fn visit(list: &[Arc<Node>], sel: &HashSet<NodeId>, inside: bool, out: &mut Vec<NodeId>) {
        for n in list {
            if !n.visible || n.locked {
                continue;
            }
            let picked = inside || sel.contains(&n.id);
            match &n.kind {
                NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. } if picked => out.push(n.id),
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => visit(children, sel, picked, out),
                _ => {}
            }
        }
    }
    let sel: HashSet<NodeId> = selected.iter().copied().collect();
    let mut out = vec![];
    visit(&doc.layers, &sel, false, &mut out);
    out
}

/// The tree edits of [`apply_output`], made in one pass over the document.
#[derive(Default)]
struct Plan {
    replace: HashMap<NodeId, Node>,
    delete: HashSet<NodeId>,
    before: HashMap<NodeId, Vec<Node>>,
    after: HashMap<NodeId, Vec<Node>>,
    /// New objects that go on top of a container (`None`: the top-level list).
    top: Option<(Option<NodeId>, Vec<Node>)>,
}

impl Plan {
    fn pending(&self) -> bool {
        !(self.replace.is_empty() && self.delete.is_empty() && self.before.is_empty() && self.after.is_empty() && self.top.is_none())
    }

    /// The edited `list` (the children of `parent`), `None` when nothing in it changed.
    fn list(&mut self, parent: Option<NodeId>, list: &[Arc<Node>]) -> Option<Vec<Arc<Node>>> {
        let mut out: Option<Vec<Arc<Node>>> = None;
        for (i, c) in list.iter().enumerate() {
            if !self.pending() {
                if let Some(o) = &mut out {
                    o.extend(list.iter().skip(i).cloned());
                }
                return out;
            }
            let mut emitted: Vec<Arc<Node>> = self.before.remove(&c.id).unwrap_or_default().into_iter().map(Arc::new).collect();
            let mut changed = !emitted.is_empty();
            if self.delete.remove(&c.id) {
                changed = true;
            } else if let Some(r) = self.replace.remove(&c.id) {
                emitted.push(Arc::new(r));
                changed = true;
            } else if let Some(ch) = match &c.kind {
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => self.list(Some(c.id), children),
                _ => None,
            } {
                let mut m = (**c).clone();
                if let Some(slot) = m.children_mut() {
                    *slot = ch;
                }
                emitted.push(Arc::new(m));
                changed = true;
            } else {
                emitted.push(c.clone());
            }
            if let Some(a) = self.after.remove(&c.id) {
                emitted.extend(a.into_iter().map(Arc::new));
                changed = true;
            }
            if changed && out.is_none() {
                out = Some(list.iter().take(i).cloned().collect());
            }
            if let Some(o) = &mut out {
                o.extend(emitted);
            }
        }
        if self.top.as_ref().is_some_and(|(p, _)| *p == parent)
            && let Some((_, nodes)) = self.top.take()
        {
            out.get_or_insert_with(|| list.to_vec()).extend(nodes.into_iter().map(Arc::new));
        }
        out
    }
}

/// Applies an object filter's output to `doc`.
///
/// `inputs` are the objects the plug-in was given, in paint order ([`filter_targets`]). Output
/// objects with an `id` update that input object (keys they leave out keep their values); objects
/// without one are new and go just above the object listed before them, or, listed before any
/// input object, just below the first one listed after them (at the first input object's place
/// when the output keeps none; on top of `parent` when there were no inputs). Input objects missing
/// from the output are deleted. Returns the resulting objects in output order (to select).
pub fn apply_output(doc: &mut Document, inputs: &[NodeId], out: Vec<OutObject>, parent: Option<NodeId>) -> Result<Vec<NodeId>> {
    let wanted: HashSet<NodeId> = inputs.iter().copied().collect();
    let mut originals: HashMap<NodeId, Node> = HashMap::new();
    doc.walk(|n| {
        if wanted.contains(&n.id) {
            originals.insert(n.id, n.clone());
        }
    });
    let mut plan = Plan::default();
    let mut result = Vec::with_capacity(out.len());
    // The run of new objects since the last input object listed, and that object.
    let mut run: Vec<Node> = vec![];
    let mut prev: Option<NodeId> = None;
    for (i, o) in out.into_iter().enumerate() {
        let at = format!("objects[{i}]");
        match o.id {
            Some(id) => {
                let mut n = originals.remove(&id).ok_or_else(|| {
                    let why = if wanted.contains(&id) { "is listed twice" } else { "is not the id of an input object" };
                    out_err(&at, format!("{} {why}", id.0))
                })?;
                write(doc, &mut n, o, &at)?;
                if !run.is_empty() {
                    let nodes = std::mem::take(&mut run);
                    match prev {
                        Some(p) => plan.after.insert(p, nodes),
                        None => plan.before.insert(id, nodes),
                    };
                }
                plan.replace.insert(id, n);
                prev = Some(id);
                result.push(id);
            }
            None => {
                if o.path.is_none() {
                    return Err(out_err(&at, "a new object needs a `path`"));
                }
                let mut n = Node::path(doc.alloc_id(), PathData::default(), Appearance::default());
                write(doc, &mut n, o, &at)?;
                result.push(n.id);
                run.push(n);
            }
        }
    }
    if !run.is_empty() {
        match prev.or(inputs.first().copied()) {
            Some(p) if prev.is_some() => {
                plan.after.insert(p, run);
            }
            Some(first) => {
                plan.before.insert(first, run);
            }
            None => plan.top = Some((parent, run)),
        }
    }
    // Input objects the output left out are deleted.
    plan.delete = originals.into_keys().collect();
    if let Some(list) = plan.list(None, &doc.layers) {
        doc.layers = list;
    }
    if plan.pending() {
        return Err(Error::Output("the objects to change are no longer in the document".into()));
    }
    Ok(result)
}

/// The geometry of an effect's output: every returned object's path, in order.
pub fn output_geometry(objects: Vec<OutObject>) -> PathData {
    PathData::new(objects.into_iter().filter_map(|o| o.path).flat_map(|p| p.subpaths).collect())
}

#[cfg(test)]
mod tests;
