//! Trim at Crossings: remove the stretch of a path between the places where other paths, or the
//! path itself, cross it. The command takes the stretch at a point, every stretch a rectangle
//! touches, or every stretch a line through some points crosses. It works on every path that can
//! be edited, selected or not.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{Line, PathSeg};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, hit};
use vectorcraft_pathops::addons::pathtools as pt;

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
        "{at: [x, y], tolerance?: pt 0..100 (3; how near `at` a path counts as under it)} or {rect: [x, y, width, height]} or {points: [[x, y]…] (2 to 100000 points)} remove stretches of paths, a stretch running between two neighboring places where the path is crossed by another path or by itself, or from such a place to an open end: `at` removes the stretch of the nearest path at that point, `rect` every stretch the rectangle touches, `points` every stretch the line through the points crosses. Give exactly one of the three. Every path that can be edited takes part, selected or not, compound path members included; hidden and locked paths, guides and clipping paths (compound ones' members included) are neither trimmed nor counted as crossing; in isolation mode only paths inside the isolated object are trimmed. A path that ends on another marks a place on the other; paths that lie along each other don't cross. A closed path that loses a stretch becomes open, an open path that loses a middle stretch becomes two paths with its appearance in its place in the stacking order, and a path nothing crosses is removed whole; a live shape becomes a plain path. One undo step, none when nothing is removed → {ids (the paths left of those trimmed), removed (stretches)}",
        has_doc,
        trim
    )]
}

/// What picks the stretches to remove.
enum Pick {
    /// The stretch of the nearest path within the tolerance of the point.
    At(Point, f64),
    /// Every stretch the rectangle touches.
    Rect(Rect),
    /// Every stretch the line through the points crosses.
    Line(Vec<Point>, Rect),
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

/// A path that takes part: its id, geometry and control-point box.
type Part<'a> = (NodeId, &'a PathData, Rect);

/// The paths that take part, bottom to top: the ones that can be edited, without guides and
/// clipping paths. The clipping path of a clip group or a clipped layer is left out whole, the
/// members of a compound clipping path included.
fn parts(d: &Document) -> Vec<Part<'_>> {
    let editable = d.editable();
    let mut masks = HashSet::new();
    d.walk(|n| {
        if n.clips()
            && let Some(first) = n.children().and_then(|c| c.first())
        {
            first.walk(&mut |m| {
                masks.insert(m.id);
            });
        }
    });
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind
            && !masks.contains(&n.id)
            && let Some(b) = path.control_bounds()
            && editable(n.id)
        {
            out.push((n.id, path, b));
        }
    });
    out
}

/// Most box tests one command makes between a line's pieces and the paths' segments.
const MAX_LINE_TESTS: usize = 60_000_000;
/// Most tests one command makes between the boxes of the paths to trim and of the other paths.
const MAX_BOX_TESTS: usize = 50_000_000;

/// Does the line through `pts` cross `sp`? Each piece of the line compared costs one of `budget`;
/// `None` once it is spent.
fn crossed_by_line(sp: &SubPath, pts: &[Point], budget: &mut usize) -> Option<bool> {
    let segs: Vec<_> = (0..sp.segment_count())
        .map(|i| {
            let c = sp.segment(i);
            (c, Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3).inflate(1e-9, 1e-9))
        })
        .collect();
    let Some(reach) = segs.iter().map(|s| s.1).reduce(|a, b| a.union(b)) else { return Some(false) };
    for w in pts.windows(2) {
        let [a, b] = w else { continue };
        let piece = Rect::from_points(*a, *b);
        *budget = budget.checked_sub(1)?;
        if !reach.overlaps(piece) {
            continue;
        }
        for (c, r) in &segs {
            *budget = budget.checked_sub(1)?;
            if r.overlaps(piece) && !PathSeg::Cubic(*c).intersect_line(Line::new(*a, *b)).is_empty() {
                return Some(true);
            }
        }
    }
    Some(false)
}

/// Does the rectangle or line of `pick` touch the outline of `piece`? Never for `Pick::At`.
fn touches(piece: &SubPath, pick: &Pick, budget: &mut usize) -> Result<bool> {
    match pick {
        Pick::At(..) => Ok(false),
        Pick::Rect(r) => Ok(hit::intersects_rect(&PathData::single(piece.clone()), *r)),
        Pick::Line(pts, _) => crossed_by_line(piece, pts, budget)
            .ok_or_else(|| bad("addon.pathtools.trim", "the line is too long, or the paths it passes too many, to check")),
    }
}

