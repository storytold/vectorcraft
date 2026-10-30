//! Path Precision Tools (`addon.pathtools.*`), a built-in add-on: the Anchor Eraser Brush's stroke,
//! the Extend Path tool's extension, the Measure & Inspect tool's readouts, the Anchor Inspector's
//! anchor edits and Split / Add Points at Crossings. Its tools, panel and submenu (Object ›
//! Plug-ins › Path Precision Tools) are the tools crate's and the UI's.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{AnchorRef, Document, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_pathops::addons::{self as po, pathtools as pt};

use super::super::draw2::{replace_with_pieces, usize_opt};
use super::super::pathops::Beside;
use super::super::{
    CommandSpec, bad, checked_id, checked_ids_or_selection, cmd, finite_numbers, has_doc, has_selection, number_or, switch_or, text_param,
};
use super::cleanup::PT_PER_TOLERANCE;
use crate::{EngineError, Result, Session};

/// The Anchor Eraser Brush's tolerance by default (the scale Simplify Anchors (Keep Shape) uses).
const ERASE_TOLERANCE: f64 = 10.0;
/// Most points one brush stroke carries.
const MAX_STROKE_POINTS: usize = 100_000;
/// Most anchors one command edits at once.
const MAX_ANCHORS: usize = 2_000_000;
/// The narrowest a brush gets at the lightest pen pressure (pt).
const MIN_RADIUS: f64 = 0.05;
/// The widest brush (pt).
const MAX_DIAMETER: f64 = 2500.0;
/// The farthest an end is extended at once (pt).
const MAX_EXTEND: f64 = 1e6;
/// Most anchors Add Points puts on each segment.
const MAX_ADD: u64 = pt::MAX_ADDED_PER_SEGMENT as u64;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pathtools.erase",
            "Anchor Eraser Brush",
            [],
            None,
            "{points: [[x, y] | [x, y, pressure 0..1]…] (the brush's path, at most 100000 points), diameter?: pt 0.1..2500 (20; a point's pressure scales it), tolerance?: 1..100 (10; the outline may move up to tolerance × 0.02 pt), ignoreTolerance?: bool (false: every anchor under the brush that can go goes, its neighbours refitted), plain?: bool (false: true deletes them without refitting the handles beside them), ids?: [id…] (default: the selection)} remove the anchors of the paths under the objects that the brush passes over and that can go while the outline stays within the tolerance, as one undo step; open ends stay → {ids, removed}",
            has_selection,
            erase
        ),
        cmd!(
            "addon.pathtools.extend",
            "Extend Path",
            [],
            None,
            "{id, subpath?: index (0), end: \"start\"|\"end\", length?: pt (negative shortens; never all of it), to?: [x, y] (instead of length: the length that brings the end nearest this point), mode?: \"straight\"|\"bezier\" (stretch the last segment's curve, no anchor added)|\"arc\" (along the circle the end curves on, at most half way round) (\"straight\")} lengthen an open path at an end the way it runs there, or shorten it back along itself, as one undo step → {id, length (pt)}",
            has_doc,
            extend
        ),
        cmd!(
            query "addon.pathtools.inspect",
            "Measure & Inspect",
            [],
            None,
            "{id, x, y, tolerance?: pt (3; how near an anchor counts as over it)} what the path reads at the point nearest (x, y): {subpath, closed, anchors, length, area (pt², an open subpath closed by a straight line), segment, t, point: [x, y], segmentLength, tangent (degrees counter-clockwise), radius (of curvature, null where straight), anchor?: {index, x, y, smooth, angle (at the anchor, null at an open end), in?: {length, angle}, out?: {length, angle}}}",
            has_doc,
            inspect
        ),
        cmd!(
            "addon.pathtools.setAnchor",
            "Set Anchor",
            [],
            None,
            "{x?, y?: pt (on the page), inLength?, outLength?: pt (0 retracts), inAngle?, outAngle?: degrees counter-clockwise} move the selected anchors (with their handles) and set their handles, as one undo step: direct-selected anchors, or every anchor of a path selected whole; a smooth anchor's other handle turns with the one set → {anchors}",
            has_selection,
            set_anchor
        ),
        cmd!(
            "addon.pathtools.smooth",
            "Make Smooth",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{ratio?: 0.1..0.5 (0.3905: a square's corners make a circle)} make the selected anchors smooth: a lone handle is mirrored; without handles, both stand at right angles to the bisector of the angle at the anchor, ratio × the nearer neighbour's distance long; two handles are put in line; an open end without handles aims at its neighbour's handle. One undo step → {anchors (changed)}",
            has_selection,
            smooth
        ),
        cmd!(
            "addon.pathtools.corner",
            "Make Corner",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{retract?: bool (true: the handles go; false: they stay where they are and move apart)} make the selected anchors corners, as one undo step → {anchors (changed)}",
            has_selection,
            corner
        ),
        cmd!(
            "addon.pathtools.equalize",
            "Equalize Handles",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{} give each selected anchor's two handles their average length (a missing one is made opposite the other), as one undo step → {anchors (changed)}",
            has_selection,
            equalize
        ),
        cmd!(
            "addon.pathtools.removeKeepShape",
            "Remove Points, Keep Shape",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{} remove the direct-selected anchors, refitting the handles beside each so the outline keeps its shape as best it can, as one undo step; open ends stay → {removed}",
            has_selection,
            remove_keep_shape
        ),
        cmd!(
            "addon.pathtools.addPoints",
            "Add Points to Segments…",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{count?: 1..100 (1)} add count anchors to each selected segment (a segment between two selected anchors, or every segment of a path selected whole), splitting it into pieces of equal length; the shape doesn't change. One undo step → {added}",
            has_selection,
            add_points
        ),
        cmd!(
            "addon.pathtools.close",
            "Close Path",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{mode?: \"handles\" (keep the ends' handles)|\"straight\" (a straight closing segment)|\"smooth\" (the ends made smooth), ratio?: 0.1..0.5 (0.3905, for smooth)} close the open subpaths of the selected paths, as one undo step → {closed}",
            has_selection,
            close
        ),
        cmd!(
            "addon.pathtools.crossings",
            "Add Points at Crossings",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{cut?: bool (false: add anchors where the paths cross; true: also cut them there), selfCrossings?: bool (false: where a path crosses itself too), ids?: [id…] (default: the selection)} where the paths under the objects cross one another, as one undo step; at most 10000 segments → {ids, crossings (anchors placed)}",
            has_selection,
            crossings
        ),
    ]
}

/// The editable paths (not guides) under `roots`, compound path members included.
fn paths_under(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let editable = d.editable();
    let mut out = vec![];
    for n in roots.iter().filter_map(|id| d.node(*id)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { guide: false, .. }) && editable(c.id) {
                out.push(c.id);
            }
        });
    }
    out
}

