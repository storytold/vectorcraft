//! Enclosing Lasso selects whole objects by a freehand loop. Path geometry is measured without
//! stroke width or effects, as with the Selection tool's enclosed marquee. Other objects use
//! their geometric box. Ordinary groups are traversed; compound paths stay one object.

mod region;

use serde_json::json;
use vectorcraft_doc::hit::enclosure_candidates;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::kurbo::{ParamCurve, PathEl};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape};
use vectorcraft_pathops::{BoolOp, DEFAULT_PRECISION, try_boolean};

use self::region::Region;
use crate::xform::{BLUE, polygon};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

pub const ID: &str = "superLasso";

#[derive(Default)]
pub struct EnclosingLassoTool {
    points: Vec<Point>,
    active: bool,
}

/// Splitting at every crossing catches curves that leave a concave loop between their anchors.
fn enclosed(path: &BezPath, region: &Region) -> bool {
    let mut any = false;
    // A subpath can be a single anchor with no segments. Check every start, including stray
    // points beside other subpaths, so a whole object's disconnected points cannot be missed.
    for el in path.elements() {
        if let PathEl::MoveTo(p) = el {
            any = true;
            if !region.contains(*p) {
                return false;
            }
        }
    }
    for seg in path.segments() {
        any = true;
        let mut ts = vec![0.0, 1.0];
        for line in region.near(seg.bounding_box()) {
            ts.extend(seg.intersect_line(*line).iter().map(|x| x.segment_t).filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup();
        if !ts.iter().all(|t| region.contains(seg.eval(*t))) || !ts.windows(2).all(|w| region.contains(seg.eval((w[0] + w[1]) / 2.0))) {
            return false;
        }
    }
    any
}

fn path_inside(path: &PathData, rule: FillRule, filled: bool, region: &Region) -> bool {
    if !enclosed(&path.to_bezpath(), region) {
        return false;
    }
    // A self-crossing loop can have a hole wholly inside a filled object without crossing its
    // outline. The filled difference also checks that interior (and implicit fill closures).
    !filled || try_boolean(path, rule, &region.data, FillRule::EvenOdd, BoolOp::Difference, DEFAULT_PRECISION).is_ok_and(|outside| outside.is_empty())
}

fn node_inside(n: &Node, symbols: &SymbolBoxes, region: &Region) -> bool {
    let mut bounds = n.geometric_bounds_with(symbols);
    // Curve bounds omit isolated MoveTo points when other subpaths have segments.
    let mut include_points = |path: &PathData| {
        for p in path.subpaths.iter().filter(|sp| sp.segment_count() == 0).filter_map(|sp| sp.anchors.first().map(|a| a.p)) {
            bounds = Some(bounds.map_or_else(|| Rect::from_points(p, p), |b| b.union_pt(p)));
        }
    };
    match &n.kind {
        NodeKind::Path { path, .. } => include_points(path),
        NodeKind::Compound { children, .. } => children.iter().filter_map(|c| c.path_data()).for_each(include_points),
        _ => {}
    }
    // Most candidates lie well inside the loop. A box untouched by its boundary is entirely
    // inside or outside, so it needs no curve intersections or filled boolean operation.
    if let Some(covered) = bounds.and_then(|b| region.box_coverage(b)) {
        return covered;
    }
    let filled = n.appearance.items.iter().any(|item| matches!(item, AppearanceItem::Fill(fill) if fill.visible && !fill.paint.is_none()));
    match &n.kind {
        NodeKind::Path { path, rule, .. } => path_inside(path, *rule, filled, region),
        NodeKind::Compound { rule, .. } => n.stroke_path().is_some_and(|path| path_inside(&PathData::from_bezpath(&path), *rule, filled, region)),
        _ => {
            n.geometric_bounds_with(symbols).is_some_and(|b| path_inside(&PathData::from_bezpath(&b.to_path(0.01)), FillRule::NonZero, true, region))
        }
    }
}

impl Tool for EnclosingLassoTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.active
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            if ev.kind == PointerKind::Up {
                self.active = false;
                self.points.clear();
            }
            return vec![];
        }
        match ev.kind {
            PointerKind::Down => {
                self.points = vec![ev.pos];
                self.active = true;
            }
            PointerKind::Drag if self.active => {
                if self.points.last().is_none_or(|p| p.distance(ev.pos) >= cx.tol(1.0)) {
                    // Cancel rather than testing a truncated loop that could select other art.
                    if self.points.len() >= 4096 {
                        self.active = false;
                        self.points.clear();
                        return vec![];
                    }
                    self.points.push(ev.pos);
                }
            }
            PointerKind::Up if self.active => {
                self.active = false;
                let mut points = std::mem::take(&mut self.points);
                if points.last().is_some_and(|p| p.distance(ev.pos) >= cx.tol(1.0)) {
                    points.push(ev.pos);
                }
                let symbols = cx.doc.symbol_boxes();
                let region = (points.len() >= 3).then(|| Region::new(polygon(&points, true))).flatten();
                let mut ids = if let Some(region) = region {
                    enclosure_candidates(cx.doc, region.path.bounding_box(), cx.isolation)
                        .into_iter()
                        .filter(|id| cx.doc.node(*id).is_some_and(|n| node_inside(n, &symbols, &region)))
                        .collect()
                } else {
                    vec![]
                };
                let command = if ev.mods.alt {
                    ids.retain(|id| cx.selection.contains(*id));
                    "select.toggle"
                } else if ev.mods.shift {
                    "select.add"
                } else {
                    "select.set"
                };
                if ids.is_empty() && (ev.mods.alt || ev.mods.shift) {
                    return vec![];
                }
                return vec![Action::Exec(command.into(), json!({"ids": json_ids(&ids)}))];
            }
            _ => {}
        }
        vec![]
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.active = false;
            self.points.clear();
        }
        vec![]
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        if !self.active || self.points.len() < 2 {
            return vec![];
        }
        vec![Overlay::Path { path: polygon(&self.points, true), color: BLUE, width: 1.0, dashed: false }]
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Lasso
    }
}

#[cfg(test)]
mod tests;
