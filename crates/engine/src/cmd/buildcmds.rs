//! Shape Builder (`shapeBuilder.*`), Live Paint (`livePaint.*`) and Image Trace (`imageTrace.*`).
//!
//! **Live Paint groups** need no special node kind: a Live Paint group is a plain group whose name
//! starts with "Live Paint" and that holds a group named "Live Paint Sources". That group is its
//! first child, hidden, and holds the original paths (the planar map's inputs, used by Release,
//! Merge and re-computation); then come the faces (paths named "Face", fill only) and the edges
//! (open paths named "Edge", stroke only). See `vectorcraft_tools::builder` for the shared helpers.
//!
//! **Image Trace** makes a group named "Image Trace" holding the source image (hidden, bottom)
//! and the traced filled paths above it, mapped with the image's transform. Expand drops the
//! image (a plain group of paths remains); Release puts the image back in place of the group.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer, LineCap, LineJoin, StrokeLayer};
use vectorcraft_doc::{Appearance, Document, GapOptions, GapSize, LiveShape, Node, NodeId, NodeKind, Selection, TraceView};
use vectorcraft_geom::{FillRule, PathData, Point, Rect, SubPath};
use vectorcraft_pathops as po;
use vectorcraft_tools::builder::{
    self as b, BuilderMap, EDGE_NAME, FACE_NAME, LIVE_PAINT_NAME, SOURCES_NAME, Touched, edge_near, face_at, is_live_paint, sorted_roots,
};
use vectorcraft_trace as tr;

use super::draw2::replace_with_pieces;
use super::edit::{outermost, selected_roots};
use super::paint::paint_from;
use super::pathops::{Beside, shape_node};
use super::{
    CommandSpec, EngineError, Result, Session, always, bad, check_numbers, cmd, color_value, f64_or, finite_numbers, has_doc, id_param, ids_param,
    number_or, point_param, str_param, switch_or,
};

mod live_paint;
mod shape_builder;
mod trace;

pub use live_paint::live_paint_gaps;
use live_paint::{lp_expand, lp_fill, lp_gap_options, lp_info, lp_make, lp_merge, lp_release, lp_stroke_edge};
use shape_builder::{sb_merge, sb_regions};
pub(crate) use trace::TracePresetChanges;
pub use trace::{TracePreset, TracePresetChange};
use trace::{trace_expand, trace_make, trace_presets_delete, trace_presets_save, trace_release, trace_set_view};