/// Put `np` in path `id`'s place; its live shape goes (its anchors are edited).
fn set_path(d: &mut Document, id: NodeId, np: PathData) {
    if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
        *path = np;
        *live = None;
    }
}

/// Error unless the paths together have at most [`MAX_ANCHORS`] anchors.
fn check_size<'a>(paths: impl IntoIterator<Item = &'a PathData>, cmd: &str) -> Result<()> {
    if paths.into_iter().map(PathData::anchor_count).sum::<usize>() > MAX_ANCHORS {
        return Err(bad(cmd, "the paths have too many anchors to edit at once"));
    }
    Ok(())
}

// ---------- Anchor Eraser Brush ----------

/// The brush's points (with the radius each has), from `points`.
fn stroke(p: &Value, radius: f64, cmd: &str) -> Result<Vec<(Point, f64)>> {
    let list = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`points` must be a list of [x, y] points"))?;
    if list.len() > MAX_STROKE_POINTS {
        return Err(bad(cmd, "too many `points` (at most 100000)"));
    }
    list.iter()
        .map(|v| {
            let point = |[x, y]: [f64; 2], pressure: f64| (Point::new(x, y), (radius * pressure.clamp(0.0, 1.0)).max(MIN_RADIUS));
            finite_numbers::<2>(v)
                .map(|xy| point(xy, 1.0))
                .or_else(|| finite_numbers::<3>(v).map(|[x, y, f]| point([x, y], f)))
                .ok_or_else(|| bad(cmd, format!("bad point {v}: expected [x, y] or [x, y, pressure]")))
        })
        .collect()
}

