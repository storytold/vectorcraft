//! Tracing in the script object model: `PlacedItem.trace()` and `RasterItem.trace()`, the
//! `PluginItem` an Image Trace object is, its `TracingObject` and its `TracingOptions`, as the help
//! pages <https://ai-scripting.docsforadobe.dev/jsobjref/PluginItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TracingObject/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TracingOptions/> describe them.
//!
//! An Image Trace object is a group that holds its source image, hidden, as its first object.
//! Tracing runs `imageTrace.make`, expanding `imageTrace.expand`, releasing `imageTrace.release`
//! and a change of view `imageTrace.setView`, so each joins the run's transaction and its work
//! counts against the run's limits.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `trace()` returns once the trace is made, so the counts can be read at once. It traces with
//!   the Default preset.
//! - The traced image stays the same object: `sourceArt` gives it, and `releaseTracing()` gives it
//!   back. `trace()` on an image that is the source of a tracing throws [`ALREADY_TRACED`], and on
//!   a linked image whose file is missing throws `Linked image is missing`.
//! - `pathCount` counts the traced paths, without the seam fills an Abutting trace adds,
//!   `anchorCount` their anchor points and `usedColorCount` the different paints they are filled
//!   and stroked with, as the tracing is when they are read. `areaCount` throws. `imageResolution`
//!   is the source image's pixels per inch as it is placed; an image placed at a different
//!   resolution across and down has none.
//! - Assigning an option, or `loadFromPreset(name)`, traces again unless the options are already
//!   so. Tracing again makes a new object: the `PluginItem`, its `TracingObject` and its
//!   `TracingOptions` follow it. A tracing that another command traced again, expanded or
//!   released, or whose source image was deleted, is stale.
//! - `palette` is `""` for the automatic palette, or `"Limited"`, `"Full Tone"` or
//!   `"Document Library"`. `minArea` is the Noise setting in pixels: an area smaller than it is not
//!   traced. `tracingMode` has no value for the Flat Logo mode. The options with no VectorCraft
//!   setting throw [`not_implemented`].
//! - `viewRaster` and `viewVector` take only the pairs of the Image Trace views
//!   ([`VIEWS`]); the Split view has neither.
//! - `storeToPreset(name)` gives `false` for a built-in preset and `Custom`; a script cannot change
//!   the saved presets, so any other name throws. A preset name has at most [`MAX_PRESET_NAME`]
//!   bytes and none of `/`, `\`, `:` or control characters.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Data, HostClass, HostError, HostMember, host_ref, host_ref_parts};

use super::{
    Answer, Host, Item, Link, STALE_ITEM, arg, boolean, class, command, command_value, edit_item, find_doc, float, get, id_of, item_of, item_ref,
    method, named, num, number, on_doc, property, text, typename_of, whole, with_doc,
};
use crate::doc::{Node, TraceView};
use crate::{NodeId, NodeKind};

/// The error of `trace()` on an image that is the source of a tracing.
pub const ALREADY_TRACED: &str = "The image is the source art of a tracing";
/// The error of options that trace with neither fills nor strokes.
pub const NEEDS_FILLS_OR_STROKES: &str = "TracingOptions requires fills or strokes";
/// The error of `expandTracing(true)`.
pub const VIEWED_EXPAND: &str = "TracingObject.expandTracing(true) is not implemented";
/// The longest preset name `loadFromPreset` and `storeToPreset` take, in bytes.
pub const MAX_PRESET_NAME: usize = 256;
/// The most times one run traces a tracing again through its options.
pub const MAX_RETRACES: usize = 10_000;
/// The largest `minArea`: the most pixels the tracer traces ([`vectorcraft_trace::MAX_PIXELS`]).
pub const MAX_MIN_AREA: u64 = vectorcraft_trace::MAX_PIXELS;

