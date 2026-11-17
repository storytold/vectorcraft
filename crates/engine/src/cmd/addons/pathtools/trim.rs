//! Trim at Crossings: remove the stretch of a path between the places where other paths, or the
//! path itself, cross it. The command takes the stretch at a point, every stretch a rectangle
//! touches, or every stretch a line through some points crosses. It works on every path that can
//! be edited, selected or not. What it removes is worked out by
//! [`vectorcraft_tools::addons::trim::plan`], which the tool also uses to show it beforehand.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::addons::trim::plan::{self, Limits, Pick, Stretches};

use super::super::super::draw2::replace_with_pieces;
use super::super::super::pathops::Beside;
use super::super::super::{CommandSpec, bad, cmd, finite_numbers, has_doc, number_or};
use crate::{Result, Session};

#[cfg(test)]
mod tests;

/// The undo step's name.
const LABEL: &str = "Trim at Crossings";
/// How near `at` a path counts as under it by default (pt), and the most.
const TOLERANCE: f64 = 3.0;
const MAX_TOLERANCE: f64 = 100.0;
/// Most points of `points`.
const MAX_POINTS: usize = 100_000;
/// The farthest coordinate the command takes.
const MAX_COORD: f64 = 1e7;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.trim",
        "Trim at Crossings",
        [],
        None,
        "{at: [x, y], tolerance?: pt 0..100 (3; how near `at` a path counts as under it)} or {rect: [x, y, width, height]} or {points: [[x, y]…] (2 to 100000 points)} remove stretches of paths, a stretch running between two neighboring places where the path is crossed by another path or by itself, or from such a place to an open end: `at` removes the stretch of the nearest path at that point, `rect` every stretch the rectangle touches, `points` every stretch the line through the points crosses. Give exactly one of the three. Every path that can be edited takes part, selected or not, compound path members included; hidden and locked paths, guides and clipping paths (compound ones' members included) are neither trimmed nor counted as crossing; in isolation mode only paths inside the isolated object are trimmed. A path that ends on another, or within 0.001 pt of it, marks a place on the other; paths that lie along each other don't cross. A closed path that loses a stretch becomes open, an open path that loses a middle stretch becomes two paths with its appearance in its place in the stacking order, and a path nothing crosses is removed whole; a live shape becomes a plain path. One undo step, none when nothing is removed → {ids (the paths left of those trimmed), removed (stretches)}",
        has_doc,
        trim
    )]
}

fn pick(p: &Value) -> Result<Pick> {
    const C: &str = "addon.pathtools.trim";
    let given = |key: &str| p.get(key).filter(|v| !v.is_null());
    let within = |v: &[f64]| v.iter().all(|x| x.abs() <= MAX_COORD);
    match (given("at"), given("rect"), given("points")) {
        (Some(at), None, None) => {
            let [x, y] = finite_numbers(at).filter(|v| within(v)).ok_or_else(|| bad(C, "`at` must be a point [x, y]"))?;
            let tolerance = number_or(p, "tolerance", TOLERANCE, C)?;
            if !(0.0..=MAX_TOLERANCE).contains(&tolerance) {
                return Err(bad(C, "`tolerance` must be from 0 to 100 pt"));
            }
            Ok(Pick::At(Point::new(x, y), tolerance))
        }
        (None, Some(rect), None) => {
            let [x, y, w, h] = finite_numbers(rect).filter(|v| within(v)).ok_or_else(|| bad(C, "`rect` must be [x, y, width, height]"))?;
            Ok(Pick::Rect(Rect::new(x, y, x + w, y + h).abs()))
        }
        (None, None, Some(points)) => {
            let list = points.as_array().ok_or_else(|| bad(C, "`points` must be a list of [x, y] points"))?;
            if !(2..=MAX_POINTS).contains(&list.len()) {
                return Err(bad(C, "`points` must hold from 2 to 100000 points"));
            }
            let pts = list
                .iter()
                .map(|v| {
                    finite_numbers(v)
                        .filter(|xy| within(xy))
                        .map(|[x, y]| Point::new(x, y))
                        .ok_or_else(|| bad(C, format!("bad point {v}: expected [x, y]")))
                })
                .collect::<Result<Vec<Point>>>()?;
            let bounds = pts.iter().fold(None, |r: Option<Rect>, q| Some(r.map_or(Rect::from_points(*q, *q), |r| r.union_pt(*q))));
            Ok(Pick::Line(pts, bounds.unwrap_or_default()))
        }
        _ => Err(bad(C, "give exactly one of `at`, `rect` and `points`")),
    }
}

fn trim(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.trim";
    let pick = pick(p)?;
    let st = s.doc()?;
    // In isolation mode only the isolated object's paths are trimmed; paths outside it still cross them.
    let root = crate::cmd::ordinary_isolation(st);
    let trimmed = plan::plan(&st.doc, root, &pick, Limits::COMMAND, &mut Stretches::default()).map_err(|e| bad(C, e.message()))?;
    let removed: usize = trimmed.iter().map(|t| t.count).sum();
    let plan: Vec<(NodeId, Vec<_>)> = trimmed.into_iter().map(|t| (t.id, t.pieces)).collect();
    if plan.is_empty() {
        return Ok(json!({ "ids": [], "removed": 0 }));
    }
    let ids = s.edit(LABEL, |d, sel| {
        let trimmed: Vec<NodeId> = plan.iter().map(|(id, _)| *id).collect();
        // The compound paths that may lose their last member.
        let mut compounds: Vec<NodeId> = trimmed
            .iter()
            .filter_map(|id| d.parent_of(*id))
            .filter(|c| matches!(d.node(*c).map(|n| &n.kind), Some(NodeKind::Compound { .. })))
            .collect();
        compounds.dedup();
        let mut beside = Beside::replacing(d, &trimmed);
        let mut all = vec![];
        // The selection is read through a set and changed once at the end, so a trim of many
        // paths does not go through the selected objects once per path and per piece.
        let mut selected_ids: HashSet<NodeId> = sel.objects.iter().copied().collect();
        let mut joining: Vec<NodeId> = vec![];
        let mut gone: Vec<NodeId> = vec![];
        for (id, pieces) in plan {
            let selected = selected_ids.contains(&id);
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            let made = replace_with_pieces(d, &mut beside, id, pieces)?;
            // The pieces of a selected path are selected; what was not selected stays so.
            if made.is_empty() {
                gone.push(id);
            } else if selected {
                joining.extend(made.iter().copied().filter(|n| selected_ids.insert(*n)));
            }
            all.extend(made);
        }
        beside.finish(d)?;
        for c in compounds {
            if d.node(c).is_some_and(|n| n.children().is_some_and(|ch| ch.is_empty())) {
                d.remove(c)?;
                gone.push(c);
            }
        }
        sel.remove_all(gone);
        if let Some((first, rest)) = joining.split_first() {
            // `add` clears what any change to the object selection clears; the rest are new ids.
            sel.add(*first);
            sel.objects.extend_from_slice(rest);
        }
        Ok(all)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "removed": removed }))
}
