//! Live Corners: the widgets inside the corners of the selected paths. The Selection tool shows
//! them on live rectangles and polygons, Direct Selection on any path (a star, a pen path) at each
//! corner anchor: one without handles between two straight sides. Dragging one rounds (or
//! sharpens) the corners whose widgets show together, on every selected path (#938): every corner
//! of a path selected whole, those with a selected anchor when Direct Selection picked some. Alt-clicking a widget cycles their corner
//! kind (round, inverted round, chamfer); double-clicking one opens the Corners dialog. The
//! Selection and Direct Selection tools share this, and the canvas draws the widgets from the same
//! geometry.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, LiveCorners, LiveShape, NodeId, NodeKind, Selection};
use vectorcraft_geom::corners::{Corner, cut_corners};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Vec2};

use crate::{Action, Overlay, PointerEvent, ToolContext};

/// The dialog a double-click on a corner widget opens: `{id, corners}`.
pub const DIALOG: &str = "corners";

/// Widgets sit at least this far inside their corner along each side of a right angle (screen px;
/// √2 times it along the bisector, at any angle), further in once the radius is.
const MIN_INSET_PX: f64 = 10.0;
/// Corners with a side shorter than this on screen (px) hide their widgets.
const MIN_SIDE_PX: f64 = 3.0 * MIN_INSET_PX;

/// One corner's widget.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Widget {
    /// The corner, in the corners' own space.
    corner: Corner,
    /// The radius it is set to (document units) and its kind.
    radius: f64,
    kind: CornerKind,
    /// The angle between its sides in the document, in degrees.
    angle: f64,
    /// The widget's centre in the document.
    at: Point,
}

/// The corner widgets of one selected path.
#[derive(Clone, Debug, PartialEq)]
struct PathWidgets {
    id: NodeId,
    /// Maps the corners' space into the document (a live rectangle's own; else none).
    xf: Affine,
    /// The widgets that show (and which a drag edits), in anchor order.
    widgets: Vec<Widget>,
    /// Every corner of the path shows its widget.
    all: bool,
}

impl PathWidgets {
    /// The widgets of path `id` at `zoom` (screen px per document point), if it has any: any path
    /// with Direct Selection (`any_path`), live rectangles and polygons with Selection.
    fn of(doc: &Document, selection: &Selection, id: NodeId, zoom: f64, any_path: bool) -> Option<Self> {
        if !doc.is_editable(id) {
            return None;
        }
        let NodeKind::Path { path, live, .. } = &doc.node(id)?.kind else { return None };
        if !any_path && !matches!(live, Some(LiveShape::Rectangle { .. } | LiveShape::Polygon { .. })) {
            return None;
        }
        let lc = LiveCorners::new(path, live.as_ref())?;
        let det = lc.xf.determinant().abs();
        if det < 1e-12 {
            return None;
        }
        // Screen pixels per unit of the corners' space (a rectangle's may be skewed).
        let scale = zoom * det.sqrt();
        let picked = lc.picked(selection.partial(id));
        let widgets: Vec<_> = lc
            .corners
            .iter()
            .filter(|c| picked.contains(&c.index) && c.lu.min(c.lv) * scale >= MIN_SIDE_PX)
            .map(|c| {
                // On the bisector at the centre of the arc as drawn, at least the minimum inset
                // in and at most half the shorter side.
                let radius = lc.radius(c.index);
                let k = (c.fitted(radius) / c.sin()).max(MIN_INSET_PX / (scale * (1.0 + c.cos()).sqrt())).min(c.lu.min(c.lv) / 2.0);
                Widget { corner: *c, radius, kind: lc.kind(c.index), angle: angle_in(lc.xf, c), at: lc.xf * c.on_bisector(k) }
            })
            .collect();
        let all = widgets.len() == lc.corners.len();
        (!widgets.is_empty()).then_some(Self { id, xf: lc.xf, widgets, all })
    }

    /// The corners whose widgets show: anchor indices of the path's uncut outline.
    fn corners(&self) -> Vec<usize> {
        self.widgets.iter().map(|w| w.corner.index).collect()
    }
}

