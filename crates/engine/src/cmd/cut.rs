//! Cutting tools' commands: Mirror & Cut, Line Cut and Rectangle Cut.
//!
//! They work on the selected paths and compound paths (inside groups too; a compound path is cut
//! as one shape): filled shapes through `vectorcraft-pathops` booleans against a half-plane or a
//! rectangle, open paths by splitting them where they cross the line or the rectangle's edge (the
//! Knife and Path Eraser helpers). Each command is one undo step; the tools in
//! `vectorcraft_tools::cut` preview it live while dragging.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Anchor, FillRule, PathData, Point, Rect, SubPath, Vec2};
use vectorcraft_pathops as po;
use vectorcraft_pathops::BoolOp;

use super::draw2::{components, erase_outline, ids_json, replace_with_pieces};
use super::edit::selected_roots;
use super::pathops::{Beside, node_path};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.mirrorCut",
            "Mirror & Cut",
            [],
            None,
            "{axis?: \"vertical\"|\"horizontal\"|\"free\" (vertical), from?: [x,y], to?: [x,y] (free: the axis through both; vertical/horizontal: the axis through `from`, default the selection's centre), keep?: \"left\"|\"right\"|\"top\"|\"bottom\" (the side kept; default left, or top for a horizontal-ish axis)} cut the selected paths and compound paths at the axis, reflect the kept half across it and join both halves into one closed shape (open paths are joined where they meet the axis); art wholly on the other side is deleted, as one undo step → {ids, removed}",
            has_selection,
            mirror_cut
        ),
        cmd!(
            "path.lineCut",
            "Line Cut",
            [],
            None,
            "{from: [x,y], to: [x,y]} split the selected paths and compound paths along the line through both points, extended across the art: filled shapes become their pieces on each side (holes kept), open paths split where they cross; as one undo step → {ids, removed: 0}",
            has_selection,
            line_cut
        ),
        cmd!(
            "path.rectCut",
            "Rectangle Cut",
            [],
            None,
            "{rect: [x, y, width, height]} or {from: [x,y], to: [x,y]} (opposite corners) crop the selected paths and compound paths to the rectangle as real geometry (not a clipping mask): filled shapes are intersected with it, open paths keep their parts inside, art wholly outside is deleted; as one undo step → {ids, removed}",
            has_selection,
            rect_cut
        ),
    ]
}

/// Largest coordinate the cut commands accept (keeps the half-plane polygons finite).
const MAX_COORD: f64 = 1e7;
/// How close an open path's end must be to the axis to be joined to its reflection.
const AXIS_TOL: f64 = 1e-3;

/// A straight line through `p` along the unit vector `d`.
#[derive(Clone, Copy, Debug)]
struct Axis {
    p: Point,
    d: Vec2,
}

impl Axis {
    /// The line through `a` and `b` (`None` when they coincide).
    fn through(a: Point, b: Point) -> Option<Self> {
        let v = b - a;
        let len = v.hypot();
        (len > 1e-9 && len.is_finite()).then(|| Axis { p: a, d: v / len })
    }

    /// Signed distance of `q` from the line: positive on the side `d` points to when turned a
    /// quarter turn clockwise on screen (y pointing down).
    fn side(&self, q: Point) -> f64 {
        self.d.cross(q - self.p)
    }

    /// `q` moved onto the line.
    fn project(&self, q: Point) -> Point {
        self.p + self.d * self.d.dot(q - self.p)
    }

    /// The reflection across the line.
    fn reflection(&self) -> Affine {
        let (x, y) = (self.d.x, self.d.y);
        let (a, b, d) = (2.0 * x * x - 1.0, 2.0 * x * y, 2.0 * y * y - 1.0);
        let p = self.p;
        Affine::new([a, b, b, d, p.x - (a * p.x + b * p.y), p.y - (b * p.x + d * p.y)])
    }

    /// A polygon covering everything in `around` on one side of the line (`sign` > 0: where
    /// [`Axis::side`] is positive).
    fn half_plane(&self, sign: f64, around: Rect) -> PathData {
        let r = around.width() + around.height() + self.p.distance(around.center()) + 10.0;
        let n = Vec2::new(-self.d.y, self.d.x) * sign.signum() * 2.0 * r;
        let (a, b) = (self.p - self.d * r, self.p + self.d * r);
        polygon(&[a, b, b + n, a + n])
    }
}

fn polygon(pts: &[Point]) -> PathData {
    PathData::single(SubPath::new(pts.iter().map(|p| Anchor::corner(*p)).collect(), true))
}