/// Does the brush, sweeping through `pts`, cover `q`?
fn under_brush(pts: &[(Point, f64)], q: Point) -> bool {
    let hit = |(a, ra): (Point, f64), (b, rb): (Point, f64)| {
        let ab = b - a;
        let len2 = ab.hypot2();
        let t = if len2 > 0.0 { ((q - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
        q.distance(a + ab * t) <= ra + (rb - ra) * t
    };
    match pts {
        [only] => hit(*only, *only),
        _ => pts.windows(2).any(|w| matches!(w, [a, b] if hit(*a, *b))),
    }
}

/// The anchors of `sp` that go when the brush passes over those marked, removed without refitting
/// the handles beside them: open ends stay, and the subpath keeps two anchors (three closed).
fn delete_marked(sp: &SubPath, marked: &[bool]) -> SubPath {
    let n = sp.anchors.len();
    let min = if sp.closed { 3 } else { 2 };
    let mut left = n.saturating_sub(min);
    let mut out = sp.clone();
    let keep: Vec<bool> = (0..n)
        .map(|i| {
            let end = !sp.closed && (i == 0 || i + 1 == n);
            let go = !end && left > 0 && marked.get(i).copied().unwrap_or(false);
            left -= usize::from(go);
            !go
        })
        .collect();
    let mut it = keep.iter();
    out.anchors.retain(|_| it.next().copied().unwrap_or(true));
    out
}

fn erase(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.erase";
    let diameter = number_or(p, "diameter", 20.0, C)?;
    if !(0.1..=MAX_DIAMETER).contains(&diameter) {
        return Err(bad(C, "`diameter` must be from 0.1 to 2500 pt"));
    }
    let tolerance = number_or(p, "tolerance", ERASE_TOLERANCE, C)?;
    if !(1.0..=100.0).contains(&tolerance) {
        return Err(bad(C, "`tolerance` must be from 1 to 100"));
    }
    let (ignore, plain) = (switch_or(p, "ignoreTolerance", false, C)?, switch_or(p, "plain", false, C)?);
    let pts = stroke(p, diameter / 2.0, C)?;
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let ids = paths_under(d, &roots);
    let paths: Vec<(NodeId, &PathData)> = ids.iter().filter_map(|id| Some((*id, d.node(*id)?.path_data()?))).collect();
    check_size(paths.iter().map(|(_, p)| *p), C)?;
    let tol = if ignore { f64::INFINITY } else { tolerance * PT_PER_TOLERANCE };
    let mut removed = 0;
    let mut changed = vec![];
    for (id, path) in paths {
        let subs: Vec<SubPath> = path
            .subpaths
            .iter()
            .map(|sp| {
                let marked: Vec<bool> = sp.anchors.iter().map(|a| !pts.is_empty() && under_brush(&pts, a.p)).collect();
                match marked.contains(&true) {
                    false => sp.clone(),
                    true if plain => delete_marked(sp, &marked),
                    true => po::simplify_anchors_keep_shape(sp, tol, &marked),
                }
            })
            .collect();
        let np = PathData::new(subs);
        if np != *path {
            removed += path.anchor_count().saturating_sub(np.anchor_count());
            changed.push((id, np));
        }
    }
    edit_whole(s, "Anchor Eraser Brush", changed)?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "removed": removed }))
}

/// Replace paths with the new data given, as one step (none when there is nothing to change); each
/// is selected whole, as its anchors' indexes changed.
fn edit_whole(s: &mut Session, label: &str, changed: Vec<(NodeId, PathData)>) -> Result<()> {
    if changed.is_empty() {
        return Ok(());
    }
    s.edit(label, |d, sel| {
        for (id, np) in changed {
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            set_path(d, id, np);
        }
        Ok(())
    })
}

// ---------- Extend Path ----------