/// The corner widgets of the selected paths that have some.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerWidgets {
    paths: Vec<PathWidgets>,
}

impl CornerWidgets {
    /// The widgets at `zoom` (screen px per document point), if the selection has them: on any
    /// path with Direct Selection (`any_path`), on live rectangles and polygons with Selection.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64, any_path: bool) -> Option<Self> {
        let paths: Vec<_> = selection.objects.iter().filter_map(|id| PathWidgets::of(doc, selection, *id, zoom, any_path)).collect();
        (!paths.is_empty()).then_some(Self { paths })
    }

    /// Selection & Anchor Display → Hide Corner Widget for angles greater than: the widgets of
    /// corners wider than `max` degrees (in the document) hide; none if all do.
    pub fn within_angle(mut self, max: f64) -> Option<Self> {
        for p in &mut self.paths {
            let n = p.widgets.len();
            p.widgets.retain(|w| w.angle <= max + 1e-9);
            p.all &= p.widgets.len() == n;
        }
        self.paths.retain(|p| !p.widgets.is_empty());
        (!self.paths.is_empty()).then_some(self)
    }

    /// The widgets on show: [`Self::of`] with View → Show Corner Widget `on`, less those wider
    /// than `max_angle` ([`Self::within_angle`]).
    pub fn showing(doc: &Document, selection: &Selection, zoom: f64, any_path: bool, on: bool, max_angle: f64) -> Option<Self> {
        if !on {
            return None;
        }
        Self::of(doc, selection, zoom, any_path)?.within_angle(max_angle)
    }

    /// The widgets the active tool can drag (Direct Selection: `any_path`).
    pub fn for_tool(cx: &ToolContext, any_path: bool) -> Option<Self> {
        Self::showing(cx.doc, cx.selection, cx.zoom, any_path, cx.corner_widgets, cx.corner_widget_max_angle)
    }

    /// The centres of the widgets that show.
    pub fn visible(&self) -> impl Iterator<Item = Point> + '_ {
        self.visible_on().map(|(_, at)| at)
    }

    /// The centres of the widgets that show, each with the object it belongs to.
    pub fn visible_on(&self) -> impl Iterator<Item = (NodeId, Point)> + '_ {
        self.paths.iter().flat_map(|p| p.widgets.iter().map(move |w| (p.id, w.at)))
    }

    /// The corners whose widgets show, path after path: anchor indices of each path's uncut
    /// outline.
    pub fn corners(&self) -> Vec<usize> {
        self.paths.iter().flat_map(PathWidgets::corners).collect()
    }

    /// The widget nearest to `p` within `tol` (document units): its path's and its own index.
    fn hit(&self, p: Point, tol: f64) -> Option<(usize, usize)> {
        let d = |&(i, k): &(usize, usize)| self.widget(i, k).map_or(f64::INFINITY, |w| w.at.distance(p));
        self.paths
            .iter()
            .enumerate()
            .flat_map(|(i, path)| (0..path.widgets.len()).map(move |k| (i, k)))
            .filter(|ik| d(ik) <= tol)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
    }

    fn widget(&self, path: usize, k: usize) -> Option<&Widget> {
        self.paths.get(path).and_then(|p| p.widgets.get(k))
    }

    /// `object.setLiveShape` params setting `key` on the shown corners (every corner of a path
    /// unless some are hidden, which `corners` then leaves out): one path by its `id`, several as
    /// `items`.
    fn command(&self, key: &str, value: Value) -> Value {
        let corners = |p: &PathWidgets| (!p.all).then(|| p.corners());
        let mut out = Map::new();
        match self.paths.as_slice() {
            [p] => {
                out.insert("id".into(), json!(p.id.0));
                if let Some(c) = corners(p) {
                    out.insert("corners".into(), json!(c));
                }
            }
            paths => {
                let items: Vec<Value> = paths
                    .iter()
                    .map(|p| match corners(p) {
                        Some(c) => json!({ "id": p.id.0, "corners": c }),
                        None => json!({ "id": p.id.0 }),
                    })
                    .collect();
                out.insert("items".into(), Value::Array(items));
            }
        }
        out.insert(key.into(), value);
        Value::Object(out)
    }
}