/// A point param that must be finite and within [`MAX_COORD`].
fn point_req(p: &Value, key: &str, cmd: &str) -> Result<Point> {
    let q = point_param(p, key).ok_or_else(|| bad(cmd, format!("missing point `{key}` ([x, y])")))?;
    if q.x.abs() > MAX_COORD || q.y.abs() > MAX_COORD {
        return Err(bad(cmd, format!("`{key}` is out of range")));
    }
    Ok(q)
}

/// The editable paths and compound paths in the selection (inside groups and layers too; a
/// compound path is one target; guides and clipping paths are left alone).
fn cut_targets(s: &Session) -> Result<Vec<NodeId>> {
    fn visit(n: &Node, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. } => out.push(n.id),
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().for_each(|c| visit(c, out)),
            _ => {}
        }
    }
    let roots = selected_roots(s)?;
    let doc = &s.doc()?.doc;
    let mut out = vec![];
    for id in roots {
        if let Some(n) = doc.node(id) {
            visit(n, &mut out);
        }
    }
    out.dedup();
    let editable = doc.editable();
    out.retain(|id| editable(*id));
    Ok(out)
}

/// What a cut does to one object: its new pieces (none deletes it).
type Plan = Vec<(NodeId, Vec<PathData>)>;

/// Run `cut` on every target's geometry (`None`: unchanged) and apply the result as one undo step.
fn run_cut(s: &mut Session, cmd: &str, label: &str, cut: &dyn Fn(&PathData, FillRule) -> Option<Vec<PathData>>) -> Result<Value> {
    let targets = cut_targets(s)?;
    if targets.is_empty() {
        return Err(bad(cmd, "select paths or compound paths"));
    }
    let doc = &s.doc()?.doc;
    let mut plan: Plan = vec![];
    for id in targets {
        let Some((pd, rule)) = doc.node(id).and_then(node_path) else { continue };
        if pd.subpaths.iter().flat_map(|sp| &sp.anchors).any(|a| !(a.p.x.is_finite() && a.p.y.is_finite())) {
            return Err(bad(cmd, "paths contain invalid coordinates"));
        }
        if let Some(pieces) = cut(&pd, rule) {
            plan.push((id, pieces));
        }
    }
    apply(s, label, plan)
}

fn apply(s: &mut Session, label: &str, plan: Plan) -> Result<Value> {
    if plan.is_empty() {
        return Ok(json!({ "ids": [], "removed": 0 }));
    }
    let (ids, removed) = s.edit(label, |d: &mut Document, sel| {
        let (mut all, mut removed) = (vec![], 0);
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            removed += pieces.is_empty() as usize;
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok((all, removed))
    })?;
    let mut out = ids_json(&ids);
    out["removed"] = json!(removed);
    Ok(out)
}

/// Bounds of `pd` grown a little (the reference box of the half-planes).
fn reach(pd: &PathData) -> Rect {
    pd.control_bounds().unwrap_or_default().inflate(1.0, 1.0)
}

/// `pd ∩ region`, dropping slivers.
fn intersect(pd: &PathData, rule: FillRule, region: &PathData) -> PathData {
    let r = po::boolean(pd, rule, region, FillRule::NonZero, BoolOp::Intersect);
    if po::area(&r, FillRule::NonZero) <= 1e-6 { PathData::default() } else { r }
}

// ---------- Line Cut ----------

fn line_cut(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.lineCut";
    let axis = Axis::through(point_req(p, "from", C)?, point_req(p, "to", C)?).ok_or_else(|| bad(C, "`from` and `to` must differ"))?;
    run_cut(s, C, "Line Cut", &|pd, rule| split_by_line(pd, rule, &axis))
}

/// `pd` split by the line: the pieces on both sides, `None` when the line misses it.
fn split_by_line(pd: &PathData, rule: FillRule, axis: &Axis) -> Option<Vec<PathData>> {
    if pd.is_closed() {
        let around = reach(pd);
        let a = intersect(pd, rule, &axis.half_plane(1.0, around));
        let b = intersect(pd, rule, &axis.half_plane(-1.0, around));
        if a.is_empty() || b.is_empty() {
            return None;
        }
        return Some(components(&a).into_iter().chain(components(&b)).collect());
    }
    // Open paths: the parts on each side (each pass removes the other side).
    let a = erase_outline(pd, &|q| axis.side(q) < 0.0)?;
    let b = erase_outline(pd, &|q| axis.side(q) >= 0.0)?;
    Some(if pd.subpaths.len() == 1 {
        a.into_iter().chain(b).collect()
    } else {
        vec![PathData::new(a.into_iter().chain(b).flat_map(|p| p.subpaths).collect())]
    })
}