/// Name of an Image Trace group.
pub const TRACE_NAME: &str = "Image Trace";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shapeBuilder.merge",
            "Shape Builder",
            [],
            None,
            "{ids?|id?: [id…] (default: selection), points: [[x,y]…] (drag path; one point = click; may be left out with rect), rect?: [x, y, width, height] (Shift-drag marquee from where the drag started, x and y, to x+width and y+height; width and height may be negative: touches every region whose outline crosses the rectangle or has an anchor inside it and the region holding the whole rectangle, the region under x, y comes first and the one under the opposite corner comes next; with points, it also touches the regions along them; for pickColorFrom, x, y is the drag's first point and the opposite corner is its last), erase?: bool (erase mode, Alt: deletes the regions touched, and the pieces of open paths that cut regions and the edges (outline pieces between the points where paths meet; see shapeBuilder.regions) within tolerance, except that a point in a region no more than twice the tolerance across there touches the region; a deleted edge opens its path there, and edges bounding a deleted region go with it; what is left of each path keeps its look, with gradients fitted to its bounds kept where they were on it, its name, opacity mask and attributes, and its fill rule unless merged regions shaped it), splitStroke?: false (In Merge Mode, Clicking Stroke Splits the Path; not with erase or rect: when the one point of a click is within tolerance of a stroke, the path that stroke is on splits in two instead of a merge: the path keeps the rest of it, and a copy of the path holding the run of it from the anchor or crossing point before the click to the one after it, in the path's direction, goes directly above it; both keep the path's gradients where they were on it; the other objects stay as they are; with pickColorFrom \"swatches\" or fill, the run's stroke takes that paint; a path in a Live Paint group or a Shaper Group does not split, and nor does a path the run is all of, such as a lone line, or all but 0.05 pt or less of (a ten-thousandth of the larger side of a path over 500 pt)), tolerance?: pt (2, erase or splitStroke), gap?: pt (0–72; 0: Gap Detection off), openFilledAsClosed?: true (Consider Open Filled Paths as Closed: a filled open path bounds a region as if an invisible edge closed it; false: it cuts the regions it crosses like an unfilled one), pickColorFrom?: \"artwork\" (default: the merged shape takes the look of the front-most object under the first point, else under the last point, else of the front-most object) | \"swatches\" (the current fill, with the rest of the look of the front-most object over the first region the drag touches that an object covers, else of the front-most object), fill?: \"#rrggbb\" | {color|swatch|gradient|none} (overrides pickColorFrom)} replaces the paths it uses where they are; other selected objects (type, hidden or locked objects, clipping paths) stay, and a group goes only when it is left empty → {ids (selected after: each selected object still there, or what took its place), merged, regions, lines, edges (counts deleted, merged or split), split?: id (the run split off, after a split)}; after a split, ids are the roots, with the rest and the run in place of the path when it was one",
            has_selection_or_ids,
            sb_merge
        ),
        cmd!(
            query "shapeBuilder.regions",
            "Shape Builder Regions",
            [],
            None,
            "{ids?|id?, gap?: pt (0–72; 0: Gap Detection off), openFilledAsClosed?: true (as shapeBuilder.merge)} → {regions: [{bounds: [x0,y0,x1,y1], area, sources: [id…]}], lines: [{bounds, source: id}], edges: [{bounds, source: id}]} faces of the planar arrangement (unfilled open paths cut them, and filled ones too with openFilledAsClosed false; sources is empty for an area only they enclose), the pieces of those open paths and the edges (the other paths' outlines cut where paths meet; source: the front-most path along it), which erasing deletes",
            has_selection_or_ids,
            sb_regions
        ),
        cmd!(
            "livePaint.make",
            "Make",
            ["Object", "Live Paint"],
            Some("Cmd+Alt+X"),
            "{ids?|id?} build a Live Paint group from the selected paths (paths, compound paths and groups of them; type, images and other objects are refused, nothing changes); with a Live Paint group among them, add the others to it as livePaint.merge does → {id, faces, edges}",
            has_selection_or_ids,
            lp_make
        ),
        cmd!(
            "livePaint.merge",
            "Merge",
            ["Object", "Live Paint"],
            None,
            "{ids?|id?} add the other selected paths (compound paths, groups of paths and Live Paint groups too; type, images and other objects are refused, nothing changes) to the bottom-most Live Paint group, keeping its id, its paint and its own properties (name, lock, visibility, transparency, appearance, opacity mask, graphic style link) → {id, faces, edges}",
            has_multi_or_ids,
            lp_merge
        ),
        cmd!(
            "livePaint.release",
            "Release",
            ["Object", "Live Paint"],
            None,
            "{id?, ids?: [id…] (instead of id)} replace Live Paint groups with their original paths → {ids}",
            has_selection_or_ids,
            lp_release
        ),
        cmd!(
            "livePaint.expand",
            "Expand",
            ["Object", "Live Paint"],
            None,
            "{id?, ids?: [id…] (instead of id)} turn Live Paint groups into plain groups of their painted faces and edges → {ids}",
            has_selection_or_ids,
            lp_expand
        ),
        cmd!(
            "livePaint.gapOptions",
            "Gap Options",
            [],
            None,
            "{id?, ids?: [id…] (instead of id; default: the selected Live Paint groups), detect?: bool (Gap Detection: paint stops at gaps up to size wide), size?: \"small\" (3 pt) | \"medium\" (6 pt) | \"large\" (12 pt) | pt (custom, over 0 up to 72) (Paint Stops At), color?: #rrggbb or a layer color's name such as \"Red\" (Gap Preview Color), closeGaps?: bool (add an unpainted path across each gap up to size wide to the group's paths)} set the Gap Options of Live Paint groups (what is not given stays as it was; a group starts with detect false, small and Red) and make their faces again, keeping their paint → {ids, faces, gaps: [[x0,y0,x1,y1]…] (the gaps up to size wide that no path closes, from a loose end of a path to where it closes to; detect closes them for paint), closed (paths added)}",
            has_selection_or_ids,
            lp_gap_options
        ),
        cmd!(
            "livePaint.fill",
            "Live Paint Bucket",
            [],
            None,
            "{group?: id | ids?|id?: [id…] (paths to make live first, as livePaint.make does), point: [x,y], color?|swatch?|gradient?|none? (default: current fill), tint?: 0..100, keepModel?: bool (both as paint.setFill)} fill the face containing point, skipping hidden faces → {group, face}",
            has_doc,
            lp_fill
        ),
        cmd!(
            "livePaint.strokeEdge",
            "Live Paint Bucket (Stroke)",
            [],
            None,
            "{group: id, point: [x,y], color?|swatch?|none? (default: current stroke), width?: pt (default: current weight), tolerance?: pt (4), tint?: 0..100, keepModel?: bool (both as paint.setFill)} paint the edge nearest point → {group, edge}",
            has_doc,
            lp_stroke_edge
        ),
        cmd!(
            query "livePaint.info",
            "Live Paint Info",
            [],
            None,
            "{group, id?: object id (default: the selection)} → {faces: [{id, fill, bounds}], edges: [{id, stroke, width}], sources, gapOptions: {detect, size: \"small\"|\"medium\"|\"large\"|pt, points, color: #rrggbb} (as livePaint.gapOptions sets them), gaps: [[x0,y0,x1,y1]…] (as livePaint.gapOptions returns them)}",
            has_doc,
            lp_info
        ),
        cmd!(
            "imageTrace.make",
            "Make",
            ["Object", "Image Trace"],
            None,
            "{id?: image or Image Trace group (default: selection), preset?: name (imageTrace.presets), savedPreset?: {settings} (a saved preset's settings, which the journal records so a replay traces with them after the preset is changed, renamed or deleted; used in place of the saved preset `preset` names), enhanced?: false (the preset's enhanced version: with Auto Grouping, and with Shapes in all but High Fidelity Photo; Flat Logo has none and is traced as it is), params?: {mode: \"blackAndWhite\"|\"grayscale\"|\"color\"|\"logo\", threshold: 0-255, palette: \"limited\"|\"fullTone\"|\"automatic\"|\"documentLibrary\" (color mode; limited: the `colors` best colours, fullTone: as many as the tones need, more with colorDetail, automatic: flat art with its own colours, else fullTone, documentLibrary: at most `colors` of the library's colours, as they are), colors: 2-256, colorDetail: 0-100 (fullTone, automatic), library: \"document\" (the document's swatches) or a swatch library id or name (swatch.library.list; documentLibrary), colorGroup?: name (color mode, documentLibrary with library \"document\": the colors of that document color group instead of every swatch; \"\" is every swatch), paths: 0-100, corners: 0-100, noise: px, method: \"abutting\"|\"overlapping\", ignoreWhite (leave out white areas), ignoreColor?: \"#rrggbb\" (with ignoreWhite: the colour left out instead of white), snapCurvesToLines, shapes: false (filled outlines that are a circle, an ellipse, a rectangle or a regular polygon come out as that live shape), autoGrouping: false (each path that has others inside it is grouped with them), gradients: false (grayscale and color: an area whose colour changes evenly along one direction is one path with a linear gradient fill), fills: true, strokes: false (Create: lines up to strokeWidth wide become stroked centre lines; without fills, wider areas become stroked outlines), strokeWidth: px (10), logoColors?: [\"#rrggbb\"…] (mode logo: flat-colour logos only; colours found automatically or named here, white is always its own colour, at most ten; noise = areas under noise/10 px ignored; edges to a fraction of a pixel, sharp corners restored, lines and circles snapped; enclosed transparent areas are painted white unless ignoreWhite makes them holes; fills/strokes are ignored; gradients and photos are refused)}, view?: as imageTrace.setView (default: the traced object's own, else tracingResult), ids?: [id…] (instead of id)} → {id, paths, anchors, colors, warnings: [text…] (the image is over 1500 px on a side, so it traces more slowly; or it was traced scaled down; Flat Logo names only the size it traced a larger image at)}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, false)
        ),
        cmd!(
            "imageTrace.makeAndExpand",
            "Make and Expand",
            ["Object", "Image Trace"],
            None,
            "{…as imageTrace.make} same as imageTrace.make, then expand → {id, paths, anchors, colors}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, true)
        ),
        cmd!(
            "imageTrace.release",
            "Release",
            ["Object", "Image Trace"],
            None,
            "{id?, ids?: [id…] (instead of id)} put the source image back in place of Image Trace groups → {ids}",
            has_selection_or_ids,
            trace_release
        ),
        cmd!(
            "imageTrace.expand",
            "Expand",
            ["Object", "Image Trace"],
            None,
            "{id?, ids?: [id…] (instead of id)} keep only the traced paths (drop the source image) → {ids}",
            has_selection_or_ids,
            trace_expand
        ),
        cmd!(
            "imageTrace.setView",
            "Image Trace View",
            [],
            None,
            "{id?|ids?: Image Trace objects (default: selection), view: \"tracingResult\"|\"tracingResultWithOutlines\"|\"outlines\"|\"outlinesWithSourceImage\"|\"sourceImage\"} how they draw on screen; exports, printing and Expand keep the tracing result → {ids, view}",
            has_selection_or_ids,
            trace_set_view
        ),
        cmd!(query "imageTrace.presets", "Image Trace Presets", [], None, "{} → {presets: [{name, builtIn, params, enhanced: the params of the preset's enhanced version, null if it has none}], revision: grows each time the saved presets change} the built-in presets, then the saved ones (builtIn false, enhanced null); any of these names works as imageTrace.make's `preset`", always, |s, _| {
            let builtin = tr::presets().into_iter().map(|(n, p)| json!({ "name": n, "builtIn": true, "params": p, "enhanced": tr::enhanced_preset(n) }));
            let saved = s.prefs.trace_presets.iter().map(|p| json!({ "name": p.name, "builtIn": false, "params": p.params, "enhanced": null }));
            Ok(json!({ "presets": builtin.chain(saved).collect::<Vec<_>>(), "revision": s.trace_presets_revision() }))
        }),
        cmd!(
            query "imageTrace.presets.save",
            "Save Image Trace Preset",
            [],
            None,
            "{name?: (default: a new \"Tracing Preset N\"), newName?: rename the saved preset `name`, preset?: the preset to start from (default: the saved preset `name`, else Default), enhanced?, params?: {settings, as imageTrace.make takes them, over the preset's}} create or change a saved tracing preset, kept with the preferences (built-in ones can't change) → {name, params, created, renamedFrom: the name before `newName` changed it, else null}",
            always,
            trace_presets_save
        ),
        cmd!(
            query "imageTrace.presets.delete",
            "Delete Image Trace Preset",
            [],
            None,
            "{name} delete a saved tracing preset (built-in ones stay) → {deleted: name}",
            always,
            trace_presets_delete
        ),
    ]
}