/// The angle between corner `c`'s sides once `xf` maps them into the document, in degrees.
fn angle_in(xf: Affine, c: &Corner) -> f64 {
    let [a, b, cc, d, _, _] = xf.as_coeffs();
    let map = |v: Vec2| Vec2::new(a * v.x + cc * v.y, b * v.x + d * v.y);
    let (u, v) = (map(c.u), map(c.v));
    u.cross(v).abs().atan2(u.dot(v)).to_degrees()
}

/// Is `p` over a corner widget the active tool would drag (Direct Selection: `any_path`)?
pub fn over_widget(cx: &ToolContext, p: Point, any_path: bool) -> bool {
    CornerWidgets::for_tool(cx, any_path).and_then(|w| w.hit(p, cx.tol(5.0))).is_some()
}

/// A double-click on a corner widget opens the Corners dialog for the shown corners of its path.
pub fn double_click(cx: &ToolContext, p: Point, any_path: bool) -> Option<Action> {
    let w = CornerWidgets::for_tool(cx, any_path)?;
    let (i, _) = w.hit(p, cx.tol(5.0))?;
    let path = w.paths.get(i)?;
    Some(Action::Dialog(DIALOG.into(), json!({ "id": path.id.0, "corners": path.corners() })))
}

/// A path's uncut outline with its corners' radii and kinds, and the corners' space (to outline
/// the corners at their largest).
#[derive(Clone, Debug)]
struct Outline {
    base: PathData,
    radii: Vec<f64>,
    kinds: Vec<CornerKind>,
    xf: Affine,
}

/// Dragging a corner widget: the radius follows the pointer along the corner's bisector. An
/// Alt-click (no drag) cycles the corner kind instead.
#[derive(Clone, Debug)]
pub struct CornerDrag {
    widgets: CornerWidgets,
    /// The dragged widget: its path's and its own index among the shown ones.
    widget: (usize, usize),
    /// Each shown path's outline, in the order of [`CornerWidgets`]' paths.
    outlines: Vec<Outline>,
    start: Point,
    began: bool,
    alt: bool,
    radius: f64,
    /// The pointer went past fully round: the dragged corner is at its largest radius.
    at_limit: bool,
    at: Point,
}

