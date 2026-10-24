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
use super::*;

/// Name of an Image Trace group.
pub const TRACE_NAME: &str = "Image Trace";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shapeBuilder.merge",
            "Shape Builder",
            [],
            None,
            "{ids?|id?: [id…] (default: selection), points: [[x,y]…] (drag path; one point = click; may be left out with rect), rect?: [x, y, width, height] (Shift-drag marquee from where the drag started, x and y, to x+width and y+height; width and height may be negative: touches every region whose outline crosses the rectangle or has an anchor inside it and the region holding the whole rectangle, the region under x, y comes first and the one under the opposite corner comes next; with points, it also touches the regions along them; for pickColorFrom, x, y is the drag's first point and the opposite corner is its last), erase?: bool (erase mode, Alt: deletes the regions touched, and the pieces of open paths that cut regions and the edges (outline pieces between the points where paths meet; see shapeBuilder.regions) within tolerance; a deleted edge opens its path there, and edges bounding a deleted region go with it; what is left of each path keeps its look, with gradients fitted to its bounds kept where they were on it, its name, opacity mask and attributes, and its fill rule unless merged regions shaped it), splitStroke?: false (In Merge Mode, Clicking Stroke Splits the Path; not with erase or rect: when the one point of a click is within tolerance of a stroke, the path that stroke is on splits in two instead of a merge: the path keeps the rest of it, and a copy of the path holding the run of it from the anchor or crossing point before the click to the one after it, in the path's direction, goes directly above it; both keep the path's gradients where they were on it; the other objects stay as they are; with pickColorFrom \"swatches\" or fill, the run's stroke takes that paint; a path in a Live Paint group or a Shaper Group does not split, and nor does a path the run is all of, such as a lone line, or all but 0.05 pt or less of (a ten-thousandth of the larger side of a path over 500 pt)), tolerance?: pt (2, erase or splitStroke), gap?: pt (0–72; 0: Gap Detection off), openFilledAsClosed?: true (Consider Open Filled Paths as Closed: a filled open path bounds a region as if an invisible edge closed it; false: it cuts the regions it crosses like an unfilled one), pickColorFrom?: \"artwork\" (default: the merged shape takes the look of the front-most object under the first point, else under the last point, else of the front-most object) | \"swatches\" (the current fill, with the rest of the look of the front-most object over the first region the drag touches that an object covers, else of the front-most object), fill?: \"#rrggbb\" | {color|swatch|gradient|none} (overrides pickColorFrom)} → {ids, merged, regions, lines, edges (counts deleted, merged or split), split?: id (the run split off, after a split)}; after a split, ids are the roots, with the rest and the run in place of the path when it was one",
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
            "{id?: image or Image Trace group (default: selection), preset?: name (imageTrace.presets), enhanced?: false (the preset's enhanced version: with Auto Grouping, and with Shapes in all but High Fidelity Photo; Flat Logo has none and is traced as it is), params?: {mode: \"blackAndWhite\"|\"grayscale\"|\"color\"|\"logo\", threshold: 0-255, palette: \"limited\"|\"fullTone\"|\"automatic\"|\"documentLibrary\" (color mode; limited: the `colors` best colours, fullTone: as many as the tones need, more with colorDetail, automatic: flat art with its own colours, else fullTone, documentLibrary: at most `colors` of the library's colours, as they are), colors: 2-256, colorDetail: 0-100 (fullTone, automatic), library: \"document\" (the document's swatches) or a swatch library id or name (swatch.library.list; documentLibrary), paths: 0-100, corners: 0-100, noise: px, method: \"abutting\"|\"overlapping\", ignoreWhite (leave out white areas), ignoreColor?: \"#rrggbb\" (with ignoreWhite: the colour left out instead of white), snapCurvesToLines, shapes: false (filled outlines that are a circle, an ellipse, a rectangle or a regular polygon come out as that live shape), autoGrouping: false (each path that has others inside it is grouped with them), gradients: false (grayscale and color: an area whose colour changes evenly along one direction is one path with a linear gradient fill), fills: true, strokes: false (Create: lines up to strokeWidth wide become stroked centre lines; without fills, wider areas become stroked outlines), strokeWidth: px (10), logoColors?: [\"#rrggbb\"…] (mode logo: flat-colour logos only; colours found automatically or named here, white is always its own colour, at most ten; noise = areas under noise/10 px ignored; edges to a fraction of a pixel, sharp corners restored, lines and circles snapped; enclosed transparent areas are painted white unless ignoreWhite makes them holes; fills/strokes are ignored; gradients and photos are refused)}, view?: as imageTrace.setView (default: the traced object's own, else tracingResult), ids?: [id…] (instead of id)} → {id, paths, anchors, colors, warnings: [text…] (the image is over 1500 px on a side, so it traces more slowly; or it was traced scaled down; Flat Logo names only the size it traced a larger image at)}",
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
        cmd!(query "imageTrace.presets", "Image Trace Presets", [], None, "{} → {presets: [{name, params, enhanced: the params of the preset's enhanced version, null if it has none}]}", always, |_, _| {
            Ok(json!({ "presets": tr::presets().into_iter().map(|(n, p)| json!({ "name": n, "params": p, "enhanced": tr::enhanced_preset(n) })).collect::<Vec<_>>() }))
        }),
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

