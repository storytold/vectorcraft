use std::collections::HashMap;

use vectorcraft_color::Swatch;
use vectorcraft_doc::ImageObject;

use super::{
    Appearance, AppearanceItem, Arc, Beside, Color, Document, EngineError, LineCap, LineJoin, LiveShape, Node, NodeId, NodeKind, Paint, Point, Rect,
    Result, Session, StrokeLayer, TRACE_NAME, TraceView, Value, bad, fill_only, ids_json, json, root_ids, shape_node, str_param, switch_or, tr,
};

#[cfg(test)]
thread_local! {
    static BOX_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CANDIDATE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// ---------- Image Trace ----------

fn is_trace_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(TRACE_NAME)
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

const LIVE_PATHFINDER_BY_COLOR: &str = "byColor can't regroup a trace with a live Pathfinder effect: expand without byColor";

/// The `view` param ([`TraceView`] id), if given.
fn view_param(cmd: &str, p: &Value) -> Result<Option<TraceView>> {
    let Some(v) = p.get("view").filter(|v| !v.is_null()) else { return Ok(None) };
    let ids = || TraceView::ALL.map(TraceView::id).join(", ");
    v.as_str().and_then(TraceView::from_id).map(Some).ok_or_else(|| bad(cmd, format!("view must be one of {}", ids())))
}

/// A saved Image Trace preset ([`crate::Prefs::trace_presets`]): a name and its settings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracePreset {
    pub name: String,
    #[serde(default)]
    pub params: tr::TraceParams,
}

/// A change to the saved tracing presets: `imageTrace.presets.save` saved `name` (made it,
/// changed its settings, or renamed it from `renamed_from`), or `imageTrace.presets.delete`
/// deleted it. `revision` is [`Session::trace_presets_revision`] after the change.
#[derive(Clone, Debug, PartialEq)]
pub struct TracePresetChange {
    pub revision: u64,
    pub name: String,
    pub renamed_from: Option<String>,
    pub deleted: bool,
}

/// The revision of the saved tracing presets and their latest changes ([`TracePresetChange`]).
#[derive(Default)]
pub(crate) struct TracePresetChanges {
    revision: u64,
    /// Every change the commands made in this session, oldest first, so a panel that was closed
    /// meanwhile still follows a rename (one record for each save or delete).
    changes: Vec<TracePresetChange>,
}

impl TracePresetChanges {
    /// A change the commands made: a new revision that records it.
    fn note(&mut self, mut change: TracePresetChange) {
        self.revision += 1;
        change.revision = self.revision;
        self.changes.push(change);
    }

    /// A change made by replacing the preferences: a new revision with no record.
    pub(crate) fn replaced(&mut self) {
        self.revision += 1;
    }
}

impl Session {
    /// A number that grows each time the saved tracing presets change: saved, renamed or deleted
    /// by `imageTrace.presets.save` and `.delete`, or replaced with the preferences
    /// ([`Session::apply_prefs`]). A list read at one revision holds until the number changes.
    pub fn trace_presets_revision(&self) -> u64 {
        self.trace_preset_changes.revision
    }

    /// The changes `imageTrace.presets.save` and `.delete` made after `revision`, oldest first.
    pub fn trace_preset_changes_since(&self, revision: u64) -> impl Iterator<Item = &TracePresetChange> {
        let changes = &self.trace_preset_changes.changes;
        let from = changes.partition_point(|c| c.revision <= revision);
        changes.get(from..).unwrap_or_default().iter()
    }

    /// The saved tracing preset `name`, any case.
    pub fn trace_preset(&self, name: &str) -> Option<&TracePreset> {
        self.prefs.trace_presets.iter().find(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The index of the saved tracing preset `name`.
    fn trace_preset_index(&self, name: &str) -> Option<usize> {
        self.prefs.trace_presets.iter().position(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Whether `name` is a built-in preset's, "Custom" (changed settings, as the panel and a traced
    /// object show them), or a saved preset's other than the one at `except`.
    fn trace_preset_taken(&self, name: &str, except: Option<usize>) -> bool {
        tr::preset(name).is_some() || name.trim().eq_ignore_ascii_case("Custom") || self.trace_preset_index(name).is_some_and(|i| Some(i) != except)
    }

    /// The first free "Tracing Preset N": the name a new tracing preset gets.
    pub fn new_trace_preset_name(&self) -> String {
        (1..).map(|i| format!("Tracing Preset {i}")).find(|n| !self.trace_preset_taken(n, None)).unwrap_or_default()
    }
}

pub(super) fn trace_presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.presets.save";
    // A name or preset of the wrong type is an error, not dropped: dropping it would save other
    // settings, or under another name.
    let new_name = opt_str(C, p, "newName")?;
    let preset = opt_str(C, p, "preset")?;
    let name = match opt_str(C, p, "name")?.map(str::trim) {
        Some("") => return Err(bad(C, "`name` is empty")),
        Some(n) => n.to_string(),
        None => s.new_trace_preset_name(),
    };
    if tr::preset(&name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and can't change: save it under another name")));
    }
    if name.eq_ignore_ascii_case("Custom") {
        return Err(bad(C, "`Custom` stands for changed settings: save the preset under another name"));
    }
    let at = s.trace_preset_index(&name);
    // The saved preset's name as it was saved.
    let old = at.and_then(|i| s.prefs.trace_presets.get(i)).map(|q| q.name.clone());
    if old.is_none() && new_name.is_some() {
        return Err(bad(C, format!("no saved preset named `{name}` to rename")));
    }
    // A saved preset changes from its own settings unless `preset` says where to start.
    let mut q = p.clone();
    if let (Some(old), Some(o), None) = (&old, q.as_object_mut(), preset) {
        o.insert("preset".into(), json!(old));
    }
    let params = trace_params(s, C, &q)?;
    let name = match new_name.map(str::trim) {
        Some("") => return Err(bad(C, "`newName` is empty")),
        Some(n) if s.trace_preset_taken(n, at) => return Err(bad(C, format!("a preset named `{n}` exists"))),
        Some(n) => n.to_string(),
        None => old.clone().unwrap_or(name),
    };
    let json_params = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    let preset = TracePreset { name: name.clone(), params };
    match at.and_then(|i| s.prefs.trace_presets.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => s.prefs.trace_presets.push(preset),
    }
    let renamed_from = old.as_ref().filter(|o| **o != name).cloned();
    s.trace_preset_changes.note(TracePresetChange { revision: 0, name: name.clone(), renamed_from: renamed_from.clone(), deleted: false });
    Ok(json!({"name": name, "params": json_params, "created": old.is_none(), "renamedFrom": renamed_from}))
}

pub(super) fn trace_presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.presets.delete";
    let name = opt_str(C, p, "name")?.ok_or_else(|| bad(C, "missing `name`"))?;
    if tr::preset(name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and stays")));
    }
    let i = s.trace_preset_index(name).ok_or_else(|| bad(C, format!("no saved preset named `{name}`")))?;
    let deleted = s.prefs.trace_presets.remove(i).name;
    s.trace_preset_changes.note(TracePresetChange { revision: 0, name: deleted.clone(), renamed_from: None, deleted: true });
    Ok(json!({ "deleted": deleted }))
}

/// The string `key` of `p` (`None` when it is missing or null); any other type is an error of
/// command `c`.
fn opt_str<'a>(c: &str, p: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v)),
        Some(_) => Err(bad(c, format!("`{key}` must be a string"))),
    }
}