/// The enumerations the members here give values of, as the help page
/// <https://ai-scripting.docsforadobe.dev/jsobjref/scripting-constants/> lists them.
pub(super) const ENUMS: &[(&str, &[&str])] = &[
    ("TracingModeType", &["TRACINGMODEBLACKANDWHITE", "TRACINGMODECOLOR", "TRACINGMODEGRAY"]),
    (
        "ViewRasterType",
        &["TRACINGVIEWRASTERADJUSTEDIMAGE", "TRACINGVIEWRASTERNOIMAGE", "TRACINGVIEWRASTERORIGINALIMAGE", "TRACINGVIEWRASTERTRANSPARENTIMAGE"],
    ),
    (
        "ViewVectorType",
        &["TRACINGVIEWVECTORNOTRACINGRESULT", "TRACINGVIEWVECTOROUTLINES", "TRACINGVIEWVECTOROUTLINESWITHTRACING", "TRACINGVIEWVECTORTRACINGRESULT"],
    ),
];

/// The classes this module defines.
pub(super) const CLASSES: &[&str] = &["PluginItem"];

/// Each Image Trace view with the `viewRaster` and `viewVector` that give it.
const VIEWS: &[(TraceView, &str, &str)] = &[
    (TraceView::Result, "ViewRasterType.TRACINGVIEWRASTERNOIMAGE", "ViewVectorType.TRACINGVIEWVECTORTRACINGRESULT"),
    (TraceView::ResultWithOutlines, "ViewRasterType.TRACINGVIEWRASTERNOIMAGE", "ViewVectorType.TRACINGVIEWVECTOROUTLINESWITHTRACING"),
    (TraceView::Outlines, "ViewRasterType.TRACINGVIEWRASTERNOIMAGE", "ViewVectorType.TRACINGVIEWVECTOROUTLINES"),
    (TraceView::OutlinesWithSource, "ViewRasterType.TRACINGVIEWRASTERORIGINALIMAGE", "ViewVectorType.TRACINGVIEWVECTOROUTLINES"),
    (TraceView::Source, "ViewRasterType.TRACINGVIEWRASTERORIGINALIMAGE", "ViewVectorType.TRACINGVIEWVECTORNOTRACINGRESULT"),
];

/// Each `tracingMode` with its Image Trace mode.
const MODES: &[(&str, &str)] = &[
    ("TracingModeType.TRACINGMODEBLACKANDWHITE", "blackAndWhite"),
    ("TracingModeType.TRACINGMODEGRAY", "grayscale"),
    ("TracingModeType.TRACINGMODECOLOR", "color"),
];

/// Each `palette` name with its Image Trace palette.
const PALETTES: &[(&str, &str)] = &[("", "automatic"), ("Limited", "limited"), ("Full Tone", "fullTone"), ("Document Library", "documentLibrary")];

/// The options with no VectorCraft setting.
const NOT_IMPLEMENTED: &[&str] = &[
    "cornerAngle",
    "livePaintOutput",
    "maxStrokeWeight",
    "minStrokeLength",
    "outputToSwatches",
    "pathFitting",
    "preprocessBlur",
    "resample",
    "resampleResolution",
];

/// What a run keeps for tracings: for each document uid and Image Trace object a script traced
/// again through its options, the id of the object that replaced it.
#[derive(Default)]
pub(super) struct State {
    next: RefCell<HashMap<(u64, u64), u64>>,
}

/// `id` while the document `uid` holds it (a run never gives an id out twice, so an undo that put
/// it back put back the same object), else the first object the document holds among those that
/// replaced it, one after the other, when this run traced it again (an Undo can put back one of
/// them), else the last of them, else `id`.
pub(super) fn follow(h: &Host, uid: u64, id: NodeId) -> NodeId {
    let s = h.session.borrow();
    let doc = s.documents().iter().find(|d| d.uid == uid);
    let held = |id: NodeId| doc.is_some_and(|d| d.doc.node(id).is_some());
    let next = h.dom.tracing.next.borrow();
    let mut id = id;
    // Each object is replaced by one made later, so the steps end; the bound holds them anyway.
    for _ in 0..=next.len() {
        if held(id) {
            return id;
        }
        match next.get(&(uid, id.0)) {
            Some(n) => id = NodeId(*n),
            None => return id,
        }
    }
    id
}

/// Records that `new` replaced `old` in the document `uid`.
fn replaced(h: &Host, uid: u64, old: NodeId, new: NodeId) {
    h.dom.tracing.next.borrow_mut().insert((uid, old.0), new.0);
}

/// The error of a member with no VectorCraft setting.
pub fn not_implemented(class: &str, name: &str) -> String {
    format!("{class}.{name} is not implemented")
}