fn has_selection_or_ids(s: &Session) -> std::result::Result<(), String> {
    // Agents may pass ids explicitly; menus need a selection. Accept either (params are checked in run).
    has_doc(s)
}

fn has_multi_or_ids(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)
}

fn root_ids(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let d = &s.doc()?.doc;
    if let Some(bad_id) = ids.iter().find(|i| d.node(**i).is_none()) {
        return Err(EngineError::NoNode(*bad_id));
    }
    // Layers are containers, not objects.
    let ids: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(|n| !n.is_layer())).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("select objects, not layers".into()));
    }
    Ok(outermost(d, sorted_roots(d, &ids)))
}

fn points_param(p: &Value, key: &str) -> Option<Vec<Point>> {
    p.get(key)?.as_array()?.iter().map(|v| Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))).collect()
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= crate::MAX_COORD && p.y.abs() <= crate::MAX_COORD
}

fn fill_only(paint: Paint) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], ..Default::default() }
}

fn stroke_only(paint: Paint, width: f64) -> Appearance {
    Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, width))], ..Default::default() }
}

// ---------- Shape Builder ----------

fn fill_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Paint>> {
    match p.get("fill") {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Object(o)) if o.contains_key("color") || o.contains_key("swatch") || o.contains_key("gradient") || o.contains_key("none") => {
            paint_from(s, v)
        }
        Some(v) => color_value(v).map(|c| Some(Paint::solid(super::paint::new_color(s, p, c)))).ok_or_else(|| bad(cmd, format!("bad fill {v}"))),
    }
}