/// The open subpath an `extend` request names: (path id, subpath index, its data).
fn open_subpath(s: &Session, p: &Value, cmd: &str) -> Result<(NodeId, usize, SubPath)> {
    let id = checked_id(s, p.get("id").unwrap_or(&Value::Null), cmd)?;
    let si = usize_opt(p, "subpath", cmd)?.unwrap_or(0);
    let d = &s.doc()?.doc;
    if !d.is_editable(id) {
        return Err(bad(cmd, "the path is locked or hidden"));
    }
    let path = d.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(cmd, "`id` is not a path"))?;
    let sp = path.subpaths.get(si).ok_or_else(|| bad(cmd, "no such subpath"))?;
    if sp.closed || sp.anchors.len() < 2 {
        return Err(bad(cmd, "the subpath has no ends to extend (it is closed or a single point)"));
    }
    Ok((id, si, sp.clone()))
}

fn extend(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.extend";
    let (id, si, sp) = open_subpath(s, p, C)?;
    let at_start = match text_param(p, "end", C)? {
        Some("start") => true,
        Some("end") => false,
        _ => return Err(bad(C, "`end` must be \"start\" or \"end\"")),
    };
    let mode = match text_param(p, "mode", C)? {
        None => pt::ExtendMode::Straight,
        Some(m) => pt::ExtendMode::parse(m).ok_or_else(|| bad(C, "`mode` must be \"straight\", \"bezier\" or \"arc\""))?,
    };
    let length = match (p.get("to").filter(|v| !v.is_null()), p.get("length").filter(|v| !v.is_null())) {
        (Some(to), None) => {
            let [x, y] = finite_numbers::<2>(to).ok_or_else(|| bad(C, "`to` must be [x, y]"))?;
            pt::extend_length_to(&sp, at_start, Point::new(x, y), mode).ok_or_else(|| bad(C, "the path can't be extended"))?
        }
        (None, Some(_)) => number_or(p, "length", 0.0, C)?,
        _ => return Err(bad(C, "give `length` or `to`")),
    };
    if !length.is_finite() || length.abs() > MAX_EXTEND {
        return Err(bad(C, "`length` must be a number of at most 1000000 pt either way"));
    }
    let new = pt::extend_open(&sp, at_start, length, mode).ok_or_else(|| bad(C, "the path is too short to shorten that much"))?;
    if new != sp {
        s.edit("Extend Path", |d, sel| {
            let n = d.node(id).and_then(|n| n.path_data()).cloned().ok_or(EngineError::NoNode(id))?;
            let mut np = n;
            if let Some(slot) = np.subpaths.get_mut(si) {
                *slot = new;
            }
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            set_path(d, id, np);
            Ok(())
        })?;
    }
    Ok(json!({ "id": id.0, "length": length }))
}

// ---------- Measure & Inspect ----------

fn polar(h: Option<(f64, f64)>) -> Value {
    h.map_or(Value::Null, |(length, angle)| json!({ "length": length, "angle": angle }))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.inspect";
    let id = checked_id(s, p.get("id").unwrap_or(&Value::Null), C)?;
    let (x, y) = (number_or(p, "x", f64::NAN, C)?, number_or(p, "y", f64::NAN, C)?);
    let tolerance = number_or(p, "tolerance", 3.0, C)?;
    if !(x.is_finite() && y.is_finite()) || !(tolerance >= 0.0 && tolerance.is_finite()) {
        return Err(bad(C, "give `x` and `y` (and a `tolerance` of 0 or more)"));
    }
    let path = s.doc()?.doc.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(C, "`id` is not a path"))?;
    let i = pt::inspect(path, Point::new(x, y), tolerance).ok_or_else(|| bad(C, "the path has no segments to measure"))?;
    let anchor = i.anchor.map(|a| {
        json!({ "index": a.index, "x": a.p.x, "y": a.p.y, "smooth": a.smooth, "angle": a.angle, "in": polar(a.handle_in), "out": polar(a.handle_out) })
    });
    Ok(json!({
        "subpath": i.subpath, "closed": i.closed, "anchors": i.anchors, "length": i.length, "area": i.area,
        "segment": i.segment, "t": i.t, "point": [i.point.x, i.point.y], "segmentLength": i.segment_length,
        "tangent": i.tangent, "radius": i.radius, "anchor": anchor,
    }))
}

// ---------- Anchor Inspector ----------

/// The anchors the Anchor Inspector acts on, per path: each path's direct-selected anchors, or
/// every anchor of a path selected whole (with `whole`), keyed by subpath.
type Picked = Vec<(NodeId, BTreeMap<usize, BTreeSet<usize>>)>;

