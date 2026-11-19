//! Common Tangents (`addon.pathtools.commonTangents`): the straight lines that touch two selected
//! convex closed paths (ellipses at any rotation and scale, and convex Bézier paths) without
//! crossing into either, added as separate open paths, with the line joining their centers if
//! asked. One of the two may be a point (a path of one anchor) instead: the lines then run from it
//! to where they touch the path. The geometry is `vectorcraft_pathops::addons::pathtools::common_tangents`.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_geom::kurbo::Affine;
use vectorcraft_geom::{FillRule, PathData, Point, SubPath};
use vectorcraft_pathops::addons::pathtools as pt;

use super::super::super::object::choice;
use super::super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_selection, switch_or};
use crate::{EngineError, Result, Session};

/// The command's id.
pub const COMMON_TANGENTS: &str = "addon.pathtools.commonTangents";
/// The undo step's name.
const LABEL: &str = "Common Tangents";
/// The kinds of tangent `tangents` picks.
const KINDS: [&str; 3] = ["external", "internal", "all"];

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.commonTangents",
        "Common Tangents",
        ["Object", "Plug-ins", "Path Precision Tools"],
        None,
        "{tangents?: \"external\" (both paths on the same side of each line) | \"internal\" (the lines that run between them) | \"all\" (\"all\"), centerline?: bool (false: true also adds the line joining the two centers, an ellipse's center or a path's center of area), ids?: [id, id] (default: the selection)} add the common tangent lines of two convex closed paths as separate open paths, each from where it touches the first path to where it touches the second (the whole stretch where a straight side lies along it), stroked with the current stroke color and weight (black when the stroke is None) and unfilled, directly above the upper of the two, as one undo step; the new lines are selected. An ellipse drawn with the Ellipse tool, turned or scaled, is measured as an exact ellipse; any other path must be closed, have one outline, bend the same way all around and have at most 10000 segments. Separate paths have two external and two internal tangents; overlapping paths only external ones; one path inside the other has none. Paths that touch, from outside or from inside, share one tangent there instead: a line centered on the point where they touch and as long as the narrower path is wide along it; paths that meet at a corner or along a shared stretch of outline get one such line, at the middle of the directions that touch both. Paths count as touching when the gap between them is less than a billionth of the larger of their sizes and the distance between their centers. One of the two may be a point (a path of one anchor): the lines then run from the point to where they touch the path, two when the point lies outside it (one that lies along a straight side covers the whole side), one when the point lies on its outline, that is, less than the touching tolerance from it, with the point's distance from the path's center in place of the distance between centers (centered on the point and as long as the path is wide along it, or the whole straight side the point lies on), none when it lies inside; every `tangents` value gives the same lines, counted as internal when `tangents` is \"internal\" and as external otherwise, and the centerline runs from the point to the path's center; two points are refused. When there is no tangent of the kind asked for, nothing changes and the result has a `warning` → {ids, external, internal, centerline (its id or null), warning?}",
        has_selection,
        common_tangents
    )]
}

/// The outline of path `id` as the geometry takes it: a point for a path of one anchor, an exact
/// ellipse for a whole live ellipse, else the path's one closed convex subpath.
fn outline_of(d: &Document, id: NodeId) -> Result<pt::Outline> {
    const C: &str = COMMON_TANGENTS;
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    let NodeKind::Path { path, live, .. } = &n.kind else { return Err(bad(C, "select two closed paths")) };
    let [sp] = path.subpaths.as_slice() else { return Err(bad(C, "select two closed paths")) };
    if sp.anchors.len() == 1
        && let Some(only) = sp.anchors.first()
    {
        return pt::Outline::point(only.p).map_err(refusal);
    }
    if let Some(LiveShape::Ellipse { w, h, pie, xf }) = live.as_deref() {
        let span = (pie.1 - pie.0).rem_euclid(360.0);
        if span < 1e-9 || 360.0 - span < 1e-9 {
            let unit = *xf * Affine::new([w / 2.0, 0.0, 0.0, h / 2.0, w / 2.0, h / 2.0]);
            return pt::Outline::ellipse(unit).map_err(refusal);
        }
    }
    pt::Outline::convex(sp).map_err(refusal)
}