/// Gap Detection length in points, bounded like Live Paint's custom gap size.
fn gap_param(p: &Value) -> f64 {
    let gap = f64_or(p, "gap", 0.0);
    if gap.is_finite() { gap.clamp(0.0, po::SHAPE_BUILDER_MAX_GAP) } else { 0.0 }
}

/// The Shift-drag marquee `rect: [x, y, width, height]`, as the point where the drag started
/// (`x`, `y`) and the opposite corner; the width and height may be negative.
fn marquee_param(p: &Value, cmd: &str) -> Result<Option<(Point, Point)>> {
    let Some(v) = p.get("rect").filter(|v| !v.is_null()) else { return Ok(None) };
    let corners = finite_numbers(v).map(|[x, y, w, h]| (Point::new(x, y), Point::new(x + w, y + h)));
    match corners {
        Some((a, b)) if finite(a) && finite(b) => Ok(Some((a, b))),
        _ => Err(bad(cmd, "rect must be [x, y, width, height]")),
    }
}

/// Remove the roots and insert `nodes` where the front-most root was. Returns the new ids.
/// (Roots go first: the new nodes may reuse their ids, e.g. Live Paint sources.)
fn replace_roots(d: &mut Document, roots: &[NodeId], nodes: Vec<Node>) -> Result<Vec<NodeId>> {
    let Some((&top, rest)) = roots.split_last() else { return Err(EngineError::Other("nothing selected".into())) };
    let ids = nodes.iter().map(|n| n.id).collect();
    let mut replaced = Beside::replacing(d, roots);
    for r in rest {
        if d.node(*r).is_some() {
            replaced.put(d, *r, vec![])?;
        }
    }
    replaced.put(d, top, nodes)?;
    replaced.finish(d)?;
    Ok(ids)
}

