use vectorcraft_color::Swatch;
use vectorcraft_doc::ImageObject;

use super::{
    Appearance, AppearanceItem, Arc, Beside, Color, Document, EngineError, LineCap, LineJoin, LiveShape, Node, NodeId, NodeKind, Paint, Point,
    Result, Session, StrokeLayer, TRACE_NAME, TraceView, Value, bad, fill_only, ids_json, json, root_ids, shape_node, str_param, switch_or, tr,
};

// ---------- Image Trace ----------

fn is_trace_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(TRACE_NAME)
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

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

/// Document swatch `w`'s solid color with the paint a trace fills with it: a global or spot color,
/// or a tint of one, linked to its swatch as `swatch_solid` (paint.rs) links it; any other color
/// unlinked. The color is the one the paint shows (a Lab spot color's CMYK equivalent when the Spot
/// Colors options use CMYK values).
fn trace_swatch_paint(d: &Document, w: &Swatch) -> Option<(Color, Paint)> {
    let linked = d.swatch_link(&w.name).and_then(|(base, tint)| d.tint_paint(&base, tint));
    let color = linked.as_ref().and_then(Paint::color).or_else(|| w.paint.color())?;
    Some((color, linked.unwrap_or_else(|| Paint::solid(color))))
}

/// Document Library with `colorGroup`: the solid colors of the active document's color group
/// `name` (so named exactly, else ignoring case), each with its paint ([`trace_swatch_paint`]).
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
/// `params.swatches` → each one as traced (sRGB) with the paint to fill with. The document's
/// global and spot colors fill linked to their swatches ([`trace_swatch_paint`]); a swatch
/// library's colors fill unlinked. Errors name command `c`.
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
            // A library color fills with the swatch's own color (its model kept), linked to a
            // global or spot swatch of the document.
            let solid = library
                .iter()
                .find(|(rgb, _)| *rgb == tp.color)
                .map_or_else(|| Paint::solid(Color::rgb8(tp.color[0], tp.color[1], tp.color[2])), |l| l.1.clone());
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
    let targets = trace_targets(s, p)?;
    s.edit("Expand Image Trace", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.extras_mut().trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| !matches!(c.kind, NodeKind::Image(_)));
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
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