impl CornerDrag {
    /// Start a drag when the press `ev` is on a widget of the selection (Direct Selection:
    /// `any_path`).
    pub fn hit(cx: &ToolContext, ev: &PointerEvent, any_path: bool) -> Option<Self> {
        let (widgets, p) = (CornerWidgets::for_tool(cx, any_path)?, ev.pos);
        let widget = widgets.hit(p, cx.tol(5.0))?;
        let radius = widgets.widget(widget.0, widget.1)?.radius;
        let outlines = widgets
            .paths
            .iter()
            .map(|w| {
                let lc = cx.doc.node(w.id).and_then(LiveCorners::of)?;
                let n = lc.base.anchor_count();
                let (radii, kinds) = ((0..n).map(|k| lc.radius(k)).collect(), (0..n).map(|k| lc.kind(k)).collect());
                Some(Outline { base: lc.base.into_owned(), radii, kinds, xf: w.xf })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { widgets, widget, outlines, start: p, began: false, alt: ev.mods.alt, radius, at_limit: false, at: p })
    }

    /// The start radius (as drawn: no larger than fits) plus the pointer's travel along the
    /// corner's bisector (in the corners' own units), from square to fully round; and whether the
    /// travel reached fully round.
    fn radius_at(&self, p: Point) -> (f64, bool) {
        let (path, k) = self.widget;
        let (Some(w), Some(xf)) = (self.widgets.widget(path, k), self.widgets.paths.get(path).map(|p| p.xf)) else {
            return (self.radius, self.at_limit);
        };
        let inv = xf.inverse();
        let max = w.corner.max_radius();
        let r = w.corner.fitted(w.radius) + w.corner.radius_change(inv * p - inv * self.start);
        (r.min(max).max(0.0), max > 0.0 && r >= max)
    }

    /// The cuts of the edited corners that are at their largest radius, in document coordinates.
    fn corners_at_limit(&self) -> BezPath {
        let mut out = BezPath::new();
        for (shown, o) in self.widgets.paths.iter().zip(&self.outlines) {
            let shown = &shown.widgets;
            let mut radii = o.radii.clone();
            for w in shown {
                if let Some(r) = radii.get_mut(w.corner.index) {
                    *r = self.radius;
                }
            }
            let full = |k: &usize| shown.iter().any(|w| w.corner.index == *k && self.radius >= w.corner.max_radius() - 1e-9);
            let (path, sources) = cut_corners(&o.base, &radii, &o.kinds);
            for (sp, from) in path.transformed(o.xf).subpaths.iter().zip(&sources) {
                // A corner's cut is the segment between its two anchors (the last one, for a
                // closed subpath's first corner).
                for i in 0..sp.segment_count() {
                    let (a, b) = (from.get(i), from.get(i + 1).or(from.first()));
                    if a == b && a.is_some_and(full) {
                        let c = sp.segment(i);
                        out.move_to(c.p0);
                        out.curve_to(c.p1, c.p2, c.p3);
                    }
                }
            }
        }
        out
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Corner Radius".into()));
        }
        (self.radius, self.at_limit) = self.radius_at(p);
        self.at = p;
        out.push(Action::Preview("object.setLiveShape".into(), self.widgets.command("radius", json!(self.radius))));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began {
            vec![Action::Commit]
        } else if let Some(w) = self.widgets.widget(self.widget.0, self.widget.1).filter(|_| self.alt) {
            vec![Action::Exec("object.setLiveShape".into(), self.widgets.command("kind", json!(w.kind.next())))]
        } else {
            vec![]
        }
    }