/// What is left of path or compound path `src`: a copy of it with a fresh id and the geometry `path`
/// (a path for one subpath, else a compound path, as [`shape_node`] makes them). It keeps the rest
/// of `src`: its look, with gradients fitted to its bounds kept where they were on it, its name,
/// opacity mask and attributes, and its fill rule unless `rule` is given.
fn remnant(d: &mut Document, src: &Node, path: PathData, rule: Option<FillRule>) -> Node {
    let own = match &src.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        _ => FillRule::NonZero,
    };
    let shaped = shape_node(d, path, None);
    let mut n = src.clone();
    n.pin_gradients();
    n.id = shaped.id;
    n.kind = shaped.kind;
    if let NodeKind::Path { rule: r, .. } | NodeKind::Compound { rule: r, .. } = &mut n.kind {
        *r = rule.unwrap_or(own);
    }
    n
}

/// Put each list of `outs` in place of the leaf of `leaves` at the same index (none: the leaf stays
/// as it is), then remove the groups under `roots` that this leaves empty. A Live Paint or Shaper
/// group (the outermost one under the root) is replaced as a whole, by the lists of its leaves,
/// with a copy of each leaf that stays and of each hidden or locked object in it. Other objects
/// under the roots, such as type, hidden or locked objects and clipping paths, stay where they are. Returns what to select: each root
/// still there, or what went in its place.
fn replace_leaves(d: &mut Document, roots: &[NodeId], leaves: &[Node], outs: Vec<Option<Vec<Node>>>) -> Result<Vec<NodeId>> {
    let listed: HashSet<NodeId> = roots.iter().copied().collect();
    let paths = d.index_paths(leaves.iter().map(|l| l.id));
    // Each leaf's unit (the leaf, or the group replaced as a whole) and the groups from the unit's
    // parent up to its root, innermost first.
    let mut places: Vec<(NodeId, Vec<NodeId>)> = vec![];
    for leaf in leaves {
        let anc = paths.get(&leaf.id).and_then(|p| d.ancestry_at(p)).unwrap_or_default();
        let below = anc.iter().position(|a| listed.contains(a)).and_then(|r| anc.get(r..)).unwrap_or_default();
        let whole = below.iter().position(|a| d.node(*a).is_some_and(|n| is_live_paint(n) || n.extras().shaper.is_some()));
        let at = whole.unwrap_or(below.len().saturating_sub(1));
        let unit = below.get(at).copied().unwrap_or(leaf.id);
        places.push((unit, below.get(..at).unwrap_or_default().iter().rev().copied().collect()));
    }
    // (unit, groups above it, what goes in place of each of its leaves, whether it changes); a
    // unit's leaves are next to each other, in paint order.
    type Unit = (NodeId, Vec<NodeId>, Vec<(NodeId, Vec<Node>)>, bool);
    let mut units: Vec<Unit> = vec![];
    for ((leaf, out), (unit, above)) in leaves.iter().zip(outs).zip(places) {
        let changed = out.is_some();
        let list = match out {
            Some(list) => list,
            None if unit != leaf.id => vec![d.reid(leaf)],
            None => vec![],
        };
        match units.last_mut() {
            Some(u) if u.0 == unit => {
                u.2.push((leaf.id, list));
                u.3 |= changed;
            }
            _ => units.push((unit, above, vec![(leaf.id, list)], changed)),
        }
    }
    units.retain(|u| u.3);
    let ids: Vec<NodeId> = units.iter().map(|u| u.0).collect();
    let mut replaced = Beside::replacing(d, &ids);
    let mut put: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut emptied = vec![];
    for (unit, above, lists, _) in units {
        // A group replaced as a whole keeps a copy of each hidden or locked object in it, where it was.
        let members: Vec<Node> = d.node(unit).map(|n| b::builder_members(n).into_iter().cloned().collect()).unwrap_or_default();
        let mut at: HashMap<NodeId, usize> = HashMap::new();
        for (k, l) in lists.iter().enumerate() {
            at.entry(l.0).or_insert(k);
        }
        let mut lists: Vec<Option<Vec<Node>>> = lists.into_iter().map(|l| Some(l.1)).collect();
        let mut list = vec![];
        for m in &members {
            if let Some(l) = at.remove(&m.id).and_then(|k| lists.get_mut(k)).and_then(Option::take) {
                list.extend(l);
            } else if (m.locked || !m.visible) && m.id != unit {
                list.push(d.reid(m));
            }
        }
        list.extend(lists.into_iter().flatten().flatten());
        if list.is_empty() {
            emptied.push(above);
        }
        put.insert(unit, list.iter().map(|n| n.id).collect());
        replaced.put(d, unit, list)?;
    }
    replaced.finish(d)?;
    // A group goes only when nothing is left in it.
    for above in emptied {
        for g in above {
            match d.node(g) {
                None => continue,
                Some(n) if !n.is_layer() && n.children().is_some_and(|c| c.is_empty()) => {
                    d.remove(g)?;
                }
                Some(_) => break,
            }
        }
    }
    Ok(roots.iter().flat_map(|r| put.get(r).cloned().unwrap_or_else(|| d.node(*r).map(|_| vec![*r]).unwrap_or_default())).collect())
}