fn sb_merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shapeBuilder.merge";
    let marquee = marquee_param(p, C)?;
    let pts = match points_param(p, "points") {
        Some(pts) => pts,
        // A marquee needs no drag path.
        None if marquee.is_some() && p.get("points").is_none_or(Value::is_null) => vec![],
        None => return Err(bad(C, "missing points [[x,y]…]")),
    };
    if (pts.is_empty() && marquee.is_none()) || !pts.iter().all(|q| finite(*q)) {
        return Err(bad(C, "points must be a non-empty list of finite [x,y]"));
    }
    let erase = switch_or(p, "erase", false, C)?;
    check_numbers(p, &["tolerance", "gap"], C)?;
    // Line pieces and edges are only deleted (erase mode), never merged.
    let piece_tol = erase.then(|| f64_or(p, "tolerance", 2.0).clamp(0.0, 1000.0));
    // In Merge Mode, Clicking Stroke Splits the Path: a click (one point) on a stroke splits it; a
    // marquee does not.
    let split_tol = (switch_or(p, "splitStroke", false, C)? && !erase && pts.len() == 1 && marquee.is_none())
        .then(|| f64_or(p, "tolerance", 2.0).clamp(0.0, 1000.0));
    let open_filled_as_closed = switch_or(p, "openFilledAsClosed", true, C)?;
    let swatches = match p.get("pickColorFrom") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("artwork") => false,
            Some("swatches") => true,
            _ => return Err(bad(C, format!("pickColorFrom is \"artwork\" or \"swatches\", not {v}"))),
        },
    };
    let fill = match fill_param(s, p, C)? {
        None if swatches => Some(s.paint.fill.clone()),
        f => f,
    };
    let roots = root_ids(s, p)?;
    let label = if erase { "Shape Builder Delete" } else { "Shape Builder" };
    let (ids, merged, touched, split) = s.edit(label, |d, sel| {
        let (map, leaves) = BuilderMap::with_gap(d, &roots, erase || split_tol.is_some(), open_filled_as_closed, gap_param(p));
        let outlines = b::drawn_outlines(d, &leaves);
        let leaves: Vec<Node> = leaves.into_iter().cloned().collect();
        if map.shapes.is_empty() {
            return Err(EngineError::Other("Shape Builder: select paths or compound paths".into()));
        }
        if let Some(tol) = split_tol
            && let Some(&q) = pts.first()
            && let Some((i, run)) = map.stroke_at(&outlines, q, tol)
            && let (Some(leaf), Some(drawn)) = (leaves.get(i), outlines.get(i))
            && let Some(rest) = po::cut_out(drawn, &run, open_filled_as_closed && !leaf.appearance.fill_paint().is_none(), b::outline_snap(drawn))
            && rest.length() > b::outline_snap(drawn)
        {
            // The path keeps the rest of it, and a copy of it with the run under the click goes
            // directly above it; the other objects stay as they are. Gradients fitted to the
            // path's bounds keep that fit on both.
            if let Some(n) = d.node_mut(leaf.id) {
                n.pin_gradients();
            }
            let pieces = [rest, PathData::single(run)].into_iter().filter(|pd| !pd.is_empty()).collect();
            let mut replaced = Beside::replacing(d, &[leaf.id]);
            let new_ids = replace_with_pieces(d, &mut replaced, leaf.id, pieces)?;
            replaced.finish(d)?;
            let split = *new_ids.last().ok_or(EngineError::NoNode(leaf.id))?;
            // Pick Color From Color Swatches (or `fill`): the stroke takes that paint.
            if let Some(f) = &fill
                && let Some(n) = d.node_mut(split)
            {
                n.appearance.set_stroke(f.clone());
            }
            // The roots stay selected, with the two paths in place of the path when it was one.
            let ids: Vec<NodeId> = roots.iter().flat_map(|r| if *r == leaf.id { new_ids.clone() } else { vec![*r] }).collect();
            sel.set(ids.iter().copied());
            let mut touched = Touched::default();
            touched.add(map.hit(q, Some(tol)));
            return Ok((ids, None, touched, Some(split)));
        }
        let mut touched = Touched { regions: marquee.map(|(a, b)| map.regions_in_marquee(a, b)).unwrap_or_default(), ..Default::default() };
        touched.extend(&map.hits_along(&pts, piece_tol));
        touched.edges = map.erased_edges(&touched);
        if touched.is_empty() {
            return Err(EngineError::Other("Shape Builder: no region under the pointer".into()));
        }
        let cuts: Vec<&SubPath> = touched.edges.iter().filter_map(|&k| map.edges.get(k)?.0.path.subpaths.first()).collect();
        let picked: Vec<&po::Region> = touched.regions.iter().filter_map(|&i| map.regions.get(i).map(|r| &r.0)).collect();
        let faces = po::FaceMerger::with_gap(&map.shapes, gap_param(p));
        let union = if picked.is_empty() { PathData::default() } else { faces.merge(&picked) };
        // Pick Color From Artwork: the merged shape takes the look of the front-most object under the
        // drag's first point, else under its last point, else of the front-most object. Color
        // Swatches: the current fill, with the rest of the look of the front-most object over the
        // first region the drag touched that an object covers, else of the front-most object. A
        // marquee's drag starts at its first corner and ends at the opposite one.
        let top_at = |q: &Point| map.region_at(*q).and_then(|k| map.regions.get(k)).and_then(|(r, ..)| r.top());
        let (first, last) = match marquee {
            Some((a, b)) => (Some(a), Some(b)),
            None => (pts.first().copied(), pts.last().copied()),
        };
        let covered =
            if swatches { picked.iter().find_map(|r| r.top()) } else { first.as_ref().and_then(top_at).or_else(|| last.as_ref().and_then(top_at)) };
        let style_src = covered.or(map.shapes.len().checked_sub(1));
        let mut new_nodes: Vec<Node> = vec![];
        let mut merged_id = None;
        for (i, (sh, leaf)) in map.shapes.iter().zip(&leaves).enumerate() {
            let pieces: Vec<usize> = map.lines.iter().enumerate().filter(|(_, (l, _))| l.key == i as u64).map(|(k, _)| k).collect();
            if !pieces.is_empty() {
                // An open path loses the pieces deleted and those between two merged (or deleted) regions.
                let keep: Vec<usize> =
                    pieces.iter().copied().filter(|k| !touched.lines.contains(k) && !map.line_between(*k, &touched.regions)).collect();
                if keep.len() == pieces.len() {
                    new_nodes.push(d.reid(leaf));
                } else {
                    for k in keep {
                        if let Some((l, _)) = map.lines.get(k) {
                            new_nodes.push(remnant(d, leaf, l.path.clone(), None));
                        }
                    }
                }
            } else {
                // A shape loses the regions deleted or merged, then the edges deleted (each one on
                // its outline, shared edges from every shape along them).
                let in_union = picked.iter().any(|r| r.sources.contains(&i));
                let mut rest = if in_union {
                    // What is left of it: its faces not merged or deleted, united. They share their
                    // edges with the merged faces exactly, where subtracting the merged shape left
                    // slivers of the outline along them (#569).
                    let kept: Vec<&po::Region> = map
                        .regions
                        .iter()
                        .enumerate()
                        .filter(|(k, (r, ..))| r.sources.contains(&i) && !touched.regions.contains(k))
                        .map(|(_, (r, ..))| r)
                        .collect();
                    if kept.is_empty() { PathData::default() } else { faces.merge(&kept) }
                } else {
                    b::node_outline(leaf).map_or_else(|| sh.path.clone(), |o| o.0)
                };
                let fill_closes = open_filled_as_closed && !leaf.appearance.fill_paint().is_none();
                // Edges come back from the planar map within a few hundredths of the outlines they
                // follow, whatever else is selected.
                let cut_tol = b::outline_snap(&sh.path);
                let mut cut = false;
                for piece in &cuts {
                    if let Some(r) = po::cut_out(&rest, piece, fill_closes, cut_tol) {
                        rest = r;
                        cut = true;
                    }
                }
                if !in_union && !cut {
                    new_nodes.push(d.reid(leaf));
                } else if !rest.is_empty() {
                    // What is left of its own outline keeps its fill rule; united faces fill with non-zero.
                    new_nodes.push(remnant(d, leaf, rest, in_union.then_some(FillRule::NonZero)));
                }
            }
            if !erase && !union.is_empty() && Some(i) == style_src {
                let mut m = shape_node(d, union.clone(), Some(leaf));
                if let Some(f) = &fill {
                    m.appearance.set_fill(f.clone());
                }
                merged_id = Some(m.id);
                new_nodes.push(m);
            }
        }
        let new_ids = replace_roots(d, &roots, new_nodes)?;
        sel.set(new_ids.iter().copied());
        Ok((new_ids, merged_id, touched, None))
    })?;
    let mut out = json!({
        "ids": ids_json(&ids),
        "merged": merged.map(|m| m.0),
        "regions": touched.regions.len(),
        "lines": touched.lines.len(),
        "edges": touched.edges.len(),
    });
    if let Some(e) = split {
        out["split"] = json!(e.0);
    }
    Ok(out)
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