fn out_of_range(name: &str, range: &str) -> HostError {
    HostError::Range(format!("TracingOptions.{name} is out of range: {range}"))
}

fn invalid(name: &str) -> HostError {
    HostError::Error(format!("TracingOptions.{name} has an invalid value"))
}

fn stale() -> HostError {
    HostError::Error(STALE_ITEM.into())
}

fn field<'a>(d: &'a Data, k: &str) -> Option<&'a Data> {
    match d {
        Data::Object(o) => o.iter().find(|(n, _)| n == k).map(|(_, v)| v),
        _ => None,
    }
}

/// The first id of a command's result `key`: a number or a list of them, read exactly.
fn result_id(v: &Value, key: &str) -> Option<NodeId> {
    match &v[key] {
        Value::Array(a) => a.first().and_then(Value::as_u64).map(NodeId),
        n => n.as_u64().map(NodeId),
    }
}

/// The Image Trace object, its document and the id its handle holds, of the `cls` (`TracingObject`
/// or `TracingOptions`) `d` names. It must still be an Image Trace object of its document.
fn tracing_of(h: &Host, d: &Data, cls: &str) -> Result<(Item, [f64; 4]), HostError> {
    let Some((c, handle)) = host_ref_parts(d).filter(|(c, _)| *c == cls) else {
        return Err(HostError::Type(format!("this is not a {cls}")));
    };
    let &[run, uid, high, low] = handle.as_slice() else { return Err(HostError::Type(format!("this is not a {c}"))) };
    let doc = find_doc(h, run, uid).ok_or_else(stale)?;
    let id = follow(h, doc.uid, id_of(high, low).ok_or_else(stale)?);
    let live = with_doc(h, doc, |_, st| st.doc.node(id).is_some_and(crate::cmd::is_trace_group));
    if !live {
        return Err(stale());
    }
    Ok((Item { doc, id }, [run, uid, high, low]))
}

/// The `PluginItem` `this` names, with the handle numbers its `tracing` keeps.
fn plugin_of(h: &Host, a: &[Data]) -> Result<(Item, [f64; 4]), HostError> {
    match host_ref_parts(arg(a, 0)) {
        Some(("PluginItem", handle)) => {
            let item = item_of(h, arg(a, 0))?;
            match handle.as_slice() {
                &[run, uid, high, low] => Ok((item, [run, uid, high, low])),
                _ => Err(HostError::Type("this is not a PluginItem".into())),
            }
        }
        _ => Err(HostError::Type("this is not a PluginItem".into())),
    }
}

/// Reads the Image Trace object of `item` with `f`.
fn read_group<T>(h: &Host, item: Item, f: impl FnOnce(&crate::DocState, &Node) -> Result<T, HostError>) -> Result<T, HostError> {
    with_doc(h, item.doc, |_, st| match st.doc.node(item.id) {
        Some(n) => f(st, n),
        None => Err(stale()),
    })
}

/// Runs command `id` with `params` on `item`'s document → the object its result names under `key`.
/// The selection stays as it was, with that object in place of `item`.
fn replace(h: &Host, item: Item, id: &str, params: &Value, key: &str) -> Result<NodeId, HostError> {
    let before = with_doc(h, item.doc, |_, st| st.selection.objects.clone());
    on_doc(h, item.doc.uid, false, |h| {
        let (_, r) = command_value(h, id, params)?;
        let new = result_id(&r, key).ok_or_else(|| HostError::Error(format!("{id} gave no object")))?;
        let (want, now) = {
            let s = h.session.borrow();
            let st = s.documents().iter().find(|d| d.uid == item.doc.uid).ok_or_else(|| HostError::Error(super::STALE_DOCUMENT.into()))?;
            let mut want: Vec<NodeId> = Vec::new();
            for i in before.iter().map(|i| if *i == item.id { new } else { *i }) {
                if st.doc.node(i).is_some() && !want.contains(&i) {
                    want.push(i);
                }
            }
            (want, st.selection.objects.clone())
        };
        if now != want {
            if want.is_empty() {
                command(h, "select.none", &json!({}))?;
            } else {
                command(h, "select.set", &json!({"ids": want.iter().map(|i| i.0).collect::<Vec<_>>()}))?;
            }
        }
        Ok(new)
    })
}