/// The parameters command `c` traces with: the preset's (built-in, else the `savedPreset`
/// settings a journal entry records, else saved), with `params` over them.
fn trace_params(s: &Session, c: &str, p: &Value) -> Result<tr::TraceParams> {
    let name = opt_str(c, p, "preset")?.unwrap_or("Default");
    let enhanced = switch_or(p, "enhanced", false, c)?;
    let recorded = match p.get("savedPreset").filter(|v| !v.is_null()) {
        Some(v) => Some(serde_json::from_value::<tr::TraceParams>(v.clone()).map_err(|e| bad(c, format!("savedPreset: {e}")))?),
        None => None,
    };
    // (A preset without an enhanced version, a saved one too, is traced as it is.)
    let base = enhanced.then(|| tr::enhanced_preset(name)).flatten().or_else(|| tr::preset(name));
    let base = base.or(recorded).or_else(|| s.trace_preset(name).map(|q| q.params.clone()));
    let base = base.ok_or_else(|| bad(c, format!("unknown preset `{name}`: not a built-in or saved preset (see imageTrace.presets)")))?;
    let params: tr::TraceParams = match p.get("params").filter(|v| !v.is_null()) {
        None => base,
        Some(over) => {
            let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
            let (Some(o), Some(src)) = (v.as_object_mut(), over.as_object()) else { return Err(bad(c, "params must be an object")) };
            // A key that isn't one of the settings is an error, with the nearest setting.
            if let Some(key) = src.keys().find(|k| !o.contains_key(k.as_str())) {
                return Err(crate::cmd::params::unknown(c, key, &o.keys().map(String::as_str).collect::<Vec<_>>()));
            }
            for (k, val) in src {
                o.insert(k.clone(), val.clone());
            }
            serde_json::from_value(v).map_err(|e| bad(c, e.to_string()))?
        }
    };
    // (Recorded and saved settings are checked too.)
    if !params.fills && !params.strokes {
        return Err(bad(c, "Create needs fills, strokes or both"));
    }
    if !(params.stroke_width.is_finite() && params.stroke_width > 0.0) {
        return Err(bad(c, "strokeWidth must be a positive number of pixels"));
    }
    // A color group is refused unless the trace uses it: Color mode with Document Library and the
    // document's swatches.
    if params.color_group.as_deref().is_some_and(|g| !g.is_empty()) {
        if params.palette != tr::Palette::DocumentLibrary {
            return Err(bad(c, "`colorGroup` needs `palette: \"documentLibrary\"`"));
        }
        if params.library != crate::cmd::swatchlib::DOCUMENT_SWATCHES {
            return Err(bad(c, "`colorGroup` needs `library: \"document\"`"));
        }
        if params.mode != tr::Mode::Color {
            return Err(bad(c, "`colorGroup` needs `mode: \"color\"`"));
        }
    }
    Ok(params)
}

/// Document swatch `w`'s displayed solid color and unlinked trace paint. A tint uses its tinted
/// color; a Lab spot color uses its CMYK equivalent when the Spot Colors options use CMYK values.
fn trace_swatch_paint(d: &Document, w: &Swatch) -> Option<(Color, Paint)> {
    let linked = d.swatch_link(&w.name).and_then(|(base, tint)| d.tint_paint(&base, tint));
    let color = linked.as_ref().and_then(Paint::color).or_else(|| w.paint.color())?;
    Some((color, Paint::solid(color)))
}

/// Document Library with `colorGroup`: the solid colors of the active document's color group
/// `name` (so named exactly, else ignoring case), each with its unlinked paint ([`trace_swatch_paint`]).
/// Errors name command `c`. ([`trace_params`] has checked that the library is the document's
/// swatches.)
fn trace_group_colors(c: &str, s: &Session, name: &str) -> Result<Vec<(Color, Paint)>> {
    let d = &s.doc()?.doc;
    let groups = &d.swatch_groups;
    let lower = name.to_lowercase();
    let group = groups
        .iter()
        .find(|g| g.name == name)
        .or_else(|| groups.iter().find(|g| g.name.to_lowercase() == lower))
        .ok_or_else(|| bad(c, format!("no color group `{name}` in the document (see swatch.list)")))?;
    Ok(group.swatches.iter().filter_map(|w| trace_swatch_paint(d, w)).collect())
}

/// Document Library in Color mode: look the library's colors (or its color group's) up into
/// `params.swatches` → each one as traced (sRGB) with its unlinked paint. Errors name command `c`.
fn trace_library(c: &str, s: &Session, params: &mut tr::TraceParams) -> Result<Vec<([u8; 3], Paint)>> {
    if params.mode != tr::Mode::Color || params.palette != tr::Palette::DocumentLibrary {
        return Ok(vec![]);
    }
    let group = params.color_group.as_deref().filter(|g| !g.is_empty());
    let key = group.unwrap_or(params.library.as_str());
    let missing = || bad(c, format!("no swatch library `{key}` (see swatch.library.list; \"document\": the document's swatches)"));
    let colors: Vec<(Color, Paint)> = match group {
        Some(name) => trace_group_colors(c, s, name)?,
        None if key == crate::cmd::swatchlib::DOCUMENT_SWATCHES => {
            let d = &s.active().ok_or_else(missing)?.doc;
            d.swatches_iter().filter_map(|w| trace_swatch_paint(d, w)).collect()
        }
        None => crate::cmd::swatchlib::library_colors(s, key).ok_or_else(missing)?.into_iter().map(|c| (c, Paint::solid(c))).collect(),
    };
    if colors.is_empty() {
        return Err(bad(c, format!("`{key}` has no colours")));
    }
    let library: Vec<([u8; 3], Paint)> = colors
        .into_iter()
        .map(|(c, paint)| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            ([r, g, b], paint)
        })
        .collect();
    params.swatches = library.iter().map(|l| l.0).collect();
    Ok(library)
}