fn sb_regions(s: &mut Session, p: &Value) -> Result<Value> {
    let open_filled_as_closed = switch_or(p, "openFilledAsClosed", true, "shapeBuilder.regions")?;
    check_numbers(p, &["gap"], "shapeBuilder.regions")?;
    let roots = root_ids(s, p)?;
    let d = &s.doc()?.doc;
    let (map, leaves) = BuilderMap::with_gap(d, &roots, true, open_filled_as_closed, gap_param(p));
    let id_of = |i: usize| leaves.get(i).map(|n| n.id.0);
    let regions: Vec<Value> = map
        .regions
        .iter()
        .map(|(r, _, bb)| {
            json!({
                "bounds": [bb.x0, bb.y0, bb.x1, bb.y1],
                "area": po::area(&r.path, FillRule::NonZero),
                "sources": r.sources.iter().filter_map(|&i| id_of(i)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let pieces = |list: &[(po::Shape, Rect)]| -> Vec<Value> {
        list.iter().map(|(l, bb)| json!({ "bounds": [bb.x0, bb.y0, bb.x1, bb.y1], "source": id_of(l.key as usize) })).collect()
    };
    Ok(json!({ "regions": regions, "lines": pieces(&map.lines), "edges": pieces(&map.edges) }))
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

/// `options` as stored on a Live Paint group: none for the defaults.
fn stored_gap_options(options: GapOptions) -> Option<Box<GapOptions>> {
    (options != GapOptions::default()).then(|| Box::new(options))
}

/// The Gap Options of Live Paint groups merged into one, given bottom-most first: the bottom-most
/// group's when its Gap Detection is on; else Gap Detection on with Small Gaps when another
/// group's is on; else the bottom-most group's.
fn merged_gap_options(groups: &[GapOptions]) -> GapOptions {
    let Some(&bottom) = groups.first() else { return GapOptions::default() };
    if !bottom.detect && groups.iter().any(|g| g.detect) { GapOptions { detect: true, size: GapSize::Small, ..bottom } } else { bottom }
}

/// Refuse `roots` when one of them is something Live Paint can't take ([`b::live_paint_takes`]):
/// its faces and edges come from path outlines alone, so type or an image would vanish among the
/// group's hidden sources. Checked before anything changes.
fn check_live_paint_takes(d: &Document, roots: &[NodeId]) -> Result<()> {
    for r in roots {
        if !b::live_paint_takes(d.node(*r).ok_or(EngineError::NoNode(*r))?) {
            return Err(EngineError::Other(
                "Live Paint takes paths and compound paths: convert type, images and other objects to paths first".into(),
            ));
        }
    }
    Ok(())
}

/// Make a Live Paint group from `roots`, or add them to the first Live Paint group among them
/// as Merge does; returns (group, faces, edges).
fn make_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    check_live_paint_takes(d, roots)?;
    if roots.iter().any(|r| d.node(*r).is_some_and(is_live_paint)) {
        return merge_live(d, sel, roots);
    }
    let mut sources = vec![];
    for r in roots {
        let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
        sources.extend(source_nodes(n));
    }
    let (children, nf, ne) = build_children(d, sources, None)?;
    let gid = d.alloc_id();
    let mut g = Node::group(gid, children);
    g.name = Some(LIVE_PAINT_NAME.into());
    replace_roots(d, roots, vec![g])?;
    sel.set([gid]);
    Ok((gid, nf, ne))
}

fn lp_make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Make Live Paint", |d, sel| make_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

/// Add the other `roots` (back → front) to the first Live Paint group among them, which keeps its
/// id, the paint of its faces and edges, and its own properties apart from the Gap Options, which
/// are merged; returns (group, faces, edges).
fn merge_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    let target = roots
        .iter()
        .copied()
        .find(|r| d.node(*r).is_some_and(is_live_paint))
        .ok_or_else(|| EngineError::Other("Live Paint Merge: select a Live Paint group and the paths to add".into()))?;
    check_live_paint_takes(d, roots)?;
    let old = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
    // Paint to carry over: (point inside face, fill) and (point on edge, stroke).
    let keep_faces: Vec<(Point, Paint)> = b::faces(&old)
        .iter()
        .filter(|f| !f.appearance.fill_paint().is_none())
        .filter_map(|f| Some((po::interior_point(f.path_data()?)?, f.appearance.fill_paint())))
        .collect();
    let keep_edges: Vec<(Point, Appearance)> = b::edges(&old)
        .iter()
        .filter_map(|e| {
            let pd = e.path_data()?;
            let sp = pd.subpaths.first().filter(|sp| sp.segment_count() > 0)?;
            Some((kurbo::ParamCurve::eval(&sp.segment(0), 0.5), (*e.appearance).clone()))
        })
        .collect();
    let mut sources = source_nodes(&old);
    for r in roots {
        if *r != target {
            sources.extend(source_nodes(d.node(*r).ok_or(EngineError::NoNode(*r))?));
        }
    }
    // Merge applies the Gap Options' merge rules to the groups' options, bottom-most first.
    let groups: Vec<GapOptions> = roots.iter().filter_map(|r| d.node(*r)).filter(|n| is_live_paint(n)).map(Node::gap_options).collect();
    let options = merged_gap_options(&groups);
    let (children, nf, ne) = build_children(d, sources, options.max_gap())?;
    // The group keeps its name, lock, visibility, transparency, appearance, opacity mask, graphic
    // style link and the rest; its children are made again and its Gap Options set to the merged ones.
    let mut g = old;
    if let Some(ch) = g.children_mut() {
        *ch = children;
    }
    g.extras_mut().live_paint_gaps = stored_gap_options(options);
    for (pt, paint) in keep_faces {
        if let Some(fid) = face_at(&g, pt).map(|f| f.id)
            && let Some(ch) = g.children_mut()
            && let Some(f) = ch.iter_mut().find(|c| c.id == fid)
        {
            Arc::make_mut(f).appearance.set_fill(paint);
        }
    }
    for (pt, app) in keep_edges {
        if let Some(eid) = edge_near(&g, pt, 0.5).map(|e| e.id)
            && let Some(ch) = g.children_mut()
            && let Some(e) = ch.iter_mut().find(|c| c.id == eid)
        {
            Arc::make_mut(e).appearance = app.into();
        }
    }
    let mut merged = Beside::replacing(d, roots);
    for r in roots.iter().filter(|r| **r != target) {
        if d.node(*r).is_some() {
            merged.put(d, *r, vec![])?;
        }
    }
    merged.put(d, target, vec![g])?;
    merged.finish(d)?;
    sel.set([target]);
    Ok((target, nf, ne))
}

fn lp_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Merge Live Paint", |d, sel| merge_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

fn live_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = root_ids(s, p)?;
    let v: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(is_live_paint)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select a Live Paint group".into()));
    }
    Ok(v)
}

fn lp_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    let ids = s.edit("Release Live Paint", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &targets);
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let srcs: Vec<Node> = source_nodes(&n).iter().map(|c| (**c).clone()).collect();
            out.extend(srcs.iter().map(|c| c.id));
            released.put(d, *g, srcs)?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn lp_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    s.edit("Expand Live Paint", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            {
                let mut e = n.extras_mut();
                e.trace = None;
                e.live_paint_gaps = None;
            }
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| match c.name.as_deref() {
                    Some(SOURCES_NAME) => false,
                    Some(FACE_NAME) => !c.appearance.fill_paint().is_none(),
                    Some(EDGE_NAME) => !c.appearance.stroke_paint().is_none() && c.appearance.stroke_width() > 0.0,
                    _ => true,
                });
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

/// Gaps as `[x0, y0, x1, y1]` (the loose end, then where it closes to).
fn gaps_json(gaps: &[[Point; 2]]) -> Value {
    json!(gaps.iter().map(|[a, b]| [a.x, a.y, b.x, b.y]).collect::<Vec<_>>())
}

/// A group's Gap Options as `livePaint.info` gives them.
fn gap_options_json(o: GapOptions) -> Value {
    let [r, g, b] = o.color;
    let size = o.size.id().map_or_else(|| json!(o.size.points()), |id| json!(id));
    json!({ "detect": o.detect, "size": size, "points": o.size.points(), "color": format!("#{r:02x}{g:02x}{b:02x}") })
}

/// The gaps up to `options`' size wide among a Live Paint group's sources.
fn open_gaps(group: &Node, options: GapOptions) -> Vec<[Point; 2]> {
    po::find_gaps(&source_shapes(&source_nodes(group)).0, options.size.points())
}

/// The gaps View › Show Live Paint Gaps shows on `group`: those up to its Gap Options' size wide,
/// as `livePaint.info` lists them. None when `group` is not a Live Paint group.
pub fn live_paint_gaps(group: &Node) -> Vec<[Point; 2]> {
    if is_live_paint(group) { open_gaps(group, group.gap_options()) } else { vec![] }
}

fn lp_gap_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.gapOptions";
    let detect = match p.get("detect").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(C, "detect must be true or false"))?),
        None => None,
    };
    let size =
        match p.get("size").filter(|v| !v.is_null()) {
            Some(v) => Some(GapSize::parse(v).ok_or_else(|| {
                bad(C, format!("size must be \"small\", \"medium\", \"large\" or a number of points over 0 up to {}", GapSize::MAX))
            })?),
            None => None,
        };
    let color = match p.get("color").filter(|v| !v.is_null()) {
        Some(v) => Some(super::layer::layer_color_param(v).map_err(|e| bad(C, e))?.rgb()),
        None => None,
    };
    let close = switch_or(p, "closeGaps", false, C)?;
    let targets = live_targets(s, p)?;
    let (faces, gaps, closed) = s.edit("Gap Options", |d, sel| {
        let (mut faces, mut gaps, mut closed) = (0, vec![], 0);
        let mut changed = Beside::replacing(d, &targets);
        for g in &targets {
            let old = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut options = old.gap_options();
            options.detect = detect.unwrap_or(options.detect);
            options.size = size.unwrap_or(options.size);
            options.color = color.unwrap_or(options.color);
            let mut sources = source_nodes(&old);
            if close {
                // Close Gaps With Paths adds an unpainted line across each gap to the group's paths.
                for gap in po::find_gaps(&source_shapes(&sources).0, options.size.points()) {
                    let line = PathData::single(SubPath::polyline(&gap, false));
                    sources.push(Arc::new(Node::path(d.alloc_id(), line, Appearance::default())));
                    closed += 1;
                }
            }
            let mut n = old.clone();
            // The faces change only with what paint stops at, or with new paths.
            if close || options.max_gap() != old.gap_options().max_gap() {
                let (children, _, _) = build_children(d, sources, options.max_gap())?;
                if let Some(ch) = n.children_mut() {
                    *ch = children;
                }
                repaint_from(&mut n, &old);
            }
            n.extras_mut().live_paint_gaps = stored_gap_options(options);
            faces += b::faces(&n).len();
            gaps.extend(open_gaps(&n, options));
            changed.put(d, *g, vec![n])?;
        }
        changed.finish(d)?;
        sel.set(targets.iter().copied());
        Ok((faces, gaps, closed))
    })?;
    Ok(json!({ "ids": ids_json(&targets), "faces": faces, "gaps": gaps_json(&gaps), "closed": closed }))
}

