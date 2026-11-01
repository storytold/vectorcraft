//! Object › Intertwine (#1763): the overlap areas of an Intertwine group, and the tool that
//! arranges them on the canvas.
//!
//! An overlap area is a face of the planar arrangement of what the group's members paint (their
//! fills and strokes, type outlined; other objects by their bounds): a part where two or more of
//! them overlap, split wherever a member's edge crosses it. `object.intertwine.arrange` changes
//! which member is drawn on top there.
//!
//! The Intertwine tool is what `object.intertwine.make` and `object.intertwine.edit` leave the
//! canvas in: hovering highlights the overlap area under the pointer (where three or more members
//! overlap, the outline of the member nearest to the pointer, which the right-click menu
//! arranges: [`menu_target`]), a click arranges it, a drag
//! draws a path around overlap areas (Shift: a rectangle) and arranges every one inside it.
//! Esc or Enter goes back to the tool chosen before. Behaviour:
//! <https://helpx.adobe.com/illustrator/using/intertwine-objects.html>.

use std::sync::Arc;

use kurbo::BezPath;
use serde_json::{Value, json};
use vectorcraft_doc::intertwine::tier_at;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind};
use vectorcraft_effects as effects;
use vectorcraft_geom::{FillRule, PathData, Point, Rect, SubPath, shapes};
use vectorcraft_pathops as po;

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The area `n` paints, in document coordinates, filled non-zero: its visible fills and stroke
/// outlines; type by its outlines; groups by their visible members (clip groups inside their
/// clipping path); other objects by their bounds. Effects are left out. `None` when it paints
/// nothing.
pub fn painted_area(n: &Node) -> Option<PathData> {
    if !n.visible {
        return None;
    }
    let shape = match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => return None,
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            let subs: Vec<SubPath> = children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect();
            Some((PathData::new(subs), *rule))
        }
        NodeKind::Group { children, clip: true } => {
            let clip = children.first().and_then(|c| match &c.kind {
                NodeKind::Path { path, rule, .. } => Some(po::normalize(path, *rule)),
                _ => c.geometric_bounds().map(shapes::rectangle),
            })?;
            let inside = unite(children.iter().skip(1).filter_map(|c| painted_area(c)).collect())?;
            return nonempty(po::boolean(&inside, FillRule::NonZero, &clip, FillRule::NonZero, po::BoolOp::Intersect));
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Intertwine { children } => {
            return unite(children.iter().filter_map(|c| painted_area(c)).collect());
        }
        NodeKind::Text(_) => return effects::outline_text(n).as_ref().and_then(painted_area),
        _ => None,
    };
    let mut parts: Vec<PathData> = vec![];
    match shape {
        Some((path, rule)) => {
            if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !f.paint.is_none())) {
                parts.push(po::normalize(&path, rule));
            }
            for st in n.appearance.painted_strokes().filter(|st| st.width > 0.0 && st.width.is_finite()) {
                parts.push(effects::stroke::outline_region(&path, rule, st));
            }
        }
        None => parts.push(shapes::rectangle(n.visual_bounds()?)),
    }
    unite(parts)
}

fn unite(parts: Vec<PathData>) -> Option<PathData> {
    let parts: Vec<PathData> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => nonempty(po::unite_all(&parts.iter().map(|p| (p, FillRule::NonZero)).collect::<Vec<_>>())),
    }
}

/// Areas smaller than this (pt²) are what a boolean leaves of an area taken from itself.
pub const SLIVER: f64 = 1e-3;

/// `p`, unless it is empty or a sliver.
pub fn nonempty(p: PathData) -> Option<PathData> {
    (!p.is_empty() && po::area(&p, FillRule::NonZero).abs() > SLIVER).then_some(p)
}

/// Does area `a` (filled non-zero) contain `p`?
pub fn contains(a: &PathData, p: Point) -> bool {
    vectorcraft_geom::hit::fill_contains(&a.to_bezpath(), FillRule::NonZero, p)
}

/// One overlap area: its outline, a point inside it, and the members (child indices, bottom
/// first) that paint there.
#[derive(Clone, Debug)]
pub struct Face {
    pub path: PathData,
    pub inside: Point,
    pub members: Vec<usize>,
}

/// Every overlap area of `children`.
pub fn faces(children: &[Arc<Node>]) -> Vec<Face> {
    faces_of(&painted_areas(children))
}