    /// The radius readout next to the pointer; at the largest radius, the corners there in red.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began {
            return vec![];
        }
        let mut out = vec![Overlay::Measure { p: self.at, text: format!("Radius: {}", cx.len(self.radius)) }];
        if self.at_limit {
            out.push(Overlay::Path { path: self.corners_at_limit(), color: crate::builder::HIGHLIGHT_RED, width: 2.0, dashed: false });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{Mods, PointerKind};
    use vectorcraft_doc::{Appearance, Node};

    /// A live 100 × 100 rectangle at (100, 100) with corner radius `r`.
    fn live_rect(r: f64, xf: Affine) -> (Document, NodeId) {
        live_shape(100.0, 100.0, r, xf)
    }

    /// A live `w` × `h` rectangle with corner radius `r`.
    fn live_shape(w: f64, h: f64, r: f64, xf: Affine) -> (Document, NodeId) {
        let live = LiveShape::Rectangle { w, h, radii: [r; 4], kinds: Default::default(), xf };
        doc_with(live.to_path(), Some(live))
    }

    /// A document holding one path with its live shape.
    fn doc_with(path: PathData, live: Option<LiveShape>) -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut n = Node::path(id, path, Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = live;
        }
        d.insert(Some(l), 0, n).unwrap();
        (d, id)
    }

    /// A five-pointed star at (200, 200) of radii 60 and 30 (#511): a plain path.
    fn star() -> (Document, NodeId) {
        doc_with(vectorcraft_geom::shapes::star(Point::new(200.0, 200.0), 60.0, 30.0, 5, 0.0), None)
    }

    fn selected(id: NodeId) -> Selection {
        let mut s = Selection::default();
        s.add(id);
        s
    }

    fn pts(w: &CornerWidgets) -> Vec<Point> {
        w.visible().collect()
    }

    fn radius(a: &Action) -> f64 {
        let Action::Preview(c, v) = a else { panic!("not a preview: {a:?}") };
        assert_eq!(c, "object.setLiveShape");
        v["radius"].as_f64().unwrap()
    }

    #[test]
    fn widgets_sit_inside_each_corner() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
        assert_eq!(pts(&w), [Point::new(110.0, 110.0), Point::new(190.0, 110.0), Point::new(190.0, 190.0), Point::new(110.0, 190.0)]);
        // A larger radius moves them to the arc centres; zooming in keeps a 10 px minimum.
        let (d, id) = live_rect(20.0, Affine::translate((100.0, 100.0)));
        assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap())[0], Point::new(120.0, 120.0));
        assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 4.0, false).unwrap())[2], Point::new(180.0, 180.0));
        // Hidden when the shape is tiny on screen and without a single selection; a plain path
        // shows them with Direct Selection only.
        assert!(CornerWidgets::of(&d, &selected(id), 0.2, true).is_none());
        let (d, id) = doc_with_rect();
        assert!(CornerWidgets::of(&d, &selected(id), 1.0, false).is_none());
        assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 1.0, true).unwrap())[0], Point::new(110.0, 110.0));
        assert!(CornerWidgets::of(&d, &Selection::default(), 1.0, true).is_none());
    }

    #[test]
    fn dragging_a_widget_inward_rounds_all_corners_in_one_step() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for mut t in [crate::create("selection"), crate::create("directSelection")] {
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 191.0)).is_empty());
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 175.0, 176.0));
            assert_eq!(a[0], Action::Begin("Corner Radius".into()));
            assert!((radius(&a[1]) - 15.0).abs() < 1e-9);
            assert!(matches!(&a[1], Action::Preview(_, v) if v["id"] == id.0));
            assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Radius: 15.00 pt")));
            // Past the middle the corner is fully round; back outside it is square.
            assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 100.0))[0]), 50.0);
            assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 260.0))[0]), 0.0);
            assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 260.0)), vec![Action::Commit]);
        }
    }

    /// #938: with two rectangles selected, both show their widgets, and a drag on either rounds
    /// the corners of both in one step; with Direct Selection, each its shown corners.
    #[test]
    fn widgets_of_several_selected_paths_drag_together() {
        let (mut d, a) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let live = LiveShape::Rectangle { w: 100.0, h: 100.0, radii: [0.0; 4], kinds: Default::default(), xf: Affine::translate((300.0, 100.0)) };
        let b = d.alloc_id();
        let mut n = Node::path(b, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(live);
        }
        let l = d.layers[0].id;
        d.insert(Some(l), 1, n).unwrap();
        let mut s = selected(a);
        s.add(b);
        assert_eq!(pts(&CornerWidgets::of(&d, &s, 1.0, false).unwrap()).len(), 8);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = crate::create("selection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 390.0, 191.0));
        let preview = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 375.0, 176.0));
        assert!((radius(&preview[1]) - 15.0).abs() < 1e-9);
        let Action::Preview(_, v) = &preview[1] else { panic!() };
        assert_eq!(v["items"], json!([{"id": a.0}, {"id": b.0}]));
        // Direct Selection with one anchor of the first: its corner, and the whole second.
        s.anchors.insert(a, [(0, 2)].into());
        let w = CornerWidgets::of(&d, &s, 1.0, true).unwrap();
        assert_eq!(w.corners(), [2, 0, 1, 2, 3]);
        assert_eq!(w.command("radius", json!(5.0)), json!({"items": [{"id": a.0, "corners": [2]}, {"id": b.0}], "radius": 5.0}));
    }

    /// #442: on a 200 × 80 rectangle the widgets sit at the centres of the corners as drawn, also
    /// with a radius past half the shorter side; a drag past fully round stops every corner there
    /// and outlines them in red.
    #[test]
    fn widgets_of_a_non_square_rectangle_follow_the_drawn_corners() {
        let (d, id) = live_shape(200.0, 80.0, 60.0, Affine::translate((100.0, 100.0)));
        let s = selected(id);
        let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
        let centres = [Point::new(140.0, 140.0), Point::new(260.0, 140.0), Point::new(260.0, 140.0), Point::new(140.0, 140.0)];
        assert_eq!(pts(&w), centres);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let red = |t: &dyn crate::Tool| {
            t.overlays(&cx).into_iter().find_map(|o| match o {
                Overlay::Path { path, color, .. } if color == crate::builder::HIGHLIGHT_RED => Some(path),
                _ => None,
            })
        };
        let mut t = crate::create("selection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 140.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 200.0));
        assert_eq!(radius(&a[1]), 40.0);
        // The four arcs, each a quarter circle of radius 40 starting on a side.
        let arcs = red(t.as_ref()).expect("the limit shows");
        let starts: Vec<_> =
            arcs.elements().iter().filter_map(|e| if let vectorcraft_geom::PathEl::MoveTo(p) = e { Some(*p) } else { None }).collect();
        assert_eq!(starts, [Point::new(260.0, 100.0), Point::new(300.0, 140.0), Point::new(140.0, 180.0), Point::new(100.0, 140.0)]);
        // Back below the limit, the outline goes.
        assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0))[0]), 30.0);
        assert!(red(t.as_ref()).is_none());
    }

    /// #442: a rectangle saved stretched (an uneven scale in its transform, before #291) shows its
    /// widgets and drags its radius in document units, as `object.setLiveShape` sets it.
    #[test]
    fn a_stretched_rectangle_drags_its_radius_in_document_units() {
        // 50 × 80 with 10 pt corners stretched 4 × 1: drawn 200 × 80, a mean radius of 20.
        let (d, id) = live_shape(50.0, 80.0, 10.0, Affine::translate((100.0, 100.0)) * Affine::scale_non_uniform(4.0, 1.0));
        let s = selected(id);
        let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
        assert_eq!(pts(&w)[0], Point::new(120.0, 120.0));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = crate::create("directSelection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 120.0, 120.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 110.0, 110.0));
        assert!((radius(&a[1]) - 10.0).abs() < 1e-9, "{a:?}");
    }

    #[test]
    fn drag_follows_a_rotated_shape_and_a_click_does_nothing() {
        // Rotated 90°: the shape's top-left corner is at document (200, 100).
        let xf = Affine::translate((200.0, 100.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let (d, id) = live_rect(10.0, xf);
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
        assert!(pts(&w)[0].distance(Point::new(190.0, 110.0)) < 1e-9);
        let mut t = crate::create("directSelection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 120.0));
        assert!((radius(&a[1]) - 20.0).abs() < 1e-9);
        // A click without a drag leaves the shape (and the undo history) alone.
        let mut t = crate::create("selection");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0)).is_empty());
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 190.0, 110.0)).is_empty());
    }

    #[test]
    fn direct_selected_corners_show_their_widgets_alone_and_round_alone() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        // Direct Selection picked the bottom-right anchor.
        let mut s = selected(id);
        s.anchors.insert(id, [(0, 2)].into());
        let p = paint();
        let cx = cx(&d, &s, &p);
        let w = CornerWidgets::of(&d, &s, 1.0, true).unwrap();
        assert_eq!(w.corners(), [2]);
        assert_eq!(pts(&w), [Point::new(190.0, 190.0)]);
        assert!(!over_widget(&cx, Point::new(110.0, 110.0), true), "the other corners hide theirs");
        let mut t = crate::create("directSelection");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 191.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 175.0, 176.0));
        assert_eq!(a[0], Action::Begin("Corner Radius".into()));
        assert_eq!(a[1], Action::Preview("object.setLiveShape".into(), json!({"id": id.0, "radius": 15.0, "corners": [2]})));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 175.0, 176.0)), vec![Action::Commit]);
    }

    #[test]
    fn alt_click_cycles_the_kind_and_double_click_opens_the_dialog() {
        let (d, id) = live_rect(10.0, Affine::translate((100.0, 100.0)));
        let p = paint();
        let alt = Mods { alt: true, ..Mods::default() };
        let whole = selected(id);
        let mut part = selected(id);
        part.anchors.insert(id, [(0, 0), (0, 7)].into());
        for (s, extra) in [(&whole, json!({})), (&part, json!({"corners": [0]}))] {
            let cx = cx(&d, s, &p);
            let mut want = json!({"id": id.0, "kind": "invertedRound"});
            want.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            for tool in ["selection", "directSelection"] {
                let mut t = crate::create(tool);
                assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 110.0).with_mods(alt)).is_empty());
                let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 110.0, 110.0).with_mods(alt));
                assert_eq!(a, vec![Action::Exec("object.setLiveShape".into(), want.clone())], "{tool}");
            }
        }
        // A double-click on a widget opens Corners for the shown corners; elsewhere it doesn't.
        let cx1 = cx(&d, &part, &p);
        let a = crate::create("directSelection").pointer(&cx1, &PointerEvent::new(PointerKind::DoubleClick, 110.0, 110.0));
        assert_eq!(a, vec![Action::Dialog(DIALOG.into(), json!({"id": id.0, "corners": [0]}))]);
        let cx2 = cx(&d, &whole, &p);
        let a = crate::create("selection").pointer(&cx2, &PointerEvent::new(PointerKind::DoubleClick, 190.0, 110.0));
        assert_eq!(a, vec![Action::Dialog(DIALOG.into(), json!({"id": id.0, "corners": [0, 1, 2, 3]}))]);
        assert!(crate::create("selection").pointer(&cx2, &PointerEvent::new(PointerKind::DoubleClick, 150.0, 150.0)).is_empty());
    }

    /// Hide Corner Widget for angles greater than (#394): a rectangle's right angles hide below
    /// 90°; sheared, only its acute corners keep their widgets.
    #[test]
    fn corners_wider_than_the_preference_hide_their_widgets() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
        assert_eq!(w.clone().within_angle(177.0).map(|w| w.corners()), Some(vec![0, 1, 2, 3]));
        assert_eq!(w.clone().within_angle(90.0).map(|w| w.corners()), Some(vec![0, 1, 2, 3]));
        assert!(w.within_angle(89.0).is_none());
        // Sheared by 30°: 60° at the top-left and bottom-right corners, 120° at the others.
        let shear = Affine::translate((100.0, 100.0)) * Affine::new([1.0, 0.0, 30f64.to_radians().tan(), 1.0, 0.0, 0.0]);
        let (d, id) = live_rect(0.0, shear);
        let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
        assert_eq!(w.clone().within_angle(100.0).map(|w| w.corners()), Some(vec![0, 2]));
        let s = selected(id);
        let p = paint();
        let c = ToolContext { corner_widget_max_angle: 100.0, ..cx(&d, &s, &p) };
        assert!(over_widget(&c, pts(&w)[0], false));
        assert!(!over_widget(&c, pts(&w)[1], false), "the 120° corner's widget is hidden");
    }

    #[test]
    fn widgets_off_or_elsewhere_keep_the_tools_behaviour() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let s = selected(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        assert!(over_widget(&c, Point::new(110.0, 110.0), false));
        assert!(!over_widget(&c, Point::new(150.0, 150.0), false));
        c.corner_widgets = false;
        assert!(!over_widget(&c, Point::new(110.0, 110.0), true));
        // With the widgets hidden, a drag from the same spot moves the object.
        let mut t = crate::create("selection");
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 110.0, 110.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 110.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
    }

    /// #511: Direct Selection shows a widget in each of a star's ten corners (the Selection tool
    /// none: a star isn't a live shape); dragging a tip's inward rounds them all as one step.
    #[test]
    fn a_star_shows_widgets_in_every_corner_with_direct_selection() {
        let (d, id) = star();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        assert!(CornerWidgets::for_tool(&cx, false).is_none());
        let w = CornerWidgets::for_tool(&cx, true).expect("widgets on a plain path");
        assert_eq!(w.corners(), (0..10).collect::<Vec<_>>());
        // The top tip's widget sits 10 √2 px down its bisector.
        let tip = pts(&w)[0];
        assert!(tip.distance(Point::new(200.0, 140.0 + 10.0 * 2f64.sqrt())) < 1e-9, "{tip:?}");
        let mut t = crate::create("directSelection");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, tip.x, tip.y + 10.0));
        assert_eq!(a[0], Action::Begin("Corner Radius".into()));
        // The widget follows the arc's centre: r / sin(angle / 2) along the bisector.
        let c = vectorcraft_geom::corners::path_corners(&star_path(&d, id))[0];
        let want = 10.0 * (c.angle_deg() / 2.0).to_radians().sin();
        assert!((radius(&a[1]) - want).abs() < 1e-9, "{a:?} want {want}");
        assert!(matches!(&a[1], Action::Preview(_, v) if v["id"] == id.0 && v.get("corners").is_none()), "every corner: {a:?}");
        // The Selection tool drags the star instead.
        let mut t = crate::create("selection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, tip.x, tip.y + 10.0))[0], Action::Begin("Move".into()));
    }

    fn star_path(d: &Document, id: NodeId) -> PathData {
        let Some(NodeKind::Path { path, .. }) = d.node(id).map(|n| &n.kind) else { panic!("not a path") };
        path.clone()
    }

    /// #511: a Direct-Selected anchor of the star shows its corner's widget alone, and the
    /// preference hides the star's wide inner corners (about 125°) below their angle.
    #[test]
    fn star_corners_follow_the_anchors_and_the_angle_preference() {
        let (d, id) = star();
        let mut s = selected(id);
        s.anchors.insert(id, [(0, 3)].into());
        let p = paint();
        let c = cx(&d, &s, &p);
        let w = CornerWidgets::for_tool(&c, true).unwrap();
        assert_eq!(w.corners(), [3]);
        let mut t = crate::create("directSelection");
        let at = pts(&w)[0];
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x + 20.0, at.y + 20.0));
        assert!(matches!(&a[1], Action::Preview(_, v) if v["corners"] == json!([3])), "{a:?}");
        // Corners wider than 90° hide: the five tips keep theirs.
        let whole = selected(id);
        let c = ToolContext { corner_widget_max_angle: 90.0, ..cx(&d, &whole, &p) };
        let w = CornerWidgets::for_tool(&c, true).unwrap();
        assert_eq!(w.corners(), [0, 2, 4, 6, 8]);
        assert_eq!(w.command("radius", json!(4.0))["corners"], json!([0, 2, 4, 6, 8]));
    }

    /// A live polygon shows its widgets to the Selection tool too; past every corner's limit the
    /// cuts outline in red.
    #[test]
    fn a_live_polygon_rounds_with_either_tool() {
        let live = LiveShape::Polygon { radius: 80.0, sides: 6, xf: Affine::translate((200.0, 200.0)), radii: vec![], kinds: vec![] };
        let (d, id) = doc_with(live.to_path(), Some(live));
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for any_path in [false, true] {
            assert_eq!(CornerWidgets::for_tool(&cx, any_path).unwrap().corners().len(), 6);
        }
        let tip = pts(&CornerWidgets::for_tool(&cx, false).unwrap())[0];
        let mut t = crate::create("selection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 300.0));
        // A hexagon's 120° corners stop where their cuts meet: 40 · tan 60°.
        assert!((radius(&a[1]) - 40.0 * 3f64.sqrt()).abs() < 1e-9, "{a:?}");
        let red = t
            .overlays(&cx)
            .into_iter()
            .any(|o| matches!(o, Overlay::Path { color, path, .. } if color == crate::builder::HIGHLIGHT_RED && path.elements().len() == 12));
        assert!(red, "six arcs at the limit");
    }

    /// A pen path's middle corner shows a widget; its ends and curved corners don't.
    #[test]
    fn a_pen_path_shows_widgets_on_its_corners_only() {
        let mut sp = vectorcraft_geom::SubPath::polyline(
            &[Point::new(100.0, 100.0), Point::new(300.0, 100.0), Point::new(300.0, 300.0), Point::new(100.0, 300.0)],
            false,
        );
        sp.anchors[2].h_out = Point::new(250.0, 350.0);
        let (d, id) = doc_with(PathData::single(sp), None);
        let s = selected(id);
        let p = paint();
        let w = CornerWidgets::for_tool(&cx(&d, &s, &p), true).unwrap();
        assert_eq!(w.corners(), [1]);
        assert_eq!(pts(&w), [Point::new(290.0, 110.0)]);
    }
}