fn lp_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.fill";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    super::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.fill.clone());
    let group = id_param(p, "group");
    let roots = if group.is_none() { root_ids(s, p)? } else { vec![] };
    let (g, face) = s.edit("Live Paint Bucket", |d, sel| {
        let gid = match group {
            Some(g) => {
                if !d.node(g).is_some_and(is_live_paint) {
                    return Err(bad(C, format!("{g} is not a Live Paint group")));
                }
                g
            }
            None => {
                let live_at = |face: fn(&Node, Point) -> Option<&Arc<Node>>| {
                    roots.iter().copied().find(|r| d.node(*r).is_some_and(|n| is_live_paint(n) && face(n, pt).is_some()))
                };
                match (live_at(b::visible_face_at), live_at(face_at)) {
                    (Some(g), _) => g,
                    // At a point on a hidden face of a Live Paint group among the objects, the command returns an error
                    // and does not merge the objects.
                    (None, Some(_)) => return Err(EngineError::Other("no Live Paint face at that point".into())),
                    (None, None) => make_live(d, sel, &roots)?.0,
                }
            }
        };
        let fid = d
            .node(gid)
            .and_then(|n| b::visible_face_at(n, pt))
            .map(|f| f.id)
            .ok_or_else(|| EngineError::Other("no Live Paint face at that point".into()))?;
        d.node_mut(fid).ok_or(EngineError::NoNode(fid))?.appearance.set_fill(paint.clone());
        Ok((gid, fid))
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "face": face.0 }))
}