/// The most anchor points an Image Trace may make, a guard for memory: a trace past it is refused
/// with advice instead (#525). A trace keeps about 90 bytes an anchor in the document (measured on
/// code-drawn photos of 2000 × 2000 to 6000 × 4000 pixels), so 8,000,000 anchors are about
/// 0.7 GB, with a peak of about 1 GB while tracing: a 4000 × 3000 photo at High Fidelity Photo
/// makes about 1.9 million, and even at Paths 100, Noise 1 and every tone about 7.9 million. On the
/// web, where a page has at most 4 GB of memory, the limit stays at 2,000,000.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_TRACE_ANCHORS: usize = 8_000_000;

/// The most anchor points an Image Trace may make on the web (see the native limit).
#[cfg(target_arch = "wasm32")]
pub const MAX_TRACE_ANCHORS: usize = 2_000_000;

/// The live shape for a shape the tracer found (in the image's pixels), placed in the document by `xf`.
fn traced_shape(shape: tr::TracedShape, xf: vectorcraft_geom::Affine) -> LiveShape {
    use vectorcraft_geom::Affine;
    let about = |c: Point, deg: f64| Affine::translate(c.to_vec2()) * Affine::rotate(deg.to_radians());
    let mut live = match shape {
        tr::TracedShape::Rectangle { center, width, height, radius, rotation } => LiveShape::Rectangle {
            w: width,
            h: height,
            radii: [radius; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: about(center, rotation) * Affine::translate((-width / 2.0, -height / 2.0)),
        },
        tr::TracedShape::Ellipse { center, rx, ry, rotation } => {
            LiveShape::Ellipse { w: 2.0 * rx, h: 2.0 * ry, pie: (0.0, 360.0), xf: about(center, rotation) * Affine::translate((-rx, -ry)) }
        }
        tr::TracedShape::Polygon { center, radius, sides, rotation } => {
            LiveShape::Polygon { radius, sides, xf: about(center, rotation), radii: vec![], kinds: vec![], radii_relative: false }
        }
    };
    // (Whether the move changed the shape's own numbers does not matter: its path is made from them next.)
    let _ = live.transform(xf);
    live
}

/// The gradient fill for a linear gradient the tracer found (in the image's pixels), placed in the
/// document by `xf`.
fn traced_gradient(g: &tr::TracedGradient, xf: vectorcraft_geom::Affine) -> Paint {
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = g.stops.iter().map(|&(at, c)| GradientStop::new(at as f32, Color::rgb8(c[0], c[1], c[2]))).collect();
    let mut paint = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    paint.geom = Some(GradientGeom { start: xf * g.start, end: xf * g.end, aspect: 1.0, focal: None });
    Paint::Gradient(Box::new(paint))
}

/// Auto Grouping: `nodes` (one per traced path, bottom-most first) with each path that has others
/// inside it (`parents`, as the tracer gives them) in a group with them, in the order they had:
/// abutting paths painted in another order would show a seam where they meet. Without parents,
/// the nodes as they are.
fn grouped(d: &mut Document, nodes: Vec<Node>, parents: &[Option<usize>]) -> Vec<Arc<Node>> {
    let n = nodes.len();
    let mut inside: Vec<Vec<usize>> = vec![vec![]; n];
    let mut roots = vec![];
    for i in 0..n {
        match parents.get(i).copied().flatten().and_then(|p| inside.get_mut(p).filter(|_| p != i)) {
            Some(of) => of.push(i),
            None => roots.push(i),
        }
    }
    // Outermost first, so that backwards each path comes after those inside it.
    let mut order = roots.clone();
    let mut at = 0;
    while let Some(&i) = order.get(at) {
        order.extend(inside.get(i).into_iter().flatten());
        at += 1;
    }
    let mut nodes: Vec<Option<Node>> = nodes.into_iter().map(Some).collect();
    let mut built: Vec<Option<Arc<Node>>> = vec![None; n];
    for &i in order.iter().rev() {
        let Some(node) = nodes.get_mut(i).and_then(Option::take) else { continue };
        let mut within: Vec<(usize, Arc<Node>)> = inside.get(i).into_iter().flatten().filter_map(|&c| Some((c, built.get_mut(c)?.take()?))).collect();
        let made = if within.is_empty() {
            node
        } else {
            within.push((i, Arc::new(node)));
            within.sort_by_key(|m| m.0);
            Node::group(d.alloc_id(), within.into_iter().map(|m| m.1).collect())
        };
        if let Some(slot) = built.get_mut(i) {
            *slot = Some(Arc::new(made));
        }
    }
    // (Paths that are inside each other in a ring are reached from no root: the tracer makes none,
    // and they are kept all the same.)
    let ring = nodes.into_iter().flatten().map(Arc::new).collect::<Vec<_>>();
    roots.iter().filter_map(|&r| built.get_mut(r)?.take()).chain(ring).collect()
}

/// The `byColor` param of an Expand (false when not given).
fn by_color_param(cmd: &str, p: &Value) -> Result<bool> {
    match p.get("byColor").filter(|v| !v.is_null()) {
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "byColor must be true or false")),
        None => Ok(false),
    }
}

