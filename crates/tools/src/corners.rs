//! Live Corners: the widgets inside the corners of the selected paths. The Selection tool shows
//! them on live rectangles and polygons, Direct Selection on any path (a star, a pen path) at each
//! corner anchor: one that isn't smooth, where two sides, straight or curved, meet at an angle. Dragging one rounds (or
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
    /// The radius it is set to (document units), the one it is drawn with (no larger than its
    /// neighbours leave room for) and its kind.
    radius: f64,
    drawn: f64,
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
    /// with Direct Selection (`any_path`), live rectangles and polygons with Selection. `id` is
    /// editable ([`Document::is_editable`]).
    fn of(doc: &Document, selection: &Selection, id: NodeId, zoom: f64, any_path: bool) -> Option<Self> {
        if selection.segments.contains_key(&id) {
            return None;
        }
        let NodeKind::Path { path, live, .. } = &doc.node(id)?.kind else { return None };
        if !any_path && !matches!(live.as_deref(), Some(LiveShape::Rectangle { .. } | LiveShape::Polygon { .. })) {
            return None;
        }
        let lc = LiveCorners::new(path, live.as_deref())?;
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
                let (radius, drawn) = (lc.radius(c.index), lc.drawn_radius(c.index));
                let k = (drawn / c.sin()).max(MIN_INSET_PX / (scale * (1.0 + c.cos()).sqrt())).min(c.lu.min(c.lv) / 2.0);
                Widget { corner: *c, radius, drawn, kind: lc.kind(c.index), angle: angle_in(lc.xf, c), at: lc.xf * c.on_bisector(k) }
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
        let editable = doc.editable();
        let paths: Vec<_> =
            selection.objects.iter().filter(|id| editable(**id)).filter_map(|id| PathWidgets::of(doc, selection, *id, zoom, any_path)).collect();
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
    /// The corners of `base`.
    corners: Vec<Corner>,
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
    /// Direct Selection: what a click (no drag) on the pressed widget selects.
    click: Option<CornerClick>,
}

/// Direct Selection: the drawn anchors of a widget's corner, which a click on the widget selects
/// (Shift: toggles), as a click on the corner's anchor does.
#[derive(Clone, Debug)]
struct CornerClick {
    id: NodeId,
    anchors: Vec<(usize, usize)>,
    shift: bool,
}

impl CornerDrag {
    /// The drag has left the click: the corner radius is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on a widget of the selection (Direct Selection:
    /// `any_path`).
    pub fn hit(cx: &ToolContext, ev: &PointerEvent, any_path: bool) -> Option<Self> {
        let (widgets, p) = (CornerWidgets::for_tool(cx, any_path)?, ev.pos);
        let widget = widgets.hit(p, cx.tol(5.0))?;
        let radius = widgets.widget(widget.0, widget.1)?.radius;
        let click = any_path
            .then(|| {
                let path = widgets.paths.get(widget.0)?;
                let corner = path.widgets.get(widget.1)?.corner.index;
                let lc = cx.doc.node(path.id).and_then(LiveCorners::of)?;
                Some(CornerClick { id: path.id, anchors: lc.anchors_of(&[corner].into()).into_iter().collect(), shift: ev.mods.shift })
            })
            .flatten();
        let outlines = widgets
            .paths
            .iter()
            .map(|w| {
                let lc = cx.doc.node(w.id).and_then(LiveCorners::of)?;
                let n = lc.base.anchor_count();
                let (radii, kinds) = ((0..n).map(|k| lc.radius(k)).collect(), (0..n).map(|k| lc.kind(k)).collect());
                Some(Outline { corners: lc.corners.clone(), base: lc.base.into_owned(), radii, kinds, xf: w.xf })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { widgets, widget, outlines, start: p, began: false, alt: ev.mods.alt, radius, at_limit: false, at: p, click })
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
        let max = self.max_radius(path, &w.corner);
        let r = w.drawn + w.corner.radius_change(inv * p - inv * self.start);
        (r.min(max).max(0.0), max > 0.0 && r >= max)
    }

    /// The largest radius `corner` of shown path `path` draws while the drag sets every shown
    /// corner of that path to one radius: up to the cuts of its neighbours, as far as its own
    /// sides when they are square (#2332).
    fn max_radius(&self, path: usize, corner: &Corner) -> f64 {
        let (Some(shown), Some(o)) = (self.widgets.paths.get(path), self.outlines.get(path)) else { return corner.max_radius() };
        corner.max_shared_radius(&o.corners, &o.radii, |k| shown.widgets.iter().any(|w| w.corner.index == k))
    }

    /// The cuts of the edited corners that are at their largest radius, in document coordinates.
    fn corners_at_limit(&self) -> BezPath {
        let mut out = BezPath::new();
        for (n, (shown, o)) in self.widgets.paths.iter().zip(&self.outlines).enumerate() {
            let shown = &shown.widgets;
            let mut radii = o.radii.clone();
            for w in shown {
                if let Some(r) = radii.get_mut(w.corner.index) {
                    *r = self.radius;
                }
            }
            let full = |k: &usize| shown.iter().any(|w| w.corner.index == *k && self.radius >= self.max_radius(n, &w.corner) - 1e-9);
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
        } else if let Some(c) = self.click {
            // Direct Selection: the click selects the widget's corner, so the next drag on this
            // widget rounds that corner alone (its widget is then the only one showing).
            let mode = if c.shift { "toggle" } else { "set" };
            vec![Action::Exec("select.anchors".into(), json!({"id": c.id.0, "anchors": c.anchors, "mode": mode}))]
        } else {
            vec![]
        }
    }

    /// The radius readout next to the pointer; at the largest radius, the corners there in red.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began {
            return vec![];
        }
        let mut out = vec![];
        if cx.measurement_labels {
            out.push(Overlay::Measure { p: self.at, text: format!("Radius: {}", cx.len(self.radius)) });
        }
        if self.at_limit {
            out.push(Overlay::Path { path: self.corners_at_limit(), color: crate::builder::HIGHLIGHT_RED, width: 2.0, dashed: false });
        }
        out
    }
}

#[cfg(test)]
mod tests;