/// Traces the Image Trace object of `item` again with `params` (`id` is added) and makes its
/// script objects follow the new one.
fn retrace(h: &Host, item: Item, mut params: Map<String, Value>) -> Result<(), HostError> {
    if h.dom.tracing.next.borrow().len() >= MAX_RETRACES {
        return Err(HostError::Range(format!("a run traces a tracing again at most {MAX_RETRACES} times")));
    }
    params.insert("id".into(), json!(item.id.0));
    let new = replace(h, item, "imageTrace.make", &Value::Object(params), "id")?;
    replaced(h, item.doc.uid, item.id, new);
    Ok(())
}

// ---- PlacedItem and RasterItem ----

/// `trace()`: `imageTrace.make` of the image with the Default preset → its `PluginItem`.
fn image_trace(h: &Host, a: &[Data], cls: &str) -> Answer {
    let item = match host_ref_parts(arg(a, 0)) {
        Some((c, _)) if c == cls => item_of(h, arg(a, 0))?,
        _ => return Err(HostError::Type(format!("this is not a {cls}"))),
    };
    let (source, linked) = with_doc(h, item.doc, |_, st| {
        let d = &st.doc;
        let source = crate::cmd::is_trace_source(d, item.id);
        let linked = matches!(d.node(item.id).map(|n| &n.kind), Some(NodeKind::Image(im)) if im.link.is_some());
        (source, linked)
    });
    if source {
        return Err(HostError::Error(ALREADY_TRACED.into()));
    }
    if linked {
        let info = on_doc(h, item.doc.uid, false, |h| command(h, "links.info", &json!({"id": item.id.0})))?;
        if matches!(field(&info, "status"), Some(Data::String(s)) if s == "missing") {
            return Err(HostError::Error(super::images::MISSING_LINK.into()));
        }
    }
    let id = replace(h, item, "imageTrace.make", &json!({"id": item.id.0}), "id")?;
    Ok(with_doc(h, item.doc, |_, st| item_ref(h, st, id)))
}

fn placed_trace(h: &Host, a: &[Data]) -> Answer {
    image_trace(h, a, "PlacedItem")
}

fn raster_trace(h: &Host, a: &[Data]) -> Answer {
    image_trace(h, a, "RasterItem")
}

// ---- PluginItem ----

fn plugin_is_tracing(h: &Host, a: &[Data]) -> Answer {
    plugin_of(h, a)?;
    Ok(Data::Bool(true))
}

fn plugin_tracing(h: &Host, a: &[Data]) -> Answer {
    let (_, handle) = plugin_of(h, a)?;
    Ok(host_ref("TracingObject", &handle))
}

// ---- TracingObject ----

/// What a tracing's counts are made of.
#[derive(Default)]
struct Counts {
    paths: u64,
    anchors: u64,
    colors: u64,
    /// The objects looked at.
    visited: u64,
}

fn anchors_of(n: &Node) -> u64 {
    match &n.kind {
        NodeKind::Path { path, .. } => path.anchor_count() as u64,
        _ => n.children().map_or(0, |c| c.iter().map(|m| anchors_of(m)).sum()),
    }
}

/// The traced paths of the Image Trace object `g`, their anchor points and paints: every path
/// and compound path in it but its source image, the seam fills and clipping paths.
fn counts(g: &Node) -> Counts {
    let mut c = Counts::default();
    let mut paints = HashSet::new();
    let mut todo: Vec<&Arc<Node>> = g.children().map(|ch| ch.iter().skip(1).collect()).unwrap_or_default();
    while let Some(n) = todo.pop() {
        c.visited += 1;
        if crate::cmd::is_seam_fills(n) {
            continue;
        }
        match &n.kind {
            NodeKind::Path { clipping: true, .. } => {}
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                c.paths += 1;
                c.anchors += anchors_of(n);
                for paint in [n.appearance.fill_paint(), n.appearance.stroke_paint()] {
                    if paint != vectorcraft_color::Paint::None
                        && let Ok(key) = serde_json::to_string(&paint)
                    {
                        paints.insert(key);
                    }
                }
            }
            _ => todo.extend(n.children().into_iter().flatten()),
        }
    }
    c.colors = paints.len() as u64;
    c
}