/// The selected anchors ([`Picked`]); `whole` counts the anchors of paths selected whole.
fn picked_anchors(s: &Session, whole: bool) -> Result<Picked> {
    let st = s.doc()?;
    let (d, sel) = (&st.doc, &st.selection);
    let roots = super::super::edit::selected_roots(s)?;
    let mut out = vec![];
    for id in paths_under(d, &roots) {
        let Some(path) = d.node(id).and_then(|n| n.path_data()) else { continue };
        let refs: Vec<AnchorRef> = match sel.anchors.get(&id).filter(|a| !a.is_empty()) {
            Some(set) => set.iter().copied().collect(),
            None if whole => path.anchors().map(|(si, ai, _)| (si, ai)).collect(),
            None => continue,
        };
        let mut by_sub: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (si, ai) in refs.into_iter().filter(|(si, ai)| path.subpaths.get(*si).is_some_and(|sp| *ai < sp.anchors.len())) {
            by_sub.entry(si).or_default().insert(ai);
        }
        if !by_sub.is_empty() {
            out.push((id, by_sub));
        }
    }
    Ok(out)
}

/// Edit each selected anchor ([`picked_anchors`]) in place with `f` (true when it changed one), as
/// one step labelled `label`; the selection stays. → how many anchors changed.
fn edit_anchors(s: &mut Session, label: &str, cmd: &str, f: impl Fn(&mut SubPath, usize) -> bool) -> Result<usize> {
    let picked = picked_anchors(s, true)?;
    if picked.is_empty() {
        return Err(bad(cmd, "select paths or anchor points"));
    }
    let d = &s.doc()?.doc;
    check_size(picked.iter().filter_map(|(id, _)| d.node(*id)?.path_data()), cmd)?;
    let mut count = 0;
    let mut changed = vec![];
    for (id, by_sub) in &picked {
        let Some(mut np) = d.node(*id).and_then(|n| n.path_data()).cloned() else { continue };
        let before = count;
        for (si, anchors) in by_sub {
            let Some(sp) = np.subpaths.get_mut(*si) else { continue };
            count += anchors.iter().filter(|&&i| f(sp, i)).count();
        }
        if count > before {
            changed.push((*id, np));
        }
    }
    if !changed.is_empty() {
        s.edit(label, |d, _| {
            for (id, np) in changed {
                set_path(d, id, np);
            }
            Ok(())
        })?;
    }
    Ok(count)
}

/// A smoothing `ratio` param (default [`pt::SMOOTH_RATIO`]).
pub(super) fn ratio(p: &Value, cmd: &str) -> Result<f64> {
    let r = number_or(p, "ratio", pt::SMOOTH_RATIO, cmd)?;
    let (lo, hi) = pt::SMOOTH_RATIO_RANGE;
    if !(lo..=hi).contains(&r) {
        return Err(bad(cmd, "`ratio` must be from 0.1 to 0.5"));
    }
    Ok(r)
}

fn smooth(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.smooth";
    let r = ratio(p, C)?;
    let n = edit_anchors(s, "Make Smooth", C, |sp, i| pt::auto_smooth(sp, i, r))?;
    Ok(json!({ "anchors": n }))
}

fn corner(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.corner";
    let retract = switch_or(p, "retract", true, C)?;
    let n = edit_anchors(s, "Make Corner", C, |sp, i| pt::make_corner(sp, i, retract))?;
    Ok(json!({ "anchors": n }))
}

fn equalize(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.equalize";
    let n = edit_anchors(s, "Equalize Handles", C, pt::equalize_handles)?;
    Ok(json!({ "anchors": n }))
}

/// A number param that is given, finite, and (with `range`) inside it.
fn given(p: &Value, key: &str, range: Option<(f64, f64)>, cmd: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
            match range {
                Some((lo, hi)) if !(lo..=hi).contains(&x) => Err(bad(cmd, format!("`{key}` must be from {lo} to {hi}"))),
                _ => Ok(Some(x)),
            }
        }
    }
}

