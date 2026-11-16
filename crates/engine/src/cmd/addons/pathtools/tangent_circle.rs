//! Tangent Circle: draw the circles that touch two paths or compound paths at a given radius, or
//! that touch three of them. The geometry is the pathops crate's
//! ([`tangent_circles`](vectorcraft_pathops::addons::pathtools::tangent_circles)).

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData};
use vectorcraft_pathops::addons::pathtools::{self as pt, TangentCircleError};

use super::super::super::{CommandSpec, bad, checked_ids_or_selection, checked_ids_param, cmd, has_selection, index_opt, number_or};
use crate::{Result, Session};

#[cfg(test)]
mod tests;

/// The command.
pub(crate) const COMMAND: &str = "addon.pathtools.tangentCircle";
/// The undo step's name.
const LABEL: &str = "Tangent Circle";
/// The radius the command takes with two shapes (pt).
const RADIUS: std::ops::RangeInclusive<f64> = 0.01..=1e6;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.tangentCircle",
        "Tangent Circle",
        ["Object", "Plug-ins", "Path Precision Tools"],
        None,
        "{ids?: [id…] (two or three paths or compound paths; default: the selection), radius?: pt 0.01..1000000 (needed with two shapes; three shapes set the radius themselves and take none), circle?: 0..64 (0: every circle found; n: only the nth, the circles listed by radius, then top to bottom, then left to right)} draw the circles that touch two shapes and have the radius, or that touch three shapes. A circle touches a shape when it meets the shape's outline without crossing it, the whole outline lying outside the circle or inside it: it is tangent to the outline at a smooth point and passes through a corner or an open end; the members of a compound path count as one outline. With three shapes, circles up to 64 times the diagonal of the box around the shapes' anchors and handles are found. Each circle is a new ellipse with the current paint on top of the current layer, or of the isolated group (inside an isolated compound path they join it); the circles are selected; one undo step. Locked and hidden shapes given by `ids` are read and left as they are. No circle is an error, and so are more than 64 (shapes touched along a whole curve, such as two concentric circles), an object named twice, shapes at one point or too small for the radius, and more than 2000 segments in all; the document is then unchanged → {ids, found (how many circles touch the shapes)}",
        has_selection,
        tangent_circle
    )]
}

/// The paths of path or compound path `id`: itself, or its members.
fn paths_of(d: &Document, id: NodeId) -> Result<Vec<&PathData>> {
    let value = id.0;
    let n = d.node(id).ok_or_else(|| bad(COMMAND, format!("no such object id {value}")))?;
    match &n.kind {
        NodeKind::Path { path, .. } => Ok(vec![path]),
        NodeKind::Compound { children, .. } => Ok(children.iter().filter_map(|c| c.path_data()).collect()),
        _ => Err(bad(COMMAND, format!("object {value} is not a path or compound path"))),
    }
}

/// The outlines of `ids`, each path's or compound path's subpaths together; counted before they
/// are copied.
fn outlines(d: &Document, ids: &[NodeId]) -> Result<Vec<PathData>> {
    let parts = ids.iter().map(|id| paths_of(d, *id)).collect::<Result<Vec<_>>>()?;
    let segments: usize = parts.iter().flatten().map(|p| pt::tangent_circle_segments(p)).sum();
    if segments > pt::MAX_TANGENT_CIRCLE_SEGMENTS {
        return Err(bad(COMMAND, TOO_COMPLEX));
    }
    Ok(parts.iter().map(|paths| PathData::new(paths.iter().flat_map(|p| p.subpaths.iter().cloned()).collect())).collect())
}

const TOO_COMPLEX: &str = "the shapes are too complex: at most 2000 segments in all";

fn tangent_circle(s: &mut Session, p: &Value) -> Result<Value> {
    // A repeated id is refused before the ids inside another given one are dropped.
    if p.get("ids").is_some_and(|v| !v.is_null()) {
        let mut seen = HashSet::new();
        if checked_ids_param(s, p, "ids", COMMAND)?.iter().any(|id| !seen.insert(*id)) {
            return Err(bad(COMMAND, "`ids` names an object twice"));
        }
    }
    let ids = checked_ids_or_selection(s, p, COMMAND)?;
    let d = &s.doc()?.doc;
    if !(2..=3).contains(&ids.len()) {
        return Err(bad(COMMAND, "select two or three paths or compound paths"));
    }
    let outlines = outlines(d, &ids)?;
    if outlines.iter().any(|o| o.subpaths.iter().all(|sp| sp.anchors.is_empty())) {
        return Err(bad(COMMAND, "a shape has no outline"));
    }
    let given_radius = p.get("radius").is_some_and(|v| !v.is_null());
    let radius = number_or(p, "radius", 0.0, COMMAND)?;
    let radius = match (ids.len(), given_radius) {
        (2, true) if RADIUS.contains(&radius) => Some(radius),
        (2, true) => return Err(bad(COMMAND, "`radius` must be from 0.01 to 1000000 pt")),
        (2, false) => return Err(bad(COMMAND, "two shapes need a `radius`")),
        (_, true) => return Err(bad(COMMAND, "three shapes set the radius themselves: leave `radius` out")),
        (_, false) => None,
    };
    let pick = index_opt(p, "circle", COMMAND)?.unwrap_or(0);
    if pick > pt::MAX_TANGENT_CIRCLES {
        return Err(bad(COMMAND, "`circle` must be from 0 to 64"));
    }
    let refs: Vec<&PathData> = outlines.iter().collect();
    let found = pt::tangent_circles(&refs, radius).map_err(|e| match e {
        // Every outline has points (checked above): all at one place, or too small for the radius.
        TangentCircleError::BadInput => bad(COMMAND, "these shapes are too small to measure"),
        // The segments were counted above: the search itself went past its limits.
        TangentCircleError::TooComplex => bad(COMMAND, "these shapes take too long to search"),
        TangentCircleError::TooMany => bad(COMMAND, "more than 64 circles touch these shapes: they may touch along a whole curve"),
    })?;
    if found.is_empty() {
        return Err(bad(COMMAND, "no circle touches these shapes"));
    }
    let chosen: Vec<pt::Circle> = match pick {
        0 => found.clone(),
        n => match found.get(n - 1) {
            Some(c) => vec![*c],
            None => {
                let found = found.len();
                return Err(bad(COMMAND, format!("`circle` is past the last circle found ({found})")));
            }
        },
    };
    // Path art: inside an isolated compound path, the circles join it and take its appearance.
    let parent = s.doc()?.target_parent_for_path()?;
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let ids = s.edit(LABEL, |d, sel| {
        let mut ids = vec![];
        for c in &chosen {
            let (r, at) = (c.radius, c.center);
            let live = LiveShape::Ellipse { w: 2.0 * r, h: 2.0 * r, pie: (0.0, 360.0), xf: Affine::translate((at.x - r, at.y - r)) };
            let kind = NodeKind::Path { path: live.to_path(), rule: FillRule::NonZero, live: Some(Box::new(live)), clipping: false, guide: false };
            let mut n = Node::new(d.alloc_id(), kind);
            look.apply(d, &mut n);
            ids.push(n.id);
            d.insert(parent, usize::MAX, n)?;
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "found": found.len() }))
}