/// `path` without the stretches `pick` takes (`on`: the place of `Pick::At` on it): its subpaths
/// then, and how many stretches went. `None` when none does. A rectangle or a line that touches no
/// part of `path` leaves it out before its crossings are looked for.
fn trimmed(
    path: &PathData,
    others: &[&PathData],
    pick: &Pick,
    on: Option<pt::Place>,
    budget: &mut usize,
    work: &mut pt::Work,
) -> Result<Option<(Vec<SubPath>, usize)>> {
    const C: &str = "addon.pathtools.trim";
    let reached = match pick {
        Pick::At(..) => true,
        Pick::Rect(r) => hit::intersects_rect(path, *r),
        Pick::Line(..) => {
            let mut any = false;
            for sp in &path.subpaths {
                if touches(sp, pick, budget)? {
                    any = true;
                    break;
                }
            }
            any
        }
    };
    if !reached {
        return Ok(None);
    }
    let places =
        pt::crossings_on_within(path, others, work).ok_or_else(|| bad(C, "the paths are too many or too tangled to find where they cross"))?;
    let mut subs = vec![];
    let mut removed = 0;
    for (si, sp) in path.subpaths.iter().enumerate() {
        // The places come in subpath order: this subpath's are one run of them.
        let run = places.partition_point(|p| p.0 < si)..places.partition_point(|p| p.0 <= si);
        let here: Vec<(usize, f64)> = places.get(run).unwrap_or_default().iter().map(|&(_, seg, t)| (seg, t)).collect();
        let spans = pt::spans(sp, &here);
        let marks: Vec<bool> = match pick {
            Pick::At(..) => {
                let at = on.filter(|(s, ..)| *s == si).and_then(|(_, seg, t)| pt::span_at(sp, &spans, seg, t));
                (0..spans.len()).map(|i| Some(i) == at).collect()
            }
            _ => spans.iter().map(|s| touches(&pt::slice(sp, *s), pick, budget)).collect::<Result<_>>()?,
        };
        match pt::without_spans(sp, &spans, &marks) {
            Some(pieces) => {
                removed += marks.iter().filter(|m| **m).count();
                subs.extend(pieces);
            }
            None => subs.push(sp.clone()),
        }
    }
    Ok((removed > 0).then_some((subs, removed)))
}

fn trim(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.trim";
    let pick = pick(p)?;
    let st = s.doc()?;
    let d = &st.doc;
    let parts = parts(d);
    // In isolation mode only the isolated object's paths are trimmed; paths outside it still cross them.
    let root = crate::cmd::ordinary_isolation(st);
    let in_scope = |id: NodeId| root.is_none_or(|r| id == r || crate::cmd::inside_isolation(d, r, id));
    // The paths to trim, each with the place `at` falls on.
    let targets: Vec<(usize, Option<pt::Place>)> = match &pick {
        Pick::At(at, tolerance) => {
            let mut best: Option<(usize, pt::Place, f64)> = None;
            for (i, (id, path, b)) in parts.iter().enumerate() {
                if !in_scope(*id) {
                    continue;
                }
                let b = b.inflate(*tolerance, *tolerance);
                if !(b.x0 <= at.x && at.x <= b.x1 && b.y0 <= at.y && at.y <= b.y1) {
                    continue;
                }
                // Of paths equally near, the one on top.
                if let Some((si, seg, t, _, dist)) = path.nearest(*at)
                    && dist <= *tolerance
                    && best.is_none_or(|b| dist <= b.2)
                {
                    best = Some((i, (si, seg, t), dist));
                }
            }
            best.map(|(i, on, _)| (i, Some(on))).into_iter().collect()
        }
        Pick::Rect(r) | Pick::Line(_, r) => {
            parts.iter().enumerate().filter(|(_, (id, _, b))| in_scope(*id) && b.overlaps(*r)).map(|(i, _)| (i, None)).collect()
        }
    };
    // Each path to trim has its box tested against every other path's.
    if targets.len().saturating_mul(parts.len()) > MAX_BOX_TESTS {
        return Err(bad(C, "the rectangle or line reaches too many paths to check"));
    }
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    let mut removed = 0;
    let mut budget = MAX_LINE_TESTS;
    // One budget of crossing work for every path the command trims.
    let mut work = pt::Work::default();
    for (i, on) in targets {
        let Some((id, path, b)) = parts.get(i) else { continue };
        let reach = b.inflate(1e-3, 1e-3);
        let others: Vec<&PathData> =
            parts.iter().enumerate().filter(|(j, (_, _, ob))| *j != i && reach.overlaps(*ob)).map(|(_, (_, o, _))| *o).collect();
        let Some((subs, n)) = trimmed(path, &others, &pick, on, &mut budget, &mut work)? else { continue };
        removed += n;
        // A path of one subpath becomes a path per piece; the pieces of one with more stay its subpaths.
        let pieces = match (path.subpaths.len(), subs.is_empty()) {
            (_, true) => vec![],
            (1, false) => subs.into_iter().map(PathData::single).collect(),
            _ => vec![PathData::new(subs)],
        };
        plan.push((*id, pieces));
    }
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
        for (id, pieces) in plan {
            let selected = sel.contains(id);
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            let made = replace_with_pieces(d, &mut beside, id, pieces)?;
            // The pieces of a selected path are selected; what was not selected stays so.
            if made.is_empty() {
                sel.remove(id);
            } else if selected {
                for n in &made {
                    if !sel.contains(*n) {
                        sel.add(*n);
                    }
                }
            }
            all.extend(made);
        }
        beside.finish(d)?;
        for c in compounds {
            if d.node(c).is_some_and(|n| n.children().is_some_and(|ch| ch.is_empty())) {
                d.remove(c)?;
                sel.remove(c);
            }
        }
        Ok(all)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "removed": removed }))
}