fn lp_stroke_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.strokeEdge";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let g = id_param(p, "group").ok_or_else(|| bad(C, "missing group"))?;
    super::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.stroke.clone());
    let width = number_or(p, "width", s.paint.stroke_width.max(0.0), C)?;
    if !(0.0..=1000.0).contains(&width) {
        return Err(bad(C, "width out of range"));
    }
    let tol = number_or(p, "tolerance", 4.0, C)?.clamp(0.0, 1000.0);
    let edge = s.edit("Live Paint Bucket", |d, _| {
        let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad(C, format!("{g} is not a Live Paint group")))?;
        let eid = b::visible_edge_near(n, pt, tol).map(|e| e.id).ok_or_else(|| EngineError::Other("no Live Paint edge near that point".into()))?;
        let e = d.node_mut(eid).ok_or(EngineError::NoNode(eid))?;
        e.appearance.set_stroke(paint.clone());
        if let Some(st) = e.appearance.stroke_mut() {
            st.width = width;
        }
        Ok(eid)
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "edge": edge.0 }))
}

fn lp_info(s: &mut Session, p: &Value) -> Result<Value> {
    let g = id_param(p, "group").or_else(|| id_param(p, "id")).ok_or_else(|| bad("livePaint.info", "missing group"))?;
    let d = &s.doc()?.doc;
    let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad("livePaint.info", format!("{g} is not a Live Paint group")))?;
    let faces: Vec<Value> = b::faces(n)
        .iter()
        .map(|f| {
            let bb = f.geometric_bounds().unwrap_or(Rect::ZERO);
            json!({ "id": f.id.0, "fill": f.appearance.fill_paint().label(), "bounds": [bb.x0, bb.y0, bb.x1, bb.y1] })
        })
        .collect();
    let edges: Vec<Value> = b::edges(n)
        .iter()
        .map(|e| json!({ "id": e.id.0, "stroke": e.appearance.stroke_paint().label(), "width": e.appearance.stroke_width() }))
        .collect();
    let sources = b::sources(n).and_then(|g| g.children()).map(|c| c.len()).unwrap_or(0);
    let options = n.gap_options();
    Ok(json!({
        "faces": faces,
        "edges": edges,
        "sources": sources,
        "gapOptions": gap_options_json(options),
        "gaps": gaps_json(&open_gaps(n, options)),
    }))
}

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

