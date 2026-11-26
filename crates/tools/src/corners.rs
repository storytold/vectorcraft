//! Live Corners: the widgets inside the corners of the selected paths. The Selection tool shows
//! them on live rectangles and polygons, Direct Selection on any path (a star, a pen path) at each
//! corner anchor: one that isn't smooth, where two sides, straight or curved, meet at an angle. Dragging one rounds (or
//! sharpens) the picked corners together, on every selected path (#938): every corner of a path
//! selected whole, those with a selected anchor when Direct Selection picked some. Direct Selection
//! also shows the widgets of the corners it did not pick, drawn apart from the picked ones, so a
//! click or Shift-click on any of them picks it (#1232); a plain click on a picked one keeps the
//! picks, so a double-click opens Corners for all of them; dragging or Alt-clicking one that is
//! not picked adds its corner to the edit. Alt-clicking a widget cycles their corner
//! kind (round, inverted round, chamfer), as the Up and Down arrows do while one is dragged
//! (#2898); double-clicking one opens the Corners dialog with Direct Selection and shows the
//! Transform panel with the Selection tool. The
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
    /// Its corner is picked: a drag, an Alt-click and the Corners dialog edit it.
    picked: bool,
}

/// The corner widgets of one selected path.
#[derive(Clone, Debug, PartialEq)]
struct PathWidgets {
    id: NodeId,
    /// Maps the corners' space into the document (a live rectangle's own; else none).
    xf: Affine,
    /// The widgets that show, picked or not, in anchor order.
    widgets: Vec<Widget>,
    /// How many corners the path has, shown or not.
    total: usize,
    /// Every corner of the path is selected: the path selected whole, or every corner's anchor.
    whole: bool,
}

impl PathWidgets {
    /// The widgets of path `id` at `zoom` (screen px per document point), if it has corners: any
    /// path with Direct Selection (`any_path`), every corner's, picked or not; live rectangles' and
    /// polygons' with Selection, the picked corners' alone. Corners whose sides are too short on
    /// screen show no widget, so `widgets` can be empty. Only when `editable`
    /// ([`Document::editable`]) answers true for `id`.
    fn of(doc: &Document, selection: &Selection, id: NodeId, zoom: f64, any_path: bool, editable: &impl Fn(NodeId) -> bool) -> Option<Self> {
        if selection.segments.contains_key(&id) {
            return None;
        }
        let NodeKind::Path { path, live, .. } = &doc.node(id)?.kind else { return None };
        if !any_path && !matches!(live.as_deref(), Some(LiveShape::Rectangle { .. } | LiveShape::Polygon { .. })) {
            return None;
        }
        // After the kind: on a locked artboard it measures the art of the layer object holding `id`.
        if !editable(id) {
            return None;
        }
        let lc = LiveCorners::new(path, live.as_deref())?;
        let det = lc.xf.determinant().abs();
        if det < 1e-12 {
            return None;
        }
        // Screen pixels per unit of the corners' space (a rectangle's may be skewed).
        let scale = zoom * det.sqrt();
        let partial = selection.partial(id);
        let picked = lc.picked(partial);
        let whole = partial.is_none_or(|anchors| lc.all().is_subset(&lc.corners_of(anchors)));
        let widgets: Vec<_> = lc
            .corners
            .iter()
            .filter(|c| (any_path || picked.contains(&c.index)) && c.lu.min(c.lv) * scale >= MIN_SIDE_PX)
            .map(|c| {
                // On the bisector at the centre of the arc as drawn, at least the minimum inset
                // in and at most half the shorter side.
                let (radius, drawn) = (lc.radius(c.index), lc.drawn_radius(c.index));
                let k = (drawn / c.sin()).max(MIN_INSET_PX / (scale * (1.0 + c.cos()).sqrt())).min(c.lu.min(c.lv) / 2.0);
                let at = lc.xf * c.on_bisector(k);
                Widget { corner: *c, radius, drawn, kind: lc.kind(c.index), angle: angle_in(lc.xf, c), at, picked: picked.contains(&c.index) }
            })
            .collect();
        Some(Self { id, xf: lc.xf, widgets, total: lc.corners.len(), whole })
    }