/// Expand by color: an expanded trace's `items` (bottom-most first) in a group for each paint, a
/// stroked line counted by its stroke. A solid color's group is named by the swatch it links to (a
/// tint with its percentage) or as `#rrggbb`, and gradient fills share one group, "Gradients". The
/// groups stack in the order of the lowest item in each, and each keeps its items' order. When the
/// paths meet, an item joins its paint's last group only if no item of a later group meets it
/// (by the boxes of everything they paint, effects included, within one point); otherwise it
/// starts another group of that paint, which keeps the stacking order of those items. Items
/// farther apart can still share antialiased pixels at small view or raster-export scales, so
/// their edge pixels may change. A group among `items` (Auto Grouping) is refused.
fn color_groups(cmd: &str, d: &mut Document, items: Vec<Arc<Node>>) -> Result<Vec<Arc<Node>>> {
    use std::collections::{BTreeMap, HashMap};
    use std::ops::Bound::{Excluded, Unbounded};
    use vectorcraft_color::tint_percent;
    // Grouped by the paint: unlinked (its #rrggbb), linked (the swatch and its exact tint), a
    // gradient, or another paint (its label). The name is apart from it: a swatch may be named as
    // a color's `#rrggbb` or as "Gradients".
    type PaintKey = (u8, String, u32);
    struct Run {
        name: String,
        members: Vec<Arc<Node>>,
    }
    // Boxes closer than a point count as overlapping: their antialiased edges can share pixels.
    let meet = |a: &Rect, b: &Rect| {
        #[cfg(test)]
        BOX_TESTS.with(|n| n.set(n.get() + 1));
        let b = b.inflate(1.0, 1.0);
        a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
    };
    // Choose a cell size from the painted extent and path count. A box over more than eight cells
    // across but at most eight down (a stripe) is indexed by its rows, one the other way round by
    // its columns, and one over more than eight both ways stays in a separate list, keeping the
    // index bounded even when a path covers most of the document.
    let items: Vec<_> = items
        .into_iter()
        .map(|item| {
            let bounds = vectorcraft_render::painted_bounds(d, &item);
            (item, bounds)
        })
        .collect();
    let extent = items.iter().filter_map(|(_, b)| *b).reduce(|a, b| a.union(b));
    let cell_size = extent.map_or(1.0, |r| ((r.x1 - r.x0).abs() * (r.y1 - r.y0).abs() / items.len().max(1) as f64).sqrt().max(1.0));
    // The first and last column and row of the cells `r` covers; `None` when it isn't finite.
    let span = |r: Rect| -> Option<(i64, i64, i64, i64)> {
        let coord = |v: f64| (v / cell_size).floor() as i64;
        [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then(|| (coord(r.x0), coord(r.y0), coord(r.x1), coord(r.y1)))
    };
    let count = |first: i64, last: i64| i128::from(last) - i128::from(first) + 1;
    // A box over more cells than this is checked against every box.
    const MAX_CELLS: i128 = 1024;
    let mut groups: Vec<Run> = vec![];
    let mut latest: HashMap<PaintKey, usize> = HashMap::new();
    // An item can join an earlier group, so insertion order is not group-index order. Index
    // boxes by group to skip older groups without scanning their entries.
    let mut spatial: HashMap<(i64, i64), BTreeMap<usize, Vec<Rect>>> = HashMap::new();
    let mut rows: HashMap<i64, BTreeMap<usize, Vec<Rect>>> = HashMap::new();
    let mut columns: HashMap<i64, BTreeMap<usize, Vec<Rect>>> = HashMap::new();
    let mut large: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
    let mut all_boxes: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
    for (item, bounds) in items {
        if matches!(item.kind, NodeKind::Group { .. }) {
            return Err(bad(cmd, "byColor can't split Auto Grouping groups: trace without Auto Grouping"));
        }
        let paint = match item.appearance.fill_paint() {
            Paint::None => item.appearance.stroke_paint(),
            fill => fill,
        };
        let (key, name) = match &paint {
            Paint::Solid { swatch: Some(n), tint, .. } => {
                let percent = tint_percent(*tint);
                let name = if *tint < 1.0 { format!("{n} {percent}%") } else { n.clone() };
                ((1, n.clone(), tint.to_bits()), name)
            }
            Paint::Solid { color, .. } => ((0, color.to_hex(), 0), color.to_hex()),
            Paint::Gradient(_) => ((2, String::new(), 0), "Gradients".to_string()),
            other => ((3, other.label(), 0), other.label()),
        };
        // Joining an earlier group moves the item under every later group's items.
        let hidden_by_later = |at: usize| {
            bounds.is_some_and(|b| {
                let overlaps_later = |boxes: &BTreeMap<usize, Vec<Rect>>| {
                    boxes.range((Excluded(at), Unbounded)).rev().any(|(_, group_boxes)| {
                        group_boxes.iter().rev().any(|box_bounds| {
                            #[cfg(test)]
                            CANDIDATE_VISITS.with(|n| n.set(n.get() + 1));
                            meet(box_bounds, &b)
                        })
                    })
                };
                match span(b) {
                    Some((x0, y0, x1, y1)) if count(x0, x1) * count(y0, y1) <= MAX_CELLS => {
                        (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).filter_map(|c| spatial.get(&c)).any(overlaps_later)
                            || (y0..=y1).filter_map(|y| rows.get(&y)).any(overlaps_later)
                            || (x0..=x1).filter_map(|x| columns.get(&x)).any(overlaps_later)
                            || overlaps_later(&large)
                    }
                    _ => overlaps_later(&all_boxes),
                }
            })
        };
        let at = latest.get(&key).copied().filter(|&at| !hidden_by_later(at));
        let mut started = Run { name, members: vec![] };
        let joins = at.and_then(|at| groups.get_mut(at));
        let new = joins.is_none();
        let g = joins.unwrap_or(&mut started);
        g.members.push(item);
        if new {
            groups.push(started);
            latest.insert(key, groups.len() - 1);
        }
        if let Some(b) = bounds {
            let index = if new { groups.len() - 1 } else { at.unwrap_or_default() };
            all_boxes.entry(index).or_default().push(b);
            match span(b.inflate(1.0, 1.0)) {
                Some((x0, y0, x1, y1)) if count(x0, x1) <= 8 && count(y0, y1) <= 8 => {
                    for place in (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))) {
                        spatial.entry(place).or_default().entry(index).or_default().push(b);
                    }
                }
                Some((_, y0, _, y1)) if count(y0, y1) <= 8 => {
                    for y in y0..=y1 {
                        rows.entry(y).or_default().entry(index).or_default().push(b);
                    }
                }
                Some((x0, _, x1, _)) if count(x0, x1) <= 8 => {
                    for x in x0..=x1 {
                        columns.entry(x).or_default().entry(index).or_default().push(b);
                    }
                }
                _ => large.entry(index).or_default().push(b),
            }
        }
    }
    Ok(groups
        .into_iter()
        .map(|run| {
            let mut g = Node::group(d.alloc_id(), run.members);
            g.name = Some(run.name);
            Arc::new(g)
        })
        .collect())
}