fn refusal(e: pt::OutlineError) -> EngineError {
    bad(
        COMMON_TANGENTS,
        match e {
            pt::OutlineError::Open => "a selected path is open",
            pt::OutlineError::NotConvex => "a selected path is not convex",
            pt::OutlineError::Empty => "a selected path has no area",
            pt::OutlineError::TooComplex => "a selected path has more than 10000 segments",
        },
    )
}

fn common_tangents(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = COMMON_TANGENTS;
    let kinds = choice(p, C, "tangents", &KINDS)?.unwrap_or("all");
    let centerline = switch_or(p, "centerline", false, C)?;
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let ordered = d.paint_order(roots.iter().copied());
    let [first, second] = ordered.as_slice() else { return Err(bad(C, "select two closed paths")) };
    let (a, b) = (outline_of(d, *first)?, outline_of(d, *second)?);
    if a.is_point() && b.is_point() {
        return Err(bad(C, "select two closed paths"));
    }
    // A point goes first, so that each line runs from it to where it touches the path.
    let point = a.is_point() || b.is_point();
    let (a, b) = if b.is_point() { (b, a) } else { (a, b) };
    let all = pt::common_tangents(&a, &b).map_err(|_| bad(C, "the two paths have the same outline"))?;
    let external = all.iter().filter(|t| t.kind == pt::TangentKind::External).count();
    // A point's external and internal tangents are the same lines: every kind keeps them.
    let keep = |t: &&pt::Tangent| {
        point
            || match kinds {
                "external" => t.kind == pt::TangentKind::External,
                "internal" => t.kind == pt::TangentKind::Internal,
                _ => true,
            }
    };
    let kept: Vec<&pt::Tangent> = all.iter().filter(keep).collect();
    let count = |kind: pt::TangentKind| kept.iter().filter(|t| t.kind == kind).count();
    let (external_kept, internal_kept) = match (point, kinds) {
        (true, "internal") => (0, kept.len()),
        (true, _) => (kept.len(), 0),
        _ => (count(pt::TangentKind::External), count(pt::TangentKind::Internal)),
    };
    let lines: Vec<(Point, Point)> = kept.iter().map(|t| (t.from, t.to)).collect();
    if lines.is_empty() && point {
        // Written as `EngineError::Other` so that the i18n message scan finds it.
        let warning = EngineError::Other("The point lies inside the path, so it has no tangents".into());
        return Ok(json!({ "ids": [], "external": 0, "internal": 0, "centerline": null, "warning": warning.to_string() }));
    }
    if lines.is_empty() {
        // Written as `EngineError::Other` so that the i18n message scan finds them. Exactly one
        // external tangent means one path lies inside the other and touches it there; paths that
        // overlap without one inside the other have at least two.
        let warning = match external {
            0 => EngineError::Other("One path lies inside the other, so they have no common tangents".into()),
            1 => EngineError::Other("One path lies inside the other, so they have no internal tangents".into()),
            _ => EngineError::Other("The paths overlap, so they have no internal tangents".into()),
        };
        return Ok(json!({ "ids": [], "external": 0, "internal": 0, "centerline": null, "warning": warning.to_string() }));
    }
    // Centers closer than a billionth of the larger path's size count as one point: no line.
    let center = (centerline && (a.center() - b.center()).hypot() > 1e-9 * a.size().max(b.size())).then(|| (a.center(), b.center()));
    let fill = Paint::None;
    let stroke = if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() };
    let mut look = s.new_art_look(fill, stroke, s.paint.stroke_width.max(0.1));
    // The new art's look may carry more fills than the top one: none of them is kept.
    look.appearance.items.retain(|i| !matches!(i, AppearanceItem::Fill(_)));
    let top = *second;
    let (ids, center_id) = s.edit(LABEL, |d, sel| {
        let (parent, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let mut ids = vec![];
        let mut center_id = None;
        for (k, (from, to)) in lines.iter().copied().chain(center).enumerate() {
            let id = d.alloc_id();
            let path = PathData::single(SubPath::polyline(&[from, to], false));
            let mut n = Node::new(id, NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false });
            look.apply(d, &mut n);
            d.insert(parent, idx + 1 + k, n)?;
            if k == lines.len() {
                center_id = Some(id);
            }
            ids.push(id);
        }
        sel.set(ids.iter().copied());
        Ok((ids, center_id))
    })?;
    Ok(json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        "external": external_kept,
        "internal": internal_kept,
        "centerline": center_id.map(|i| i.0),
    }))
}