/// Reads the counts of the tracing `this` names with `f`.
fn count(h: &Host, a: &[Data], f: fn(&Counts) -> u64) -> Answer {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    let c = read_group(h, item, |_, g| Ok(counts(g)))?;
    h.charge_call(c.visited)?;
    Ok(num(float(f(&c))))
}

fn tracing_paths(h: &Host, a: &[Data]) -> Answer {
    count(h, a, |c| c.paths)
}

fn tracing_anchors(h: &Host, a: &[Data]) -> Answer {
    count(h, a, |c| c.anchors)
}

fn tracing_colors(h: &Host, a: &[Data]) -> Answer {
    count(h, a, |c| c.colors)
}

fn tracing_areas(h: &Host, a: &[Data]) -> Answer {
    tracing_of(h, arg(a, 0), "TracingObject")?;
    Err(HostError::Error(not_implemented("TracingObject", "areaCount")))
}

/// `imageResolution`: the source image's pixels per inch, 72 over the size of a pixel in points.
fn tracing_resolution(h: &Host, a: &[Data]) -> Answer {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    read_group(h, item, |_, g| {
        let source = g.children().and_then(|c| c.first()).ok_or_else(stale)?;
        let NodeKind::Image(im) = &source.kind else { return Err(stale()) };
        let none = || HostError::Error("the source image has no single resolution".into());
        if source.is_distorted_image() {
            return Err(none());
        }
        let [xa, xb, ya, yb, ..] = im.xf.as_coeffs();
        let (across, down) = (xa.hypot(xb), ya.hypot(yb));
        if across <= 1e-12 || down <= 1e-12 || (across - down).abs() > 1e-9 * across.max(down) {
            return Err(none());
        }
        Ok(num(72.0 / across))
    })
}

fn tracing_parent(h: &Host, a: &[Data]) -> Answer {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    Ok(with_doc(h, item.doc, |_, st| item_ref(h, st, item.id)))
}

fn tracing_source(h: &Host, a: &[Data]) -> Answer {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    read_group(h, item, |st, g| g.children().and_then(|c| c.first()).map(|s| item_ref(h, st, s.id)).ok_or_else(stale))
}

fn tracing_options(h: &Host, a: &[Data]) -> Answer {
    let (_, handle) = tracing_of(h, arg(a, 0), "TracingObject")?;
    Ok(host_ref("TracingOptions", &handle))
}

/// `expandTracing(viewed?)`: `imageTrace.expand` → the `GroupItem` that takes the tracing's place.
fn tracing_expand(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::Undefined | Data::Bool(false) => {}
        Data::Bool(true) => {
            tracing_of(h, arg(a, 0), "TracingObject")?;
            return Err(HostError::Error(VIEWED_EXPAND.into()));
        }
        _ => return Err(HostError::Type("viewed must be true or false".into())),
    }
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    let id = replace(h, item, "imageTrace.expand", &json!({"ids": [item.id.0]}), "ids")?;
    Ok(with_doc(h, item.doc, |_, st| item_ref(h, st, id)))
}

/// `releaseTracing()`: `imageTrace.release` → the source image, back in the tracing's place.
fn tracing_release(h: &Host, a: &[Data]) -> Answer {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingObject")?;
    let id = replace(h, item, "imageTrace.release", &json!({"ids": [item.id.0]}), "ids")?;
    Ok(with_doc(h, item.doc, |_, st| item_ref(h, st, id)))
}

// ---- TracingOptions ----

/// The tracing `this` names, the settings it was traced with, its view and its preset's name.
fn options_of(h: &Host, a: &[Data]) -> Result<(Item, Map<String, Value>, TraceView, String), HostError> {
    let (item, _) = tracing_of(h, arg(a, 0), "TracingOptions")?;
    read_group(h, item, |_, g| {
        let trace = g.extras().trace.as_deref();
        let params = trace.and_then(|t| t.get("params")).and_then(Value::as_object).cloned();
        let params = params.ok_or_else(|| HostError::Error("the tracing has no recorded options".into()))?;
        let preset = trace.and_then(|t| t.get("preset")).and_then(Value::as_str).unwrap_or_default().to_string();
        Ok((item, params, g.trace_view(), preset))
    })
}

/// Reads the setting `key` of the tracing `this` names with `f`.
fn read_option(h: &Host, a: &[Data], key: &str, f: impl FnOnce(&Value) -> Answer) -> Answer {
    let (_, params, ..) = options_of(h, a)?;
    f(params.get(key).unwrap_or(&Value::Null))
}