/// The parameters command `c` traces with: the preset's, with `params` over them.
fn trace_params(c: &str, p: &Value) -> Result<tr::TraceParams> {
    let name = str_param(p, "preset").unwrap_or("Default");
    let enhanced = switch_or(p, "enhanced", false, c)?;
    // (A preset without an enhanced version is traced as it is.)
    let base = enhanced.then(|| tr::enhanced_preset(name)).flatten().or_else(|| tr::preset(name));
    let base = base.ok_or_else(|| bad(c, format!("unknown preset `{name}` (see imageTrace.presets)")))?;
    let Some(over) = p.get("params").filter(|v| !v.is_null()) else { return Ok(base) };
    let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
    let (Some(o), Some(src)) = (v.as_object_mut(), over.as_object()) else { return Err(bad(c, "params must be an object")) };
    // A key that isn't one of the settings is an error, with the nearest setting.
    if let Some(key) = src.keys().find(|k| !o.contains_key(k.as_str())) {
        return Err(super::params::unknown(c, key, &o.keys().map(String::as_str).collect::<Vec<_>>()));
    }
    for (k, val) in src {
        o.insert(k.clone(), val.clone());
    }
    let params: tr::TraceParams = serde_json::from_value(v).map_err(|e| bad(c, e.to_string()))?;
    if !params.fills && !params.strokes {
        return Err(bad(c, "Create needs fills, strokes or both"));
    }
    if !(params.stroke_width.is_finite() && params.stroke_width > 0.0) {
        return Err(bad(c, "strokeWidth must be a positive number of pixels"));
    }
    Ok(params)
}