    /// The corners whose widgets show: anchor indices of the path's uncut outline.
    fn corners(&self) -> Vec<usize> {
        self.widgets.iter().map(|w| w.corner.index).collect()
    }

    /// The shown corners that are picked, which an edit changes.
    fn edited(&self) -> Vec<usize> {
        self.widgets.iter().filter(|w| w.picked).map(|w| w.corner.index).collect()
    }

    /// An edit changes every corner of the path, which the selection already selects: each one
    /// shows and is picked, and none was picked by the press alone.
    fn edits_every(&self) -> bool {
        self.whole && self.widgets.len() == self.total && self.widgets.iter().all(|w| w.picked)
    }

    /// Is shown corner `index` edited?
    fn edits(&self, index: usize) -> bool {
        self.widgets.iter().any(|w| w.picked && w.corner.index == index)
    }
}

/// The corner widgets of the selected paths that have some.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerWidgets {
    paths: Vec<PathWidgets>,
    /// Every corner of every selected path with corners is selected, those whose widgets are
    /// hidden included.
    every: bool,
}

impl CornerWidgets {
    /// The widgets at `zoom` (screen px per document point), if the selection has them: on any
    /// path with Direct Selection (`any_path`), on live rectangles and polygons with Selection.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64, any_path: bool) -> Option<Self> {
        // Made for the first path, then shared: the art on the locked artboards is measured once.
        let locks = std::cell::OnceCell::new();
        let editable = |id| locks.get_or_init(|| doc.editable())(id);
        let mut paths: Vec<_> = selection.objects.iter().filter_map(|id| PathWidgets::of(doc, selection, *id, zoom, any_path, &editable)).collect();
        let every = paths.iter().all(|p| p.whole);
        paths.retain(|p| !p.widgets.is_empty());
        (!paths.is_empty()).then_some(Self { paths, every })
    }

    /// Selection & Anchor Display → Hide Corner Widget for angles greater than: the widgets of
    /// corners wider than `max` degrees (in the document) hide; none if all do.
    pub fn within_angle(mut self, max: f64) -> Option<Self> {
        for p in &mut self.paths {
            p.widgets.retain(|w| w.angle <= max + 1e-9);
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

    /// The widgets the active tool can drag (Direct Selection: `any_path`). The live shapes' widgets
    /// of the other tools also need Show Shape Widgets.
    pub fn for_tool(cx: &ToolContext, any_path: bool) -> Option<Self> {
        let on = cx.corner_widgets && (any_path || cx.shape_widgets);
        Self::showing(cx.doc, cx.selection, cx.zoom, any_path, on, cx.corner_widget_max_angle)
    }

    /// The centres of the widgets that show.
    pub fn visible(&self) -> impl Iterator<Item = Point> + '_ {
        self.visible_on().map(|(_, at, _)| at)
    }

    /// The centers of the widgets that show, each with the object it belongs to and whether its
    /// corner is picked.
    pub fn visible_on(&self) -> impl Iterator<Item = (NodeId, Point, bool)> + '_ {
        self.paths.iter().flat_map(|p| p.widgets.iter().map(move |w| (p.id, w.at, w.picked)))
    }

    /// The corners whose widgets show, path after path: anchor indices of each path's uncut
    /// outline.
    pub fn corners(&self) -> Vec<usize> {
        self.paths.iter().flat_map(PathWidgets::corners).collect()
    }

    /// The shown corners that are picked, path after path, which a drag, an Alt-click or the
    /// Corners dialog edit.
    pub fn picked(&self) -> Vec<usize> {
        self.paths.iter().flat_map(PathWidgets::edited).collect()
    }

    /// The widget nearest to `p` within `tol` (document units): its path's and its own index. Of
    /// widgets at the same spot, a picked one.
    fn hit(&self, p: Point, tol: f64) -> Option<(usize, usize)> {
        let d = |&(i, k): &(usize, usize)| self.widget(i, k).map_or(f64::INFINITY, |w| w.at.distance(p));
        let unpicked = |&(i, k): &(usize, usize)| self.widget(i, k).is_none_or(|w| !w.picked);
        self.paths
            .iter()
            .enumerate()
            .flat_map(|(i, path)| (0..path.widgets.len()).map(move |k| (i, k)))
            .filter(|ik| d(ik) <= tol)
            .min_by(|a, b| d(a).total_cmp(&d(b)).then(unpicked(a).cmp(&unpicked(b))))
    }

    fn widget(&self, path: usize, k: usize) -> Option<&Widget> {
        self.paths.get(path).and_then(|p| p.widgets.get(k))
    }

    /// `object.setLiveShape` params setting `key` on the picked corners that show (every corner of
    /// a path when all are, which `corners` then leaves out): one path by its `id`, several as
    /// `items`. Paths with no picked corner whose widget shows are left out.
    fn command(&self, key: &str, value: Value) -> Value {
        let corners = |p: &PathWidgets| (!p.edits_every()).then(|| p.edited());
        let paths: Vec<&PathWidgets> = self.paths.iter().filter(|p| p.widgets.iter().any(|w| w.picked)).collect();
        let mut out = Map::new();
        match paths.as_slice() {
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
    widget_at(cx, p, any_path).is_some()
}

/// The corner widget at `p` the active tool would drag (Direct Selection: `any_path`): whether its
/// corner is picked, and its center.
pub fn widget_at(cx: &ToolContext, p: Point, any_path: bool) -> Option<(bool, Point)> {
    let w = CornerWidgets::for_tool(cx, any_path)?;
    let (i, k) = w.hit(p, cx.tol(5.0))?;
    w.widget(i, k).map(|w| (w.picked, w.at))
}

/// A double-click on a corner widget opens the Corners dialog for the picked corners of its path
/// that show, or for the clicked widget's corner alone when it is not picked (Direct Selection;
/// the Selection tool shows the Transform panel instead). With Direct Selection the
/// double-click's first click has left the picks as they are on a picked widget,
/// and picked that corner alone on one that is not (`CornerDrag::finish`).
pub fn double_click(cx: &ToolContext, p: Point, any_path: bool) -> Option<Action> {
    double_click_unless(cx, p, any_path, |_| false)
}

/// [`double_click`], except that a widget whose corner is not picked does nothing when `yields`
/// for its center (something else under the pointer takes the double-click).
pub fn double_click_unless(cx: &ToolContext, p: Point, any_path: bool, yields: impl FnOnce(Point) -> bool) -> Option<Action> {
    let w = CornerWidgets::for_tool(cx, any_path)?;
    let (i, k) = w.hit(p, cx.tol(5.0))?;
    let path = w.paths.get(i)?;
    let clicked = path.widgets.get(k)?;
    if !clicked.picked && yields(clicked.at) {
        return None;
    }
    let corners: Vec<usize> = if clicked.picked { path.edited() } else { vec![clicked.corner.index] };
    Some(Action::Dialog(DIALOG.into(), json!({ "id": path.id.0, "corners": corners })))
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
    /// The interaction began (a drag or an arrow press): the corners preview and commit.
    began: bool,
    /// The pointer moved past the drag threshold: the radius follows it.
    dragging: bool,
    alt: bool,
    radius: f64,
    /// The pointer went past fully round: the dragged corner is at its largest radius.
    at_limit: bool,
    at: Point,
    /// Direct Selection: what a click (no drag) on the pressed widget selects.
    click: Option<CornerClick>,
    /// The pressed widget's corner was not picked: the press adds it to the edit.
    unpicked: bool,
    /// The corner kind the Up and Down arrows picked during the drag, which it sets with the
    /// radius (#2898); None: the corners keep theirs.
    kind: Option<CornerKind>,
}

/// Direct Selection: the drawn anchors of a widget's corner, which a click on the widget selects
/// (Shift: toggles, unless every corner is selected), as a click on the corner's anchor does.
#[derive(Clone, Debug)]
struct CornerClick {
    id: NodeId,
    anchors: Vec<(usize, usize)>,
    mode: ClickMode,
}

/// How a click on a widget changes the selected anchors.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ClickMode {
    /// The corner alone is selected.
    Set,
    /// The selection stays as it is: a plain click on a picked widget while some corners are not
    /// picked, as a plain click on one of several selected anchors keeps them.
    Keep,
    /// The corner's anchors are toggled on a path with some anchors selected.
    Toggle,
    /// The corner's anchors are toggled on a path selected whole, starting from all of them.
    ToggleWhole,
}