/// What a trace depends on: the object traced (an image, or an Image Trace object traced again),
/// its image, and the settings it is traced with. The tracer makes the same paths of the same
/// inputs, so a trace made away from the command ([`TraceRun`]) stands in for the command's own.
#[derive(Clone, Debug, PartialEq)]
struct TraceInputs {
    target: NodeId,
    image: Box<ImageObject>,
    /// The settings traced with, as JSON.
    params: Value,
    /// Document Library: the library's colours, which the JSON leaves out
    /// ([`tr::TraceParams::swatches`]).
    swatches: Vec<[u8; 3]>,
}

/// What `imageTrace.make` and `imageTrace.makeAndExpand` take from the session before they trace.
struct Prepared {
    inputs: TraceInputs,
    trace: tr::TraceParams,
    view: Option<TraceView>,
    /// The settings asked for, which the Image Trace object keeps.
    params_json: Value,
    preset_label: String,
    library: Vec<([u8; 3], Paint)>,
    bytes: Arc<Vec<u8>>,
}

/// What command `c` with params `p` traces, and how, from the session: everything but the trace.
fn prepare(s: &Session, c: &str, p: &Value) -> Result<Prepared> {
    let mut params = trace_params(s, c, p)?;
    let view = view_param(c, p)?;
    let by_color = c == "imageTrace.makeAndExpand" && by_color_param(c, p)?;
    if by_color && params.auto_grouping {
        return Err(bad(c, "byColor can't split Auto Grouping groups: trace without Auto Grouping"));
    }
    let params_json = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    // The preset's name if the parameters are exactly that preset's or its enhanced version's (a
    // saved preset's under its saved name), else "Custom".
    let named = str_param(p, "preset").unwrap_or("Default");
    let is = |b: Option<tr::TraceParams>| b.and_then(|b| serde_json::to_value(&b).ok()).as_ref() == Some(&params_json);
    let saved = s.trace_preset(named).filter(|q| tr::preset(named).is_none() && is(Some(q.params.clone()))).map(|q| q.name.clone());
    let preset_label = match saved {
        Some(name) => name,
        None if is(tr::preset(named)) || is(tr::enhanced_preset(named)) => named.to_string(),
        None => "Custom".to_string(),
    };
    let roots = root_ids(s, p)?;
    // The image (inside a trace group, or selected directly).
    let (target, image) = {
        let d = &s.doc()?.doc;
        roots
            .iter()
            .find_map(|r| {
                let n = d.node(*r)?;
                match &n.kind {
                    NodeKind::Image(i) => Some((*r, i.clone())),
                    _ if is_trace_group(n) => match &n.children()?.first()?.kind {
                        NodeKind::Image(i) => Some((*r, i.clone())),
                        _ => None,
                    },
                    _ => None,
                }
            })
            .ok_or_else(|| bad(c, "select an image (or an Image Trace group)"))?
    };
    if by_color && s.doc()?.doc.node(target).is_some_and(vectorcraft_render::effects::has_pathfinder) {
        return Err(bad(c, LIVE_PATHFINDER_BY_COLOR));
    }
    let blob = s.doc()?.doc.images.get(&image.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", image.key)))?;
    let bytes = Arc::clone(&blob.bytes);
    let library = trace_library(c, s, &mut params)?;
    let traced_with = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    let inputs = TraceInputs { target, image, params: traced_with, swatches: params.swatches.clone() };
    Ok(Prepared { inputs, trace: params, view, params_json, preset_label, library, bytes })
}

/// What a trace made of an image: the size in pixels it was decoded at, and the paths.
pub struct Traced {
    inputs: TraceInputs,
    raster: (u32, u32),
    res: tr::TraceResult,
}

impl Traced {
    /// Hand it to the next run of its command with the same inputs ([`TraceRun::params`]), which
    /// applies it as one undo step instead of tracing again. Any other trace drops it.
    pub fn hand_in(self, s: &mut Session) {
        s.trace_hand_off = Some(self);
    }
}

/// Decode the image `bytes` and trace it with `params`, telling `steps` how far it has got
/// ([`tr::trace_watched`]); [`EngineError::Cancelled`] once `steps` answers `false`.
fn trace_now(inputs: &TraceInputs, params: &tr::TraceParams, bytes: &[u8], steps: &(dyn Fn(u64, u64) -> bool + Sync)) -> Result<Traced> {
    // An image over the tracer's size is traced scaled down (the paths are scaled back).
    let raster = tr::Raster::decode_to_fit(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    if !steps(0, 0) {
        return Err(EngineError::Cancelled);
    }
    let res = tr::trace_watched(&raster, params, MAX_TRACE_ANCHORS, steps).map_err(|e| match e {
        tr::TraceError::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    })?;
    Ok(Traced { inputs: inputs.clone(), raster: (raster.width, raster.height), res })
}

/// An Image Trace made away from its command, off the UI thread (#3098). [`TraceRun::prepare`]
/// takes what `imageTrace.make` or `imageTrace.makeAndExpand` traces from the session, checked as
/// the command checks it; [`TraceRun::run`] decodes and traces the image without the session, on
/// any thread; [`Traced::hand_in`] gives the result to the command.
pub struct TraceRun {
    command: &'static str,
    params: Value,
    prepared: Prepared,
}

impl TraceRun {
    /// The trace `command` (`imageTrace.make` or `imageTrace.makeAndExpand`) with params `p` makes
    /// in the active document.
    pub fn prepare(s: &Session, command: &str, p: &Value) -> Result<TraceRun> {
        let command = match command {
            "imageTrace.make" => "imageTrace.make",
            "imageTrace.makeAndExpand" => "imageTrace.makeAndExpand",
            other => return Err(EngineError::Other(format!("`{other}` doesn't trace an image"))),
        };
        let prepared = prepare(s, command, p)?;
        // The object traced is named, so the command applies the trace to it whatever is
        // selected by then.
        let mut params = if p.is_object() { p.clone() } else { json!({}) };
        if let Some(o) = params.as_object_mut() {
            o.remove("ids");
            o.insert("id".into(), json!(prepared.inputs.target.0));
        }
        Ok(TraceRun { command, params, prepared })
    }

    /// `imageTrace.make` or `imageTrace.makeAndExpand`.
    pub fn command(&self) -> &'static str {
        self.command
    }

    /// The command's params that apply this trace: those it was prepared with, the object traced
    /// in `id`.
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// The object it traces: an image, or an Image Trace object traced again.
    pub fn target(&self) -> NodeId {
        self.prepared.inputs.target
    }

    /// The size in pixels of the image it traces.
    pub fn image_size(&self) -> (u32, u32) {
        (self.prepared.inputs.image.width, self.prepared.inputs.image.height)
    }

    /// Does it still apply: would the command trace the same object, image and settings now?
    pub fn current(&self, s: &Session) -> bool {
        prepare(s, self.command, &self.params).is_ok_and(|now| now.inputs == self.prepared.inputs)
    }

    /// Decode and trace the image, telling `steps` how far it has got → what it traced, or why it
    /// failed or stopped ([`EngineError::Cancelled`] once `steps` answers `false`). It touches no
    /// document, so it can run on any thread.
    pub fn run(&self, steps: &(dyn Fn(u64, u64) -> bool + Sync)) -> Result<Traced> {
        trace_now(&self.prepared.inputs, &self.prepared.trace, &self.prepared.bytes, steps)
    }
}

pub(super) fn trace_make(s: &mut Session, p: &Value, expand: bool) -> Result<Value> {
    let c = if expand { "imageTrace.makeAndExpand" } else { "imageTrace.make" };
    let Prepared { inputs, trace, view, params_json, preset_label, library, bytes } = prepare(s, c, p)?;
    let group_by_color = expand && by_color_param(c, p)?;
    // A saved preset's settings join the journal entry, so a replay traces as this run did after
    // the preset is changed, renamed or deleted.
    let named = str_param(p, "preset").unwrap_or("Default");
    if let Some(q) = s.trace_preset(named).filter(|_| tr::preset(named).is_none()) {
        let recorded = serde_json::to_value(&q.params).map_err(|e| EngineError::Other(e.to_string()))?;
        s.note_journal("savedPreset", recorded);
    }
    // A trace handed in for these very inputs ([`TraceRun`]), else one made now.
    let Traced { raster, res, .. } = match s.trace_hand_off.take() {
        Some(traced) if traced.inputs == inputs => traced,
        _ => trace_now(&inputs, &trace, &bytes, &|_, _| true)?,
    };
    let TraceInputs { target, image: img, .. } = inputs;
    let warnings = trace_warnings((img.width, img.height), raster, res.worked_at);
    // Pixel space of the decoded raster → the image object's pixel space → document.
    let sx = img.width.max(1) as f64 / raster.0.max(1) as f64;
    let sy = img.height.max(1) as f64 / raster.1.max(1) as f64;
    let xf = img.xf * vectorcraft_geom::Affine::scale_non_uniform(sx, sy);
    // Document units per raster pixel (stroke widths).
    let unit = xf.determinant().abs().sqrt();
    let (npaths, anchors, colors) = (res.paths.len(), res.anchor_count(), res.palette.len());
    let label = if expand { "Image Trace (Make and Expand)" } else { "Image Trace" };
    // A library color fills with the swatch's displayed color (its model kept), unlinked.
    // Keep the first swatch that traces as a given color. Build the lookup once per trace.
    let mut by_color: HashMap<[u8; 3], &Paint> = HashMap::new();
    for (rgb, paint) in &library {
        by_color.entry(*rgb).or_insert(paint);
    }
    let id = s.edit(label, |d, sel| {
        let src = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        let mut image = if is_trace_group(&src) {
            src.children()
                .and_then(|c| c.first())
                .map(|c| (**c).clone())
                .ok_or_else(|| EngineError::Other("Image Trace: the traced image is missing".into()))?
        } else {
            src.clone()
        };
        image.visible = true;
        let mut children: Vec<Arc<Node>> = vec![];
        if !expand {
            let mut hidden = image.clone();
            hidden.visible = false;
            children.push(Arc::new(hidden));
        }
        let mut traced = Vec::with_capacity(res.paths.len());
        for tp in res.paths {
            let path = tp.path.transformed(xf);
            let mut n = shape_node(d, path, None);
            // Shapes: a path the tracer found to be a shape is that live shape.
            if let (Some(shape), NodeKind::Path { path, live, .. }) = (tp.shape, &mut n.kind) {
                let shape = traced_shape(shape, xf);
                *path = shape.to_path();
                *live = Some(Box::new(shape));
            }
            let solid =
                by_color.get(&tp.color).map_or_else(|| Paint::solid(Color::rgb8(tp.color[0], tp.color[1], tp.color[2])), |paint| (*paint).clone());
            // Gradients: a path the tracer found to be a linear gradient is filled with it.
            let paint = tp.gradient.as_ref().map_or(solid, |g| traced_gradient(g, xf));
            n.appearance = match tp.stroke {
                // Traced lines: round corners, and ends round or square as the tracer found them.
                Some(width) => Appearance {
                    items: vec![AppearanceItem::Stroke(StrokeLayer {
                        cap: if tp.square_caps { LineCap::Square } else { LineCap::Round },
                        join: LineJoin::Round,
                        ..StrokeLayer::new(paint, width * unit)
                    })],
                    ..Default::default()
                }
                .into(),
                None => fill_only(paint).into(),
            };
            traced.push(n);
        }
        children.extend(grouped(d, traced, &res.parents));
        if group_by_color {
            children = color_groups(c, d, children)?;
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = if expand { None } else { Some(TRACE_NAME.into()) };
        if !expand {
            // Tracing again keeps the object's view unless another is asked for.
            let view = view.unwrap_or_else(|| src.trace_view());
            g.extras_mut().trace = Some(Box::new(json!({ "preset": preset_label, "params": params_json, "view": view.id() })));
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "paths": npaths, "anchors": anchors, "colors": colors, "warnings": warnings }))
}

/// Images larger than this on a side are traced, but more slowly: the trace says so.
pub const LARGE_TRACE_SIDE: u32 = 1500;

/// What a trace of an image of `size` pixels, traced at `traced` pixels, tells the user: that
/// Flat Logo averaged it down to `worked` pixels first, or else that a large image takes longer
/// (the public documentation's warning for images over 1500 px) and that one over the tracer's
/// [`tr::MAX_PIXELS`] was traced scaled down.
pub(crate) fn trace_warnings(size: (u32, u32), traced: (u32, u32), worked: Option<(u32, u32)>) -> Vec<String> {
    let mut out = vec![];
    if worked.is_none() && size.0.max(size.1) > LARGE_TRACE_SIDE {
        out.push(format!(
            "The image is {} × {} pixels, more than {LARGE_TRACE_SIDE} on a side: it is traced, but more slowly. A lower-fidelity preset or a smaller copy of the image traces faster.",
            size.0, size.1
        ));
    }
    if let Some(worked) = worked {
        out.push(format!("Flat Logo traced the image at {} × {} pixels, averaged down from {} × {}.", worked.0, worked.1, size.0, size.1));
    } else if u64::from(traced.0) * u64::from(traced.1) < u64::from(size.0) * u64::from(size.1) {
        out.push(format!(
            "The image was traced at {} × {} pixels, scaled down from {} × {} ({} megapixels at most).",
            traced.0,
            traced.1,
            size.0,
            size.1,
            tr::MAX_PIXELS >> 20
        ));
    }
    out
}

fn trace_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let v: Vec<NodeId> = root_ids(s, p)?.into_iter().filter(|i| d.node(*i).is_some_and(is_trace_group)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select an Image Trace object".into()));
    }
    Ok(v)
}

pub(super) fn trace_set_view(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.setView";
    let view = view_param(C, p)?.ok_or_else(|| bad(C, "view is required"))?;
    let targets = trace_targets(s, p)?;
    let d = &s.doc()?.doc;
    if targets.iter().all(|g| d.node(*g).is_some_and(|n| n.trace_view() == view)) {
        return Ok(json!({ "ids": ids_json(&targets), "view": view.id() }));
    }
    s.edit("Image Trace View", |d, _| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            if let Some(t) = n.extras_if_any().and_then(|e| e.trace.as_deref_mut()).and_then(Value::as_object_mut) {
                t.insert("view".into(), json!(view.id()));
            } else {
                n.extras_mut().trace = Some(Box::new(json!({ "preset": "Custom", "view": view.id() })));
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets), "view": view.id() }))
}