/// Traces again with the setting `key` at `value`, unless it is at that value already.
fn write_option(h: &Host, a: &[Data], key: &str, value: Value) -> Answer {
    let (item, mut params, ..) = options_of(h, a)?;
    if params.get(key) == Some(&value) {
        return Ok(Data::Undefined);
    }
    params.insert(key.into(), value);
    let on = |k: &str| params.get(k).and_then(Value::as_bool) == Some(true);
    if !on("fills") && !on("strokes") {
        return Err(HostError::Error(NEEDS_FILLS_OR_STROKES.into()));
    }
    let mut p = Map::new();
    p.insert("params".into(), Value::Object(params));
    retrace(h, item, p)?;
    Ok(Data::Undefined)
}

/// A whole number from `lo` to `hi` for the option `name`.
fn whole_in(d: &Data, name: &str, lo: u64, hi: u64) -> Result<u64, HostError> {
    let n = number(d, &format!("TracingOptions.{name}"))?;
    if n.fract() != 0.0 || n < float(lo) || n > float(hi) {
        return Err(out_of_range(name, &format!("a whole number from {lo} to {hi}")));
    }
    whole(n).ok_or_else(|| out_of_range(name, &format!("a whole number from {lo} to {hi}")))
}

fn as_number(v: &Value) -> Answer {
    v.as_f64().map(num).ok_or_else(|| HostError::Error("the tracing's options are damaged".into()))
}

fn as_bool(v: &Value) -> Answer {
    v.as_bool().map(Data::Bool).ok_or_else(|| HostError::Error("the tracing's options are damaged".into()))
}

fn options_mode(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "mode", |v| {
        let mode = v.as_str().unwrap_or_default();
        match MODES.iter().find(|(_, m)| *m == mode) {
            Some((name, _)) => Ok(text(*name)),
            None => Err(HostError::Error("TracingOptions.tracingMode has no value for this tracing's mode".into())),
        }
    })
}

fn options_set_mode(h: &Host, a: &[Data]) -> Answer {
    let mode = match arg(a, 1) {
        Data::String(s) => MODES.iter().find(|(n, _)| n == s).map(|(_, m)| *m),
        _ => None,
    };
    let mode = mode.ok_or_else(|| HostError::Type("TracingOptions.tracingMode must be a TracingModeType value".into()))?;
    write_option(h, a, "mode", json!(mode))
}

fn options_threshold(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "threshold", as_number)
}

fn options_set_threshold(h: &Host, a: &[Data]) -> Answer {
    let v = whole_in(arg(a, 1), "threshold", 0, 255)?;
    write_option(h, a, "threshold", json!(v))
}

fn options_max_colors(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "colors", as_number)
}

fn options_set_max_colors(h: &Host, a: &[Data]) -> Answer {
    let v = whole_in(arg(a, 1), "maxColors", 2, 256)?;
    write_option(h, a, "colors", json!(v))
}

fn options_min_area(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "noise", as_number)
}

fn options_set_min_area(h: &Host, a: &[Data]) -> Answer {
    let v = whole_in(arg(a, 1), "minArea", 0, MAX_MIN_AREA)?;
    write_option(h, a, "noise", json!(v))
}

fn options_fills(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "fills", as_bool)
}

fn options_set_fills(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "TracingOptions.fills")?;
    write_option(h, a, "fills", json!(v))
}

fn options_strokes(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "strokes", as_bool)
}

fn options_set_strokes(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "TracingOptions.strokes")?;
    write_option(h, a, "strokes", json!(v))
}

fn options_ignore_white(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "ignoreWhite", as_bool)
}

fn options_set_ignore_white(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "TracingOptions.ignoreWhite")?;
    write_option(h, a, "ignoreWhite", json!(v))
}

fn options_palette(h: &Host, a: &[Data]) -> Answer {
    read_option(h, a, "palette", |v| {
        let palette = v.as_str().unwrap_or_default();
        match PALETTES.iter().find(|(_, p)| *p == palette) {
            Some((name, _)) => Ok(text(*name)),
            None => Err(HostError::Error("the tracing's options are damaged".into())),
        }
    })
}