/// Document Library in Color mode: look the library's colours up into `params.swatches` → each
/// one as traced (sRGB) with the colour to fill with. Errors name command `c`.
fn trace_library(c: &str, s: &Session, params: &mut tr::TraceParams) -> Result<Vec<([u8; 3], Color)>> {
    if params.mode != tr::Mode::Color || params.palette != tr::Palette::DocumentLibrary {
        return Ok(vec![]);
    }
    let key = params.library.as_str();
    let colors = super::swatchlib::library_colors(s, key)
        .ok_or_else(|| bad(c, format!("no swatch library `{key}` (see swatch.library.list; \"document\": the document's swatches)")))?;
    if colors.is_empty() {
        return Err(bad(c, format!("`{key}` has no colours")));
    }
    let library: Vec<([u8; 3], Color)> = colors
        .into_iter()
        .map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            ([r, g, b], c)
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

fn trace_make(s: &mut Session, p: &Value, expand: bool) -> Result<Value> {
    let c = if expand { "imageTrace.makeAndExpand" } else { "imageTrace.make" };
    let mut params = trace_params(c, p)?;
    let view = view_param(c, p)?;
    let params_json = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    // The preset's name if the parameters are exactly that preset's or its enhanced version's,
    // else "Custom".
    let named = str_param(p, "preset").unwrap_or("Default");
    let is = |b: Option<tr::TraceParams>| b.and_then(|b| serde_json::to_value(&b).ok()).as_ref() == Some(&params_json);
    let preset_label = if is(tr::preset(named)) || is(tr::enhanced_preset(named)) { named.to_string() } else { "Custom".to_string() };
    let roots = root_ids(s, p)?;
    // The image (inside a trace group, or selected directly).
    let (target, img) = {
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
    let blob = s.doc()?.doc.images.get(&img.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", img.key)))?;
    // An image over the tracer's size is traced scaled down (the paths are scaled back below).
    let raster = tr::Raster::decode_to_fit(&blob.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let library = trace_library(c, s, &mut params)?;
    let res = tr::trace_within(&raster, &params, MAX_TRACE_ANCHORS).map_err(|e| EngineError::Other(e.to_string()))?;
    let warnings = trace_warnings((img.width, img.height), (raster.width, raster.height), res.worked_at);
    // Pixel space of the decoded raster → the image object's pixel space → document.
    let sx = img.width.max(1) as f64 / raster.width.max(1) as f64;
    let sy = img.height.max(1) as f64 / raster.height.max(1) as f64;
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
            // A library colour fills with the swatch's own colour (its model kept).
            let color = library.iter().find(|(rgb, _)| *rgb == tp.color).map_or_else(|| Color::rgb8(tp.color[0], tp.color[1], tp.color[2]), |l| l.1);
            // Gradients: a path the tracer found to be a linear gradient is filled with it.
            let paint = tp.gradient.as_ref().map_or_else(|| Paint::solid(color), |g| traced_gradient(g, xf));
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

fn trace_set_view(s: &mut Session, p: &Value) -> Result<Value> {
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

fn trace_release(s: &mut Session, p: &Value) -> Result<Value> {
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

fn trace_expand(s: &mut Session, p: &Value) -> Result<Value> {
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