impl CornerDrag {
    /// The drag has left the click: the corner radius is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// The pressed widget's corner is not picked.
    pub fn on_unpicked(&self) -> bool {
        self.unpicked
    }

    /// The pressed widget's center.
    pub fn widget_center(&self) -> Option<Point> {
        self.widgets.widget(self.widget.0, self.widget.1).map(|w| w.at)
    }

    /// Start a drag when the press `ev` is on a widget of the selection (Direct Selection:
    /// `any_path`).
    pub fn hit(cx: &ToolContext, ev: &PointerEvent, any_path: bool) -> Option<Self> {
        let (mut widgets, p) = (CornerWidgets::for_tool(cx, any_path)?, ev.pos);
        let widget = widgets.hit(p, cx.tol(5.0))?;
        let (radius, unpicked) = widgets.widget(widget.0, widget.1).map(|w| (w.radius, !w.picked))?;
        let click = any_path
            .then(|| {
                let path = widgets.paths.get(widget.0)?;
                let corner = path.widgets.get(widget.1)?.corner.index;
                let lc = cx.doc.node(path.id).and_then(LiveCorners::of)?;
                // While every corner of every selected path is selected (the paths selected
                // whole), a Shift-click picks this corner alone, as the first click of "click one
                // widget, then Shift-click the others" does: toggling it would deselect it and
                // leave the rest (#5188). A path whose widgets are all hidden counts too, so its
                // picks are kept. Selected anchors that are no corner show every widget too, but
                // select no corner: there a Shift-click adds the corner.
                let every = widgets.every;
                // A plain click on a widget whose corner is selected keeps the picks, so the second
                // click of a double-click opens Corners for all of them. Anchors that are no corner
                // pick every widget but select no corner.
                let selected = cx.selection.partial(path.id).is_none_or(|anchors| lc.corners_of(anchors).contains(&corner));
                let mode = match (ev.mods.shift && !every, cx.selection.partial(path.id).is_some()) {
                    (false, _) if !every && !unpicked && selected => ClickMode::Keep,
                    (false, _) => ClickMode::Set,
                    (true, true) => ClickMode::Toggle,
                    // A path selected whole beside others with corners picked: the Shift-click
                    // deselects this corner and keeps the rest of the path selected.
                    (true, false) => ClickMode::ToggleWhole,
                };
                // A toggle takes the corner out or puts it in whole: of a corner drawn with two
                // anchors, it flips only the selected ones of a selected corner and only the others
                // of a corner not selected, so one selected anchor does not swap for the other.
                let partial = cx.selection.partial(path.id);
                let anchors = lc
                    .anchors_of(&[corner].into())
                    .into_iter()
                    .filter(|a| mode != ClickMode::Toggle || partial.is_some_and(|s| s.contains(a)) == selected)
                    .collect();
                Some(CornerClick { id: path.id, anchors, mode })
            })
            .flatten();
        // Pressing a widget whose corner is not picked adds that corner to what the press edits;
        // the press does not pick it.
        if let Some(w) = widgets.paths.get_mut(widget.0).and_then(|path| path.widgets.get_mut(widget.1)) {
            w.picked = true;
        }
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
        Some(Self {
            widgets,
            widget,
            outlines,
            start: p,
            began: false,
            dragging: false,
            alt: ev.mods.alt,
            radius,
            at_limit: false,
            at: p,
            click,
            unpicked,
            kind: None,
        })
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

    /// The largest radius `corner` of shown path `path` draws while the drag sets every edited
    /// corner of that path to one radius: up to the cuts of its neighbours, as far as its own
    /// sides when they are square (#2332).
    fn max_radius(&self, path: usize, corner: &Corner) -> f64 {
        let (Some(shown), Some(o)) = (self.widgets.paths.get(path), self.outlines.get(path)) else { return corner.max_radius() };
        corner.max_shared_radius(&o.corners, &o.radii, |k| shown.edits(k))
    }

    /// The cuts of the edited corners that are at their largest radius, in document coordinates.
    fn corners_at_limit(&self) -> BezPath {
        let mut out = BezPath::new();
        for (n, (shown, o)) in self.widgets.paths.iter().zip(&self.outlines).enumerate() {
            let shown: Vec<&Widget> = shown.widgets.iter().filter(|w| w.picked).collect();
            let mut radii = o.radii.clone();
            for w in &shown {
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
        if !self.dragging {
            // The drag threshold holds after an arrow press too: until the pointer passes it, the
            // radius stays as it was.
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.dragging = true;
            if !self.began {
                self.began = true;
                out.push(Action::Begin("Corner Radius".into()));
            }
        }
        (self.radius, self.at_limit) = self.radius_at(p);
        self.at = p;
        out.push(self.preview());
        out
    }

    /// The corners as the gesture sets them: the radius once the pointer drags it, and the kind
    /// the arrows picked. An arrow press without a drag sends the kind alone, so the corners keep
    /// their own radii.
    fn preview(&self) -> Action {
        let p = match (self.dragging, self.kind) {
            (true, kind) => {
                let mut p = self.widgets.command("radius", json!(self.radius));
                if let Some(k) = kind {
                    p["kind"] = json!(k);
                }
                p
            }
            (false, Some(k)) => self.widgets.command("kind", json!(k)),
            (false, None) => self.widgets.command("radius", json!(self.radius)),
        };
        Action::Preview("object.setLiveShape".into(), p)
    }

    /// The Up (`up`) or Down arrow while the widget is pressed or dragged: the corners take the
    /// next (or previous) kind, round, inverted round and chamfer in turn, starting from the
    /// pressed corner's, and keep it when the drag ends (#2898).
    pub fn step_kind(&mut self, up: bool) -> Vec<Action> {
        let Some(from) = self.kind.or_else(|| self.widgets.widget(self.widget.0, self.widget.1).map(|w| w.kind)) else { return vec![] };
        self.kind = Some(if up { from.next() } else { from.previous() });
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin("Corner Radius".into()));
        }
        out.push(self.preview());
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began {
            vec![Action::Commit]
        } else if let Some(w) = self.widgets.widget(self.widget.0, self.widget.1).filter(|_| self.alt) {
            vec![Action::Exec("object.setLiveShape".into(), self.widgets.command("kind", json!(w.kind.next())))]
        } else if let Some(c) = self.click {
            // Direct Selection: the click selects the widget's corner, so the next drag on this
            // widget rounds that corner alone; the other corners' widgets show as not picked.
            match c.mode {
                ClickMode::Keep => vec![],
                ClickMode::Set => vec![Action::Exec("select.anchors".into(), json!({"id": c.id.0, "anchors": c.anchors, "mode": "set"}))],
                ClickMode::Toggle => vec![Action::Exec("select.anchors".into(), json!({"id": c.id.0, "anchors": c.anchors, "mode": "toggle"}))],
                ClickMode::ToggleWhole => {
                    vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": c.id.0, "anchors": c.anchors}], "mode": "toggle"}))]
                }
            }
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