// ---------- Rectangle Cut ----------

fn rect_cut(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.rectCut";
    let rect = match p.get("rect") {
        Some(v) => {
            // All four or nothing: a malformed entry dropped would cut a different region.
            let [x, y, w, h] = finite_numbers(v).ok_or_else(|| bad(C, "rect must be [x, y, width, height]"))?;
            Rect::new(x, y, x + w, y + h).abs()
        }
        None => Rect::from_points(point_req(p, "from", C)?, point_req(p, "to", C)?),
    };
    let finite = [rect.x0, rect.y0, rect.x1, rect.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD);
    if !finite || rect.width() <= 1e-9 || rect.height() <= 1e-9 {
        return Err(bad(C, "the rectangle must have a width and a height"));
    }
    run_cut(s, C, "Rectangle Cut", &|pd, rule| crop(pd, rule, rect))
}

/// `pd` cropped to `rect`: `None` when it lies inside, no pieces when it lies outside.
fn crop(pd: &PathData, rule: FillRule, rect: Rect) -> Option<Vec<PathData>> {
    let b = pd.bounds()?;
    if rect.inflate(1e-9, 1e-9).contains_rect(b) {
        return None;
    }
    if pd.is_closed() {
        let r =
            intersect(pd, rule, &polygon(&[rect.origin(), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)]));
        return Some(if r.is_empty() { vec![] } else { vec![r] });
    }
    let inside = rect.inflate(1e-9, 1e-9);
    let pieces = erase_outline(pd, &|q| !(q.x >= inside.x0 && q.x <= inside.x1 && q.y >= inside.y0 && q.y <= inside.y1))?;
    Some(if pd.subpaths.len() == 1 || pieces.is_empty() {
        pieces
    } else {
        vec![PathData::new(pieces.into_iter().flat_map(|p| p.subpaths).collect())]
    })
}

// ---------- Mirror & Cut ----------

fn mirror_cut(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.mirrorCut";
    let kind = str_param(p, "axis").unwrap_or("vertical");
    let at = || -> Result<Point> {
        match p.get("from") {
            Some(_) => point_req(p, "from", C),
            None => {
                let roots = selected_roots(s)?;
                let b = s.doc()?.doc.bounds_of(&roots, false).ok_or_else(|| bad(C, "the selection has no bounds"))?;
                Ok(b.center())
            }
        }
    };
    let axis = match kind {
        "vertical" => {
            let a = at()?;
            Axis { p: a, d: Vec2::new(0.0, 1.0) }
        }
        "horizontal" => {
            let a = at()?;
            Axis { p: a, d: Vec2::new(1.0, 0.0) }
        }
        "free" => Axis::through(point_req(p, "from", C)?, point_req(p, "to", C)?).ok_or_else(|| bad(C, "`from` and `to` must differ"))?,
        other => return Err(bad(C, format!("unknown axis `{other}` (vertical|horizontal|free)"))),
    };
    let sign = keep_sign(&axis, str_param(p, "keep")).map_err(|e| bad(C, e))?;
    run_cut(s, C, "Mirror & Cut", &|pd, rule| Some(mirror(pd, rule, &axis, sign)))
}

/// The sign of [`Axis::side`] on the side `keep` names (default: left, or top when the axis runs
/// more across than down).
fn keep_sign(axis: &Axis, keep: Option<&str>) -> std::result::Result<f64, String> {
    let keep = keep.unwrap_or(if axis.d.x.abs() > axis.d.y.abs() { "top" } else { "left" });
    let toward = match keep {
        "left" => Vec2::new(-1.0, 0.0),
        "right" => Vec2::new(1.0, 0.0),
        "top" => Vec2::new(0.0, -1.0),
        "bottom" => Vec2::new(0.0, 1.0),
        other => return Err(format!("unknown side `{other}` (left|right|top|bottom)")),
    };
    let s = axis.d.cross(toward);
    if s.abs() < 1e-6 {
        return Err(format!("`{keep}` is not a side of this axis"));
    }
    Ok(s.signum())
}

/// The kept half of `pd` and its reflection, joined (no pieces when nothing is on the kept side).
fn mirror(pd: &PathData, rule: FillRule, axis: &Axis, sign: f64) -> Vec<PathData> {
    let m = axis.reflection();
    if pd.is_closed() {
        let kept = intersect(pd, rule, &axis.half_plane(sign, reach(pd)));
        if kept.is_empty() {
            return vec![];
        }
        let joined = po::boolean(&kept, FillRule::NonZero, &kept.transformed(m), FillRule::NonZero, BoolOp::Union);
        return if joined.is_empty() { vec![] } else { vec![joined] };
    }
    let kept = match erase_outline(pd, &|q| axis.side(q) * sign < 0.0) {
        Some(pieces) => pieces.into_iter().flat_map(|p| p.subpaths).collect(),
        None => pd.subpaths.clone(),
    };
    let subs: Vec<SubPath> = kept.iter().flat_map(|sp| mirror_open(sp, axis, m)).collect();
    if subs.is_empty() { vec![] } else { vec![PathData::new(subs)] }
}