fn set_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.setAnchor";
    const LIMIT: Option<(f64, f64)> = Some((-1e7, 1e7));
    let (x, y) = (given(p, "x", LIMIT, C)?, given(p, "y", LIMIT, C)?);
    let lengths = [given(p, "inLength", Some((0.0, 1e7)), C)?, given(p, "outLength", Some((0.0, 1e7)), C)?];
    let angles = [given(p, "inAngle", LIMIT, C)?, given(p, "outAngle", LIMIT, C)?];
    if [x, y].iter().chain(&lengths).chain(&angles).all(Option::is_none) {
        return Err(bad(C, "give x, y, or a handle's length or angle"));
    }
    let n = edit_anchors(s, "Set Anchor", C, |sp, i| {
        let Some(a) = sp.anchors.get_mut(i) else { return false };
        let before = *a;
        let to = Point::new(x.unwrap_or(a.p.x), y.unwrap_or(a.p.y));
        a.translate(to - a.p);
        for (out, (len, ang)) in [false, true].into_iter().zip(lengths.into_iter().zip(angles)) {
            if len.is_none() && ang.is_none() {
                continue;
            }
            let h = if out { a.h_out } else { a.h_in };
            let (l0, a0) = pt::handle_polar(a.p, h).unwrap_or((0.0, if out { 0.0 } else { 180.0 }));
            let (l, deg) = (len.unwrap_or(l0), ang.unwrap_or(a0));
            // A length alone keeps a smooth anchor's other handle where it is; an angle turns it too.
            let other = if out { a.h_in } else { a.h_out };
            a.set_handle(out, pt::handle_at(a.p, l, deg), false);
            if ang.is_none() {
                if out { a.h_in = other } else { a.h_out = other }
            }
        }
        *a != before
    })?;
    Ok(json!({ "anchors": n }))
}

fn remove_keep_shape(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.removeKeepShape";
    let picked = picked_anchors(s, false)?;
    if picked.is_empty() {
        return Err(bad(C, "direct-select the anchor points to remove"));
    }
    let d = &s.doc()?.doc;
    check_size(picked.iter().filter_map(|(id, _)| d.node(*id)?.path_data()), C)?;
    let mut removed = 0;
    let mut changed = vec![];
    for (id, by_sub) in &picked {
        let Some(mut np) = d.node(*id).and_then(|n| n.path_data()).cloned() else { continue };
        for (si, anchors) in by_sub {
            let Some(sp) = np.subpaths.get_mut(*si) else { continue };
            // From the last back, so the indexes left stay right.
            for &i in anchors.iter().rev() {
                if let Some((out, _)) = po::remove_anchor_keep_shape(sp, i) {
                    *sp = out;
                    removed += 1;
                }
            }
        }
        if np != *d.node(*id).and_then(|n| n.path_data()).ok_or(EngineError::NoNode(*id))? {
            changed.push((*id, np));
        }
    }
    edit_whole(s, "Remove Points, Keep Shape", changed)?;
    Ok(json!({ "removed": removed }))
}

fn add_points(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.addPoints";
    let count = number_or(p, "count", 1.0, C)?;
    if !(1.0..=MAX_ADD as f64).contains(&count) || count.fract() != 0.0 {
        return Err(bad(C, "`count` must be a whole number from 1 to 100"));
    }
    let picked = picked_anchors(s, true)?;
    let st = s.doc()?;
    let d = &st.doc;
    check_size(picked.iter().filter_map(|(id, _)| d.node(*id)?.path_data()), C)?;
    let mut added = 0;
    let mut changed = vec![];
    for (id, by_sub) in &picked {
        let Some(path) = d.node(*id).and_then(|n| n.path_data()) else { continue };
        let picked_segments = st.selection.segments.get(id);
        let mut np = path.clone();
        for (si, sp) in np.subpaths.iter_mut().enumerate() {
            let Some(anchors) = by_sub.get(&si) else { continue };
            let n = sp.anchors.len();
            // Picked segments, else those between two selected anchors.
            let segs: BTreeSet<usize> = match picked_segments {
                Some(set) => set.iter().filter(|(s, _)| *s == si).map(|(_, k)| *k).collect(),
                None => (0..sp.segment_count()).filter(|k| anchors.contains(k) && anchors.contains(&((k + 1) % n.max(1)))).collect(),
            };
            let (out, marks) = pt::add_points(sp, Some(&segs), count as usize);
            added += marks.len();
            *sp = out;
        }
        if np != *path {
            changed.push((*id, np));
        }
    }
    if added == 0 {
        return Err(bad(C, "select segments, or paths, to add points to"));
    }
    edit_whole(s, "Add Points to Segments", changed)?;
    Ok(json!({ "added": added }))
}