pub(super) fn trace_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    let ids = s.edit("Release Image Trace", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &targets);
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut img = (**n.children().and_then(|c| c.first()).ok_or(EngineError::NoNode(*g))?).clone();
            img.visible = true;
            out.push(img.id);
            released.put(d, *g, vec![img])?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn trace_expand(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.expand";
    let by_color = by_color_param(C, p)?;
    let targets = trace_targets(s, p)?;
    if by_color {
        // A trace made with Auto Grouping is refused even when none of its paths were grouped.
        let d = &s.doc()?.doc;
        let grouped = |n: &Node| n.extras().trace.as_deref().and_then(|t| t.pointer("/params/autoGrouping")).and_then(Value::as_bool) == Some(true);
        if targets.iter().any(|g| d.node(*g).is_some_and(grouped)) {
            return Err(bad(C, "byColor can't split Auto Grouping groups: trace without Auto Grouping"));
        }
        if targets.iter().any(|g| d.node(*g).is_some_and(vectorcraft_render::effects::has_pathfinder)) {
            return Err(bad(C, LIVE_PATHFINDER_BY_COLOR));
        }
    }
    s.edit("Expand Image Trace", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.extras_mut().trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| !matches!(c.kind, NodeKind::Image(_)));
            }
            if by_color {
                let items = d.node(*g).and_then(Node::children).cloned().unwrap_or_default();
                // The paths may have been moved or reshaped since the trace was made, so any trace's
                // paths are taken to overlap.
                let groups = color_groups(C, d, items)?;
                if let Some(ch) = d.node_mut(*g).and_then(Node::children_mut) {
                    *ch = groups;
                }
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

#[cfg(test)]
mod color_group_tests {
    use super::*;
    use vectorcraft_geom::{PathData, SubPath};

    fn area(d: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, color: Color) -> Arc<Node> {
        let path = PathData::single(SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true));
        Arc::new(Node::path(d.alloc_id(), path, fill_only(Paint::solid(color))))
    }

    #[test]
    fn abutting_color_groups_preserve_order_at_shared_edges() {
        let mut d = Document::new(100.0, 100.0);
        let a = Color::rgb(1.0, 0.0, 0.0);
        let b = Color::rgb(0.0, 0.0, 1.0);
        let items = vec![area(&mut d, 0.0, 0.0, 20.0, 20.0, a), area(&mut d, 20.0, 0.0, 40.0, 20.0, b), area(&mut d, 40.0, 0.0, 60.0, 20.0, a)];
        let original: Vec<_> = items.iter().map(|n| n.id).collect();
        let groups = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
        let actual: Vec<_> = groups.iter().flat_map(|g| g.children().unwrap().iter().map(|n| n.id)).collect();
        assert_eq!(actual, original, "shared-edge paths changed stacking order");
    }

    #[test]
    fn dense_color_groups_use_bounded_box_tests() {
        let mut d = Document::new(200.0, 200.0);
        let a = Color::rgb(1.0, 0.0, 0.0);
        let b = Color::rgb(0.0, 0.0, 1.0);
        let items = (0..4000)
            .map(|i| {
                if i % 2 == 0 {
                    let x = 100.0 + (i % 40) as f64 * 0.01;
                    area(&mut d, x, 100.0, x + 0.1, 100.1, a)
                } else {
                    let x = if i % 4 == 1 { 0.0 } else { 198.0 };
                    area(&mut d, x, x, x + 2.0, x + 2.0, b)
                }
            })
            .collect();
        BOX_TESTS.with(|n| n.set(0));
        let groups = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
        let tests = BOX_TESTS.with(std::cell::Cell::get);
        assert_eq!(groups.iter().map(|g| g.children().map_or(0, Vec::len)).sum::<usize>(), 4000);
        assert!(tests < 4000 * 32, "{tests} box tests for 4000 paths");
    }

    #[test]
    fn overlapping_alternating_colors_visit_bounded_candidates() {
        const COUNT: usize = 3000;
        let mut d = Document::new(100.0, 100.0);
        let colors = [Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)];
        let items = (0..COUNT).map(|i| area(&mut d, 10.0, 10.0, 20.0, 20.0, colors[i % 2])).collect();
        CANDIDATE_VISITS.with(|n| n.set(0));
        let groups = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
        let visits = CANDIDATE_VISITS.with(std::cell::Cell::get);
        assert_eq!(groups.len(), COUNT);
        assert!(visits < COUNT * 16, "{visits} candidate visits for {COUNT} paths");
    }

    #[test]
    fn separate_stripes_of_alternating_colors_visit_bounded_candidates() {
        const COUNT: usize = 2000;
        let mut d = Document::new(1000.0, 4000.0);
        let colors = [Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)];
        for tall in [false, true] {
            let items = (0..COUNT)
                .map(|i| {
                    let at = i as f64 * 2.0;
                    if tall { area(&mut d, at, 0.0, at + 0.5, 1000.0, colors[i % 2]) } else { area(&mut d, 0.0, at, 1000.0, at + 0.5, colors[i % 2]) }
                })
                .collect();
            CANDIDATE_VISITS.with(|n| n.set(0));
            let groups = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
            let visits = CANDIDATE_VISITS.with(std::cell::Cell::get);
            assert_eq!(groups.len(), 2, "tall {tall}");
            assert!(visits < COUNT * 32, "tall {tall}: {visits} candidate visits for {COUNT} stripes");
        }
    }

    #[test]
    fn spatial_color_groups_match_simple_scan() {
        let colors = [Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0), Color::rgb(0.0, 1.0, 0.0)];
        for layout in 0..6 {
            let mut d = Document::new(200.0, 200.0);
            let items: Vec<_> = (0..90)
                .map(|i| {
                    let x = match layout {
                        0 => i as f64 * 2.0,
                        1 => (i % 10) as f64 * 5.0,
                        2 => (i % 7) as f64 * 3.0,
                        3 => (i % 13) as f64 * 9.0,
                        4 => (i % 4) as f64 * 10.0,
                        _ => i as f64 * 1.5,
                    };
                    let y = match layout {
                        0 => 0.0,
                        1 => (i / 10) as f64 * 5.0,
                        2 => (i / 7) as f64 * 3.0,
                        3 => (i / 13) as f64 * 9.0,
                        4 => i as f64 * 1.5,
                        _ => (i % 4) as f64 * 10.0,
                    };
                    // Layouts 4 and 5: wide and tall stripes, some meeting their neighbors.
                    let (w, h) = match layout {
                        4 => (150.0, 1.0),
                        5 => (1.0, 150.0),
                        _ => (3.0, 3.0),
                    };
                    area(&mut d, x, y, x + w, y + h, colors[i % 3])
                })
                .collect();
            let mut expected: Vec<(String, Vec<(NodeId, Rect)>)> = vec![];
            for item in &items {
                let key = item.appearance.fill_paint().color().unwrap().to_hex();
                let bounds = vectorcraft_render::painted_bounds(&d, item).unwrap();
                let previous = expected.iter().rposition(|(k, _)| *k == key);
                let can_join = previous.filter(|&at| {
                    !expected.iter().skip(at + 1).any(|(_, boxes)| {
                        boxes.iter().any(|(_, m)| {
                            let b = bounds.inflate(1.0, 1.0);
                            m.x0 < b.x1 && b.x0 < m.x1 && m.y0 < b.y1 && b.y0 < m.y1
                        })
                    })
                });
                match can_join.and_then(|at| expected.get_mut(at)) {
                    Some((_, members)) => members.push((item.id, bounds)),
                    None => expected.push((key, vec![(item.id, bounds)])),
                }
            }
            let actual = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
            let actual: Vec<Vec<_>> = actual.iter().map(|g| g.children().unwrap().iter().map(|n| n.id).collect()).collect();
            let expected: Vec<Vec<_>> = expected.into_iter().map(|(_, members)| members.into_iter().map(|(id, _)| id).collect()).collect();
            assert_eq!(actual, expected, "layout {layout}");
        }
    }

    #[test]
    fn an_abutting_gradient_stays_above_the_solid_area_it_borders() {
        use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
        let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
        let ramp = || Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops.clone()))));
        let mut d = Document::new(100.0, 100.0);
        let mut area = |x0: f64, x1: f64, paint: Paint| {
            let path =
                PathData::single(SubPath::polyline(&[Point::new(x0, 0.0), Point::new(x1, 0.0), Point::new(x1, 60.0), Point::new(x0, 60.0)], true));
            Arc::new(Node::path(d.alloc_id(), path, fill_only(paint)))
        };
        // A gradient, a solid area beside it and a second gradient beside that, bottom to top.
        let items = vec![area(0.0, 50.0, ramp()), area(50.0, 80.0, Paint::solid(Color::rgb(0.2, 0.6, 0.2))), area(80.0, 100.0, ramp())];
        let groups = color_groups("imageTrace.makeAndExpand", &mut d, items).unwrap();
        let names: Vec<_> = groups.iter().map(|g| g.name.clone().unwrap_or_default()).collect();
        assert_eq!(names, ["Gradients", "#339933", "Gradients"]);
    }
}