/// What each of `children` paints ([`painted_area`]), by child index.
pub fn painted_areas(children: &[Arc<Node>]) -> Vec<Option<PathData>> {
    children.iter().map(|c| painted_area(c)).collect()
}

/// Every overlap area of the members that paint `areas` ([`painted_areas`]).
pub fn faces_of(areas: &[Option<PathData>]) -> Vec<Face> {
    let shapes: Vec<po::Shape> =
        areas.iter().enumerate().filter_map(|(i, a)| Some(po::Shape::new(a.clone()?, FillRule::NonZero, i as u64))).collect();
    let key = |k: usize| shapes.get(k).map(|s| s.key as usize);
    po::regions(&shapes)
        .into_iter()
        .filter(|r| r.sources.len() >= 2)
        .filter_map(|r| {
            let inside = po::interior_point(&r.path)?;
            Some(Face { members: r.sources.iter().filter_map(|k| key(*k)).collect(), path: r.path, inside })
        })
        .collect()
}

/// The overlap area of `faces` under `p`, with `p` as its inside point (a click picks it).
pub fn face_at(faces: &[Face], p: Point) -> Option<Face> {
    faces.iter().find(|f| contains(&f.path, p)).map(|f| Face { inside: p, ..f.clone() })
}

/// The overlap areas of `faces` whose inside `lasso` (a closed path) encloses.
pub fn faces_in(faces: &[Face], lasso: &PathData) -> Vec<Face> {
    faces.iter().filter(|f| contains(lasso, f.inside)).cloned().collect()
}

/// The members of `face` in the order they are drawn there, bottom first: by their tier there
/// ([`tier_at`]), and members of one tier in the group's order.
pub fn drawn_order(children: &[Arc<Node>], face: &Face) -> Vec<usize> {
    let tier = |i: usize| children.get(i).map_or(0, |c| tier_at(c, face.inside));
    let mut order = face.members.clone();
    order.sort_by_cached_key(|i| (tier(*i), *i));
    order
}

/// The fewest members an overlap area needs for the right-click menu that arranges one of them.
pub const MENU_MEMBERS: usize = 3;

/// The member of `face` whose outline (what it paints, `areas` from [`painted_areas`]) is nearest
/// to `p`.
pub fn nearest_member(areas: &[Option<PathData>], face: &Face, p: Point) -> Option<usize> {
    let dist = |i: usize| areas.get(i).and_then(Option::as_ref).map(|a| vectorcraft_geom::hit::distance_to_outline(&a.to_bezpath(), p));
    face.members.iter().filter_map(|i| Some((*i, dist(*i)?))).filter(|(_, d)| d.is_finite()).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

/// What a right-click at `p` arranges with the Intertwine tool: the Intertwine group of the
/// selection, and the member nearest to `p` (by id) in the overlap area under `p`, when
/// [`MENU_MEMBERS`] or more members overlap there.
pub fn menu_target(doc: &Document, selected: &[NodeId], p: Point) -> Option<(NodeId, NodeId)> {
    let g = selected_group(doc, selected)?;
    let children = doc.node(g)?.children()?;
    let areas = painted_areas(children);
    let face = face_at(&faces_of(&areas), p).filter(|f| f.members.len() >= MENU_MEMBERS)?;
    let m = nearest_member(&areas, &face, p)?;
    Some((g, children.get(m)?.id))
}

/// The Intertwine group the selection is in, or is (the innermost one around the first selected
/// object that is in one).
pub fn selected_group(doc: &Document, selected: &[NodeId]) -> Option<NodeId> {
    selected.iter().find_map(|id| doc.ancestry(*id)?.into_iter().rev().find(|a| doc.node(*a).is_some_and(is_intertwine)))
}

pub fn is_intertwine(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Intertwine { .. })
}

// ---------- the tool ----------

/// The overlap areas of an Intertwine group, kept for the document revision they were found in.
struct Found {
    revision: (u64, u64),
    group: NodeId,
    /// What each member paints, by child index.
    areas: Arc<Vec<Option<PathData>>>,
    faces: Arc<Vec<Face>>,
}