// ---------- Live Paint ----------

/// The source nodes a root contributes (a Live Paint group contributes its own sources).
fn source_nodes(n: &Node) -> Vec<Arc<Node>> {
    if is_live_paint(n) { b::sources(n).and_then(|g| g.children().cloned()).unwrap_or_default() } else { vec![Arc::new(n.clone())] }
}

/// The planar map's inputs from `sources` (paths and compound paths, back → front), with the
/// fill, stroke and stroke width of each.
fn source_shapes(sources: &[Arc<Node>]) -> (Vec<po::Shape>, Vec<(Paint, Paint, f64)>) {
    let mut lv = vec![];
    for n in sources {
        b::leaf_shapes(n, &mut lv);
    }
    let mut shapes = vec![];
    let mut styles: Vec<(Paint, Paint, f64)> = vec![];
    for l in lv {
        if let Some((path, rule)) = b::node_outline(l)
            && !path.is_empty()
        {
            shapes.push(po::Shape::new(path, rule, styles.len() as u64));
            let a = &l.appearance;
            styles.push((a.fill_paint(), a.stroke_paint(), a.stroke().map(|s| s.width).unwrap_or(1.0)));
        }
    }
    (shapes, styles)
}

/// Planar map children (sources group, faces, edges) for `sources`; with `max_gap` (Gap
/// Detection), paint stops at the gaps up to that wide.
fn build_children(d: &mut Document, sources: Vec<Arc<Node>>, max_gap: Option<f64>) -> Result<(Vec<Arc<Node>>, usize, usize)> {
    let (shapes, styles) = source_shapes(&sources);
    if shapes.is_empty() {
        return Err(EngineError::Other("Live Paint: select paths or compound paths".into()));
    }
    let mut children: Vec<Arc<Node>> = vec![];
    let gid = d.alloc_id();
    let mut src_group = Node::group(gid, sources);
    src_group.name = Some(SOURCES_NAME.into());
    src_group.visible = false;
    children.push(Arc::new(src_group));
    // Every path is an edge, open ones included; faces are the areas they enclose.
    let (regs, edges) = match max_gap {
        Some(gap) => {
            let (regs, edges, _) = po::live_paint_with_gaps(&shapes, gap);
            (regs, edges)
        }
        None => po::live_paint(&shapes),
    };
    if regs.is_empty() && edges.is_empty() {
        return Err(EngineError::Other("Live Paint: these paths are too degenerate to paint".into()));
    }
    let nfaces = regs.len();
    for r in regs {
        let id = d.alloc_id();
        let fill = r.top().and_then(|t| styles.get(t)).map_or(Paint::None, |s| s.0.clone());
        let mut n = Node::path(id, r.path, fill_only(fill));
        n.name = Some(FACE_NAME.into());
        children.push(Arc::new(n));
    }
    let nedges = edges.len();
    for e in edges {
        let Some((_, stroke, w)) = styles.get(e.key as usize) else { continue };
        let id = d.alloc_id();
        let mut n = Node::path(id, e.path, stroke_only(stroke.clone(), *w));
        n.name = Some(EDGE_NAME.into());
        children.push(Arc::new(n));
    }
    Ok((children, nfaces, nedges))
}