#[cfg(test)]
mod trace_limit_tests {
    use super::{MAX_TRACE_ANCHORS, trace_warnings};

    #[test]
    fn trace_warnings_name_a_large_or_scaled_down_image() {
        assert!(trace_warnings((1500, 1500), (1500, 1500), None).is_empty());
        assert_eq!(trace_warnings((1501, 10), (1501, 10), None).len(), 1);
        let w = trace_warnings((16000, 12000), (9459, 7094), None);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("16000 × 12000") && w[1].contains("9459 × 7094") && w[1].contains("64 megapixels"), "{w:?}");
        // Flat Logo's own working size is named instead of the megapixel limit.
        let w = trace_warnings((1025, 1025), (1025, 1025), Some((513, 513)));
        assert_eq!(w, ["Flat Logo traced the image at 513 × 513 pixels, averaged down from 1025 × 1025."]);
        // Flat Logo traces a large image at its reduced size, so it gives no slow-trace warning.
        let w = trace_warnings((1600, 1600), (1600, 1600), Some((800, 800)));
        assert_eq!(w, ["Flat Logo traced the image at 800 × 800 pixels, averaged down from 1600 × 1600."]);
        let w = trace_warnings((16000, 12000), (9459, 7094), Some((946, 710)));
        assert_eq!(w, ["Flat Logo traced the image at 946 × 710 pixels, averaged down from 16000 × 12000."]);
    }

    // A 4000 × 3000 photo at High Fidelity Photo makes about 1.9 million anchors, which must fit;
    // the limit still guards memory (about 90 bytes an anchor). Checked when the tests build.
    const _: () = assert!(MAX_TRACE_ANCHORS >= 2_000_000 && MAX_TRACE_ANCHORS <= 16_000_000);
}