/// An open run and its reflection, joined where its ends touch the axis (closed when both do);
/// the two apart when neither does.
fn mirror_open(sp: &SubPath, axis: &Axis, m: Affine) -> Vec<SubPath> {
    let on = |a: Option<&Anchor>| a.is_some_and(|a| axis.side(a.p).abs() <= AXIS_TOL);
    let (start_on, end_on) = (on(sp.anchors.first()), on(sp.anchors.last()));
    if sp.closed || sp.anchors.len() < 2 || !(start_on || end_on) {
        return vec![sp.clone(), SubPath::new(sp.anchors.iter().map(|a| a.transform(m)).collect(), sp.closed)];
    }
    // The run ends on the axis; the reflection comes back from there.
    let mut run = sp.clone();
    if !end_on {
        run.reverse();
    }
    let Some((last, body)) = run.anchors.split_last() else { return vec![] };
    let q = axis.project(last.p);
    let mut anchors = body.to_vec();
    anchors.push(Anchor::with_handles(q, last.h_in, m * last.h_in));
    anchors.extend(body.iter().rev().map(|a| Anchor::with_handles(m * a.p, m * a.h_out, m * a.h_in)));
    let closed = start_on && end_on && anchors.len() > 2;
    if closed && let (Some(first), Some(back)) = (anchors.first().copied(), anchors.pop()) {
        // The start and its reflection meet on the axis: one closed shape.
        if let Some(f) = anchors.first_mut() {
            *f = Anchor::with_handles(axis.project(first.p), back.h_in, first.h_out);
        }
    }
    vec![SubPath::new(anchors, closed)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_maps_across_the_axis() {
        let a = Axis::through(Point::new(0.0, 0.0), Point::new(10.0, 10.0)).unwrap();
        let q = a.reflection() * Point::new(5.0, 0.0);
        assert!((q.x - 0.0).abs() < 1e-9 && (q.y - 5.0).abs() < 1e-9, "{q:?}");
        let v = Axis { p: Point::new(100.0, 0.0), d: Vec2::new(0.0, 1.0) };
        let q = v.reflection() * Point::new(90.0, 7.0);
        assert!((q.x - 110.0).abs() < 1e-9 && (q.y - 7.0).abs() < 1e-9);
    }

    #[test]
    fn keep_names_a_side_of_the_axis() {
        let v = Axis { p: Point::ZERO, d: Vec2::new(0.0, 1.0) };
        let left = keep_sign(&v, Some("left")).unwrap();
        assert!(v.side(Point::new(-5.0, 0.0)) * left > 0.0);
        assert_eq!(keep_sign(&v, None).unwrap(), left);
        assert!(keep_sign(&v, Some("top")).is_err());
        let h = Axis { p: Point::ZERO, d: Vec2::new(1.0, 0.0) };
        let top = keep_sign(&h, None).unwrap();
        assert!(h.side(Point::new(0.0, -5.0)) * top > 0.0);
    }

    #[test]
    fn open_half_mirrors_into_a_closed_shape() {
        // An arch from the axis and back to it: its reflection closes it.
        let axis = Axis { p: Point::new(50.0, 0.0), d: Vec2::new(0.0, 1.0) };
        let sp = SubPath::new(
            vec![Anchor::corner(Point::new(50.0, 0.0)), Anchor::corner(Point::new(10.0, 50.0)), Anchor::corner(Point::new(50.0, 100.0))],
            false,
        );
        let out = mirror_open(&sp, &axis, axis.reflection());
        assert_eq!(out.len(), 1);
        assert!(out[0].closed);
        assert_eq!(out[0].anchors.len(), 4);
        assert!(out[0].anchors.iter().any(|a| (a.p.x - 90.0).abs() < 1e-9));
        // One end on the axis: one open path through it.
        let sp = SubPath::new(vec![Anchor::corner(Point::new(10.0, 0.0)), Anchor::corner(Point::new(50.0, 40.0))], false);
        let out = mirror_open(&sp, &axis, axis.reflection());
        assert_eq!((out.len(), out[0].closed, out[0].anchors.len()), (1, false, 3));
    }
}