/// The Intertwine tool (not in the toolbar: Make and Edit choose it).
#[derive(Default)]
pub struct IntertwineTool {
    /// The tool to go back to (Esc, Enter).
    prev: Option<String>,
    /// The overlap areas of the group, for the document revision they were found in.
    cache: Option<Found>,
    /// What the pointer highlights: the overlap area under it, or where [`MENU_MEMBERS`] or more
    /// members overlap, the outline of the member nearest to it.
    hover: Option<BezPath>,
    /// A drag's points (document space); the first is where it started.
    points: Vec<Point>,
    /// The drag draws a rectangle (Shift at its start).
    rect: bool,
    dragging: bool,
}

impl IntertwineTool {
    fn faces(&mut self, cx: &ToolContext) -> Option<(NodeId, Arc<Vec<Face>>)> {
        self.found(cx).map(|c| (c.group, c.faces.clone()))
    }

    /// The overlap areas of the selection's group, found again when the document changed.
    fn found(&mut self, cx: &ToolContext) -> Option<&Found> {
        let g = selected_group(cx.doc, &cx.selection.objects)?;
        if !self.cache.as_ref().is_some_and(|c| c.revision == cx.revision && c.group == g) {
            let children = cx.doc.node(g)?.children()?;
            let areas = painted_areas(children);
            let faces = Arc::new(faces_of(&areas));
            self.cache = Some(Found { revision: cx.revision, group: g, areas: Arc::new(areas), faces });
        }
        self.cache.as_ref()
    }

    /// What hovering `p` highlights ([`IntertwineTool::hover`]).
    fn highlight(&mut self, cx: &ToolContext, p: Point) -> Option<BezPath> {
        let found = self.found(cx)?;
        let face = face_at(&found.faces, p)?;
        if face.members.len() >= MENU_MEMBERS
            && let Some(a) = nearest_member(&found.areas, &face, p).and_then(|m| found.areas.get(m)).and_then(Option::as_ref)
        {
            return Some(a.to_bezpath());
        }
        Some(face.path.to_bezpath())
    }

    /// The closed path the drag drew.
    fn lasso(&self) -> Option<PathData> {
        let (first, last) = (*self.points.first()?, *self.points.last()?);
        let pts = if self.rect {
            let r = Rect::from_points(first, last);
            vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
        } else {
            self.points.clone()
        };
        (pts.len() >= 3).then(|| PathData::single(SubPath::polyline(&pts, true)))
    }

    fn back(&self) -> Vec<Action> {
        vec![Action::SwitchTool(self.prev.clone().unwrap_or_else(|| "selection".into()))]
    }
}

impl Tool for IntertwineTool {
    fn id(&self) -> &'static str {
        "intertwine"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = self.highlight(cx, p);
                vec![]
            }
            PointerKind::Down => {
                self.dragging = true;
                self.rect = ev.mods.shift;
                self.points = vec![p];
                vec![]
            }
            PointerKind::Drag if self.dragging => {
                if self.points.last().is_none_or(|q| q.distance(p) >= cx.tol(2.0)) {
                    self.points.push(p);
                }
                vec![]
            }
            PointerKind::Up if self.dragging => {
                self.dragging = false;
                let start = self.points.first().copied().unwrap_or(p);
                self.points.push(p);
                let Some((g, faces)) = self.faces(cx) else { return vec![] };
                // A drag that never left the press is a click (a closed loop ends where it began).
                let click = self.points.iter().all(|q| q.distance(start) < cx.tol(3.0));
                let lasso = if click { None } else { self.lasso() };
                self.points.clear();
                let params = match lasso {
                    // Only where there is something to arrange: a miss changes nothing.
                    Some(l) if !faces_in(&faces, &l).is_empty() => {
                        let area: Vec<Value> = l.subpaths.iter().flat_map(|s| s.anchors.iter()).map(|a| json!([a.p.x, a.p.y])).collect();
                        json!({ "id": g.0, "area": area })
                    }
                    None if face_at(&faces, p).is_some() => json!({ "id": g.0, "at": [p.x, p.y] }),
                    _ => return vec![],
                };
                self.hover = None;
                vec![Action::Exec("object.intertwine.arrange".into(), params)]
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape | ToolKey::Enter if !self.dragging => self.back(),
            ToolKey::Escape => {
                self.dragging = false;
                self.points.clear();
                vec![]
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Escape | ToolKey::Enter)
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        let color = crate::builder::HIGHLIGHT_RED;
        let mut out = vec![];
        if self.dragging && self.points.len() > 1 {
            if let Some(l) = self.lasso() {
                out.push(Overlay::Path { path: l.to_bezpath(), color, width: 1.0, dashed: true });
            }
        } else if let Some(h) = &self.hover {
            out.push(Overlay::Path { path: h.clone(), color, width: 2.0, dashed: false });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Arrow
    }

    fn options(&self) -> Value {
        json!({ "prev": self.prev })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "prev" {
            self.prev = value.as_str().filter(|t| *t != "intertwine").map(str::to_string);
        }
    }

    fn busy(&self) -> bool {
        self.dragging
    }
}

pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == "intertwine").then(|| Box::new(IntertwineTool::default()) as Box<dyn Tool>)
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{Appearance, NodeId};

    use super::*;

    fn square(id: u64, x: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default_art()))
    }

    #[test]
    fn two_overlapping_squares_have_one_overlap_area() {
        let kids = vec![square(1, 0.0), square(2, 5.0), square(3, 40.0)];
        let f = faces(&kids);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].members, [0, 1]);
        let at = face_at(&f, Point::new(7.0, 5.0)).unwrap();
        assert_eq!(at.inside, Point::new(7.0, 5.0));
        assert!(face_at(&f, Point::new(2.0, 5.0)).is_none(), "one square alone is no overlap");
        assert_eq!(drawn_order(&kids, &at), [0, 1]);
    }

    #[test]
    fn a_member_with_an_area_there_is_drawn_last() {
        let mut a = (*square(1, 0.0)).clone();
        a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(5.0, 0.0, 10.0, 10.0))));
        let kids = vec![Arc::new(a), square(2, 5.0)];
        let f = faces(&kids);
        assert_eq!(drawn_order(&kids, &f[0]), [1, 0]);
    }

    #[test]
    fn members_drawn_at_higher_tiers_are_drawn_later() {
        // Three squares over one another; the bottom one at tier 2, the middle one at tier 1.
        let over = || shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut a = (*square(1, 0.0)).clone();
        a.extras_mut().intertwine_tiers = vec![over()];
        let mut b = (*square(2, 0.0)).clone();
        b.extras_mut().intertwine = Some(Box::new(over()));
        let kids = vec![Arc::new(a), Arc::new(b), square(3, 0.0)];
        let f = faces(&kids);
        assert_eq!(f.len(), 1);
        assert_eq!(drawn_order(&kids, &f[0]), [2, 1, 0]);
    }

    #[test]
    fn the_member_nearest_to_the_pointer_is_picked_where_three_overlap() {
        // Squares 20 wide from x = 0, 4 and 8: all three overlap from x = 8 to 20.
        let wide =
            |id: u64, x: f64| Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default_art()));
        let kids = vec![wide(1, 0.0), wide(2, 4.0), wide(3, 8.0)];
        let areas = painted_areas(&kids);
        let all = faces_of(&areas);
        let face = face_at(&all, Point::new(14.0, 10.0)).unwrap();
        assert_eq!(face.members, [0, 1, 2]);
        // By the left edge of the third square, then by the right edge of the first one.
        assert_eq!(nearest_member(&areas, &face, Point::new(8.5, 10.0)), Some(2));
        assert_eq!(nearest_member(&areas, &face, Point::new(19.5, 10.0)), Some(0));
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let g = d.alloc_id();
        d.insert(Some(l), 0, Node::new(g, NodeKind::Intertwine { children: kids })).unwrap();
        assert_eq!(menu_target(&d, &[g], Point::new(8.5, 10.0)), Some((g, NodeId(3))));
        assert_eq!(menu_target(&d, &[g], Point::new(19.5, 10.0)), Some((g, NodeId(1))));
        assert_eq!(menu_target(&d, &[g], Point::new(6.0, 10.0)), None, "only two overlap there");
        assert_eq!(menu_target(&d, &[], Point::new(8.5, 10.0)), None, "no Intertwine group selected");
    }

    #[test]
    fn a_lasso_takes_the_areas_whose_inside_it_encloses() {
        let kids = vec![square(1, 0.0), square(2, 5.0)];
        let f = faces(&kids);
        let around = |r: Rect| shapes::rectangle(r);
        assert_eq!(faces_in(&f, &around(Rect::new(4.0, -1.0, 11.0, 11.0))).len(), 1);
        assert!(faces_in(&f, &around(Rect::new(20.0, 0.0, 30.0, 10.0))).is_empty());
    }
}