/// Give the faces and edges of `g`, a Live Paint group made again from the sources of `old`, the
/// look of `old`'s face or edge where they lie: each face takes everything but the outline (its
/// appearance, none included, its transparency and the rest) of the old face its inside point is
/// in, and each edge that of the old edge its middle lies on. Faces and edges where `old` had none
/// keep the paint of their sources.
fn repaint_from(g: &mut Node, old: &Node) {
    /// How far an edge's middle may be from the old edge it lies on.
    const ON_EDGE: f64 = 1e-3;
    let Some(ch) = g.children_mut() else { return };
    for c in ch.iter_mut() {
        let Some(pd) = c.path_data() else { continue };
        let was = match c.name.as_deref() {
            Some(FACE_NAME) => po::interior_point(pd).and_then(|pt| face_at(old, pt)),
            Some(EDGE_NAME) => pd
                .subpaths
                .first()
                .filter(|sp| sp.segment_count() > 0)
                .and_then(|sp| edge_near(old, kurbo::ParamCurve::eval(&sp.segment(0), 0.5), ON_EDGE)),
            _ => None,
        };
        if let Some(was) = was {
            let mut n = (**was).clone();
            // Gradients fitted to the old outline keep that fit on the new one.
            n.pin_gradients();
            n.id = c.id;
            n.kind = c.kind.clone();
            n.bbox_angle = c.bbox_angle;
            *c = Arc::new(n);
        }
    }
}