fn options_set_palette(h: &Host, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("TracingOptions.palette must be a string".into())) };
    let palette = PALETTES.iter().find(|(n, _)| n == name).map(|(_, p)| *p).ok_or_else(|| invalid("palette"))?;
    write_option(h, a, "palette", json!(palette))
}

/// The view's `viewRaster` (`raster`) or `viewVector`.
fn view_part(view: TraceView, raster: bool) -> Answer {
    let name = if raster { "viewRaster" } else { "viewVector" };
    VIEWS
        .iter()
        .find(|(v, ..)| *v == view)
        .map(|(_, r, v)| text(if raster { *r } else { *v }))
        .ok_or_else(|| HostError::Error(format!("TracingOptions.{name} has no value for the Split view")))
}

fn options_view(h: &Host, a: &[Data], raster: bool) -> Answer {
    let (_, _, view, _) = options_of(h, a)?;
    view_part(view, raster)
}

/// Shows the view `viewRaster` (`raster`) or `viewVector` makes with the other one as it is.
fn options_set_view(h: &Host, a: &[Data], raster: bool) -> Answer {
    let (name, kind) = if raster { ("viewRaster", "ViewRasterType") } else { ("viewVector", "ViewVectorType") };
    let given = match arg(a, 1) {
        Data::String(s) if ENUMS.iter().any(|(e, members)| *e == kind && members.iter().any(|m| *s == format!("{kind}.{m}"))) => s.clone(),
        _ => return Err(HostError::Type(format!("TracingOptions.{name} must be a {kind} value"))),
    };
    let (item, _, view, _) = options_of(h, a)?;
    let Data::String(other) = view_part(view, !raster)? else { return Err(invalid(name)) };
    let pair = |r: &str, v: &str| if raster { given == r && other == v } else { other == r && given == v };
    let (to, ..) = VIEWS.iter().find(|(_, r, v)| pair(r, v)).ok_or_else(|| invalid(name))?;
    if *to != view {
        edit_item(h, item, "imageTrace.setView", &json!({"ids": [item.id.0], "view": to.id()}))?;
    }
    Ok(Data::Undefined)
}

fn options_view_raster(h: &Host, a: &[Data]) -> Answer {
    options_view(h, a, true)
}

fn options_set_view_raster(h: &Host, a: &[Data]) -> Answer {
    options_set_view(h, a, true)
}

fn options_view_vector(h: &Host, a: &[Data]) -> Answer {
    options_view(h, a, false)
}

fn options_set_view_vector(h: &Host, a: &[Data]) -> Answer {
    options_set_view(h, a, false)
}

fn options_preset(h: &Host, a: &[Data]) -> Answer {
    let (_, _, _, preset) = options_of(h, a)?;
    Ok(text(preset))
}

fn options_parent(h: &Host, a: &[Data]) -> Answer {
    let (_, handle) = tracing_of(h, arg(a, 0), "TracingOptions")?;
    Ok(host_ref("TracingObject", &handle))
}

/// A preset name given to `method`: a string of at most [`MAX_PRESET_NAME`] bytes without `/`,
/// `\`, `:` or control characters, trimmed.
fn preset_name<'a>(d: &'a Data, method: &str) -> Result<&'a str, HostError> {
    let Data::String(name) = d else { return Err(HostError::Type(format!("TracingOptions.{method} needs a preset name, as a string"))) };
    if name.len() > MAX_PRESET_NAME {
        return Err(HostError::Range(format!("TracingOptions.{method}: a preset name has at most {MAX_PRESET_NAME} bytes")));
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':') || c.is_control()) {
        return Err(HostError::Error(format!("TracingOptions.{method}: a preset name cannot hold / \\ : or control characters")));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(HostError::Error(format!("TracingOptions.{method}: the preset name is empty")));
    }
    Ok(name)
}

/// `loadFromPreset(name)`: traces again with the built-in or saved preset `name` → `true`.
fn options_load(h: &Host, a: &[Data]) -> Answer {
    let name = preset_name(arg(a, 1), "loadFromPreset")?;
    let (item, params, _, preset) = options_of(h, a)?;
    let wanted = vectorcraft_trace::preset(name).or_else(|| h.session.borrow().trace_preset(name).map(|q| q.params.clone()));
    let wanted = wanted.ok_or_else(|| HostError::Error("TracingOptions.loadFromPreset: no built-in or saved preset has that name".into()))?;
    let same = serde_json::to_value(&wanted).ok().as_ref().and_then(Value::as_object) == Some(&params);
    if !(same && preset.eq_ignore_ascii_case(name)) {
        let mut p = Map::new();
        p.insert("preset".into(), json!(name));
        retrace(h, item, p)?;
    }
    Ok(Data::Bool(true))
}