pub(super) fn close(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.close";
    let mode = match text_param(p, "mode", C)? {
        None => pt::CloseMode::Handles,
        Some(m) => pt::CloseMode::parse(m).ok_or_else(|| bad(C, "`mode` must be \"handles\", \"straight\" or \"smooth\""))?,
    };
    let r = ratio(p, C)?;
    let roots = super::super::edit::selected_roots(s)?;
    let d = &s.doc()?.doc;
    let mut closed = 0;
    let mut changed = vec![];
    for id in paths_under(d, &roots) {
        let Some(path) = d.node(id).and_then(|n| n.path_data()) else { continue };
        let mut np = path.clone();
        let n = np.subpaths.iter_mut().map(|sp| pt::close(sp, mode, r)).filter(|c| *c).count();
        if n > 0 {
            closed += n;
            changed.push((id, np));
        }
    }
    if changed.is_empty() {
        return Err(bad(C, "select open paths to close"));
    }
    s.edit("Close Path", |d, _| {
        for (id, np) in changed {
            set_path(d, id, np);
        }
        Ok(())
    })?;
    Ok(json!({ "closed": closed }))
}

// ---------- Split / Add Points at Crossings ----------

fn crossings(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.crossings";
    let (cut, self_crossings) = (switch_or(p, "cut", false, C)?, switch_or(p, "selfCrossings", false, C)?);
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let ids = paths_under(d, &roots);
    let paths: Vec<&PathData> = ids.iter().filter_map(|id| d.node(*id)?.path_data()).collect();
    if paths.len() != ids.len() {
        return Err(EngineError::Other("Add Points at Crossings: a path went missing".into()));
    }
    let places = pt::crossings(&paths, self_crossings)
        .ok_or_else(|| bad(C, "the paths are too many or too tangled to find where they cross (at most 10000 segments)"))?;
    let mut count = 0;
    // Each path that is crossed: its new subpaths, and the anchors at the crossings on each.
    let mut changed: Vec<(NodeId, Vec<SubPath>, Vec<BTreeSet<usize>>)> = vec![];
    for ((id, path), on) in ids.iter().zip(&paths).zip(places) {
        if on.is_empty() {
            continue;
        }
        let mut subs = vec![];
        let mut marks = vec![];
        for (si, sp) in path.subpaths.iter().enumerate() {
            let here: Vec<(usize, f64)> = on.iter().filter(|(s, ..)| *s == si).map(|&(_, seg, t)| (seg, t)).collect();
            let (out, m) = pt::insert_at(sp, &here);
            count += m.len();
            subs.push(out);
            marks.push(m);
        }
        changed.push((*id, subs, marks));
    }
    if changed.is_empty() {
        return Ok(json!({ "ids": [], "crossings": 0 }));
    }
    let label = if cut { "Split at Crossings" } else { "Add Points at Crossings" };
    let out = s.edit(label, |d, sel| {
        let mut all = vec![];
        let mut beside = Beside::replacing(d, &changed.iter().map(|(id, ..)| *id).collect::<Vec<_>>());
        for (id, subs, marks) in changed {
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            if !cut {
                set_path(d, id, PathData::new(subs));
                all.push(id);
                continue;
            }
            let pieces: Vec<SubPath> = subs.iter().zip(&marks).flat_map(|(sp, m)| sp.cut_at(m)).collect();
            // A path of one subpath becomes a path per piece; a compound shape's pieces stay its subpaths.
            if subs.len() == 1 {
                let made = replace_with_pieces(d, &mut beside, id, pieces.into_iter().map(PathData::single).collect())?;
                for n in &made {
                    if !sel.contains(*n) {
                        sel.add(*n);
                    }
                }
                all.extend(made);
            } else {
                set_path(d, id, PathData::new(pieces));
                all.push(id);
            }
        }
        beside.finish(d)?;
        Ok(all)
    })?;
    Ok(json!({ "ids": out.iter().map(|i| i.0).collect::<Vec<_>>(), "crossings": count }))
}