/// `storeToPreset(name)`: `false` for a built-in preset or `Custom`; any other name runs
/// `imageTrace.presets.save`, which a script cannot run.
fn options_store(h: &Host, a: &[Data]) -> Answer {
    let name = preset_name(arg(a, 1), "storeToPreset")?;
    let (_, params, ..) = options_of(h, a)?;
    if vectorcraft_trace::preset(name).is_some() || name.eq_ignore_ascii_case("Custom") {
        return Ok(Data::Bool(false));
    }
    command(h, "imageTrace.presets.save", &json!({"name": name, "params": params}))?;
    Ok(Data::Bool(true))
}

/// A member of `TracingOptions` with no VectorCraft setting: reading and assigning it both throw,
/// after the check that the tracing `this` names is still there.
fn missing(link: &Link, name: &'static str) -> (String, HostMember) {
    let message = not_implemented("TracingOptions", name);
    let l = link.clone();
    let f: vectorcraft_js::HostFunction = std::rc::Rc::new(move |a: Vec<Data>| {
        let h = super::current(&l)?;
        h.charge_call(16)?;
        tracing_of(&h, arg(&a, 0), "TracingOptions")?;
        Err(HostError::Error(message.clone()))
    });
    (name.to_string(), HostMember::Property { get: f.clone(), set: f })
}

/// Adds `trace()` to the `PlacedItem` and `RasterItem` classes in `classes`.
pub(super) fn add_trace(classes: &mut [HostClass], l: &Link) {
    for c in classes.iter_mut() {
        match c.name.as_str() {
            "PlacedItem" => c.members.push(("trace".into(), method(l, placed_trace))),
            "RasterItem" => c.members.push(("trace".into(), method(l, raster_trace))),
            _ => {}
        }
    }
}

/// `PluginItem`, after `PageItem`, `TracingObject` and `TracingOptions`.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let plugin = named(vec![("isTracing", get(l, plugin_is_tracing)), ("tracing", get(l, plugin_tracing))]);
    let tracing = named(vec![
        ("typename", typename_of("TracingObject")),
        ("anchorCount", get(l, tracing_anchors)),
        ("areaCount", get(l, tracing_areas)),
        ("imageResolution", get(l, tracing_resolution)),
        ("parent", get(l, tracing_parent)),
        ("pathCount", get(l, tracing_paths)),
        ("sourceArt", get(l, tracing_source)),
        ("tracingOptions", get(l, tracing_options)),
        ("usedColorCount", get(l, tracing_colors)),
        ("expandTracing", method(l, tracing_expand)),
        ("releaseTracing", method(l, tracing_release)),
    ]);
    let mut options = named(vec![
        ("typename", typename_of("TracingOptions")),
        ("parent", get(l, options_parent)),
        ("preset", get(l, options_preset)),
        ("tracingMode", property(l, options_mode, options_set_mode)),
        ("threshold", property(l, options_threshold, options_set_threshold)),
        ("fills", property(l, options_fills, options_set_fills)),
        ("strokes", property(l, options_strokes, options_set_strokes)),
        ("maxColors", property(l, options_max_colors, options_set_max_colors)),
        ("palette", property(l, options_palette, options_set_palette)),
        ("ignoreWhite", property(l, options_ignore_white, options_set_ignore_white)),
        ("minArea", property(l, options_min_area, options_set_min_area)),
        ("viewRaster", property(l, options_view_raster, options_set_view_raster)),
        ("viewVector", property(l, options_view_vector, options_set_view_vector)),
        ("loadFromPreset", method(l, options_load)),
        ("storeToPreset", method(l, options_store)),
    ]);
    options.extend(NOT_IMPLEMENTED.iter().map(|m| missing(l, m)));
    vec![
        class("PluginItem", Some("PageItem"), plugin, None),
        class("TracingObject", None, tracing, None),
        class("TracingOptions", None, options, None),
    ]
}
